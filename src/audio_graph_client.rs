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
use crate::config::AudioGraphConfig;
use crate::dsp::{MeterSnapshot, StereoFrame};
use crate::jack::{Client as JackClient, Port as JackPort, PortDirection, PortGetBuffer};
use anyhow::{anyhow, bail, Context, Result};
use libc::{c_int, c_uint, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

const SOURCE_NODE: u32 = 1;
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
    direct: [Connection; 2],
    graph: [Connection; 4],
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
    input_left: *mut JackPort,
    input_right: *mut JackPort,
    output_left: *mut JackPort,
    output_right: *mut JackPort,
    port_get_buffer: PortGetBuffer,
    sample_rate: u32,
    armed: AtomicBool,
    client_lost: AtomicBool,
    timing: CallbackTimingCounters,
}

// JACK owns callback scheduling, while the box itself remains pinned and is
// reclaimed only after deactivation on the non-real-time owner thread.
unsafe impl Send for CallbackData {}

pub(crate) struct OwnedAudioGraph {
    jack: JackClient,
    callback: Box<CallbackData>,
    routes: BoundaryRoutes,
    aux_return_nodes: [Option<(u8, u32)>; 2],
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

impl OwnedAudioGraph {
    pub(crate) fn sample_rate(&self) -> u32 {
        self.callback.sample_rate
    }

    pub(crate) fn start_with_routing(
        config: &AudioGraphConfig,
        source_ports: [String; 2],
        destinations: [String; 2],
        rack: &InsertRack,
        aux_routing: &ProjectAuxRouting,
    ) -> Result<Self> {
        validate_stereo_boundary(&source_ports, "managed-engine source")?;
        validate_stereo_boundary(&destinations, "main output")?;

        let mut jack = JackClient::open(&config.client_name).context("open owned audio graph")?;
        let sample_rate = jack.sample_rate();
        if sample_rate == 0 {
            bail!("JACK reported a zero sample rate");
        }
        let definition = managed_graph_definition(
            sample_rate,
            config.maximum_callback_frames,
            &destinations,
            rack,
            aux_routing,
        );
        let plan = GraphPlan::compile(&definition).context("compile managed audio graph")?;

        let input_left = jack.register_audio_port("managed_in_l", PortDirection::Input)?;
        let input_right = jack.register_audio_port("managed_in_r", PortDirection::Input)?;
        let output_left = jack.register_audio_port("main_out_l", PortDirection::Output)?;
        let output_right = jack.register_audio_port("main_out_r", PortDirection::Output)?;
        let graph_port_names = [
            jack.port_name_string(input_left)?,
            jack.port_name_string(input_right)?,
            jack.port_name_string(output_left)?,
            jack.port_name_string(output_right)?,
        ];
        let routes = BoundaryRoutes {
            direct: [
                connection(&source_ports[0], &destinations[0]),
                connection(&source_ports[1], &destinations[1]),
            ],
            graph: [
                connection(&source_ports[0], &graph_port_names[0]),
                connection(&source_ports[1], &graph_port_names[1]),
                connection(&graph_port_names[2], &destinations[0]),
                connection(&graph_port_names[3], &destinations[1]),
            ],
        };
        let mut callback = Box::new(CallbackData {
            plan,
            input_left,
            input_right,
            output_left,
            output_right,
            port_get_buffer: jack.port_get_buffer(),
            sample_rate,
            armed: AtomicBool::new(false),
            client_lost: AtomicBool::new(false),
            timing: CallbackTimingCounters::default(),
        });
        let callback_pointer = ((&mut *callback) as *mut CallbackData).cast();
        // SAFETY: callback remains boxed until after explicit JACK deactivation.
        unsafe {
            jack.set_process_callback(process_callback, callback_pointer)?;
            jack.set_shutdown_callback(shutdown_callback, callback_pointer);
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
        // ready and both direct links are gone before dry output is published.
        callback.armed.store(true, Ordering::Release);
        Ok(Self {
            jack,
            callback,
            routes,
            aux_return_nodes: aux_return_nodes(aux_routing),
        })
    }

    pub(crate) fn client_lost(&self) -> bool {
        self.callback.client_lost.load(Ordering::Acquire)
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
            output: self.callback.plan.meter(MASTER_NODE)?.load(),
        })
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
            self.routes.graph[2].destination.clone(),
            self.routes.graph[3].destination.clone(),
        ];
        let definition = managed_graph_definition(
            self.callback.sample_rate,
            self.callback.plan.maximum_frames() as u32,
            &destinations,
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

    /// Restore both exact direct links best-effort. This runs only on the
    /// non-real-time owner thread, including client-loss recovery.
    pub(crate) fn restore_direct(&mut self) -> Result<()> {
        self.callback.armed.store(false, Ordering::Release);
        // Join the callback before creating either direct link. A callback
        // that sampled the previous publish flag can therefore never overlap
        // the restored dry path for even one block.
        self.jack.deactivate();
        let mut first_error = None;
        for connection in &self.routes.direct {
            if let Err(error) = self
                .jack
                .ensure_connection(&connection.source, &connection.destination)
            {
                first_error.get_or_insert(error);
            }
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
    rack: &InsertRack,
    aux_routing: &ProjectAuxRouting,
) -> GraphDefinition {
    let mut nodes = vec![Node {
        id: SOURCE_NODE,
        layout: ChannelLayout::Stereo,
        kind: NodeKind::Source {
            source: SourceKind::ManagedEngine,
        },
    }];
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
        monitoring: Monitoring::default(),
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
        &InsertRack::default(),
        &ProjectAuxRouting::default(),
    )
}

fn process_block(
    callback: &mut CallbackData,
    frames: usize,
    input_left: &[f32],
    input_right: &[f32],
    output_left: &mut [f32],
    output_right: &mut [f32],
) -> ProcessStatus {
    let publish = callback.armed.load(Ordering::Acquire);
    if frames > callback.plan.maximum_frames()
        || input_left.len() < frames
        || input_right.len() < frames
        || output_left.len() < frames
        || output_right.len() < frames
    {
        output_left.fill(0.0);
        output_right.fill(0.0);
        return ProcessStatus::OversizedBlock;
    }
    let Some(source) = callback.plan.source_buffer_mut(SOURCE_NODE, frames) else {
        output_left[..frames].fill(0.0);
        output_right[..frames].fill(0.0);
        return ProcessStatus::OversizedBlock;
    };
    for index in 0..frames {
        source[index] = StereoFrame::new(input_left[index], input_right[index]);
    }
    let status = callback.plan.process(frames);
    if !publish || !matches!(status, ProcessStatus::Complete) {
        output_left[..frames].fill(0.0);
        output_right[..frames].fill(0.0);
        return status;
    }
    let Some(output) = callback.plan.output_buffer(SINK_NODE, frames) else {
        output_left[..frames].fill(0.0);
        output_right[..frames].fill(0.0);
        return ProcessStatus::OversizedBlock;
    };
    for index in 0..frames {
        output_left[index] = output[index].left;
        output_right[index] = output[index].right;
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
    let input_left = unsafe { get_buffer(callback.input_left, frames) }.cast::<f32>();
    let input_right = unsafe { get_buffer(callback.input_right, frames) }.cast::<f32>();
    let output_left = unsafe { get_buffer(callback.output_left, frames) }.cast::<f32>();
    let output_right = unsafe { get_buffer(callback.output_right, frames) }.cast::<f32>();
    if input_left.is_null()
        || input_right.is_null()
        || output_left.is_null()
        || output_right.is_null()
    {
        return 0;
    }
    let frame_count = frames as usize;
    // SAFETY: JACK provides exactly `frames` f32 samples for each audio port.
    let input_left = unsafe { std::slice::from_raw_parts(input_left, frame_count) };
    let input_right = unsafe { std::slice::from_raw_parts(input_right, frame_count) };
    let output_left = unsafe { std::slice::from_raw_parts_mut(output_left, frame_count) };
    let output_right = unsafe { std::slice::from_raw_parts_mut(output_right, frame_count) };
    let status = process_block(
        callback,
        frame_count,
        input_left,
        input_right,
        output_left,
        output_right,
    );
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
        unsafe { &*argument.cast::<CallbackData>() }
            .client_lost
            .store(true, Ordering::Release);
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
            direct: [
                connection("engine:l", "main:l"),
                connection("engine:r", "main:r"),
            ],
            graph: [
                connection("engine:l", "graph:in_l"),
                connection("engine:r", "graph:in_r"),
                connection("graph:out_l", "main:l"),
                connection("graph:out_r", "main:r"),
            ],
        }
    }

    fn callback(maximum_frames: u32) -> CallbackData {
        let destinations = ["main:l".to_owned(), "main:r".to_owned()];
        CallbackData {
            plan: GraphPlan::compile(&dry_graph_definition(48_000, maximum_frames, &destinations))
                .unwrap(),
            input_left: std::ptr::null_mut(),
            input_right: std::ptr::null_mut(),
            output_left: std::ptr::null_mut(),
            output_right: std::ptr::null_mut(),
            port_get_buffer: dummy_get_buffer,
            sample_rate: 48_000,
            armed: AtomicBool::new(false),
            client_lost: AtomicBool::new(false),
            timing: CallbackTimingCounters::default(),
        }
    }

    unsafe extern "C" fn dummy_get_buffer(_: *mut JackPort, _: c_uint) -> *mut c_void {
        std::ptr::null_mut()
    }

    #[test]
    fn dry_topology_is_valid_and_contains_one_managed_path() {
        let destinations = ["main:l".to_owned(), "main:r".to_owned()];
        let graph = dry_graph_definition(48_000, 128, &destinations);
        assert_eq!(
            graph.validate().unwrap(),
            [SOURCE_NODE, MASTER_NODE, SINK_NODE]
        );
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert!(graph.effects.is_empty());
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
            &rack,
            &ProjectAuxRouting::default(),
        );
        assert_eq!(
            graph.validate().unwrap(),
            [
                SOURCE_NODE,
                FIRST_EFFECT_NODE,
                FIRST_EFFECT_NODE + 1,
                MASTER_NODE,
                SINK_NODE
            ]
        );
        assert_eq!(graph.source_chains[0].effects, [eq, compressor]);
        assert_eq!(graph.edges.len(), 4);
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
        let graph = managed_graph_definition(48_000, 128, &destinations, &rack, &routing);
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
        connections.fail_at = Some(8);
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
        let mut output_left = [1.0; 128];
        let mut output_right = [1.0; 128];
        assert_no_allocations(|| {
            assert_eq!(
                process_block(
                    &mut callback,
                    128,
                    &left,
                    &right,
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
                    &left,
                    &right,
                    &mut output_left,
                    &mut output_right,
                ),
                ProcessStatus::Complete
            );
        });
        assert_eq!(output_left, left);
        assert_eq!(output_right, right);
    }

    #[test]
    fn oversized_callback_is_silent_and_countable_without_allocation() {
        let mut callback = callback(64);
        let input = [1.0; 128];
        let mut left = [1.0; 128];
        let mut right = [1.0; 128];
        assert_no_allocations(|| {
            let status = process_block(&mut callback, 128, &input, &input, &mut left, &mut right);
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
