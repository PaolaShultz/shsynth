//! Preallocated callback plan compiled from a validated audio graph.

use crate::audio_graph::{GraphDefinition, NodeId, NodeKind, SourceKind, MAX_CALLBACK_FRAMES};
use crate::dsp::{db_to_gain, AtomicMeter, MeterAccumulator, SmoothedValue, StereoFrame};
use crate::effects::{EffectSlot, MeterHandles};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const TIMING_HISTOGRAM_BUCKET_NANOSECONDS: u64 = 1_000;
const TIMING_HISTOGRAM_BUCKETS: usize = 8_193;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanError(String);

impl PlanError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PlanError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessStatus {
    Complete,
    OversizedBlock,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CallbackTimingSnapshot {
    pub callbacks: u64,
    pub total_nanoseconds: u64,
    pub maximum_nanoseconds: u64,
    pub p95_nanoseconds: u64,
    pub p99_nanoseconds: u64,
    pub missed_deadlines: u64,
    pub oversized_callbacks: u64,
}

impl CallbackTimingSnapshot {
    pub fn mean_nanoseconds(self) -> u64 {
        self.total_nanoseconds
            .checked_div(self.callbacks)
            .unwrap_or(0)
    }
}

/// Lock-free counters written by the JACK callback and sampled by the owner.
/// The callback increments one fixed one-microsecond histogram bucket. The
/// owner scans those buckets to calculate percentiles outside the callback.
pub struct CallbackTimingCounters {
    callbacks: AtomicU64,
    total_nanoseconds: AtomicU64,
    maximum_nanoseconds: AtomicU64,
    missed_deadlines: AtomicU64,
    oversized_callbacks: AtomicU64,
    histogram: Box<[AtomicU64]>,
}

impl Default for CallbackTimingCounters {
    fn default() -> Self {
        Self {
            callbacks: AtomicU64::new(0),
            total_nanoseconds: AtomicU64::new(0),
            maximum_nanoseconds: AtomicU64::new(0),
            missed_deadlines: AtomicU64::new(0),
            oversized_callbacks: AtomicU64::new(0),
            histogram: (0..TIMING_HISTOGRAM_BUCKETS)
                .map(|_| AtomicU64::new(0))
                .collect(),
        }
    }
}

impl CallbackTimingCounters {
    pub fn record(
        &self,
        frames: u32,
        sample_rate: u32,
        elapsed_nanoseconds: u64,
        status: ProcessStatus,
    ) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.total_nanoseconds
            .fetch_add(elapsed_nanoseconds, Ordering::Relaxed);
        self.maximum_nanoseconds
            .fetch_max(elapsed_nanoseconds, Ordering::Relaxed);
        let bucket = elapsed_nanoseconds.saturating_add(TIMING_HISTOGRAM_BUCKET_NANOSECONDS - 1)
            / TIMING_HISTOGRAM_BUCKET_NANOSECONDS;
        let bucket = usize::try_from(bucket)
            .unwrap_or(usize::MAX)
            .min(TIMING_HISTOGRAM_BUCKETS - 1);
        self.histogram[bucket].fetch_add(1, Ordering::Relaxed);
        if matches!(status, ProcessStatus::OversizedBlock) {
            self.oversized_callbacks.fetch_add(1, Ordering::Relaxed);
        }
        let deadline = u64::from(frames)
            .saturating_mul(1_000_000_000)
            .checked_div(u64::from(sample_rate))
            .unwrap_or(0);
        if deadline > 0 && elapsed_nanoseconds > deadline {
            self.missed_deadlines.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> CallbackTimingSnapshot {
        let maximum_nanoseconds = self.maximum_nanoseconds.load(Ordering::Acquire);
        let histogram = self
            .histogram
            .iter()
            .map(|bucket| bucket.load(Ordering::Acquire))
            .collect::<Vec<_>>();
        CallbackTimingSnapshot {
            callbacks: self.callbacks.load(Ordering::Acquire),
            total_nanoseconds: self.total_nanoseconds.load(Ordering::Acquire),
            maximum_nanoseconds,
            p95_nanoseconds: percentile_nanoseconds(&histogram, 95, maximum_nanoseconds),
            p99_nanoseconds: percentile_nanoseconds(&histogram, 99, maximum_nanoseconds),
            missed_deadlines: self.missed_deadlines.load(Ordering::Acquire),
            oversized_callbacks: self.oversized_callbacks.load(Ordering::Acquire),
        }
    }
}

fn percentile_nanoseconds(histogram: &[u64], percentile: u64, maximum: u64) -> u64 {
    let samples = histogram.iter().copied().sum::<u64>();
    if samples == 0 {
        return 0;
    }
    let target = samples.saturating_mul(percentile).saturating_add(99) / 100;
    let mut seen = 0_u64;
    for (index, count) in histogram.iter().copied().enumerate() {
        seen = seen.saturating_add(count);
        if seen >= target {
            if index == TIMING_HISTOGRAM_BUCKETS - 1 {
                return maximum;
            }
            return (index as u64).saturating_mul(TIMING_HISTOGRAM_BUCKET_NANOSECONDS);
        }
    }
    maximum
}

enum Operation {
    Source,
    Pass,
    Fader(Box<RuntimeFader>),
    Effect(Box<EffectSlot>),
    Sink,
}

struct RuntimeFader {
    gain: SmoothedValue,
    meter: MeterAccumulator,
    published: Arc<AtomicMeter>,
}

impl RuntimeFader {
    fn new(gain_db: f32, maximum_frames: usize) -> Result<Self, PlanError> {
        Ok(Self {
            gain: SmoothedValue::new(
                db_to_gain(gain_db).map_err(|error| PlanError::new(error.to_string()))?,
            )
            .map_err(|error| PlanError::new(error.to_string()))?,
            meter: MeterAccumulator::new(maximum_frames)
                .map_err(|error| PlanError::new(error.to_string()))?,
            published: Arc::new(AtomicMeter::default()),
        })
    }

    #[inline]
    fn process(&mut self, frames: &mut [StereoFrame]) {
        for frame in frames.iter_mut() {
            let gain = self.gain.next_value();
            *frame = self
                .meter
                .process(StereoFrame::new(frame.left * gain, frame.right * gain));
        }
        self.published.publish(self.meter.snapshot_and_clear_peak());
    }
}

struct RuntimeNode {
    id: NodeId,
    buffer: usize,
    inputs: Box<[usize]>,
    operation: Operation,
}

pub struct GraphPlan {
    sample_rate: u32,
    maximum_frames: usize,
    buffers: Vec<Box<[StereoFrame]>>,
    nodes: Vec<RuntimeNode>,
    node_buffers: BTreeMap<NodeId, usize>,
    source_nodes: Box<[NodeId]>,
    sink_nodes: Box<[NodeId]>,
}

impl GraphPlan {
    pub fn compile(graph: &GraphDefinition) -> Result<Self, PlanError> {
        Self::compile_retaining(graph, None)
    }

    /// Compile on the control thread, moving compatible effect slots out of
    /// the previous plan so reorder/publication does not clear filter,
    /// detector, hold, dither, smoothing, or metering state.
    pub fn compile_retaining(
        graph: &GraphDefinition,
        previous: Option<Self>,
    ) -> Result<Self, PlanError> {
        let order = graph
            .validate()
            .map_err(|error| PlanError::new(error.to_string()))?;
        let maximum_frames = graph.maximum_callback_frames as usize;
        if maximum_frames == 0 || maximum_frames > MAX_CALLBACK_FRAMES as usize {
            return Err(PlanError::new("invalid callback frame capacity"));
        }
        let node_by_id = graph
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<BTreeMap<_, _>>();
        let node_buffers = order
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect::<BTreeMap<_, _>>();
        let mut incoming = BTreeMap::<NodeId, Vec<usize>>::new();
        for edge in &graph.edges {
            incoming
                .entry(edge.to)
                .or_default()
                .push(node_buffers[&edge.from]);
        }
        let effects = graph
            .effects
            .iter()
            .map(|effect| (effect.id, effect))
            .collect::<BTreeMap<_, _>>();
        let sends = graph
            .sends
            .iter()
            .map(|send| ((send.source_node, send.aux_id), send))
            .collect::<BTreeMap<_, _>>();
        let aux_buses = graph
            .aux_buses
            .iter()
            .map(|aux| (aux.id, aux))
            .collect::<BTreeMap<_, _>>();
        let mut retained = BTreeMap::new();
        if let Some(previous) = previous.filter(|plan| {
            plan.sample_rate == graph.sample_rate && plan.maximum_frames == maximum_frames
        }) {
            for node in previous.nodes {
                if let Operation::Effect(slot) = node.operation {
                    retained.insert(slot.id(), slot);
                }
            }
        }
        let mut nodes = Vec::with_capacity(order.len());
        let mut source_nodes = Vec::new();
        let mut sink_nodes = Vec::new();
        for id in order {
            let node = node_by_id[&id];
            let operation = match &node.kind {
                NodeKind::Source { .. } => {
                    source_nodes.push(id);
                    Operation::Source
                }
                NodeKind::StereoMixer => {
                    Operation::Fader(Box::new(RuntimeFader::new(0.0, maximum_frames)?))
                }
                NodeKind::Processor { effect_id } => {
                    let effect = effects
                        .get(effect_id)
                        .ok_or_else(|| PlanError::new("processor effect missing"))?;
                    let slot = match retained.remove(effect_id) {
                        Some(mut slot) if slot.kind() == effect.kind => {
                            slot.apply_instance(effect)
                                .map_err(|error| PlanError::new(error.to_string()))?;
                            slot
                        }
                        _ => Box::new(
                            EffectSlot::compile(effect, graph.sample_rate, maximum_frames)
                                .map_err(|error| PlanError::new(error.to_string()))?,
                        ),
                    };
                    Operation::Effect(slot)
                }
                NodeKind::Sink { .. } => {
                    sink_nodes.push(id);
                    Operation::Sink
                }
                NodeKind::SendTap {
                    aux_id,
                    source_node,
                } => Operation::Fader(Box::new(RuntimeFader::new(
                    sends
                        .get(&(*source_node, *aux_id))
                        .ok_or_else(|| PlanError::new("send tap route missing"))?
                        .level_db,
                    maximum_frames,
                )?)),
                NodeKind::AuxReturn { aux_id } => Operation::Fader(Box::new(RuntimeFader::new(
                    aux_buses
                        .get(aux_id)
                        .ok_or_else(|| PlanError::new("aux return bus missing"))?
                        .return_gain_db,
                    maximum_frames,
                )?)),
                NodeKind::MonoToStereo => Operation::Pass,
            };
            nodes.push(RuntimeNode {
                id,
                buffer: node_buffers[&id],
                inputs: incoming.remove(&id).unwrap_or_default().into_boxed_slice(),
                operation,
            });
        }
        let buffers = (0..nodes.len())
            .map(|_| vec![StereoFrame::SILENCE; maximum_frames].into_boxed_slice())
            .collect();
        Ok(Self {
            sample_rate: graph.sample_rate,
            maximum_frames,
            buffers,
            nodes,
            node_buffers,
            source_nodes: source_nodes.into_boxed_slice(),
            sink_nodes: sink_nodes.into_boxed_slice(),
        })
    }

    pub fn maximum_frames(&self) -> usize {
        self.maximum_frames
    }

    /// Replace topology on a stopped control thread. Compatible slots are
    /// moved into the new plan before publication, preserving their state.
    pub fn reconfigure(&mut self, graph: &GraphDefinition) -> Result<(), PlanError> {
        let mut next = Self::compile(graph)?;
        if self.sample_rate == next.sample_rate && self.maximum_frames == next.maximum_frames {
            let effects = graph
                .effects
                .iter()
                .map(|effect| (effect.id, effect))
                .collect::<BTreeMap<_, _>>();
            for next_index in 0..next.nodes.len() {
                let (effect_id, effect_kind) = match &next.nodes[next_index].operation {
                    Operation::Effect(slot) => (slot.id(), slot.kind()),
                    _ => continue,
                };
                let Some(old_index) = self.nodes.iter().position(|node| {
                    matches!(
                        &node.operation,
                        Operation::Effect(slot)
                            if slot.id() == effect_id && slot.kind() == effect_kind
                    )
                }) else {
                    continue;
                };
                let old_slot = match &mut self.nodes[old_index].operation {
                    Operation::Effect(slot) => slot,
                    _ => unreachable!("position selected an effect slot"),
                };
                let next_slot = match &mut next.nodes[next_index].operation {
                    Operation::Effect(slot) => slot,
                    _ => unreachable!("loop selected an effect slot"),
                };
                old_slot
                    .apply_instance(effects[&effect_id])
                    .map_err(|error| PlanError::new(error.to_string()))?;
                std::mem::swap(old_slot, next_slot);
            }
        }
        *self = next;
        Ok(())
    }

    pub fn source_nodes(&self) -> &[NodeId] {
        &self.source_nodes
    }

    pub fn sink_nodes(&self) -> &[NodeId] {
        &self.sink_nodes
    }

    pub fn source_kind(graph: &GraphDefinition, id: NodeId) -> Option<&SourceKind> {
        graph.nodes.iter().find_map(|node| match &node.kind {
            NodeKind::Source { source } if node.id == id => Some(source),
            _ => None,
        })
    }

    pub fn source_buffer_mut(&mut self, id: NodeId, frames: usize) -> Option<&mut [StereoFrame]> {
        if frames > self.maximum_frames || !self.source_nodes.contains(&id) {
            return None;
        }
        let index = *self.node_buffers.get(&id)?;
        Some(&mut self.buffers[index][..frames])
    }

    pub fn output_buffer(&self, id: NodeId, frames: usize) -> Option<&[StereoFrame]> {
        if frames > self.maximum_frames || !self.sink_nodes.contains(&id) {
            return None;
        }
        let index = *self.node_buffers.get(&id)?;
        Some(&self.buffers[index][..frames])
    }

    pub fn meter(&self, node_id: NodeId) -> Option<Arc<AtomicMeter>> {
        self.nodes.iter().find_map(|node| match &node.operation {
            Operation::Effect(slot) if node.id == node_id => Some(slot.meters().output),
            Operation::Fader(fader) if node.id == node_id => Some(Arc::clone(&fader.published)),
            _ => None,
        })
    }

    pub fn effect_meters(&self, node_id: NodeId) -> Option<MeterHandles> {
        self.nodes.iter().find_map(|node| match &node.operation {
            Operation::Effect(slot) if node.id == node_id => Some(slot.meters()),
            _ => None,
        })
    }

    pub fn effect_meters_by_id(&self, effect_id: u32) -> Option<MeterHandles> {
        self.nodes.iter().find_map(|node| match &node.operation {
            Operation::Effect(slot) if slot.id() == effect_id => Some(slot.meters()),
            _ => None,
        })
    }

    pub fn set_utility(
        &mut self,
        node_id: NodeId,
        trim_db: f32,
        pan: f32,
    ) -> Result<(), PlanError> {
        if !trim_db.is_finite()
            || !(-60.0..=12.0).contains(&trim_db)
            || !pan.is_finite()
            || !(-1.0..=1.0).contains(&pan)
        {
            return Err(PlanError::new("utility target outside safe range"));
        }
        let state = self
            .nodes
            .iter_mut()
            .find_map(|node| match &mut node.operation {
                Operation::Effect(slot) if node.id == node_id => Some(slot),
                _ => None,
            })
            .ok_or_else(|| PlanError::new("utility node not found"))?;
        state
            .set_parameter("trim_db", trim_db)
            .map_err(|error| PlanError::new(error.to_string()))?;
        state
            .set_parameter("pan", pan)
            .map_err(|error| PlanError::new(error.to_string()))?;
        Ok(())
    }

    pub fn set_effect_bypass(&mut self, node_id: NodeId, bypass: bool) -> Result<(), PlanError> {
        let slot = self
            .nodes
            .iter_mut()
            .find_map(|node| match &mut node.operation {
                Operation::Effect(slot) if node.id == node_id => Some(slot),
                _ => None,
            })
            .ok_or_else(|| PlanError::new("effect node not found"))?;
        slot.set_bypass(bypass)
            .map_err(|error| PlanError::new(error.to_string()))
    }

    /// Process one block. Source buffers must be filled before this call.
    pub fn process(&mut self, frames: usize) -> ProcessStatus {
        if frames > self.maximum_frames {
            return ProcessStatus::OversizedBlock;
        }
        for node_index in 0..self.nodes.len() {
            let target = self.nodes[node_index].buffer;
            if !matches!(self.nodes[node_index].operation, Operation::Source) {
                self.buffers[target][..frames].fill(StereoFrame::SILENCE);
                for input in self.nodes[node_index].inputs.iter().copied() {
                    mix_buffers(&mut self.buffers, input, target, frames);
                }
            }
            match &mut self.nodes[node_index].operation {
                Operation::Source | Operation::Pass | Operation::Sink => {}
                Operation::Fader(fader) => fader.process(&mut self.buffers[target][..frames]),
                Operation::Effect(slot) => slot.process(&mut self.buffers[target][..frames]),
            }
        }
        ProcessStatus::Complete
    }
}

fn mix_buffers(buffers: &mut [Box<[StereoFrame]>], source: usize, target: usize, frames: usize) {
    debug_assert_ne!(source, target);
    let (source_buffer, target_buffer) = if source < target {
        let (left, right) = buffers.split_at_mut(target);
        (&left[source], &mut right[0])
    } else {
        let (left, right) = buffers.split_at_mut(source);
        (&right[0], &mut left[target])
    };
    for (input, output) in source_buffer[..frames]
        .iter()
        .zip(&mut target_buffer[..frames])
    {
        output.left = crate::dsp::finite_or_zero(output.left + input.left);
        output.right = crate::dsp::finite_or_zero(output.right + input.right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_graph::*;
    use crate::dsp::allocation_test::assert_no_allocations;
    use std::collections::BTreeMap;

    fn graph(with_utility: bool) -> GraphDefinition {
        let mut nodes = vec![
            Node {
                id: 1,
                layout: ChannelLayout::Stereo,
                kind: NodeKind::Source {
                    source: SourceKind::ManagedEngine,
                },
            },
            Node {
                id: 2,
                layout: ChannelLayout::Stereo,
                kind: NodeKind::StereoMixer,
            },
        ];
        let mut effects = vec![];
        let mut edges = vec![Edge {
            id: 1,
            from: 1,
            to: 2,
        }];
        let sink_source = if with_utility {
            nodes.push(Node {
                id: 3,
                layout: ChannelLayout::Stereo,
                kind: NodeKind::Processor { effect_id: 1 },
            });
            effects.push(EffectInstance {
                id: 1,
                kind: EffectKind::Utility,
                version: 1,
                bypass: false,
                parameters: BTreeMap::new(),
                owned_memory_bytes: 0,
            });
            edges.push(Edge {
                id: 2,
                from: 2,
                to: 3,
            });
            3
        } else {
            2
        };
        nodes.push(Node {
            id: 4,
            layout: ChannelLayout::Stereo,
            kind: NodeKind::Sink {
                sink: SinkKind::MainPlayback {
                    ports: StereoPorts {
                        left: "out:1".into(),
                        right: "out:2".into(),
                    },
                },
            },
        });
        edges.push(Edge {
            id: 3,
            from: sink_source,
            to: 4,
        });
        GraphDefinition {
            format_version: 1,
            enabled: true,
            sample_rate: 48_000,
            maximum_callback_frames: 128,
            nodes,
            edges,
            effects,
            source_chains: vec![SourceChain {
                source_node: 1,
                effects: if with_utility { vec![1] } else { vec![] },
            }],
            master_chain: vec![],
            aux_buses: vec![],
            sends: vec![],
            monitoring: Monitoring::default(),
            recording_tap: RecordingTap::PostMaster,
        }
    }

    #[test]
    fn aux_send_and_return_gains_mix_once_and_publish_return_meter() {
        let mut delay_parameters = crate::effect_schema::defaults(EffectKind::Delay);
        delay_parameters.insert("time_ms".into(), 1.0);
        delay_parameters.insert("feedback_percent".into(), 0.0);
        delay_parameters.insert("wet_percent".into(), 100.0);
        delay_parameters.insert("dry_percent".into(), 0.0);
        let graph = GraphDefinition {
            format_version: GRAPH_FORMAT_VERSION,
            enabled: true,
            sample_rate: 48_000,
            maximum_callback_frames: 64,
            nodes: vec![
                Node {
                    id: 1,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::Source {
                        source: SourceKind::ManagedEngine,
                    },
                },
                Node {
                    id: 2,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::SendTap {
                        aux_id: 1,
                        source_node: 1,
                    },
                },
                Node {
                    id: 3,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::Processor { effect_id: 10 },
                },
                Node {
                    id: 4,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::AuxReturn { aux_id: 1 },
                },
                Node {
                    id: 5,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::StereoMixer,
                },
                Node {
                    id: 6,
                    layout: ChannelLayout::Stereo,
                    kind: NodeKind::Sink {
                        sink: SinkKind::MainPlayback {
                            ports: StereoPorts {
                                left: "main:l".into(),
                                right: "main:r".into(),
                            },
                        },
                    },
                },
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: 1,
                    to: 5,
                },
                Edge {
                    id: 2,
                    from: 1,
                    to: 2,
                },
                Edge {
                    id: 3,
                    from: 2,
                    to: 3,
                },
                Edge {
                    id: 4,
                    from: 3,
                    to: 4,
                },
                Edge {
                    id: 5,
                    from: 4,
                    to: 5,
                },
                Edge {
                    id: 6,
                    from: 5,
                    to: 6,
                },
            ],
            effects: vec![EffectInstance {
                id: 10,
                kind: EffectKind::Delay,
                version: EFFECT_FORMAT_VERSION,
                bypass: false,
                parameters: delay_parameters,
                owned_memory_bytes: 0,
            }],
            source_chains: vec![SourceChain {
                source_node: 1,
                effects: vec![],
            }],
            master_chain: vec![],
            aux_buses: vec![AuxBus {
                id: 1,
                effects: vec![10],
                return_gain_db: -6.0206,
            }],
            sends: vec![SendRoute {
                source_node: 1,
                aux_id: 1,
                level_db: -6.0206,
                point: SendPoint::PreInsert,
            }],
            monitoring: Monitoring::default(),
            recording_tap: RecordingTap::PostMaster,
        };
        let mut plan = GraphPlan::compile(&graph).unwrap();
        let return_meter = plan.meter(4).unwrap();
        let source = plan.source_buffer_mut(1, 64).unwrap();
        source.fill(StereoFrame::SILENCE);
        source[0] = StereoFrame::new(1.0, 1.0);
        assert_no_allocations(|| assert_eq!(plan.process(64), ProcessStatus::Complete));
        let output = plan.output_buffer(6, 64).unwrap();
        assert_eq!(output[0], StereoFrame::new(1.0, 1.0));
        assert!((output[48].left - 0.25).abs() < 0.001);
        assert!((return_meter.load().peak.left - 0.25).abs() < 0.001);
    }

    #[test]
    fn dry_single_source_is_bit_identical_and_chunk_invariant() {
        let mut plan = GraphPlan::compile(&graph(false)).unwrap();
        let input = (0..128)
            .map(|index| StereoFrame::new(index as f32 / 128.0, -(index as f32) / 128.0))
            .collect::<Vec<_>>();
        plan.source_buffer_mut(1, 128)
            .unwrap()
            .copy_from_slice(&input);
        assert_eq!(plan.process(128), ProcessStatus::Complete);
        assert_eq!(plan.output_buffer(4, 128).unwrap(), input);
    }

    #[test]
    fn utility_is_smoothed_metered_finite_and_allocation_free() {
        let mut plan = GraphPlan::compile(&graph(true)).unwrap();
        let meter = plan.meter(3).unwrap();
        plan.set_utility(3, -6.0206, 1.0).unwrap();
        assert_no_allocations(|| {
            for _ in 0..100 {
                plan.source_buffer_mut(1, 128)
                    .unwrap()
                    .fill(StereoFrame::new(0.5, 0.5));
                assert_eq!(plan.process(128), ProcessStatus::Complete);
            }
        });
        let output = plan.output_buffer(4, 128).unwrap();
        assert!(output
            .iter()
            .all(|frame| frame.left.abs() < 0.001 && (frame.right - 0.25).abs() < 0.001));
        let snapshot = meter.load();
        assert!(snapshot.peak.right > 0.24 && snapshot.non_finite == 0);
        assert!(plan.set_utility(3, f32::NAN, 0.0).is_err());
    }

    #[test]
    fn oversized_block_and_invalid_effect_schema_are_refused() {
        let mut plan = GraphPlan::compile(&graph(false)).unwrap();
        assert_eq!(plan.process(129), ProcessStatus::OversizedBlock);
        let mut graph = graph(true);
        graph.effects[0].parameters.insert("future".into(), 1.0);
        assert!(GraphPlan::compile(&graph).is_err());
    }

    #[test]
    fn implemented_eq_compiles_into_the_allocation_free_graph_plan() {
        let mut graph = graph(true);
        graph.effects[0].kind = EffectKind::Eq;
        graph.effects[0]
            .parameters
            .insert("low_cut_enabled".into(), 1.0);
        let mut plan = GraphPlan::compile(&graph).unwrap();
        assert_no_allocations(|| {
            plan.source_buffer_mut(1, 128)
                .unwrap()
                .fill(StereoFrame::new(0.5, -0.5));
            assert_eq!(plan.process(128), ProcessStatus::Complete);
        });
        assert!(plan.output_buffer(4, 128).unwrap().iter().all(|frame| {
            frame.left.is_finite() && frame.right.is_finite() && frame.left.abs() <= 0.5
        }));
    }

    #[test]
    fn compatible_instance_id_retains_runtime_and_meter_handles_after_reorder() {
        let first = GraphPlan::compile(&graph(true)).unwrap();
        let meters = first.effect_meters_by_id(1).unwrap();
        let mut reordered = graph(true);
        reordered
            .nodes
            .iter_mut()
            .find(|node| matches!(node.kind, NodeKind::Processor { .. }))
            .unwrap()
            .id = 9;
        reordered.edges[1].to = 9;
        reordered.edges[2].from = 9;
        reordered.effects[0]
            .parameters
            .insert("trim_db".into(), -3.0);

        let mut second = first;
        second.reconfigure(&reordered).unwrap();
        let retained = second.effect_meters_by_id(1).unwrap();
        assert!(Arc::ptr_eq(&meters.input, &retained.input));
        assert!(Arc::ptr_eq(&meters.output, &retained.output));
        assert!(second.effect_meters(9).is_some());
    }

    #[test]
    fn callback_timing_counts_deadlines_and_oversized_blocks_without_allocating() {
        let counters = CallbackTimingCounters::default();
        assert_no_allocations(|| {
            counters.record(128, 48_000, 1_000_000, ProcessStatus::Complete);
            counters.record(128, 48_000, 3_000_000, ProcessStatus::Complete);
            counters.record(8_192, 48_000, 500_000, ProcessStatus::OversizedBlock);
        });
        assert_eq!(
            counters.snapshot(),
            CallbackTimingSnapshot {
                callbacks: 3,
                total_nanoseconds: 4_500_000,
                maximum_nanoseconds: 3_000_000,
                p95_nanoseconds: 3_000_000,
                p99_nanoseconds: 3_000_000,
                missed_deadlines: 1,
                oversized_callbacks: 1,
            }
        );
        assert_eq!(counters.snapshot().mean_nanoseconds(), 1_500_000);
    }

    #[test]
    fn callback_timing_percentiles_are_calculated_from_fixed_buckets() {
        let counters = CallbackTimingCounters::default();
        assert_no_allocations(|| {
            for microseconds in 1..=100 {
                counters.record(128, 48_000, microseconds * 1_000, ProcessStatus::Complete);
            }
        });
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.p95_nanoseconds, 95_000);
        assert_eq!(snapshot.p99_nanoseconds, 99_000);
    }
}
