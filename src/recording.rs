use crate::control::CONTROLS;
use crate::preset::{BackendKind, Preset, PresetId};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const FORMAT_VERSION: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimedEvent {
    pub micros: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Recorder {
    started: Option<Instant>,
    pub events: Vec<TimedEvent>,
}

impl Recorder {
    pub fn start(&mut self, now: Instant) {
        self.events.clear();
        self.started = Some(now);
    }
    pub fn stop(&mut self) {
        self.started = None;
    }
    pub fn is_recording(&self) -> bool {
        self.started.is_some()
    }
    pub fn capture(&mut self, now: Instant, bytes: &[u8]) {
        let Some(start) = self.started else { return };
        let Some(elapsed) = now.checked_duration_since(start) else {
            return;
        };
        if is_musical(bytes) {
            self.events.push(TimedEvent {
                micros: elapsed.as_micros() as u64,
                bytes: bytes.to_vec(),
            });
        }
    }
}

pub fn is_musical(m: &[u8]) -> bool {
    let Some(&status) = m.first() else {
        return false;
    };
    let data_len = match status & 0xf0 {
        0xc0 | 0xd0 => 1,
        0x80 | 0x90 | 0xa0 | 0xb0 | 0xe0 => 2,
        _ => return false,
    };
    m.len() == data_len + 1 && m[1..].iter().all(|byte| *byte <= 127)
}

pub fn all_notes_off() -> Vec<Vec<u8>> {
    (0..16)
        .flat_map(|ch| {
            [
                vec![0xb0 | ch, 64, 0],
                vec![0xb0 | ch, 123, 0],
                vec![0xb0 | ch, 120, 0],
            ]
        })
        .collect()
}

pub fn ideas_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
                .join(".local/share")
        })
        .join("shsynth/ideas")
}

pub fn safe_name(input: &str) -> String {
    let s: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    s.trim_matches('-').chars().take(64).collect::<String>()
}

pub fn list(base: &Path) -> Result<Vec<String>> {
    let mut names = match fs::read_dir(base) {
        Ok(entries) => entries
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| !name.starts_with('.') && safe_name(name) == *name)
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    names.sort_by_key(|x| x.to_lowercase());
    Ok(names)
}

pub fn inspect(base: &Path, name: &str) -> Result<String> {
    fs::read_to_string(idea_dir(base, name)?.join("metadata.json")).map_err(Into::into)
}

#[cfg(test)]
pub fn load(base: &Path, name: &str) -> Result<(Preset, Vec<TimedEvent>)> {
    let (_, preset, events) = load_core(base, name)?;
    Ok((preset, events))
}

pub fn load_with_parameters(
    base: &Path,
    name: &str,
) -> Result<(Preset, HashMap<u8, f32>, Vec<TimedEvent>)> {
    let (dir, preset, events) = load_core(base, name)?;
    let parameters = read_saved_parameters(&dir.join("metadata.json"), preset.backend)?;
    Ok((preset, parameters, events))
}

fn load_core(base: &Path, name: &str) -> Result<(PathBuf, Preset, Vec<TimedEvent>)> {
    let dir = idea_dir(base, name)?;
    let preset = if dir.join("preset.ref").is_file() {
        read_preset_ref(&dir.join("preset.ref"), &dir)?
    } else {
        let path = dir.join("preset.synthv1");
        if !path.is_file() {
            bail!("idea preset snapshot is missing");
        }
        Preset::synthv1(name, path)
    };
    let events = decode_smf(&fs::read(dir.join("recording.mid"))?)?;
    Ok((dir, preset, events))
}

fn read_saved_parameters(path: &Path, backend: BackendKind) -> Result<HashMap<u8, f32>> {
    if backend != BackendKind::Synthv1 || !path.is_file() {
        return Ok(HashMap::new());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("parse idea metadata {}", path.display()))?;
    let Some(parameters) = metadata
        .get("parameters")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(HashMap::new());
    };
    let mut values = HashMap::new();
    for control in CONTROLS {
        let Some(value) = parameters.get(control.xml_name) else {
            continue;
        };
        let value = value
            .as_f64()
            .with_context(|| format!("idea parameter {} is not numeric", control.xml_name))?;
        let value = value as f32;
        if !value.is_finite() || !(control.min..=control.max).contains(&value) {
            bail!(
                "idea parameter {} must be {}..={}",
                control.xml_name,
                control.min,
                control.max
            );
        }
        values.insert(control.cc, value);
    }
    Ok(values)
}

pub fn delete(base: &Path, name: &str) -> Result<()> {
    let path = idea_dir(base, name)?;
    if !path.is_dir() {
        bail!("idea does not exist");
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

pub fn save(
    base: &Path,
    name: &str,
    preset: &Preset,
    values: &HashMap<u8, f32>,
    events: &[TimedEvent],
) -> Result<PathBuf> {
    let name = safe_name(name);
    if name.is_empty() {
        bail!("idea name is empty after sanitizing");
    }
    fs::create_dir_all(base)?;
    let final_dir = base.join(&name);
    if final_dir.exists() {
        bail!("idea '{name}' already exists; choose another name or delete it explicitly");
    }
    if events.windows(2).any(|pair| {
        pair[0].micros > pair[1].micros
            || pair[1].micros / 1000 - pair[0].micros / 1000 > 0x0fff_ffff
    }) || events
        .first()
        .is_some_and(|event| event.micros / 1000 > 0x0fff_ffff)
        || events.iter().any(|event| !is_musical(&event.bytes))
    {
        bail!("idea contains invalid or out-of-order MIDI events");
    }
    let tmp = create_temporary_idea(base, &name)?;
    let result = (|| -> Result<()> {
        write_preset_ref(&tmp.join("preset.ref"), preset)?;
        if let PresetId::Synthv1 { path } = &preset.id {
            fs::copy(path, tmp.join("preset.synthv1"))?;
        }
        fs::write(tmp.join("recording.mid"), encode_smf(events))?;
        let created = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let mut parameters = serde_json::Map::new();
        if preset.backend == BackendKind::Synthv1 {
            for control in CONTROLS {
                let value = values.get(&control.cc).copied().unwrap_or(control.min);
                if !value.is_finite() || !(control.min..=control.max).contains(&value) {
                    bail!(
                        "idea parameter {} must be {}..={}",
                        control.xml_name,
                        control.min,
                        control.max
                    );
                }
                parameters.insert(control.xml_name.into(), serde_json::json!(value));
            }
        }
        let snapshot = matches!(preset.id, PresetId::Synthv1 { .. }).then_some("preset.synthv1");
        let metadata = serde_json::json!({
            "format": "shsynth-idea",
            "version": FORMAT_VERSION,
            "created_unix": created,
            "backend": preset.backend.label(),
            "preset": preset.name,
            "preset_snapshot": snapshot,
            "preset_reference": "preset.ref",
            "midi": "recording.mid",
            "event_count": events.len(),
            "parameters": parameters,
        });
        fs::write(
            tmp.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata)?,
        )?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if let Err(error) = crate::fsutil::rename_noreplace(&tmp, &final_dir) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(error).context("atomically publish idea");
    }
    Ok(final_dir)
}

fn create_temporary_idea(base: &Path, name: &str) -> Result<PathBuf> {
    for sequence in 0..10_000 {
        let path = base.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).with_context(|| format!("create {}", path.display())),
        }
    }
    bail!("too many temporary ideas named {name}")
}

fn idea_dir(base: &Path, name: &str) -> Result<PathBuf> {
    let safe = safe_name(name);
    if safe.is_empty() || safe != name {
        bail!("invalid idea name");
    }
    Ok(base.join(safe))
}

fn write_preset_ref(path: &Path, preset: &Preset) -> Result<()> {
    let (source, extra) = match &preset.id {
        PresetId::Synthv1 { .. } => ("preset.synthv1".to_owned(), String::new()),
        PresetId::Yoshimi { path } => (safe_ref_value(&path.to_string_lossy())?, String::new()),
        PresetId::FluidSynth {
            soundfont,
            soundfont_index,
            bank,
            program,
        } => (
            safe_ref_value(&soundfont.to_string_lossy())?,
            format!("soundfont_index={soundfont_index}\nbank={bank}\nprogram={program}\n"),
        ),
    };
    fs::write(
        path,
        format!(
            "backend={}\nname={}\ncategory={}\npath={}\n{extra}",
            preset.backend.label().to_ascii_lowercase(),
            safe_ref_value(&preset.name)?,
            safe_ref_value(preset.category.as_deref().unwrap_or(""))?,
            source
        ),
    )?;
    Ok(())
}

fn read_preset_ref(path: &Path, idea_dir: &Path) -> Result<Preset> {
    let text = fs::read_to_string(path)?;
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name).map(str::to_owned))
    };
    let backend: BackendKind = field("backend=")
        .context("preset reference has no backend")?
        .parse()?;
    let name = field("name=").context("preset reference has no name")?;
    let category = field("category=").filter(|value| !value.is_empty());
    let source = field("path=").context("preset reference has no path")?;
    let source = if backend == BackendKind::Synthv1 {
        if source != "preset.synthv1" {
            bail!("synthv1 idea must use its private preset snapshot");
        }
        idea_dir.join("preset.synthv1")
    } else {
        PathBuf::from(source)
    };
    if !source.is_file() {
        bail!("idea preset source is missing: {}", source.display());
    }
    let id = match backend {
        BackendKind::Synthv1 => PresetId::Synthv1 { path: source },
        BackendKind::Yoshimi => PresetId::Yoshimi { path: source },
        BackendKind::FluidSynth => PresetId::FluidSynth {
            soundfont: source,
            soundfont_index: field("soundfont_index=")
                .context("FluidSynth reference has no SoundFont index")?
                .parse()?,
            bank: field("bank=")
                .context("FluidSynth reference has no bank")?
                .parse()?,
            program: field("program=")
                .context("FluidSynth reference has no program")?
                .parse()?,
        },
    };
    Ok(Preset {
        backend,
        name,
        category,
        id,
    })
}

fn safe_ref_value(value: &str) -> Result<String> {
    if value.contains(['\n', '\r']) {
        bail!("preset reference contains a newline");
    }
    Ok(value.to_owned())
}

pub fn encode_smf(events: &[TimedEvent]) -> Vec<u8> {
    // SMF defaults to 120 BPM. Declare 60 BPM explicitly so other players use
    // the same millisecond tick interpretation as SHR-DAW.
    const TPQ: u16 = 1000;
    let mut track = vec![0, 0xff, 0x51, 0x03, 0x0f, 0x42, 0x40];
    let mut previous_ms = 0u64;
    for event in events {
        let ms = event.micros / 1000;
        vlq(
            ms.saturating_sub(previous_ms).min(0x0fff_ffff) as u32,
            &mut track,
        );
        previous_ms = previous_ms.max(ms);
        track.extend_from_slice(&event.bytes);
    }
    track.extend_from_slice(&[0, 0xff, 0x2f, 0]);
    let mut out = b"MThd".to_vec();
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&TPQ.to_be_bytes());
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(track.len() as u32).to_be_bytes());
    out.extend(track);
    out
}

fn vlq(mut n: u32, out: &mut Vec<u8>) {
    let mut buf = [0u8; 4];
    let mut i = 3;
    buf[i] = (n & 0x7f) as u8;
    while {
        n >>= 7;
        n != 0
    } {
        i -= 1;
        buf[i] = ((n & 0x7f) as u8) | 0x80;
    }
    out.extend_from_slice(&buf[i..]);
}

pub fn decode_smf(bytes: &[u8]) -> Result<Vec<TimedEvent>> {
    if bytes.len() < 14 || &bytes[..4] != b"MThd" {
        bail!("not a supported MIDI file");
    }
    let header_len = read_u32(bytes, 4)? as usize;
    if header_len < 6 {
        bail!("invalid MIDI header length");
    }
    let header_end = 8usize
        .checked_add(header_len)
        .filter(|end| *end <= bytes.len())
        .context("truncated MIDI header")?;
    let format = read_u16(bytes, 8)?;
    if format != 0 {
        bail!("only single-track MIDI files are supported");
    }
    if read_u16(bytes, 10)? != 1 {
        bail!("single-track MIDI must declare exactly one track");
    }
    let division = u64::from(read_u16(bytes, 12)?);
    if division == 0 || division & 0x8000 != 0 {
        bail!("invalid MIDI division");
    }

    let mut chunk = header_end;
    let (mut p, end) = loop {
        let chunk_header_end = chunk
            .checked_add(8)
            .filter(|end| *end <= bytes.len())
            .context("missing MIDI track")?;
        let len = read_u32(bytes, chunk + 4)? as usize;
        let data_end = chunk_header_end
            .checked_add(len)
            .filter(|end| *end <= bytes.len())
            .context("truncated MIDI track")?;
        if &bytes[chunk..chunk + 4] == b"MTrk" {
            break (chunk_header_end, data_end);
        }
        chunk = data_end;
    };

    let mut elapsed_numerator = 0u128;
    // Older SHR-DAW ideas omitted FF51 but deliberately treated TPQ as 60 BPM.
    let mut tempo_micros = 1_000_000u32;
    let mut running_status = None;
    let mut out = Vec::new();
    while p < end {
        let (delta, n) = read_vlq(&bytes[p..end])?;
        p += n;
        elapsed_numerator = elapsed_numerator
            .checked_add(u128::from(delta) * u128::from(tempo_micros))
            .context("MIDI timing overflow")?;
        if p >= end {
            bail!("truncated MIDI event");
        }

        let first = bytes[p];
        if first == 0xff {
            running_status = None;
            p += 1;
            let kind = *bytes
                .get(p)
                .filter(|_| p < end)
                .context("truncated MIDI meta event")?;
            p += 1;
            let (len, consumed) = read_vlq(&bytes[p..end])?;
            p += consumed;
            let len = len as usize;
            let data_end = p
                .checked_add(len)
                .filter(|data_end| *data_end <= end)
                .context("truncated MIDI meta event")?;
            if kind == 0x51 {
                if len != 3 {
                    bail!("invalid MIDI tempo event");
                }
                tempo_micros = u32::from_be_bytes([0, bytes[p], bytes[p + 1], bytes[p + 2]]);
                if tempo_micros == 0 {
                    bail!("invalid zero MIDI tempo");
                }
            }
            p = data_end;
            if kind == 0x2f {
                break;
            }
            continue;
        }
        if matches!(first, 0xf0 | 0xf7) {
            running_status = None;
            p += 1;
            let (len, consumed) = read_vlq(&bytes[p..end])?;
            p += consumed;
            p = p
                .checked_add(len as usize)
                .filter(|data_end| *data_end <= end)
                .context("truncated MIDI SysEx event")?;
            continue;
        }

        let (status, has_status) = if first & 0x80 != 0 {
            if !(0x80..=0xef).contains(&first) {
                bail!("unsupported MIDI status 0x{first:02x}");
            }
            running_status = Some(first);
            (first, true)
        } else {
            (
                running_status.context("MIDI running status has no prior status")?,
                false,
            )
        };
        let data_len = if matches!(status & 0xf0, 0xc0 | 0xd0) {
            1
        } else {
            2
        };
        if has_status {
            p += 1;
        }
        let data_end = p
            .checked_add(data_len)
            .filter(|data_end| *data_end <= end)
            .context("truncated MIDI channel event")?;
        if bytes[p..data_end].iter().any(|byte| byte & 0x80 != 0) {
            bail!("invalid MIDI channel data byte");
        }
        let mut message = Vec::with_capacity(data_len + 1);
        message.push(status);
        message.extend_from_slice(&bytes[p..data_end]);
        out.push(TimedEvent {
            micros: u64::try_from(elapsed_numerator / u128::from(division))
                .context("MIDI duration is too long")?,
            bytes: message,
        });
        p = data_end;
    }
    Ok(out)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let value = bytes
        .get(offset..offset + 2)
        .context("truncated MIDI integer")?;
    Ok(u16::from_be_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .context("truncated MIDI integer")?;
    Ok(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}
fn read_vlq(b: &[u8]) -> Result<(u32, usize)> {
    let mut v = 0;
    for (i, x) in b.iter().take(4).enumerate() {
        v = (v << 7) | u32::from(x & 0x7f);
        if x & 0x80 == 0 {
            return Ok((v, i + 1));
        }
    }
    bail!("invalid MIDI delta")
}

pub fn play_events<F: FnMut(&[u8])>(
    events: &[TimedEvent],
    mut send: F,
    stop: &std::sync::atomic::AtomicBool,
) {
    let start = Instant::now();
    for e in events {
        while start.elapsed() < Duration::from_micros(e.micros) {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        send(&e.bytes);
    }
    for m in all_notes_off() {
        send(&m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_and_smf_round_trip() {
        let t = Instant::now();
        let mut r = Recorder::default();
        r.start(t);
        r.capture(t - Duration::from_millis(1), &[0x90, 59, 99]);
        r.capture(t + Duration::from_millis(25), &[0x90, 60, 99]);
        r.capture(t + Duration::from_millis(100), &[0x80, 60, 0]);
        assert_eq!(decode_smf(&encode_smf(&r.events)).unwrap(), r.events);
    }
    #[test]
    fn cleanup_has_all_channels() {
        let m = all_notes_off();
        assert_eq!(m.len(), 48);
        for channel in 0..16 {
            assert_eq!(
                &m[channel * 3..channel * 3 + 3],
                &[
                    vec![0xb0 | channel as u8, 64, 0],
                    vec![0xb0 | channel as u8, 123, 0],
                    vec![0xb0 | channel as u8, 120, 0],
                ]
            );
        }
    }

    #[test]
    fn smf_declares_sixty_bpm_for_external_players() {
        let encoded = encode_smf(&[TimedEvent {
            micros: 1_000,
            bytes: vec![0x90, 60, 100],
        }]);
        assert!(encoded
            .windows(7)
            .any(|window| window == [0, 0xff, 0x51, 3, 0x0f, 0x42, 0x40]));
        assert_eq!(
            decode_smf(&encoded).unwrap()[0],
            TimedEvent {
                micros: 1_000,
                bytes: vec![0x90, 60, 100]
            }
        );
    }

    #[test]
    fn legacy_ideas_without_tempo_keep_the_original_sixty_bpm_timing() {
        let track = [1, 0x90, 60, 100, 0, 0xff, 0x2f, 0];
        let mut encoded = b"MThd".to_vec();
        encoded.extend_from_slice(&6u32.to_be_bytes());
        encoded.extend_from_slice(&0u16.to_be_bytes());
        encoded.extend_from_slice(&1u16.to_be_bytes());
        encoded.extend_from_slice(&1000u16.to_be_bytes());
        encoded.extend_from_slice(b"MTrk");
        encoded.extend_from_slice(&(track.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&track);
        assert_eq!(decode_smf(&encoded).unwrap()[0].micros, 1_000);

        encoded[10..12].copy_from_slice(&2u16.to_be_bytes());
        assert!(decode_smf(&encoded).is_err());
    }

    #[test]
    fn malformed_track_markers_and_lengths_return_errors_without_panicking() {
        let mut near_eof = vec![0; 22];
        near_eof[..4].copy_from_slice(b"MThd");
        near_eof[4..8].copy_from_slice(&6u32.to_be_bytes());
        near_eof[10..12].copy_from_slice(&1u16.to_be_bytes());
        near_eof[12..14].copy_from_slice(&1000u16.to_be_bytes());
        near_eof[18..22].copy_from_slice(b"MTrk");
        assert!(decode_smf(&near_eof).is_err());

        let mut truncated = encode_smf(&[]);
        truncated.pop();
        assert!(decode_smf(&truncated).is_err());
    }
    #[test]
    fn filenames_are_safe_and_overwrite_is_refused() {
        assert_eq!(safe_name(" My idea/one "), "My-idea-one");
        let base = std::env::temp_dir().join(format!("shsynth-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("taken")).unwrap();
        let preset_path = base.join("p.synthv1");
        fs::write(&preset_path, "preset").unwrap();
        let preset = Preset::synthv1("p", preset_path);
        assert!(save(&base, "taken", &preset, &HashMap::new(), &[]).is_err());
        assert!(inspect(&base, "../taken").is_err());
        assert!(delete(&base, "../taken").is_err());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn listing_and_save_preserve_unowned_or_invalid_directories() {
        let base = std::env::temp_dir().join(format!("shsynth-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::create_dir(base.join("valid")).unwrap();
        std::os::unix::fs::symlink(base.join("valid"), base.join("alias")).unwrap();
        fs::create_dir(base.join("not valid")).unwrap();
        assert_eq!(list(&base).unwrap(), ["valid"]);

        let stale = base.join(format!(".fresh.tmp-{}-0", std::process::id()));
        fs::create_dir(&stale).unwrap();
        fs::write(stale.join("keep"), b"unfinished").unwrap();
        let preset_path = base.join("p.synthv1");
        fs::write(&preset_path, "preset").unwrap();
        let preset = Preset::synthv1("p", preset_path);
        save(&base, "fresh", &preset, &HashMap::new(), &[]).unwrap();
        assert_eq!(fs::read(stale.join("keep")).unwrap(), b"unfinished");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn invalid_or_unrepresentable_events_are_not_saved() {
        let base = std::env::temp_dir().join(format!("shsynth-events-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let preset_path = base.join("p.synthv1");
        fs::write(&preset_path, "preset").unwrap();
        let preset = Preset::synthv1("p", preset_path);
        let event = |micros, bytes| TimedEvent { micros, bytes };

        assert!(save(
            &base,
            "bad-data",
            &preset,
            &HashMap::new(),
            &[event(0, vec![0x90, 128, 100])],
        )
        .is_err());
        assert!(save(
            &base,
            "backwards",
            &preset,
            &HashMap::new(),
            &[
                event(2_000, vec![0x90, 60, 100]),
                event(1_000, vec![0x80, 60, 0]),
            ],
        )
        .is_err());
        assert!(save(
            &base,
            "too-long",
            &preset,
            &HashMap::new(),
            &[event(
                (u64::from(0x0fff_ffff_u32) + 1) * 1000,
                vec![0x90, 60, 100],
            )],
        )
        .is_err());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn backend_identity_round_trips_without_copying_external_sound_data() {
        let base =
            std::env::temp_dir().join(format!("shsynth-idea-backend-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let instrument = base.join("system-instrument.xiz");
        fs::write(&instrument, "external").unwrap();
        let preset = Preset {
            backend: BackendKind::Yoshimi,
            name: "External Bass".into(),
            category: Some("Bass".into()),
            id: PresetId::Yoshimi {
                path: instrument.clone(),
            },
        };
        let saved = save(&base, "idea", &preset, &HashMap::new(), &[]).unwrap();
        assert!(!saved.join("preset.synthv1").exists());
        let (loaded, _) = load(&base, "idea").unwrap();
        assert_eq!(loaded, preset);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn synth_idea_load_returns_validated_saved_control_values() {
        let base = std::env::temp_dir().join(format!("shsynth-idea-values-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let preset_path = base.join("source.synthv1");
        fs::write(&preset_path, "preset").unwrap();
        let preset = Preset::synthv1("Saved sound", preset_path);
        let values = CONTROLS
            .iter()
            .map(|control| (control.cc, (control.min + control.max) * 0.5))
            .collect::<HashMap<_, _>>();
        save(&base, "saved", &preset, &values, &[]).unwrap();

        let (loaded, restored, events) = load_with_parameters(&base, "saved").unwrap();
        assert_eq!(loaded.name, preset.name);
        assert_eq!(events, Vec::<TimedEvent>::new());
        assert_eq!(restored.len(), CONTROLS.len());
        for control in CONTROLS {
            assert!((restored[&control.cc] - values[&control.cc]).abs() < 0.000_01);
        }
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn older_metadata_without_parameters_remains_loadable_and_json_is_escaped() {
        let base = std::env::temp_dir().join(format!("shsynth-idea-legacy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let preset_path = base.join("source.synthv1");
        fs::write(&preset_path, "preset").unwrap();
        let preset = Preset::synthv1("Tabbed\tName", preset_path);
        let saved = save(&base, "legacy", &preset, &HashMap::new(), &[]).unwrap();
        let metadata_path = saved.join("metadata.json");
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata["preset"], "Tabbed\tName");
        metadata.as_object_mut().unwrap().remove("parameters");
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();

        let (_, parameters, _) = load_with_parameters(&base, "legacy").unwrap();
        assert!(parameters.is_empty());
        let _ = fs::remove_dir_all(base);
    }
}
