use crate::audio_graph::{InsertRack, ProjectAuxRouting};
use crate::audio_graph_client::{
    AuxMeterSnapshot, EffectMeterSnapshot, OwnedAudioGraph, PerformanceBusPorts,
};
use crate::audio_graph_runtime::CallbackTimingSnapshot;
use crate::config::{BackendConfig, RuntimeConfig};
use crate::control::{self, CONTROLS};
use crate::pads::{EncoderAction, PadAction, PadConfig};
use crate::preset::{self, BackendKind, Preset, PresetId};
use anyhow::{anyhow, bail, Context, Result};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
    Arc, Mutex, RwLock,
};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum MidiEvent {
    MappedControl(u8, f32),
    Value(u8, f32),
    Raw { received: Instant, bytes: Vec<u8> },
    Pad(PadAction, bool),
    Encoder(EncoderAction),
    PadLock(bool),
    Learn { received: Instant, bytes: Vec<u8> },
    Error(String),
}

/// The single owned process behind the active backend. All backend-specific
/// lifecycle operations are kept behind this API so callers cannot layer them.
pub struct Engine {
    backend: BackendKind,
    child: Child,
    stdin: Option<ChildStdin>,
    state: PathBuf,
    output: SharedOutput,
    control_routes: Vec<(u8, u8)>,
    fluid_soundfonts: Vec<(PathBuf, u16)>,
    audio_graph: Option<OwnedAudioGraph>,
    final_recording_last: crate::audio_recorder::FinalMixRecorderStatus,
    audio_graph_fallback: Option<String>,
    audio_route_notice: Option<String>,
    midi_lifecycle: Option<MidiLifecycle>,
}

pub type SharedOutput = Arc<Mutex<Option<MidiOutputConnection>>>;
pub type SharedPickup = Arc<Mutex<crate::midi::Pickup>>;
pub type SharedBackend = Arc<Mutex<BackendKind>>;
pub type SharedTrackerRoute = Arc<Mutex<TrackerRoute>>;
pub type SharedTrackerInput = Arc<Mutex<Option<crate::sequencer::LiveInput>>>;
pub type SharedPlaybackScale = Arc<Mutex<Option<crate::scale::Scale>>>;
pub type SharedControllerConfig = Arc<RwLock<PadConfig>>;
pub type SharedLearnMode = Arc<AtomicBool>;
pub type SharedFxControlMode = Arc<AtomicBool>;

#[derive(Clone, Debug)]
pub struct MidiLifecycle {
    state: Arc<Mutex<LiveMidiState>>,
}

impl Default for MidiLifecycle {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(LiveMidiState::default())),
        }
    }
}

impl MidiLifecycle {
    fn new(state: Arc<Mutex<LiveMidiState>>) -> Self {
        Self { state }
    }

    pub fn clear_after_all_notes_off(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = LiveMidiState::default();
        }
    }
}

pub struct TrackerRouteConfig<'a> {
    pub enabled: bool,
    pub target: crate::sequencer::PageTarget,
    pub columns: [(u8, (u8, u8, u8)); crate::sequencer::LANES_PER_PAGE],
    pub start_column: usize,
    pub percussion: bool,
    pub audition_note: Option<u8>,
    pub scale: Option<crate::scale::Scale>,
    pub external: &'a crate::config::ExternalMidiConfig,
}

#[derive(Clone)]
pub struct TrackerRoute {
    enabled: bool,
    target: crate::sequencer::PageTarget,
    columns: [TrackerColumnRoute; crate::sequencer::LANES_PER_PAGE],
    start_column: usize,
    note_map: [u8; 128],
    scale: Option<crate::scale::Scale>,
    bank_select: crate::config::BankSelectMode,
    revision: u64,
    navigation_revision: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct TrackerColumnRoute {
    channel: u8,
    program: Option<u8>,
    bank_msb: u8,
    bank_lsb: u8,
}

struct CallbackRouting {
    output: SharedOutput,
    pickup: SharedPickup,
    backend: SharedBackend,
    tracker_route: SharedTrackerRoute,
    tracker_input: SharedTrackerInput,
    playback_scale: SharedPlaybackScale,
    controller: SharedControllerConfig,
    learn_mode: SharedLearnMode,
    fx_control_mode: SharedFxControlMode,
    live_state: Arc<Mutex<LiveMidiState>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct InputRoles {
    controller: bool,
    performance: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PlannedMidiInput {
    name: String,
    roles: InputRoles,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MidiInputState {
    pub wanted: String,
    pub resolved: Option<String>,
    pub error: Option<String>,
}

impl MidiInputState {
    pub fn available(&self) -> bool {
        self.resolved.is_some() && self.error.is_none()
    }

    pub fn description(&self) -> String {
        match (&self.resolved, &self.error) {
            (Some(name), None) => format!("connected · {name}"),
            (_, Some(error)) => format!("unavailable · {error}"),
            (None, None) => "not configured".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MidiInputAvailability {
    pub controller: Option<MidiInputState>,
    pub performance: Vec<MidiInputState>,
}

impl MidiInputAvailability {
    pub fn controller_available(&self) -> bool {
        self.controller
            .as_ref()
            .is_some_and(MidiInputState::available)
    }

    #[cfg(test)]
    pub fn performance_available(&self) -> usize {
        self.performance
            .iter()
            .filter(|state| state.available())
            .count()
    }
}

#[derive(Clone, Debug, Default)]
struct MidiInputPlan {
    inputs: Vec<PlannedMidiInput>,
    availability: MidiInputAvailability,
}

#[derive(Debug, Default)]
struct LiveMidiState {
    tracker_revision: u64,
    tracker_navigation_revision: u64,
    tracker_next_column: std::collections::BTreeMap<String, usize>,
    tracker_programs: std::collections::BTreeMap<(crate::sequencer::PageTarget, u8), u8>,
    tracker_notes:
        crate::note_lifecycle::SourceNoteLifecycle<String, (crate::sequencer::PageTarget, u8, u8)>,
    tracker_destinations: std::collections::BTreeMap<(crate::sequencer::PageTarget, u8, u8), usize>,
    direct_notes: crate::note_lifecycle::SourceNoteLifecycle<String, (u8, u8)>,
    direct_destinations: std::collections::BTreeMap<(u8, u8), usize>,
    sustain_sources: std::collections::BTreeSet<(String, u8)>,
    sustain_counts: [usize; 16],
}

impl Default for TrackerRoute {
    fn default() -> Self {
        Self {
            enabled: false,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [TrackerColumnRoute::default(); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            note_map: std::array::from_fn(|note| note as u8),
            scale: None,
            bank_select: crate::config::BankSelectMode::Off,
            revision: 0,
            navigation_revision: 0,
        }
    }
}

impl TrackerRoute {
    pub fn configure(&mut self, config: TrackerRouteConfig<'_>) {
        self.revision = self.revision.wrapping_add(1);
        self.navigation_revision = self.navigation_revision.wrapping_add(1);
        self.apply(config);
    }

    pub(crate) fn configure_navigation(&mut self, config: TrackerRouteConfig<'_>) {
        self.navigation_revision = self.navigation_revision.wrapping_add(1);
        self.apply(config);
    }

    fn apply(&mut self, config: TrackerRouteConfig<'_>) {
        self.enabled = config.enabled;
        let software_synth = matches!(
            &config.target,
            crate::sequencer::PageTarget::ActiveInstrument
                | crate::sequencer::PageTarget::Synthv1(_)
                | crate::sequencer::PageTarget::Software(_)
        );
        self.target = config.target;
        self.columns = config
            .columns
            .map(|(channel, selection)| TrackerColumnRoute {
                channel,
                program: (config.external.program_changes && !software_synth)
                    .then_some(selection.0),
                bank_msb: selection.1,
                bank_lsb: selection.2,
            });
        self.start_column = config
            .start_column
            .min(crate::sequencer::LANES_PER_PAGE - 1);
        self.note_map = std::array::from_fn(|note| note as u8);
        self.scale = config.scale;
        self.bank_select = config.external.bank_select;
        if config.percussion {
            if let Some(note) = config.audition_note {
                self.note_map.fill(note);
            } else {
                for (offset, &note) in config.external.percussion_notes.iter().enumerate() {
                    self.note_map[usize::from(config.external.percussion_input_base) + offset] =
                        note;
                }
            }
        }
    }

    fn mapped_note(&self, note: u8) -> Option<u8> {
        if self.scale.is_some_and(|scale| !scale.contains(note)) {
            return None;
        }
        self.note_map.get(usize::from(note)).copied()
    }

    fn column(&self, index: usize) -> TrackerColumnRoute {
        self.columns[index % crate::sequencer::LANES_PER_PAGE]
    }

    pub(crate) fn destinations(&self) -> Vec<(crate::sequencer::PageTarget, u8)> {
        self.columns
            .iter()
            .map(|column| (self.target.clone(), column.channel))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[cfg(test)]
    pub fn preview_state(&self) -> (bool, Option<u8>, u8, u8) {
        let column = self.column(self.start_column);
        (
            self.enabled,
            column.program,
            column.bank_msb,
            column.bank_lsb,
        )
    }
}

#[cfg(test)]
fn tracker_route_consumes_note(route: Option<&TrackerRoute>, message: &[u8]) -> bool {
    route.is_some_and(|route| route.enabled && valid_note_message(message))
}

fn playback_filter_allows(scale: Option<crate::scale::Scale>, message: &[u8]) -> bool {
    !valid_note_message(message) || scale.is_none_or(|scale| scale.contains(message[1]))
}

fn valid_note_message(message: &[u8]) -> bool {
    message.len() == 3
        && matches!(message[0] & 0xf0, 0x80 | 0x90)
        && message[1] <= 127
        && message[2] <= 127
}

fn invalid_note_message(message: &[u8]) -> bool {
    message
        .first()
        .is_some_and(|status| matches!(status & 0xf0, 0x80 | 0x90))
        && !valid_note_message(message)
}

pub struct MidiRouter {
    _inputs: Vec<MidiInputConnection<()>>,
    output: SharedOutput,
    pickup: SharedPickup,
    backend: SharedBackend,
    tracker_route: SharedTrackerRoute,
    tracker_input: SharedTrackerInput,
    playback_scale: SharedPlaybackScale,
    controller: SharedControllerConfig,
    learn_mode: SharedLearnMode,
    fx_control_mode: SharedFxControlMode,
    live_state: Arc<Mutex<LiveMidiState>>,
    availability: MidiInputAvailability,
    tx: Sender<MidiEvent>,
    monitor_stop: Arc<AtomicBool>,
    monitor_thread: Option<thread::JoinHandle<()>>,
}

impl MidiRouter {
    pub fn start(state: &Path, config: &RuntimeConfig, tx: Sender<MidiEvent>) -> Result<Self> {
        if !config.midi_autoconnect {
            bail!("MIDI routing is disabled in shsynth.conf");
        }
        let pads = PadConfig::load(&state.join("controller.conf"))?;
        let controller = Arc::new(RwLock::new(pads));
        let learn_mode = Arc::new(AtomicBool::new(false));
        let fx_control_mode = Arc::new(AtomicBool::new(false));
        let output = Arc::new(Mutex::new(None));
        let pickup = Arc::new(Mutex::new(crate::midi::Pickup::default()));
        let backend = Arc::new(Mutex::new(BackendKind::Synthv1));
        let tracker_route = Arc::new(Mutex::new(TrackerRoute::default()));
        let tracker_input = Arc::new(Mutex::new(None));
        let playback_scale = Arc::new(Mutex::new(None));
        let live_state = Arc::new(Mutex::new(LiveMidiState::default()));
        let pads_snapshot = controller
            .read()
            .map(|pads| pads.clone())
            .unwrap_or_default();
        let names = midi_input_names()?;
        let mut plan = plan_midi_inputs(&names, &pads_snapshot, config);
        let mut inputs = Vec::new();
        let mut opened_names = Vec::new();
        for planned in plan.inputs.clone() {
            let routing = CallbackRouting {
                output: Arc::clone(&output),
                pickup: Arc::clone(&pickup),
                backend: Arc::clone(&backend),
                tracker_route: Arc::clone(&tracker_route),
                tracker_input: Arc::clone(&tracker_input),
                playback_scale: Arc::clone(&playback_scale),
                controller: Arc::clone(&controller),
                learn_mode: Arc::clone(&learn_mode),
                fx_control_mode: Arc::clone(&fx_control_mode),
                live_state: Arc::clone(&live_state),
            };
            match connect_midi_input(tx.clone(), &planned, config, routing) {
                Ok(connection) => {
                    opened_names.push(planned.name.clone());
                    inputs.push(connection);
                }
                Err(error) => mark_input_open_error(
                    &mut plan.availability,
                    &planned,
                    format!("{}: {error:#}", planned.name),
                ),
            }
        }
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let monitor_thread = (!opened_names.is_empty()).then(|| {
            let stop = Arc::clone(&monitor_stop);
            let state = Arc::clone(&live_state);
            let output = Arc::clone(&output);
            let tracker_input = Arc::clone(&tracker_input);
            let tx = tx.clone();
            thread::spawn(move || {
                let mut disconnected = std::collections::BTreeSet::new();
                while !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(250));
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let visible = midi_input_names().unwrap_or_default();
                    for source in &opened_names {
                        if visible.iter().any(|name| name == source) {
                            continue;
                        }
                        if disconnected.insert(source.clone()) {
                            let deliveries = state
                                .lock()
                                .map(|mut state| release_source(&mut state, source))
                                .unwrap_or_default();
                            deliver_midi(deliveries, Instant::now(), &tx, &output, &tracker_input);
                            let _ = tx.send(MidiEvent::Error(format!(
                                "MIDI input disconnected: {source}"
                            )));
                        }
                    }
                }
            })
        });
        Ok(Self {
            _inputs: inputs,
            output,
            pickup,
            backend,
            tracker_route,
            tracker_input,
            playback_scale,
            controller,
            learn_mode,
            fx_control_mode,
            live_state,
            availability: plan.availability,
            tx,
            monitor_stop,
            monitor_thread,
        })
    }

    pub fn output(&self) -> SharedOutput {
        Arc::clone(&self.output)
    }

    pub fn pickup(&self) -> SharedPickup {
        Arc::clone(&self.pickup)
    }

    pub fn backend(&self) -> SharedBackend {
        Arc::clone(&self.backend)
    }

    pub fn tracker_route(&self) -> SharedTrackerRoute {
        Arc::clone(&self.tracker_route)
    }

    pub fn tracker_input(&self) -> SharedTrackerInput {
        Arc::clone(&self.tracker_input)
    }

    pub fn playback_scale(&self) -> SharedPlaybackScale {
        Arc::clone(&self.playback_scale)
    }

    pub fn controller_config(&self) -> SharedControllerConfig {
        Arc::clone(&self.controller)
    }

    pub fn learn_mode(&self) -> SharedLearnMode {
        Arc::clone(&self.learn_mode)
    }

    pub fn fx_control_mode(&self) -> SharedFxControlMode {
        Arc::clone(&self.fx_control_mode)
    }

    pub fn availability(&self) -> &MidiInputAvailability {
        &self.availability
    }

    pub fn lifecycle(&self) -> MidiLifecycle {
        MidiLifecycle::new(Arc::clone(&self.live_state))
    }

    pub fn arm_pickup(&self, values: &std::collections::HashMap<u8, f32>) {
        if let Ok(mut pickup) = self.pickup.lock() {
            pickup.arm(values);
        }
    }

    /// Replace only SHR-owned MIDI input subscriptions while retaining the
    /// output, pickup, tracker, learning, and note-lifecycle shared state used
    /// by the active engine. Old inputs are released and closed before any new
    /// input is opened, so routes are never layered.
    pub fn reconfigure_inputs(&mut self, config: &RuntimeConfig) -> Result<MidiInputAvailability> {
        self.stop_input_monitor();
        let deliveries = self
            .live_state
            .lock()
            .map_or_else(|_| Vec::new(), |mut state| release_all_inputs(&mut state));
        deliver_midi(
            deliveries,
            Instant::now(),
            &self.tx,
            &self.output,
            &self.tracker_input,
        );
        self._inputs.clear();
        if !config.midi_autoconnect {
            self.availability = MidiInputAvailability::default();
            return Ok(self.availability.clone());
        }

        let pads = self
            .controller
            .read()
            .map(|pads| pads.clone())
            .unwrap_or_default();
        let names = midi_input_names()?;
        let mut plan = plan_midi_inputs(&names, &pads, config);
        let mut opened_names = Vec::new();
        for planned in plan.inputs.clone() {
            let routing = CallbackRouting {
                output: Arc::clone(&self.output),
                pickup: Arc::clone(&self.pickup),
                backend: Arc::clone(&self.backend),
                tracker_route: Arc::clone(&self.tracker_route),
                tracker_input: Arc::clone(&self.tracker_input),
                playback_scale: Arc::clone(&self.playback_scale),
                controller: Arc::clone(&self.controller),
                learn_mode: Arc::clone(&self.learn_mode),
                fx_control_mode: Arc::clone(&self.fx_control_mode),
                live_state: Arc::clone(&self.live_state),
            };
            match connect_midi_input(self.tx.clone(), &planned, config, routing) {
                Ok(connection) => {
                    opened_names.push(planned.name.clone());
                    self._inputs.push(connection);
                }
                Err(error) => {
                    self._inputs.clear();
                    mark_input_open_error(
                        &mut plan.availability,
                        &planned,
                        format!("{}: {error:#}", planned.name),
                    );
                    self.availability = plan.availability;
                    return Err(anyhow!("MIDI input activation failed: {error:#}"));
                }
            }
        }
        self.start_input_monitor(opened_names);
        self.availability = plan.availability;
        Ok(self.availability.clone())
    }

    fn stop_input_monitor(&mut self) {
        self.monitor_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.monitor_thread.take() {
            let _ = thread.join();
        }
        self.monitor_stop = Arc::new(AtomicBool::new(false));
    }

    fn start_input_monitor(&mut self, opened_names: Vec<String>) {
        if opened_names.is_empty() {
            return;
        }
        let stop = Arc::clone(&self.monitor_stop);
        let state = Arc::clone(&self.live_state);
        let output = Arc::clone(&self.output);
        let tracker_input = Arc::clone(&self.tracker_input);
        let tx = self.tx.clone();
        self.monitor_thread = Some(thread::spawn(move || {
            let mut disconnected = std::collections::BTreeSet::new();
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(250));
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let visible = midi_input_names().unwrap_or_default();
                for source in &opened_names {
                    if visible.iter().any(|name| name == source) {
                        continue;
                    }
                    if disconnected.insert(source.clone()) {
                        let deliveries = state
                            .lock()
                            .map(|mut state| release_source(&mut state, source))
                            .unwrap_or_default();
                        deliver_midi(deliveries, Instant::now(), &tx, &output, &tracker_input);
                        let _ = tx.send(MidiEvent::Error(format!(
                            "MIDI input disconnected: {source}"
                        )));
                    }
                }
            }
        }));
    }
}

impl Drop for MidiRouter {
    fn drop(&mut self) {
        self.stop_input_monitor();
        let deliveries = self
            .live_state
            .lock()
            .map_or_else(|_| Vec::new(), |mut state| release_all_inputs(&mut state));
        deliver_midi(
            deliveries,
            Instant::now(),
            &self.tx,
            &self.output,
            &self.tracker_input,
        );
    }
}

impl Engine {
    pub fn start(
        preset: &Preset,
        state: &Path,
        output: SharedOutput,
        config: &RuntimeConfig,
    ) -> Result<Self> {
        Self::start_with_rack(preset, state, output, config, &InsertRack::default())
    }

    pub fn start_with_rack(
        preset: &Preset,
        state: &Path,
        output: SharedOutput,
        config: &RuntimeConfig,
        rack: &InsertRack,
    ) -> Result<Self> {
        Self::start_with_routing(
            preset,
            state,
            output,
            config,
            rack,
            &ProjectAuxRouting::default(),
        )
    }

    pub fn start_with_routing(
        preset: &Preset,
        state: &Path,
        output: SharedOutput,
        config: &RuntimeConfig,
        rack: &InsertRack,
        aux_routing: &ProjectAuxRouting,
    ) -> Result<Self> {
        fs::create_dir_all(state)?;
        let EnginePreflight {
            controller,
            fluid_soundfonts,
            backend_config,
            mut command,
        } = preflight_start(preset, state, config)?;

        stop_managed(state)?;
        if preset.backend == BackendKind::Synthv1 {
            write_synthv1_config(&state.join("config"), &controller)?;
        }
        if preset.backend == BackendKind::FluidSynth {
            write_fluidsynth_config(state, &fluid_soundfonts)?;
        }

        let log_path = state.join("engine.log");
        let log = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&log_path)?;
        let log_err = log.try_clone()?;
        set_command_affinity(&mut command, config.audio_engine_cpu);
        let mut child = command
            .stdin(if preset.backend == BackendKind::Synthv1 {
                Stdio::null()
            } else {
                Stdio::piped()
            })
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .spawn()
            .with_context(|| format!("start {}", backend_config.command))?;
        let mut audio_graph = None;
        let mut audio_graph_fallback = None;
        let mut audio_route_notice = None;
        let prepare = (|| -> Result<()> {
            write_owner(&state.join("engine.pid"), child.id() as i32)?;
            wait_ready(
                &mut child,
                preset.backend,
                &backend_config.client_name,
                config.startup_timeout,
                &log_path,
            )?;
            let resolved_audio = config.resolve_audio_route(&jack_ports());
            let mut runtime_config = config.clone();
            runtime_config.audio_outputs = resolved_audio.outputs;
            audio_route_notice = resolved_audio.notice;
            connect_audio(&backend_config.client_name, &runtime_config)?;
            if config.audio_graph.enabled {
                match start_managed_audio_graph(
                    &backend_config.client_name,
                    &runtime_config,
                    rack,
                    aux_routing,
                ) {
                    Ok(graph) => audio_graph = Some(graph),
                    Err(error) => {
                        audio_graph_fallback = Some(format!("{error:#}"));
                    }
                }
            }
            if config.midi_autoconnect {
                attach_midi_output(&output, &backend_config.midi_output_match, preset.backend)?;
                retain_midi_destination("SHR-DAW MIDI output", &backend_config.client_name);
            }
            retain_midi_destination(
                &config.external_midi.client_name,
                &config.external_midi.output_match,
            );
            if let Ok(names) = midi_input_names() {
                for source in plan_midi_inputs(&names, &controller, config).inputs {
                    disconnect_direct_midi(&source.name, &backend_config.client_name);
                }
            }
            fs::write(
                state.join("current"),
                format!("{}\t{}\n", preset.backend.label(), preset.name),
            )?;
            Ok(())
        })();
        if let Err(error) = prepare {
            if let Ok(mut connection) = output.lock() {
                *connection = None;
            }
            audio_graph.take();
            terminate(&mut child);
            cleanup_state(state);
            return Err(error);
        }

        let engine = Self {
            backend: preset.backend,
            stdin: child.stdin.take(),
            child,
            state: state.to_path_buf(),
            output,
            control_routes: if preset.backend == BackendKind::Synthv1 {
                controller
                    .controls
                    .iter()
                    .map(|(&incoming, &target)| (incoming, target))
                    .collect()
            } else {
                Vec::new()
            },
            fluid_soundfonts,
            audio_graph,
            final_recording_last: crate::audio_recorder::FinalMixRecorderStatus::default(),
            audio_graph_fallback,
            audio_route_notice,
            midi_lifecycle: None,
        };
        if preset.backend == BackendKind::FluidSynth {
            engine.select_fluidsynth(preset)?;
        }
        Ok(engine)
    }

    pub fn backend(&self) -> BackendKind {
        self.backend
    }

    pub fn bind_midi_lifecycle(&mut self, lifecycle: MidiLifecycle) {
        self.midi_lifecycle = Some(lifecycle);
    }

    pub fn alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }

    pub fn audio_route_status(&self) -> Option<String> {
        let graph = if self.audio_graph.is_some() {
            Some("owned insert graph active".to_owned())
        } else {
            self.audio_graph_fallback
                .as_ref()
                .map(|error| format!("direct audio fallback · {error}"))
        };
        match (&self.audio_route_notice, graph) {
            (Some(route), Some(graph)) => Some(format!("{route} · {graph}")),
            (Some(route), None) => Some(route.clone()),
            (None, graph) => graph,
        }
    }

    pub fn publish_fx_routing(
        &mut self,
        rack: &InsertRack,
        aux_routing: &ProjectAuxRouting,
    ) -> Result<bool> {
        let Some(graph) = self.audio_graph.as_mut() else {
            return Ok(false);
        };
        graph.publish_routing(rack, aux_routing)?;
        Ok(true)
    }

    pub(crate) fn retry_audio_graph(
        &mut self,
        config: &RuntimeConfig,
        rack: &InsertRack,
        aux_routing: &ProjectAuxRouting,
    ) -> Result<bool> {
        if self.audio_graph.is_some() || !config.audio_graph.enabled {
            return Ok(self.audio_graph.is_some());
        }
        let backend = backend_config(config, self.backend);
        let graph = start_managed_audio_graph(&backend.client_name, config, rack, aux_routing)?;
        self.audio_graph = Some(graph);
        self.audio_graph_fallback = None;
        Ok(true)
    }

    pub(crate) fn suspend_audio_graph(&mut self) -> Result<bool> {
        let Some((_timing, restored)) = self.stop_audio_graph() else {
            return Ok(false);
        };
        restored?;
        self.audio_graph_fallback =
            Some("final bus suspended · exact direct routes restored".into());
        Ok(true)
    }

    pub(crate) fn effect_meter(&self, effect_id: u32) -> Option<EffectMeterSnapshot> {
        self.audio_graph.as_ref()?.effect_meter(effect_id)
    }

    pub(crate) fn aux_meter(&self, aux_id: u8) -> Option<AuxMeterSnapshot> {
        self.audio_graph.as_ref()?.aux_meter(aux_id)
    }

    pub(crate) fn master_meter(&self) -> Option<AuxMeterSnapshot> {
        self.audio_graph.as_ref()?.master_meter()
    }

    pub(crate) fn final_bus_meter(&self) -> Option<crate::final_bus::FinalBusMeterSnapshot> {
        Some(self.audio_graph.as_ref()?.final_bus_meter())
    }

    pub(crate) fn bus_controls(&self) -> Option<std::sync::Arc<crate::final_bus::BusControls>> {
        Some(self.audio_graph.as_ref()?.bus_controls())
    }

    pub(crate) fn final_recording_status(
        &mut self,
    ) -> Option<crate::audio_recorder::FinalMixRecorderStatus> {
        if let Some(graph) = self.audio_graph.as_mut() {
            self.final_recording_last = graph.final_recording_status();
        }
        Some(self.final_recording_last.clone())
    }

    pub(crate) fn final_recording_active(&self) -> bool {
        self.audio_graph
            .as_ref()
            .is_some_and(|graph| graph.final_recording_active())
    }

    pub(crate) fn start_final_recording(&mut self, name: Option<&str>) -> Result<()> {
        let graph = self
            .audio_graph
            .as_mut()
            .context("final mix unavailable · owned graph is inactive")?;
        graph.start_final_recording(name)?;
        self.final_recording_last = graph.final_recording_status();
        Ok(())
    }

    pub(crate) fn stop_final_recording(&mut self) -> Result<()> {
        let graph = self
            .audio_graph
            .as_mut()
            .context("final mix unavailable · owned graph is inactive")?;
        let result = graph.stop_final_recording();
        self.final_recording_last = graph.final_recording_status();
        result
    }

    pub(crate) fn process_id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn audio_graph_sample_rate(&self) -> Option<u32> {
        self.audio_graph.as_ref().map(OwnedAudioGraph::sample_rate)
    }

    pub(crate) fn finish_audio_graph_checkpoint(
        &mut self,
    ) -> Option<(CallbackTimingSnapshot, Result<()>)> {
        self.stop_audio_graph()
    }

    /// JACK shutdown notification is callback-safe; route recovery runs here
    /// on the UI/owner thread and touches only the exact managed direct links.
    pub fn poll_audio_graph(&mut self) -> Option<String> {
        if !self
            .audio_graph
            .as_ref()
            .is_some_and(|graph| graph.client_lost())
        {
            return None;
        }
        let source_lost = self
            .audio_graph
            .as_ref()
            .is_some_and(|graph| graph.source_lost());
        let (timing, restored) = self.stop_audio_graph()?;
        let loss = if source_lost {
            "SOURCE LOST"
        } else {
            "AUDIO GRAPH LOST"
        };
        let status = match restored {
            Ok(()) => format!(
                "{loss} · exact direct routes restored · {}",
                audio_graph_metrics(&timing)
            ),
            Err(error) => format!(
                "{loss} · exact direct restore unavailable: {error:#} · {}",
                audio_graph_metrics(&timing)
            ),
        };
        self.audio_graph_fallback = Some(status.clone());
        Some(status)
    }

    fn stop_audio_graph(&mut self) -> Option<(CallbackTimingSnapshot, Result<()>)> {
        let mut graph = self.audio_graph.take()?;
        let restored = graph.restore_direct();
        self.final_recording_last = graph.final_recording_status();
        let timing = graph.timing();
        drop(graph);
        Some((timing, restored))
    }

    pub fn send(&self, message: &[u8]) -> Result<()> {
        self.output
            .lock()
            .map_err(|_| anyhow!("MIDI output lock poisoned"))?
            .as_mut()
            .context("MIDI output unavailable")?
            .send(message)
            .map_err(|error| anyhow!("MIDI send: {error}"))
    }

    pub fn panic(&self) {
        for message in crate::recording::all_notes_off() {
            let _ = self.send(&message);
        }
        if let Some(lifecycle) = &self.midi_lifecycle {
            lifecycle.clear_after_all_notes_off();
        }
    }

    /// Loads a sound without replacing the process when that backend supports
    /// it. Returns false when the caller must perform an exclusive restart.
    pub fn load_in_place(&mut self, preset: &Preset) -> Result<bool> {
        if preset.backend != self.backend || !self.alive() {
            return Ok(false);
        }
        match (&self.backend, &preset.id) {
            (BackendKind::Yoshimi, PresetId::Yoshimi { path }) => {
                let path = safe_command_path(path)?;
                self.panic();
                let stdin = self
                    .stdin
                    .as_mut()
                    .context("Yoshimi command input unavailable")?;
                writeln!(stdin, "load instrument {path}")?;
                stdin.flush()?;
            }
            (BackendKind::FluidSynth, PresetId::FluidSynth { .. }) => {
                fluidsynth_selection(preset, &self.fluid_soundfonts)?;
                self.panic();
                self.select_fluidsynth(preset)?;
            }
            _ => return Ok(false),
        }
        fs::write(
            self.state.join("current"),
            format!("{}\t{}\n", preset.backend.label(), preset.name),
        )?;
        Ok(true)
    }

    pub fn supports_parameter_reset(&self) -> bool {
        self.backend == BackendKind::Synthv1
    }

    pub fn set_mapped_parameters(&self, values: &std::collections::HashMap<u8, f32>) -> Result<()> {
        if !self.supports_parameter_reset() {
            bail!(
                "{} has no SHR-DAW mapped-parameter reset",
                self.backend.label()
            );
        }
        for message in mapped_parameter_messages(&self.control_routes, values) {
            self.send(&message)?;
        }
        Ok(())
    }

    fn select_fluidsynth(&self, preset: &Preset) -> Result<()> {
        let (effective_bank, program) = fluidsynth_selection(preset, &self.fluid_soundfonts)?;
        for channel in 0..16u8 {
            self.send(&[0xb0 | channel, 0, (effective_bank >> 7) as u8])?;
            self.send(&[0xb0 | channel, 32, (effective_bank & 0x7f) as u8])?;
            self.send(&[0xc0 | channel, program])?;
        }
        Ok(())
    }
}

struct EnginePreflight {
    controller: PadConfig,
    fluid_soundfonts: Vec<(PathBuf, u16)>,
    backend_config: BackendConfig,
    command: Command,
}

pub fn validate_start(preset: &Preset, state: &Path, config: &RuntimeConfig) -> Result<()> {
    preflight_start(preset, state, config).map(|_| ())
}

fn preflight_start(
    preset: &Preset,
    state: &Path,
    config: &RuntimeConfig,
) -> Result<EnginePreflight> {
    let controller = PadConfig::load(&state.join("controller.conf"))?;
    initial_values(preset)?;
    if let PresetId::Yoshimi { path } = &preset.id {
        if !path.is_file() {
            bail!("Yoshimi instrument is missing: {}", path.display());
        }
    }
    let fluid_soundfonts = if preset.backend == BackendKind::FluidSynth {
        preset::soundfont_offsets(&config.fluidsynth.soundfonts)?
    } else {
        Vec::new()
    };
    for (path, _) in &fluid_soundfonts {
        safe_command_path(path)?;
    }
    if preset.backend == BackendKind::FluidSynth {
        fluidsynth_selection(preset, &fluid_soundfonts)?;
    }
    let backend_config = backend_config(config, preset.backend);
    if !crate::fsutil::command_exists(&backend_config.command) {
        bail!(
            "{} command is unavailable or not executable: {}",
            preset.backend.label(),
            backend_config.command
        );
    }
    let command = backend_command(preset, state, config)?;
    Ok(EnginePreflight {
        controller,
        fluid_soundfonts,
        backend_config,
        command,
    })
}

fn fluidsynth_selection(preset: &Preset, soundfonts: &[(PathBuf, u16)]) -> Result<(u16, u8)> {
    let PresetId::FluidSynth {
        soundfont,
        bank,
        program,
        ..
    } = &preset.id
    else {
        bail!("not a FluidSynth preset")
    };
    let offset = soundfonts
        .iter()
        .find_map(|(candidate, offset)| (candidate == soundfont).then_some(*offset))
        .context("preset SoundFont is not configured for this FluidSynth process")?;
    let effective_bank = offset
        .checked_add(*bank)
        .context("SoundFont bank exceeds the MIDI bank range")?;
    if effective_bank > 16_383 {
        bail!("SoundFont bank exceeds the MIDI bank range");
    }
    Ok((effective_bank, *program))
}

/// Restrict only the managed engine process. The TUI, MIDI routing, and WAV
/// writer remain on housekeeping CPUs. System setup is responsible for making
/// the selected CPU available and keeping unrelated work away from it.
fn set_command_affinity(command: &mut Command, cpu: Option<usize>) {
    let Some(cpu) = cpu else {
        return;
    };
    unsafe {
        command.pre_exec(move || {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_ZERO(&mut set);
            libc::CPU_SET(cpu, &mut set);
            let result = libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
            if result == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.panic();
        // Restore the conservative dry boundary and deactivate the graph while
        // its callback allocation and the managed source are both still alive.
        self.audio_graph.take();
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = match self.backend {
                BackendKind::Yoshimi => writeln!(stdin, "exit y"),
                BackendKind::FluidSynth => writeln!(stdin, "quit"),
                BackendKind::Synthv1 => Ok(()),
            };
            let _ = stdin.flush();
        }
        if let Ok(mut output) = self.output.lock() {
            *output = None;
        }
        terminate(&mut self.child);
        cleanup_state(&self.state);
    }
}

fn backend_config(config: &RuntimeConfig, backend: BackendKind) -> BackendConfig {
    match backend {
        BackendKind::Synthv1 => BackendConfig {
            command: config.synth_command.clone(),
            client_name: config.client_name.clone(),
            midi_output_match: config.midi_output_match.clone(),
            preset_roots: config.preset_dir.iter().cloned().collect(),
        },
        BackendKind::Yoshimi => config.yoshimi.backend.clone(),
        BackendKind::FluidSynth => config.fluidsynth.backend.clone(),
    }
}

fn backend_command(preset: &Preset, state: &Path, config: &RuntimeConfig) -> Result<Command> {
    let backend = backend_config(config, preset.backend);
    let mut command = Command::new(&backend.command);
    match &preset.id {
        PresetId::Synthv1 { path } => {
            command
                .args(["--no-gui", "--client-name", &backend.client_name])
                .arg(path)
                .env("XDG_CONFIG_HOME", state.join("config"));
        }
        PresetId::Yoshimi { path } => {
            command
                .args(["--no-gui", "--cmdline", "--jack-audio", "--alsa-midi"])
                .arg(format!("--name-tag={}", backend.client_name))
                .arg(format!("--load-instrument={}", safe_command_path(path)?));
        }
        PresetId::FluidSynth { .. } => {
            command.args([
                "--audio-driver=jack",
                "--midi-driver=alsa_seq",
                "--portname",
                &backend.client_name,
                "-o",
                &format!("audio.jack.id={}", backend.client_name),
                "-o",
                "synth.midi-bank-select=mma",
            ]);
            command
                .arg("--gain")
                .arg(config.fluidsynth.gain.to_string());
            command
                .arg("--load-config")
                .arg(state.join("fluidsynth.conf"));
        }
    }
    Ok(command)
}

fn safe_command_path(path: &Path) -> Result<String> {
    let path = path.to_string_lossy();
    if path.contains(['\n', '\r']) {
        bail!("preset path contains a newline")
    }
    Ok(path.into_owned())
}

fn write_fluidsynth_config(state: &Path, soundfonts: &[(PathBuf, u16)]) -> Result<()> {
    let mut file = File::create(state.join("fluidsynth.conf"))?;
    for (path, offset) in soundfonts {
        let path = safe_command_path(path)?;
        writeln!(file, "load \"{}\" 0 {}", path.replace('"', "\\\""), offset)?;
    }
    Ok(())
}

fn wait_ready(
    child: &mut Child,
    backend: BackendKind,
    client_name: &str,
    timeout: Duration,
    log_path: &Path,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            bail!(
                "{} exited with {status}; see {}",
                backend.label(),
                log_path.display()
            );
        }
        if resolve_managed_audio_outputs(client_name, jack_ports()).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    terminate(child);
    bail!(
        "{} did not register an unambiguous JACK stereo output; see {}",
        backend.label(),
        log_path.display()
    )
}

fn mapped_parameter_messages(
    routes: &[(u8, u8)],
    values: &std::collections::HashMap<u8, f32>,
) -> Vec<[u8; 3]> {
    routes
        .iter()
        .filter_map(|&(incoming, target)| {
            let value = values.get(&target)?;
            let control = control::by_cc(target)?;
            Some([0xb0, incoming, control::value_to_cc(control, *value)])
        })
        .collect()
}

fn write_synthv1_config(home: &Path, controller: &PadConfig) -> Result<()> {
    let dir = home.join("rncbc.org");
    fs::create_dir_all(&dir)?;
    let mut file = File::create(dir.join("synthv1.conf"))?;
    writeln!(file, "[Default]\nControlsEnabled=true\n\n[Controllers]")?;
    for (incoming, target) in &controller.controls {
        let Some(control) = CONTROLS.iter().find(|control| control.cc == *target) else {
            continue;
        };
        writeln!(file, "Control_0_CC_{incoming}={}, 4", control.index)?;
    }
    Ok(())
}

fn midi_input_names() -> Result<Vec<String>> {
    let input = MidiInput::new("SHR-DAW MIDI discovery")?;
    input
        .ports()
        .iter()
        .map(|port| input.port_name(port).map_err(anyhow::Error::from))
        .collect()
}

pub fn inspect_midi_inputs(
    config: &RuntimeConfig,
    pads: &PadConfig,
) -> Result<MidiInputAvailability> {
    Ok(plan_midi_inputs(&midi_input_names()?, pads, config).availability)
}

fn plan_midi_inputs(names: &[String], pads: &PadConfig, config: &RuntimeConfig) -> MidiInputPlan {
    let mut plan = MidiInputPlan::default();
    let controller_wanted = pads
        .input_match
        .as_ref()
        .map(|wanted| vec![wanted.clone()])
        .unwrap_or_else(|| config.midi_input_matches.clone());
    if !controller_wanted.is_empty() {
        let mut state = MidiInputState {
            wanted: controller_wanted.join(", "),
            resolved: None,
            error: None,
        };
        for wanted in &controller_wanted {
            match unique_name_match(names, wanted, "controller MIDI input") {
                Ok(Some(index)) => {
                    let name = names[index].clone();
                    state.resolved = Some(name.clone());
                    merge_planned_input(
                        &mut plan.inputs,
                        name,
                        InputRoles {
                            controller: true,
                            performance: config.midi_controller_musical_input,
                        },
                    );
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    state.error = Some(error.to_string());
                    break;
                }
            }
        }
        if state.resolved.is_none() && state.error.is_none() {
            state.error = Some(format!("not found (wanted: {})", state.wanted));
        }
        plan.availability.controller = Some(state);
    }
    for wanted in &config.midi_performance_input_matches {
        let mut state = MidiInputState {
            wanted: wanted.clone(),
            resolved: None,
            error: None,
        };
        match unique_name_match(names, wanted, "performance MIDI input") {
            Ok(Some(index)) => {
                let name = names[index].clone();
                state.resolved = Some(name.clone());
                merge_planned_input(
                    &mut plan.inputs,
                    name,
                    InputRoles {
                        controller: false,
                        performance: true,
                    },
                );
            }
            Ok(None) => state.error = Some(format!("not found (wanted: {wanted})")),
            Err(error) => state.error = Some(error.to_string()),
        }
        plan.availability.performance.push(state);
    }
    plan
}

fn merge_planned_input(inputs: &mut Vec<PlannedMidiInput>, name: String, roles: InputRoles) {
    if let Some(existing) = inputs.iter_mut().find(|input| input.name == name) {
        existing.roles.controller |= roles.controller;
        existing.roles.performance |= roles.performance;
    } else {
        inputs.push(PlannedMidiInput { name, roles });
    }
}

fn mark_input_open_error(
    availability: &mut MidiInputAvailability,
    planned: &PlannedMidiInput,
    error: String,
) {
    if planned.roles.controller {
        if let Some(state) = availability.controller.as_mut() {
            state.error = Some(error.clone());
        }
    }
    if planned.roles.performance {
        for state in &mut availability.performance {
            if state.resolved.as_deref() == Some(&planned.name) {
                state.error = Some(error.clone());
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum MidiDelivery {
    Raw(Vec<u8>),
    Direct(Vec<u8>),
    Tracker(crate::sequencer::PageTarget, Vec<u8>),
}

fn controller_allows_musical(
    roles: InputRoles,
    mapped_control: bool,
    forced_note_release: bool,
) -> bool {
    roles.performance || mapped_control || forced_note_release
}

fn controller_learning_owns_message(roles: InputRoles, learning: bool) -> bool {
    roles.controller && learning
}

fn decrement_destination<K: Ord + Clone>(
    counts: &mut std::collections::BTreeMap<K, usize>,
    destination: &K,
) -> bool {
    let Some(count) = counts.get_mut(destination) else {
        return true;
    };
    *count = count.saturating_sub(1);
    if *count == 0 {
        counts.remove(destination);
        true
    } else {
        false
    }
}

fn release_tracker_notes(
    state: &mut LiveMidiState,
    notes: Vec<(crate::sequencer::PageTarget, u8, u8)>,
) -> Vec<MidiDelivery> {
    notes
        .into_iter()
        .filter_map(|destination| {
            let (target, channel, note) = destination.clone();
            decrement_destination(&mut state.tracker_destinations, &destination).then(|| {
                let message = vec![0x80 | channel, note, 0];
                vec![
                    MidiDelivery::Raw(message.clone()),
                    MidiDelivery::Tracker(target, message),
                ]
            })
        })
        .flatten()
        .collect()
}

fn release_direct_notes(state: &mut LiveMidiState, notes: Vec<(u8, u8)>) -> Vec<MidiDelivery> {
    notes
        .into_iter()
        .filter_map(|destination @ (channel, note)| {
            decrement_destination(&mut state.direct_destinations, &destination).then(|| {
                let message = vec![0x80 | channel, note, 0];
                vec![
                    MidiDelivery::Raw(message.clone()),
                    MidiDelivery::Direct(message),
                ]
            })
        })
        .flatten()
        .collect()
}

fn release_source_channel(
    state: &mut LiveMidiState,
    source: &String,
    channel: u8,
) -> Vec<MidiDelivery> {
    let tracker = state.tracker_notes.drain_source_channel(source, channel);
    let direct = state.direct_notes.drain_source_channel(source, channel);
    let mut deliveries = release_tracker_notes(state, tracker);
    deliveries.extend(release_direct_notes(state, direct));
    if state.sustain_sources.remove(&(source.clone(), channel)) {
        state.sustain_counts[usize::from(channel)] =
            state.sustain_counts[usize::from(channel)].saturating_sub(1);
        if state.sustain_counts[usize::from(channel)] == 0 {
            let message = vec![0xb0 | channel, 64, 0];
            deliveries.push(MidiDelivery::Raw(message.clone()));
            deliveries.push(MidiDelivery::Direct(message));
        }
    }
    deliveries
}

fn release_source(state: &mut LiveMidiState, source: &String) -> Vec<MidiDelivery> {
    let tracker = state.tracker_notes.drain_source(source);
    let direct = state.direct_notes.drain_source(source);
    let mut deliveries = release_tracker_notes(state, tracker);
    deliveries.extend(release_direct_notes(state, direct));
    for channel in 0..16u8 {
        if state.sustain_sources.remove(&(source.clone(), channel)) {
            state.sustain_counts[usize::from(channel)] =
                state.sustain_counts[usize::from(channel)].saturating_sub(1);
            if state.sustain_counts[usize::from(channel)] == 0 {
                let message = vec![0xb0 | channel, 64, 0];
                deliveries.push(MidiDelivery::Raw(message.clone()));
                deliveries.push(MidiDelivery::Direct(message));
            }
        }
    }
    state.tracker_next_column.remove(source);
    deliveries
}

fn release_all_inputs(state: &mut LiveMidiState) -> Vec<MidiDelivery> {
    let tracker = state.tracker_notes.drain();
    let mut deliveries = release_tracker_notes(state, tracker);
    let direct = state.direct_notes.drain();
    deliveries.extend(release_direct_notes(state, direct));
    for channel in 0..16u8 {
        if state.sustain_counts[usize::from(channel)] > 0 {
            let message = vec![0xb0 | channel, 64, 0];
            deliveries.push(MidiDelivery::Raw(message.clone()));
            deliveries.push(MidiDelivery::Direct(message));
        }
    }
    state.sustain_sources.clear();
    state.sustain_counts.fill(0);
    state.tracker_next_column.clear();
    state.tracker_programs.clear();
    deliveries
}

fn route_live_message(
    state: &mut LiveMidiState,
    source: &String,
    message: &[u8],
    route: Option<&TrackerRoute>,
    playback_scale: Option<crate::scale::Scale>,
) -> Vec<MidiDelivery> {
    let mut deliveries = Vec::new();
    let revision = route.map_or(0, |route| route.revision);
    if revision != state.tracker_revision {
        let tracker = state.tracker_notes.drain();
        deliveries.extend(release_tracker_notes(state, tracker));
        let direct = state.direct_notes.drain();
        deliveries.extend(release_direct_notes(state, direct));
        state.tracker_next_column.clear();
        state.tracker_programs.clear();
        state.tracker_revision = revision;
        state.tracker_navigation_revision = route.map_or(0, |route| route.navigation_revision);
    } else {
        let navigation_revision = route.map_or(0, |route| route.navigation_revision);
        if navigation_revision != state.tracker_navigation_revision {
            state.tracker_next_column.clear();
            state.tracker_navigation_revision = navigation_revision;
        }
    }
    let status = message.first().copied().unwrap_or(0);
    let channel = status & 0x0f;
    if message.len() == 3 && status & 0xf0 == 0xb0 && matches!(message[1], 120 | 123) {
        deliveries.extend(release_source_channel(state, source, channel));
        return deliveries;
    }
    if valid_note_message(message) && route.is_some_and(|route| route.enabled) {
        let source_note = message[1];
        let note_off = status & 0xf0 == 0x80 || message[2] == 0;
        if note_off {
            if let Some(destination) = state.tracker_notes.note_off(source, channel, source_note) {
                let (target, output_channel, output_note) = destination.clone();
                if decrement_destination(&mut state.tracker_destinations, &destination) {
                    let output = vec![status & 0xf0 | output_channel, output_note, message[2]];
                    deliveries.push(MidiDelivery::Raw(output.clone()));
                    deliveries.push(MidiDelivery::Tracker(target, output));
                }
            }
            return deliveries;
        }
        let route = route.expect("enabled tracker route");
        let Some(mapped_note) = route.mapped_note(source_note) else {
            return deliveries;
        };
        let next = state
            .tracker_next_column
            .entry(source.clone())
            .or_insert(route.start_column);
        let column = route.column(*next);
        *next = (*next + 1) % crate::sequencer::LANES_PER_PAGE;
        let destination = (route.target.clone(), column.channel, mapped_note);
        state
            .tracker_notes
            .note_on(source, channel, source_note, destination.clone());
        let destination_count = state
            .tracker_destinations
            .entry(destination.clone())
            .or_default();
        let first_owner = *destination_count == 0;
        *destination_count += 1;
        if !first_owner {
            return deliveries;
        }
        if let Some(program) = column.program.filter(|program| {
            state
                .tracker_programs
                .get(&(route.target.clone(), column.channel))
                != Some(program)
        }) {
            match route.bank_select {
                crate::config::BankSelectMode::Off => {}
                crate::config::BankSelectMode::Cc0 => deliveries.push(MidiDelivery::Tracker(
                    route.target.clone(),
                    vec![0xb0 | column.channel, 0, column.bank_msb],
                )),
                crate::config::BankSelectMode::Cc0Cc32 => {
                    deliveries.push(MidiDelivery::Tracker(
                        route.target.clone(),
                        vec![0xb0 | column.channel, 0, column.bank_msb],
                    ));
                    deliveries.push(MidiDelivery::Tracker(
                        route.target.clone(),
                        vec![0xb0 | column.channel, 32, column.bank_lsb],
                    ));
                }
            }
            deliveries.push(MidiDelivery::Tracker(
                route.target.clone(),
                vec![0xc0 | column.channel, program],
            ));
            state
                .tracker_programs
                .insert((route.target.clone(), column.channel), program);
        }
        let output = vec![status & 0xf0 | column.channel, mapped_note, message[2]];
        deliveries.push(MidiDelivery::Raw(output.clone()));
        deliveries.push(MidiDelivery::Tracker(route.target.clone(), output));
        return deliveries;
    }
    if valid_note_message(message) {
        if !playback_filter_allows(playback_scale, message) {
            return deliveries;
        }
        let note = message[1];
        let destination = (channel, note);
        let note_off = status & 0xf0 == 0x80 || message[2] == 0;
        if note_off {
            let owned = state.direct_notes.note_off(source, channel, note);
            if owned.is_some() {
                if !decrement_destination(&mut state.direct_destinations, &destination) {
                    return deliveries;
                }
            } else if state.direct_destinations.contains_key(&destination) {
                return deliveries;
            }
        } else {
            state
                .direct_notes
                .note_on(source, channel, note, destination);
            let destination_count = state.direct_destinations.entry(destination).or_default();
            let first_owner = *destination_count == 0;
            *destination_count += 1;
            if !first_owner {
                return deliveries;
            }
        }
        deliveries.push(MidiDelivery::Raw(message.to_vec()));
        deliveries.push(MidiDelivery::Direct(message.to_vec()));
        return deliveries;
    }
    if message.len() == 3 && status & 0xf0 == 0xb0 && message[1] == 64 {
        let key = (source.clone(), channel);
        if message[2] >= 64 {
            if state.sustain_sources.insert(key) {
                state.sustain_counts[usize::from(channel)] += 1;
                if state.sustain_counts[usize::from(channel)] > 1 {
                    return deliveries;
                }
            } else {
                return deliveries;
            }
        } else if state.sustain_sources.remove(&key) {
            state.sustain_counts[usize::from(channel)] =
                state.sustain_counts[usize::from(channel)].saturating_sub(1);
            if state.sustain_counts[usize::from(channel)] > 0 {
                return deliveries;
            }
        } else if state.sustain_counts[usize::from(channel)] > 0 {
            return deliveries;
        }
    }
    deliveries.push(MidiDelivery::Raw(message.to_vec()));
    deliveries.push(MidiDelivery::Direct(message.to_vec()));
    deliveries
}

fn deliver_midi(
    deliveries: Vec<MidiDelivery>,
    received: Instant,
    tx: &Sender<MidiEvent>,
    output: &SharedOutput,
    tracker_input: &SharedTrackerInput,
) {
    for delivery in deliveries {
        match delivery {
            MidiDelivery::Raw(bytes) => {
                let _ = tx.send(MidiEvent::Raw { received, bytes });
            }
            MidiDelivery::Direct(bytes) => {
                if let Ok(mut output) = output.lock() {
                    if let Some(output) = output.as_mut() {
                        if let Err(error) = output.send(&bytes) {
                            let _ = tx.send(MidiEvent::Error(format!("MIDI forward: {error}")));
                        }
                    }
                }
            }
            MidiDelivery::Tracker(target, bytes) => {
                if let Ok(input) = tracker_input.lock() {
                    if let Some(input) = input.as_ref() {
                        input.send(&target, &bytes);
                    }
                }
            }
        }
    }
}

fn connect_midi_input(
    tx: Sender<MidiEvent>,
    planned: &PlannedMidiInput,
    config: &RuntimeConfig,
    routing: CallbackRouting,
) -> Result<MidiInputConnection<()>> {
    let CallbackRouting {
        output,
        pickup,
        backend,
        tracker_route,
        tracker_input,
        playback_scale,
        controller: callback_controller,
        learn_mode,
        fx_control_mode,
        live_state,
    } = routing;
    let mut input = MidiInput::new("SHR-DAW MIDI input")?;
    input.ignore(Ignore::None);
    let ports = input.ports();
    let names = ports
        .iter()
        .map(|port| input.port_name(port).map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;
    let port_index = unique_name_match(&names, &planned.name, "MIDI input")?
        .ok_or_else(|| anyhow!("MIDI input disappeared before open"))?;
    let port = &ports[port_index];
    let input_name = names[port_index].clone();
    let source_id = input_name.clone();
    let roles = planned.roles;
    let mut pad_locked = false;
    let mut lock_pressed = false;
    let mut page_cycle_chord = crate::pads::PageCycleChordState::default();
    let mut locked_pad_notes = std::collections::HashMap::new();
    let connection = input
        .connect(
            port,
            "SHR-DAW monitor",
            move |_stamp, message, _| {
                let received = Instant::now();
                if controller_learning_owns_message(roles, learn_mode.load(Ordering::Relaxed)) {
                    let _ = tx.send(MidiEvent::Learn {
                        received,
                        bytes: message.to_vec(),
                    });
                    return;
                }
                if invalid_note_message(message) {
                    let _ = tx.send(MidiEvent::Error(
                        "ignored malformed MIDI note message".into(),
                    ));
                    return;
                }
                let mut musical = roles.performance.then(|| message.to_vec());
                if roles.controller {
                    let Ok(pads) = callback_controller.read() else {
                        let _ = tx.send(MidiEvent::Error("controller mapping lock failed".into()));
                        return;
                    };
                    let backend = backend
                        .lock()
                        .map(|kind| *kind)
                        .unwrap_or(BackendKind::Synthv1);
                    let (chord_message, chord_action) =
                        pads.page_cycle_chord_action(message, &mut page_cycle_chord);
                    if chord_message {
                        if let Some((action, pressed)) = chord_action {
                            let _ = tx.send(MidiEvent::Pad(action, pressed));
                        }
                        return;
                    }
                    let (lock_message, lock_down) = pads.lock_action(message);
                    if lock_message && lock_down && !lock_pressed {
                        pad_locked = !pad_locked;
                        let _ = tx.send(MidiEvent::PadLock(pad_locked));
                    }
                    if lock_message {
                        lock_pressed = lock_down;
                    }
                    let forced_pad_release =
                        locked_pad_release(&pads, message, pad_locked, &mut locked_pad_notes);
                    let fx_value = (fx_control_mode.load(Ordering::Relaxed)
                        && message.len() >= 3
                        && message[0] & 0xf0 == 0xb0)
                        .then(|| pads.target_cc(message[1]))
                        .flatten()
                        .and_then(control::by_cc)
                        .map(|control| (control.cc, control::value_from_cc(control, message[2])));
                    if let Some((cc, value)) = fx_value {
                        let _ = tx.send(MidiEvent::MappedControl(cc, value));
                        return;
                    }
                    let routed =
                        crate::midi::route_with_pad_lock(&pads, backend, message, pad_locked);
                    if let Some((cc, value)) = routed.value {
                        let _ = tx.send(MidiEvent::MappedControl(cc, value));
                    }
                    let accepted = routed
                        .value
                        .map(|(cc, value)| {
                            pickup
                                .lock()
                                .map(|mut pickup| pickup.accept(cc, value))
                                .unwrap_or(true)
                        })
                        .unwrap_or(true);
                    if !pad_locked {
                        if let Some((action, pressed)) = pads.action_state(message) {
                            let _ = tx.send(MidiEvent::Pad(action, pressed));
                        }
                    }
                    if let Some(action) = routed.encoder {
                        let _ = tx.send(MidiEvent::Encoder(action));
                    }
                    if !accepted {
                        return;
                    }
                    if let Some((cc, value)) = routed.value {
                        let _ = tx.send(MidiEvent::Value(cc, value));
                    }
                    let mapped = routed.value.is_some() || routed.translated.is_some();
                    musical = routed
                        .translated
                        .map(|bytes| bytes.to_vec())
                        .or_else(|| {
                            controller_allows_musical(roles, mapped, forced_pad_release)
                                .then(|| routed.forward.map(<[u8]>::to_vec))
                                .flatten()
                        })
                        .or_else(|| forced_pad_release.then(|| message.to_vec()));
                }
                let Some(message) = musical else {
                    return;
                };
                let route = tracker_route.lock().ok().map(|route| route.clone());
                let scale = playback_scale.lock().ok().and_then(|scale| *scale);
                let deliveries = live_state
                    .lock()
                    .map(|mut state| {
                        route_live_message(&mut state, &source_id, &message, route.as_ref(), scale)
                    })
                    .unwrap_or_default();
                deliver_midi(deliveries, received, &tx, &output, &tracker_input);
            },
            (),
        )
        .map_err(|error| anyhow!("connect MIDI input: {error}"))?;
    for client in [
        &config.client_name,
        &config.yoshimi.backend.client_name,
        &config.fluidsynth.backend.client_name,
    ] {
        disconnect_direct_midi(&input_name, client);
    }
    Ok(connection)
}

fn locked_pad_release(
    pads: &PadConfig,
    message: &[u8],
    pad_locked: bool,
    active: &mut std::collections::HashMap<(u8, u8), usize>,
) -> bool {
    if !valid_note_message(message) {
        return false;
    }
    let key = (message[0] & 0x0f, message[1]);
    let note_on = message[0] & 0xf0 == 0x90 && message[2] > 0;
    if note_on {
        if pad_locked && pads.action_state(message).is_some() {
            *active.entry(key).or_default() += 1;
        }
        return false;
    }
    let Some(count) = active.get_mut(&key) else {
        return false;
    };
    *count -= 1;
    if *count == 0 {
        active.remove(&key);
    }
    true
}

fn disconnect_direct_midi(source: &str, client_name: &str) {
    disconnect_midi_routes(source, &[client_name]);
}

/// Removes only subscriptions whose source and destination names match the
/// supplied SHR-DAW-owned/configured clients. Other clients and routes are
/// never reconfigured.
pub fn disconnect_midi_routes(source_match: &str, destination_matches: &[&str]) {
    let lines = command_lines("aconnect", &["-l"]);
    let clients = parse_alsa_clients(&lines);
    let sources = parse_alsa_source_ports(&lines);
    let source_names = sources
        .iter()
        .map(|(_, _, name)| name.clone())
        .collect::<Vec<_>>();
    let wanted = crate::controller_learn::stable_input_match(source_match);
    let Ok(Some(source_index)) = unique_name_match(&source_names, &wanted, "MIDI source") else {
        return;
    };
    let (source_id, source_port, _) = &sources[source_index];
    let destination_ids = destination_matches
        .iter()
        .filter_map(|wanted| unique_client_match(&clients, wanted).map(|(id, _)| *id))
        .collect::<std::collections::HashSet<_>>();
    for destination_id in destination_ids {
        let _ = Command::new("aconnect")
            .args([
                "-d",
                &format!("{source_id}:{source_port}"),
                &format!("{destination_id}:0"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn parse_alsa_source_ports(lines: &[String]) -> Vec<(u32, u32, String)> {
    let mut sources = Vec::new();
    let mut client = None;
    for line in lines {
        if let Some(raw) = line.strip_prefix("client ") {
            let Some((id, rest)) = raw.split_once(':') else {
                client = None;
                continue;
            };
            let name = rest
                .split('\'')
                .nth(1)
                .map(str::to_owned)
                .unwrap_or_default();
            client = id.parse::<u32>().ok().map(|id| (id, name));
            continue;
        }
        let Some((client_id, client_name)) = client.as_ref() else {
            continue;
        };
        let trimmed = line.trim_start();
        let Some((port, rest)) = trimmed.split_once(' ') else {
            continue;
        };
        let Ok(port) = port.parse::<u32>() else {
            continue;
        };
        let port_name = rest
            .split('\'')
            .nth(1)
            .map(str::to_owned)
            .unwrap_or_default();
        if !port_name.is_empty() {
            sources.push((*client_id, port, format!("{client_name}:{port_name}")));
        }
    }
    sources
}

/// Keeps only the configured destination on an SHR-DAW-owned ALSA source port.
/// Desktop auto-subscriptions are removed without touching another client's
/// source routes.
pub fn retain_midi_destination(source_match: &str, destination_match: &str) {
    let lines = command_lines("aconnect", &["-l"]);
    let clients = parse_alsa_clients(&lines);
    let Some((source_id, _)) = unique_client_match(&clients, source_match) else {
        return;
    };
    let Some((destination_id, _)) = unique_client_match(&clients, destination_match) else {
        return;
    };
    for (client, port) in parse_alsa_destinations(&lines, *source_id) {
        if client != *destination_id {
            let _ = Command::new("aconnect")
                .args(["-d", &format!("{source_id}:0"), &format!("{client}:{port}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn unique_client_match<'a>(
    clients: &'a [(u32, String)],
    wanted: &str,
) -> Option<&'a (u32, String)> {
    let wanted = wanted.to_ascii_lowercase();
    if wanted.is_empty() {
        return None;
    }
    let exact = clients
        .iter()
        .filter(|(_, name)| name.to_ascii_lowercase() == wanted)
        .collect::<Vec<_>>();
    match exact.as_slice() {
        [client] => return Some(*client),
        [] => {}
        _ => return None,
    }
    let partial = clients
        .iter()
        .filter(|(_, name)| name.to_ascii_lowercase().contains(&wanted))
        .collect::<Vec<_>>();
    match partial.as_slice() {
        [client] => Some(*client),
        _ => None,
    }
}

fn parse_alsa_clients(lines: &[String]) -> Vec<(u32, String)> {
    lines
        .iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("client ")?;
            let (id, description) = rest.split_once(':')?;
            let name = description.split('\'').nth(1)?;
            Some((id.parse().ok()?, name.to_owned()))
        })
        .collect()
}

fn parse_alsa_destinations(lines: &[String], source_id: u32) -> Vec<(u32, u32)> {
    let mut current_client = None;
    let mut destinations = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("client ") {
            current_client = rest
                .split_once(':')
                .and_then(|(id, _)| id.parse::<u32>().ok());
            continue;
        }
        if current_client != Some(source_id) {
            continue;
        }
        let Some(raw) = line.trim().strip_prefix("Connecting To:") else {
            continue;
        };
        for destination in raw.split(',') {
            let destination = destination.trim().split('[').next().unwrap_or("");
            let Some((client, port)) = destination.split_once(':') else {
                continue;
            };
            if let (Ok(client), Ok(port)) = (client.parse(), port.parse()) {
                if !destinations.contains(&(client, port)) {
                    destinations.push((client, port));
                }
            }
        }
    }
    destinations
}

fn attach_midi_output(
    shared: &SharedOutput,
    output_match: &str,
    backend: BackendKind,
) -> Result<()> {
    let output = MidiOutput::new("SHR-DAW MIDI output")?;
    let ports = output.ports();
    let names = ports
        .iter()
        .map(|port| output.port_name(port).map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;
    let index =
        unique_backend_name_match(&names, output_match, "MIDI output")?.ok_or_else(|| {
            anyhow!(
                "{} MIDI output matching {output_match:?} not found",
                backend.label()
            )
        })?;
    let connection = output
        .connect(&ports[index], "SHR-DAW forward")
        .map_err(|error| anyhow!("connect {} MIDI output: {error}", backend.label()))?;
    *shared
        .lock()
        .map_err(|_| anyhow!("MIDI output lock poisoned"))? = Some(connection);
    Ok(())
}

fn unique_name_match(names: &[String], wanted: &str, description: &str) -> Result<Option<usize>> {
    crate::midi_endpoint::matching_optional_index(names, wanted, description)
}

/// Managed synth configuration historically uses short selectors such as
/// `synthv1` and `yoshimi`, while the programs publish generated ALSA
/// identities such as `shs-synthv1:in`. Preserve strict stable identities for
/// physical devices, but accept one unique contained selector for an owned
/// backend whose process and full destination list SHR controls.
fn unique_backend_name_match(
    names: &[String],
    wanted: &str,
    description: &str,
) -> Result<Option<usize>> {
    if let Some(index) = unique_name_match(names, wanted, description)? {
        return Ok(Some(index));
    }
    let wanted = wanted.trim().to_ascii_lowercase();
    let matches = names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            crate::midi_endpoint::stable_identity(name)
                .to_ascii_lowercase()
                .contains(&wanted)
                .then_some(index)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [index] => Ok(Some(*index)),
        _ => bail!(
            "{description} selector {wanted:?} is ambiguous: {}",
            matches
                .iter()
                .map(|index| crate::midi_endpoint::stable_identity(&names[*index]))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn command_lines(program: &str, args: &[&str]) -> Vec<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn jack_ports() -> Vec<String> {
    command_lines("jack_lsp", &[])
}

/// Read-only discovery of JACK audio source ports. These names are candidates
/// for deliberate recorder assignment; discovery never mutates configuration
/// and never substitutes one source for another.
pub(crate) fn jack_capture_sources() -> Vec<String> {
    parse_jack_audio_sources(command_lines("jack_lsp", &["-p", "-t"]))
}

fn parse_jack_audio_sources(lines: Vec<String>) -> Vec<String> {
    fn finish(port: &mut String, properties: &mut String, audio: &mut bool, out: &mut Vec<String>) {
        if !port.is_empty()
            && *audio
            && properties
                .split(|character: char| character == ',' || character.is_whitespace())
                .any(|property| property.eq_ignore_ascii_case("output"))
        {
            out.push(std::mem::take(port));
        } else {
            port.clear();
        }
        properties.clear();
        *audio = false;
    }

    let mut sources = Vec::new();
    let mut port = String::new();
    let mut properties = String::new();
    let mut audio = false;
    for line in lines {
        if !line.starts_with(char::is_whitespace) {
            finish(&mut port, &mut properties, &mut audio, &mut sources);
            port = line;
        } else {
            let detail = line.trim();
            if detail.to_ascii_lowercase().starts_with("properties:") {
                properties.push_str(detail);
            } else if detail.to_ascii_lowercase().contains("audio") {
                audio = true;
            }
        }
    }
    finish(&mut port, &mut properties, &mut audio, &mut sources);
    sources.sort();
    sources.dedup();
    sources
}

fn connect_audio(client_name: &str, config: &RuntimeConfig) -> Result<()> {
    if !config.audio_autoconnect {
        return Ok(());
    }
    let outputs = managed_audio_outputs(client_name)?;
    for (source, destination) in outputs.iter().zip(config.audio_outputs.iter()) {
        let status = Command::new("jack_connect")
            .args([source.as_str(), destination.as_str()])
            .status()
            .with_context(|| format!("connect JACK audio {source} -> {destination}"))?;
        if !status.success() && !jack_connection_exists(source, destination) {
            bail!("connect JACK audio {source} -> {destination} exited with {status}");
        }
    }
    Ok(())
}

fn jack_connection_exists(source: &str, destination: &str) -> bool {
    parse_jack_connections(command_lines("jack_lsp", &["-c"]))
        .iter()
        .any(|(connected_source, connected_destination)| {
            connected_source == source && connected_destination == destination
        })
}

fn parse_jack_connections(lines: Vec<String>) -> Vec<(String, String)> {
    let mut port: Option<String> = None;
    let mut connections = Vec::new();
    for line in lines {
        if line.starts_with(char::is_whitespace) {
            if let Some(source) = port.as_ref() {
                let destination = line.trim();
                if !destination.is_empty() {
                    connections.push((source.clone(), destination.to_owned()));
                }
            }
        } else {
            port = Some(line);
        }
    }
    connections
}

fn start_managed_audio_graph(
    client_name: &str,
    config: &RuntimeConfig,
    rack: &InsertRack,
    aux_routing: &ProjectAuxRouting,
) -> Result<OwnedAudioGraph> {
    let available = jack_ports();
    let source_ports = resolve_managed_audio_outputs(client_name, available.clone())?;
    let (loop_source_ports, live_source_ports) =
        resolve_performance_bus_sources(config, &available)?;
    let destinations: [String; 2] = config
        .audio_outputs
        .clone()
        .try_into()
        .map_err(|_| anyhow!("owned graph requires exactly two configured main outputs"))?;
    let loop_destinations: [String; 2] = config
        .loop_player
        .outputs
        .clone()
        .try_into()
        .map_err(|_| anyhow!("final bus requires exactly two configured loop.output routes"))?;
    OwnedAudioGraph::start_with_routing(
        &config.audio_graph,
        PerformanceBusPorts {
            synth: source_ports,
            loop_player: loop_source_ports,
            live_input: live_source_ports,
            playback: destinations,
            loop_direct_playback: loop_destinations,
        },
        &config.capture,
        rack,
        aux_routing,
    )
}

fn resolve_performance_bus_sources(
    config: &RuntimeConfig,
    available: &[String],
) -> Result<([String; 2], [String; 2])> {
    let loop_source_ports = crate::loop_player::configured_output_ports(&config.loop_player);
    for port in &loop_source_ports {
        if !available.iter().any(|candidate| candidate == port) {
            bail!("owned WAV loop source {port:?} is offline; load the configured loop before activating the final bus");
        }
    }
    let input = config
        .audio_graph
        .input
        .as_ref()
        .or_else(|| config.capture.inputs.first())
        .context("final bus needs one configured stereo JACK input")?;
    let live_source_ports = [input.left_port.clone(), input.right_port.clone()];
    for port in &live_source_ports {
        if !available.iter().any(|candidate| candidate == port) {
            bail!("configured final-bus input {port:?} is offline; no nearby JACK port is substituted");
        }
    }
    Ok((loop_source_ports, live_source_ports))
}

fn managed_audio_outputs(client_name: &str) -> Result<[String; 2]> {
    resolve_managed_audio_outputs(client_name, jack_ports())
}

fn resolve_managed_audio_outputs(
    client_name: &str,
    ports: impl IntoIterator<Item = String>,
) -> Result<[String; 2]> {
    let wanted = client_name.to_ascii_lowercase();
    let mut exact_candidates = Vec::new();
    let mut partial_candidates = Vec::new();
    for port in ports {
        let Some((client, short_name)) = port.split_once(':') else {
            continue;
        };
        let short_name = short_name.to_ascii_lowercase();
        let is_output = short_name.contains("out")
            || short_name.contains("audio")
            || short_name == "left"
            || short_name == "right";
        let client_lower = client.to_ascii_lowercase();
        if is_output {
            if client_lower == wanted {
                exact_candidates.push((client.to_owned(), port));
            } else if client_lower.contains(&wanted) {
                partial_candidates.push((client.to_owned(), port));
            }
        }
    }
    let candidates = if exact_candidates.is_empty() {
        partial_candidates
    } else {
        exact_candidates
    };
    let mut matching_clients = candidates
        .iter()
        .map(|(client, _)| client.clone())
        .collect::<Vec<_>>();
    matching_clients.sort();
    matching_clients.dedup();
    if matching_clients.len() != 1 {
        bail!(
            "managed JACK client {client_name:?} matched {} audio clients, expected 1",
            matching_clients.len()
        );
    }
    let matched = &matching_clients[0];
    let mut outputs = candidates
        .into_iter()
        .filter_map(|(client, port)| (client == *matched).then_some(port))
        .collect::<Vec<_>>();
    outputs.sort();
    outputs.dedup();
    if outputs.len() != 2 {
        bail!(
            "managed JACK client {matched:?} has {} unambiguous audio outputs, expected 2",
            outputs.len()
        );
    }
    outputs
        .try_into()
        .map_err(|_| anyhow!("managed JACK output pair changed during resolution"))
}

fn terminate(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn cleanup_state(state: &Path) {
    let _ = fs::remove_file(state.join("engine.pid"));
    let _ = fs::remove_file(state.join("current"));
}

#[derive(Debug, Eq, PartialEq)]
struct Owner {
    pid: i32,
    start_time: u64,
    executable: PathBuf,
}

fn proc_start_time(pid: i32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = stat.rsplit_once(')')?.1.trim();
    // Fields after the process name begin at field 3; starttime is field 22.
    after_name.split_whitespace().nth(19)?.parse().ok()
}

fn owner_for(pid: i32) -> Option<Owner> {
    Some(Owner {
        pid,
        start_time: proc_start_time(pid)?,
        executable: fs::read_link(format!("/proc/{pid}/exe")).ok()?,
    })
}

fn stable_owner_for(pid: i32) -> Option<Owner> {
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut previous = None;
    loop {
        let current = owner_for(pid);
        if current.is_some() && current == previous {
            return current;
        }
        previous = current;
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn write_owner(path: &Path, pid: i32) -> Result<()> {
    let owner = stable_owner_for(pid).context("read stable spawned process identity")?;
    fs::write(
        path,
        format!(
            "pid={}\nstart={}\nexe={}\n",
            owner.pid,
            owner.start_time,
            owner.executable.display()
        ),
    )?;
    Ok(())
}

fn read_owner(path: &Path) -> Option<Owner> {
    let text = fs::read_to_string(path).ok()?;
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name).map(str::to_owned))
    };
    Some(Owner {
        pid: field("pid=")?.parse().ok()?,
        start_time: field("start=")?.parse().ok()?,
        executable: PathBuf::from(field("exe=")?),
    })
}

fn still_owned(owner: &Owner) -> bool {
    owner_for(owner.pid).is_some_and(|current| {
        current.pid == owner.pid
            && current.start_time == owner.start_time
            && same_executable(&owner.executable, &current.executable)
    })
}

fn confirm_still_owned(owner: &Owner) -> bool {
    let deadline = Instant::now() + Duration::from_millis(100);
    loop {
        if still_owned(owner) {
            return true;
        }
        if proc_start_time(owner.pid).is_some_and(|start| start != owner.start_time) {
            return false;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn same_executable(recorded: &Path, current: &Path) -> bool {
    executable_bytes(recorded) == executable_bytes(current)
}

fn executable_bytes(path: &Path) -> &[u8] {
    let bytes = path.as_os_str().as_bytes();
    bytes.strip_suffix(b" (deleted)").unwrap_or(bytes)
}

fn stop_owned(path: &Path, timeout: Duration) -> Result<()> {
    let Some(owner) = read_owner(path) else {
        // Legacy bare-PID files are deliberately not trusted: PID reuse could
        // otherwise terminate an unrelated live process.
        let _ = fs::remove_file(path);
        return Ok(());
    };
    if confirm_still_owned(&owner) {
        signal_owned(&owner, libc::SIGTERM)?;
        let deadline = Instant::now() + timeout;
        while still_owned(&owner) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        if still_owned(&owner) {
            signal_owned(&owner, libc::SIGKILL)?;
            let kill_deadline = Instant::now() + Duration::from_secs(2);
            while still_owned(&owner) && Instant::now() < kill_deadline {
                thread::sleep(Duration::from_millis(25));
            }
            if still_owned(&owner) {
                bail!(
                    "owned process {} remained alive after forced shutdown",
                    owner.pid
                );
            }
        }
    }
    fs::remove_file(path).or_else(|error| {
        (error.kind() == std::io::ErrorKind::NotFound)
            .then_some(())
            .ok_or(error)
    })?;
    Ok(())
}

fn signal_owned(owner: &Owner, signal: libc::c_int) -> Result<()> {
    let result = unsafe { libc::kill(owner.pid, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(error).with_context(|| format!("signal owned process {}", owner.pid))
}

pub fn stop_managed(state: &Path) -> Result<()> {
    stop_owned(&state.join("daemon.pid"), Duration::from_secs(3))?;
    stop_owned(&state.join("engine.pid"), Duration::from_secs(2))?;
    cleanup_state(state);
    Ok(())
}

pub fn status(state: &Path) -> String {
    match (
        read_owner(&state.join("engine.pid")),
        fs::read_to_string(state.join("current")),
    ) {
        (Some(owner), Ok(name)) if still_owned(&owner) => format!(
            "Running: {}\nPID: {}\nAudio: JACK\nMIDI: monitored and forwarded by SHR-DAW",
            name.trim(),
            owner.pid
        ),
        _ => "SHR-DAW is stopped.".to_owned(),
    }
}

pub fn daemon(preset: Preset, state: PathBuf, config: RuntimeConfig) -> Result<()> {
    fs::create_dir_all(&state)?;
    let (tx, rx) = std::sync::mpsc::channel();
    // A daemon has no UI event consumer. Keeping this receiver alive queues a
    // copy of every MIDI message for the lifetime of the process.
    drop(rx);
    let router = MidiRouter::start(&state, &config, tx)?;
    if let Ok(mut backend) = router.backend().lock() {
        *backend = preset.backend;
    }
    router.arm_pickup(&initial_values(&preset)?);
    let mut engine = Engine::start(&preset, &state, router.output(), &config)?;
    engine.bind_midi_lifecycle(router.lifecycle());
    if let Some(route) = engine.audio_route_status() {
        eprintln!("AUDIO ROUTE · {route}");
    }
    write_owner(&state.join("daemon.pid"), std::process::id() as i32)?;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&stop))?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&stop))?;
    while !stop.load(std::sync::atomic::Ordering::Relaxed) && engine.alive() {
        if let Some(status) = engine.poll_audio_graph() {
            eprintln!("{status}");
        }
        thread::sleep(Duration::from_millis(100));
    }
    if let Some((timing, restored)) = engine.stop_audio_graph() {
        eprintln!("AUDIO GRAPH METRICS · {}", audio_graph_metrics(&timing));
        if let Err(error) = restored {
            eprintln!("AUDIO GRAPH SHUTDOWN · direct restore unavailable: {error:#}");
        }
    }
    drop(engine);
    let _ = fs::remove_file(state.join("daemon.pid"));
    Ok(())
}

pub(crate) fn audio_graph_metrics(timing: &CallbackTimingSnapshot) -> String {
    format!(
        "callbacks={} mean_us={:.3} p95_us={:.3} p99_us={:.3} max_us={:.3} missed_deadlines={} oversized_callbacks={}",
        timing.callbacks,
        timing.mean_nanoseconds() as f64 / 1_000.0,
        timing.p95_nanoseconds as f64 / 1_000.0,
        timing.p99_nanoseconds as f64 / 1_000.0,
        timing.maximum_nanoseconds as f64 / 1_000.0,
        timing.missed_deadlines,
        timing.oversized_callbacks,
    )
}

pub fn initial_values(preset: &Preset) -> Result<std::collections::HashMap<u8, f32>> {
    preset::values(preset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_edit_route_targets_casio_track_and_sparse_percussion_map() {
        let mut config = RuntimeConfig::default().external_midi;
        config.program_changes = true;
        config.percussion_input_base = 60;
        config.percussion_notes = vec![36, 38, 40];
        let mut route = TrackerRoute::default();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [(2, (9, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: true,
            audition_note: None,
            scale: None,
            external: &config,
        });
        assert!(route.enabled);
        assert_eq!(route.column(0).channel, 2);
        assert_eq!(route.column(0).program, Some(9));
        assert_eq!(route.mapped_note(60), Some(36));
        assert_eq!(route.mapped_note(61), Some(38));
        assert_eq!(route.mapped_note(72), Some(72));
        assert_eq!(route.mapped_note(128), None);
        assert!(tracker_route_consumes_note(Some(&route), &[0x90, 60, 100]));
        assert!(tracker_route_consumes_note(Some(&route), &[0x90, 60, 0]));
        assert!(tracker_route_consumes_note(Some(&route), &[0x80, 60, 0]));
        assert!(!tracker_route_consumes_note(Some(&route), &[0xb0, 1, 64]));
        assert!(!tracker_route_consumes_note(
            Some(&route),
            &[0x90, 128, 100]
        ));
        assert!(invalid_note_message(&[0x90, 128, 100]));
        assert!(invalid_note_message(&[0x80, 60]));
        assert!(invalid_note_message(&[0x90, 60, 100, 0]));
        route.configure(TrackerRouteConfig {
            enabled: false,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [(2, (9, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: true,
            audition_note: None,
            scale: None,
            external: &config,
        });
        assert!(!tracker_route_consumes_note(Some(&route), &[0x90, 60, 100]));
    }

    #[test]
    fn gm_drum_audition_can_pin_keyboard_notes_to_the_selected_drum() {
        let config = RuntimeConfig::default().external_midi;
        let mut route = TrackerRoute::default();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [(9, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: true,
            audition_note: Some(42),
            scale: None,
            external: &config,
        });
        assert_eq!(route.mapped_note(36), Some(42));
        assert_eq!(route.mapped_note(60), Some(42));
        assert_eq!(route.mapped_note(96), Some(42));
    }

    #[test]
    fn noob_routes_suppress_outside_notes_without_remapping_allowed_notes() {
        let config = RuntimeConfig::default().external_midi;
        let scale = crate::scale::Scale {
            root: 1,
            kind: crate::scale::ScaleKind::NaturalMinor,
        };
        let mut route = TrackerRoute::default();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [(0, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            audition_note: None,
            scale: Some(scale),
            external: &config,
        });
        assert_eq!(route.mapped_note(61), Some(61));
        assert_eq!(route.mapped_note(62), None);
        assert_eq!(route.mapped_note(63), Some(63));

        assert!(playback_filter_allows(Some(scale), &[0x90, 61, 100]));
        assert!(!playback_filter_allows(Some(scale), &[0x90, 62, 100]));
        assert!(!playback_filter_allows(Some(scale), &[0x80, 62, 0]));
        assert!(playback_filter_allows(Some(scale), &[0xb0, 1, 64]));
        assert!(playback_filter_allows(None, &[0x90, 62, 100]));
    }

    #[test]
    fn live_route_switch_exposes_the_exact_old_route_for_note_cleanup() {
        let config = RuntimeConfig::default().external_midi;
        let mut route = TrackerRoute::default();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::Synthv1("Pattern Sound".into()),
            columns: [(0, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            audition_note: None,
            scale: None,
            external: &config,
        });
        assert_eq!(
            route.destinations(),
            vec![(
                crate::sequencer::PageTarget::Synthv1("Pattern Sound".into()),
                0
            )]
        );
        assert_eq!(route.preview_state().1, None);

        let old = route.destinations();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [(5, (8, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            audition_note: None,
            scale: None,
            external: &config,
        });
        assert_eq!(old[0].1, 0);
        assert_eq!(
            route.destinations(),
            vec![(crate::sequencer::PageTarget::ConfiguredExternal, 5)]
        );
        assert_eq!(route.preview_state().1, Some(8));
    }

    #[test]
    fn parameter_reset_uses_physical_routes_without_restarting_engine() {
        let messages = mapped_parameter_messages(
            &[(86, 74), (89, 76)],
            &std::collections::HashMap::from([(74, 0.5), (76, 0.0)]),
        );
        assert_eq!(messages, [[0xb0, 86, 64], [0xb0, 89, 64]]);
    }

    #[test]
    fn pad_lock_preserves_note_off_ownership_across_unlock() {
        let pads = PadConfig {
            pads: std::collections::HashMap::from([(36, PadAction::Play)]),
            ..PadConfig::default()
        };
        let mut active = std::collections::HashMap::new();
        assert!(!locked_pad_release(
            &pads,
            &[0x92, 36, 100],
            true,
            &mut active
        ));
        assert!(locked_pad_release(
            &pads,
            &[0x82, 36, 0],
            false,
            &mut active
        ));
        assert!(!locked_pad_release(
            &pads,
            &[0x82, 36, 0],
            false,
            &mut active
        ));
    }

    #[test]
    fn alsa_graph_parser_finds_clients_and_owned_destinations() {
        let lines = [
            "client 28: 'AudioBox USB 96' [type=kernel]".into(),
            "    0 'AudioBox MIDI'".into(),
            "    1 'AudioBox Control'".into(),
            "client 133: 'shs-casio' [type=user]".into(),
            "    0 'SHR-DAW accompaniment'".into(),
            "\tConnecting To: 28:0, 134:0, 28:0[real:0]".into(),
            "client 134: 'yoshimi-shs-yoshimi' [type=user]".into(),
        ];
        assert_eq!(
            parse_alsa_clients(&lines),
            [
                (28, "AudioBox USB 96".into()),
                (133, "shs-casio".into()),
                (134, "yoshimi-shs-yoshimi".into())
            ]
        );
        assert_eq!(parse_alsa_destinations(&lines, 133), [(28, 0), (134, 0)]);
        assert_eq!(
            parse_alsa_source_ports(&lines),
            [
                (28, 0, "AudioBox USB 96:AudioBox MIDI".into()),
                (28, 1, "AudioBox USB 96:AudioBox Control".into()),
                (133, 0, "shs-casio:SHR-DAW accompaniment".into())
            ]
        );
        let clients = parse_alsa_clients(&lines);
        assert_eq!(
            unique_client_match(&clients, "AudioBox").map(|client| client.0),
            Some(28)
        );
    }

    #[test]
    fn runtime_port_matching_prefers_exact_and_rejects_ambiguity() {
        let names = vec![
            "Controller MIDI 1".into(),
            "Controller MIDI 2".into(),
            "Exact".into(),
        ];
        assert_eq!(
            unique_name_match(&names, "Exact", "MIDI input").unwrap(),
            Some(2)
        );
        assert_eq!(
            unique_name_match(&names, "Controller", "MIDI input").unwrap(),
            None,
            "partial names must remain offline"
        );
        assert_eq!(
            unique_name_match(&names, "missing", "MIDI input").unwrap(),
            None
        );
        let ambiguous = vec!["Box:Port 20:0".into(), "Box-Port 21:0".into()];
        assert!(unique_name_match(&ambiguous, "Box Port", "MIDI input").is_err());

        let clients = vec![(1, "synth-one".into()), (2, "synth-two".into())];
        assert!(unique_client_match(&clients, "synth").is_none());
        assert_eq!(
            unique_client_match(&clients, "synth-one").map(|client| client.0),
            Some(1)
        );
    }

    fn input_role_config() -> RuntimeConfig {
        let mut config = RuntimeConfig::default();
        config.midi_input_matches.clear();
        config.midi_performance_input_matches.clear();
        config
    }

    #[test]
    fn legacy_controller_input_remains_a_combined_musical_source() {
        let mut config = input_role_config();
        config.midi_input_matches = vec!["Combined Device".into()];
        let plan = plan_midi_inputs(&["Combined Device".into()], &PadConfig::default(), &config);
        assert_eq!(plan.inputs.len(), 1);
        assert_eq!(
            plan.inputs[0].roles,
            InputRoles {
                controller: true,
                performance: true
            }
        );
        assert!(plan.availability.controller_available());
    }

    #[test]
    fn separate_controller_and_performance_ports_keep_separate_roles() {
        let mut config = input_role_config();
        config.midi_input_matches = vec!["Surface MIDI".into()];
        config.midi_performance_input_matches = vec!["Keyboard MIDI".into()];
        config.midi_controller_musical_input = false;
        let plan = plan_midi_inputs(
            &["Surface MIDI".into(), "Keyboard MIDI".into()],
            &PadConfig::default(),
            &config,
        );
        assert_eq!(plan.inputs.len(), 2);
        assert_eq!(
            plan.inputs[0].roles,
            InputRoles {
                controller: true,
                performance: false
            }
        );
        assert_eq!(
            plan.inputs[1].roles,
            InputRoles {
                controller: false,
                performance: true
            }
        );
    }

    #[test]
    fn same_exact_port_is_deduplicated_and_combines_roles() {
        let mut config = input_role_config();
        config.midi_input_matches = vec!["Shared MIDI".into()];
        config.midi_performance_input_matches = vec!["Shared MIDI".into()];
        config.midi_controller_musical_input = false;
        let plan = plan_midi_inputs(&["Shared MIDI".into()], &PadConfig::default(), &config);
        assert_eq!(plan.inputs.len(), 1);
        assert_eq!(
            plan.inputs[0].roles,
            InputRoles {
                controller: true,
                performance: true
            }
        );
        assert_eq!(plan.availability.performance_available(), 1);
    }

    #[test]
    fn missing_and_ambiguous_roles_do_not_hide_available_inputs() {
        let mut config = input_role_config();
        config.midi_input_matches = vec!["Missing Surface".into()];
        config.midi_performance_input_matches = vec![
            "Keyboard".into(),
            "Ambiguous A".into(),
            "Missing Keys".into(),
        ];
        let plan = plan_midi_inputs(
            &[
                "Keyboard".into(),
                "Ambiguous:A".into(),
                "Ambiguous-A".into(),
            ],
            &PadConfig::default(),
            &config,
        );
        assert_eq!(plan.inputs.len(), 1);
        assert_eq!(plan.inputs[0].name, "Keyboard");
        assert!(!plan.availability.controller_available());
        assert_eq!(plan.availability.performance_available(), 1);
        assert!(plan.availability.performance[1]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("ambiguous")));
        assert!(plan.availability.performance[2]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("not found")));
    }

    #[test]
    fn control_only_suppresses_unmapped_music_but_keeps_mapped_controls() {
        let roles = InputRoles {
            controller: true,
            performance: false,
        };
        assert!(!controller_allows_musical(roles, false, false));
        assert!(controller_allows_musical(roles, true, false));
        assert!(controller_allows_musical(roles, false, true));
    }

    #[test]
    fn learning_accepts_only_the_selected_controller_role() {
        assert!(controller_learning_owns_message(
            InputRoles {
                controller: true,
                performance: false
            },
            true
        ));
        assert!(!controller_learning_owns_message(
            InputRoles {
                controller: false,
                performance: true
            },
            true
        ));
        assert!(!controller_learning_owns_message(
            InputRoles {
                controller: true,
                performance: true
            },
            false
        ));
    }

    #[test]
    fn performance_path_bypasses_controller_command_note_mapping() {
        let mut state = LiveMidiState::default();
        let source = "performance keyboard".to_owned();
        let deliveries = route_live_message(&mut state, &source, &[0x90, 36, 100], None, None);
        assert_eq!(
            deliveries,
            [
                MidiDelivery::Raw(vec![0x90, 36, 100]),
                MidiDelivery::Direct(vec![0x90, 36, 100])
            ]
        );
    }

    #[test]
    fn performance_note_uses_selected_tracker_page_and_output_channel() {
        let external = RuntimeConfig::default().external_midi;
        let mut route = TrackerRoute::default();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::Midi("selected keyboard target".into()),
            columns: [(6, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            audition_note: None,
            scale: None,
            external: &external,
        });
        let mut state = LiveMidiState::default();
        let source = "performance keyboard".to_owned();
        let deliveries =
            route_live_message(&mut state, &source, &[0x91, 72, 101], Some(&route), None);
        assert_eq!(
            deliveries,
            [
                MidiDelivery::Tracker(
                    crate::sequencer::PageTarget::Midi("selected keyboard target".into()),
                    vec![0xc6, 0]
                ),
                MidiDelivery::Raw(vec![0x96, 72, 101]),
                MidiDelivery::Tracker(
                    crate::sequencer::PageTarget::Midi("selected keyboard target".into()),
                    vec![0x96, 72, 101]
                )
            ]
        );
    }

    #[test]
    fn identical_notes_from_two_sources_release_only_after_both_note_offs() {
        let mut state = LiveMidiState::default();
        let first = "first".to_owned();
        let second = "second".to_owned();
        route_live_message(&mut state, &first, &[0x92, 60, 90], None, None);
        route_live_message(&mut state, &second, &[0x92, 60, 110], None, None);
        assert!(route_live_message(&mut state, &first, &[0x82, 60, 0], None, None).is_empty());
        assert_eq!(
            route_live_message(&mut state, &second, &[0x92, 60, 0], None, None),
            [
                MidiDelivery::Raw(vec![0x92, 60, 0]),
                MidiDelivery::Direct(vec![0x92, 60, 0])
            ]
        );
    }

    #[test]
    fn source_all_notes_off_and_disconnect_preserve_other_source_notes() {
        let mut state = LiveMidiState::default();
        let first = "first".to_owned();
        let second = "second".to_owned();
        route_live_message(&mut state, &first, &[0x90, 64, 90], None, None);
        route_live_message(&mut state, &second, &[0x90, 64, 110], None, None);
        assert!(route_live_message(&mut state, &first, &[0xb0, 123, 0], None, None).is_empty());
        assert_eq!(
            release_source(&mut state, &second),
            [
                MidiDelivery::Raw(vec![0x80, 64, 0]),
                MidiDelivery::Direct(vec![0x80, 64, 0])
            ]
        );
    }

    #[test]
    fn performance_channel_messages_remain_byte_exact() {
        for message in [
            vec![0xb1, 1, 99],
            vec![0xa1, 60, 44],
            vec![0xd1, 55],
            vec![0xe1, 0, 96],
            vec![0xc1, 12],
        ] {
            let mut state = LiveMidiState::default();
            let deliveries =
                route_live_message(&mut state, &"keyboard".into(), &message, None, None);
            assert_eq!(
                deliveries,
                [
                    MidiDelivery::Raw(message.clone()),
                    MidiDelivery::Direct(message)
                ]
            );
        }
    }

    #[test]
    fn route_change_and_shutdown_release_every_owned_destination() {
        let config = RuntimeConfig::default().external_midi;
        let mut route = TrackerRoute::default();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::Midi("first output".into()),
            columns: [(2, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            audition_note: None,
            scale: None,
            external: &config,
        });
        let mut state = LiveMidiState::default();
        let source = "keyboard".to_owned();
        route_live_message(&mut state, &source, &[0x90, 60, 100], Some(&route), None);
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::Midi("second output".into()),
            columns: [(5, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            audition_note: None,
            scale: None,
            external: &config,
        });
        let changed = route_live_message(&mut state, &source, &[0xb0, 1, 64], Some(&route), None);
        assert!(changed.contains(&MidiDelivery::Tracker(
            crate::sequencer::PageTarget::Midi("first output".into()),
            vec![0x82, 60, 0]
        )));
        route_live_message(&mut state, &source, &[0x95, 67, 100], Some(&route), None);
        let shutdown = release_all_inputs(&mut state);
        assert!(shutdown.contains(&MidiDelivery::Tracker(
            crate::sequencer::PageTarget::Midi("second output".into()),
            vec![0x85, 67, 0]
        )));
        assert!(release_all_inputs(&mut state).is_empty());
    }

    #[test]
    fn tracker_navigation_keeps_held_notes_on_their_original_route() {
        let config = RuntimeConfig::default().external_midi;
        let mut route = TrackerRoute::default();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::Midi("bass output".into()),
            columns: [(2, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            audition_note: None,
            scale: None,
            external: &config,
        });
        let mut state = LiveMidiState::default();
        let source = "keyboard".to_owned();
        route_live_message(&mut state, &source, &[0x90, 60, 100], Some(&route), None);

        route.configure_navigation(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::Midi("lead output".into()),
            columns: [(5, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            audition_note: None,
            scale: None,
            external: &config,
        });
        let new_note =
            route_live_message(&mut state, &source, &[0x90, 64, 110], Some(&route), None);
        assert!(!new_note.contains(&MidiDelivery::Tracker(
            crate::sequencer::PageTarget::Midi("bass output".into()),
            vec![0x82, 60, 0]
        )));
        assert!(new_note.contains(&MidiDelivery::Tracker(
            crate::sequencer::PageTarget::Midi("lead output".into()),
            vec![0x95, 64, 110]
        )));

        let old_note_off =
            route_live_message(&mut state, &source, &[0x80, 60, 0], Some(&route), None);
        assert!(old_note_off.contains(&MidiDelivery::Tracker(
            crate::sequencer::PageTarget::Midi("bass output".into()),
            vec![0x82, 60, 0]
        )));
    }

    #[test]
    fn all_notes_off_lifecycle_reset_clears_stale_source_ownership() {
        let shared = Arc::new(Mutex::new(LiveMidiState::default()));
        let lifecycle = MidiLifecycle::new(Arc::clone(&shared));
        let source = "keyboard".to_owned();
        {
            let mut state = shared.lock().unwrap();
            route_live_message(&mut state, &source, &[0x90, 60, 100], None, None);
        }
        lifecycle.clear_after_all_notes_off();
        let replayed = route_live_message(
            &mut shared.lock().unwrap(),
            &source,
            &[0x90, 60, 110],
            None,
            None,
        );
        assert_eq!(
            replayed,
            [
                MidiDelivery::Raw(vec![0x90, 60, 110]),
                MidiDelivery::Direct(vec![0x90, 60, 110])
            ]
        );
    }

    #[test]
    fn managed_audio_graph_resolves_only_one_exact_stereo_client() {
        let ports = vec![
            "shs-synth:audio_out_1".into(),
            "unrelated:audio_out_1".into(),
            "shs-synth:audio_out_2".into(),
            "shs-synth-midi:audio_out_1".into(),
        ];
        assert_eq!(
            resolve_managed_audio_outputs("shs-synth", ports).unwrap(),
            ["shs-synth:audio_out_1", "shs-synth:audio_out_2"]
        );
        assert!(resolve_managed_audio_outputs(
            "shs-synth",
            vec![
                "shs-synth:audio_out_1".into(),
                "shs-synth:audio_out_2".into(),
                "shs-synth:audio_out_3".into(),
            ]
        )
        .is_err());
    }

    #[test]
    fn managed_audio_outputs_accept_one_unambiguous_backend_prefix() {
        let ports = vec![
            "yoshimi-shs-yoshimi:audio/out_1".into(),
            "yoshimi-shs-yoshimi:audio/out_2".into(),
            "unrelated:audio/out_1".into(),
        ];
        assert_eq!(
            resolve_managed_audio_outputs("shs-yoshimi", ports).unwrap(),
            [
                "yoshimi-shs-yoshimi:audio/out_1",
                "yoshimi-shs-yoshimi:audio/out_2"
            ]
        );

        let ambiguous = vec![
            "first-shs-yoshimi:audio/out_1".into(),
            "first-shs-yoshimi:audio/out_2".into(),
            "second-shs-yoshimi:audio/out_1".into(),
            "second-shs-yoshimi:audio/out_2".into(),
        ];
        assert!(resolve_managed_audio_outputs("shs-yoshimi", ambiguous).is_err());
    }

    #[test]
    fn final_bus_sources_stay_exact_across_absence_and_reconnection() {
        let mut config = RuntimeConfig::default();
        config.audio_graph.input = Some(crate::config::StereoInputConfig {
            name: "External mix".into(),
            left_port: "interface:mix_l".into(),
            right_port: "interface:mix_r".into(),
        });
        let loop_ports = crate::loop_player::configured_output_ports(&config.loop_player);
        let nearby = vec![
            loop_ports[0].clone(),
            loop_ports[1].clone(),
            "interface:mix_l".into(),
            "interface:nearby_r".into(),
        ];
        let error = resolve_performance_bus_sources(&config, &nearby)
            .unwrap_err()
            .to_string();
        assert!(error.contains("interface:mix_r"));
        let mut returned = nearby;
        returned.push("interface:mix_r".into());
        let (resolved_loop, resolved_input) =
            resolve_performance_bus_sources(&config, &returned).unwrap();
        assert_eq!(resolved_loop, loop_ports);
        assert_eq!(resolved_input, ["interface:mix_l", "interface:mix_r"]);
        assert_eq!(
            config.audio_graph.input.unwrap().right_port,
            "interface:mix_r"
        );
    }

    #[test]
    fn legacy_or_mismatched_owner_records_are_never_signalled() {
        let dir = std::env::temp_dir().join(format!("shsynth-owner-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("engine.pid"), format!("{}\n", std::process::id())).unwrap();
        stop_managed(&dir).unwrap();
        assert!(proc_start_time(std::process::id() as i32).is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deleted_executable_suffix_does_not_orphan_a_verified_owner() {
        assert!(same_executable(
            Path::new("/usr/bin/shr"),
            Path::new("/usr/bin/shr (deleted)")
        ));
        assert!(!same_executable(
            Path::new("/usr/bin/shr"),
            Path::new("/usr/bin/other (deleted)")
        ));
    }

    #[test]
    fn verified_owned_process_is_stopped_and_marker_is_cleaned() {
        let dir = std::env::temp_dir().join(format!("shsynth-owned-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut child = Command::new("sleep").arg("300").spawn().unwrap();
        write_owner(&dir.join("engine.pid"), child.id() as i32).unwrap();
        let recorded = read_owner(&dir.join("engine.pid")).unwrap();
        if !still_owned(&recorded) {
            let current = owner_for(child.id() as i32);
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("recorded owner {recorded:?} did not match spawned process {current:?}");
        }
        stop_managed(&dir).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        if child.try_wait().unwrap().is_none() {
            let current = owner_for(child.id() as i32);
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("recorded owner {recorded:?} failed to stop spawned process {current:?}");
        }
        assert!(!dir.join("engine.pid").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fluid_bank_selection_encodes_14_bit_mma_banks() {
        let bank = 258u16;
        assert_eq!([(bank >> 7) as u8, (bank & 0x7f) as u8], [2, 2]);

        let path = PathBuf::from("configured.sf2");
        let preset = Preset {
            backend: BackendKind::FluidSynth,
            name: "Program".into(),
            category: None,
            id: PresetId::FluidSynth {
                soundfont: path.clone(),
                soundfont_index: 0,
                bank: 2,
                program: 9,
            },
        };
        assert!(fluidsynth_selection(&preset, &[]).is_err());
        assert_eq!(
            fluidsynth_selection(&preset, &[(path, 256)]).unwrap(),
            (258, 9)
        );
    }

    #[test]
    fn managed_fluidsynth_uses_jack_without_a_tcp_server() {
        let preset = Preset {
            backend: BackendKind::FluidSynth,
            name: "Program".into(),
            category: None,
            id: PresetId::FluidSynth {
                soundfont: PathBuf::from("configured.sf2"),
                soundfont_index: 0,
                bank: 0,
                program: 0,
            },
        };
        let config = RuntimeConfig::default();
        let command = backend_command(&preset, Path::new("/tmp/shr-state"), &config).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg == "--audio-driver=jack"));
        assert!(args.iter().any(|arg| arg == "--midi-driver=alsa_seq"));
        assert!(!args.iter().any(|arg| arg == "--server"));
        let gain = args.iter().position(|arg| arg == "--gain").unwrap();
        assert_eq!(args.get(gain + 1).map(String::as_str), Some("0.4"));
        let load = args.iter().position(|arg| arg == "--load-config").unwrap();
        assert!(gain < load);
        assert_eq!(
            args.get(load + 1).map(String::as_str),
            Some("/tmp/shr-state/fluidsynth.conf")
        );
    }

    #[test]
    fn managed_backend_midi_selectors_accept_one_unique_generated_identity() {
        let synthv1 = vec!["shs-synthv1:in 133:0".into()];
        assert_eq!(
            unique_backend_name_match(&synthv1, "synthv1", "MIDI output").unwrap(),
            Some(0)
        );
        assert_eq!(
            unique_name_match(&synthv1, "synthv1", "MIDI output").unwrap(),
            None,
            "physical-device matching must remain exact"
        );

        let ambiguous = vec![
            "first-synthv1:in 133:0".into(),
            "second-synthv1:in 134:0".into(),
        ];
        assert!(unique_backend_name_match(&ambiguous, "synthv1", "MIDI output").is_err());
    }

    #[test]
    fn jack_connection_listing_preserves_exact_existing_routes() {
        let lines = [
            "yoshimi-shs-yoshimi:left",
            "   system:playback_1",
            "yoshimi-shs-yoshimi:right",
            "   system:playback_2",
            "system:playback_1",
            "   yoshimi-shs-yoshimi:left",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(
            parse_jack_connections(lines),
            vec![
                (
                    "yoshimi-shs-yoshimi:left".into(),
                    "system:playback_1".into()
                ),
                (
                    "yoshimi-shs-yoshimi:right".into(),
                    "system:playback_2".into()
                ),
                (
                    "system:playback_1".into(),
                    "yoshimi-shs-yoshimi:left".into()
                ),
            ]
        );
    }

    #[test]
    fn engine_preflight_rejects_an_unavailable_command_without_state_markers() {
        let base = std::env::temp_dir().join(format!("shsynth-preflight-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let preset_path = base.join("preset.synthv1");
        fs::write(&preset_path, "<preset/>").unwrap();
        let preset = Preset::synthv1("test", preset_path);
        let mut config = RuntimeConfig::default();
        config.synth_command = base.join("missing-synth").to_string_lossy().into_owned();

        assert!(validate_start(&preset, &base, &config).is_err());
        assert!(!base.join("engine.pid").exists());
        assert!(!base.join("current").exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn jack_source_discovery_keeps_only_exact_audio_outputs() {
        let lines = [
            "system:capture_2",
            "    properties: output,physical,terminal,",
            "    32 bit float mono audio",
            "midi:out",
            "    properties: output,",
            "    8 bit raw midi",
            "system:playback_1",
            "    properties: input,physical,terminal,",
            "    32 bit float mono audio",
            "source:one",
            "    properties: output,",
            "    32 bit float mono audio",
            "system:capture_1",
            "    properties: output,physical,terminal,",
            "    32 bit float mono audio",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(
            parse_jack_audio_sources(lines),
            ["source:one", "system:capture_1", "system:capture_2"]
        );
    }
}
