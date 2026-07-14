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

#[derive(Clone, Debug)]
pub struct AudioCaptureConfig {
    pub client_name: String,
    pub directory: PathBuf,
    pub inputs: Vec<StereoInputConfig>,
    pub ring_frames: usize,
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
    pub midi_autoconnect: bool,
    pub midi_input_matches: Vec<String>,
    pub audio_autoconnect: bool,
    pub audio_outputs: Vec<String>,
    pub cpu_temperature_path: Option<PathBuf>,
    pub external_midi: ExternalMidiConfig,
    pub capture: AudioCaptureConfig,
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
            midi_autoconnect: false,
            midi_input_matches: Vec::new(),
            audio_autoconnect: false,
            audio_outputs: Vec::new(),
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
        let mut saw_yoshimi_roots = false;
        let mut saw_categories = false;
        let mut saw_soundfonts = false;
        let mut saw_external_channels = false;
        let mut saw_percussion_notes = false;
        let mut saw_capture_inputs = false;
        for (line_no, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
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
                _ => bail!("{}:{}: unknown setting {key}", path.display(), line_no + 1),
            }
        }
        if self.midi_autoconnect && self.midi_input_matches.is_empty() {
            bail!("midi.autoconnect requires at least one midi.input");
        }
        if self.audio_autoconnect && self.audio_outputs.is_empty() {
            bail!("audio.autoconnect requires at least one audio.output");
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
            "# SHSynth runtime and routing configuration v3\n\
             synthv1.command={}\n\
             synthv1.client={}\n\
             synth.startup_timeout_ms={}\n\
             synthv1.presets={}\n\
             synthv1.midi_output={}\n\
             yoshimi.command={}\n\
             yoshimi.client={}\n\
             yoshimi.midi_output={}\n",
            self.synth_command,
            self.client_name,
            self.startup_timeout.as_millis(),
            display_path(self.preset_dir.as_ref()),
            self.midi_output_match,
            self.yoshimi.backend.command,
            self.yoshimi.backend.client_name,
            self.yoshimi.backend.midi_output_match,
        );
        for root in &self.yoshimi.backend.preset_roots {
            text.push_str(&format!("yoshimi.preset_root={}\n", root.display()));
        }
        for category in &self.yoshimi.categories {
            text.push_str(&format!("yoshimi.category={category}\n"));
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
        text.push_str(&format!("midi.autoconnect={}\n", self.midi_autoconnect));
        for input in &self.midi_input_matches {
            text.push_str(&format!("midi.input={input}\n"));
        }
        text.push_str(&format!("audio.autoconnect={}\n", self.audio_autoconnect));
        for output in &self.audio_outputs {
            text.push_str(&format!("audio.output={output}\n"));
        }
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
        text.push_str(&format!(
            "status.cpu_temperature_path={}\n",
            display_path(self.cpu_temperature_path.as_ref())
        ));
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
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
