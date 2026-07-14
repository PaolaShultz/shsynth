use crate::config::{BackendConfig, RuntimeConfig};
use crate::control::{self, CONTROLS};
use crate::pads::{EncoderAction, PadAction, PadConfig};
use crate::preset::{self, BackendKind, Preset, PresetId};
use anyhow::{anyhow, bail, Context, Result};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc::Sender, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum MidiEvent {
    Value(u8, f32),
    Raw(Vec<u8>),
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
}

pub type SharedOutput = Arc<Mutex<Option<MidiOutputConnection>>>;
pub type SharedPickup = Arc<Mutex<crate::midi::Pickup>>;
pub type SharedBackend = Arc<Mutex<BackendKind>>;
pub type SharedTrackerRoute = Arc<Mutex<TrackerRoute>>;
pub type SharedTrackerInput = Arc<Mutex<Option<crate::sequencer::LiveInput>>>;

#[derive(Clone)]
pub struct TrackerRoute {
    enabled: bool,
    target: crate::sequencer::PageTarget,
    channel: u8,
    note_map: [u8; 128],
    program: Option<u8>,
    revision: u64,
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
            channel: 0,
            note_map: std::array::from_fn(|note| note as u8),
            program: None,
            revision: 0,
        }
    }
}

impl TrackerRoute {
    pub fn configure(
        &mut self,
        enabled: bool,
        target: crate::sequencer::PageTarget,
        channel: u8,
        percussion: bool,
        program: u8,
        config: &crate::config::ExternalMidiConfig,
    ) {
        self.revision = self.revision.wrapping_add(1);
        self.enabled = enabled;
        self.target = target;
        self.channel = channel;
        self.note_map = std::array::from_fn(|note| note as u8);
        self.program = config.program_changes.then_some(program);
        if percussion {
            for (offset, &note) in config.percussion_notes.iter().enumerate() {
                self.note_map[usize::from(config.percussion_input_base) + offset] = note;
            }
        }
    }

    fn mapped_note(&self, note: u8) -> u8 {
        self.note_map[usize::from(note)]
    }
}

fn tracker_edit_consumes_note(route: Option<&TrackerRoute>, message: &[u8]) -> bool {
    let note_message = message.len() >= 3 && matches!(message[0] & 0xf0, 0x80 | 0x90);
    route.is_some_and(|route| route.enabled && note_message)
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
        let pads = PadConfig::load(&state.join("controller.conf")).unwrap_or_default();
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
        stop_managed(state)?;
        fs::create_dir_all(state)?;
        let controller = PadConfig::load(&state.join("controller.conf")).unwrap_or_default();
        let fluid_soundfonts = if preset.backend == BackendKind::FluidSynth {
            preset::soundfont_offsets(&config.fluidsynth.soundfonts)?
        } else {
            Vec::new()
        };
        if preset.backend == BackendKind::Synthv1 {
            write_synthv1_config(&state.join("config"), &controller)?;
        }
        if preset.backend == BackendKind::FluidSynth {
            write_fluidsynth_config(state, &fluid_soundfonts)?;
        }

        let backend_config = backend_config(config, preset.backend);
        let log_path = state.join("engine.log");
        let log = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&log_path)?;
        let log_err = log.try_clone()?;
        let mut command = backend_command(preset, state, config)?;
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
        let prepare = (|| -> Result<()> {
            write_owner(&state.join("engine.pid"), child.id() as i32)?;
            fs::write(
                state.join("current"),
                format!("{}\t{}\n", preset.backend.label(), preset.name),
            )?;
            wait_ready(
                &mut child,
                preset.backend,
                &backend_config.client_name,
                config.startup_timeout,
                &log_path,
            )?;
            connect_audio(&backend_config.client_name, config);
            if config.midi_autoconnect {
                attach_midi_output(&output, &backend_config.midi_output_match, preset.backend)?;
                retain_midi_destination("SHSynth MIDI output", &backend_config.client_name);
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
            Ok(())
        })();
        if let Err(error) = prepare {
            if let Ok(mut connection) = output.lock() {
                *connection = None;
            }
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
        if self.backend == BackendKind::FluidSynth {
            // CC120 complements All Notes Off by silencing any sustained voice.
            for channel in 0..16 {
                let _ = self.send(&[0xb0 | channel, 120, 0]);
            }
        }
    }

    /// Loads a sound without replacing the process when that backend supports
    /// it. Returns false when the caller must perform an exclusive restart.
    pub fn load_in_place(&mut self, preset: &Preset) -> Result<bool> {
        if preset.backend != self.backend || !self.alive() {
            return Ok(false);
        }
        self.panic();
        match (&self.backend, &preset.id) {
            (BackendKind::Yoshimi, PresetId::Yoshimi { path }) => {
                let path = safe_command_path(path)?;
                let stdin = self
                    .stdin
                    .as_mut()
                    .context("Yoshimi command input unavailable")?;
                writeln!(stdin, "load instrument {path}")?;
                stdin.flush()?;
            }
            (BackendKind::FluidSynth, PresetId::FluidSynth { .. }) => {
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
                "{} has no SHSynth mapped-parameter reset",
                self.backend.label()
            );
        }
        for message in mapped_parameter_messages(&self.control_routes, values) {
            self.send(&message)?;
        }
        Ok(())
    }

    fn select_fluidsynth(&self, preset: &Preset) -> Result<()> {
        let PresetId::FluidSynth {
            soundfont,
            bank,
            program,
            ..
        } = &preset.id
        else {
            bail!("not a FluidSynth preset")
        };
        let offset = self
            .fluid_soundfonts
            .iter()
            .find_map(|(candidate, offset)| (candidate == soundfont).then_some(*offset))
            .context("preset SoundFont is not configured for this FluidSynth process")?;
        let effective_bank = offset
            .checked_add(*bank)
            .context("SoundFont bank exceeds the MIDI bank range")?;
        if effective_bank > 16_383 {
            bail!("SoundFont bank exceeds the MIDI bank range");
        }
        for channel in 0..16u8 {
            self.send(&[0xb0 | channel, 0, (effective_bank >> 7) as u8])?;
            self.send(&[0xb0 | channel, 32, (effective_bank & 0x7f) as u8])?;
            self.send(&[0xc0 | channel, *program])?;
        }
        Ok(())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.panic();
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
    let mut input = MidiInput::new("SHSynth MIDI input")?;
    input.ignore(Ignore::None);
    let ports = input.ports();
    let matches = |port: &&midir::MidiInputPort, needles: &[&str]| {
        input
            .port_name(port)
            .map(|name| {
                let name = name.to_lowercase();
                needles
                    .iter()
                    .any(|needle| name.contains(&needle.to_lowercase()))
            })
            .unwrap_or(false)
    };
    let configured = pads
        .input_match
        .as_ref()
        .and_then(|wanted| ports.iter().find(|port| matches(port, &[wanted])));
    let port = configured
        .or_else(|| {
            config
                .midi_input_matches
                .iter()
                .find_map(|wanted| ports.iter().find(|port| matches(port, &[wanted])))
        })
        .ok_or_else(|| {
            anyhow!(
                "MIDI input not found (wanted: {})",
                config.midi_input_matches.join(", ")
            )
        })?;
    let input_name = input.port_name(port).unwrap_or_default();
    let output2 = Arc::clone(&output);
    let mut pad_locked = false;
    let mut lock_pressed = false;
    let mut preview_notes: [Option<(crate::sequencer::PageTarget, u8, u8)>; 128] =
        std::array::from_fn(|_| None);
    let mut preview_programs = std::collections::BTreeMap::new();
    let mut route_revision = 0;
    let connection = input
        .connect(
            port,
            "SHSynth monitor",
            move |_stamp, message, _| {
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
                    {
                        let status = message.first().copied().unwrap_or(0);
                        let note_message =
                            message.len() >= 3 && matches!(status & 0xf0, 0x80 | 0x90);
                        let route = tracker_route.lock().ok().map(|route| route.clone());
                        let tracker_consumes_note =
                            tracker_edit_consumes_note(route.as_ref(), message);
                        if route
                            .as_ref()
                            .is_some_and(|route| route.revision != route_revision)
                        {
                            route_revision = route.as_ref().map_or(0, |route| route.revision);
                            preview_programs.clear();
                        }
                        let mut preview = None;
                        let mut program = None;
                        if note_message {
                            let source_note = message[1];
                            let note_off = status & 0xf0 == 0x80 || message[2] == 0;
                            if note_off {
                                preview =
                                    preview_notes[usize::from(source_note)].take().or_else(|| {
                                        route.as_ref().and_then(|route| {
                                            route.enabled.then(|| {
                                                (
                                                    route.target.clone(),
                                                    route.channel,
                                                    route.mapped_note(source_note),
                                                )
                                            })
                                        })
                                    });
                            } else if let Some(route) = route.filter(|route| route.enabled) {
                                let destination = (
                                    route.target.clone(),
                                    route.channel,
                                    route.mapped_note(source_note),
                                );
                                preview_notes[usize::from(source_note)] = Some(destination.clone());
                                preview = Some(destination);
                                program = route.program;
                            }
                        }
                        if let Some((target, channel, note)) = preview {
                            let preview_message = [status & 0xf0 | channel, note, message[2]];
                            let _ = tx.send(MidiEvent::Raw(preview_message.to_vec()));
                            if let Ok(input) = tracker_input.lock() {
                                if let Some(input) = input.as_ref() {
                                    if let Some(program) = program.filter(|program| {
                                        preview_programs.get(&(target.clone(), channel))
                                            != Some(program)
                                    }) {
                                        input.send(&target, &[0xc0 | channel, program]);
                                        preview_programs.insert((target.clone(), channel), program);
                                    }
                                    input.send(&target, &preview_message);
                                }
                            }
                        } else if !tracker_consumes_note {
                            let _ = tx.send(MidiEvent::Raw(message.to_vec()));
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

fn disconnect_direct_midi(source: &str, client_name: &str) {
    disconnect_midi_routes(source, &[client_name]);
}

/// Removes only subscriptions whose source and destination names match the
/// supplied SHSynth-owned/configured clients. Other clients and routes are
/// never reconfigured.
pub fn disconnect_midi_routes(source_match: &str, destination_matches: &[&str]) {
    let clients = parse_alsa_clients(&command_lines("aconnect", &["-l"]));
    let source = clients.iter().find(|(_, name)| {
        name.to_ascii_lowercase()
            .contains(&source_match.to_ascii_lowercase())
    });
    let Some((source_id, _)) = source else {
        return;
    };
    for (destination_id, name) in &clients {
        if destination_matches.iter().any(|wanted| {
            name.to_ascii_lowercase()
                .contains(&wanted.to_ascii_lowercase())
        }) {
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
}

/// Keeps only the configured destination on an SHSynth-owned ALSA source port.
/// Desktop auto-subscriptions are removed without touching another client's
/// source routes.
pub fn retain_midi_destination(source_match: &str, destination_match: &str) {
    let lines = command_lines("aconnect", &["-l"]);
    let clients = parse_alsa_clients(&lines);
    let Some((source_id, _)) = clients.iter().find(|(_, name)| {
        name.to_ascii_lowercase()
            .contains(&source_match.to_ascii_lowercase())
    }) else {
        return;
    };
    let destination = destination_match.to_ascii_lowercase();
    for (client, port) in parse_alsa_destinations(&lines, *source_id) {
        let keep = clients
            .iter()
            .find(|(id, _)| *id == client)
            .is_some_and(|(_, name)| name.to_ascii_lowercase().contains(&destination));
        if !keep {
            let _ = Command::new("aconnect")
                .args(["-d", &format!("{source_id}:0"), &format!("{client}:{port}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
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
    let output = MidiOutput::new("SHSynth MIDI output")?;
    let ports = output.ports();
    let port = ports
        .iter()
        .find(|port| {
            output
                .port_name(port)
                .map(|name| name.to_lowercase().contains(&output_match.to_lowercase()))
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            anyhow!(
                "{} MIDI output matching {output_match:?} not found",
                backend.label()
            )
        })?;
    let connection = output
        .connect(port, "SHSynth forward")
        .map_err(|error| anyhow!("connect {} MIDI output: {error}", backend.label()))?;
    *shared
        .lock()
        .map_err(|_| anyhow!("MIDI output lock poisoned"))? = Some(connection);
    Ok(())
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
    let client = client_name.to_ascii_lowercase();
    let outputs: Vec<_> = jack_ports()
        .into_iter()
        .filter(|port| {
            let lower = port.to_ascii_lowercase();
            lower.contains(&client)
                && (lower.contains("out")
                    || lower.contains("audio")
                    || lower.ends_with(":left")
                    || lower.ends_with(":right"))
        })
        .collect();
    for (source, destination) in outputs.iter().zip(config.audio_outputs.iter()) {
        let _ = Command::new("jack_connect")
            .args([source.as_str(), destination.as_str()])
            .status();
    }
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

fn write_owner(path: &Path, pid: i32) -> Result<()> {
    let owner = owner_for(pid).context("read spawned process identity")?;
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
    owner_for(owner.pid).is_some_and(|current| current == *owner)
}

fn stop_owned(path: &Path, timeout: Duration) {
    let Some(owner) = read_owner(path) else {
        // Legacy bare-PID files are deliberately not trusted: PID reuse could
        // otherwise terminate an unrelated live process.
        let _ = fs::remove_file(path);
        return;
    };
    if still_owned(&owner) {
        unsafe {
            libc::kill(owner.pid, libc::SIGTERM);
        }
        let deadline = Instant::now() + timeout;
        while still_owned(&owner) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        if still_owned(&owner) {
            unsafe {
                libc::kill(owner.pid, libc::SIGKILL);
            }
        }
    }
    let _ = fs::remove_file(path);
}

pub fn stop_managed(state: &Path) -> Result<()> {
    stop_owned(&state.join("daemon.pid"), Duration::from_secs(3));
    stop_owned(&state.join("engine.pid"), Duration::from_secs(2));
    cleanup_state(state);
    Ok(())
}

pub fn status(state: &Path) -> String {
    match (
        read_owner(&state.join("engine.pid")),
        fs::read_to_string(state.join("current")),
    ) {
        (Some(owner), Ok(name)) if still_owned(&owner) => format!(
            "Running: {}\nPID: {}\nAudio: JACK\nMIDI: monitored and forwarded by SHSynth",
            name.trim(),
            owner.pid
        ),
        _ => "SHSynth is stopped.".to_owned(),
    }
}

pub fn daemon(preset: Preset, state: PathBuf, config: RuntimeConfig) -> Result<()> {
    fs::create_dir_all(&state)?;
    let (tx, _rx) = std::sync::mpsc::channel();
    let router = MidiRouter::start(&state, &config, tx)?;
    if let Ok(mut backend) = router.backend().lock() {
        *backend = preset.backend;
    }
    router.arm_pickup(&initial_values(&preset)?);
    let mut engine = Engine::start(&preset, &state, router.output(), &config)?;
    write_owner(&state.join("daemon.pid"), std::process::id() as i32)?;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&stop))?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&stop))?;
    while !stop.load(std::sync::atomic::Ordering::Relaxed) && engine.alive() {
        thread::sleep(Duration::from_millis(100));
    }
    drop(engine);
    let _ = fs::remove_file(state.join("daemon.pid"));
    Ok(())
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
        route.configure(
            true,
            crate::sequencer::PageTarget::ConfiguredExternal,
            2,
            true,
            9,
            &config,
        );
        assert!(route.enabled);
        assert_eq!(route.channel, 2);
        assert_eq!(route.program, Some(9));
        assert_eq!(route.mapped_note(60), 36);
        assert_eq!(route.mapped_note(61), 38);
        assert_eq!(route.mapped_note(72), 72);
        assert!(tracker_edit_consumes_note(Some(&route), &[0x90, 60, 100]));
        assert!(tracker_edit_consumes_note(Some(&route), &[0x90, 60, 0]));
        assert!(tracker_edit_consumes_note(Some(&route), &[0x80, 60, 0]));
        assert!(!tracker_edit_consumes_note(Some(&route), &[0xb0, 1, 64]));
        route.configure(
            false,
            crate::sequencer::PageTarget::ConfiguredExternal,
            2,
            true,
            9,
            &config,
        );
        assert!(!tracker_edit_consumes_note(Some(&route), &[0x90, 60, 100]));
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
    fn alsa_graph_parser_finds_clients_and_owned_destinations() {
        let lines = [
            "client 28: 'AudioBox USB 96' [type=kernel]".into(),
            "client 133: 'shs-casio' [type=user]".into(),
            "    0 'SHSynth accompaniment'".into(),
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
    fn verified_owned_process_is_stopped_and_marker_is_cleaned() {
        let dir = std::env::temp_dir().join(format!("shsynth-owned-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        write_owner(&dir.join("engine.pid"), child.id() as i32).unwrap();
        stop_managed(&dir).unwrap();
        assert!(child.wait().unwrap().code().is_none());
        assert!(!dir.join("engine.pid").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fluid_bank_selection_encodes_14_bit_mma_banks() {
        let bank = 258u16;
        assert_eq!([(bank >> 7) as u8, (bank & 0x7f) as u8], [2, 2]);
    }
}
