//! Preallocated callback plan compiled from a validated audio graph.

use crate::audio_graph::{
    EffectKind, GraphDefinition, NodeId, NodeKind, SourceKind, MAX_CALLBACK_FRAMES,
};
use crate::dsp::{db_to_gain, AtomicMeter, MeterAccumulator, SmoothedValue, StereoFrame};
use crate::effect_schema;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const PARAMETER_SMOOTH_SAMPLES: u32 = 64;
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
    Sum,
    Pass,
    Utility(UtilityState),
    Sink,
}

struct RuntimeNode {
    id: NodeId,
    buffer: usize,
    inputs: Box<[usize]>,
    operation: Operation,
}

struct UtilityState {
    left_gain: SmoothedValue,
    right_gain: SmoothedValue,
    meter: MeterAccumulator,
    published: Arc<AtomicMeter>,
}

pub struct GraphPlan {
    maximum_frames: usize,
    buffers: Vec<Box<[StereoFrame]>>,
    nodes: Vec<RuntimeNode>,
    node_buffers: BTreeMap<NodeId, usize>,
    source_nodes: Box<[NodeId]>,
    sink_nodes: Box<[NodeId]>,
}

impl GraphPlan {
    pub fn compile(graph: &GraphDefinition) -> Result<Self, PlanError> {
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
                NodeKind::StereoMixer => Operation::Sum,
                NodeKind::Processor { effect_id } => {
                    let effect = effects
                        .get(effect_id)
                        .ok_or_else(|| PlanError::new("processor effect missing"))?;
                    if effect.kind != EffectKind::Utility {
                        return Err(PlanError::new(
                            "creative effects are not enabled in the dry graph",
                        ));
                    }
                    let trim = effect_schema::parameter(effect, "trim_db")
                        .map_err(|error| PlanError::new(error.to_string()))?;
                    let pan = effect_schema::parameter(effect, "pan")
                        .map_err(|error| PlanError::new(error.to_string()))?;
                    let gain =
                        db_to_gain(trim).map_err(|error| PlanError::new(error.to_string()))?;
                    let (left, right) = stereo_pan_gains(pan);
                    Operation::Utility(UtilityState {
                        left_gain: SmoothedValue::new(gain * left)
                            .map_err(|error| PlanError::new(error.to_string()))?,
                        right_gain: SmoothedValue::new(gain * right)
                            .map_err(|error| PlanError::new(error.to_string()))?,
                        meter: MeterAccumulator::new(maximum_frames)
                            .map_err(|error| PlanError::new(error.to_string()))?,
                        published: Arc::new(AtomicMeter::default()),
                    })
                }
                NodeKind::Sink { .. } => {
                    sink_nodes.push(id);
                    Operation::Sink
                }
                NodeKind::SendTap { .. } | NodeKind::AuxReturn { .. } | NodeKind::MonoToStereo => {
                    Operation::Pass
                }
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
            Operation::Utility(state) if node.id == node_id => Some(Arc::clone(&state.published)),
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
        let gain = db_to_gain(trim_db).map_err(|error| PlanError::new(error.to_string()))?;
        let (left, right) = stereo_pan_gains(pan);
        let state = self
            .nodes
            .iter_mut()
            .find_map(|node| match &mut node.operation {
                Operation::Utility(state) if node.id == node_id => Some(state),
                _ => None,
            })
            .ok_or_else(|| PlanError::new("utility node not found"))?;
        state
            .left_gain
            .set_target(gain * left, PARAMETER_SMOOTH_SAMPLES)
            .map_err(|error| PlanError::new(error.to_string()))?;
        state
            .right_gain
            .set_target(gain * right, PARAMETER_SMOOTH_SAMPLES)
            .map_err(|error| PlanError::new(error.to_string()))?;
        Ok(())
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
                Operation::Source | Operation::Sum | Operation::Pass | Operation::Sink => {}
                Operation::Utility(state) => {
                    for frame in &mut self.buffers[target][..frames] {
                        frame.left *= state.left_gain.next_value();
                        frame.right *= state.right_gain.next_value();
                        *frame = state.meter.process(*frame);
                    }
                    state
                        .published
                        .publish(state.meter.snapshot_and_clear_peak());
                }
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

fn stereo_pan_gains(pan: f32) -> (f32, f32) {
    if pan < 0.0 {
        (1.0, (-pan * std::f32::consts::FRAC_PI_2).cos())
    } else {
        ((pan * std::f32::consts::FRAC_PI_2).cos(), 1.0)
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
    fn oversized_block_and_creative_effect_are_refused() {
        let mut plan = GraphPlan::compile(&graph(false)).unwrap();
        assert_eq!(plan.process(129), ProcessStatus::OversizedBlock);
        let mut graph = graph(true);
        graph.effects[0].kind = EffectKind::Delay;
        assert!(GraphPlan::compile(&graph).is_err());
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
