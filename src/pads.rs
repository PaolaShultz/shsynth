use anyhow::{bail, Context, Result};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, Instant};

const DEFAULT_CONTROLLER_CONFIG: &str = include_str!("../config/controller.conf");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PadAction {
    Page1,
    Page2,
    Page3,
    Page4,
    CyclePage,
    Item1,
    Item2,
    Item3,
    Item4,
    // Legacy v1 names retain the physical eight-pad order.  They are
    // normalized into the new page/item model by `menu_input`.
    Arp,
    Pad,
    Prog,
    Loop,
    Stop,
    Play,
    Rec,
    TapTempo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerLayout {
    Eight,
    Five,
    Four,
}

impl Default for ControllerLayout {
    fn default() -> Self {
        Self::Eight
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuInput {
    SelectPage(usize),
    CyclePage,
    ActivateItem(usize),
}

impl PadAction {
    pub const fn menu_input(self) -> MenuInput {
        match self {
            Self::Page1 | Self::Arp => MenuInput::SelectPage(0),
            Self::Page2 | Self::Pad => MenuInput::SelectPage(1),
            Self::Page3 | Self::Prog => MenuInput::SelectPage(2),
            Self::Page4 | Self::Loop => MenuInput::SelectPage(3),
            Self::CyclePage => MenuInput::CyclePage,
            Self::Item1 | Self::Stop => MenuInput::ActivateItem(0),
            Self::Item2 | Self::Play => MenuInput::ActivateItem(1),
            Self::Item3 | Self::Rec => MenuInput::ActivateItem(2),
            Self::Item4 | Self::TapTempo => MenuInput::ActivateItem(3),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncoderAction {
    Up,
    Down,
    Select,
}

impl fmt::Display for PadAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Page1 => "page-1",
            Self::Page2 => "page-2",
            Self::Page3 => "page-3",
            Self::Page4 => "page-4",
            Self::CyclePage => "page-cycle",
            Self::Item1 => "item-1",
            Self::Item2 => "item-2",
            Self::Item3 => "item-3",
            Self::Item4 => "item-4",
            Self::Arp => "arp",
            Self::Pad => "pad",
            Self::Prog => "prog",
            Self::Loop => "loop",
            Self::Stop => "stop",
            Self::Play => "play",
            Self::Rec => "rec",
            Self::TapTempo => "tap-tempo",
        })
    }
}

impl FromStr for PadAction {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "page-1" | "page1" => Ok(Self::Page1),
            "page-2" | "page2" => Ok(Self::Page2),
            "page-3" | "page3" => Ok(Self::Page3),
            "page-4" | "page4" => Ok(Self::Page4),
            "page-cycle" | "cycle-page" | "cycle" => Ok(Self::CyclePage),
            "item-1" | "item1" => Ok(Self::Item1),
            "item-2" | "item2" => Ok(Self::Item2),
            "item-3" | "item3" => Ok(Self::Item3),
            "item-4" | "item4" => Ok(Self::Item4),
            "arp" => Ok(Self::Arp),
            "pad" => Ok(Self::Pad),
            "prog" => Ok(Self::Prog),
            "loop" => Ok(Self::Loop),
            "stop" | "stop-record" | "stop-recording" | "panic" | "stop-synth" => Ok(Self::Stop),
            "play" | "play-stop" => Ok(Self::Play),
            "rec" | "record" | "start-recording" => Ok(Self::Rec),
            "tap" | "tap-tempo" => Ok(Self::TapTempo),
            _ => bail!("unknown pad action: {value}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PadConfig {
    pub input_match: Option<String>,
    pub pads: HashMap<u8, PadAction>,
    /// Optional zero-based MIDI channel for each note command. Missing keeps
    /// the legacy behavior of matching the note on every channel.
    pub pad_channels: HashMap<u8, u8>,
    /// Incoming controller CC buttons. Note buttons remain in `pads` for
    /// compatibility with the original profile format.
    pub cc_buttons: HashMap<u8, PadAction>,
    /// Optional zero-based MIDI channel for each CC command.
    pub cc_button_channels: HashMap<u8, u8>,
    /// Incoming controller CC -> synthv1 mapped CC from control::CONTROLS.
    pub controls: HashMap<u8, u8>,
    pub encoder_relative_cc: Option<u8>,
    pub encoder_relative_reverse: bool,
    pub encoder_press_cc: Option<u8>,
    pub encoder_press_note: Option<u8>,
    /// Dedicated toggle control; this uses the raw Shift CC, not its shifted pad layer.
    pub lock_cc: Option<u8>,
    pub layout: ControllerLayout,
}

impl Default for PadConfig {
    fn default() -> Self {
        let mut config = Self {
            input_match: None,
            pads: HashMap::new(),
            pad_channels: HashMap::new(),
            cc_buttons: HashMap::new(),
            cc_button_channels: HashMap::new(),
            controls: HashMap::new(),
            encoder_relative_cc: None,
            encoder_relative_reverse: false,
            encoder_press_cc: None,
            encoder_press_note: None,
            lock_cc: None,
            layout: ControllerLayout::Eight,
        };
        config
            .merge(
                DEFAULT_CONTROLLER_CONFIG,
                Path::new("config/controller.conf"),
            )
            .expect("bundled controller.conf must be valid");
        config
    }
}

impl PadConfig {
    pub fn unmapped(input_match: impl Into<String>) -> Self {
        Self {
            input_match: Some(input_match.into()),
            ..Self::default()
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let mut config = Self::default();
        config.merge(&text, path)?;
        Ok(config)
    }

    fn merge(&mut self, text: &str, path: &Path) -> Result<()> {
        let mut saw_pads = false;
        let mut saw_cc_buttons = false;
        let mut saw_controls = false;
        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=').with_context(|| {
                format!("{}:{}: expected KEY=VALUE", path.display(), line_no + 1)
            })?;
            if key.trim() == "input" {
                self.input_match = (!value.trim().is_empty()).then(|| value.trim().to_owned());
                continue;
            }
            if key.trim() == "menu.layout" {
                self.layout = match value.trim() {
                    "8" | "eight" => ControllerLayout::Eight,
                    "5" | "five" => ControllerLayout::Five,
                    "4" | "four" => ControllerLayout::Four,
                    _ => bail!("menu.layout must be 8, 5, or 4"),
                };
                continue;
            }
            if key.trim() == "encoder.relative_cc" {
                self.encoder_relative_cc = optional_midi_number(value, "encoder relative CC")?;
                continue;
            }
            if key.trim() == "encoder.relative_reverse" {
                self.encoder_relative_reverse = match value.trim() {
                    "true" | "yes" | "1" => true,
                    "false" | "no" | "0" => false,
                    _ => bail!("encoder.relative_reverse must be true or false"),
                };
                continue;
            }
            if key.trim() == "encoder.press_cc" {
                self.encoder_press_cc = optional_midi_number(value, "encoder press CC")?;
                continue;
            }
            if key.trim() == "encoder.press_note" {
                self.encoder_press_note = optional_midi_number(value, "encoder press note")?;
                continue;
            }
            if key.trim() == "lock.cc" {
                self.lock_cc = optional_midi_number(value, "pad lock CC")?;
                continue;
            }
            if let Some(raw) = key.trim().strip_prefix("cc.") {
                if !saw_controls {
                    self.controls.clear();
                    saw_controls = true;
                }
                let raw = midi_number(raw, "controller CC")?;
                let target: u8 = value
                    .trim()
                    .parse()
                    .context("target CC must be a mapped CC number")?;
                if crate::control::by_cc(target).is_none() {
                    bail!("target CC {target} is not one of the 12 mapped controls");
                }
                self.controls.insert(raw, target);
                continue;
            }
            if let Some(raw) = key.trim().strip_prefix("button.cc.") {
                if !saw_cc_buttons {
                    self.cc_buttons.clear();
                    self.cc_button_channels.clear();
                    saw_cc_buttons = true;
                }
                let (channel, raw) = command_binding(raw, "controller button CC")?;
                self.cc_buttons.insert(raw, value.trim().parse()?);
                match channel {
                    Some(channel) => {
                        self.cc_button_channels.insert(raw, channel);
                    }
                    None => {
                        self.cc_button_channels.remove(&raw);
                    }
                }
                continue;
            }
            if !saw_pads {
                self.pads.clear();
                self.pad_channels.clear();
                saw_pads = true;
            }
            let note_text = key.trim().strip_prefix("pad.").unwrap_or(key.trim());
            let (channel, note) = command_binding(note_text, "pad note")?;
            self.pads.insert(note, value.trim().parse()?);
            match channel {
                Some(channel) => {
                    self.pad_channels.insert(note, channel);
                }
                None => {
                    self.pad_channels.remove(&note);
                }
            }
        }
        self.validate()
    }

    pub fn validate(&self) -> Result<()> {
        if self.input_match.as_ref().is_some_and(|input| {
            input.trim().is_empty() || input.trim() != input || input.contains(['\n', '\r'])
        }) {
            bail!("controller input match must be a non-empty single-line value");
        }
        for &cc in self.controls.keys() {
            ensure_midi_number(cc, "controller CC")?;
        }
        for &target in self.controls.values() {
            if crate::control::by_cc(target).is_none() {
                bail!("target CC {target} is not one of the 12 mapped controls");
            }
        }
        for &cc in self.cc_buttons.keys() {
            ensure_midi_number(cc, "controller button CC")?;
        }
        for &note in self.pads.keys() {
            ensure_midi_number(note, "pad note")?;
        }
        if self
            .pad_channels
            .iter()
            .any(|(note, channel)| !self.pads.contains_key(note) || *channel > 15)
        {
            bail!("pad channel qualifiers require a mapped note and channel 1..16");
        }
        if self
            .cc_button_channels
            .iter()
            .any(|(cc, channel)| !self.cc_buttons.contains_key(cc) || *channel > 15)
        {
            bail!("button CC channel qualifiers require a mapped CC and channel 1..16");
        }
        for (number, description) in [
            (self.encoder_relative_cc, "encoder relative CC"),
            (self.encoder_press_cc, "encoder press CC"),
            (self.encoder_press_note, "encoder press note"),
            (self.lock_cc, "pad lock CC"),
        ] {
            if let Some(number) = number {
                ensure_midi_number(number, description)?;
            }
        }
        for encoder_cc in [
            self.encoder_relative_cc,
            self.encoder_press_cc,
            self.lock_cc,
        ]
        .into_iter()
        .flatten()
        {
            if self.controls.contains_key(&encoder_cc) {
                bail!("encoder CC {encoder_cc} is also mapped as a synth control");
            }
            if self.cc_buttons.contains_key(&encoder_cc) {
                bail!("encoder CC {encoder_cc} is also mapped as a command button");
            }
        }
        if self
            .controls
            .keys()
            .any(|cc| self.cc_buttons.contains_key(cc))
        {
            bail!("a controller CC cannot be both continuous and a command button");
        }
        if self.encoder_relative_cc == self.encoder_press_cc && self.encoder_relative_cc.is_some() {
            bail!("encoder turn and press CCs must be different");
        }
        if self.lock_cc.is_some()
            && [self.encoder_relative_cc, self.encoder_press_cc].contains(&self.lock_cc)
        {
            bail!("pad lock CC must differ from encoder CCs");
        }
        if self.encoder_press_cc.is_some() && self.encoder_press_note.is_some() {
            bail!("encoder press must use either a CC or a note, not both");
        }
        if self
            .encoder_press_note
            .is_some_and(|note| self.pads.contains_key(&note))
        {
            bail!("encoder press note is also mapped as a command button");
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut entries: Vec<_> = self.pads.iter().collect();
        entries.sort_by_key(|(note, _)| **note);
        let mut text = String::from("# SHR-DAW controller profile v4\n");
        if let Some(input) = &self.input_match {
            text.push_str(&format!("input={input}\n"));
        }
        text.push_str(&format!(
            "menu.layout={}\nencoder.relative_cc={}\nencoder.relative_reverse={}\nencoder.press_cc={}\nencoder.press_note={}\nlock.cc={}\n",
            match self.layout {
                ControllerLayout::Eight => 8,
                ControllerLayout::Five => 5,
                ControllerLayout::Four => 4,
            },
            self.encoder_relative_cc
                .map(|cc| cc.to_string())
                .unwrap_or_default(),
            self.encoder_relative_reverse,
            self.encoder_press_cc
                .map(|cc| cc.to_string())
                .unwrap_or_default(),
            self.encoder_press_note
                .map(|note| note.to_string())
                .unwrap_or_default(),
            self.lock_cc.map(|cc| cc.to_string()).unwrap_or_default(),
        ));
        let mut controls: Vec<_> = self.controls.iter().collect();
        controls.sort_by_key(|(cc, _)| **cc);
        for (incoming, target) in controls {
            text.push_str(&format!("cc.{incoming}={target}\n"));
        }
        let mut cc_buttons: Vec<_> = self.cc_buttons.iter().collect();
        cc_buttons.sort_by_key(|(cc, _)| **cc);
        for (cc, action) in cc_buttons {
            if let Some(channel) = self.cc_button_channels.get(cc) {
                text.push_str(&format!("button.cc.{}.{cc}={action}\n", channel + 1));
            } else {
                text.push_str(&format!("button.cc.{cc}={action}\n"));
            }
        }
        for (note, action) in entries {
            if let Some(channel) = self.pad_channels.get(note) {
                text.push_str(&format!("pad.{}.{note}={action}\n", channel + 1));
            } else {
                text.push_str(&format!("pad.{note}={action}\n"));
            }
        }
        crate::fsutil::atomic_write(path, text.as_bytes())
    }

    /// Returns an action only for note-on with non-zero velocity. Note-off is
    /// consumed too, preventing both stuck notes and double triggering.
    pub fn route(&self, message: &[u8]) -> (bool, Option<PadAction>) {
        if message.len() < 3 {
            return (false, None);
        }
        let kind = message[0] & 0xf0;
        if !matches!(kind, 0x80 | 0x90 | 0xa0) {
            return (false, None);
        }
        match self.note_action(message[0], message[1]) {
            Some(action) => (true, (kind == 0x90 && message[2] > 0).then_some(action)),
            None => (false, None),
        }
    }

    pub fn action_state(&self, message: &[u8]) -> Option<(PadAction, bool)> {
        if message.len() < 3 {
            return None;
        }
        let kind = message[0] & 0xf0;
        if kind == 0xb0 {
            return self
                .cc_action(message[0], message[1])
                .map(|action| (action, message[2] > 0));
        }
        if kind != 0x90 && kind != 0x80 {
            return None;
        }
        self.note_action(message[0], message[1]).map(|action| {
            let pressed = kind == 0x90 && message[2] > 0;
            (action, pressed)
        })
    }

    fn note_action(&self, status: u8, note: u8) -> Option<PadAction> {
        self.pads.get(&note).copied().filter(|_| {
            self.pad_channels
                .get(&note)
                .is_none_or(|channel| *channel == status & 0x0f)
        })
    }

    fn cc_action(&self, status: u8, cc: u8) -> Option<PadAction> {
        self.cc_buttons.get(&cc).copied().filter(|_| {
            self.cc_button_channels
                .get(&cc)
                .is_none_or(|channel| *channel == status & 0x0f)
        })
    }

    pub fn target_cc(&self, incoming: u8) -> Option<u8> {
        self.controls.get(&incoming).copied()
    }

    /// Arturia relative mode uses 64 as stationary, lower values for left and
    /// higher values for right. Press and release are both consumed, while
    /// only a non-zero press selects.
    pub fn encoder_action(&self, message: &[u8]) -> (bool, Option<EncoderAction>) {
        if message.len() < 3 || message[0] & 0xf0 != 0xb0 {
            return (false, None);
        }
        if self.encoder_relative_cc == Some(message[1]) {
            let mut action = match message[2].cmp(&64) {
                std::cmp::Ordering::Less => Some(EncoderAction::Up),
                std::cmp::Ordering::Greater => Some(EncoderAction::Down),
                std::cmp::Ordering::Equal => None,
            };
            if self.encoder_relative_reverse {
                action = action.map(|action| match action {
                    EncoderAction::Up => EncoderAction::Down,
                    EncoderAction::Down => EncoderAction::Up,
                    EncoderAction::Select => EncoderAction::Select,
                });
            }
            return (true, action);
        }
        if self.encoder_press_cc == Some(message[1]) {
            return (true, (message[2] > 0).then_some(EncoderAction::Select));
        }
        (false, None)
    }

    pub fn encoder_note_action(&self, message: &[u8]) -> (bool, Option<EncoderAction>) {
        if message.len() < 3 || !matches!(message[0] & 0xf0, 0x80 | 0x90) {
            return (false, None);
        }
        if self.encoder_press_note != Some(message[1]) {
            return (false, None);
        }
        let pressed = message[0] & 0xf0 == 0x90 && message[2] > 0;
        (true, pressed.then_some(EncoderAction::Select))
    }

    /// Press and release are consumed; only a non-zero press toggles the lock.
    pub fn lock_action(&self, message: &[u8]) -> (bool, bool) {
        if message.len() < 3 || message[0] & 0xf0 != 0xb0 || self.lock_cc != Some(message[1]) {
            return (false, false);
        }
        (true, message[2] > 0)
    }
}

pub(crate) fn midi_number(value: &str, description: &str) -> Result<u8> {
    let number = value
        .parse::<u8>()
        .with_context(|| format!("{description} must be 0..127"))?;
    ensure_midi_number(number, description)?;
    Ok(number)
}

pub(crate) fn ensure_midi_number(number: u8, description: &str) -> Result<()> {
    if number > 127 {
        bail!("{description} must be 0..127");
    }
    Ok(())
}

fn optional_midi_number(value: &str, description: &str) -> Result<Option<u8>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    midi_number(value, description).map(Some)
}

fn command_binding(value: &str, description: &str) -> Result<(Option<u8>, u8)> {
    let Some((channel, number)) = value.split_once('.') else {
        return Ok((None, midi_number(value, description)?));
    };
    if number.contains('.') {
        bail!("{description} binding must be NUMBER or CHANNEL.NUMBER");
    }
    let channel = channel
        .parse::<u8>()
        .with_context(|| format!("{description} channel must be 1..16"))?;
    if !(1..=16).contains(&channel) {
        bail!("{description} channel must be 1..16");
    }
    Ok((Some(channel - 1), midi_number(number, description)?))
}

#[derive(Debug, Default)]
pub struct TapTempo {
    taps: VecDeque<Instant>,
    bpm: Option<f32>,
}

impl TapTempo {
    pub fn tap(&mut self, now: Instant) -> Option<f32> {
        if let Some(last) = self.taps.back() {
            let gap = now.duration_since(*last);
            if !(Duration::from_millis(250)..=Duration::from_secs(2)).contains(&gap) {
                self.taps.clear();
                self.bpm = None;
            }
        }
        self.taps.push_back(now);
        while self.taps.len() > 5 {
            self.taps.pop_front();
        }
        if self.taps.len() >= 2 {
            let mut gaps: Vec<_> = self
                .taps
                .iter()
                .zip(self.taps.iter().skip(1))
                .map(|(a, b)| b.duration_since(*a).as_secs_f32())
                .collect();
            gaps.sort_by(f32::total_cmp);
            let seconds = gaps[gaps.len() / 2];
            self.bpm = Some(60.0 / seconds);
        }
        self.bpm
    }
    pub fn bpm(&self) -> Option<f32> {
        self.bpm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_note_on_triggers_once_and_note_off_is_consumed() {
        let c = PadConfig {
            pads: HashMap::from([(36, PadAction::Rec)]),
            ..PadConfig::default()
        };
        assert_eq!(c.route(&[0x90, 36, 100]), (true, Some(PadAction::Rec)));
        assert_eq!(c.route(&[0x80, 36, 0]), (true, None));
        assert_eq!(c.route(&[0x90, 40, 100]), (false, None));
    }
    #[test]
    fn channel_qualified_note_commands_consume_press_release_zero_release_and_pressure() {
        let c = PadConfig {
            pads: HashMap::from([(36, PadAction::Page1)]),
            pad_channels: HashMap::from([(36, 9)]),
            ..PadConfig::default()
        };
        for channel in 0..16 {
            let expected_press = if channel == 9 {
                (true, Some(PadAction::Page1))
            } else {
                (false, None)
            };
            assert_eq!(c.route(&[0x90 | channel, 36, 100]), expected_press);
            for (kind, value) in [(0x80, 0), (0x90, 0), (0xa0, 72)] {
                assert_eq!(c.route(&[kind | channel, 36, value]), (channel == 9, None));
            }
        }
    }

    #[test]
    fn channel_qualified_cc_commands_match_only_the_configured_channel() {
        let c = PadConfig {
            cc_buttons: HashMap::from([(44, PadAction::Item1)]),
            cc_button_channels: HashMap::from([(44, 9)]),
            ..PadConfig::default()
        };
        for channel in 0..16 {
            let expected_press = (channel == 9).then_some((PadAction::Item1, true));
            let expected_release = (channel == 9).then_some((PadAction::Item1, false));
            assert_eq!(c.action_state(&[0xb0 | channel, 44, 127]), expected_press);
            assert_eq!(c.action_state(&[0xb0 | channel, 44, 0]), expected_release);
        }
    }
    #[test]
    fn relative_encoder_turns_and_press_are_consumed() {
        let c = PadConfig {
            encoder_relative_cc: Some(28),
            encoder_press_cc: Some(118),
            ..PadConfig::default()
        };
        assert_eq!(
            c.encoder_action(&[0xb0, 28, 61]),
            (true, Some(EncoderAction::Up))
        );
        assert_eq!(
            c.encoder_action(&[0xb0, 28, 66]),
            (true, Some(EncoderAction::Down))
        );
        assert_eq!(
            c.encoder_action(&[0xb0, 118, 127]),
            (true, Some(EncoderAction::Select))
        );
        assert_eq!(c.encoder_action(&[0xb0, 118, 0]), (true, None));
    }
    #[test]
    fn older_controller_profile_keeps_unspecified_encoder_controls_unmapped() {
        let path =
            std::env::temp_dir().join(format!("shsynth-controller-{}.conf", std::process::id()));
        fs::write(&path, "input=AudioBox USB 96\ncc.86=74\npad.36=arp\n").unwrap();
        let config = PadConfig::load(&path).unwrap();
        assert_eq!(config.input_match.as_deref(), Some("AudioBox USB 96"));
        assert_eq!(config.controls, HashMap::from([(86, 74)]));
        assert_eq!(config.encoder_relative_cc, None);
        assert_eq!(config.encoder_press_cc, None);
        assert_eq!(config.layout, ControllerLayout::Eight);
        assert_eq!(PadAction::Arp.menu_input(), MenuInput::SelectPage(0));
        assert_eq!(PadAction::TapTempo.menu_input(), MenuInput::ActivateItem(3));
        let _ = fs::remove_file(path);
    }
    #[test]
    fn five_and_four_button_profiles_are_configurable_without_device_constants() {
        let path = std::env::temp_dir().join(format!(
            "shsynth-controller-layout-{}.conf",
            std::process::id()
        ));
        fs::write(
            &path,
            "menu.layout=5\nencoder.relative_cc=12\nencoder.press_cc=13\npad.60=page-cycle\npad.61=item-1\npad.62=item-2\npad.63=item-3\npad.64=item-4\n",
        )
        .unwrap();
        let config = PadConfig::load(&path).unwrap();
        assert_eq!(config.layout, ControllerLayout::Five);
        assert_eq!(config.pads[&60].menu_input(), MenuInput::CyclePage);
        assert_eq!(config.pads[&64].menu_input(), MenuInput::ActivateItem(3));
        let _ = fs::remove_file(path);
    }
    #[test]
    fn qualified_and_legacy_unqualified_commands_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "shsynth-controller-qualified-{}.conf",
            std::process::id()
        ));
        fs::write(
            &path,
            "pad.10.36=page-1\npad.37=page-2\nbutton.cc.10.44=item-1\nbutton.cc.45=item-2\n",
        )
        .unwrap();
        let config = PadConfig::load(&path).unwrap();
        assert_eq!(config.pad_channels, HashMap::from([(36, 9)]));
        assert_eq!(config.cc_button_channels, HashMap::from([(44, 9)]));
        config.save(&path).unwrap();
        let loaded = PadConfig::load(&path).unwrap();
        assert_eq!(loaded.pad_channels, config.pad_channels);
        assert_eq!(loaded.cc_button_channels, config.cc_button_channels);
        assert!(loaded.route(&[0x90, 37, 100]).0);
        assert!(loaded.route(&[0x9f, 37, 100]).0);
        let _ = fs::remove_file(path);
    }
    #[test]
    fn tap_tempo_uses_stable_recent_intervals_and_rejects_long_gap() {
        let t = Instant::now();
        let mut tap = TapTempo::default();
        assert_eq!(tap.tap(t), None);
        assert!((tap.tap(t + Duration::from_millis(500)).unwrap() - 120.0).abs() < 0.1);
        assert_eq!(tap.tap(t + Duration::from_secs(4)), None);
    }
    #[test]
    fn shift_press_toggles_pad_lock_and_release_is_only_consumed() {
        let c = PadConfig {
            lock_cc: Some(27),
            ..PadConfig::default()
        };
        assert_eq!(c.lock_action(&[0xb0, 27, 127]), (true, true));
        assert_eq!(c.lock_action(&[0xb0, 27, 0]), (true, false));
        assert_eq!(c.lock_action(&[0xb0, 28, 127]), (false, false));
    }

    #[test]
    fn reversed_encoder_cc_buttons_and_note_press_are_supported() {
        let c = PadConfig {
            cc_buttons: HashMap::from([(44, PadAction::Item1)]),
            encoder_relative_cc: Some(28),
            encoder_relative_reverse: true,
            encoder_press_note: Some(99),
            ..PadConfig::default()
        };
        assert_eq!(
            c.encoder_action(&[0xb0, 28, 1]),
            (true, Some(EncoderAction::Down))
        );
        assert_eq!(
            c.action_state(&[0xb0, 44, 127]),
            Some((PadAction::Item1, true))
        );
        assert_eq!(
            c.encoder_note_action(&[0x90, 99, 100]),
            (true, Some(EncoderAction::Select))
        );
    }

    #[test]
    fn controller_numbers_are_limited_to_seven_bit_midi_values() {
        let path = std::env::temp_dir().join(format!(
            "shsynth-controller-range-{}.conf",
            std::process::id()
        ));
        for text in [
            "cc.128=74\n",
            "button.cc.128=item-1\n",
            "pad.128=item-1\n",
            "encoder.relative_cc=128\n",
            "encoder.press_cc=128\n",
            "encoder.press_note=128\n",
            "lock.cc=128\n",
            "pad.0.36=item-1\n",
            "pad.17.36=item-1\n",
            "button.cc.17.44=item-1\n",
        ] {
            fs::write(&path, text).unwrap();
            assert!(PadConfig::load(&path).is_err(), "accepted {text:?}");
        }
        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_rejects_conflicting_cli_style_mutations() {
        let path = std::env::temp_dir().join(format!(
            "shsynth-controller-conflict-{}.conf",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let mut config = PadConfig {
            encoder_press_note: Some(36),
            pads: HashMap::from([(36, PadAction::Item1)]),
            ..PadConfig::default()
        };
        assert!(config.save(&path).is_err());

        config = PadConfig {
            encoder_relative_cc: Some(28),
            controls: HashMap::from([(28, 74)]),
            ..PadConfig::default()
        };
        assert!(config.save(&path).is_err());

        config = PadConfig {
            pads: HashMap::from([(36, PadAction::Item1)]),
            pad_channels: HashMap::from([(37, 9)]),
            ..PadConfig::default()
        };
        assert!(config.save(&path).is_err());

        config = PadConfig {
            cc_buttons: HashMap::from([(44, PadAction::Item1)]),
            cc_button_channels: HashMap::from([(44, 16)]),
            ..PadConfig::default()
        };
        assert!(config.save(&path).is_err());

        config = PadConfig {
            encoder_press_cc: Some(118),
            encoder_press_note: Some(99),
            ..PadConfig::default()
        };
        assert!(config.save(&path).is_err());

        config = PadConfig {
            input_match: Some("controller\nmenu.layout=8".into()),
            ..PadConfig::default()
        };
        assert!(config.save(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn unmapped_input_drops_an_old_controller_profile() {
        let old = PadConfig {
            input_match: Some("Old controller".into()),
            pads: HashMap::from([(36, PadAction::Page1)]),
            controls: HashMap::from([(74, 74)]),
            encoder_relative_cc: Some(28),
            ..PadConfig::default()
        };
        assert!(!old.pads.is_empty());

        let selected = PadConfig::unmapped("Unknown controller");
        assert_eq!(selected.input_match.as_deref(), Some("Unknown controller"));
        assert!(selected.pads.is_empty());
        assert!(selected.cc_buttons.is_empty());
        assert!(selected.controls.is_empty());
        assert_eq!(selected.encoder_relative_cc, None);
        assert_eq!(selected.encoder_press_cc, None);
        assert_eq!(selected.encoder_press_note, None);
        assert_eq!(selected.lock_cc, None);
    }

    #[test]
    fn controller_names_can_contain_hash_characters() {
        let path = std::env::temp_dir().join(format!(
            "shsynth-controller-hash-{}.conf",
            std::process::id()
        ));
        fs::write(&path, "# comment\ninput=Controller #1\n").unwrap();
        let config = PadConfig::load(&path).unwrap();
        assert_eq!(config.input_match.as_deref(), Some("Controller #1"));
        let _ = fs::remove_file(path);
    }
}
