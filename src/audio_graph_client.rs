//! Disabled-by-default owner for the Phase 1 stereo JACK dry graph.
//!
//! All graph construction and JACK connection changes happen on the owner
//! thread. The callback only copies fixed buffers, runs a preallocated plan,
//! reads atomics, and updates lock-free counters.

use crate::audio_graph::{
    AuxBus, ChannelLayout, Edge, GraphDefinition, InsertRack, Monitoring, Node, NodeKind,
    ProjectAuxRouting, RecordingTap, SendRoute, SinkKind, SourceChain, SourceKind, StereoPorts,
    GRAPH_FORMAT_VERSION,
};
use crate::audio_graph_runtime::{
    CallbackTimingCounters, CallbackTimingSnapshot, GraphPlan, ProcessStatus,
};
use crate::audio_recorder::{FinalMixCapture, FinalMixRecorder, FinalMixRecorderStatus};
use crate::config::{AudioCaptureConfig, AudioGraphConfig};
use crate::dsp::{MeterSnapshot, StereoFrame};
use crate::final_bus::{
    BusControls, BusSource, FinalBusMeterSnapshot, FinalBusMeters, FinalBusProcessor,
};
use crate::jack::{Client as JackClient, Port as JackPort, PortDirection, PortGetBuffer};
use anyhow::{anyhow, bail, Context, Result};
use libc::{c_int, c_uint, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

const SOURCE_NODE: u32 = 1;
const LOOP_SOURCE_NODE: u32 = 2;
const INPUT_SOURCE_NODE: u32 = 3;
const FIRST_EFFECT_NODE: u32 = 10;
const FIRST_SEND_NODE: u32 = 30;
const FIRST_AUX_EFFECT_NODE: u32 = 40;
const FIRST_AUX_RETURN_NODE: u32 = 70;
const FIRST_MASTER_EFFECT_NODE: u32 = 80;
const MASTER_NODE: u32 = 90;
const SINK_NODE: u32 = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Connection {
    source: String,
    destination: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChangeKind {
    Connect,
    Disconnect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundaryChange {
    kind: ChangeKind,
    connection: Connection,
}

trait BoundaryConnections {
    /// Return true only when the requested operation changed graph state.
    fn connect(&mut self, connection: &Connection) -> Result<bool>;
    fn disconnect(&mut self, connection: &Connection) -> Result<bool>;
}

impl BoundaryConnections for JackClient {
    fn connect(&mut self, connection: &Connection) -> Result<bool> {
        self.ensure_connection(&connection.source, &connection.destination)
    }

    fn disconnect(&mut self, connection: &Connection) -> Result<bool> {
        self.remove_connection(&connection.source, &connection.destination)
    }
}

fn apply_transaction(
    connections: &mut impl BoundaryConnections,
    changes: &[BoundaryChange],
) -> Result<()> {
    let mut applied = Vec::with_capacity(changes.len());
    for change in changes {
        let result = match change.kind {
            ChangeKind::Connect => connections.connect(&change.connection),
            ChangeKind::Disconnect => connections.disconnect(&change.connection),
        };
        match result {
            Ok(true) => applied.push(change.clone()),
            Ok(false) => {}
            Err(error) => {
                let rollback_error = rollback(connections, &applied).err();
                return match rollback_error {
                    Some(rollback) => Err(anyhow!(
                        "audio boundary change failed: {error:#}; rollback failed: {rollback:#}"
                    )),
                    None => Err(error.context("audio boundary change rolled back")),
                };
            }
        }
    }
    Ok(())
}

fn rollback(connections: &mut impl BoundaryConnections, applied: &[BoundaryChange]) -> Result<()> {
    let mut first_error = None;
    for change in applied.iter().rev() {
        let result = match change.kind {
            ChangeKind::Connect => connections.disconnect(&change.connection),
            ChangeKind::Disconnect => connections.connect(&change.connection),
        };
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

struct BoundaryRoutes {
    direct: Vec<Connection>,
    graph: Vec<Connection>,
}

impl BoundaryRoutes {
    fn direct_connection_changes(&self) -> Vec<BoundaryChange> {
        self.direct
            .iter()
            .cloned()
            .map(|connection| BoundaryChange {
                kind: ChangeKind::Connect,
                connection,
            })
            .collect()
    }

    fn activation_changes(&self) -> Vec<BoundaryChange> {
        self.graph
            .iter()
            .cloned()
            .map(|connection| BoundaryChange {
                kind: ChangeKind::Connect,
                connection,
            })
            .chain(
                self.direct
                    .iter()
                    .cloned()
                    .map(|connection| BoundaryChange {
                        kind: ChangeKind::Disconnect,
                        connection,
                    }),
            )
            .collect()
    }
}

struct CallbackData {
    plan: GraphPlan,
    inputs: [*mut JackPort; 6],
    input_port_ids: [u32; 6],
    output_left: *mut JackPort,
    output_right: *mut JackPort,
    port_get_buffer: PortGetBuffer,
    sample_rate: u32,
    armed: AtomicBool,
    client_lost: AtomicBool,
    source_lost: AtomicBool,
    timing: CallbackTimingCounters,
    final_bus: FinalBusProcessor,
    final_capture: FinalMixCapture,
    final_buffer: Box<[StereoFrame]>,
}

// JACK owns callback scheduling, while the box itself remains pinned and is
// reclaimed only after deactivation on the non-real-time owner thread.
unsafe impl Send for CallbackData {}

pub(crate) struct OwnedAudioGraph {
    jack: JackClient,
    callback: Box<CallbackData>,
    routes: BoundaryRoutes,
    aux_return_nodes: [Option<(u8, u32)>; 2],
    controls: std::sync::Arc<BusControls>,
    meters: std::sync::Arc<FinalBusMeters>,
    final_recorder: FinalMixRecorder,
    monitoring: Monitoring,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct EffectMeterSnapshot {
    pub input: MeterSnapshot,
    pub output: MeterSnapshot,
    pub gain_reduction_db: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AuxMeterSnapshot {
    pub output: MeterSnapshot,
}

pub(crate) struct PerformanceBusPorts {
    pub synth: [String; 2],
    pub loop_player: [String; 2],
    pub live_input: [String; 2],
    pub playback: [String; 2],
    pub loop_direct_playback: [String; 2],
}

impl OwnedAudioGraph {
    pub(crate) fn sample_rate(&self) -> u32 {
        self.callback.sample_rate
    }

    pub(crate) fn start_with_routing(
        config: &AudioGraphConfig,
        ports: PerformanceBusPorts,
        recording: &AudioCaptureConfig,
        rack: &InsertRack,
        aux_routing: &ProjectAuxRouting,
    ) -> Result<Self> {
        let PerformanceBusPorts {
            synth: source_ports,
            loop_player: loop_source_ports,
            live_input: live_source_ports,
            playback: destinations,
            loop_direct_playback: loop_destinations,
        } = ports;
        validate_stereo_boundary(&source_ports, "managed-engine source")?;
        validate_stereo_boundary(&loop_source_ports, "owned WAV loop source")?;
        validate_stereo_boundary(&live_source_ports, "configured stereo input")?;
        validate_stereo_boundary(&destinations, "main output")?;
        validate_stereo_boundary(&loop_destinations, "loop direct output")?;
        if config.input_direct_monitoring && !config.confirm_doubled_monitoring {
            bail!("configured stereo input has interface direct monitoring enabled; confirm the deliberate doubled monitoring path or disable direct monitoring before software monitoring");
        }

        let mut jack = JackClient::open(&config.client_name).context("open owned audio graph")?;
        let sample_rate = jack.sample_rate();
        if sample_rate == 0 {
            bail!("JACK reported a zero sample rate");
        }
        let monitoring = Monitoring {
            direct: config.input_direct_monitoring,
            software: true,
            doubled_path_confirmed: config.confirm_doubled_monitoring,
        };
        let definition = managed_graph_definition(
            sample_rate,
            config.maximum_callback_frames,
            &destinations,
            &live_source_ports,
            monitoring,
            rack,
            aux_routing,
        );
        let plan = GraphPlan::compile(&definition).context("compile managed audio graph")?;

        let inputs = [
            jack.register_audio_port("managed_in_l", PortDirection::Input)?,
            jack.register_audio_port("managed_in_r", PortDirection::Input)?,
            jack.register_audio_port("loop_in_l", PortDirection::Input)?,
            jack.register_audio_port("loop_in_r", PortDirection::Input)?,
            jack.register_audio_port("stereo_in_l", PortDirection::Input)?,
            jack.register_audio_port("stereo_in_r", PortDirection::Input)?,
        ];
        let input_port_ids = [
            jack.port_id(inputs[0])?,
            jack.port_id(inputs[1])?,
            jack.port_id(inputs[2])?,
            jack.port_id(inputs[3])?,
            jack.port_id(inputs[4])?,
            jack.port_id(inputs[5])?,
        ];
        let output_left = jack.register_audio_port("main_out_l", PortDirection::Output)?;
        let output_right = jack.register_audio_port("main_out_r", PortDirection::Output)?;
        let graph_port_names = [
            jack.port_name_string(inputs[0])?,
            jack.port_name_string(inputs[1])?,
            jack.port_name_string(inputs[2])?,
            jack.port_name_string(inputs[3])?,
            jack.port_name_string(inputs[4])?,
            jack.port_name_string(inputs[5])?,
            jack.port_name_string(output_left)?,
            jack.port_name_string(output_right)?,
        ];
        let routes = BoundaryRoutes {
            direct: vec![
                connection(&source_ports[0], &destinations[0]),
                connection(&source_ports[1], &destinations[1]),
                connection(&loop_source_ports[0], &loop_destinations[0]),
                connection(&loop_source_ports[1], &loop_destinations[1]),
            ],
            graph: vec![
                connection(&source_ports[0], &graph_port_names[0]),
                connection(&source_ports[1], &graph_port_names[1]),
                connection(&loop_source_ports[0], &graph_port_names[2]),
                connection(&loop_source_ports[1], &graph_port_names[3]),
                connection(&live_source_ports[0], &graph_port_names[4]),
                connection(&live_source_ports[1], &graph_port_names[5]),
                connection(&graph_port_names[6], &destinations[0]),
                connection(&graph_port_names[7], &destinations[1]),
            ],
        };
        let controls = std::sync::Arc::new(BusControls::default());
        let meters = std::sync::Arc::new(FinalBusMeters::default());
        let final_bus = FinalBusProcessor::new(
            sample_rate,
            config.maximum_callback_frames as usize,
            std::sync::Arc::clone(&controls),
            std::sync::Arc::clone(&meters),
        )
        .map_err(anyhow::Error::msg)
        .context("prepare final performance bus")?;
        let final_recorder = FinalMixRecorder::new(
            recording.directory.clone(),
            sample_rate,
            recording.ring_frames,
            config.maximum_callback_frames as usize,
        )?;
        let final_capture = final_recorder.capture_handle();
        let mut callback = Box::new(CallbackData {
            plan,
            inputs,
            input_port_ids,
            output_left,
            output_right,
            port_get_buffer: jack.port_get_buffer(),
            sample_rate,
            armed: AtomicBool::new(false),
            client_lost: AtomicBool::new(false),
            source_lost: AtomicBool::new(false),
            timing: CallbackTimingCounters::default(),
            final_bus,
            final_capture,
            final_buffer: vec![StereoFrame::SILENCE; config.maximum_callback_frames as usize]
                .into_boxed_slice(),
        });
        let callback_pointer = ((&mut *callback) as *mut CallbackData).cast();
        // SAFETY: callback remains boxed until after explicit JACK deactivation.
        unsafe {
            jack.set_process_callback(process_callback, callback_pointer)?;
            jack.set_shutdown_callback(shutdown_callback, callback_pointer);
            jack.set_xrun_callback(xrun_callback, callback_pointer)?;
            jack.set_port_connect_callback(port_connect_callback, callback_pointer)?;
        }
        jack.activate().context("activate owned audio graph")?;
        // Re-establish the conservative route through JACK's checked API even
        // if the legacy jack_connect helper was unavailable or raced startup.
        if let Err(error) = apply_transaction(&mut jack, &routes.direct_connection_changes()) {
            jack.deactivate();
            return Err(error.context("establish direct fallback before graph routing"));
        }
        if let Err(error) = apply_transaction(&mut jack, &routes.activation_changes()) {
            jack.deactivate();
            return Err(error.context("activate owned graph boundary"));
        }
        // The callback samples this once per block. All graph connections are
        // ready and all four synth/loop direct links are gone before output is
        // published.
        callback.armed.store(true, Ordering::Release);
        Ok(Self {
            jack,
            callback,
            routes,
            aux_return_nodes: aux_return_nodes(aux_routing),
            controls,
            meters,
            final_recorder,
            monitoring,
        })
    }

    pub(crate) fn client_lost(&self) -> bool {
        self.callback.client_lost.load(Ordering::Acquire)
            || self.callback.source_lost.load(Ordering::Acquire)
    }

    pub(crate) fn source_lost(&self) -> bool {
        self.callback.source_lost.load(Ordering::Acquire)
    }

    pub(crate) fn timing(&self) -> CallbackTimingSnapshot {
        self.callback.timing.snapshot()
    }

    pub(crate) fn effect_meter(&self, effect_id: u32) -> Option<EffectMeterSnapshot> {
        let handles = self.callback.plan.effect_meters_by_id(effect_id)?;
        Some(EffectMeterSnapshot {
            input: handles.input.load(),
            output: handles.output.load(),
            gain_reduction_db: handles.gain_reduction.map(|meter| meter.load()),
        })
    }

    pub(crate) fn aux_meter(&self, aux_id: u8) -> Option<AuxMeterSnapshot> {
        let node = self
            .aux_return_nodes
            .iter()
            .flatten()
            .find_map(|(id, node)| (*id == aux_id).then_some(*node))?;
        Some(AuxMeterSnapshot {
            output: self.callback.plan.meter(node)?.load(),
        })
    }

    pub(crate) fn master_meter(&self) -> Option<AuxMeterSnapshot> {
        Some(AuxMeterSnapshot {
            output: self.meters.snapshot().output,
        })
    }

    pub(crate) fn final_bus_meter(&self) -> FinalBusMeterSnapshot {
        self.meters.snapshot()
    }

    pub(crate) fn bus_controls(&self) -> std::sync::Arc<BusControls> {
        std::sync::Arc::clone(&self.controls)
    }

    pub(crate) fn final_recording_status(&mut self) -> FinalMixRecorderStatus {
        self.final_recorder.status()
    }

    pub(crate) fn final_recording_active(&self) -> bool {
        self.final_recorder.is_recording()
    }

    pub(crate) fn start_final_recording(&mut self, name: Option<&str>) -> Result<()> {
        self.final_recorder.start(name)
    }

    pub(crate) fn stop_final_recording(&mut self) -> Result<()> {
        self.final_recorder.request_stop();
        self.final_recorder.finish_stop()
    }

    /// Publish a validated structural rack change while transport and all
    /// recording are stopped. JACK callback execution is joined before the
    /// plan is mutated; compatible effect IDs retain their runtime state.
    pub(crate) fn publish_routing(
        &mut self,
        rack: &InsertRack,
        aux_routing: &ProjectAuxRouting,
    ) -> Result<()> {
        let destinations = [
            self.routes.graph[6].destination.clone(),
            self.routes.graph[7].destination.clone(),
        ];
        let definition = managed_graph_definition(
            self.callback.sample_rate,
            self.callback.plan.maximum_frames() as u32,
            &destinations,
            &[
                self.routes.graph[4].source.clone(),
                self.routes.graph[5].source.clone(),
            ],
            self.monitoring,
            rack,
            aux_routing,
        );
        definition
            .validate()
            .map_err(|error| anyhow!(error.to_string()))?;
        self.callback.armed.store(false, Ordering::Release);
        self.jack.deactivate();
        if let Err(error) = self.callback.plan.reconfigure(&definition) {
            if self.jack.activate().is_ok() {
                self.callback.armed.store(true, Ordering::Release);
            } else {
                let _ = self.restore_direct();
            }
            return Err(anyhow!(error.to_string()).context("compile replacement audio rack"));
        }
        self.callback.final_bus.reset();
        if let Err(error) = self.jack.activate() {
            let _ = self.restore_direct();
            return Err(error.context("reactivate audio graph after rack publication"));
        }
        if let Err(error) = apply_transaction(&mut self.jack, &self.routes.activation_changes()) {
            let _ = self.restore_direct();
            return Err(error.context("restore audio graph boundary after rack publication"));
        }
        self.callback.armed.store(true, Ordering::Release);
        self.aux_return_nodes = aux_return_nodes(aux_routing);
        Ok(())
    }

    /// Restore all four exact synth/loop direct links best-effort. This runs only on the
    /// non-real-time owner thread, including client-loss recovery.
    pub(crate) fn restore_direct(&mut self) -> Result<()> {
        self.callback.armed.store(false, Ordering::Release);
        // Join the callback before creating either direct link. A callback
        // that sampled the previous publish flag can therefore never overlap
        // the restored dry path for even one block.
        self.jack.deactivate();
        let recorder_result = self.final_recorder.stop_after_deactivate();
        let mut first_error = None;
        for connection in &self.routes.direct {
            if let Err(error) = self
                .jack
                .ensure_connection(&connection.source, &connection.destination)
            {
                first_error.get_or_insert(error);
            }
        }
        if let Err(error) = recorder_result {
            first_error.get_or_insert(error);
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn aux_return_nodes(routing: &ProjectAuxRouting) -> [Option<(u8, u32)>; 2] {
    let mut nodes = [None, None];
    for (index, bus) in routing.buses.iter().take(2).enumerate() {
        nodes[index] = Some((bus.id, FIRST_AUX_RETURN_NODE + index as u32));
    }
    nodes
}

impl Drop for OwnedAudioGraph {
    fn drop(&mut self) {
        let _ = self.restore_direct();
        // `callback` is still alive here and is dropped only after this method.
    }
}

fn connection(source: &str, destination: &str) -> Connection {
    Connection {
        source: source.into(),
        destination: destination.into(),
    }
}

fn validate_stereo_boundary(ports: &[String; 2], description: &str) -> Result<()> {
    if ports.iter().any(|port| port.trim().is_empty()) {
        bail!("{description} contains an empty JACK port name");
    }
    if ports[0] == ports[1] {
        bail!("{description} JACK ports are ambiguous");
    }
    Ok(())
}

fn managed_graph_definition(
    sample_rate: u32,
    maximum_callback_frames: u32,
    destinations: &[String; 2],
    live_source_ports: &[String; 2],
    monitoring: Monitoring,
    rack: &InsertRack,
    aux_routing: &ProjectAuxRouting,
) -> GraphDefinition {
    let mut nodes = vec![
        Node {
            id: SOURCE_NODE,
            layout: ChannelLayout::Stereo,
            kind: NodeKind::Source {
                source: SourceKind::ManagedEngine,
            },
        },
        Node {
            id: LOOP_SOURCE_NODE,
            layout: ChannelLayout::Stereo,
            kind: NodeKind::Source {
                source: SourceKind::LoopPlayer,
            },
        },
        Node {
            id: INPUT_SOURCE_NODE,
            layout: ChannelLayout::Stereo,
            kind: NodeKind::Source {
                source: SourceKind::LiveInput {
                    ports: StereoPorts {
                        left: live_source_ports[0].clone(),
                        right: live_source_ports[1].clone(),
                    },
                },
            },
        },
    ];
    let mut edges = Vec::new();
    let mut previous = SOURCE_NODE;
    for (index, effect_id) in rack.order.iter().copied().enumerate() {
        let node_id = FIRST_EFFECT_NODE + index as u32;
        nodes.push(Node {
            id: node_id,
            layout: ChannelLayout::Stereo,
            kind: NodeKind::Processor { effect_id },
        });
        edges.push(Edge {
            id: edges.len() as u32 + 1,
            from: previous,
            to: node_id,
        });
        previous = node_id;
    }
    nodes.push(Node {
        id: MASTER_NODE,
        layout: ChannelLayout::Stereo,
        kind: NodeKind::StereoMixer,
    });
    edges.push(Edge {
        id: edges.len() as u32 + 1,
        from: previous,
        to: MASTER_NODE,
    });
    for source in [LOOP_SOURCE_NODE, INPUT_SOURCE_NODE] {
        edges.push(Edge {
            id: edges.len() as u32 + 1,
            from: source,
            to: MASTER_NODE,
        });
    }

    let mut effects = rack.effects.clone();
    let mut aux_buses = Vec::new();
    let mut sends = Vec::new();
    let mut aux_effect_node = FIRST_AUX_EFFECT_NODE;
    for (bus_index, bus) in aux_routing.buses.iter().enumerate() {
        effects.extend(bus.rack.effects.iter().cloned());
        aux_buses.push(AuxBus {
            id: bus.id,
            effects: bus.rack.order.clone(),
            return_gain_db: bus.return_gain_db,
        });
        let send = aux_routing.sends.iter().find(|send| send.aux_id == bus.id);
        let mut aux_previous = None;
        if let Some(send) = send {
            let send_node = FIRST_SEND_NODE + bus_index as u32;
            nodes.push(Node {
                id: send_node,
                layout: ChannelLayout::Stereo,
                kind: NodeKind::SendTap {
                    aux_id: bus.id,
                    source_node: SOURCE_NODE,
                },
            });
            edges.push(Edge {
                id: edges.len() as u32 + 1,
                from: match send.point {
                    crate::audio_graph::SendPoint::PreInsert => SOURCE_NODE,
                    crate::audio_graph::SendPoint::PostInsert => previous,
                },
                to: send_node,
            });
            sends.push(SendRoute {
                source_node: SOURCE_NODE,
                aux_id: bus.id,
                level_db: send.level_db,
                point: send.point,
            });
            aux_previous = Some(send_node);
        }
        for effect_id in bus.rack.order.iter().copied() {
            let node_id = aux_effect_node;
            aux_effect_node += 1;
            nodes.push(Node {
                id: node_id,
                layout: ChannelLayout::Stereo,
                kind: NodeKind::Processor { effect_id },
            });
            if let Some(from) = aux_previous {
                edges.push(Edge {
                    id: edges.len() as u32 + 1,
                    from,
                    to: node_id,
                });
            }
            aux_previous = Some(node_id);
        }
        let return_node = FIRST_AUX_RETURN_NODE + bus_index as u32;
        nodes.push(Node {
            id: return_node,
            layout: ChannelLayout::Stereo,
            kind: NodeKind::AuxReturn { aux_id: bus.id },
        });
        if let Some(from) = aux_previous {
            edges.push(Edge {
                id: edges.len() as u32 + 1,
                from,
                to: return_node,
            });
        }
        edges.push(Edge {
            id: edges.len() as u32 + 1,
            from: return_node,
            to: MASTER_NODE,
        });
    }

    let mut master_previous = MASTER_NODE;
    for (index, effect_id) in aux_routing.master_rack.order.iter().copied().enumerate() {
        let node_id = FIRST_MASTER_EFFECT_NODE + index as u32;
        nodes.push(Node {
            id: node_id,
            layout: ChannelLayout::Stereo,
            kind: NodeKind::Processor { effect_id },
        });
        edges.push(Edge {
            id: edges.len() as u32 + 1,
            from: master_previous,
            to: node_id,
        });
        master_previous = node_id;
    }
    effects.extend(aux_routing.master_rack.effects.iter().cloned());

    nodes.push(Node {
        id: SINK_NODE,
        layout: ChannelLayout::Stereo,
        kind: NodeKind::Sink {
            sink: SinkKind::MainPlayback {
                ports: StereoPorts {
                    left: destinations[0].clone(),
                    right: destinations[1].clone(),
                },
            },
        },
    });
    edges.push(Edge {
        id: edges.len() as u32 + 1,
        from: master_previous,
        to: SINK_NODE,
    });
    GraphDefinition {
        format_version: GRAPH_FORMAT_VERSION,
        enabled: true,
        sample_rate,
        maximum_callback_frames,
        nodes,
        edges,
        effects,
        source_chains: vec![SourceChain {
            source_node: SOURCE_NODE,
            effects: rack.order.clone(),
        }],
        master_chain: aux_routing.master_rack.order.clone(),
        aux_buses,
        sends,
        monitoring,
        recording_tap: RecordingTap::PostMaster,
    }
}

#[cfg(test)]
fn dry_graph_definition(
    sample_rate: u32,
    maximum_callback_frames: u32,
    destinations: &[String; 2],
) -> GraphDefinition {
    managed_graph_definition(
        sample_rate,
        maximum_callback_frames,
        destinations,
        &["input:l".into(), "input:r".into()],
        Monitoring {
            direct: false,
            software: true,
            doubled_path_confirmed: false,
        },
        &InsertRack::default(),
        &ProjectAuxRouting::default(),
    )
}

fn process_block(
    callback: &mut CallbackData,
    frames: usize,
    inputs: [&[f32]; 6],
    output_left: &mut [f32],
    output_right: &mut [f32],
) -> ProcessStatus {
    let publish = callback.armed.load(Ordering::Acquire);
    if frames > callback.plan.maximum_frames()
        || inputs.iter().any(|input| input.len() < frames)
        || output_left.len() < frames
        || output_right.len() < frames
    {
        callback.final_capture.callback_violation();
        output_left.fill(0.0);
        output_right.fill(0.0);
        return ProcessStatus::OversizedBlock;
    }
    for (node, source_kind, left, right) in [
        (SOURCE_NODE, BusSource::Synth, 0, 1),
        (LOOP_SOURCE_NODE, BusSource::Loop, 2, 3),
        (INPUT_SOURCE_NODE, BusSource::Input, 4, 5),
    ] {
        let Some(source) = callback.plan.source_buffer_mut(node, frames) else {
            callback.final_capture.callback_violation();
            output_left[..frames].fill(0.0);
            output_right[..frames].fill(0.0);
            return ProcessStatus::OversizedBlock;
        };
        for ((frame, &left_sample), &right_sample) in source
            .iter_mut()
            .zip(inputs[left].iter())
            .zip(inputs[right].iter())
            .take(frames)
        {
            *frame = StereoFrame::new(left_sample, right_sample);
        }
        callback.final_bus.process_source(source_kind, source);
    }
    let status = callback.plan.process(frames);
    if !publish || !matches!(status, ProcessStatus::Complete) {
        if publish && !matches!(status, ProcessStatus::Complete) {
            callback.final_capture.callback_violation();
        }
        output_left[..frames].fill(0.0);
        output_right[..frames].fill(0.0);
        return status;
    }
    let Some(output) = callback.plan.output_buffer(SINK_NODE, frames) else {
        callback.final_capture.callback_violation();
        output_left[..frames].fill(0.0);
        output_right[..frames].fill(0.0);
        return ProcessStatus::OversizedBlock;
    };
    callback.final_buffer[..frames].copy_from_slice(output);
    callback
        .final_bus
        .process_final(&mut callback.final_buffer[..frames]);
    callback
        .final_capture
        .capture(&callback.final_buffer[..frames]);
    for index in 0..frames {
        output_left[index] = callback.final_buffer[index].left;
        output_right[index] = callback.final_buffer[index].right;
    }
    status
}

unsafe extern "C" fn process_callback(frames: c_uint, argument: *mut c_void) -> c_int {
    if argument.is_null() {
        return 0;
    }
    // SAFETY: OwnedAudioGraph pins CallbackData until JACK is inactive.
    let callback = unsafe { &mut *argument.cast::<CallbackData>() };
    let start = monotonic_nanoseconds();
    let get_buffer = callback.port_get_buffer;
    let mut input_pointers = [std::ptr::null_mut(); 6];
    for (pointer, port) in input_pointers.iter_mut().zip(callback.inputs) {
        *pointer = unsafe { get_buffer(port, frames) }.cast::<f32>();
    }
    let output_left = unsafe { get_buffer(callback.output_left, frames) }.cast::<f32>();
    let output_right = unsafe { get_buffer(callback.output_right, frames) }.cast::<f32>();
    if input_pointers.iter().any(|pointer| pointer.is_null())
        || output_left.is_null()
        || output_right.is_null()
    {
        callback.final_capture.invalid_buffer();
        return 0;
    }
    let frame_count = frames as usize;
    // SAFETY: JACK provides exactly `frames` f32 samples for each audio port.
    let inputs =
        input_pointers.map(|pointer| unsafe { std::slice::from_raw_parts(pointer, frame_count) });
    let output_left = unsafe { std::slice::from_raw_parts_mut(output_left, frame_count) };
    let output_right = unsafe { std::slice::from_raw_parts_mut(output_right, frame_count) };
    let status = process_block(callback, frame_count, inputs, output_left, output_right);
    let end = monotonic_nanoseconds();
    let elapsed = if start == 0 || end == 0 {
        0
    } else {
        end.saturating_sub(start)
    };
    callback
        .timing
        .record(frames, callback.sample_rate, elapsed, status);
    0
}

unsafe extern "C" fn shutdown_callback(argument: *mut c_void) {
    if !argument.is_null() {
        // SAFETY: OwnedAudioGraph pins CallbackData until client close.
        let callback = unsafe { &*argument.cast::<CallbackData>() };
        callback.client_lost.store(true, Ordering::Release);
        callback.final_capture.jack_shutdown();
    }
}

unsafe extern "C" fn xrun_callback(argument: *mut c_void) -> c_int {
    if !argument.is_null() {
        unsafe { &*argument.cast::<CallbackData>() }
            .final_capture
            .xrun();
    }
    0
}

unsafe extern "C" fn port_connect_callback(
    first: c_uint,
    second: c_uint,
    connected: c_int,
    argument: *mut c_void,
) {
    if connected != 0 || argument.is_null() {
        return;
    }
    let callback = unsafe { &*argument.cast::<CallbackData>() };
    if callback.armed.load(Ordering::Acquire)
        && callback
            .input_port_ids
            .iter()
            .any(|port| *port == first || *port == second)
    {
        callback.source_lost.store(true, Ordering::Release);
        callback.final_capture.source_lost();
    }
}

fn monotonic_nanoseconds() -> u64 {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: clock_gettime initializes the timespec on success.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, time.as_mut_ptr()) } != 0 {
        return 0;
    }
    // SAFETY: the successful call above initialized `time`.
    let time = unsafe { time.assume_init() };
    (time.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.tv_nsec as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::allocation_test::assert_no_allocations;
    use std::collections::BTreeSet;

    #[derive(Default)]
    struct MockConnections {
        connected: BTreeSet<(String, String)>,
        operations: usize,
        fail_at: Option<usize>,
    }

    impl BoundaryConnections for MockConnections {
        fn connect(&mut self, connection: &Connection) -> Result<bool> {
            self.change(connection, true)
        }

        fn disconnect(&mut self, connection: &Connection) -> Result<bool> {
            self.change(connection, false)
        }
    }

    impl MockConnections {
        fn change(&mut self, connection: &Connection, connect: bool) -> Result<bool> {
            self.operations += 1;
            if self.fail_at == Some(self.operations) {
                bail!("injected connection failure");
            }
            let pair = (connection.source.clone(), connection.destination.clone());
            Ok(if connect {
                self.connected.insert(pair)
            } else {
                self.connected.remove(&pair)
            })
        }
    }

    fn routes() -> BoundaryRoutes {
        BoundaryRoutes {
            direct: vec![
                connection("engine:l", "main:l"),
                connection("engine:r", "main:r"),
                connection("loop:l", "loop-playback:l"),
                connection("loop:r", "loop-playback:r"),
            ],
            graph: vec![
                connection("engine:l", "graph:in_l"),
                connection("engine:r", "graph:in_r"),
                connection("loop:l", "graph:loop_l"),
                connection("loop:r", "graph:loop_r"),
                connection("capture:l", "graph:input_l"),
                connection("capture:r", "graph:input_r"),
                connection("graph:out_l", "main:l"),
                connection("graph:out_r", "main:r"),
            ],
        }
    }

    fn test_monitoring() -> Monitoring {
        Monitoring {
            direct: false,
            software: true,
            doubled_path_confirmed: false,
        }
    }

    fn test_live_ports() -> [String; 2] {
        ["capture:l".into(), "capture:r".into()]
    }

    fn callback(maximum_frames: u32) -> CallbackData {
        let destinations = ["main:l".to_owned(), "main:r".to_owned()];
        let controls = std::sync::Arc::new(BusControls::default());
        for source in BusSource::ALL {
            assert!(controls.set_source_gain_db(source, 0.0));
        }
        let meters = std::sync::Arc::new(FinalBusMeters::default());
        let recorder =
            FinalMixRecorder::new(std::env::temp_dir(), 48_000, 4096, maximum_frames as usize)
                .unwrap();
        CallbackData {
            plan: GraphPlan::compile(&dry_graph_definition(48_000, maximum_frames, &destinations))
                .unwrap(),
            inputs: [std::ptr::null_mut(); 6],
            input_port_ids: [0; 6],
            output_left: std::ptr::null_mut(),
            output_right: std::ptr::null_mut(),
            port_get_buffer: dummy_get_buffer,
            sample_rate: 48_000,
            armed: AtomicBool::new(false),
            client_lost: AtomicBool::new(false),
            source_lost: AtomicBool::new(false),
            timing: CallbackTimingCounters::default(),
            final_bus: FinalBusProcessor::new(48_000, maximum_frames as usize, controls, meters)
                .unwrap(),
            final_capture: recorder.capture_handle(),
            final_buffer: vec![StereoFrame::SILENCE; maximum_frames as usize].into_boxed_slice(),
        }
    }

    unsafe extern "C" fn dummy_get_buffer(_: *mut JackPort, _: c_uint) -> *mut c_void {
        std::ptr::null_mut()
    }

    #[test]
    fn dry_topology_is_valid_and_contains_exactly_three_sources() {
        let destinations = ["main:l".to_owned(), "main:r".to_owned()];
        let graph = dry_graph_definition(48_000, 128, &destinations);
        assert_eq!(
            graph.validate().unwrap(),
            [
                SOURCE_NODE,
                LOOP_SOURCE_NODE,
                INPUT_SOURCE_NODE,
                MASTER_NODE,
                SINK_NODE
            ]
        );
        assert_eq!(graph.nodes.len(), 5);
        assert_eq!(graph.edges.len(), 4);
        assert!(graph.effects.is_empty());
    }

    #[test]
    fn graph_sums_three_distinguishable_stereo_sources_exactly_once() {
        let destinations = ["main:l".to_owned(), "main:r".to_owned()];
        let graph = dry_graph_definition(48_000, 64, &destinations);
        let mut plan = GraphPlan::compile(&graph).unwrap();
        for (node, left, right) in [
            (SOURCE_NODE, 0.01, 0.02),
            (LOOP_SOURCE_NODE, 0.04, 0.08),
            (INPUT_SOURCE_NODE, 0.16, 0.32),
        ] {
            plan.source_buffer_mut(node, 64)
                .unwrap()
                .fill(StereoFrame::new(left, right));
        }
        assert_eq!(plan.process(64), ProcessStatus::Complete);
        assert!(plan
            .output_buffer(SINK_NODE, 64)
            .unwrap()
            .iter()
            .all(|frame| {
                (frame.left - 0.21).abs() < 1e-7 && (frame.right - 0.42).abs() < 1e-7
            }));
    }

    #[test]
    fn managed_rack_builds_one_ordered_source_path() {
        let destinations = ["main:l".to_owned(), "main:r".to_owned()];
        let mut rack = InsertRack::default();
        let compressor = rack
            .add(crate::audio_graph::EffectKind::Compressor)
            .unwrap();
        let eq = rack.add(crate::audio_graph::EffectKind::Eq).unwrap();
        rack.move_to(eq, 0).unwrap();
        let graph = managed_graph_definition(
            48_000,
            128,
            &destinations,
            &test_live_ports(),
            test_monitoring(),
            &rack,
            &ProjectAuxRouting::default(),
        );
        assert_eq!(
            graph.validate().unwrap(),
            [
                SOURCE_NODE,
                LOOP_SOURCE_NODE,
                INPUT_SOURCE_NODE,
                FIRST_EFFECT_NODE,
                FIRST_EFFECT_NODE + 1,
                MASTER_NODE,
                SINK_NODE
            ]
        );
        assert_eq!(graph.source_chains[0].effects, [eq, compressor]);
        assert_eq!(graph.edges.len(), 6);
    }

    #[test]
    fn master_chain_follows_level_change_and_empty_master_identity() {
        let destinations = ["main:l".to_owned(), "main:r".to_owned()];
        for (routing, expected) in [
            (ProjectAuxRouting::default(), 0.5_f32),
            (
                {
                    let mut routing = ProjectAuxRouting::default();
                    routing
                        .master_rack
                        .add_with_id(crate::audio_graph::EffectKind::Utility, 1)
                        .unwrap();
                    routing
                        .master_rack
                        .effect_mut(1)
                        .unwrap()
                        .parameters
                        .insert("trim_db".into(), -6.0206);
                    routing
                },
                0.25_f32,
            ),
        ] {
            let graph = managed_graph_definition(
                48_000,
                64,
                &destinations,
                &test_live_ports(),
                test_monitoring(),
                &InsertRack::default(),
                &routing,
            );
            let mut plan = GraphPlan::compile(&graph).unwrap();
            plan.source_buffer_mut(SOURCE_NODE, 64)
                .unwrap()
                .fill(StereoFrame::new(0.5, -0.5));
            assert_eq!(plan.process(64), ProcessStatus::Complete);
            let output = plan.output_buffer(SINK_NODE, 64).unwrap();
            assert!(output.iter().all(|frame| {
                (frame.left - expected).abs() < 0.0001 && (frame.right + expected).abs() < 0.0001
            }));
        }
    }

    #[test]
    fn managed_aux_builds_one_scaled_pre_or_post_send_and_one_wet_return() {
        let destinations = ["main:l".to_owned(), "main:r".to_owned()];
        let mut rack = InsertRack::default();
        rack.add(crate::audio_graph::EffectKind::Eq).unwrap();
        let mut routing = ProjectAuxRouting::default();
        let aux = routing.add_bus().unwrap();
        let reverb = routing
            .add_effect(&rack, aux, crate::audio_graph::EffectKind::Reverb)
            .unwrap();
        let master = routing.next_effect_id(&rack).unwrap();
        routing
            .master_rack
            .add_with_id(crate::audio_graph::EffectKind::Compressor, master)
            .unwrap();
        routing
            .set_send(&rack, aux, -18.0, crate::audio_graph::SendPoint::PostInsert)
            .unwrap();
        let graph = managed_graph_definition(
            48_000,
            128,
            &destinations,
            &test_live_ports(),
            test_monitoring(),
            &rack,
            &routing,
        );
        graph.validate().unwrap();
        assert_eq!(graph.aux_buses[0].effects, [reverb]);
        assert_eq!(graph.master_chain, [master]);
        assert_eq!(graph.sends[0].level_db, -18.0);
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| matches!(node.kind, NodeKind::AuxReturn { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn activation_connection_failure_restores_the_exact_direct_topology() {
        let routes = routes();
        let mut connections = MockConnections::default();
        apply_transaction(&mut connections, &routes.direct_connection_changes()).unwrap();
        let direct = connections.connected.clone();
        connections.fail_at = Some(10);
        assert!(apply_transaction(&mut connections, &routes.activation_changes()).is_err());
        assert_eq!(connections.connected, direct);
    }

    #[test]
    fn committed_activation_has_one_graph_path_and_no_direct_doubling() {
        let routes = routes();
        let mut connections = MockConnections::default();
        connections
            .connected
            .insert(("unrelated:out".into(), "unrelated:in".into()));
        for direct in &routes.direct {
            connections
                .connected
                .insert((direct.source.clone(), direct.destination.clone()));
        }
        apply_transaction(&mut connections, &routes.activation_changes()).unwrap();
        let expected = routes
            .graph
            .iter()
            .map(|route| (route.source.clone(), route.destination.clone()))
            .chain(std::iter::once((
                "unrelated:out".into(),
                "unrelated:in".into(),
            )))
            .collect();
        assert_eq!(connections.connected, expected);
    }

    #[test]
    fn publication_is_block_boundary_dry_and_allocation_free() {
        let mut callback = callback(128);
        let left = [0.25; 128];
        let right = [-0.5; 128];
        let silence = [0.0; 128];
        let mut output_left = [1.0; 128];
        let mut output_right = [1.0; 128];
        assert_no_allocations(|| {
            assert_eq!(
                process_block(
                    &mut callback,
                    128,
                    [&left, &right, &silence, &silence, &silence, &silence],
                    &mut output_left,
                    &mut output_right,
                ),
                ProcessStatus::Complete
            );
        });
        assert_eq!(output_left, [0.0; 128]);
        assert_eq!(output_right, [0.0; 128]);

        callback.armed.store(true, Ordering::Release);
        assert_no_allocations(|| {
            assert_eq!(
                process_block(
                    &mut callback,
                    128,
                    [&left, &right, &silence, &silence, &silence, &silence],
                    &mut output_left,
                    &mut output_right,
                ),
                ProcessStatus::Complete
            );
        });
        assert_eq!(&output_left[..120], &[0.0; 120]);
        assert_eq!(&output_left[120..], &[0.25; 8]);
        assert_eq!(&output_right[120..], &[-0.5; 8]);
    }

    #[test]
    fn oversized_callback_is_silent_and_countable_without_allocation() {
        let mut callback = callback(64);
        let input = [1.0; 128];
        let mut left = [1.0; 128];
        let mut right = [1.0; 128];
        assert_no_allocations(|| {
            let status = process_block(
                &mut callback,
                128,
                [&input, &input, &input, &input, &input, &input],
                &mut left,
                &mut right,
            );
            callback.timing.record(128, 48_000, 10, status);
        });
        assert_eq!(left, [0.0; 128]);
        assert_eq!(right, [0.0; 128]);
        assert_eq!(callback.timing.snapshot().oversized_callbacks, 1);
    }

    #[test]
    fn callback_clock_reads_are_allocation_free() {
        assert_no_allocations(|| {
            let start = monotonic_nanoseconds();
            let end = monotonic_nanoseconds();
            assert!(end >= start);
        });
    }

    #[test]
    fn jack_shutdown_only_marks_client_loss_for_owner_recovery() {
        let mut callback = callback(64);
        assert!(!callback.client_lost.load(Ordering::Acquire));
        let pointer = ((&mut callback) as *mut CallbackData).cast();
        unsafe { shutdown_callback(pointer) };
        assert!(callback.client_lost.load(Ordering::Acquire));
    }

    #[test]
    fn ambiguous_boundaries_are_rejected_before_jack_activation() {
        let duplicate = ["same:port".to_owned(), "same:port".to_owned()];
        assert!(validate_stereo_boundary(&duplicate, "test").is_err());
        let empty = [String::new(), "right:port".to_owned()];
        assert!(validate_stereo_boundary(&empty, "test").is_err());
    }
}
