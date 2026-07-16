//! Multi-destination FT2-style sequencing. Song editing/storage and event
//! planning remain independent from the owned software-synth lifecycle.
use crate::config::{BankSelectMode, ExternalMidiConfig};
use anyhow::{anyhow, bail, Context, Result};
use midir::{MidiOutput, MidiOutputConnection};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const SONG_VERSION: u8 = 0;
pub const LANES_PER_PAGE: usize = 4;
#[cfg(test)]
const DEFAULT_GESTURE_SETTLE: Duration = Duration::from_millis(45);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Song {
    pub name: String,
    pub steps_per_beat: u8,
    pub gate_percent: u8,
    pub audio_loop: Option<LoopSettings>,
    pub order: Vec<u16>,
    pub patterns: BTreeMap<u16, Pattern>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BpmInterpretation {
    Half,
    #[default]
    Normal,
    Double,
}

impl BpmInterpretation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Half => "1/2x",
            Self::Normal => "1x",
            Self::Double => "2x",
        }
    }

    pub fn apply(self, bpm: f64) -> f64 {
        match self {
            Self::Half => bpm / 2.0,
            Self::Normal => bpm,
            Self::Double => bpm * 2.0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopSettings {
    /// Filename only; imported files always live in the private loop store.
    pub file: String,
    /// Hundredths of a BPM. WAV files are not assumed to contain BPM metadata.
    pub source_bpm_x100: u32,
    pub interpretation: BpmInterpretation,
    pub start_beat: u32,
    pub length_beats: u32,
    /// Placement offset in song beats. Positive values move the loop later.
    pub offset_beats: i32,
}

impl LoopSettings {
    pub fn source_bpm(&self) -> f64 {
        f64::from(self.source_bpm_x100) / 100.0
    }

    pub fn interpreted_bpm(&self) -> f64 {
        self.interpretation.apply(self.source_bpm())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    pub name: String,
    pub enabled: bool,
    pub channel: u8,
    pub bank_msb: u8,
    pub bank_lsb: u8,
    pub program: u8,
    pub velocity: u8,
    pub percussion: bool,
    pub target: PageTarget,
    /// Reserved for a later small per-page MIDI setup sequence. It is stored
    /// and routed, but deliberately has no editor yet.
    pub setup: Vec<Vec<u8>>,
    pub lanes: Vec<Lane>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PageTarget {
    /// The one software instrument currently owned and monitored by SHR-DAW.
    ActiveInstrument,
    /// An exact ALSA MIDI output port name selected by the user.
    Midi(String),
    /// The configured `external_midi.output` route.
    ConfiguredExternal,
}

impl PageTarget {
    pub fn label(&self) -> &str {
        match self {
            Self::ActiveInstrument => "SHR-DAW instrument",
            Self::Midi(name) => name,
            Self::ConfiguredExternal => "Configured MIDI output",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lane {
    pub name: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pattern {
    pub tempo: u16,
    pub meter: u8,
    pub pages: Vec<Page>,
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Cell {
    pub note: Note,
    pub velocity: Option<u8>,
    pub program: Option<u8>,
    /// Percentage of one row used as this note's gate. `None` inherits the
    /// song gate.
    pub gate: Option<u8>,
    pub command: Command,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Note {
    #[default]
    Empty,
    On(u8),
    Off,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Command {
    #[default]
    None,
    Cut(u8),
    Delay(u8),
    Retrigger(u8),
    Tempo(u16),
}

impl Command {
    /// Stable one-column FT2 marker. A cell has exactly one command.
    pub const fn marker(self) -> char {
        match self {
            Self::None => ' ',
            Self::Cut(_) => 'C',
            Self::Delay(_) => 'D',
            Self::Retrigger(_) => 'R',
            Self::Tempo(_) => 'T',
        }
    }
}

impl Cell {
    pub fn validate(self) -> Result<()> {
        if self.velocity.is_some_and(|value| value > 127)
            || self.program.is_some_and(|value| value > 127)
        {
            bail!("cell MIDI value out of range");
        }
        if self.gate.is_some_and(|gate| !(1..=100).contains(&gate)) {
            bail!("cell gate must be 1..=100 percent");
        }
        if matches!(self.note, Note::On(128..=u8::MAX)) {
            bail!("cell note out of MIDI range");
        }
        match self.command {
            Command::None => {}
            Command::Cut(tick) | Command::Delay(tick) if tick <= 15 => {}
            Command::Retrigger(count) if (1..=8).contains(&count) => {}
            Command::Tempo(bpm) if (20..=300).contains(&bpm) => {}
            Command::Cut(_) | Command::Delay(_) => bail!("command tick must be 0..=15"),
            Command::Retrigger(_) => bail!("retrigger count must be 1..=8"),
            Command::Tempo(_) => bail!("tempo command must be 20..=300 BPM"),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GestureCommit {
    pub notes: Vec<(u8, u8)>,
    pub overflowed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GestureCapture {
    held: BTreeMap<u8, u16>,
    collected: BTreeMap<u8, u8>,
    released_at: Option<Instant>,
    overflowed: bool,
}

impl GestureCapture {
    pub fn observe(&mut self, now: Instant, message: &[u8]) {
        if message.len() < 3 {
            return;
        }
        let kind = message[0] & 0xf0;
        let note = message[1];
        if kind == 0x90 && message[2] > 0 {
            *self.held.entry(note).or_default() += 1;
            if self.collected.len() < LANES_PER_PAGE || self.collected.contains_key(&note) {
                self.collected.entry(note).or_insert(message[2]);
            } else {
                self.overflowed = true;
            }
            self.released_at = None;
        } else if kind == 0x80 || (kind == 0x90 && message[2] == 0) {
            if let Some(count) = self.held.get_mut(&note) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.held.remove(&note);
                }
            }
            if self.held.is_empty() && !self.collected.is_empty() {
                self.released_at = Some(now);
            }
        }
    }

    pub fn finish(&mut self, now: Instant, settle: Duration) -> Option<GestureCommit> {
        let ready = self.held.is_empty()
            && self
                .released_at
                .is_some_and(|released| now.saturating_duration_since(released) >= settle);
        ready.then(|| {
            let commit = GestureCommit {
                notes: std::mem::take(&mut self.collected).into_iter().collect(),
                overflowed: std::mem::take(&mut self.overflowed),
            };
            self.released_at = None;
            commit
        })
    }

    pub fn cancel(&mut self) {
        self.held.clear();
        self.collected.clear();
        self.released_at = None;
        self.overflowed = false;
    }

    pub fn is_active(&self) -> bool {
        !self.collected.is_empty()
    }
}

impl Song {
    pub fn new(config: &ExternalMidiConfig) -> Self {
        let mut patterns = BTreeMap::new();
        patterns.insert(
            0,
            Pattern::new(
                config.default_pattern_rows,
                config.default_tempo,
                4,
                default_pages(config),
            ),
        );
        Self {
            name: "untitled".into(),
            steps_per_beat: config.steps_per_beat,
            gate_percent: config.gate_percent,
            audio_loop: None,
            order: vec![0],
            patterns,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !(1..=16).contains(&self.steps_per_beat) || !(1..=100).contains(&self.gate_percent) {
            bail!("project steps/gate out of range");
        }
        if self.order.is_empty() {
            bail!("project needs an FT2 arrangement");
        }
        if let Some(audio_loop) = &self.audio_loop {
            if audio_loop.file.is_empty()
                || Path::new(&audio_loop.file)
                    .file_name()
                    .and_then(|name| name.to_str())
                    != Some(audio_loop.file.as_str())
                || !(2_000..=30_000).contains(&audio_loop.source_bpm_x100)
                || audio_loop.length_beats == 0
                || !(-16_384..=16_384).contains(&audio_loop.offset_beats)
            {
                bail!("invalid private loop settings");
            }
        }
        if self
            .order
            .iter()
            .any(|number| !self.patterns.contains_key(number))
        {
            bail!("order references a missing pattern");
        }
        for pattern in self.patterns.values() {
            pattern.validate()?;
        }
        Ok(())
    }
}

fn default_pages(config: &ExternalMidiConfig) -> Vec<Page> {
    let melody_channel = config.melody_channel;
    let drum_channel = config.percussion_channel.unwrap_or(1);
    vec![
        Page::new("MELODY", melody_channel, false, 0),
        Page::new(
            "DRUMS",
            drum_channel,
            true,
            config.percussion_program.unwrap_or(0),
        ),
    ]
}

impl Page {
    pub fn new(name: &str, channel: u8, percussion: bool, program: u8) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            channel,
            bank_msb: 0,
            bank_lsb: 0,
            program,
            velocity: 96,
            percussion,
            target: PageTarget::ConfiguredExternal,
            setup: Vec::new(),
            lanes: (1..=LANES_PER_PAGE)
                .map(|lane| Lane {
                    name: format!("L{lane}"),
                    enabled: true,
                })
                .collect(),
        }
    }
}

impl Song {
    #[cfg(test)]
    pub fn add_page(&mut self, target: PageTarget, channel: u8) -> Result<usize> {
        let pattern = self.order.first().copied().context("missing pattern")?;
        self.add_page_to_pattern(pattern, target, channel)
    }

    pub fn add_page_to_pattern(
        &mut self,
        pattern_number: u16,
        target: PageTarget,
        channel: u8,
    ) -> Result<usize> {
        if channel > 15 {
            bail!("MIDI channel out of range");
        }
        let pattern = self
            .patterns
            .get_mut(&pattern_number)
            .context("pattern missing")?;
        let number = pattern.pages.len() + 1;
        let mut page = Page::new(&format!("PAGE {number}"), channel, false, 0);
        page.target = target;
        pattern.pages.push(page);
        for row in &mut pattern.rows {
            row.extend(std::iter::repeat(Cell::default()).take(LANES_PER_PAGE));
        }
        let index = pattern.pages.len() - 1;
        self.validate()?;
        Ok(index)
    }

    #[cfg(test)]
    pub fn total_lanes(&self) -> usize {
        self.order
            .first()
            .and_then(|number| self.patterns.get(number))
            .map_or(0, Pattern::total_lanes)
    }
}

impl Pattern {
    pub fn new(rows: usize, tempo: u16, meter: u8, pages: Vec<Page>) -> Self {
        let tracks = pages.len() * LANES_PER_PAGE;
        Self {
            tempo: tempo.clamp(20, 300),
            meter,
            pages,
            rows: vec![vec![Cell::default(); tracks]; rows],
        }
    }

    pub fn from_config(config: &ExternalMidiConfig, rows: usize, meter: u8) -> Self {
        Self::new(rows, config.default_tempo, meter, default_pages(config))
    }

    pub fn empty_like_setup(rows: usize, setup: &Pattern) -> Self {
        Self::new(rows, setup.tempo, setup.meter, setup.pages.clone())
    }

    #[cfg(test)]
    pub fn empty(rows: usize, tracks: usize) -> Self {
        let pages = (0..tracks.div_ceil(LANES_PER_PAGE))
            .map(|index| Page::new(&format!("PAGE {}", index + 1), 0, false, 0))
            .collect::<Vec<_>>();
        Self::new(rows, 120, 4, pages)
    }

    pub fn total_lanes(&self) -> usize {
        self.pages.len() * LANES_PER_PAGE
    }

    fn validate(&self) -> Result<()> {
        if !(20..=300).contains(&self.tempo) || !matches!(self.meter, 3 | 4) {
            bail!("pattern tempo/meter out of range");
        }
        if self.pages.is_empty() || self.pages.len() > 64 {
            bail!("pattern needs 1..=64 pages");
        }
        if self
            .pages
            .iter()
            .any(|page| page.lanes.len() != LANES_PER_PAGE)
        {
            bail!("each pattern page needs exactly four lanes");
        }
        if self.rows.is_empty() || self.rows.len() > 256 {
            bail!("pattern must have 1..=256 rows");
        }
        if self.rows.iter().any(|row| row.len() != self.total_lanes()) {
            bail!("pattern track count mismatch");
        }
        for cell in self.rows.iter().flatten() {
            cell.validate()?;
        }
        Ok(())
    }
}

pub fn songs_dir() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".local/share")
        })
        .join("shsynth/songs")
}

pub fn safe_name(input: &str) -> String {
    let name = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if name.is_empty() {
        "untitled".into()
    } else {
        name.chars().take(64).collect()
    }
}

pub fn list(base: &Path) -> Vec<String> {
    let mut names = fs::read_dir(base)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .path()
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Versioned line format. Unsupported or newer versions are refused for load
/// and overwrite. Explicit deletion is independent of file contents.
pub fn encode(song: &Song) -> Result<String> {
    song.validate()?;
    let mut out = format!(
        "SHSYNTH-SONG {SONG_VERSION}\nname={}\nsteps={}\ngate={}\norder={}\n",
        escape(&song.name),
        song.steps_per_beat,
        song.gate_percent,
        song.order
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    if let Some(audio_loop) = &song.audio_loop {
        let interpretation = match audio_loop.interpretation {
            BpmInterpretation::Half => "half",
            BpmInterpretation::Normal => "normal",
            BpmInterpretation::Double => "double",
        };
        out.push_str(&format!(
            "loop={}|{}|{}|{}|{}|{}\n",
            escape(&audio_loop.file),
            audio_loop.source_bpm_x100,
            interpretation,
            audio_loop.start_beat,
            audio_loop.length_beats,
            audio_loop.offset_beats
        ));
    }
    for (number, pattern) in &song.patterns {
        out.push_str(&format!(
            "pattern={number}|{}|{}|{}\n",
            pattern.rows.len(),
            pattern.tempo,
            pattern.meter
        ));
        for (page_index, page) in pattern.pages.iter().enumerate() {
            out.push_str(&format!(
                "pattern_page={number}|{page_index}|{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
                escape(&page.name),
                u8::from(page.enabled),
                page.channel + 1,
                page.bank_msb,
                page.bank_lsb,
                page.program,
                page.velocity,
                u8::from(page.percussion),
                target_text(&page.target)
            ));
            for (lane_index, lane) in page.lanes.iter().enumerate() {
                out.push_str(&format!(
                    "pattern_lane={number}|{page_index}|{lane_index}|{}|{}\n",
                    escape(&lane.name),
                    u8::from(lane.enabled)
                ));
            }
            for message in &page.setup {
                out.push_str(&format!(
                    "pattern_setup={number}|{page_index}|{}\n",
                    message
                        .iter()
                        .map(|byte| format!("{byte:02X}"))
                        .collect::<Vec<_>>()
                        .join(":")
                ));
            }
        }
        for (row_index, row) in pattern.rows.iter().enumerate() {
            for (track_index, cell) in row
                .iter()
                .enumerate()
                .filter(|(_, c)| **c != Cell::default())
            {
                out.push_str(&format!(
                    "cell={number}|{row_index}|{track_index}|{}|{}|{}|{}|{}\n",
                    note_text(cell.note),
                    cell.velocity.map_or("-".into(), |v| v.to_string()),
                    cell.program.map_or("-".into(), |v| v.to_string()),
                    cell.gate.map_or("-".into(), |v| v.to_string()),
                    command_text(cell.command)
                ));
            }
        }
    }
    Ok(out)
}

pub fn decode(text: &str) -> Result<Song> {
    let mut lines = text.lines();
    let header = lines.next().context("empty song")?;
    let version = header
        .strip_prefix("SHSYNTH-SONG ")
        .context("not an SHR-DAW song")?
        .parse::<u8>()?;
    if version != SONG_VERSION {
        bail!("unsupported song version {version}; file was not changed");
    }
    let mut name = None;
    let mut steps = None;
    let mut gate = None;
    let mut audio_loop = None;
    let mut order = None;
    let mut patterns: BTreeMap<u16, Pattern> = BTreeMap::new();
    let mut pattern_pages: BTreeMap<u16, BTreeMap<usize, Page>> = BTreeMap::new();
    let mut pattern_lanes = Vec::new();
    let mut pattern_setup = Vec::new();
    let mut cells = Vec::new();
    for line in lines.filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
        let (key, value) = line.split_once('=').context("invalid song line")?;
        match key {
            "name" => name = Some(unescape(value)?),
            "steps" => steps = Some(value.parse()?),
            "gate" => gate = Some(value.parse()?),
            "loop" => {
                let f = value.split('|').collect::<Vec<_>>();
                if f.len() != 6 {
                    bail!("invalid loop settings");
                }
                audio_loop = Some(LoopSettings {
                    file: unescape(f[0])?,
                    source_bpm_x100: f[1].parse()?,
                    interpretation: match f[2] {
                        "half" => BpmInterpretation::Half,
                        "normal" => BpmInterpretation::Normal,
                        "double" => BpmInterpretation::Double,
                        _ => bail!("invalid loop BPM interpretation"),
                    },
                    start_beat: f[3].parse()?,
                    length_beats: f[4].parse()?,
                    offset_beats: f[5].parse()?,
                });
            }
            "order" => {
                order = Some(
                    value
                        .split(',')
                        .map(str::parse)
                        .collect::<std::result::Result<Vec<u16>, _>>()?,
                )
            }
            "pattern" => {
                let f = value.split('|').collect::<Vec<_>>();
                match f.as_slice() {
                    [number, rows, tempo, meter] => {
                        patterns.insert(
                            number.parse()?,
                            Pattern {
                                tempo: tempo.parse()?,
                                meter: meter.parse()?,
                                pages: Vec::new(),
                                rows: vec![Vec::new(); rows.parse()?],
                            },
                        );
                    }
                    _ => bail!("invalid pattern"),
                }
            }
            "pattern_page" => {
                let f = value.split('|').collect::<Vec<_>>();
                if f.len() != 11 {
                    bail!("invalid pattern page");
                }
                pattern_pages
                    .entry(f[0].parse::<u16>()?)
                    .or_default()
                    .insert(
                        f[1].parse::<usize>()?,
                        Page {
                            name: unescape(f[2])?,
                            enabled: f[3] == "1",
                            channel: one_based_channel(f[4])?,
                            bank_msb: midi_value(f[5])?,
                            bank_lsb: midi_value(f[6])?,
                            program: midi_value(f[7])?,
                            velocity: midi_value(f[8])?,
                            percussion: f[9] == "1",
                            target: parse_target(f[10])?,
                            setup: Vec::new(),
                            lanes: Vec::new(),
                        },
                    );
            }
            "pattern_lane" => pattern_lanes.push(value.to_owned()),
            "pattern_setup" => pattern_setup.push(value.to_owned()),
            "cell" => cells.push(value.to_owned()),
            _ => bail!("unknown song field {key}; file was not changed"),
        }
    }
    for (number, pages) in pattern_pages {
        if !pages.keys().copied().eq(0..pages.len()) {
            bail!("pattern pages must be contiguous from zero");
        }
        let pattern = patterns.get_mut(&number).context("pattern page missing")?;
        pattern.pages = pages.into_values().collect();
    }
    attach_pattern_lanes(&mut patterns, pattern_lanes)?;
    attach_pattern_setup(&mut patterns, pattern_setup)?;
    for pattern in patterns.values_mut() {
        let total_lanes = pattern.pages.len() * LANES_PER_PAGE;
        for row in &mut pattern.rows {
            row.resize(total_lanes, Cell::default());
        }
    }
    for value in cells {
        let f = value.split('|').collect::<Vec<_>>();
        if f.len() != 8 {
            bail!("invalid cell");
        }
        let pattern = patterns
            .get_mut(&f[0].parse()?)
            .context("cell pattern missing")?;
        let row_index = f[1].parse::<usize>()?;
        let track_index = f[2].parse::<usize>()?;
        let cell = pattern
            .rows
            .get_mut(row_index)
            .and_then(|r| r.get_mut(track_index))
            .context("cell outside pattern")?;
        *cell = Cell {
            note: parse_note(f[3])?,
            velocity: optional_midi(f[4])?,
            program: optional_midi(f[5])?,
            gate: optional_gate(f[6])?,
            command: parse_command(f[7])?,
        };
    }
    let song = Song {
        name: name.context("missing name")?,
        steps_per_beat: steps.context("missing steps")?,
        gate_percent: gate.context("missing gate")?,
        audio_loop,
        order: order.context("missing order")?,
        patterns,
    };
    song.validate()?;
    Ok(song)
}

fn attach_pattern_lanes(patterns: &mut BTreeMap<u16, Pattern>, lanes: Vec<String>) -> Result<()> {
    for value in lanes {
        let f = value.split('|').collect::<Vec<_>>();
        if f.len() != 5 {
            bail!("invalid pattern lane");
        }
        let pattern = patterns
            .get_mut(&f[0].parse::<u16>()?)
            .context("lane pattern missing")?;
        let page = pattern
            .pages
            .get_mut(f[1].parse::<usize>()?)
            .context("lane page missing")?;
        let index = f[2].parse::<usize>()?;
        if index != page.lanes.len() {
            bail!("lanes must be contiguous");
        }
        page.lanes.push(Lane {
            name: unescape(f[3])?,
            enabled: f[4] == "1",
        });
    }
    Ok(())
}

fn attach_pattern_setup(patterns: &mut BTreeMap<u16, Pattern>, setup: Vec<String>) -> Result<()> {
    for value in setup {
        let f = value.split('|').collect::<Vec<_>>();
        if f.len() != 3 {
            bail!("invalid pattern setup");
        }
        let pattern = patterns
            .get_mut(&f[0].parse::<u16>()?)
            .context("setup pattern missing")?;
        let page = pattern
            .pages
            .get_mut(f[1].parse::<usize>()?)
            .context("setup page missing")?;
        page.setup.push(parse_setup_message(f[2])?);
    }
    Ok(())
}

fn parse_setup_message(bytes: &str) -> Result<Vec<u8>> {
    let message = if bytes.is_empty() {
        Vec::new()
    } else {
        bytes
            .split(':')
            .map(|byte| u8::from_str_radix(byte, 16).context("invalid setup byte"))
            .collect::<Result<Vec<_>>>()?
    };
    if message.is_empty() || message.len() > 256 {
        bail!("setup message must contain 1..=256 bytes");
    }
    Ok(message)
}

pub fn save(base: &Path, song: &Song, overwrite: bool) -> Result<PathBuf> {
    fs::create_dir_all(base)?;
    let path = base.join(format!("{}.shsong", safe_name(&song.name)));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    if overwrite {
        options.create_new(false).create(true).truncate(true);
    }
    if path.exists() && !overwrite {
        bail!("song already exists; confirm overwrite explicitly");
    }
    if path.exists() && overwrite {
        let existing = fs::read_to_string(&path)?;
        let supported = existing
            .lines()
            .next()
            .and_then(|header| header.strip_prefix("SHSYNTH-SONG "))
            .and_then(|version| version.parse::<u8>().ok())
            .is_some_and(|version| version == SONG_VERSION);
        if !supported {
            bail!("refusing to overwrite unsupported/newer song file");
        }
    }
    let tmp = base.join(format!(
        ".{}.{}.tmp",
        safe_name(&song.name),
        std::process::id()
    ));
    if tmp.exists() {
        fs::remove_file(&tmp)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    file.write_all(encode(song)?.as_bytes())?;
    file.sync_all()?;
    if path.exists() && !overwrite {
        let _ = fs::remove_file(&tmp);
        bail!("song already exists");
    }
    if overwrite {
        fs::rename(&tmp, &path)?;
    } else {
        rename_noreplace(&tmp, &path)?;
    }
    Ok(path)
}

fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("publish song without replacement");
    }
    Ok(())
}

pub fn load(base: &Path, name: &str) -> Result<Song> {
    decode(&fs::read_to_string(
        base.join(format!("{}.shsong", safe_name(name))),
    )?)
}

pub fn delete(base: &Path, name: &str) -> Result<()> {
    let path = base.join(format!("{}.shsong", safe_name(name)));
    fs::remove_file(path)?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledMessage {
    pub at: Duration,
    /// Empty for an internal transport-row marker. Row markers advance the
    /// UI and preserve the full pattern duration, but are never transmitted.
    pub bytes: Vec<u8>,
    pub order: usize,
    pub row: usize,
    pub lane: Option<usize>,
    pub target: Option<PageTarget>,
}

pub fn schedule(
    song: &Song,
    config: &ExternalMidiConfig,
    start_order: usize,
    start_row: usize,
) -> Result<Vec<ScheduledMessage>> {
    song.validate()?;
    let mut result = Vec::new();
    let mut at = Duration::ZERO;
    let mut active: BTreeMap<usize, (PageTarget, u8, u8)> = BTreeMap::new();
    for (order_index, pattern_number) in song.order.iter().enumerate().skip(start_order) {
        let pattern = &song.patterns[pattern_number];
        let mut tempo = pattern.tempo;
        let mut programmed = vec![false; pattern.pages.len()];
        for page in pattern.pages.iter().filter(|page| page.enabled) {
            for message in &page.setup {
                push(
                    &mut result,
                    at,
                    order_index,
                    0,
                    message.clone(),
                    Some(page.target.clone()),
                );
            }
        }
        let first_row = if order_index == start_order {
            start_row.min(pattern.rows.len())
        } else {
            0
        };
        for (row_index, row) in pattern.rows.iter().enumerate().skip(first_row) {
            let row_duration =
                Duration::from_secs_f64(60.0 / f64::from(tempo) / f64::from(song.steps_per_beat));
            // A row is part of the transport even when it contains no MIDI.
            // Keep this marker ahead of messages at the same instant so the
            // play cursor moves before that row's notes are sent.
            push(&mut result, at, order_index, row_index, Vec::new(), None);
            if config.send_transport {
                let clocks = (24 / usize::from(song.steps_per_beat)).max(1);
                let targets = pattern
                    .pages
                    .iter()
                    .filter(|page| page.enabled)
                    .map(|page| page.target.clone())
                    .collect::<BTreeSet<_>>();
                for target in targets {
                    for clock in 0..clocks {
                        push(
                            &mut result,
                            at + row_duration.mul_f64(clock as f64 / clocks as f64),
                            order_index,
                            row_index,
                            vec![0xf8],
                            Some(target.clone()),
                        );
                    }
                }
            }
            for (lane_index, cell) in row.iter().enumerate() {
                let page_index = lane_index / LANES_PER_PAGE;
                let page = &pattern.pages[page_index];
                let lane = &page.lanes[lane_index % LANES_PER_PAGE];
                if !page.enabled || !lane.enabled {
                    continue;
                }
                if let Command::Tempo(new_tempo) = cell.command {
                    tempo = new_tempo.clamp(20, 300);
                }
                let delay = match cell.command {
                    Command::Delay(tick) => row_duration.mul_f64(f64::from(tick.min(15)) / 16.0),
                    _ => Duration::ZERO,
                };
                let event_at = at + delay;
                match cell.note {
                    Note::On(note) => {
                        if cell.program.is_some() || !programmed[page_index] {
                            append_program(
                                &mut result,
                                event_at,
                                order_index,
                                row_index,
                                page,
                                cell.program.unwrap_or(page.program),
                                config,
                            );
                            programmed[page_index] = true;
                        }
                        if let Some((old_target, old_channel, old)) = active.remove(&lane_index) {
                            push_lane(
                                &mut result,
                                event_at,
                                order_index,
                                row_index,
                                vec![0x80 | old_channel, old, 0],
                                lane_index,
                                &old_target,
                            );
                        }
                        active.insert(lane_index, (page.target.clone(), page.channel, note));
                        let pulses = match cell.command {
                            Command::Retrigger(count) => count,
                            _ => 1,
                        };
                        let pulse_span = row_duration.div_f64(f64::from(pulses));
                        let remaining = row_duration.saturating_sub(delay);
                        let gate = pulse_span
                            .mul_f64(f64::from(cell.gate.unwrap_or(song.gate_percent)) / 100.0)
                            .min(remaining);
                        for pulse in 0..pulses {
                            let pulse_at = event_at
                                + row_duration.mul_f64(f64::from(pulse) / f64::from(pulses));
                            push_lane(
                                &mut result,
                                pulse_at,
                                order_index,
                                row_index,
                                vec![
                                    0x90 | page.channel,
                                    note,
                                    cell.velocity.unwrap_or(page.velocity),
                                ],
                                lane_index,
                                &page.target,
                            );
                            push_lane(
                                &mut result,
                                (pulse_at + gate).min(at + row_duration),
                                order_index,
                                row_index,
                                vec![0x80 | page.channel, note, 0],
                                lane_index,
                                &page.target,
                            );
                        }
                    }
                    Note::Off => {
                        if let Some((target, channel, note)) = active.remove(&lane_index) {
                            push_lane(
                                &mut result,
                                event_at,
                                order_index,
                                row_index,
                                vec![0x80 | channel, note, 0],
                                lane_index,
                                &target,
                            );
                        }
                    }
                    Note::Empty => {}
                }
                if let Command::Cut(tick) = cell.command {
                    if let Some((target, channel, note)) = active.remove(&lane_index) {
                        push_lane(
                            &mut result,
                            at + row_duration.mul_f64(f64::from(tick.min(15)) / 16.0),
                            order_index,
                            row_index,
                            vec![0x80 | channel, note, 0],
                            lane_index,
                            &target,
                        );
                    }
                }
            }
            at += row_duration;
        }
    }
    release_active_notes(
        &mut result,
        at,
        song.order.len().saturating_sub(1),
        0,
        &mut active,
    );
    // Do not loop as soon as the last note's gate closes: the final rest rows
    // are musically significant. This boundary marker holds the transport to
    // the exact end of the scheduled pattern/order span.
    if let Some((order, pattern_number)) = song.order.iter().enumerate().next_back() {
        let row = song.patterns[pattern_number].rows.len().saturating_sub(1);
        push(&mut result, at, order, row, Vec::new(), None);
    }
    result.sort_by_key(|message| message.at);
    Ok(result)
}

fn release_active_notes(
    out: &mut Vec<ScheduledMessage>,
    at: Duration,
    order: usize,
    row: usize,
    active: &mut BTreeMap<usize, (PageTarget, u8, u8)>,
) {
    for (lane_index, (target, channel, note)) in std::mem::take(active) {
        push_lane(
            out,
            at,
            order,
            row,
            vec![0x80 | channel, note, 0],
            lane_index,
            &target,
        );
    }
}

fn append_program(
    out: &mut Vec<ScheduledMessage>,
    at: Duration,
    order: usize,
    row: usize,
    page: &Page,
    program: u8,
    config: &ExternalMidiConfig,
) {
    match config.bank_select {
        BankSelectMode::Off => {}
        BankSelectMode::Cc0 => push(
            out,
            at,
            order,
            row,
            vec![0xb0 | page.channel, 0, page.bank_msb],
            Some(page.target.clone()),
        ),
        BankSelectMode::Cc0Cc32 => {
            push(
                out,
                at,
                order,
                row,
                vec![0xb0 | page.channel, 0, page.bank_msb],
                Some(page.target.clone()),
            );
            push(
                out,
                at,
                order,
                row,
                vec![0xb0 | page.channel, 32, page.bank_lsb],
                Some(page.target.clone()),
            );
        }
    }
    if config.program_changes {
        push(
            out,
            at,
            order,
            row,
            vec![0xc0 | page.channel, program],
            Some(page.target.clone()),
        );
    }
}
fn push(
    out: &mut Vec<ScheduledMessage>,
    at: Duration,
    order: usize,
    row: usize,
    bytes: Vec<u8>,
    target: Option<PageTarget>,
) {
    out.push(ScheduledMessage {
        at,
        bytes,
        order,
        row,
        lane: None,
        target,
    });
}

fn push_lane(
    out: &mut Vec<ScheduledMessage>,
    at: Duration,
    order: usize,
    row: usize,
    bytes: Vec<u8>,
    lane: usize,
    target: &PageTarget,
) {
    out.push(ScheduledMessage {
        at,
        bytes,
        order,
        row,
        lane: Some(lane),
        target: Some(target.clone()),
    });
}

#[cfg(test)]
fn message_channel(bytes: &[u8]) -> Option<u8> {
    let status = *bytes.first()?;
    (0x80..=0xef).contains(&status).then_some(status & 0x0f)
}

pub fn panic_messages(channels: impl IntoIterator<Item = u8>) -> Vec<Vec<u8>> {
    let channels = channels.into_iter().collect::<BTreeSet<_>>();
    channels
        .into_iter()
        .flat_map(|ch| {
            [
                vec![0xb0 | ch, 64, 0],
                vec![0xb0 | ch, 123, 0],
                vec![0xb0 | ch, 120, 0],
            ]
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct SequencerStatus {
    pub available: bool,
    pub playing: bool,
    pub order: usize,
    pub row: usize,
    pub error: Option<String>,
    pub generation: u64,
    pub targets: BTreeMap<PageTarget, Option<String>>,
}
enum Transport {
    Play(Song, usize, usize),
    Stop,
    Mute(usize, bool),
    Thru(PageTarget, Vec<u8>),
    CancelThru(PageTarget, u8),
    Tempo(u16),
    Shutdown,
}

#[derive(Clone)]
pub struct LiveInput {
    tx: mpsc::Sender<Transport>,
}

impl LiveInput {
    pub fn send(&self, target: &PageTarget, message: &[u8]) {
        let _ = self
            .tx
            .send(Transport::Thru(target.clone(), message.to_vec()));
    }

    pub fn cancel(&self, target: &PageTarget, channel: u8) {
        let _ = self.tx.send(Transport::CancelThru(target.clone(), channel));
    }
}

pub struct Sequencer {
    tx: mpsc::Sender<Transport>,
    status: Arc<Mutex<SequencerStatus>>,
    thread: Option<thread::JoinHandle<()>>,
    config: ExternalMidiConfig,
}
impl Sequencer {
    pub fn start_with_clock(
        config: &ExternalMidiConfig,
        instrument: crate::engine::SharedOutput,
        clock: Arc<crate::loop_player::TransportClock>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(SequencerStatus::default()));
        let thread_status = Arc::clone(&status);
        let cfg = config.clone();
        let handle = thread::Builder::new()
            .name("shsynth-sequencer".into())
            .spawn(move || run_transport(rx, thread_status, cfg, instrument, clock))
            .ok();
        Self {
            tx,
            status,
            thread: handle,
            config: config.clone(),
        }
    }
    pub fn play(&self, song: &Song, order: usize, row: usize) {
        if let Ok(mut status) = self.status.lock() {
            status.playing = true;
            status.order = order;
            status.row = row;
            status.generation = status.generation.wrapping_add(1);
        }
        let _ = self.tx.send(Transport::Play(song.clone(), order, row));
    }
    pub fn live_input(&self) -> LiveInput {
        LiveInput {
            tx: self.tx.clone(),
        }
    }
    pub fn stop(&self) {
        if let Ok(mut status) = self.status.lock() {
            status.playing = false;
        }
        let _ = self.tx.send(Transport::Stop);
    }
    pub fn mute(&self, track: usize, muted: bool) {
        let _ = self.tx.send(Transport::Mute(track, muted));
    }
    pub fn mute_page(&self, page: usize, muted: bool) {
        for lane in 0..LANES_PER_PAGE {
            let _ = self
                .tx
                .send(Transport::Mute(page * LANES_PER_PAGE + lane, muted));
        }
    }
    pub fn tempo(&self, bpm: u16) {
        let _ = self.tx.send(Transport::Tempo(bpm.clamp(20, 300)));
    }
    pub fn thru(&self, message: &[u8]) {
        if self.config.live_thru {
            let _ = self.tx.send(Transport::Thru(
                PageTarget::ConfiguredExternal,
                message.to_vec(),
            ));
        }
    }
    pub fn status(&self) -> SequencerStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
    pub fn unavailable_label(&self) -> String {
        self.status()
            .error
            .unwrap_or_else(|| "tracker target unavailable".into())
    }
}
impl Drop for Sequencer {
    fn drop(&mut self) {
        let _ = self.tx.send(Transport::Shutdown);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn run_transport(
    rx: mpsc::Receiver<Transport>,
    status: Arc<Mutex<SequencerStatus>>,
    config: ExternalMidiConfig,
    instrument: crate::engine::SharedOutput,
    clock: Arc<crate::loop_player::TransportClock>,
) {
    let mut outputs = DestinationPool::new(config.clone(), instrument);
    let mut messages = Vec::new();
    let mut index = 0;
    let mut started = Instant::now();
    let mut muted = BTreeSet::new();
    let mut active_notes: BTreeMap<usize, (PageTarget, u8, BTreeSet<u8>)> = BTreeMap::new();
    let mut note_owners: BTreeMap<(PageTarget, u8, u8), BTreeSet<usize>> = BTreeMap::new();
    let mut thru_notes: BTreeMap<(PageTarget, u8), BTreeSet<u8>> = BTreeMap::new();
    let mut transport_targets = BTreeSet::new();
    let mut transport_tempo = config.default_tempo;
    let mut loop_origin_beat = 0.0;
    loop {
        let timeout = messages
            .get(index)
            .map(|m: &ScheduledMessage| (started + m.at).saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_millis(50))
            .min(Duration::from_millis(50));
        match rx.recv_timeout(timeout) {
            Ok(Transport::Play(song, order, row)) => {
                cleanup_lanes(&mut outputs, &mut active_notes);
                note_owners.clear();
                cleanup_thru(&mut outputs, &mut thru_notes);
                transport_targets = song
                    .patterns
                    .values()
                    .flat_map(|pattern| pattern.pages.iter())
                    .filter(|page| page.enabled)
                    .map(|page| page.target.clone())
                    .collect();
                for target in &transport_targets {
                    outputs.refresh(target);
                }
                update_target_status(&status, &outputs, &transport_targets);
                match schedule(&song, &config, order, row) {
                    Ok(planned) => messages = planned,
                    Err(error) => {
                        messages.clear();
                        if let Ok(mut s) = status.lock() {
                            s.playing = false;
                            s.error = Some(error.to_string());
                        }
                        continue;
                    }
                }
                index = 0;
                started = Instant::now();
                transport_tempo = song
                    .order
                    .get(order)
                    .and_then(|number| song.patterns.get(number))
                    .map_or(config.default_tempo, |pattern| pattern.tempo);
                loop_origin_beat = crate::loop_player::song_position_beats(&song, order, row);
                clock.play(loop_origin_beat, transport_tempo);
                muted.clear();
                active_notes.clear();
                note_owners.clear();
                if config.send_transport {
                    for target in &transport_targets {
                        let _ = outputs.send(target, &[0xfa]);
                    }
                }
                if let Ok(mut s) = status.lock() {
                    s.playing = true;
                    s.order = order;
                    s.row = row;
                }
            }
            Ok(Transport::Stop) => {
                clock.stop();
                messages.clear();
                index = 0;
                cleanup_lanes(&mut outputs, &mut active_notes);
                note_owners.clear();
                cleanup_thru(&mut outputs, &mut thru_notes);
                if config.send_transport {
                    for target in &transport_targets {
                        let _ = outputs.send(target, &[0xfc]);
                    }
                }
                if let Ok(mut s) = status.lock() {
                    s.playing = false;
                }
            }
            Ok(Transport::Mute(lane, value)) => {
                if value {
                    muted.insert(lane);
                    if let Some((target, channel, notes)) = active_notes.remove(&lane) {
                        for note in notes {
                            if release_note_owner(&mut note_owners, lane, &target, channel, note) {
                                let _ = outputs.send(&target, &[0x80 | channel, note, 0]);
                            }
                        }
                    }
                } else {
                    muted.remove(&lane);
                }
            }
            Ok(Transport::Thru(target, message)) => {
                if let Err(error) = outputs.send(&target, &message) {
                    if let Ok(mut s) = status.lock() {
                        s.available = false;
                        s.error = Some(error);
                    }
                } else if let [status, note, velocity, ..] = message.as_slice() {
                    let channel = status & 0x0f;
                    match status & 0xf0 {
                        0x90 if *velocity > 0 => {
                            thru_notes
                                .entry((target.clone(), channel))
                                .or_default()
                                .insert(*note);
                        }
                        0x80 | 0x90 => {
                            if let Some(notes) = thru_notes.get_mut(&(target.clone(), channel)) {
                                notes.remove(note);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Transport::CancelThru(target, channel)) => {
                if let Some(notes) = thru_notes.remove(&(target.clone(), channel)) {
                    for note in notes {
                        let _ = outputs.send(&target, &[0x80 | channel, note, 0]);
                    }
                }
            }
            Ok(Transport::Tempo(bpm)) => {
                let elapsed = started.elapsed();
                rescale_schedule(&mut messages, index, elapsed, transport_tempo, bpm);
                transport_tempo = bpm;
                clock.tempo(f64::from(bpm));
            }
            Ok(Transport::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                clock.stop();
                cleanup_lanes(&mut outputs, &mut active_notes);
                note_owners.clear();
                cleanup_thru(&mut outputs, &mut thru_notes);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        while let Some(message) = messages
            .get(index)
            .filter(|m| started + m.at <= Instant::now())
        {
            if message.bytes.is_empty() {
                if let Some(next) = messages[index + 1..]
                    .iter()
                    .find(|candidate| candidate.bytes.is_empty() && candidate.at > message.at)
                {
                    let seconds = (next.at - message.at).as_secs_f64();
                    if seconds > 0.0 {
                        clock.tempo(60.0 / seconds / f64::from(config.steps_per_beat));
                    }
                }
            }
            let muted_message = message.lane.is_some_and(|lane| muted.contains(&lane));
            let mut shared_note_off = false;
            if !muted_message {
                if let (Some(lane), Some(target), [midi_status, note, ..]) = (
                    message.lane,
                    message.target.as_ref(),
                    message.bytes.as_slice(),
                ) {
                    let channel = midi_status & 0x0f;
                    match midi_status & 0xf0 {
                        0x90 if message.bytes.get(2).copied().unwrap_or(0) > 0 => {
                            note_owners
                                .entry((target.clone(), channel, *note))
                                .or_default()
                                .insert(lane);
                        }
                        0x80 | 0x90 => {
                            shared_note_off =
                                !release_note_owner(&mut note_owners, lane, target, channel, *note);
                        }
                        _ => {}
                    }
                }
            }
            let send_error = if message.bytes.is_empty() || muted_message || shared_note_off {
                None
            } else {
                message
                    .target
                    .as_ref()
                    .and_then(|target| outputs.send(target, &message.bytes).err())
            };
            if !muted_message {
                if let (Some(lane), Some(target), [status, note, ..]) = (
                    message.lane,
                    message.target.as_ref(),
                    message.bytes.as_slice(),
                ) {
                    let channel = status & 0x0f;
                    match status & 0xf0 {
                        0x90 if message.bytes.get(2).copied().unwrap_or(0) > 0 => {
                            active_notes
                                .entry(lane)
                                .or_insert_with(|| (target.clone(), channel, BTreeSet::new()))
                                .2
                                .insert(*note);
                        }
                        0x80 | 0x90 => {
                            if let Some((_, _, notes)) = active_notes.get_mut(&lane) {
                                notes.remove(note);
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let Some(error) = send_error {
                if let Ok(mut s) = status.lock() {
                    s.available = false;
                    if let Some(target) = &message.target {
                        s.targets.insert(target.clone(), Some(error.clone()));
                    }
                    s.error = Some(error);
                }
            }
            if let Ok(mut s) = status.lock() {
                s.order = message.order;
                s.row = message.row;
            }
            index += 1;
        }
        if !messages.is_empty() && index == messages.len() {
            cleanup_lanes(&mut outputs, &mut active_notes);
            note_owners.clear();
            index = 0;
            started = Instant::now();
            clock.play(loop_origin_beat, transport_tempo);
        }
    }
}

struct DestinationPool {
    config: ExternalMidiConfig,
    instrument: crate::engine::SharedOutput,
    hardware: BTreeMap<PageTarget, std::result::Result<MidiOutputConnection, String>>,
}

impl DestinationPool {
    fn new(config: ExternalMidiConfig, instrument: crate::engine::SharedOutput) -> Self {
        Self {
            config,
            instrument,
            hardware: BTreeMap::new(),
        }
    }

    fn ensure(&mut self, target: &PageTarget) {
        if matches!(target, PageTarget::ActiveInstrument) || self.hardware.contains_key(target) {
            return;
        }
        let connection = connect_target(&self.config, target).map_err(|error| error.to_string());
        self.hardware.insert(target.clone(), connection);
    }

    fn refresh(&mut self, target: &PageTarget) {
        if target != &PageTarget::ActiveInstrument {
            self.hardware.remove(target);
        }
        self.ensure(target);
    }

    fn send(&mut self, target: &PageTarget, bytes: &[u8]) -> std::result::Result<(), String> {
        if target == &PageTarget::ActiveInstrument {
            return self
                .instrument
                .lock()
                .map_err(|_| "active instrument route lock failed".to_string())?
                .as_mut()
                .ok_or_else(|| "active SHR-DAW instrument is offline".to_string())?
                .send(bytes)
                .map_err(|error| error.to_string());
        }
        self.ensure(target);
        let output = self.hardware.get_mut(target).expect("target was ensured");
        let result = match output {
            Ok(output) => output.send(bytes).map_err(|error| error.to_string()),
            Err(error) => return Err(error.clone()),
        };
        if let Err(error) = &result {
            *output = Err(error.clone());
        }
        result
    }

    fn error(&self, target: &PageTarget) -> Option<String> {
        if target == &PageTarget::ActiveInstrument {
            return self
                .instrument
                .lock()
                .ok()
                .and_then(|output| output.is_none().then(|| "instrument offline".into()));
        }
        self.hardware
            .get(target)
            .and_then(|output| output.as_ref().err().cloned())
    }
}

fn update_target_status(
    status: &Arc<Mutex<SequencerStatus>>,
    outputs: &DestinationPool,
    targets: &BTreeSet<PageTarget>,
) {
    if let Ok(mut status) = status.lock() {
        status.targets = targets
            .iter()
            .map(|target| (target.clone(), outputs.error(target)))
            .collect();
        status.available = status.targets.values().any(Option::is_none);
        status.error = status.targets.iter().find_map(|(target, error)| {
            error
                .as_ref()
                .map(|error| format!("{}: {error}", target.label()))
        });
    }
}

fn cleanup_lanes(
    outputs: &mut DestinationPool,
    active: &mut BTreeMap<usize, (PageTarget, u8, BTreeSet<u8>)>,
) {
    for (target, message) in planned_lane_cleanup(&std::mem::take(active)) {
        let _ = outputs.send(&target, &message);
    }
}

fn planned_lane_cleanup(
    active: &BTreeMap<usize, (PageTarget, u8, BTreeSet<u8>)>,
) -> Vec<(PageTarget, Vec<u8>)> {
    active
        .values()
        .flat_map(|(target, channel, notes)| {
            notes
                .iter()
                .map(move |note| (target.clone(), vec![0x80 | channel, *note, 0]))
        })
        .collect()
}

fn release_note_owner(
    owners: &mut BTreeMap<(PageTarget, u8, u8), BTreeSet<usize>>,
    lane: usize,
    target: &PageTarget,
    channel: u8,
    note: u8,
) -> bool {
    let key = (target.clone(), channel, note);
    let last = if let Some(lanes) = owners.get_mut(&key) {
        lanes.remove(&lane);
        lanes.is_empty()
    } else {
        true
    };
    if last {
        owners.remove(&key);
    }
    last
}

fn cleanup_thru(
    outputs: &mut DestinationPool,
    active: &mut BTreeMap<(PageTarget, u8), BTreeSet<u8>>,
) {
    for ((target, channel), notes) in std::mem::take(active) {
        for note in notes {
            let _ = outputs.send(&target, &[0x80 | channel, note, 0]);
        }
    }
}

fn rescale_schedule(
    messages: &mut [ScheduledMessage],
    index: usize,
    elapsed: Duration,
    old_tempo: u16,
    new_tempo: u16,
) {
    let scale = f64::from(old_tempo) / f64::from(new_tempo);
    for message in messages.iter_mut().skip(index) {
        let remaining = message.at.saturating_sub(elapsed);
        message.at = elapsed + remaining.mul_f64(scale);
    }
}
fn connect_target(
    config: &ExternalMidiConfig,
    target: &PageTarget,
) -> Result<MidiOutputConnection> {
    let wanted = match target {
        PageTarget::ConfiguredExternal => {
            if !config.enabled {
                bail!("configured MIDI output is disabled");
            }
            &config.output_match
        }
        PageTarget::Midi(name) => name,
        PageTarget::ActiveInstrument => bail!("active instrument uses the monitored route"),
    };
    let output = MidiOutput::new(&config.client_name)?;
    let port = output
        .ports()
        .into_iter()
        .find(|p| {
            output
                .port_name(p)
                .map(|n| {
                    n == *wanted
                        || (matches!(target, PageTarget::ConfiguredExternal)
                            && n.to_lowercase().contains(&wanted.to_lowercase()))
                })
                .unwrap_or(false)
        })
        .with_context(|| format!("MIDI output {wanted:?} is offline"))?;
    output
        .connect(&port, "SHR-DAW tracker page")
        .map_err(|e| anyhow!(e.to_string()))
}

pub fn available_midi_outputs(client_name: &str) -> Result<Vec<String>> {
    let output = MidiOutput::new(client_name)?;
    let mut names = output
        .ports()
        .iter()
        .filter_map(|port| output.port_name(port).ok())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(names)
}

pub fn diagnostic(config: &ExternalMidiConfig) -> Result<String> {
    let output = MidiOutput::new(&config.client_name)?;
    let ports = output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect::<Vec<_>>();
    let matches = ports
        .iter()
        .filter(|name| {
            name.to_lowercase()
                .contains(&config.output_match.to_lowercase())
        })
        .cloned()
        .collect::<Vec<_>>();
    let page = Page {
        name: "dry-run".into(),
        enabled: true,
        channel: config.channels[0],
        bank_msb: 0,
        bank_lsb: 0,
        program: 0,
        velocity: 64,
        percussion: false,
        target: PageTarget::ConfiguredExternal,
        setup: Vec::new(),
        lanes: (1..=LANES_PER_PAGE)
            .map(|lane| Lane {
                name: format!("L{lane}"),
                enabled: true,
            })
            .collect(),
    };
    let mut dry = Vec::new();
    append_program(&mut dry, Duration::ZERO, 0, 0, &page, 0, config);
    push(
        &mut dry,
        Duration::ZERO,
        0,
        0,
        vec![0x90 | page.channel, 60, 64],
        Some(page.target.clone()),
    );
    push(
        &mut dry,
        Duration::from_millis(250),
        0,
        0,
        vec![0x80 | page.channel, 60, 0],
        Some(page.target.clone()),
    );
    if let Some(channel) = config.percussion_channel {
        if config.program_changes {
            if let Some(program) = config.percussion_program {
                push(
                    &mut dry,
                    Duration::ZERO,
                    0,
                    0,
                    vec![0xc0 | channel, program],
                    Some(page.target.clone()),
                );
            }
        }
        push(
            &mut dry,
            Duration::ZERO,
            0,
            0,
            vec![0x90 | channel, 36, 96],
            Some(page.target.clone()),
        );
        push(
            &mut dry,
            Duration::from_millis(125),
            0,
            0,
            vec![0x80 | channel, 36, 0],
            Some(page.target.clone()),
        );
    }
    let messages = dry
        .iter()
        .map(|m| format!("{:?} @ {}ms", m.bytes, m.at.as_millis()))
        .chain(
            panic_messages(config.channels.iter().copied())
                .iter()
                .map(|m| format!("{m:?} panic")),
        )
        .collect::<Vec<_>>()
        .join("\n  ");
    Ok(format!("profile: {}\nenabled: {}\nconfigured match: {:?}\nmatching ports: {}\navailable MIDI outputs:\n  {}\nchannels: {}\npercussion: {}; percussion program: {}; input map: {} -> [{}]\nbank: {:?}; program: {}; clock/start/stop: {}; live thru: {}\ndry run (NOT transmitted):\n  {}\n",
        config.profile, config.enabled, config.output_match, if matches.is_empty() { "none".into() } else { matches.join(", ") }, if ports.is_empty() { "none".into() } else { ports.join("\n  ") },
        config.channels.iter().map(|c| (c+1).to_string()).collect::<Vec<_>>().join(","), config.percussion_channel.map(|c| (c+1).to_string()).unwrap_or_else(|| "off".into()), config.percussion_program.map(|p| p.to_string()).unwrap_or_else(|| "unchanged".into()), config.percussion_input_base, config.percussion_notes.iter().map(u8::to_string).collect::<Vec<_>>().join(","), config.bank_select, config.program_changes, config.send_transport, config.live_thru, messages))
}

fn escape(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('|', "%7C")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}
fn target_text(target: &PageTarget) -> String {
    match target {
        PageTarget::ActiveInstrument => "instrument".into(),
        PageTarget::ConfiguredExternal => "configured".into(),
        PageTarget::Midi(name) => format!("midi:{}", escape(name)),
    }
}
fn parse_target(value: &str) -> Result<PageTarget> {
    match value {
        "instrument" => Ok(PageTarget::ActiveInstrument),
        "configured" => Ok(PageTarget::ConfiguredExternal),
        _ => value
            .strip_prefix("midi:")
            .map(unescape)
            .transpose()?
            .map(PageTarget::Midi)
            .context("invalid page target"),
    }
}
fn unescape(value: &str) -> Result<String> {
    Ok(value
        .replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%7C", "|")
        .replace("%25", "%"))
}
fn one_based_channel(v: &str) -> Result<u8> {
    let n = v.parse::<u8>()?;
    if !(1..=16).contains(&n) {
        bail!("channel out of range");
    }
    Ok(n - 1)
}
fn midi_value(v: &str) -> Result<u8> {
    let n = v.parse::<u8>()?;
    if n > 127 {
        bail!("MIDI value out of range");
    }
    Ok(n)
}
fn optional_midi(v: &str) -> Result<Option<u8>> {
    if v == "-" {
        Ok(None)
    } else {
        midi_value(v).map(Some)
    }
}
fn optional_gate(v: &str) -> Result<Option<u8>> {
    if v == "-" {
        return Ok(None);
    }
    let gate = v.parse::<u8>()?;
    if !(1..=100).contains(&gate) {
        bail!("cell gate must be 1..=100 percent");
    }
    Ok(Some(gate))
}
fn note_text(n: Note) -> String {
    match n {
        Note::Empty => "---".into(),
        Note::Off => "OFF".into(),
        Note::On(n) => n.to_string(),
    }
}
fn parse_note(v: &str) -> Result<Note> {
    match v {
        "---" => Ok(Note::Empty),
        "OFF" => Ok(Note::Off),
        _ => midi_value(v).map(Note::On),
    }
}
fn command_text(c: Command) -> String {
    match c {
        Command::None => "-".into(),
        Command::Cut(v) => format!("C{v}"),
        Command::Delay(v) => format!("D{v}"),
        Command::Retrigger(v) => format!("R{v}"),
        Command::Tempo(v) => format!("T{v}"),
    }
}
fn parse_command(v: &str) -> Result<Command> {
    if v == "-" {
        return Ok(Command::None);
    }
    let (kind, parameter) = v.split_at(v.char_indices().nth(1).map_or(v.len(), |(i, _)| i));
    if parameter.is_empty() {
        bail!("command parameter missing");
    }
    match kind {
        "C" => Ok(Command::Cut(parameter.parse()?)),
        "D" => Ok(Command::Delay(parameter.parse()?)),
        "R" => Ok(Command::Retrigger(parameter.parse()?)),
        "T" => Ok(Command::Tempo(parameter.parse()?)),
        _ => bail!("unknown command"),
    }
}

pub fn note_name(note: Note) -> String {
    match note {
        Note::Empty => "---".into(),
        Note::Off => "OFF".into(),
        Note::On(n) => {
            const N: [&str; 12] = [
                "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
            ];
            format!("{}{}", N[usize::from(n % 12)], i16::from(n) / 12 - 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> ExternalMidiConfig {
        let mut c = crate::config::RuntimeConfig::default().external_midi;
        c.program_changes = true;
        c.bank_select = BankSelectMode::Cc0Cc32;
        c
    }
    fn pages(song: &Song) -> &[Page] {
        &song.patterns[&0].pages
    }
    fn pages_mut(song: &mut Song) -> &mut [Page] {
        &mut song.patterns.get_mut(&0).unwrap().pages
    }
    #[test]
    fn serialization_round_trip_requires_current_schema() {
        let mut s = Song::new(&config());
        s.name = "a|b".into();
        s.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        let text = encode(&s).unwrap();
        assert_eq!(decode(&text).unwrap(), s);
        assert!(decode(&text.replace("gate=80\n", "")).is_err());
    }
    #[test]
    fn current_format_loop_round_trips_and_old_shapes_are_rejected() {
        let mut with_loop = Song::new(&config());
        with_loop.patterns.get_mut(&0).unwrap().meter = 3;
        with_loop.audio_loop = Some(LoopSettings {
            file: "D-sharp-minor.wav".into(),
            source_bpm_x100: 12_000,
            interpretation: BpmInterpretation::Half,
            start_beat: 3,
            length_beats: 12,
            offset_beats: -4,
        });
        assert_eq!(decode(&encode(&with_loop).unwrap()).unwrap(), with_loop);

        let missing_offset = encode(&with_loop).unwrap().replace("|3|12|-4\n", "|3|12\n");
        assert!(decode(&missing_offset).is_err());

        let old_shared_pages = encode(&with_loop)
            .unwrap()
            .replace("pattern=0|64|120|3\n", "tempo=120\nmeter=3\npattern=0|64\n")
            .replace("pattern_page=0|", "page=")
            .replace("pattern_lane=0|", "lane=");
        assert!(decode(&old_shared_pages).is_err());
    }
    #[test]
    fn current_song_format_round_trips_every_cell_field() {
        let mut song = Song::new(&config());
        song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(64),
            velocity: Some(111),
            program: Some(17),
            gate: Some(37),
            command: Command::Delay(6),
        };
        let encoded = encode(&song).unwrap();
        assert!(encoded.starts_with("SHSYNTH-SONG 0\n"));
        assert!(encoded.contains("|64|111|17|37|D6\n"));
        assert_eq!(decode(&encoded).unwrap(), song);
    }
    #[test]
    fn cell_gate_and_delay_end_within_the_row() {
        let c = config();
        let mut song = Song::new(&c);
        song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            gate: Some(40),
            command: Command::Delay(8),
            ..Cell::default()
        };
        let messages = schedule(&song, &c, 0, 0).unwrap();
        let note_on = messages.iter().find(|m| m.bytes == [0x90, 60, 96]).unwrap();
        let note_off = messages
            .iter()
            .find(|m| m.bytes == [0x80, 60, 0] && m.at > note_on.at)
            .unwrap();
        assert_eq!(note_on.at, Duration::from_micros(62_500));
        assert_eq!(note_off.at, Duration::from_micros(112_500));
        assert!(note_off.at <= Duration::from_millis(125));
    }
    #[test]
    fn every_command_schedules_deterministically_through_order_boundaries() {
        let c = config();
        let mut song = Song::new(&c);
        song.patterns
            .insert(0, Pattern::empty(4, song.total_lanes()));
        song.patterns
            .insert(1, Pattern::empty(1, song.total_lanes()));
        song.order = vec![0, 1];
        song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            command: Command::Cut(4),
            ..Cell::default()
        };
        song.patterns.get_mut(&0).unwrap().rows[1][0] = Cell {
            note: Note::On(61),
            command: Command::Delay(8),
            ..Cell::default()
        };
        song.patterns.get_mut(&0).unwrap().rows[2][0] = Cell {
            note: Note::On(62),
            command: Command::Retrigger(4),
            ..Cell::default()
        };
        song.patterns.get_mut(&0).unwrap().rows[3][0].command = Command::Tempo(60);
        song.patterns.get_mut(&1).unwrap().rows[0][0].note = Note::On(63);
        let messages = schedule(&song, &c, 0, 0).unwrap();
        assert!(messages
            .iter()
            .any(|m| m.bytes == [0x80, 60, 0] && m.at == Duration::from_micros(31_250)));
        assert!(messages
            .iter()
            .any(|m| m.bytes == [0x90, 61, 96] && m.at == Duration::from_micros(187_500)));
        let retriggers = messages
            .iter()
            .filter(|m| m.bytes == [0x90, 62, 96])
            .map(|m| m.at)
            .collect::<Vec<_>>();
        assert_eq!(
            retriggers,
            [
                Duration::from_millis(250),
                Duration::from_micros(281_250),
                Duration::from_micros(312_500),
                Duration::from_micros(343_750),
            ]
        );
        let boundary_note = messages.iter().find(|m| m.bytes == [0x90, 63, 96]).unwrap();
        assert_eq!(
            (boundary_note.order, boundary_note.at),
            (1, Duration::from_millis(500))
        );
    }
    #[test]
    fn invalid_cell_ranges_are_rejected_without_clamping_files() {
        let mut song = Song::new(&config());
        song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            gate: Some(0),
            ..Cell::default()
        };
        assert!(song.validate().unwrap_err().to_string().contains("gate"));
        song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            command: Command::Retrigger(9),
            ..Cell::default()
        };
        assert!(song
            .validate()
            .unwrap_err()
            .to_string()
            .contains("retrigger"));
    }
    #[test]
    fn effect_markers_are_stable_and_unambiguous() {
        assert_eq!(Command::None.marker(), ' ');
        assert_eq!(Command::Cut(0).marker(), 'C');
        assert_eq!(Command::Delay(0).marker(), 'D');
        assert_eq!(Command::Retrigger(2).marker(), 'R');
        assert_eq!(Command::Tempo(120).marker(), 'T');
    }
    #[test]
    fn atomic_save_refuses_overwrite() {
        let base = env::temp_dir().join(format!("shsong-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let s = Song::new(&config());
        save(&base, &s, false).unwrap();
        assert!(save(&base, &s, false).is_err());
        assert!(save(&base, &s, true).is_ok());
        assert!(!base.join(".untitled.tmp").exists());
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn bank_and_program_precede_note_and_notes_end() {
        let c = config();
        let mut s = Song::new(&c);
        let cell = &mut s.patterns.get_mut(&0).unwrap().rows[0][0];
        cell.program = Some(7);
        cell.note = Note::On(60);
        let scheduled = schedule(&s, &c, 0, 0).unwrap();
        let m = scheduled
            .iter()
            .filter(|message| !message.bytes.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(&m[0].bytes[..2], &[0xb0, 0]);
        assert_eq!(&m[1].bytes[..2], &[0xb0, 32]);
        assert_eq!(m[2].bytes[0] & 0xf0, 0xc0);
        assert_eq!(m[3].bytes[0] & 0xf0, 0x90);
        assert!(m.iter().any(|x| x.bytes[0] & 0xf0 == 0x80));
    }
    #[test]
    fn row_timing_pattern_transition_and_tempo() {
        let c = config();
        let mut s = Song::new(&c);
        s.patterns.insert(1, Pattern::empty(64, s.total_lanes()));
        s.order.push(1);
        s.patterns.get_mut(&0).unwrap().rows[1][0] = Cell {
            note: Note::On(61),
            command: Command::Tempo(60),
            ..Cell::default()
        };
        s.patterns.get_mut(&1).unwrap().rows[0][0].note = Note::On(62);
        let m = schedule(&s, &c, 0, 0).unwrap();
        let notes = m
            .iter()
            .filter(|x| x.bytes.first().is_some_and(|status| status & 0xf0 == 0x90))
            .collect::<Vec<_>>();
        assert_eq!(notes[0].at, Duration::from_millis(125));
        assert_eq!(notes[1].order, 1);
    }
    #[test]
    fn pattern_master_tempo_resets_at_arrangement_step() {
        let c = config();
        let mut song = Song::new(&c);
        let setup = song.patterns[&0].clone();
        song.patterns
            .insert(0, Pattern::empty_like_setup(2, &setup));
        song.patterns.get_mut(&0).unwrap().tempo = 120;
        song.patterns.get_mut(&0).unwrap().rows[0][0].command = Command::Tempo(60);
        let mut second = Pattern::empty_like_setup(2, &song.patterns[&0]);
        second.tempo = 240;
        second.rows[1][0].note = Note::On(62);
        song.patterns.insert(1, second);
        song.order = vec![0, 1];
        let messages = schedule(&song, &c, 0, 0).unwrap();
        let second_note = messages
            .iter()
            .find(|message| message.order == 1 && message.bytes.first() == Some(&0x90))
            .unwrap();
        assert_eq!(second_note.at, Duration::from_micros(437_500));
    }
    #[test]
    fn arrangement_steps_use_referenced_pattern_page_setup() {
        let mut c = config();
        c.bank_select = BankSelectMode::Off;
        let mut song = Song::new(&c);
        pages_mut(&mut song)[0].target = PageTarget::Midi("A".into());
        pages_mut(&mut song)[0].channel = 0;
        song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        let mut second = Pattern::empty_like_setup(1, &song.patterns[&0]);
        second.pages[0].target = PageTarget::Midi("B".into());
        second.pages[0].channel = 5;
        second.rows[0][0].note = Note::On(61);
        song.patterns.insert(1, second);
        song.order = vec![0, 1];
        let notes = schedule(&song, &c, 0, 0)
            .unwrap()
            .into_iter()
            .filter(|message| {
                message
                    .bytes
                    .first()
                    .is_some_and(|status| status & 0xf0 == 0x90)
            })
            .collect::<Vec<_>>();
        assert!(notes.iter().any(
            |message| message.target == Some(PageTarget::Midi("A".into()))
                && message.bytes == [0x90, 60, 96]
        ));
        assert!(notes.iter().any(
            |message| message.target == Some(PageTarget::Midi("B".into()))
                && message.bytes == [0x95, 61, 96]
        ));
    }
    #[test]
    fn arrangement_boundary_does_not_add_an_extra_note_off() {
        let c = config();
        let mut song = Song::new(&c);
        song.patterns.get_mut(&0).unwrap().rows.truncate(1);
        song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            gate: Some(1),
            ..Cell::default()
        };
        let mut second = Pattern::empty_like_setup(1, &song.patterns[&0]);
        second.rows[0][1].note = Note::On(64);
        song.patterns.insert(1, second);
        song.order = vec![0, 1];
        let messages = schedule(&song, &c, 0, 0).unwrap();
        let boundary = messages
            .iter()
            .find(|message| message.order == 1 && message.row == 0 && message.bytes.is_empty())
            .unwrap()
            .at;
        assert!(!messages
            .iter()
            .any(|message| message.at == boundary && message.bytes == [0x80, 60, 0]));
    }
    #[test]
    fn live_tempo_change_rescales_remaining_schedule_monotonically() {
        let c = config();
        let mut song = Song::new(&c);
        song.patterns
            .insert(0, Pattern::empty(4, song.total_lanes()));
        let mut messages = schedule(&song, &c, 0, 0).unwrap();
        rescale_schedule(&mut messages, 1, Duration::from_millis(100), 120, 60);
        let times = messages
            .iter()
            .skip(1)
            .map(|message| message.at)
            .collect::<Vec<_>>();
        assert!(times.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(times.contains(&Duration::from_millis(150)));
        assert_eq!(times.last(), Some(&Duration::from_millis(900)));
    }
    #[test]
    fn panic_covers_every_channel_with_sound_off() {
        let c = config();
        let p = panic_messages(c.channels.iter().copied());
        for ch in c.channels {
            assert!(p.contains(&vec![0xb0 | ch, 120, 0]));
            assert!(p.contains(&vec![0xb0 | ch, 123, 0]));
        }
    }
    #[test]
    fn installed_profile_has_four_lane_drum_page_on_channel_two() {
        let c = config();
        let mut song = Song::new(&c);
        assert_eq!(pages(&song)[1].channel, 1);
        assert!(pages(&song)[1].percussion);
        song.patterns.get_mut(&0).unwrap().rows[0][4].note = Note::On(36);
        assert!(schedule(&song, &c, 0, 0).unwrap().iter().any(|message| {
            message.bytes.first() == Some(&0x91) && message.bytes.get(1) == Some(&36)
        }));
    }
    #[test]
    fn mt240_profile_uses_channel_two_and_selects_percussion_first() {
        let mut c = config();
        c.channels = vec![0, 1];
        c.melody_channel = 0;
        c.percussion_channel = Some(1);
        c.percussion_program = Some(9);
        c.max_tracks = 2;
        c.bank_select = BankSelectMode::Off;
        let mut song = Song::new(&c);
        assert_eq!(pages(&song)[1].channel, 1);
        assert_eq!(pages(&song)[1].program, 9);
        assert!(pages(&song)[1].percussion);
        song.patterns.get_mut(&0).unwrap().rows[0][4].note = Note::On(36);
        let midi = schedule(&song, &c, 0, 0)
            .unwrap()
            .into_iter()
            .filter(|message| !message.bytes.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(midi[0].bytes, [0xc1, 9]);
        assert_eq!(midi[1].bytes, [0x91, 36, 96]);
    }
    #[test]
    fn disabled_track_never_schedules_notes() {
        let c = config();
        let mut s = Song::new(&c);
        pages_mut(&mut s)[0].lanes[0].enabled = false;
        s.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        assert!(schedule(&s, &c, 0, 0)
            .unwrap()
            .iter()
            .all(|message| message.bytes.is_empty()));
    }
    #[test]
    fn empty_rows_advance_at_row_timing_and_hold_the_loop_boundary() {
        let c = config();
        let mut s = Song::new(&c);
        s.patterns.insert(0, Pattern::empty(4, s.total_lanes()));
        let m = schedule(&s, &c, 0, 0).unwrap();
        let ticks = m
            .iter()
            .filter(|message| message.bytes.is_empty())
            .map(|message| (message.at, message.row))
            .collect::<Vec<_>>();
        assert_eq!(
            ticks,
            vec![
                (Duration::ZERO, 0),
                (Duration::from_millis(125), 1),
                (Duration::from_millis(250), 2),
                (Duration::from_millis(375), 3),
                (Duration::from_millis(500), 3),
            ]
        );
        assert_eq!(m.last().unwrap().at, Duration::from_millis(500));
    }
    #[test]
    fn system_realtime_messages_do_not_have_a_mute_channel() {
        assert_eq!(message_channel(&[]), None);
        assert_eq!(message_channel(&[0xf8]), None);
        assert_eq!(message_channel(&[0x99, 36, 100]), Some(9));
    }
    #[test]
    fn both_four_lane_pages_schedule_together_on_shared_page_channels() {
        let mut c = config();
        c.bank_select = BankSelectMode::Off;
        let mut song = Song::new(&c);
        let row = &mut song.patterns.get_mut(&0).unwrap().rows[0];
        for (lane, note) in [60, 64, 67, 71].into_iter().enumerate() {
            row[lane] = Cell {
                note: Note::On(note),
                velocity: Some(80 + lane as u8),
                ..Cell::default()
            };
        }
        for (lane, note) in [36, 38, 40, 41].into_iter().enumerate() {
            row[LANES_PER_PAGE + lane] = Cell {
                note: Note::On(note),
                velocity: Some(100 + lane as u8),
                ..Cell::default()
            };
        }
        let messages = schedule(&song, &c, 0, 0).unwrap();
        let note_ons = messages
            .iter()
            .filter(|message| {
                message
                    .bytes
                    .first()
                    .is_some_and(|status| status & 0xf0 == 0x90)
            })
            .collect::<Vec<_>>();
        assert_eq!(note_ons.iter().filter(|m| m.bytes[0] == 0x90).count(), 4);
        assert_eq!(note_ons.iter().filter(|m| m.bytes[0] == 0x91).count(), 4);
        assert!(note_ons.iter().all(|message| message.at == Duration::ZERO));
        assert_eq!(
            note_ons.iter().map(|m| m.bytes[2]).collect::<Vec<_>>(),
            [80, 81, 82, 83, 100, 101, 102, 103]
        );
        let program = messages.iter().position(|m| m.bytes == [0xc1, 9]).unwrap();
        let first_drum = messages
            .iter()
            .position(|m| m.bytes.first() == Some(&0x91))
            .unwrap();
        assert!(program < first_drum);
    }

    #[test]
    fn shared_channel_lanes_keep_independent_note_off_identity() {
        let c = config();
        let mut song = Song::new(&c);
        let row = &mut song.patterns.get_mut(&0).unwrap().rows[0];
        row[0].note = Note::On(60);
        row[1].note = Note::On(64);
        let messages = schedule(&song, &c, 0, 0).unwrap();
        assert!(messages
            .iter()
            .any(|m| m.lane == Some(0) && m.bytes == [0x80, 60, 0]));
        assert!(messages
            .iter()
            .any(|m| m.lane == Some(1) && m.bytes == [0x80, 64, 0]));
        assert!(!messages
            .iter()
            .any(|m| m.lane == Some(0) && m.bytes == [0x80, 64, 0]));
    }

    #[test]
    fn gesture_waits_sorts_preserves_velocity_and_accepts_staggered_notes() {
        let start = Instant::now();
        let mut gesture = GestureCapture::default();
        gesture.observe(start, &[0x90, 67, 91]);
        gesture.observe(start + Duration::from_millis(5), &[0x80, 67, 0]);
        assert_eq!(
            gesture.finish(start + Duration::from_millis(30), DEFAULT_GESTURE_SETTLE),
            None
        );
        gesture.observe(start + Duration::from_millis(35), &[0x90, 60, 73]);
        gesture.observe(start + Duration::from_millis(40), &[0x90, 64, 82]);
        gesture.observe(start + Duration::from_millis(45), &[0x90, 60, 0]);
        gesture.observe(start + Duration::from_millis(50), &[0x80, 64, 0]);
        let commit = gesture
            .finish(start + Duration::from_millis(100), DEFAULT_GESTURE_SETTLE)
            .unwrap();
        assert_eq!(commit.notes, [(60, 73), (64, 82), (67, 91)]);
        assert!(!commit.overflowed);
    }

    #[test]
    fn gesture_repeated_notes_and_fifth_note_are_deterministic() {
        let start = Instant::now();
        let mut gesture = GestureCapture::default();
        for (offset, note) in [60, 60, 62, 64, 65, 67].into_iter().enumerate() {
            gesture.observe(
                start + Duration::from_millis(offset as u64),
                &[0x90, note, 90 + offset as u8],
            );
        }
        for note in [60, 60, 62, 64, 65, 67] {
            gesture.observe(start + Duration::from_millis(10), &[0x90, note, 0]);
        }
        let commit = gesture
            .finish(start + Duration::from_millis(60), DEFAULT_GESTURE_SETTLE)
            .unwrap();
        assert_eq!(commit.notes.len(), 4);
        assert_eq!(commit.notes[0], (60, 90));
        assert!(commit.overflowed);
    }

    #[test]
    fn overwrite_refuses_newer_or_unknown_song_files() {
        let base = env::temp_dir().join(format!("shsong-newer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let path = base.join("untitled.shsong");
        fs::write(&path, "SHSYNTH-SONG 99\nfuture=data\n").unwrap();
        assert!(save(&base, &Song::new(&config()), true).is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "SHSYNTH-SONG 99\nfuture=data\n"
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn song_delete_accepts_any_listed_song_version() {
        let base = env::temp_dir().join(format!("shsong-delete-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let song = Song::new(&config());
        let path = save(&base, &song, false).unwrap();
        delete(&base, &song.name).unwrap();
        assert!(!path.exists());
        fs::write(&path, "SHSYNTH-SONG 99\nfuture=data\n").unwrap();
        delete(&base, &song.name).unwrap();
        assert!(!path.exists());
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn dry_run_is_non_transmitting_and_descriptive() {
        let mut c = config();
        c.enabled = false;
        let d = diagnostic(&c).unwrap();
        assert!(d.contains("NOT transmitted"));
        assert!(d.contains("profile:"));
    }
    #[test]
    fn disabled_or_missing_destination_is_an_offline_error_only() {
        let mut c = config();
        c.enabled = false;
        assert!(connect_target(&c, &PageTarget::ConfiguredExternal)
            .err()
            .expect("disabled output must stay offline")
            .to_string()
            .contains("disabled"));
        let song = Song::new(&c);
        assert!(schedule(&song, &c, 0, 0).is_ok());
    }

    #[test]
    fn pages_can_be_added_and_every_page_stays_four_lanes_wide() {
        let mut song = Song::new(&config());
        song.add_page(PageTarget::Midi("Port B".into()), 4).unwrap();
        song.add_page(PageTarget::ActiveInstrument, 7).unwrap();
        assert_eq!(pages(&song).len(), 4);
        assert!(pages(&song)
            .iter()
            .all(|page| page.lanes.len() == LANES_PER_PAGE));
        assert!(song.patterns[&0].rows.iter().all(|row| row.len() == 16));
    }

    #[test]
    fn pages_schedule_simultaneously_to_independent_devices_and_channels() {
        let c = config();
        let mut song = Song::new(&c);
        pages_mut(&mut song)[0].target = PageTarget::Midi("Hardware A".into());
        pages_mut(&mut song)[0].channel = 2;
        pages_mut(&mut song)[1].target = PageTarget::Midi("Hardware B".into());
        pages_mut(&mut song)[1].channel = 11;
        let row = &mut song.patterns.get_mut(&0).unwrap().rows[0];
        row[0].note = Note::On(60);
        row[4].note = Note::On(36);
        let notes = schedule(&song, &c, 0, 0)
            .unwrap()
            .into_iter()
            .filter(|message| message.bytes.first().is_some_and(|b| b & 0xf0 == 0x90))
            .collect::<Vec<_>>();
        assert!(notes.iter().any(|message| {
            message.target == Some(PageTarget::Midi("Hardware A".into()))
                && message.bytes[0] == 0x92
        }));
        assert!(notes.iter().any(|message| {
            message.target == Some(PageTarget::Midi("Hardware B".into()))
                && message.bytes[0] == 0x9b
        }));
    }

    #[test]
    fn per_cell_programs_precede_notes_and_stay_page_scoped() {
        let mut c = config();
        c.bank_select = BankSelectMode::Off;
        let mut song = Song::new(&c);
        pages_mut(&mut song)[0].target = PageTarget::Midi("A".into());
        pages_mut(&mut song)[0].channel = 2;
        pages_mut(&mut song)[1].target = PageTarget::Midi("B".into());
        pages_mut(&mut song)[1].channel = 7;
        song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            program: Some(11),
            ..Cell::default()
        };
        song.patterns.get_mut(&0).unwrap().rows[0][4] = Cell {
            note: Note::On(36),
            program: Some(22),
            ..Cell::default()
        };
        let messages = schedule(&song, &c, 0, 0).unwrap();
        for (target, program, note_status) in [
            (PageTarget::Midi("A".into()), vec![0xc2, 11], 0x92),
            (PageTarget::Midi("B".into()), vec![0xc7, 22], 0x97),
        ] {
            let program_at = messages
                .iter()
                .position(|message| {
                    message.target == Some(target.clone()) && message.bytes == program
                })
                .unwrap();
            let note_at = messages
                .iter()
                .position(|message| {
                    message.target == Some(target.clone())
                        && message.bytes.first() == Some(&note_status)
                })
                .unwrap();
            assert!(program_at < note_at);
        }
    }

    #[test]
    fn active_instrument_and_shared_device_channels_remain_distinct() {
        let c = config();
        let mut song = Song::new(&c);
        pages_mut(&mut song)[0].target = PageTarget::ActiveInstrument;
        pages_mut(&mut song)[0].channel = 5;
        pages_mut(&mut song)[1].target = PageTarget::Midi("One box".into());
        pages_mut(&mut song)[1].channel = 9;
        song.add_page(PageTarget::Midi("One box".into()), 10)
            .unwrap();
        let row = &mut song.patterns.get_mut(&0).unwrap().rows[0];
        row[0].note = Note::On(60);
        row[4].note = Note::On(61);
        row[8].note = Note::On(62);
        let notes = schedule(&song, &c, 0, 0)
            .unwrap()
            .into_iter()
            .filter(|message| message.bytes.first().is_some_and(|b| b & 0xf0 == 0x90))
            .collect::<Vec<_>>();
        assert!(notes
            .iter()
            .any(|m| { m.target == Some(PageTarget::ActiveInstrument) && m.bytes[0] == 0x95 }));
        assert!(notes.iter().any(|m| m.bytes[0] == 0x99));
        assert!(notes.iter().any(|m| m.bytes[0] == 0x9a));
    }

    #[test]
    fn offline_exact_target_and_setup_round_trip_without_rebinding() {
        let mut song = Song::new(&config());
        pages_mut(&mut song)[0].target = PageTarget::Midi("Missing forever".into());
        pages_mut(&mut song)[0].setup = vec![vec![0xb3, 0, 12], vec![0xc3, 7]];
        let decoded = decode(&encode(&song).unwrap()).unwrap();
        assert_eq!(decoded, song);
        assert!(schedule(&decoded, &config(), 0, 0)
            .unwrap()
            .iter()
            .any(|m| {
                m.target == Some(PageTarget::Midi("Missing forever".into()))
                    && m.bytes == [0xb3, 0, 12]
            }));
    }

    #[test]
    fn cleanup_is_owned_by_lane_destination_and_channel() {
        let active = BTreeMap::from([
            (0, (PageTarget::Midi("A".into()), 0, BTreeSet::from([60]))),
            (1, (PageTarget::Midi("A".into()), 1, BTreeSet::from([61]))),
            (2, (PageTarget::ActiveInstrument, 0, BTreeSet::from([62]))),
        ]);
        assert_eq!(
            planned_lane_cleanup(&active),
            vec![
                (PageTarget::Midi("A".into()), vec![0x80, 60, 0]),
                (PageTarget::Midi("A".into()), vec![0x81, 61, 0]),
                (PageTarget::ActiveInstrument, vec![0x80, 62, 0]),
            ]
        );
    }

    #[test]
    fn shared_note_is_released_only_after_its_last_lane_owner() {
        let target = PageTarget::Midi("shared".into());
        let key = (target.clone(), 3, 60);
        let mut owners = BTreeMap::from([(key.clone(), BTreeSet::from([0, 1]))]);
        assert!(!release_note_owner(&mut owners, 0, &target, 3, 60));
        assert_eq!(owners[&key], BTreeSet::from([1]));
        assert!(release_note_owner(&mut owners, 1, &target, 3, 60));
        assert!(!owners.contains_key(&key));
    }
}
