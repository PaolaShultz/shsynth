//! Reusable four-lane drum patterns, stored independently from Projects.
use crate::sequencer::{Cell, Command, Note, LANES_PER_PAGE};
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const VERSION: u8 = 1;
const MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrumPattern {
    pub name: String,
    pub genre: String,
    pub meter: u8,
    pub rows: Vec<[Cell; LANES_PER_PAGE]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub name: String,
    pub genre: String,
    pub meter: u8,
    pub rows: usize,
    pub path: PathBuf,
    pub user: bool,
    pub(crate) bundled: Option<DrumPattern>,
}

pub fn user_dir() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".local/share")
        })
        .join("shsynth/drum-patterns")
}

fn bundled_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            dirs.push(parent.join("../share/shsynth/drum-patterns"));
        }
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("drum-patterns"));
    dirs
}

pub fn discover() -> Vec<Entry> {
    let user_dir = user_dir();
    let mut roots = bundled_dirs()
        .into_iter()
        .map(|path| (path, false))
        .collect::<Vec<_>>();
    roots.push((user_dir, true));
    let mut entries = Vec::new();
    for (root, user) in roots {
        let Ok(directory) = fs::read_dir(root) else {
            continue;
        };
        for entry in directory.flatten() {
            let path = entry.path();
            if !user
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value == "catalog.shrdrums")
            {
                if let Ok(text) = fs::read_to_string(&path) {
                    if let Ok(patterns) = decode_catalog(&text) {
                        entries.extend(patterns.into_iter().map(|pattern| Entry {
                            name: pattern.name.clone(),
                            genre: pattern.genre.clone(),
                            meter: pattern.meter,
                            rows: pattern.rows.len(),
                            path: path.clone(),
                            user: false,
                            bundled: Some(pattern),
                        }));
                    }
                }
                continue;
            }
            if !entry.file_type().is_ok_and(|kind| kind.is_file())
                || !path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("shdrum"))
            {
                continue;
            }
            if let Ok(pattern) = load_path(&path) {
                entries.push(Entry {
                    name: pattern.name.clone(),
                    genre: pattern.genre.clone(),
                    meter: pattern.meter,
                    rows: pattern.rows.len(),
                    path,
                    user,
                    bundled: (!user).then_some(pattern),
                });
            }
        }
    }
    entries.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.user.cmp(&right.user))
            .then_with(|| left.path.cmp(&right.path))
    });
    // An installed development binary may see both its installed data and the
    // checkout embedded at build time. Present each bundled groove once.
    entries.dedup_by(|left, right| left.name == right.name && left.user == right.user);
    entries
}

pub fn load(entry: &Entry) -> Result<DrumPattern> {
    entry
        .bundled
        .clone()
        .map(Ok)
        .unwrap_or_else(|| load_path(&entry.path))
}

pub fn load_path(path: &Path) -> Result<DrumPattern> {
    decode(&fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?)
}

pub fn save_user(pattern: &DrumPattern, stem: &str) -> Result<PathBuf> {
    let stem = crate::sequencer::safe_name(stem);
    let base = user_dir();
    fs::create_dir_all(&base)?;
    let path = base.join(format!("{stem}.shdrum"));
    crate::fsutil::atomic_write_noreplace(&path, encode(pattern)?.as_bytes())
        .context("publish drum pattern without replacement")?;
    Ok(path)
}

pub fn delete_user(entry: &Entry) -> Result<()> {
    if !entry.user || entry.path.parent() != Some(user_dir().as_path()) {
        bail!("bundled drum patterns cannot be deleted");
    }
    fs::remove_file(&entry.path)?;
    Ok(())
}

pub fn encode(pattern: &DrumPattern) -> Result<String> {
    validate(pattern)?;
    let mut text = format!(
        "SHR-DRUM-PATTERN {VERSION}\nname={}\ngenre={}\nmeter={}\nrows={}\n",
        pattern.name,
        pattern.genre,
        pattern.meter,
        pattern.rows.len()
    );
    for (row, cells) in pattern.rows.iter().enumerate() {
        for (lane, cell) in cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| **cell != Cell::default())
        {
            text.push_str(&format!(
                "cell={row}|{lane}|{}|{}|{}|{}|{}\n",
                note_text(cell.note),
                optional(cell.velocity),
                optional(cell.program),
                optional(cell.gate),
                command_text(cell.command)
            ));
        }
    }
    Ok(text)
}

pub fn decode(text: &str) -> Result<DrumPattern> {
    if text.len() > MAX_BYTES {
        bail!("drum pattern exceeds {MAX_BYTES} bytes");
    }
    let mut lines = text.lines();
    let version = lines
        .next()
        .context("empty drum pattern")?
        .strip_prefix("SHR-DRUM-PATTERN ")
        .context("not an SHR-DAW drum pattern")?
        .parse::<u8>()?;
    if version > VERSION {
        bail!("unsupported drum pattern version {version}");
    }
    let mut name = None;
    let mut genre = None;
    let mut meter = None;
    let mut row_count = None;
    let mut cells = Vec::new();
    for line in lines.filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
        let (key, value) = line.split_once('=').context("invalid drum pattern line")?;
        match key {
            "name" if name.is_none() => name = Some(value.to_owned()),
            "genre" if genre.is_none() => genre = Some(value.to_owned()),
            "meter" if meter.is_none() => meter = Some(value.parse::<u8>()?),
            "rows" if row_count.is_none() => row_count = Some(value.parse::<usize>()?),
            "cell" => cells.push(value.to_owned()),
            "name" | "genre" | "meter" | "rows" => {
                bail!("duplicate drum pattern field {key}")
            }
            _ => bail!("unknown drum pattern field {key}"),
        }
    }
    let row_count = row_count.context("missing rows")?;
    if row_count == 0 || row_count > 256 {
        bail!("drum pattern needs 1..=256 rows");
    }
    let mut pattern = DrumPattern {
        name: name.context("missing name")?,
        genre: genre.unwrap_or_else(|| "Other".into()),
        meter: meter.context("missing meter")?,
        rows: vec![[Cell::default(); LANES_PER_PAGE]; row_count],
    };
    let mut occupied = BTreeSet::new();
    for value in cells {
        let fields = value.split('|').collect::<Vec<_>>();
        if fields.len() != 7 {
            bail!("invalid drum cell");
        }
        let row = fields[0].parse::<usize>()?;
        let lane = fields[1].parse::<usize>()?;
        if !occupied.insert((row, lane)) {
            bail!("duplicate drum cell");
        }
        let cell = pattern
            .rows
            .get_mut(row)
            .and_then(|cells| cells.get_mut(lane))
            .context("drum cell outside pattern")?;
        *cell = Cell {
            note: parse_note(fields[2])?,
            velocity: parse_optional(fields[3])?,
            program: parse_optional(fields[4])?,
            gate: parse_optional(fields[5])?,
            command: parse_command(fields[6])?,
        };
    }
    validate(&pattern)?;
    Ok(pattern)
}

fn validate(pattern: &DrumPattern) -> Result<()> {
    if pattern.name.is_empty()
        || pattern.name.chars().count() > 64
        || pattern.name.chars().any(char::is_control)
    {
        bail!("drum pattern name must contain 1..=64 printable characters");
    }
    if pattern.genre.is_empty()
        || pattern.genre.chars().count() > 24
        || pattern.genre.chars().any(char::is_control)
    {
        bail!("drum pattern genre must contain 1..=24 printable characters");
    }
    if !matches!(pattern.meter, 3 | 4) {
        bail!("drum pattern meter must be 3/4 or 4/4");
    }
    if pattern.rows.is_empty() || pattern.rows.len() > 256 {
        bail!("drum pattern needs 1..=256 rows");
    }
    for cell in pattern.rows.iter().flatten() {
        cell.validate()?;
    }
    Ok(())
}

fn note_text(note: Note) -> String {
    match note {
        Note::Empty => "-".into(),
        Note::On(note) => note.to_string(),
        Note::Off => "off".into(),
    }
}

fn parse_note(value: &str) -> Result<Note> {
    match value {
        "-" => Ok(Note::Empty),
        "off" => Ok(Note::Off),
        _ => {
            let note = value.parse::<u8>()?;
            if note > 127 {
                bail!("drum note outside MIDI range");
            }
            Ok(Note::On(note))
        }
    }
}

fn optional(value: Option<u8>) -> String {
    value.map_or_else(|| "-".into(), |value| value.to_string())
}

fn parse_optional(value: &str) -> Result<Option<u8>> {
    if value == "-" {
        return Ok(None);
    }
    let value = value.parse::<u8>()?;
    if value > 127 {
        bail!("drum cell value outside MIDI range");
    }
    Ok(Some(value))
}

fn command_text(command: Command) -> String {
    match command {
        Command::None => "-".into(),
        Command::Cut(value) => format!("C{value}"),
        Command::Delay(value) => format!("D{value}"),
        Command::Retrigger(value) => format!("R{value}"),
        Command::Tempo(value) => format!("T{value}"),
    }
}

fn parse_command(value: &str) -> Result<Command> {
    if value == "-" {
        return Ok(Command::None);
    }
    let (kind, value) = value.split_at(1);
    match kind {
        "C" => Ok(Command::Cut(value.parse()?)),
        "D" => Ok(Command::Delay(value.parse()?)),
        "R" => Ok(Command::Retrigger(value.parse()?)),
        "T" => Ok(Command::Tempo(value.parse()?)),
        _ => bail!("invalid drum command"),
    }
}

/// Compact authored catalog format. Each lane contains comma-separated step
/// numbers with an optional hit kind suffix: X accent, g ghost, o open hat,
/// c clap, or r rim. One catalog line is one one-bar groove.
fn decode_catalog(text: &str) -> Result<Vec<DrumPattern>> {
    let mut lines = text.lines();
    let version = lines
        .next()
        .context("empty drum catalog")?
        .strip_prefix("SHR-DRUM-CATALOG ")
        .context("not an SHR-DAW drum catalog")?
        .parse::<u8>()?;
    if version > 1 {
        bail!("unsupported drum catalog version {version}");
    }
    let mut patterns = Vec::new();
    for line in lines.filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
        let value = line
            .strip_prefix("pattern=")
            .context("invalid drum catalog line")?;
        let fields = value.split('|').collect::<Vec<_>>();
        if fields.len() != 7 {
            bail!("drum catalog pattern needs genre, meter, name, and four lanes");
        }
        let meter = fields[1].parse::<u8>()?;
        let row_count = match meter {
            3 => 12,
            4 => 16,
            _ => bail!("catalog meter must be 3 or 4"),
        };
        let mut pattern = DrumPattern {
            name: fields[2].to_owned(),
            genre: fields[0].to_owned(),
            meter,
            rows: vec![[Cell::default(); LANES_PER_PAGE]; row_count],
        };
        for (lane, spec) in fields[3..].iter().enumerate() {
            parse_lane_spec(&mut pattern, lane, spec)?;
        }
        validate(&pattern)?;
        patterns.push(pattern);
    }
    let mut identities = BTreeSet::new();
    if patterns
        .iter()
        .any(|pattern| !identities.insert((pattern.genre.clone(), pattern.name.clone())))
    {
        bail!("duplicate genre/name in drum catalog");
    }
    Ok(patterns)
}

fn parse_lane_spec(pattern: &mut DrumPattern, lane: usize, spec: &str) -> Result<()> {
    if spec == "-" {
        return Ok(());
    }
    let mut occupied = BTreeSet::new();
    for hit in spec.split(',') {
        let split = hit
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(hit.len());
        let row = hit[..split].parse::<usize>()?;
        let kind = &hit[split..];
        if row >= pattern.rows.len() || !occupied.insert(row) {
            bail!("duplicate or out-of-range catalog drum step");
        }
        let (note, velocity) = catalog_hit(lane, kind)?;
        pattern.rows[row][lane] = Cell {
            note: Note::On(note),
            velocity: Some(velocity),
            ..Cell::default()
        };
    }
    Ok(())
}

fn catalog_hit(lane: usize, kind: &str) -> Result<(u8, u8)> {
    let base = [36, 38, 42, 37][lane];
    match kind {
        "" => Ok((base, 92)),
        "X" => Ok((base, 118)),
        "g" => Ok((base, 54)),
        "o" if lane == 2 => Ok((46, 98)),
        "c" if lane == 1 => Ok((39, 108)),
        "r" if matches!(lane, 1 | 3) => Ok((37, 82)),
        _ => bail!("invalid catalog drum hit kind {kind:?} for lane {lane}"),
    }
}

/// Expand a one-bar authored groove into a 2/4/8-bar phrase. Alternating bars
/// gain a restrained variation, while phrase ends get a genre-aware fill.
pub fn arrange(pattern: &DrumPattern, target_rows: usize) -> Result<DrumPattern> {
    validate(pattern)?;
    if target_rows < pattern.rows.len() || target_rows % pattern.rows.len() != 0 {
        bail!(
            "{} rows do not expand evenly into {target_rows}",
            pattern.rows.len()
        );
    }
    let bar_rows = pattern.rows.len();
    let bars = target_rows / bar_rows;
    let mut arranged = pattern.clone();
    arranged.name = format!("{} · {target_rows}", pattern.name);
    arranged.rows = (0..target_rows)
        .map(|row| pattern.rows[row % bar_rows])
        .collect();
    for bar in (1..bars).step_by(2) {
        let start = bar * bar_rows;
        let pickup = start + bar_rows - 2;
        if arranged.rows[pickup][0] == Cell::default() {
            arranged.rows[pickup][0] = note_cell(36, 76);
        }
        let open_hat = start + bar_rows - 1;
        if arranged.rows[open_hat][2] == Cell::default() {
            arranged.rows[open_hat][2] = note_cell(46, 72);
        }
    }
    if bars > 1 {
        add_phrase_fill(&mut arranged);
    }
    validate(&arranged)?;
    Ok(arranged)
}

fn add_phrase_fill(pattern: &mut DrumPattern) {
    let end = pattern.rows.len();
    let start = end.saturating_sub(4);
    match pattern.genre.as_str() {
        "House" | "Techno" => {
            for row in start..end {
                pattern.rows[row][1] = note_cell(38, 58 + ((row - start) as u8 * 12));
            }
        }
        "Hip-Hop" | "Funk" => {
            for row in (start..end).step_by(2) {
                if pattern.rows[row][1] == Cell::default() {
                    pattern.rows[row][1] = note_cell(38, 52);
                }
            }
        }
        "Latin" => {
            for row in start..end {
                if pattern.rows[row][3] == Cell::default() {
                    pattern.rows[row][3] = note_cell(37, 64 + ((row - start) as u8 * 8));
                }
            }
        }
        "Jazz" => {
            pattern.rows[start][1] = note_cell(38, 48);
            pattern.rows[end - 2][1] = note_cell(38, 62);
            pattern.rows[end - 1][2] = note_cell(51, 88);
        }
        _ => {
            for (offset, note) in [45, 47, 50, 47].into_iter().enumerate() {
                pattern.rows[start + offset][3] = note_cell(note, 72 + offset as u8 * 10);
                pattern.rows[start + offset][2] = Cell::default();
            }
        }
    }
}

fn note_cell(note: u8, velocity: u8) -> Cell {
    Cell {
        note: Note::On(note),
        velocity: Some(velocity),
        ..Cell::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drum_pattern_round_trips_full_cells() {
        let mut pattern = DrumPattern {
            name: "Test beat".into(),
            genre: "Test".into(),
            meter: 4,
            rows: vec![[Cell::default(); LANES_PER_PAGE]; 16],
        };
        pattern.rows[0][0] = Cell {
            note: Note::On(36),
            velocity: Some(110),
            gate: Some(50),
            command: Command::Retrigger(2),
            ..Cell::default()
        };
        assert_eq!(decode(&encode(&pattern).unwrap()).unwrap(), pattern);
    }

    #[test]
    fn rejects_invalid_or_future_files() {
        assert!(decode("SHR-DRUM-PATTERN 2\nname=x\nmeter=4\nrows=16\n").is_err());
        assert!(decode("SHR-DRUM-PATTERN 1\nname=x\nmeter=5\nrows=16\n").is_err());
    }

    #[test]
    fn bundled_library_contains_common_valid_rhythms() {
        let bundled = discover()
            .into_iter()
            .filter(|entry| !entry.user)
            .collect::<Vec<_>>();
        assert!(bundled.len() >= 60);
        assert!(bundled.iter().any(|entry| entry.name.contains("Rock")));
        assert!(bundled.iter().any(|entry| entry.name.contains("House")));
        assert!(bundled.iter().any(|entry| entry.name.contains("Waltz")));
        assert!(bundled.iter().all(|entry| load(entry).is_ok()));
        assert!(bundled.iter().any(|entry| entry.meter == 3));
        assert!(bundled.iter().any(|entry| entry.meter == 4));
        assert!(
            bundled
                .iter()
                .map(|entry| &entry.genre)
                .collect::<BTreeSet<_>>()
                .len()
                >= 10
        );
    }

    #[test]
    fn arrangement_builds_distinct_supported_phrase_lengths() {
        let seed = discover()
            .into_iter()
            .find(|entry| !entry.user && entry.meter == 4 && entry.genre == "Rock")
            .unwrap();
        let seed = load(&seed).unwrap();
        let short = arrange(&seed, 32).unwrap();
        let medium = arrange(&seed, 64).unwrap();
        let long = arrange(&seed, 128).unwrap();
        assert_eq!(
            (short.rows.len(), medium.rows.len(), long.rows.len()),
            (32, 64, 128)
        );
        assert_ne!(short.rows[16..], seed.rows[..]);
        assert_ne!(medium.rows[60..], seed.rows[12..]);
    }
}
