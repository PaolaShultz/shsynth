use crate::chord::NoteNaming;
use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_CONFIG: &str = include_str!("../config/shsynth.conf");

#[derive(Clone, Debug, Default)]
pub struct BackendConfig {
    pub command: String,
    pub client_name: String,
    pub midi_output_match: String,
    pub preset_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct YoshimiConfig {
    pub backend: BackendConfig,
    pub categories: Vec<String>,
    pub presets_per_category: usize,
}

#[derive(Clone, Debug)]
pub struct FluidSynthConfig {
    pub backend: BackendConfig,
    pub soundfonts: Vec<PathBuf>,
    pub gain: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BankSelectMode {
    Off,
    Cc0,
    Cc0Cc32,
}

#[derive(Clone, Debug)]
pub struct ExternalMidiConfig {
    pub enabled: bool,
    pub client_name: String,
    pub output_match: String,
    pub max_tracks: usize,
    /// Zero-based MIDI channels.
    pub channels: Vec<u8>,
    pub melody_channel: u8,
    pub percussion_channel: Option<u8>,
    pub percussion_program: Option<u8>,
    /// Controller notes beginning at `percussion_input_base` are translated
    /// through this hardware-specific percussion map while tracker editing.
    pub percussion_input_base: u8,
    pub percussion_notes: Vec<u8>,
    pub bank_select: BankSelectMode,
    pub program_changes: bool,
    pub send_transport: bool,
    pub default_tempo: u16,
    pub default_pattern_rows: usize,
    pub steps_per_beat: u8,
    pub live_thru: bool,
    pub profile: String,
    pub gate_percent: u8,
    pub gesture_settle: Duration,
}

#[derive(Clone, Debug)]
pub struct StereoInputConfig {
    pub name: String,
    pub left_port: String,
    pub right_port: String,
}

/// A machine-owned stereo playback route. Projects never persist these JACK
/// names; portable pages resolve through the active runtime configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StereoOutputConfig {
    pub name: String,
    pub left_port: String,
    pub right_port: String,
}

#[derive(Clone, Debug)]
pub struct AudioCaptureConfig {
    pub client_name: String,
    pub directory: PathBuf,
    pub inputs: Vec<StereoInputConfig>,
    pub ring_frames: usize,
}

#[derive(Clone, Debug)]
pub struct LoopPlayerConfig {
    pub client_name: String,
    pub outputs: Vec<String>,
    pub import_directory: PathBuf,
}

#[derive(Clone, Debug)]
pub struct AudioGraphConfig {
    pub enabled: bool,
    pub client_name: String,
    pub maximum_callback_frames: u32,
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// Legacy names remain public so old callers/configurations keep working.
    pub synth_command: String,
    pub client_name: String,
    pub preset_dir: Option<PathBuf>,
    pub midi_output_match: String,
    pub yoshimi: YoshimiConfig,
    pub fluidsynth: FluidSynthConfig,
    pub startup_timeout: Duration,
    pub note_naming: NoteNaming,
    pub midi_autoconnect: bool,
    pub midi_input_matches: Vec<String>,
    pub audio_autoconnect: bool,
    pub audio_outputs: Vec<String>,
    /// Ordered internal-device fallbacks used only when `audio_outputs` is not
    /// currently visible. The analogue route remains a separate final choice.
    pub audio_internal_outputs: Vec<StereoOutputConfig>,
    pub audio_headphone_output: Option<StereoOutputConfig>,
    pub audio_graph: AudioGraphConfig,
    /// Optional zero-based CPU reserved by system setup for the managed engine.
    pub audio_engine_cpu: Option<usize>,
    pub cpu_temperature_path: Option<PathBuf>,
    pub external_midi: ExternalMidiConfig,
    pub capture: AudioCaptureConfig,
    pub loop_player: LoopPlayerConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioRouteState {
    Preferred,
    InternalFallback { name: String },
    HeadphoneFallback { name: String },
    Unavailable,
    Disabled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedAudioRoute {
    pub outputs: Vec<String>,
    pub state: AudioRouteState,
    /// Present only when a preferred route is unavailable. This is runtime
    /// presentation data and must never be written back into configuration.
    pub notice: Option<String>,
}

impl RuntimeConfig {
    /// Select a currently visible stereo pair without mutating remembered
    /// machine configuration. Explicit internal routes are tried in order and
    /// the analogue headphone route is deliberately last.
    pub fn resolve_audio_route(&self, available_ports: &[String]) -> ResolvedAudioRoute {
        if !self.audio_autoconnect {
            return ResolvedAudioRoute {
                outputs: Vec::new(),
                state: AudioRouteState::Disabled,
                notice: None,
            };
        }
        let available = |left: &str, right: &str| {
            left != right
                && available_ports.iter().any(|port| port == left)
                && available_ports.iter().any(|port| port == right)
        };
        if let [left, right] = self.audio_outputs.as_slice() {
            if available(left, right) {
                return ResolvedAudioRoute {
                    outputs: self.audio_outputs.clone(),
                    state: AudioRouteState::Preferred,
                    notice: None,
                };
            }
        }
        let preferred = if self.audio_outputs.is_empty() {
            "unconfigured preferred audio route".to_owned()
        } else {
            self.audio_outputs.join(" + ")
        };
        for route in &self.audio_internal_outputs {
            if available(&route.left_port, &route.right_port) {
                return ResolvedAudioRoute {
                    outputs: vec![route.left_port.clone(), route.right_port.clone()],
                    state: AudioRouteState::InternalFallback {
                        name: route.name.clone(),
                    },
                    notice: Some(format!(
                        "audio fallback: {} · missing {preferred}",
                        route.name
                    )),
                };
            }
        }
        if let Some(route) = &self.audio_headphone_output {
            if available(&route.left_port, &route.right_port) {
                return ResolvedAudioRoute {
                    outputs: vec![route.left_port.clone(), route.right_port.clone()],
                    state: AudioRouteState::HeadphoneFallback {
                        name: route.name.clone(),
                    },
                    notice: Some(format!(
                        "headphone fallback: {} · missing {preferred}",
                        route.name
                    )),
                };
            }
        }
        ResolvedAudioRoute {
            // Retain the preferred pair for diagnostics and a later safe
            // activation; connection attempts may fail but configuration is
            // never replaced by the absence of hardware.
            outputs: self.audio_outputs.clone(),
            state: AudioRouteState::Unavailable,
            notice: Some(format!("audio unavailable · missing {preferred}")),
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let mut config = Self {
            synth_command: String::new(),
            client_name: String::new(),
            preset_dir: None,
            midi_output_match: String::new(),
            yoshimi: YoshimiConfig {
                backend: BackendConfig::default(),
                categories: Vec::new(),
                presets_per_category: 0,
            },
            fluidsynth: FluidSynthConfig {
                backend: BackendConfig::default(),
                soundfonts: Vec::new(),
                gain: 0.4,
            },
            startup_timeout: Duration::ZERO,
            note_naming: NoteNaming::German,
            midi_autoconnect: false,
            midi_input_matches: Vec::new(),
            audio_autoconnect: false,
            audio_outputs: Vec::new(),
            audio_internal_outputs: Vec::new(),
            audio_headphone_output: None,
            audio_graph: AudioGraphConfig {
                enabled: false,
                client_name: "shr-graph".into(),
                maximum_callback_frames: 4_096,
            },
            audio_engine_cpu: None,
            cpu_temperature_path: None,
            external_midi: ExternalMidiConfig {
                enabled: false,
                client_name: "shs-tracker".into(),
                output_match: String::new(),
                max_tracks: 4,
                channels: vec![0, 1, 2, 9],
                melody_channel: 0,
                percussion_channel: Some(9),
                percussion_program: None,
                percussion_input_base: 60,
                percussion_notes: Vec::new(),
                bank_select: BankSelectMode::Off,
                program_changes: false,
                send_transport: false,
                default_tempo: 120,
                default_pattern_rows: 64,
                steps_per_beat: 4,
                live_thru: false,
                profile: "unknown-monophonic-safe".into(),
                gate_percent: 80,
                gesture_settle: Duration::from_millis(45),
            },
            capture: AudioCaptureConfig {
                client_name: "shs-recorder".into(),
                directory: expand_home("~/.local/share/shsynth/recordings"),
                inputs: Vec::new(),
                ring_frames: 262_144,
            },
            loop_player: LoopPlayerConfig {
                client_name: "shs-loop".into(),
                outputs: Vec::new(),
                import_directory: expand_home("~/Music"),
            },
        };
        config
            .merge(DEFAULT_CONFIG, Path::new("config/shsynth.conf"))
            .expect("bundled shsynth.conf must be valid");
        config
    }
}

impl RuntimeConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let mut config = Self::default();
        config.merge(&text, path)?;
        Ok(config)
    }

    fn merge(&mut self, text: &str, path: &Path) -> Result<()> {
        let mut saw_midi_input = false;
        let mut saw_audio_output = false;
        let mut saw_audio_internal_output = false;
        let mut saw_yoshimi_roots = false;
        let mut saw_categories = false;
        let mut saw_soundfonts = false;
        let mut saw_external_channels = false;
        let mut saw_percussion_notes = false;
        let mut saw_capture_inputs = false;
        let mut saw_loop_outputs = false;
        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=').with_context(|| {
                format!("{}:{}: expected KEY=VALUE", path.display(), line_no + 1)
            })?;
            let key = key.trim();
            let value = value.trim();
            match key {
                "synth.command" | "synthv1.command" => {
                    self.synth_command = required(key, value)?.into()
                }
                "synth.client" | "synthv1.client" => {
                    self.client_name = required(key, value)?.into()
                }
                "synth.startup_timeout_ms" => {
                    let millis = value
                        .parse::<u64>()
                        .context("startup timeout must be milliseconds")?;
                    if millis == 0 {
                        bail!("startup timeout must be greater than zero");
                    }
                    self.startup_timeout = Duration::from_millis(millis);
                }
                "display.note_names" => {
                    self.note_naming = match value.to_ascii_lowercase().as_str() {
                        "german" => NoteNaming::German,
                        "english" => NoteNaming::English,
                        _ => bail!("{key} must be german or english"),
                    }
                }
                "presets.directory" | "synthv1.presets" => {
                    self.preset_dir = (!value.is_empty()).then(|| expand_home(value))
                }
                "synthv1.midi_output" | "midi.synth_output" => {
                    self.midi_output_match = required(key, value)?.into()
                }
                "yoshimi.command" => self.yoshimi.backend.command = required(key, value)?.into(),
                "yoshimi.client" => self.yoshimi.backend.client_name = required(key, value)?.into(),
                "yoshimi.midi_output" => {
                    self.yoshimi.backend.midi_output_match = required(key, value)?.into()
                }
                "yoshimi.preset_root" => {
                    replace_list_once(
                        &mut self.yoshimi.backend.preset_roots,
                        &mut saw_yoshimi_roots,
                    );
                    if !value.is_empty() {
                        self.yoshimi.backend.preset_roots.push(expand_home(value));
                    }
                }
                "yoshimi.category" => {
                    replace_list_once(&mut self.yoshimi.categories, &mut saw_categories);
                    if !value.is_empty() {
                        self.yoshimi.categories.push(value.to_ascii_lowercase());
                    }
                }
                "yoshimi.presets_per_category" => {
                    self.yoshimi.presets_per_category = value
                        .parse::<usize>()
                        .context("Yoshimi presets per category must be a number")?;
                    if self.yoshimi.presets_per_category == 0 {
                        bail!("Yoshimi presets per category must be greater than zero");
                    }
                }
                "fluidsynth.command" => {
                    self.fluidsynth.backend.command = required(key, value)?.into()
                }
                "fluidsynth.client" => {
                    self.fluidsynth.backend.client_name = required(key, value)?.into()
                }
                "fluidsynth.midi_output" => {
                    self.fluidsynth.backend.midi_output_match = required(key, value)?.into()
                }
                "fluidsynth.gain" => {
                    let gain = value
                        .parse::<f32>()
                        .context("FluidSynth gain must be a number")?;
                    if !gain.is_finite() || !(0.0..=10.0).contains(&gain) || gain == 0.0 {
                        bail!("fluidsynth.gain must be greater than 0 and at most 10");
                    }
                    self.fluidsynth.gain = gain;
                }
                "fluidsynth.soundfont" => {
                    replace_list_once(&mut self.fluidsynth.soundfonts, &mut saw_soundfonts);
                    if !value.is_empty() {
                        self.fluidsynth.soundfonts.push(expand_home(value));
                    }
                }
                "midi.autoconnect" => self.midi_autoconnect = boolean(key, value)?,
                "midi.input" => {
                    replace_list_once(&mut self.midi_input_matches, &mut saw_midi_input);
                    if !value.is_empty() {
                        self.midi_input_matches.push(value.to_owned());
                    }
                }
                "audio.autoconnect" => self.audio_autoconnect = boolean(key, value)?,
                "audio.output" => {
                    replace_list_once(&mut self.audio_outputs, &mut saw_audio_output);
                    if !value.is_empty() {
                        self.audio_outputs.push(value.to_owned());
                    }
                }
                "audio.internal_output" => {
                    replace_list_once(
                        &mut self.audio_internal_outputs,
                        &mut saw_audio_internal_output,
                    );
                    if !value.is_empty() {
                        self.audio_internal_outputs.push(stereo_output(key, value)?);
                    }
                }
                "audio.headphone_output" => {
                    self.audio_headphone_output = if value.is_empty() {
                        None
                    } else {
                        Some(stereo_output(key, value)?)
                    }
                }
                "audio.graph.enabled" => self.audio_graph.enabled = boolean(key, value)?,
                "audio.graph.client" => self.audio_graph.client_name = required(key, value)?.into(),
                "audio.graph.maximum_callback_frames" => {
                    self.audio_graph.maximum_callback_frames =
                        bounded_usize(key, value, 1, 4_096)? as u32
                }
                "audio.engine_cpu" => {
                    self.audio_engine_cpu = if value.is_empty() {
                        None
                    } else {
                        Some(bounded_usize(
                            key,
                            value,
                            0,
                            libc::CPU_SETSIZE as usize - 1,
                        )?)
                    }
                }
                "status.cpu_temperature_path" => {
                    self.cpu_temperature_path = (!value.is_empty()).then(|| expand_home(value))
                }
                "external_midi.enabled" => self.external_midi.enabled = boolean(key, value)?,
                "external_midi.client" => {
                    self.external_midi.client_name = required(key, value)?.into()
                }
                "external_midi.output" => self.external_midi.output_match = value.into(),
                "external_midi.max_tracks" => {
                    self.external_midi.max_tracks = bounded_usize(key, value, 1, 16)?
                }
                "external_midi.channel" => {
                    replace_list_once(&mut self.external_midi.channels, &mut saw_external_channels);
                    if !value.is_empty() {
                        let channel = bounded_usize(key, value, 1, 16)? as u8 - 1;
                        self.external_midi.channels.push(channel);
                    }
                }
                "external_midi.melody_channel" => {
                    self.external_midi.melody_channel = bounded_usize(key, value, 1, 16)? as u8 - 1
                }
                "external_midi.percussion_channel" => {
                    self.external_midi.percussion_channel = if value.is_empty() {
                        None
                    } else {
                        Some(bounded_usize(key, value, 1, 16)? as u8 - 1)
                    }
                }
                "external_midi.percussion_program" => {
                    self.external_midi.percussion_program = if value.is_empty() {
                        None
                    } else {
                        Some(bounded_usize(key, value, 0, 127)? as u8)
                    }
                }
                "external_midi.percussion_input_base" => {
                    self.external_midi.percussion_input_base =
                        bounded_usize(key, value, 0, 127)? as u8
                }
                "external_midi.percussion_note" => {
                    replace_list_once(
                        &mut self.external_midi.percussion_notes,
                        &mut saw_percussion_notes,
                    );
                    if !value.is_empty() {
                        self.external_midi
                            .percussion_notes
                            .push(bounded_usize(key, value, 0, 127)? as u8);
                    }
                }
                "external_midi.bank_select" => {
                    self.external_midi.bank_select = match value.to_ascii_lowercase().as_str() {
                        "off" => BankSelectMode::Off,
                        "cc0" => BankSelectMode::Cc0,
                        "cc0+cc32" | "cc0_cc32" => BankSelectMode::Cc0Cc32,
                        _ => bail!("{key} must be off, cc0, or cc0+cc32"),
                    }
                }
                "external_midi.program_changes" => {
                    self.external_midi.program_changes = boolean(key, value)?
                }
                "external_midi.send_transport" => {
                    self.external_midi.send_transport = boolean(key, value)?
                }
                "external_midi.default_tempo" => {
                    self.external_midi.default_tempo = bounded_usize(key, value, 20, 300)? as u16
                }
                "external_midi.pattern_rows" => {
                    self.external_midi.default_pattern_rows = bounded_usize(key, value, 1, 256)?
                }
                "external_midi.steps_per_beat" => {
                    self.external_midi.steps_per_beat = bounded_usize(key, value, 1, 16)? as u8
                }
                "external_midi.live_thru" => self.external_midi.live_thru = boolean(key, value)?,
                "external_midi.profile" => {
                    self.external_midi.profile = required(key, value)?.into()
                }
                "external_midi.gate_percent" => {
                    self.external_midi.gate_percent = bounded_usize(key, value, 1, 100)? as u8
                }
                "external_midi.gesture_settle_ms" => {
                    self.external_midi.gesture_settle =
                        Duration::from_millis(bounded_usize(key, value, 10, 250)? as u64)
                }
                "capture.client" => self.capture.client_name = required(key, value)?.into(),
                "capture.directory" => self.capture.directory = expand_home(required(key, value)?),
                "capture.input" => {
                    replace_list_once(&mut self.capture.inputs, &mut saw_capture_inputs);
                    if !value.is_empty() {
                        let mut fields = value.split('|').map(str::trim);
                        let input = StereoInputConfig {
                            name: required(key, fields.next().unwrap_or(""))?.into(),
                            left_port: required(key, fields.next().unwrap_or(""))?.into(),
                            right_port: required(key, fields.next().unwrap_or(""))?.into(),
                        };
                        if fields.next().is_some() {
                            bail!("{key} must be NAME|LEFT_JACK_PORT|RIGHT_JACK_PORT");
                        }
                        self.capture.inputs.push(input);
                    }
                }
                "capture.ring_frames" => {
                    self.capture.ring_frames = bounded_usize(key, value, 1024, 4_194_304)?
                }
                "loop.client" => self.loop_player.client_name = required(key, value)?.into(),
                "loop.output" => {
                    replace_list_once(&mut self.loop_player.outputs, &mut saw_loop_outputs);
                    if !value.is_empty() {
                        self.loop_player.outputs.push(value.into());
                    }
                }
                "loop.import_directory" => {
                    self.loop_player.import_directory = expand_home(required(key, value)?)
                }
                _ => bail!("{}:{}: unknown setting {key}", path.display(), line_no + 1),
            }
        }
        if self.midi_autoconnect && self.midi_input_matches.is_empty() {
            bail!("midi.autoconnect requires at least one midi.input");
        }
        if self.audio_autoconnect && self.audio_outputs.is_empty() {
            bail!("audio.autoconnect requires at least one audio.output");
        }
        if self.audio_graph.enabled && (!self.audio_autoconnect || self.audio_outputs.len() != 2) {
            bail!("audio.graph.enabled requires audio.autoconnect and two audio.output entries");
        }
        for route in self
            .audio_internal_outputs
            .iter()
            .chain(self.audio_headphone_output.iter())
        {
            if route.left_port == route.right_port {
                bail!(
                    "stereo audio route {:?} must use distinct ports",
                    route.name
                );
            }
        }
        if self.external_midi.enabled && self.external_midi.output_match.is_empty() {
            bail!("external_midi.enabled requires external_midi.output");
        }
        if self.external_midi.channels.is_empty() {
            bail!("external_midi requires at least one channel");
        }
        if self.external_midi.max_tracks > self.external_midi.channels.len() {
            bail!("external_midi.max_tracks exceeds configured channel count");
        }
        if !self
            .external_midi
            .channels
            .contains(&self.external_midi.melody_channel)
        {
            bail!("external_midi.melody_channel must also be an available channel");
        }
        if self
            .external_midi
            .percussion_channel
            .is_some_and(|channel| !self.external_midi.channels.contains(&channel))
        {
            bail!("external_midi.percussion_channel must also be an available channel");
        }
        if usize::from(self.external_midi.percussion_input_base)
            + self.external_midi.percussion_notes.len()
            > 128
        {
            bail!("external_midi percussion input map exceeds MIDI note 127");
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut text = format!(
            "# SHR-DAW runtime and routing configuration v4\n\
             synthv1.command={}\n\
             synthv1.client={}\n\
             synth.startup_timeout_ms={}\n\
             display.note_names={}\n\
             synthv1.presets={}\n\
             synthv1.midi_output={}\n\
             yoshimi.command={}\n\
             yoshimi.client={}\n\
             yoshimi.midi_output={}\n",
            self.synth_command,
            self.client_name,
            self.startup_timeout.as_millis(),
            self.note_naming.config_value(),
            display_path(self.preset_dir.as_ref()),
            self.midi_output_match,
            self.yoshimi.backend.command,
            self.yoshimi.backend.client_name,
            self.yoshimi.backend.midi_output_match,
        );
        for root in &self.yoshimi.backend.preset_roots {
            text.push_str(&format!("yoshimi.preset_root={}\n", root.display()));
        }
        if self.yoshimi.backend.preset_roots.is_empty() {
            text.push_str("yoshimi.preset_root=\n");
        }
        for category in &self.yoshimi.categories {
            text.push_str(&format!("yoshimi.category={category}\n"));
        }
        if self.yoshimi.categories.is_empty() {
            text.push_str("yoshimi.category=\n");
        }
        text.push_str(&format!(
            "yoshimi.presets_per_category={}\nfluidsynth.command={}\nfluidsynth.client={}\nfluidsynth.midi_output={}\nfluidsynth.gain={}\n",
            self.yoshimi.presets_per_category,
            self.fluidsynth.backend.command,
            self.fluidsynth.backend.client_name,
            self.fluidsynth.backend.midi_output_match,
            self.fluidsynth.gain
        ));
        for font in &self.fluidsynth.soundfonts {
            text.push_str(&format!("fluidsynth.soundfont={}\n", font.display()));
        }
        if self.fluidsynth.soundfonts.is_empty() {
            text.push_str("fluidsynth.soundfont=\n");
        }
        text.push_str(&format!("midi.autoconnect={}\n", self.midi_autoconnect));
        for input in &self.midi_input_matches {
            text.push_str(&format!("midi.input={input}\n"));
        }
        if self.midi_input_matches.is_empty() {
            text.push_str("midi.input=\n");
        }
        text.push_str(&format!("audio.autoconnect={}\n", self.audio_autoconnect));
        for output in &self.audio_outputs {
            text.push_str(&format!("audio.output={output}\n"));
        }
        if self.audio_outputs.is_empty() {
            text.push_str("audio.output=\n");
        }
        for output in &self.audio_internal_outputs {
            text.push_str(&format!(
                "audio.internal_output={}|{}|{}\n",
                output.name, output.left_port, output.right_port
            ));
        }
        if self.audio_internal_outputs.is_empty() {
            text.push_str("audio.internal_output=\n");
        }
        if let Some(output) = &self.audio_headphone_output {
            text.push_str(&format!(
                "audio.headphone_output={}|{}|{}\n",
                output.name, output.left_port, output.right_port
            ));
        } else {
            text.push_str("audio.headphone_output=\n");
        }
        text.push_str(&format!(
            "audio.graph.enabled={}\naudio.graph.client={}\naudio.graph.maximum_callback_frames={}\n",
            self.audio_graph.enabled,
            self.audio_graph.client_name,
            self.audio_graph.maximum_callback_frames
        ));
        text.push_str(&format!(
            "audio.engine_cpu={}\n",
            self.audio_engine_cpu
                .map(|cpu| cpu.to_string())
                .unwrap_or_default()
        ));
        text.push_str(&format!(
            "external_midi.enabled={}\nexternal_midi.client={}\nexternal_midi.output={}\nexternal_midi.max_tracks={}\n",
            self.external_midi.enabled,
            self.external_midi.client_name,
            self.external_midi.output_match,
            self.external_midi.max_tracks
        ));
        for channel in &self.external_midi.channels {
            text.push_str(&format!("external_midi.channel={}\n", channel + 1));
        }
        if self.external_midi.channels.is_empty() {
            text.push_str("external_midi.channel=\n");
        }
        text.push_str(&format!(
            "external_midi.melody_channel={}\nexternal_midi.percussion_channel={}\nexternal_midi.percussion_program={}\nexternal_midi.percussion_input_base={}\n",
            self.external_midi.melody_channel + 1,
            self.external_midi
                .percussion_channel
                .map(|c| (c + 1).to_string())
                .unwrap_or_default(),
            self.external_midi
                .percussion_program
                .map(|program| program.to_string())
                .unwrap_or_default(),
            self.external_midi.percussion_input_base,
        ));
        for note in &self.external_midi.percussion_notes {
            text.push_str(&format!("external_midi.percussion_note={note}\n"));
        }
        if self.external_midi.percussion_notes.is_empty() {
            text.push_str("external_midi.percussion_note=\n");
        }
        text.push_str(&format!(
            "external_midi.bank_select={}\nexternal_midi.program_changes={}\nexternal_midi.send_transport={}\nexternal_midi.default_tempo={}\nexternal_midi.pattern_rows={}\nexternal_midi.steps_per_beat={}\nexternal_midi.live_thru={}\nexternal_midi.profile={}\nexternal_midi.gate_percent={}\nexternal_midi.gesture_settle_ms={}\ncapture.client={}\ncapture.directory={}\ncapture.ring_frames={}\n",
            match self.external_midi.bank_select { BankSelectMode::Off => "off", BankSelectMode::Cc0 => "cc0", BankSelectMode::Cc0Cc32 => "cc0+cc32" },
            self.external_midi.program_changes, self.external_midi.send_transport,
            self.external_midi.default_tempo, self.external_midi.default_pattern_rows,
            self.external_midi.steps_per_beat, self.external_midi.live_thru,
            self.external_midi.profile, self.external_midi.gate_percent, self.external_midi.gesture_settle.as_millis(), self.capture.client_name,
            self.capture.directory.display(), self.capture.ring_frames
        ));
        for input in &self.capture.inputs {
            text.push_str(&format!(
                "capture.input={}|{}|{}\n",
                input.name, input.left_port, input.right_port
            ));
        }
        if self.capture.inputs.is_empty() {
            text.push_str("capture.input=\n");
        }
        text.push_str(&format!(
            "loop.client={}\nloop.import_directory={}\n",
            self.loop_player.client_name,
            self.loop_player.import_directory.display()
        ));
        for output in &self.loop_player.outputs {
            text.push_str(&format!("loop.output={output}\n"));
        }
        if self.loop_player.outputs.is_empty() {
            text.push_str("loop.output=\n");
        }
        text.push_str(&format!(
            "status.cpu_temperature_path={}\n",
            display_path(self.cpu_temperature_path.as_ref())
        ));
        let mut validated = Self::default();
        validated
            .merge(&text, path)
            .context("refusing to save invalid runtime configuration")?;
        atomic_write(path, &text)
    }
}

fn replace_list_once<T>(list: &mut Vec<T>, seen: &mut bool) {
    if !*seen {
        list.clear();
        *seen = true;
    }
}

fn display_path(path: Option<&PathBuf>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn stereo_output(key: &str, value: &str) -> Result<StereoOutputConfig> {
    let mut fields = value.split('|').map(str::trim);
    let output = StereoOutputConfig {
        name: required(key, fields.next().unwrap_or(""))?.into(),
        left_port: required(key, fields.next().unwrap_or(""))?.into(),
        right_port: required(key, fields.next().unwrap_or(""))?.into(),
    };
    if fields.next().is_some() {
        bail!("{key} must be NAME|LEFT_JACK_PORT|RIGHT_JACK_PORT");
    }
    Ok(output)
}

fn required<'a>(key: &str, value: &'a str) -> Result<&'a str> {
    if value.is_empty() {
        bail!("{key} may not be empty");
    }
    Ok(value)
}

fn boolean(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => bail!("{key} must be true or false"),
    }
}

fn bounded_usize(key: &str, value: &str, min: usize, max: usize) -> Result<usize> {
    let parsed = value
        .parse::<usize>()
        .with_context(|| format!("{key} must be a number"))?;
    if !(min..=max).contains(&parsed) {
        bail!("{key} must be {min}..={max}");
    }
    Ok(parsed)
}

fn expand_home(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn atomic_write(path: &Path, text: &str) -> Result<()> {
    crate::fsutil::atomic_write(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_naming_defaults_to_german_and_round_trips_english() {
        assert_eq!(RuntimeConfig::default().note_naming, NoteNaming::German);
        let path =
            std::env::temp_dir().join(format!("shsynth-note-names-{}.conf", std::process::id()));
        fs::write(&path, "display.note_names=english\n").unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        assert_eq!(config.note_naming, NoteNaming::English);
        config.save(&path).unwrap();
        assert_eq!(
            RuntimeConfig::load(&path).unwrap().note_naming,
            NoteNaming::English
        );
        fs::write(&path, "display.note_names=solfege\n").unwrap();
        assert!(RuntimeConfig::load(&path).is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_v1_configuration_inherits_optional_engine_defaults() {
        let dir = std::env::temp_dir().join(format!("shsynth-config-{}", std::process::id()));
        let path = dir.join("shsynth.conf");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            "synth.client=my-synth\nmidi.input=Controller X\naudio.output=usb:out_1\n",
        )
        .unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        assert_eq!(config.client_name, "my-synth");
        assert_eq!(config.midi_input_matches, ["Controller X"]);
        assert_eq!(config.audio_outputs, ["usb:out_1"]);
        assert_eq!(config.yoshimi.backend.command, "yoshimi");
        assert_eq!(config.fluidsynth.backend.command, "fluidsynth");
        config.save(&path).unwrap();
        assert_eq!(RuntimeConfig::load(&path).unwrap().client_name, "my-synth");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn configured_backend_lists_replace_defaults() {
        let path =
            std::env::temp_dir().join(format!("shsynth-engines-{}.conf", std::process::id()));
        fs::write(
            &path,
            "yoshimi.preset_root=/sounds/yoshimi\nyoshimi.category=bass\nfluidsynth.soundfont=/sounds/tim.sf2\n",
        )
        .unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        assert_eq!(
            config.yoshimi.backend.preset_roots,
            [PathBuf::from("/sounds/yoshimi")]
        );
        assert_eq!(config.yoshimi.categories, ["bass"]);
        assert_eq!(
            config.fluidsynth.soundfonts,
            [PathBuf::from("/sounds/tim.sf2")]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn loop_routing_and_import_inbox_round_trip_without_hardcoded_ports() {
        let path = std::env::temp_dir().join(format!("shsynth-loop-{}.conf", std::process::id()));
        fs::write(
            &path,
            "loop.client=my-loop\nloop.import_directory=/private/inbox\nloop.output=usb:left\nloop.output=usb:right\n",
        )
        .unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        assert_eq!(config.loop_player.client_name, "my-loop");
        assert_eq!(
            config.loop_player.import_directory,
            PathBuf::from("/private/inbox")
        );
        assert_eq!(config.loop_player.outputs, ["usb:left", "usb:right"]);
        config.save(&path).unwrap();
        let loaded = RuntimeConfig::load(&path).unwrap();
        assert_eq!(loaded.loop_player.outputs, ["usb:left", "usb:right"]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn mt240_style_duplicate_channel_and_sparse_drum_map_are_preserved() {
        let path = std::env::temp_dir().join(format!("shsynth-mt240-{}.conf", std::process::id()));
        fs::write(
            &path,
            "external_midi.channel=1\nexternal_midi.channel=2\nexternal_midi.channel=3\nexternal_midi.channel=3\nexternal_midi.percussion_channel=3\nexternal_midi.percussion_program=9\nexternal_midi.percussion_input_base=60\nexternal_midi.percussion_note=36\nexternal_midi.percussion_note=38\n",
        )
        .unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        assert_eq!(config.external_midi.channels, [0, 1, 2, 2]);
        assert_eq!(config.external_midi.percussion_program, Some(9));
        assert_eq!(config.external_midi.percussion_notes, [36, 38]);
        config.save(&path).unwrap();
        let loaded = RuntimeConfig::load(&path).unwrap();
        assert_eq!(loaded.external_midi.channels, [0, 1, 2, 2]);
        assert_eq!(loaded.external_midi.percussion_notes, [36, 38]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn bundled_mt240_profile_uses_two_page_channels_program_and_settle() {
        let config = RuntimeConfig::default().external_midi;
        assert_eq!(config.profile, "casio-casiotone-mt-240");
        assert_eq!(config.channels, [0, 1]);
        assert_eq!(config.melody_channel, 0);
        assert_eq!(config.percussion_channel, Some(1));
        assert_eq!(config.percussion_program, Some(9));
        assert!(config.program_changes);
        assert_eq!(config.percussion_notes.len(), 12);
        assert_eq!(config.gesture_settle, Duration::from_millis(45));
    }

    #[test]
    fn rejects_unknown_settings() {
        let path = std::env::temp_dir().join(format!("shsynth-bad-{}.conf", std::process::id()));
        fs::write(&path, "audio.typo=yes\n").unwrap();
        assert!(RuntimeConfig::load(&path).is_err());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn audio_engine_cpu_is_optional_and_round_trips() {
        let path =
            std::env::temp_dir().join(format!("shsynth-audio-cpu-{}.conf", std::process::id()));
        fs::write(&path, "audio.engine_cpu=3\n").unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        assert_eq!(config.audio_engine_cpu, Some(3));
        config.save(&path).unwrap();
        assert_eq!(
            RuntimeConfig::load(&path).unwrap().audio_engine_cpu,
            Some(3)
        );
        fs::write(&path, "audio.engine_cpu=\n").unwrap();
        assert_eq!(RuntimeConfig::load(&path).unwrap().audio_engine_cpu, None);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn owned_audio_graph_is_disabled_by_default_and_requires_a_stereo_fallback() {
        let path =
            std::env::temp_dir().join(format!("shsynth-audio-graph-{}.conf", std::process::id()));
        fs::write(
            &path,
            "audio.graph.enabled=true\naudio.graph.client=my-graph\naudio.graph.maximum_callback_frames=256\n",
        )
        .unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        assert!(config.audio_graph.enabled);
        assert_eq!(config.audio_graph.client_name, "my-graph");
        assert_eq!(config.audio_graph.maximum_callback_frames, 256);
        config.save(&path).unwrap();
        assert!(RuntimeConfig::load(&path).unwrap().audio_graph.enabled);

        fs::write(&path, "audio.autoconnect=false\naudio.graph.enabled=true\n").unwrap();
        assert!(RuntimeConfig::load(&path).is_err());
        fs::write(&path, "audio.output=only:left\naudio.graph.enabled=true\n").unwrap();
        assert!(RuntimeConfig::load(&path).is_err());
        let _ = fs::remove_file(path);

        assert!(!RuntimeConfig::default().audio_graph.enabled);
    }

    #[test]
    fn audio_fallback_priority_is_runtime_only_and_headphones_are_last() {
        let mut config = RuntimeConfig::default();
        config.audio_outputs = vec!["usb:l".into(), "usb:r".into()];
        config.audio_internal_outputs = vec![StereoOutputConfig {
            name: "Pi HDMI".into(),
            left_port: "hdmi:l".into(),
            right_port: "hdmi:r".into(),
        }];
        config.audio_headphone_output = Some(StereoOutputConfig {
            name: "Pi analogue".into(),
            left_port: "phones:l".into(),
            right_port: "phones:r".into(),
        });

        let all = ["usb:l", "usb:r", "hdmi:l", "hdmi:r", "phones:l", "phones:r"].map(str::to_owned);
        assert_eq!(
            config.resolve_audio_route(&all).state,
            AudioRouteState::Preferred
        );
        let internal = ["hdmi:l", "hdmi:r", "phones:l", "phones:r"].map(str::to_owned);
        assert_eq!(
            config.resolve_audio_route(&internal).state,
            AudioRouteState::InternalFallback {
                name: "Pi HDMI".into()
            }
        );
        let phones = ["phones:l", "phones:r"].map(str::to_owned);
        assert_eq!(
            config.resolve_audio_route(&phones).state,
            AudioRouteState::HeadphoneFallback {
                name: "Pi analogue".into()
            }
        );
        assert_eq!(config.audio_outputs, ["usb:l", "usb:r"]);
    }

    #[test]
    fn audio_routes_round_trip_and_no_hardware_keeps_preference() {
        let path =
            std::env::temp_dir().join(format!("shsynth-audio-routes-{}.conf", std::process::id()));
        fs::write(
            &path,
            "audio.output=usb:l\naudio.output=usb:r\naudio.internal_output=Built in|soc:l|soc:r\naudio.headphone_output=Analogue|hp:l|hp:r\n",
        )
        .unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        let none = config.resolve_audio_route(&[]);
        assert_eq!(none.state, AudioRouteState::Unavailable);
        assert_eq!(none.outputs, ["usb:l", "usb:r"]);
        config.save(&path).unwrap();
        let loaded = RuntimeConfig::load(&path).unwrap();
        assert_eq!(loaded.audio_internal_outputs, config.audio_internal_outputs);
        assert_eq!(loaded.audio_headphone_output, config.audio_headphone_output);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn explicitly_empty_optional_lists_survive_save_and_reload() {
        let path =
            std::env::temp_dir().join(format!("shsynth-empty-lists-{}.conf", std::process::id()));
        let mut config = RuntimeConfig::default();
        config.midi_autoconnect = false;
        config.audio_autoconnect = false;
        config.yoshimi.backend.preset_roots.clear();
        config.yoshimi.categories.clear();
        config.fluidsynth.soundfonts.clear();
        config.midi_input_matches.clear();
        config.audio_outputs.clear();
        config.audio_internal_outputs.clear();
        config.audio_headphone_output = None;
        config.external_midi.percussion_notes.clear();
        config.capture.inputs.clear();
        config.loop_player.outputs.clear();
        config.save(&path).unwrap();

        let loaded = RuntimeConfig::load(&path).unwrap();
        assert!(loaded.yoshimi.backend.preset_roots.is_empty());
        assert!(loaded.yoshimi.categories.is_empty());
        assert!(loaded.fluidsynth.soundfonts.is_empty());
        assert!(loaded.midi_input_matches.is_empty());
        assert!(loaded.audio_outputs.is_empty());
        assert!(loaded.audio_internal_outputs.is_empty());
        assert!(loaded.audio_headphone_output.is_none());
        assert!(loaded.external_midi.percussion_notes.is_empty());
        assert!(loaded.capture.inputs.is_empty());
        assert!(loaded.loop_player.outputs.is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_refuses_invalid_in_memory_configuration() {
        let path =
            std::env::temp_dir().join(format!("shsynth-invalid-save-{}.conf", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut config = RuntimeConfig::default();
        config.external_midi.channels.clear();
        assert!(config.save(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn hash_characters_are_preserved_inside_values() {
        let path =
            std::env::temp_dir().join(format!("shsynth-hash-value-{}.conf", std::process::id()));
        fs::write(
            &path,
            "# full-line comment\nsynth.client=synth #1\nmidi.input=Controller #1\n",
        )
        .unwrap();
        let config = RuntimeConfig::load(&path).unwrap();
        assert_eq!(config.client_name, "synth #1");
        assert_eq!(config.midi_input_matches, ["Controller #1"]);
        let _ = fs::remove_file(path);
    }
}
