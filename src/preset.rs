use crate::config::RuntimeConfig;
use crate::control::{defaults, CONTROLS};
use anyhow::{bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use quick_xml::XmlVersion;
#[cfg(test)]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BackendKind {
    Synthv1,
    Yoshimi,
    FluidSynth,
}

impl BackendKind {
    pub const ALL: [Self; 3] = [Self::Synthv1, Self::Yoshimi, Self::FluidSynth];

    pub fn label(self) -> &'static str {
        match self {
            Self::Synthv1 => "synthv1",
            Self::Yoshimi => "Yoshimi",
            Self::FluidSynth => "FluidSynth",
        }
    }

    pub fn next(self, direction: i8) -> Self {
        let index = Self::ALL.iter().position(|kind| *kind == self).unwrap_or(0);
        Self::ALL
            [(index as isize + direction as isize).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::str::FromStr for BackendKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "synthv1" | "synth" => Ok(Self::Synthv1),
            "yoshimi" => Ok(Self::Yoshimi),
            "fluidsynth" | "fluid" => Ok(Self::FluidSynth),
            _ => bail!("unknown sound engine {value:?}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PresetId {
    Synthv1 {
        path: PathBuf,
    },
    Yoshimi {
        path: PathBuf,
    },
    FluidSynth {
        soundfont: PathBuf,
        soundfont_index: u8,
        bank: u16,
        program: u8,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preset {
    pub backend: BackendKind,
    pub name: String,
    pub category: Option<String>,
    pub id: PresetId,
}

impl Preset {
    pub fn synthv1(name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            backend: BackendKind::Synthv1,
            name: name.into(),
            category: None,
            id: PresetId::Synthv1 { path },
        }
    }

    pub fn display_name(&self) -> String {
        if self.backend == BackendKind::FluidSynth {
            return self.name.clone();
        }
        self.category
            .as_ref()
            .map(|category| format!("[{category}] {}", self.name))
            .unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub backend: BackendKind,
    pub presets: Vec<Preset>,
    pub unavailable: Option<String>,
}

pub fn discover_all(config: &RuntimeConfig, synthv1_dir: &Path) -> Vec<Catalog> {
    vec![
        catalog(
            BackendKind::Synthv1,
            command_exists(&config.synth_command),
            discover_synthv1(synthv1_dir),
            &config.synth_command,
        ),
        catalog(
            BackendKind::Yoshimi,
            command_exists(&config.yoshimi.backend.command),
            discover_yoshimi(
                &config.yoshimi.backend.preset_roots,
                &config.yoshimi.categories,
                config.yoshimi.presets_per_category,
            ),
            &config.yoshimi.backend.command,
        ),
        catalog(
            BackendKind::FluidSynth,
            command_exists(&config.fluidsynth.backend.command),
            discover_fluidsynth(&config.fluidsynth.soundfonts),
            &config.fluidsynth.backend.command,
        ),
    ]
}

fn catalog(
    backend: BackendKind,
    executable_exists: bool,
    discovered: Result<Vec<Preset>>,
    command: &str,
) -> Catalog {
    let (presets, unavailable) = match discovered {
        Ok(presets) if !executable_exists => (
            presets,
            Some(format!(
                "{} executable not found: {command}",
                backend.label()
            )),
        ),
        Ok(presets) if presets.is_empty() => (
            presets,
            Some(format!("no configured {} sounds found", backend.label())),
        ),
        Ok(presets) => (presets, None),
        Err(error) => (Vec::new(), Some(format!("{error:#}"))),
    };
    Catalog {
        backend,
        presets,
        unavailable,
    }
}

fn command_exists(program: &str) -> bool {
    crate::fsutil::command_exists(program)
}

pub fn discover_synthv1(dir: &Path) -> Result<Vec<Preset>> {
    let mut presets = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if extension_is(&path, "synthv1") {
            let name = file_stem(&path);
            presets.push(Preset::synthv1(name, path));
        }
    }
    sort_presets(&mut presets);
    Ok(presets)
}

pub fn discover_yoshimi(
    roots: &[PathBuf],
    categories: &[String],
    per_category: usize,
) -> Result<Vec<Preset>> {
    let mut grouped: BTreeMap<String, Vec<Preset>> = BTreeMap::new();
    for root in roots.iter().filter(|root| root.is_dir()) {
        let mut files = Vec::new();
        recursive_files(root, "xiz", &mut files)?;
        for path in files {
            let searchable = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_ascii_lowercase();
            let Some(category) = categories
                .iter()
                .find(|category| category_matches(category, &searchable))
                .cloned()
            else {
                continue;
            };
            grouped.entry(category.clone()).or_default().push(Preset {
                backend: BackendKind::Yoshimi,
                name: clean_numbered_name(&file_stem(&path)),
                category: Some(title_case(&category)),
                id: PresetId::Yoshimi { path },
            });
        }
    }
    let mut presets = Vec::new();
    for (_, mut group) in grouped {
        sort_presets(&mut group);
        group.truncate(per_category);
        presets.extend(group);
    }
    sort_presets(&mut presets);
    Ok(presets)
}

fn category_matches(category: &str, text: &str) -> bool {
    match category {
        "bass" => ["bass", "sub", "acid"]
            .iter()
            .any(|word| text.contains(word)),
        "lead" => ["lead", "solo", "saw"]
            .iter()
            .any(|word| text.contains(word)),
        "pad" => ["pad", "string", "choir"]
            .iter()
            .any(|word| text.contains(word)),
        "pluck" => ["pluck", "harp", "guitar"]
            .iter()
            .any(|word| text.contains(word)),
        "bell" => ["bell", "tine", "mallet"]
            .iter()
            .any(|word| text.contains(word)),
        "organ" => text.contains("organ"),
        "drone" => ["drone", "atmos", "ambient"]
            .iter()
            .any(|word| text.contains(word)),
        "keys" => ["piano", "rhodes", "keys"]
            .iter()
            .any(|word| text.contains(word)),
        other => text.contains(other),
    }
}

pub fn discover_fluidsynth(soundfonts: &[PathBuf]) -> Result<Vec<Preset>> {
    let mut presets = Vec::new();
    let mut valid_fonts = 0usize;
    let mut failures = Vec::new();
    for path in soundfonts.iter().filter(|path| path.is_file()) {
        if valid_fonts > u8::MAX as usize {
            break;
        }
        let font_name = file_stem(path);
        let programs = match soundfont_presets(path) {
            Ok(programs) => programs,
            Err(error) => {
                failures.push(format!("{error:#}"));
                continue;
            }
        };
        let index = valid_fonts as u8;
        valid_fonts += 1;
        for program in programs {
            presets.push(Preset {
                backend: BackendKind::FluidSynth,
                name: program.name,
                category: Some(format!(
                    "{font_name} {:03}:{:03}",
                    program.bank, program.program
                )),
                id: PresetId::FluidSynth {
                    soundfont: path.clone(),
                    soundfont_index: index,
                    bank: program.bank,
                    program: program.program,
                },
            });
        }
    }
    if valid_fonts == 0 && !failures.is_empty() {
        bail!("no valid configured SoundFonts: {}", failures.join("; "));
    }
    presets.sort_by(|a, b| match (&a.id, &b.id) {
        (
            PresetId::FluidSynth {
                soundfont_index: ai,
                bank: ab,
                program: ap,
                ..
            },
            PresetId::FluidSynth {
                soundfont_index: bi,
                bank: bb,
                program: bp,
                ..
            },
        ) => (ai, ab, ap).cmp(&(bi, bb, bp)),
        _ => a.name.cmp(&b.name),
    });
    Ok(presets)
}

#[derive(Debug, Eq, PartialEq)]
struct SoundFontProgram {
    name: String,
    bank: u16,
    program: u8,
}

fn soundfont_presets(path: &Path) -> Result<Vec<SoundFontProgram>> {
    const MAX_PRESET_TABLE_BYTES: u64 = 16 * 1024 * 1024;
    let mut file =
        File::open(path).with_context(|| format!("read SoundFont {}", path.display()))?;
    let file_len = file.metadata()?.len();
    let mut header = [0u8; 12];
    file.read_exact(&mut header)
        .with_context(|| format!("read SoundFont header {}", path.display()))?;
    if &header[..4] != b"RIFF" || &header[8..12] != b"sfbk" {
        bail!("{} is not an SF2/SF3 SoundFont", path.display());
    }
    let riff_end = u64::from(le_u32(&header[4..8]))
        .checked_add(8)
        .context("SoundFont RIFF length overflow")?;
    if riff_end < 12 || riff_end > file_len {
        bail!("{} has a truncated RIFF container", path.display());
    }
    let mut phdr = None::<Vec<u8>>;
    let mut offset = 12u64;
    while offset + 8 <= riff_end {
        let (id, size) = read_chunk_header(&mut file, offset)?;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .context("SoundFont chunk length overflow")?;
        if end > riff_end {
            bail!("{} has a truncated RIFF chunk", path.display());
        }
        if &id == b"LIST" && size >= 4 {
            let mut inner = start + 4;
            while inner + 8 <= end {
                let (inner_id, inner_size) = read_chunk_header(&mut file, inner)?;
                let data = inner + 8;
                let data_end = data
                    .checked_add(inner_size)
                    .context("SoundFont subchunk length overflow")?;
                if data_end > end {
                    bail!("{} has a truncated RIFF subchunk", path.display());
                }
                if &inner_id == b"phdr" {
                    if inner_size > MAX_PRESET_TABLE_BYTES {
                        bail!("{} has an oversized preset table", path.display());
                    }
                    let mut table = vec![0; inner_size as usize];
                    file.seek(SeekFrom::Start(data))?;
                    file.read_exact(&mut table)?;
                    phdr = Some(table);
                    break;
                }
                inner = padded_chunk_end(data_end, inner_size)?;
            }
        }
        if phdr.is_some() {
            break;
        }
        offset = padded_chunk_end(end, size)?;
    }
    let phdr =
        phdr.with_context(|| format!("{} has no SoundFont preset headers", path.display()))?;
    if phdr.len() < 38 || phdr.len() % 38 != 0 {
        bail!("{} has a malformed SoundFont preset table", path.display());
    }
    let count = phdr.len() / 38;
    let mut programs = Vec::new();
    for record in phdr.chunks_exact(38).take(count.saturating_sub(1)) {
        let name = nul_string(&record[..20]);
        let program = u16::from_le_bytes([record[20], record[21]]);
        let bank = u16::from_le_bytes([record[22], record[23]]);
        if program <= 127 {
            programs.push(SoundFontProgram {
                name,
                bank,
                program: program as u8,
            });
        }
    }
    Ok(programs)
}

pub(crate) fn soundfont_offsets(soundfonts: &[PathBuf]) -> Result<Vec<(PathBuf, u16)>> {
    let mut fonts = Vec::new();
    let mut next_offset = 0u32;
    for path in soundfonts.iter().filter(|path| path.is_file()) {
        let Ok(programs) = soundfont_presets(path) else {
            continue;
        };
        let max_bank = programs.iter().map(|preset| preset.bank).max().unwrap_or(0);
        if next_offset + u32::from(max_bank) > 16_383 {
            bail!("configured SoundFont banks exceed the 14-bit MIDI bank range");
        }
        fonts.push((path.clone(), next_offset as u16));
        next_offset += u32::from(max_bank) + 1;
    }
    Ok(fonts)
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

fn read_chunk_header(file: &mut File, offset: u64) -> Result<([u8; 4], u64)> {
    let mut header = [0u8; 8];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut header)?;
    Ok((
        header[..4].try_into().unwrap(),
        u64::from(le_u32(&header[4..])),
    ))
}

fn padded_chunk_end(end: u64, size: u64) -> Result<u64> {
    end.checked_add(size & 1)
        .context("SoundFont chunk padding overflow")
}

fn nul_string(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

fn recursive_files(root: &Path, extension: &str, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            recursive_files(&path, extension, files)?;
        } else if (file_type.is_file() || (file_type.is_symlink() && path.is_file()))
            && extension_is(&path, extension)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn extension_is(path: &Path, wanted: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted))
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn clean_numbered_name(name: &str) -> String {
    name.trim_start_matches(|character: char| character.is_ascii_digit())
        .trim_start_matches(['-', '_', ' '])
        .replace(['_', '-'], " ")
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn sort_presets(presets: &mut [Preset]) {
    presets.sort_by_key(|preset| {
        format!(
            "{} {}",
            preset.category.as_deref().unwrap_or_default(),
            preset.name
        )
        .to_ascii_lowercase()
    });
}

pub fn values(preset: &Preset) -> Result<HashMap<u8, f32>> {
    let PresetId::Synthv1 { path } = &preset.id else {
        return Ok(HashMap::new());
    };
    let mut out = defaults();
    let mut reader =
        Reader::from_file(path).with_context(|| format!("parse {}", path.display()))?;
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut wanted = None;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == b"param" => {
                let mut name = None;
                for attribute in e.attributes() {
                    let attribute = attribute?;
                    if attribute.key.as_ref() == b"name" {
                        name = Some(
                            attribute
                                .normalized_value(XmlVersion::Implicit1_0)?
                                .into_owned(),
                        );
                    }
                }
                wanted = name
                    .as_deref()
                    .and_then(|name| CONTROLS.iter().find(|c| c.xml_name == name).map(|c| c.cc));
            }
            Event::Text(e) if wanted.is_some() => {
                let cc = wanted.take().context("mapped preset parameter missing")?;
                let decoded = e.xml_content(XmlVersion::Implicit1_0)?;
                let value = quick_xml::escape::unescape(&decoded)?
                    .parse::<f32>()
                    .with_context(|| format!("mapped preset CC {cc} is not numeric"))?;
                let control = crate::control::by_cc(cc).context("unknown mapped preset CC")?;
                if !value.is_finite() || !(control.min..=control.max).contains(&value) {
                    bail!(
                        "preset parameter {} must be {}..={}",
                        control.xml_name,
                        control.min,
                        control.max
                    );
                }
                out.insert(cc, value);
            }
            Event::End(e) if e.name().as_ref() == b"param" => wanted = None,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

pub fn resolve<'a>(presets: &'a [Preset], arg: &str) -> Option<&'a Preset> {
    if let Some(number) = arg
        .strip_prefix("preset_")
        .and_then(|value| value.parse::<usize>().ok())
    {
        return number.checked_sub(1).and_then(|index| presets.get(index));
    }
    presets.iter().find(|preset| {
        preset.name.eq_ignore_ascii_case(arg)
            || format!("{}:{}", preset.backend.label(), preset.name).eq_ignore_ascii_case(arg)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_legacy_xml_by_name_not_obsolete_index() {
        let path = std::env::temp_dir().join(format!(
            "shsynth-legacy-preset-{}.synthv1",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"<preset><params>
                <param index="999" name="DCF1_CUTOFF">0.19</param>
                <param index="0" name="DEL1_WET">1.0</param>
            </params></preset>"#,
        )
        .unwrap();
        let preset = Preset::synthv1("Legacy fixture", path.clone());
        let values = values(&preset).unwrap();
        assert!((values[&74] - 0.19).abs() < 0.0001);
        assert!((values[&18] - 1.0).abs() < 0.0001);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn mapped_preset_values_must_be_finite_and_in_range() {
        let path = std::env::temp_dir().join(format!(
            "shsynth-invalid-value-{}.synthv1",
            std::process::id()
        ));
        let preset = Preset::synthv1("Invalid fixture", path.clone());
        for value in ["NaN", "1.5", "not-a-number"] {
            fs::write(
                &path,
                format!(
                    "<preset><params><param name=\"DCF1_CUTOFF\">{value}</param></params></preset>"
                ),
            )
            .unwrap();
            assert!(values(&preset).is_err(), "accepted {value}");
        }
        fs::write(
            &path,
            r#"<preset><params><param name="DCF1_CUTOFF" name="DEL1_WET">0.5</param></params></preset>"#,
        )
        .unwrap();
        assert!(values(&preset).is_err(), "accepted duplicate attributes");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn engine_cycle_wraps_in_both_directions() {
        assert_eq!(BackendKind::Synthv1.next(-1), BackendKind::FluidSynth);
        assert_eq!(BackendKind::FluidSynth.next(1), BackendKind::Synthv1);
    }

    #[test]
    fn fluidsynth_display_hides_font_bank_metadata() {
        let preset = Preset {
            backend: BackendKind::FluidSynth,
            name: "Warm Pad".into(),
            category: Some("TimGM6mb 002:009".into()),
            id: PresetId::FluidSynth {
                soundfont: "/sounds/tim.sf2".into(),
                soundfont_index: 0,
                bank: 2,
                program: 9,
            },
        };
        assert_eq!(preset.display_name(), "Warm Pad");
    }

    #[test]
    fn yoshimi_discovery_is_recursive_curated_and_bounded() {
        let base = std::env::temp_dir().join(format!("shsynth-yoshimi-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("bank")).unwrap();
        for name in [
            "0001-Fat_Bass.xiz",
            "0002-Soft Bass.xiz",
            "0003-Random Flute.xiz",
        ] {
            fs::write(base.join("bank").join(name), "x").unwrap();
        }
        let presets = discover_yoshimi(&[base.clone()], &["bass".into()], 1).unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].backend, BackendKind::Yoshimi);
        assert_eq!(presets[0].category.as_deref(), Some("Bass"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn yoshimi_category_ignores_parent_directory_names_and_symlink_loops() {
        let base = std::env::temp_dir().join(format!("shsynth-bass-parent-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("bank")).unwrap();
        fs::write(base.join("bank/0001-Bright_Lead.xiz"), "x").unwrap();
        std::os::unix::fs::symlink(&base, base.join("bank/loop")).unwrap();

        let presets =
            discover_yoshimi(&[base.clone()], &["bass".into(), "lead".into()], 8).unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].category.as_deref(), Some("Lead"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn executable_discovery_checks_permission_bits() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!("shsynth-command-{}", std::process::id()));
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!command_exists(path.to_str().unwrap()));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(command_exists(path.to_str().unwrap()));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn parses_soundfont_bank_and_program_headers() {
        let path = std::env::temp_dir().join(format!("shsynth-{}.sf2", std::process::id()));
        let record = |name: &str, program: u16, bank: u16| {
            let mut out = vec![0; 38];
            out[..name.len()].copy_from_slice(name.as_bytes());
            out[20..22].copy_from_slice(&program.to_le_bytes());
            out[22..24].copy_from_slice(&bank.to_le_bytes());
            out
        };
        let mut phdr = record("Warm Pad", 9, 2);
        phdr.extend(record("EOP", 0, 0));
        let mut pdta = b"pdta".to_vec();
        pdta.extend_from_slice(b"phdr");
        pdta.extend_from_slice(&(phdr.len() as u32).to_le_bytes());
        pdta.extend(phdr);
        let mut riff = b"RIFF".to_vec();
        riff.extend_from_slice(&((4 + 8 + pdta.len()) as u32).to_le_bytes());
        riff.extend_from_slice(b"sfbkLIST");
        riff.extend_from_slice(&(pdta.len() as u32).to_le_bytes());
        riff.extend(pdta);
        fs::write(&path, riff).unwrap();
        assert_eq!(
            soundfont_presets(&path).unwrap(),
            [SoundFontProgram {
                name: "Warm Pad".into(),
                bank: 2,
                program: 9
            }]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cleared_presets_use_complete_current_schema() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets/synthv1");
        let manifest = fs::read_to_string(root.join("cleared-presets.txt")).unwrap();
        let expected = manifest
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let actual = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                name.ends_with(".synthv1").then_some(name)
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected, "cleared preset manifest is stale");
        assert_eq!(expected.len(), 21);

        let template = schema(&root.join("Velvet Tines.synthv1"));
        assert_eq!(template.len(), 145);
        for filename in expected {
            let name = filename.trim_end_matches(".synthv1");
            let path = root.join(&filename);
            assert_eq!(schema(&path), template, "schema mismatch in {filename}");
            assert_eq!(values(&Preset::synthv1(name, path)).unwrap().len(), 12);
        }
    }

    fn schema(path: &Path) -> Vec<(u16, String)> {
        let mut reader = Reader::from_file(path).unwrap();
        let mut buffer = Vec::new();
        let mut schema = Vec::new();
        loop {
            match reader.read_event_into(&mut buffer).unwrap() {
                Event::Start(element) if element.name().as_ref() == b"param" => {
                    let mut index = None;
                    let mut name = None;
                    for attribute in element.attributes().flatten() {
                        if attribute.key.as_ref() == b"index" {
                            index = std::str::from_utf8(&attribute.value)
                                .ok()
                                .and_then(|value| value.parse().ok());
                        } else if attribute.key.as_ref() == b"name" {
                            name = std::str::from_utf8(&attribute.value)
                                .ok()
                                .map(str::to_owned);
                        }
                    }
                    schema.push((index.unwrap(), name.unwrap()));
                }
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
        schema.sort();
        schema
    }
}
