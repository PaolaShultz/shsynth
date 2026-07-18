use crate::audio_graph::InsertRack;
use crate::audio_graph_client::{EffectMeterSnapshot, OwnedAudioGraph};
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
use std::sync::{mpsc::Sender, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum MidiEvent {
    Value(u8, f32),
    Raw { received: Instant, bytes: Vec<u8> },
    Pad(PadAction, bool),
    Encoder(EncoderAction),
    PadLock(bool),
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
    audio_graph_fallback: Option<String>,
}

pub type SharedOutput = Arc<Mutex<Option<MidiOutputConnection>>>;
pub type SharedPickup = Arc<Mutex<crate::midi::Pickup>>;
pub type SharedBackend = Arc<Mutex<BackendKind>>;
pub type SharedTrackerRoute = Arc<Mutex<TrackerRoute>>;
pub type SharedTrackerInput = Arc<Mutex<Option<crate::sequencer::LiveInput>>>;

pub struct TrackerRouteConfig<'a> {
    pub enabled: bool,
    pub target: crate::sequencer::PageTarget,
    pub columns: [(u8, (u8, u8, u8)); crate::sequencer::LANES_PER_PAGE],
    pub start_column: usize,
    pub percussion: bool,
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
    bank_select: crate::config::BankSelectMode,
    scale: Option<crate::scale::Scale>,
    revision: u64,
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
}

impl Default for TrackerRoute {
    fn default() -> Self {
        Self {
            enabled: false,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [TrackerColumnRoute::default(); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            note_map: std::array::from_fn(|note| note as u8),
            bank_select: crate::config::BankSelectMode::Off,
            scale: None,
            revision: 0,
        }
    }
}

impl TrackerRoute {
    pub fn configure(&mut self, config: TrackerRouteConfig<'_>) {
        self.revision = self.revision.wrapping_add(1);
        self.enabled = config.enabled;
        self.target = config.target;
        self.columns = config
            .columns
            .map(|(channel, selection)| TrackerColumnRoute {
                channel,
                program: config.external.program_changes.then_some(selection.0),
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
            for (offset, &note) in config.external.percussion_notes.iter().enumerate() {
                self.note_map[usize::from(config.external.percussion_input_base) + offset] = note;
            }
        }
        if let Some(scale) = self.scale.filter(|_| !config.percussion) {
            for note in &mut self.note_map {
                *note = scale.map(*note);
            }
        }
    }

    fn mapped_note(&self, note: u8) -> Option<u8> {
        self.note_map.get(usize::from(note)).copied()
    }

    fn column(&self, index: usize) -> TrackerColumnRoute {
        self.columns[index % crate::sequencer::LANES_PER_PAGE]
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

fn tracker_edit_consumes_note(route: Option<&TrackerRoute>, message: &[u8]) -> bool {
    route.is_some_and(|route| route.enabled && valid_note_message(message))
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
    _input: MidiInputConnection<()>,
    output: SharedOutput,
    pickup: SharedPickup,
    backend: SharedBackend,
    tracker_route: SharedTrackerRoute,
    tracker_input: SharedTrackerInput,
}

impl MidiRouter {
    pub fn start(state: &Path, config: &RuntimeConfig, tx: Sender<MidiEvent>) -> Result<Self> {
        if !config.midi_autoconnect {
            bail!("MIDI routing is disabled in shsynth.conf");
        }
        let pads = PadConfig::load(&state.join("controller.conf"))?;
        let output = Arc::new(Mutex::new(None));
        let pickup = Arc::new(Mutex::new(crate::midi::Pickup::default()));
        let backend = Arc::new(Mutex::new(BackendKind::Synthv1));
        let tracker_route = Arc::new(Mutex::new(TrackerRoute::default()));
        let tracker_input = Arc::new(Mutex::new(None));
        let mut last_error = None;
        let mut input = None;
        for _ in 0..25 {
            match connect_midi_input(
                tx.clone(),
                pads.clone(),
                config,
                CallbackRouting {
                    output: Arc::clone(&output),
                    pickup: Arc::clone(&pickup),
                    backend: Arc::clone(&backend),
                    tracker_route: Arc::clone(&tracker_route),
                    tracker_input: Arc::clone(&tracker_input),
                },
            ) {
                Ok(connection) => {
                    input = Some(connection);
                    break;
                }
                Err(error) => {
                    last_error = Some(error);
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }
        let input =
            input.ok_or_else(|| last_error.unwrap_or_else(|| anyhow!("MIDI input unavailable")))?;
        Ok(Self {
            _input: input,
            output,
            pickup,
            backend,
            tracker_route,
            tracker_input,
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

    pub fn arm_pickup(&self, values: &std::collections::HashMap<u8, f32>) {
        if let Ok(mut pickup) = self.pickup.lock() {
            pickup.arm(values);
        }
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
        let prepare = (|| -> Result<()> {
            write_owner(&state.join("engine.pid"), child.id() as i32)?;
            wait_ready(
                &mut child,
                preset.backend,
                &backend_config.client_name,
                config.startup_timeout,
                &log_path,
            )?;
            connect_audio(&backend_config.client_name, config);
            if config.audio_graph.enabled {
                match start_managed_audio_graph(&backend_config.client_name, config, rack) {
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
            let input_matches = controller
                .input_match
                .iter()
                .chain(config.midi_input_matches.iter());
            for source in input_matches {
                disconnect_direct_midi(source, &backend_config.client_name);
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
            audio_graph_fallback,
        };
        if preset.backend == BackendKind::FluidSynth {
            engine.select_fluidsynth(preset)?;
        }
        Ok(engine)
    }

    pub fn backend(&self) -> BackendKind {
        self.backend
    }

    pub fn alive(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }

    pub fn audio_route_status(&self) -> Option<String> {
        if self.audio_graph.is_some() {
            Some("owned insert graph active".into())
        } else {
            self.audio_graph_fallback
                .as_ref()
                .map(|error| format!("direct audio fallback · {error}"))
        }
    }

    pub fn publish_insert_rack(&mut self, rack: &InsertRack) -> Result<bool> {
        let Some(graph) = self.audio_graph.as_mut() else {
            return Ok(false);
        };
        graph.publish_rack(rack)?;
        Ok(true)
    }

    pub(crate) fn effect_meter(&self, effect_id: u32) -> Option<EffectMeterSnapshot> {
        self.audio_graph.as_ref()?.effect_meter(effect_id)
    }

    pub(crate) fn process_id(&self) -> u32 {
        self.child.id()
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
        let (timing, restored) = self.stop_audio_graph()?;
        let status = match restored {
            Ok(()) => format!(
                "AUDIO GRAPH LOST · direct route restored · {}",
                audio_graph_metrics(&timing)
            ),
            Err(error) => format!(
                "AUDIO GRAPH LOST · direct restore unavailable: {error:#} · {}",
                audio_graph_metrics(&timing)
            ),
        };
        self.audio_graph_fallback = Some(status.clone());
        Some(status)
    }

    fn stop_audio_graph(&mut self) -> Option<(CallbackTimingSnapshot, Result<()>)> {
        let mut graph = self.audio_graph.take()?;
        let restored = graph.restore_direct();
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
                "--server",
                "--portname",
                &backend.client_name,
                "-o",
                &format!("audio.jack.id={}", backend.client_name),
                "-o",
                "synth.midi-bank-select=mma",
                "--load-config",
            ]);
            command
                .arg("--gain")
                .arg(config.fluidsynth.gain.to_string());
            command.arg(state.join("fluidsynth.conf"));
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
        if jack_ports().iter().any(|port| {
            port.to_ascii_lowercase()
                .contains(&client_name.to_ascii_lowercase())
        }) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    terminate(child);
    bail!(
        "{} did not register with JACK; see {}",
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

fn connect_midi_input(
    tx: Sender<MidiEvent>,
    pads: PadConfig,
    config: &RuntimeConfig,
    routing: CallbackRouting,
) -> Result<MidiInputConnection<()>> {
    let CallbackRouting {
        output,
        pickup,
        backend,
        tracker_route,
        tracker_input,
    } = routing;
    let mut input = MidiInput::new("SHR-DAW MIDI input")?;
    input.ignore(Ignore::None);
    let ports = input.ports();
    let names = ports
        .iter()
        .map(|port| input.port_name(port).map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;
    let port_index = if let Some(wanted) = pads.input_match.as_ref() {
        unique_name_match(&names, wanted, "MIDI input")?
    } else {
        let mut selected = None;
        for wanted in &config.midi_input_matches {
            if let Some(index) = unique_name_match(&names, wanted, "MIDI input")? {
                selected = Some(index);
                break;
            }
        }
        selected
    }
    .ok_or_else(|| {
        let wanted = pads
            .input_match
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| config.midi_input_matches.join(", "));
        anyhow!("MIDI input not found (wanted: {wanted})")
    })?;
    let port = &ports[port_index];
    let input_name = names[port_index].clone();
    let output2 = Arc::clone(&output);
    let mut pad_locked = false;
    let mut lock_pressed = false;
    let mut preview_notes = crate::scale::NoteLifecycle::default();
    let mut preview_next_column = 0usize;
    let mut preview_programs = std::collections::BTreeMap::new();
    let mut locked_pad_notes = std::collections::HashMap::new();
    let mut route_revision = 0;
    let connection = input
        .connect(
            port,
            "SHR-DAW monitor",
            move |_stamp, message, _| {
                let received = Instant::now();
                if invalid_note_message(message) {
                    let _ = tx.send(MidiEvent::Error(
                        "ignored malformed MIDI note message".into(),
                    ));
                    return;
                }
                let backend = backend
                    .lock()
                    .map(|kind| *kind)
                    .unwrap_or(BackendKind::Synthv1);
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
                let routed = crate::midi::route_with_pad_lock(&pads, backend, message, pad_locked);
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
                if accepted {
                    if let Some((cc, value)) = routed.value {
                        let _ = tx.send(MidiEvent::Value(cc, value));
                    }
                    let translated = routed.translated;
                    if let Some(message) = translated
                        .as_ref()
                        .map(|bytes| &bytes[..])
                        .or(routed.forward)
                        .or_else(|| forced_pad_release.then_some(message))
                    {
                        let status = message.first().copied().unwrap_or(0);
                        let note_message = valid_note_message(message);
                        let route = tracker_route.lock().ok().map(|route| route.clone());
                        let tracker_consumes_note =
                            tracker_edit_consumes_note(route.as_ref(), message);
                        if route
                            .as_ref()
                            .is_some_and(|route| route.revision != route_revision)
                        {
                            if let Ok(input) = tracker_input.lock() {
                                if let Some(input) = input.as_ref() {
                                    for (target, channel, note) in preview_notes.drain() {
                                        input.send(&target, &[0x80 | channel, note, 0]);
                                    }
                                }
                            }
                            route_revision = route.as_ref().map_or(0, |route| route.revision);
                            preview_next_column =
                                route.as_ref().map_or(0, |route| route.start_column);
                            preview_programs.clear();
                        }
                        let mut preview = None;
                        let mut program = None;
                        if note_message {
                            let source_note = message[1];
                            let source_channel = status & 0x0f;
                            let note_off = status & 0xf0 == 0x80 || message[2] == 0;
                            if note_off {
                                preview = preview_notes.note_off(source_channel, source_note);
                            } else if let Some(route) = route.filter(|route| route.enabled) {
                                let Some(mapped_note) = route.mapped_note(source_note) else {
                                    return;
                                };
                                let column = route.column(preview_next_column);
                                preview_next_column =
                                    (preview_next_column + 1) % crate::sequencer::LANES_PER_PAGE;
                                let destination =
                                    (route.target.clone(), column.channel, mapped_note);
                                preview_notes.note_on(
                                    source_channel,
                                    source_note,
                                    destination.clone(),
                                );
                                preview = Some(destination);
                                program = column.program.map(|program| {
                                    (program, route.bank_select, column.bank_msb, column.bank_lsb)
                                });
                            }
                        }
                        if let Some((target, channel, note)) = preview {
                            let preview_message = [status & 0xf0 | channel, note, message[2]];
                            let _ = tx.send(MidiEvent::Raw {
                                received,
                                bytes: preview_message.to_vec(),
                            });
                            if let Ok(input) = tracker_input.lock() {
                                if let Some(input) = input.as_ref() {
                                    if let Some((program, bank_select, bank_msb, bank_lsb)) =
                                        program.filter(|(program, _, _, _)| {
                                            preview_programs.get(&(target.clone(), channel))
                                                != Some(program)
                                        })
                                    {
                                        match bank_select {
                                            crate::config::BankSelectMode::Off => {}
                                            crate::config::BankSelectMode::Cc0 => {
                                                input.send(&target, &[0xb0 | channel, 0, bank_msb]);
                                            }
                                            crate::config::BankSelectMode::Cc0Cc32 => {
                                                input.send(&target, &[0xb0 | channel, 0, bank_msb]);
                                                input
                                                    .send(&target, &[0xb0 | channel, 32, bank_lsb]);
                                            }
                                        }
                                        input.send(&target, &[0xc0 | channel, program]);
                                        preview_programs.insert((target.clone(), channel), program);
                                    }
                                    input.send(&target, &preview_message);
                                }
                            }
                        } else if !tracker_consumes_note {
                            let _ = tx.send(MidiEvent::Raw {
                                received,
                                bytes: message.to_vec(),
                            });
                            if let Ok(mut output) = output2.lock() {
                                if let Some(output) = output.as_mut() {
                                    if let Err(error) = output.send(message) {
                                        let _ = tx.send(MidiEvent::Error(format!(
                                            "MIDI forward: {error}"
                                        )));
                                    }
                                }
                            }
                        }
                    }
                }
            },
            (),
        )
        .map_err(|error| anyhow!("connect controller MIDI input: {error}"))?;
    let source = input_name.split(':').next().unwrap_or(&input_name);
    for client in [
        &config.client_name,
        &config.yoshimi.backend.client_name,
        &config.fluidsynth.backend.client_name,
    ] {
        disconnect_direct_midi(source, client);
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
    let clients = parse_alsa_clients(&command_lines("aconnect", &["-l"]));
    let Some((source_id, _)) = unique_client_match(&clients, source_match) else {
        return;
    };
    let destination_ids = destination_matches
        .iter()
        .filter_map(|wanted| unique_client_match(&clients, wanted).map(|(id, _)| *id))
        .collect::<std::collections::HashSet<_>>();
    for destination_id in destination_ids {
        let _ = Command::new("aconnect")
            .args([
                "-d",
                &format!("{source_id}:0"),
                &format!("{destination_id}:0"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
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
    let index = unique_name_match(&names, output_match, "MIDI output")?.ok_or_else(|| {
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
    let wanted_lower = wanted.to_ascii_lowercase();
    if wanted_lower.is_empty() {
        bail!("{description} match cannot be empty");
    }
    let exact = names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.to_ascii_lowercase() == wanted_lower)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let matches = if exact.is_empty() {
        names
            .iter()
            .enumerate()
            .filter(|(_, name)| name.to_ascii_lowercase().contains(&wanted_lower))
            .map(|(index, _)| index)
            .collect::<Vec<_>>()
    } else {
        exact
    };
    match matches.as_slice() {
        [] => Ok(None),
        [index] => Ok(Some(*index)),
        _ => bail!(
            "{description} match {wanted:?} is ambiguous: {}",
            matches
                .iter()
                .map(|index| names[*index].as_str())
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

fn jack_ports() -> Vec<String> {
    command_lines("jack_lsp", &[])
}

fn connect_audio(client_name: &str, config: &RuntimeConfig) {
    if !config.audio_autoconnect {
        return;
    }
    let Ok(outputs) = managed_audio_outputs(client_name) else {
        return;
    };
    for (source, destination) in outputs.iter().zip(config.audio_outputs.iter()) {
        let _ = Command::new("jack_connect")
            .args([source.as_str(), destination.as_str()])
            .status();
    }
}

fn start_managed_audio_graph(
    client_name: &str,
    config: &RuntimeConfig,
    rack: &InsertRack,
) -> Result<OwnedAudioGraph> {
    let source_ports = managed_audio_outputs(client_name)?;
    let destinations: [String; 2] = config
        .audio_outputs
        .clone()
        .try_into()
        .map_err(|_| anyhow!("owned graph requires exactly two configured main outputs"))?;
    OwnedAudioGraph::start_with_rack(&config.audio_graph, source_ports, destinations, rack)
}

fn managed_audio_outputs(client_name: &str) -> Result<[String; 2]> {
    resolve_managed_audio_outputs(client_name, jack_ports())
}

fn resolve_managed_audio_outputs(
    client_name: &str,
    ports: impl IntoIterator<Item = String>,
) -> Result<[String; 2]> {
    let mut outputs = Vec::new();
    for port in ports {
        let Some((client, short_name)) = port.split_once(':') else {
            continue;
        };
        let short_name = short_name.to_ascii_lowercase();
        let is_output = short_name.contains("out")
            || short_name.contains("audio")
            || short_name == "left"
            || short_name == "right";
        if client.eq_ignore_ascii_case(client_name) && is_output && !outputs.contains(&port) {
            outputs.push(port);
        }
    }
    if outputs.len() != 2 {
        bail!(
            "managed JACK client {client_name:?} has {} unambiguous audio outputs, expected 2",
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
        assert!(tracker_edit_consumes_note(Some(&route), &[0x90, 60, 100]));
        assert!(tracker_edit_consumes_note(Some(&route), &[0x90, 60, 0]));
        assert!(tracker_edit_consumes_note(Some(&route), &[0x80, 60, 0]));
        assert!(!tracker_edit_consumes_note(Some(&route), &[0xb0, 1, 64]));
        assert!(!tracker_edit_consumes_note(Some(&route), &[0x90, 128, 100]));
        assert!(invalid_note_message(&[0x90, 128, 100]));
        assert!(invalid_note_message(&[0x80, 60]));
        assert!(invalid_note_message(&[0x90, 60, 100, 0]));
        route.configure(TrackerRouteConfig {
            enabled: false,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [(2, (9, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: true,
            scale: None,
            external: &config,
        });
        assert!(!tracker_edit_consumes_note(Some(&route), &[0x90, 60, 100]));
    }

    #[test]
    fn noob_route_applies_scale_only_to_melodic_pages() {
        let config = RuntimeConfig::default().external_midi;
        let scale = crate::scale::Scale {
            root: 3,
            kind: crate::scale::ScaleKind::NaturalMinor,
        };
        let mut route = TrackerRoute::default();
        route.configure(TrackerRouteConfig {
            enabled: true,
            target: crate::sequencer::PageTarget::ConfiguredExternal,
            columns: [(4, (0, 0, 0)); crate::sequencer::LANES_PER_PAGE],
            start_column: 0,
            percussion: false,
            scale: Some(scale),
            external: &config,
        });
        assert_eq!(route.mapped_note(64), Some(scale.map(64)));
        assert_eq!(route.mapped_note(127), Some(126));
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
        assert!(unique_name_match(&names, "Controller", "MIDI input").is_err());
        assert_eq!(
            unique_name_match(&names, "missing", "MIDI input").unwrap(),
            None
        );

        let clients = vec![(1, "synth-one".into()), (2, "synth-two".into())];
        assert!(unique_client_match(&clients, "synth").is_none());
        assert_eq!(
            unique_client_match(&clients, "synth-one").map(|client| client.0),
            Some(1)
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
}
