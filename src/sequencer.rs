//! Multi-destination FT2-style sequencing. Song editing/storage and event
//! planning remain independent from the owned software-synth lifecycle.
use crate::config::{BankSelectMode, ExternalMidiConfig};
use crate::device_profile::Registry as DeviceProfiles;
use anyhow::{anyhow, bail, Context, Result};
use midir::{MidiOutput, MidiOutputConnection};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const SONG_VERSION: u8 = 1;
pub const LANES_PER_PAGE: usize = 4;
const MAX_PROJECT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PROJECT_PATTERNS: usize = 256;
const MAX_ARRANGEMENT_STEPS: usize = 4096;
const MAX_PROJECT_CELLS: usize = 1_048_576;
const MAX_SETUP_MESSAGES_PER_PAGE: usize = 256;
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
    /// MIDI channel and master instrument are independent for each of the
    /// page's four visible tracker columns. The destination remains common.
    pub columns: [ColumnSetup; LANES_PER_PAGE],
    pub velocity: u8,
    pub percussion: bool,
    pub target: PageTarget,
    /// Reserved for a later small per-page MIDI setup sequence. It is stored
    /// and routed, but deliberately has no editor yet.
    pub setup: Vec<Vec<u8>>,
    pub lanes: Vec<Lane>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ColumnSetup {
    pub channel: u8,
    pub bank_msb: u8,
    pub bank_lsb: u8,
    pub program: u8,
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
    pub(crate) fn validate(self) -> Result<()> {
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
        if message.len() < 3 || message[1] > 127 || message[2] > 127 {
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
        validate_label(&self.name, "project name", 64)?;
        if !(1..=16).contains(&self.steps_per_beat) || !(1..=100).contains(&self.gate_percent) {
            bail!("project steps/gate out of range");
        }
        if self.order.is_empty() || self.order.len() > MAX_ARRANGEMENT_STEPS {
            bail!("project needs 1..={MAX_ARRANGEMENT_STEPS} arrangement steps");
        }
        if self.patterns.is_empty() || self.patterns.len() > MAX_PROJECT_PATTERNS {
            bail!("project needs 1..={MAX_PROJECT_PATTERNS} patterns");
        }
        if let Some(audio_loop) = &self.audio_loop {
            if validate_label(&audio_loop.file, "private loop filename", 255).is_err()
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
        let total_cells = self.total_cell_count()?;
        if total_cells > MAX_PROJECT_CELLS {
            bail!("project exceeds {MAX_PROJECT_CELLS} cells");
        }
        Ok(())
    }

    fn total_cell_count(&self) -> Result<usize> {
        self.patterns.values().try_fold(0usize, |total, pattern| {
            let pattern_cells = pattern
                .rows
                .len()
                .checked_mul(pattern.total_lanes())
                .context("project cell count overflow")?;
            total
                .checked_add(pattern_cells)
                .context("project cell count overflow")
        })
    }

    pub fn append_pattern(&mut self, pattern: Pattern) -> Result<u16> {
        if self.patterns.len() >= MAX_PROJECT_PATTERNS {
            bail!("project already has {MAX_PROJECT_PATTERNS} patterns");
        }
        if self.order.len() >= MAX_ARRANGEMENT_STEPS {
            bail!("arrangement already has {MAX_ARRANGEMENT_STEPS} steps");
        }
        pattern.validate()?;
        let added_cells = pattern
            .rows
            .len()
            .checked_mul(pattern.total_lanes())
            .context("project cell count overflow")?;
        let projected = self
            .total_cell_count()?
            .checked_add(added_cells)
            .context("project cell count overflow")?;
        if projected > MAX_PROJECT_CELLS {
            bail!("project would exceed {MAX_PROJECT_CELLS} cells");
        }
        let number = self
            .patterns
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .context("pattern number space is exhausted")?;
        self.patterns.insert(number, pattern);
        self.order.push(number);
        Ok(number)
    }

    pub fn replace_pattern(&mut self, number: u16, pattern: Pattern) -> Result<()> {
        pattern.validate()?;
        let old = self.patterns.get(&number).context("pattern missing")?;
        let old_cells = old
            .rows
            .len()
            .checked_mul(old.total_lanes())
            .context("project cell count overflow")?;
        let new_cells = pattern
            .rows
            .len()
            .checked_mul(pattern.total_lanes())
            .context("project cell count overflow")?;
        let projected = self
            .total_cell_count()?
            .checked_sub(old_cells)
            .and_then(|total| total.checked_add(new_cells))
            .context("project cell count overflow")?;
        if projected > MAX_PROJECT_CELLS {
            bail!("project would exceed {MAX_PROJECT_CELLS} cells");
        }
        self.patterns.insert(number, pattern);
        Ok(())
    }

    pub fn insert_arrangement_step(&mut self, index: usize, pattern: u16) -> Result<usize> {
        if self.order.len() >= MAX_ARRANGEMENT_STEPS {
            bail!("arrangement already has {MAX_ARRANGEMENT_STEPS} steps");
        }
        if !self.patterns.contains_key(&pattern) {
            bail!("arrangement pattern is missing");
        }
        if index > self.order.len() {
            bail!("arrangement insertion is out of range");
        }
        self.order.insert(index, pattern);
        Ok(index)
    }

    pub fn pattern_reference_count(&self, number: u16) -> usize {
        self.order
            .iter()
            .filter(|candidate| **candidate == number)
            .count()
    }

    /// Delete only an arrangement-orphaned pattern. No order step is ever
    /// rewritten as a side effect, and errors leave the song untouched.
    pub fn delete_unused_pattern(&mut self, number: u16) -> Result<()> {
        let references = self.pattern_reference_count(number);
        if references != 0 {
            bail!("pattern {number} is referenced by {references} arrangement step(s)");
        }
        if self.patterns.len() <= 1 {
            bail!("a Project must keep at least one pattern");
        }
        if !self.patterns.contains_key(&number) {
            bail!("pattern {number} does not exist");
        }
        self.patterns.remove(&number);
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
            columns: [ColumnSetup {
                channel,
                bank_msb: 0,
                bank_lsb: 0,
                program,
            }; LANES_PER_PAGE],
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

    pub fn column(&self, lane: usize) -> &ColumnSetup {
        &self.columns[lane.min(LANES_PER_PAGE - 1)]
    }

    pub fn column_mut(&mut self, lane: usize) -> &mut ColumnSetup {
        &mut self.columns[lane.min(LANES_PER_PAGE - 1)]
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
            .get(&pattern_number)
            .context("pattern missing")?;
        if pattern.pages.len() >= 64 {
            bail!("pattern already has 64 pages");
        }
        let projected = self
            .total_cell_count()?
            .checked_add(
                pattern
                    .rows
                    .len()
                    .checked_mul(LANES_PER_PAGE)
                    .context("project cell count overflow")?,
            )
            .context("project cell count overflow")?;
        if projected > MAX_PROJECT_CELLS {
            bail!("project would exceed {MAX_PROJECT_CELLS} cells");
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

    /// Transpose note-ons on melodic pages as one atomic edit. Percussion
    /// pages and note-off/empty cells are deliberately unchanged.
    pub fn transpose_melodic(&mut self, semitones: i8) -> Result<usize> {
        let melodic_lanes = self
            .pages
            .iter()
            .enumerate()
            .filter(|(_, page)| !page.percussion)
            .flat_map(|(page, _)| {
                let start = page * LANES_PER_PAGE;
                start..start + LANES_PER_PAGE
            })
            .collect::<Vec<_>>();
        let mut changed = 0;
        for lane in &melodic_lanes {
            for cell in &self.rows {
                if let Note::On(note) = cell[*lane].note {
                    let shifted = i16::from(note) + i16::from(semitones);
                    if !(0..=127).contains(&shifted) {
                        bail!("transpose would move MIDI note {note} outside 0..=127");
                    }
                    changed += 1;
                }
            }
        }
        for lane in melodic_lanes {
            for row in &mut self.rows {
                if let Note::On(note) = row[lane].note {
                    row[lane].note = Note::On((i16::from(note) + i16::from(semitones)) as u8);
                }
            }
        }
        Ok(changed)
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
        let mut channel_programs = BTreeMap::new();
        for page in &self.pages {
            validate_label(&page.name, "pattern page name", 64)?;
            for lane in &page.lanes {
                validate_label(&lane.name, "pattern lane name", 64)?;
            }
            if page.velocity > 127
                || page.columns.iter().any(|column| {
                    column.channel > 15
                        || column.bank_msb > 127
                        || column.bank_lsb > 127
                        || column.program > 127
                })
            {
                bail!("pattern page MIDI value out of range");
            }
            if page.enabled {
                for (lane, column) in page.columns.iter().enumerate() {
                    if !page.lanes[lane].enabled {
                        continue;
                    }
                    let key = (page.target.clone(), column.channel);
                    let selection = (column.bank_msb, column.bank_lsb, column.program);
                    if let Some(old) = channel_programs
                        .insert(key.clone(), selection)
                        .filter(|old| *old != selection)
                    {
                        bail!(
                            "conflicting master instruments share {} channel {}: {}/{}/{} versus {}/{}/{}",
                            key.0.label(),
                            key.1 + 1,
                            old.0,
                            old.1,
                            old.2,
                            selection.0,
                            selection.1,
                            selection.2
                        );
                    }
                }
            }
            if let PageTarget::Midi(name) = &page.target {
                validate_label(name, "pattern page MIDI target", 256)?;
            }
            if page.setup.len() > MAX_SETUP_MESSAGES_PER_PAGE
                || page
                    .setup
                    .iter()
                    .any(|message| message.is_empty() || message.len() > 256)
            {
                bail!(
                    "a page may contain at most {MAX_SETUP_MESSAGES_PER_PAGE} setup messages of 1..=256 bytes"
                );
            }
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

fn validate_label(value: &str, description: &str, max_chars: usize) -> Result<()> {
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        bail!("{description} must contain 1..={max_chars} printable characters");
    }
    Ok(())
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
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                return None;
            }
            let path = entry.path();
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("shsong"))
            {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_owned();
            (safe_name(&name) == name).then_some(name)
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
                "pattern_page={number}|{page_index}|{}|{}|{}|{}|{}\n",
                escape(&page.name),
                u8::from(page.enabled),
                page.velocity,
                u8::from(page.percussion),
                target_text(&page.target)
            ));
            for (column_index, column) in page.columns.iter().enumerate() {
                out.push_str(&format!(
                    "pattern_column={number}|{page_index}|{column_index}|{}|{}|{}|{}\n",
                    column.channel + 1,
                    column.bank_msb,
                    column.bank_lsb,
                    column.program
                ));
            }
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
    if text.len() > MAX_PROJECT_BYTES {
        bail!("song file exceeds {MAX_PROJECT_BYTES} bytes");
    }
    let mut lines = text.lines();
    let header = lines.next().context("empty song")?;
    let version = header
        .strip_prefix("SHSYNTH-SONG ")
        .context("not an SHR-DAW song")?
        .parse::<u8>()?;
    if version > SONG_VERSION {
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
    let mut pattern_columns = Vec::new();
    let mut pattern_setup = Vec::new();
    let mut cells = Vec::new();
    for line in lines.filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
        let (key, value) = line.split_once('=').context("invalid song line")?;
        match key {
            "name" => set_once(&mut name, unescape(value)?, "name")?,
            "steps" => set_once(&mut steps, value.parse()?, "steps")?,
            "gate" => set_once(&mut gate, value.parse()?, "gate")?,
            "loop" => {
                let f = value.split('|').collect::<Vec<_>>();
                if f.len() != 6 {
                    bail!("invalid loop settings");
                }
                set_once(
                    &mut audio_loop,
                    LoopSettings {
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
                    },
                    "loop",
                )?;
            }
            "order" => {
                let parsed = value
                    .split(',')
                    .map(str::parse)
                    .collect::<std::result::Result<Vec<u16>, _>>()?;
                if parsed.len() > MAX_ARRANGEMENT_STEPS {
                    bail!("arrangement exceeds {MAX_ARRANGEMENT_STEPS} steps");
                }
                set_once(&mut order, parsed, "order")?;
            }
            "pattern" => {
                let f = value.split('|').collect::<Vec<_>>();
                match f.as_slice() {
                    [number, rows, tempo, meter] => {
                        let number = number.parse()?;
                        let rows = rows.parse::<usize>()?;
                        if !(1..=256).contains(&rows) {
                            bail!("pattern must have 1..=256 rows");
                        }
                        if patterns.len() >= MAX_PROJECT_PATTERNS {
                            bail!("project exceeds {MAX_PROJECT_PATTERNS} patterns");
                        }
                        if patterns
                            .insert(
                                number,
                                Pattern {
                                    tempo: tempo.parse()?,
                                    meter: meter.parse()?,
                                    pages: Vec::new(),
                                    rows: vec![Vec::new(); rows],
                                },
                            )
                            .is_some()
                        {
                            bail!("duplicate pattern {number}");
                        }
                    }
                    _ => bail!("invalid pattern"),
                }
            }
            "pattern_page" => {
                let f = value.split('|').collect::<Vec<_>>();
                let (page, legacy_column) = match (version, f.as_slice()) {
                    (
                        0,
                        [_, _, name, enabled, channel, bank_msb, bank_lsb, program, velocity, percussion, target],
                    ) => (
                        Page {
                            name: unescape(name)?,
                            enabled: binary_flag(enabled, "pattern page enabled")?,
                            columns: [ColumnSetup {
                                channel: one_based_channel(channel)?,
                                bank_msb: midi_value(bank_msb)?,
                                bank_lsb: midi_value(bank_lsb)?,
                                program: midi_value(program)?,
                            }; LANES_PER_PAGE],
                            velocity: midi_value(velocity)?,
                            percussion: binary_flag(percussion, "pattern page percussion")?,
                            target: parse_target(target)?,
                            setup: Vec::new(),
                            lanes: Vec::new(),
                        },
                        true,
                    ),
                    (1, [_, _, name, enabled, velocity, percussion, target]) => (
                        Page {
                            name: unescape(name)?,
                            enabled: binary_flag(enabled, "pattern page enabled")?,
                            columns: [ColumnSetup::default(); LANES_PER_PAGE],
                            velocity: midi_value(velocity)?,
                            percussion: binary_flag(percussion, "pattern page percussion")?,
                            target: parse_target(target)?,
                            setup: Vec::new(),
                            lanes: Vec::new(),
                        },
                        false,
                    ),
                    _ => bail!("invalid pattern page"),
                };
                let page_number = f[1].parse::<usize>()?;
                let replaced = pattern_pages
                    .entry(f[0].parse::<u16>()?)
                    .or_default()
                    .insert(page_number, page);
                if replaced.is_some() {
                    bail!("duplicate pattern page {page_number}");
                }
                debug_assert_eq!(legacy_column, version == 0);
            }
            "pattern_lane" => pattern_lanes.push(value.to_owned()),
            "pattern_column" if version == 1 => pattern_columns.push(value.to_owned()),
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
    if version == 1 {
        attach_pattern_columns(&mut patterns, pattern_columns)?;
    }
    attach_pattern_setup(&mut patterns, pattern_setup)?;
    let total_cells = patterns.values().try_fold(0usize, |total, pattern| {
        total
            .checked_add(
                pattern
                    .rows
                    .len()
                    .checked_mul(pattern.total_lanes())
                    .context("project cell count overflow")?,
            )
            .context("project cell count overflow")
    })?;
    if total_cells > MAX_PROJECT_CELLS {
        bail!("project exceeds {MAX_PROJECT_CELLS} cells");
    }
    for pattern in patterns.values_mut() {
        let total_lanes = pattern.pages.len() * LANES_PER_PAGE;
        for row in &mut pattern.rows {
            row.resize(total_lanes, Cell::default());
        }
    }
    let mut occupied_cells = BTreeSet::new();
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
        if !occupied_cells.insert((f[0].parse::<u16>()?, row_index, track_index)) {
            bail!("duplicate cell");
        }
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
            enabled: binary_flag(f[4], "pattern lane enabled")?,
        });
    }
    Ok(())
}

fn attach_pattern_columns(
    patterns: &mut BTreeMap<u16, Pattern>,
    columns: Vec<String>,
) -> Result<()> {
    let expected = patterns
        .values()
        .map(|pattern| pattern.pages.len() * LANES_PER_PAGE)
        .sum::<usize>();
    if columns.len() != expected {
        bail!("each pattern page needs exactly four column setups");
    }
    let mut occupied = BTreeSet::new();
    for value in columns {
        let f = value.split('|').collect::<Vec<_>>();
        if f.len() != 7 {
            bail!("invalid pattern column");
        }
        let pattern_number = f[0].parse::<u16>()?;
        let page_index = f[1].parse::<usize>()?;
        let column_index = f[2].parse::<usize>()?;
        if column_index >= LANES_PER_PAGE
            || !occupied.insert((pattern_number, page_index, column_index))
        {
            bail!("duplicate or invalid pattern column");
        }
        let page = patterns
            .get_mut(&pattern_number)
            .and_then(|pattern| pattern.pages.get_mut(page_index))
            .context("column page missing")?;
        page.columns[column_index] = ColumnSetup {
            channel: one_based_channel(f[3])?,
            bank_msb: midi_value(f[4])?,
            bank_lsb: midi_value(f[5])?,
            program: midi_value(f[6])?,
        };
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
        if page.setup.len() >= MAX_SETUP_MESSAGES_PER_PAGE {
            bail!("page exceeds {MAX_SETUP_MESSAGES_PER_PAGE} setup messages");
        }
        page.setup.push(parse_setup_message(f[2])?);
    }
    Ok(())
}

fn set_once<T>(slot: &mut Option<T>, value: T, field: &str) -> Result<()> {
    if slot.replace(value).is_some() {
        bail!("duplicate song field {field}");
    }
    Ok(())
}

fn binary_flag(value: &str, description: &str) -> Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => bail!("{description} must be 0 or 1"),
    }
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
            .is_some_and(|version| version <= SONG_VERSION);
        if !supported {
            bail!("refusing to overwrite unsupported/newer song file");
        }
    }
    let encoded = encode(song)?;
    if overwrite {
        crate::fsutil::atomic_write(&path, encoded.as_bytes())?;
    } else {
        crate::fsutil::atomic_write_noreplace(&path, encoded.as_bytes())
            .context("publish song without replacement")?;
    }
    Ok(path)
}

pub fn load(base: &Path, name: &str) -> Result<Song> {
    decode(&fs::read_to_string(song_path(base, name)?)?)
}

pub fn delete(base: &Path, name: &str) -> Result<()> {
    fs::remove_file(song_path(base, name)?)?;
    Ok(())
}

/// Publish a renamed Project without replacing either source or destination.
/// The destination is fully encoded before the old directory entry is removed,
/// so every failure before removal preserves the original Project.
pub fn rename_project(base: &Path, old_stem: &str, display_name: &str) -> Result<(Song, PathBuf)> {
    validate_label(display_name, "project name", 64)?;
    let new_stem = safe_name(display_name);
    let old_path = song_path(base, old_stem)?;
    let new_path = song_path(base, &new_stem)?;
    let mut song = load(base, old_stem)?;
    song.name = display_name.to_owned();
    song.validate()?;
    if old_path == new_path {
        crate::fsutil::atomic_write(&old_path, encode(&song)?.as_bytes())?;
    } else {
        crate::fsutil::atomic_write_noreplace(&new_path, encode(&song)?.as_bytes())
            .context("publish renamed Project without replacement")?;
        if let Err(error) = fs::remove_file(&old_path) {
            let _ = fs::remove_file(&new_path);
            return Err(error).context("remove old Project name");
        }
        fs::File::open(base)?.sync_all()?;
    }
    Ok((song, new_path))
}

fn song_path(base: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || safe_name(name) != name {
        bail!("invalid song name");
    }
    Ok(base.join(format!("{name}.shsong")))
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
    let device_profiles = DeviceProfiles::discover();
    let first_pattern = song
        .order
        .get(start_order)
        .and_then(|number| song.patterns.get(number))
        .context("start order outside arrangement")?;
    if start_row >= first_pattern.rows.len() {
        bail!("start row outside pattern");
    }
    let mut result = Vec::new();
    let mut at = Duration::ZERO;
    let mut clock_step = 0usize;
    let mut active: BTreeMap<usize, (PageTarget, u8, u8)> = BTreeMap::new();
    for (order_index, pattern_number) in song.order.iter().enumerate().skip(start_order) {
        let pattern = &song.patterns[pattern_number];
        let mut tempo = pattern.tempo;
        let mut programmed = vec![false; pattern.total_lanes()];
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
                let targets = pattern
                    .pages
                    .iter()
                    .filter(|page| page.enabled)
                    .map(|page| page.target.clone())
                    .collect::<BTreeSet<_>>();
                for target in targets {
                    for offset in midi_clock_offsets(clock_step, song.steps_per_beat, row_duration)
                    {
                        push(
                            &mut result,
                            at + offset,
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
                let column_index = lane_index % LANES_PER_PAGE;
                let lane = &page.lanes[column_index];
                let column = page.column(column_index);
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
                        if cell.program.is_some() || !programmed[lane_index] {
                            append_program(
                                &mut result,
                                SchedulePosition {
                                    at: event_at,
                                    order: order_index,
                                    row: row_index,
                                },
                                page,
                                column,
                                cell.program.unwrap_or(column.program),
                                config,
                                &device_profiles,
                            );
                            programmed[lane_index] = true;
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
                        active.insert(lane_index, (page.target.clone(), column.channel, note));
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
                                    0x90 | column.channel,
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
                                vec![0x80 | column.channel, note, 0],
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
            clock_step += 1;
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

/// MIDI clock is always 24 pulses per quarter note. When the tracker uses a
/// row count that does not divide 24, distribute pulses across rows without
/// changing the average clock rate.
fn midi_clock_offsets(
    step: usize,
    steps_per_beat: u8,
    row_duration: Duration,
) -> impl Iterator<Item = Duration> {
    let steps = usize::from(steps_per_beat);
    let phase = step % steps;
    let first_tick = (phase * 24).div_ceil(steps);
    let end_tick = ((phase + 1) * 24).div_ceil(steps);
    (first_tick..end_tick).map(move |tick| {
        let numerator = tick * steps - phase * 24;
        row_duration.mul_f64(numerator as f64 / 24.0)
    })
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

#[derive(Clone, Copy)]
struct SchedulePosition {
    at: Duration,
    order: usize,
    row: usize,
}

fn append_program(
    out: &mut Vec<ScheduledMessage>,
    position: SchedulePosition,
    page: &Page,
    column: &ColumnSetup,
    program: u8,
    config: &ExternalMidiConfig,
    device_profiles: &DeviceProfiles,
) {
    let mut selection = config.clone();
    let profile = match &page.target {
        PageTarget::ConfiguredExternal => device_profiles.by_id(&config.profile),
        PageTarget::Midi(port) => device_profiles.matching_port(port),
        PageTarget::ActiveInstrument => None,
    };
    if let Some(profile) = profile {
        profile.apply_midi_selection(&mut selection);
    }
    match selection.bank_select {
        BankSelectMode::Off => {}
        BankSelectMode::Cc0 => push(
            out,
            position.at,
            position.order,
            position.row,
            vec![0xb0 | column.channel, 0, column.bank_msb],
            Some(page.target.clone()),
        ),
        BankSelectMode::Cc0Cc32 => {
            push(
                out,
                position.at,
                position.order,
                position.row,
                vec![0xb0 | column.channel, 0, column.bank_msb],
                Some(page.target.clone()),
            );
            push(
                out,
                position.at,
                position.order,
                position.row,
                vec![0xb0 | column.channel, 32, column.bank_lsb],
                Some(page.target.clone()),
            );
        }
    }
    if selection.program_changes {
        push(
            out,
            position.at,
            position.order,
            position.row,
            vec![0xc0 | column.channel, program],
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
                update_active_notes(
                    &mut active_notes,
                    message.lane,
                    message.target.as_ref(),
                    &message.bytes,
                );
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

fn update_active_notes(
    active: &mut BTreeMap<usize, (PageTarget, u8, BTreeSet<u8>)>,
    lane: Option<usize>,
    target: Option<&PageTarget>,
    bytes: &[u8],
) {
    let (Some(lane), Some(target), [status, note, velocity, ..]) = (lane, target, bytes) else {
        return;
    };
    let channel = status & 0x0f;
    match status & 0xf0 {
        0x90 if *velocity > 0 => {
            active
                .entry(lane)
                .or_insert_with(|| (target.clone(), channel, BTreeSet::new()))
                .2
                .insert(*note);
        }
        0x80 | 0x90 => {
            let empty = active.get_mut(&lane).is_some_and(|(_, _, notes)| {
                notes.remove(note);
                notes.is_empty()
            });
            if empty {
                active.remove(&lane);
            }
        }
        _ => {}
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
    let channel = *config
        .channels
        .first()
        .context("external MIDI has no configured channels")?;
    if channel > 15 {
        bail!("external MIDI channel out of range");
    }
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
        columns: [ColumnSetup {
            channel,
            bank_msb: 0,
            bank_lsb: 0,
            program: 0,
        }; LANES_PER_PAGE],
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
    append_program(
        &mut dry,
        SchedulePosition {
            at: Duration::ZERO,
            order: 0,
            row: 0,
        },
        &page,
        page.column(0),
        0,
        config,
        &DeviceProfiles::discover(),
    );
    push(
        &mut dry,
        Duration::ZERO,
        0,
        0,
        vec![0x90 | page.column(0).channel, 60, 64],
        Some(page.target.clone()),
    );
    push(
        &mut dry,
        Duration::from_millis(250),
        0,
        0,
        vec![0x80 | page.column(0).channel, 60, 0],
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
        assert!(encoded.starts_with("SHSYNTH-SONG 1\n"));
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
    fn exact_device_target_uses_its_own_program_selection_protocol() {
        let mut config = config();
        config.bank_select = BankSelectMode::Cc0Cc32;
        config.program_changes = true;
        let mut song = Song::new(&config);
        let page = &mut song.patterns.get_mut(&0).unwrap().pages[0];
        page.target = PageTarget::Midi("USB MIDI: Roland D-50".into());
        for column in &mut page.columns {
            column.bank_msb = 5;
            column.bank_lsb = 9;
        }
        song.patterns.get_mut(&0).unwrap().rows[0][0] = Cell {
            note: Note::On(60),
            program: Some(7),
            ..Cell::default()
        };

        let transmitted = schedule(&song, &config, 0, 0)
            .unwrap()
            .into_iter()
            .filter(|message| !message.bytes.is_empty())
            .map(|message| message.bytes)
            .collect::<Vec<_>>();
        assert_eq!(transmitted[0], [0xc0, 7]);
        assert_eq!(transmitted[1], [0x90, 60, 96]);
        assert!(!transmitted.iter().any(|message| {
            message.len() >= 2 && message[0] & 0xf0 == 0xb0 && matches!(message[1], 0 | 32)
        }));
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
        pages_mut(&mut song)[0].column_mut(0).channel = 0;
        song.patterns.get_mut(&0).unwrap().rows[0][0].note = Note::On(60);
        let mut second = Pattern::empty_like_setup(1, &song.patterns[&0]);
        second.pages[0].target = PageTarget::Midi("B".into());
        second.pages[0].column_mut(0).channel = 5;
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
        assert_eq!(pages(&song)[1].column(0).channel, 1);
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
        assert_eq!(pages(&song)[1].column(0).channel, 1);
        assert_eq!(pages(&song)[1].column(0).program, 9);
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
    fn bounded_project_mutations_leave_the_song_unchanged_on_error() {
        let config = config();
        let mut pages_song = Song::new(&config);
        while pages_song.patterns[&0].pages.len() < 64 {
            pages_song
                .add_page(PageTarget::ConfiguredExternal, 0)
                .unwrap();
        }
        let page_snapshot = pages_song.clone();
        assert!(pages_song
            .add_page(PageTarget::ConfiguredExternal, 0)
            .is_err());
        assert_eq!(pages_song, page_snapshot);

        let mut pattern_song = Song::new(&config);
        let pattern = pattern_song.patterns[&0].clone();
        for number in 1..MAX_PROJECT_PATTERNS as u16 {
            pattern_song.patterns.insert(number, pattern.clone());
        }
        let pattern_snapshot = pattern_song.clone();
        assert!(pattern_song.append_pattern(pattern).is_err());
        assert_eq!(pattern_song, pattern_snapshot);

        let mut arrangement_song = Song::new(&config);
        arrangement_song.order = vec![0; MAX_ARRANGEMENT_STEPS];
        let arrangement_snapshot = arrangement_song.clone();
        assert!(arrangement_song.insert_arrangement_step(0, 0).is_err());
        assert_eq!(arrangement_song, arrangement_snapshot);

        let mut replacement_song = Song::new(&config);
        let replacement_snapshot = replacement_song.clone();
        let invalid = Pattern::new(0, 120, 4, default_pages(&config));
        assert!(replacement_song.replace_pattern(0, invalid).is_err());
        assert_eq!(replacement_song, replacement_snapshot);
    }

    #[test]
    fn pages_schedule_simultaneously_to_independent_devices_and_channels() {
        let c = config();
        let mut song = Song::new(&c);
        pages_mut(&mut song)[0].target = PageTarget::Midi("Hardware A".into());
        pages_mut(&mut song)[0].column_mut(0).channel = 2;
        pages_mut(&mut song)[1].target = PageTarget::Midi("Hardware B".into());
        pages_mut(&mut song)[1].column_mut(0).channel = 11;
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
        pages_mut(&mut song)[0].column_mut(0).channel = 2;
        pages_mut(&mut song)[1].target = PageTarget::Midi("B".into());
        pages_mut(&mut song)[1].column_mut(0).channel = 7;
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
        pages_mut(&mut song)[0].column_mut(0).channel = 5;
        pages_mut(&mut song)[1].target = PageTarget::Midi("One box".into());
        pages_mut(&mut song)[1].column_mut(0).channel = 9;
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

    #[test]
    fn project_list_ignores_temporary_unrelated_and_directory_entries() {
        let base = env::temp_dir().join(format!("shsong-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("fake.shsong")).unwrap();
        fs::write(base.join("alpha.shsong"), "project").unwrap();
        fs::write(base.join("BETA.SHSONG"), "project").unwrap();
        std::os::unix::fs::symlink(base.join("alpha.shsong"), base.join("alias.shsong")).unwrap();
        fs::write(base.join(".alpha.123.tmp"), "temporary").unwrap();
        fs::write(base.join("notes.txt"), "unrelated").unwrap();
        assert_eq!(list(&base), ["BETA", "alpha"]);
        assert!(load(&base, "../alpha").is_err());
        assert!(delete(&base, "../alpha").is_err());
        assert!(base.join("alpha.shsong").exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn in_memory_page_values_and_setup_are_validated_before_save() {
        let mut song = Song::new(&config());
        pages_mut(&mut song)[0].column_mut(0).channel = 16;
        assert!(encode(&song)
            .unwrap_err()
            .to_string()
            .contains("MIDI value"));

        pages_mut(&mut song)[0].column_mut(0).channel = 0;
        pages_mut(&mut song)[0].setup = vec![Vec::new()];
        assert!(encode(&song)
            .unwrap_err()
            .to_string()
            .contains("setup message"));
    }

    #[test]
    fn schedule_rejects_out_of_range_start_without_zero_time_loop() {
        let song = Song::new(&config());
        assert!(schedule(&song, &config(), song.order.len(), 0).is_err());
        assert!(schedule(&song, &config(), 0, song.patterns[&0].rows.len()).is_err());
    }

    #[test]
    fn midi_clock_keeps_twenty_four_ppqn_for_non_divisor_row_grids() {
        let mut cfg = config();
        cfg.send_transport = true;
        cfg.steps_per_beat = 5;
        let mut song = Song::new(&cfg);
        song.patterns.get_mut(&0).unwrap().rows.truncate(5);
        let clocks = schedule(&song, &cfg, 0, 0)
            .unwrap()
            .into_iter()
            .filter(|message| message.bytes == [0xf8])
            .collect::<Vec<_>>();
        // Two enabled page targets share one configured destination, so clock
        // is de-duplicated and sent exactly 24 times per quarter note.
        assert_eq!(clocks.len(), 24);
        assert_eq!(clocks.first().unwrap().at, Duration::ZERO);
        assert!(clocks.last().unwrap().at < Duration::from_millis(500));
    }

    #[test]
    fn stopped_lane_cleanup_follows_a_later_pattern_target() {
        let first = PageTarget::Midi("first".into());
        let second = PageTarget::Midi("second".into());
        let mut active = BTreeMap::new();
        update_active_notes(&mut active, Some(0), Some(&first), &[0x90, 60, 100]);
        update_active_notes(&mut active, Some(0), Some(&first), &[0x80, 60, 0]);
        update_active_notes(&mut active, Some(0), Some(&second), &[0x95, 62, 100]);
        assert_eq!(planned_lane_cleanup(&active), [(second, vec![0x85, 62, 0])]);
    }

    #[test]
    fn song_decoder_rejects_oversized_duplicate_and_non_binary_fields() {
        let encoded = encode(&Song::new(&config())).unwrap();
        assert!(decode(&encoded.replace("pattern=0|64|", "pattern=0|257|")).is_err());
        assert!(decode(&encoded.replace("steps=4\n", "steps=4\nsteps=4\n")).is_err());
        assert!(decode(&encoded.replace("|MELODY|1|", "|MELODY|yes|")).is_err());

        let duplicate_pattern = encoded.replace(
            "pattern=0|64|120|4\n",
            "pattern=0|64|120|4\npattern=0|64|120|4\n",
        );
        assert!(decode(&duplicate_pattern).is_err());
    }

    #[test]
    fn in_memory_song_limits_apply_before_save_or_schedule() {
        let mut song = Song::new(&config());
        song.name = "bad\nname".into();
        assert!(encode(&song).is_err());

        song.name = "bounded".into();
        song.order = vec![0; MAX_ARRANGEMENT_STEPS + 1];
        assert!(schedule(&song, &config(), 0, 0).is_err());

        let mut invalid_config = config();
        invalid_config.channels.clear();
        assert!(diagnostic(&invalid_config).is_err());
    }

    #[test]
    fn version_zero_page_setup_migrates_to_four_identical_columns() {
        let legacy = "SHSYNTH-SONG 0\nname=legacy\nsteps=4\ngate=80\norder=0\npattern=0|1|120|4\npattern_page=0|0|MELODY|1|3|4|5|6|96|0|configured\npattern_lane=0|0|0|L1|1\npattern_lane=0|0|1|L2|1\npattern_lane=0|0|2|L3|1\npattern_lane=0|0|3|L4|1\n";
        let song = decode(legacy).unwrap();
        let page = &song.patterns[&0].pages[0];
        assert_eq!(
            page.columns,
            [ColumnSetup {
                channel: 2,
                bank_msb: 4,
                bank_lsb: 5,
                program: 6,
            }; LANES_PER_PAGE]
        );
        assert!(encode(&song).unwrap().starts_with("SHSYNTH-SONG 1\n"));
    }

    #[test]
    fn four_columns_schedule_distinct_channels_and_master_programs() {
        let mut cfg = config();
        cfg.program_changes = true;
        let mut song = Song::new(&cfg);
        song.patterns.get_mut(&0).unwrap().pages[1].enabled = false;
        let pattern = song.patterns.get_mut(&0).unwrap();
        pattern.rows.truncate(1);
        for column in 0..LANES_PER_PAGE {
            pattern.pages[0].columns[column] = ColumnSetup {
                channel: column as u8,
                program: 10 + column as u8,
                ..ColumnSetup::default()
            };
            pattern.rows[0][column].note = Note::On(60 + column as u8);
        }
        let messages = schedule(&song, &cfg, 0, 0).unwrap();
        for column in 0..LANES_PER_PAGE {
            assert!(messages
                .iter()
                .any(|message| message.bytes == [0xc0 | column as u8, 10 + column as u8]));
            assert!(messages
                .iter()
                .any(|message| message.bytes == [0x90 | column as u8, 60 + column as u8, 96]));
        }
    }

    #[test]
    fn shared_channel_requires_compatible_master_instruments() {
        let mut song = Song::new(&config());
        song.patterns.get_mut(&0).unwrap().pages[1].enabled = false;
        let page = &mut song.patterns.get_mut(&0).unwrap().pages[0];
        page.columns = [ColumnSetup {
            channel: 3,
            program: 9,
            ..ColumnSetup::default()
        }; LANES_PER_PAGE];
        assert!(song.validate().is_ok());
        song.patterns.get_mut(&0).unwrap().pages[0]
            .column_mut(2)
            .program = 10;
        assert!(song
            .validate()
            .unwrap_err()
            .to_string()
            .contains("conflicting"));
    }

    #[test]
    fn unused_pattern_deletion_never_rewrites_arrangement() {
        let mut song = Song::new(&config());
        let referenced_snapshot = song.clone();
        assert!(song
            .delete_unused_pattern(0)
            .unwrap_err()
            .to_string()
            .contains("1 arrangement"));
        assert_eq!(song, referenced_snapshot);
        let setup = song.patterns[&0].clone();
        let orphan = song.append_pattern(setup).unwrap();
        song.order.pop();
        let order = song.order.clone();
        song.delete_unused_pattern(orphan).unwrap();
        assert_eq!(song.order, order);
        assert!(!song.patterns.contains_key(&orphan));
    }

    #[test]
    fn transpose_is_atomic_and_never_changes_percussion_pages() {
        let mut song = Song::new(&config());
        let pattern = song.patterns.get_mut(&0).unwrap();
        pattern.rows[0][0].note = Note::On(60);
        pattern.rows[1][1].note = Note::On(127);
        pattern.rows[0][LANES_PER_PAGE].note = Note::On(36);
        let before = pattern.clone();
        assert!(pattern.transpose_melodic(1).is_err());
        assert_eq!(pattern, &before);

        pattern.rows[1][1].note = Note::On(72);
        assert_eq!(pattern.transpose_melodic(12).unwrap(), 2);
        assert_eq!(pattern.rows[0][0].note, Note::On(72));
        assert_eq!(pattern.rows[1][1].note, Note::On(84));
        assert_eq!(pattern.rows[0][LANES_PER_PAGE].note, Note::On(36));
    }

    #[test]
    fn project_rename_preserves_source_on_invalid_name_or_collision() {
        let base = std::env::temp_dir().join(format!("shr-rename-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let mut first = Song::new(&config());
        first.name = "first".into();
        save(&base, &first, false).unwrap();
        let mut taken = Song::new(&config());
        taken.name = "taken".into();
        save(&base, &taken, false).unwrap();
        assert!(rename_project(&base, "first", "taken").is_err());
        assert!(base.join("first.shsong").exists());
        assert!(rename_project(&base, "first", "bad\nname").is_err());
        assert!(base.join("first.shsong").exists());
        let (renamed, path) = rename_project(&base, "first", "My Bass Project").unwrap();
        assert_eq!(renamed.name, "My Bass Project");
        assert_eq!(path.file_name().unwrap(), "My-Bass-Project.shsong");
        assert!(!base.join("first.shsong").exists());
        let _ = fs::remove_dir_all(base);
    }
}
