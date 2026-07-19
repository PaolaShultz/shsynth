use crate::sequencer::{self, Note, PageTarget};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
struct Manifest {
    schema: u8,
    policy: String,
    demos: Vec<Demo>,
}

#[derive(Deserialize)]
struct Demo {
    id: String,
    title: String,
    bpm: u16,
    time_signature: String,
    key: String,
    tracks: Vec<String>,
    description: String,
    style_ideas: Vec<String>,
    public_domain_reasoning: String,
    sources: Vec<String>,
    midi: String,
    project: String,
    sha256: Hashes,
}

#[derive(Deserialize)]
struct Hashes {
    midi: String,
    project: String,
}

fn validate_midi(data: &[u8], expected_tracks: usize) -> Result<()> {
    if data.len() < 14 || &data[..4] != b"MThd" || u32::from_be_bytes(data[4..8].try_into()?) != 6 {
        bail!("invalid Standard MIDI header");
    }
    if u16::from_be_bytes(data[8..10].try_into()?) != 1
        || usize::from(u16::from_be_bytes(data[10..12].try_into()?)) != expected_tracks + 1
        || u16::from_be_bytes(data[12..14].try_into()?) == 0
    {
        bail!("demo MIDI must be format 1 with conductor plus declared parts");
    }
    let mut offset = 14;
    let mut chunks = 0;
    while offset < data.len() {
        if data.get(offset..offset + 4) != Some(b"MTrk") {
            bail!("invalid MIDI track chunk");
        }
        let length = usize::try_from(u32::from_be_bytes(
            data.get(offset + 4..offset + 8)
                .context("truncated MIDI track length")?
                .try_into()?,
        ))?;
        let end = offset
            .checked_add(8)
            .and_then(|start| start.checked_add(length))
            .context("MIDI track length overflow")?;
        let body = data.get(offset + 8..end).context("truncated MIDI track")?;
        if !body.ends_with(&[0xFF, 0x2F, 0]) {
            bail!("MIDI track lacks an end marker");
        }
        chunks += 1;
        offset = end;
    }
    if chunks != expected_tracks + 1 {
        bail!("MIDI chunk count does not match its header");
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_demo_dir(root: &Path) -> Result<()> {
    let manifest_path = root.join("cleared-demos.json");
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.schema != 1 || !manifest.policy.contains("Only files named here") {
        bail!("unknown or non-explicit demo manifest policy");
    }
    if manifest.demos.is_empty() {
        bail!("demo manifest is empty");
    }

    let mut ids = BTreeSet::new();
    let mut expected = BTreeSet::from([PathBuf::from("cleared-demos.json")]);
    for demo in manifest.demos {
        if !ids.insert(demo.id.clone())
            || demo.id.is_empty()
            || demo.id.starts_with('-')
            || demo.id.ends_with('-')
            || demo.id.contains("--")
            || !demo
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || demo.title.trim().is_empty()
        {
            bail!("duplicate demo id or empty title");
        }
        if demo.midi != format!("{}.mid", demo.id) || demo.project != format!("{}.shsong", demo.id)
        {
            bail!("demo paths must be safe and derived from their id");
        }
        if !(4..=8).contains(&demo.tracks.len())
            || !demo.tracks.iter().any(|track| track == "Drums")
            || !demo.tracks.iter().any(|track| track == "Bass")
            || !demo.tracks.iter().any(|track| track == "Pad")
            || !demo.tracks.iter().any(|track| track == "Lead")
        {
            bail!("demo lacks the required useful parts");
        }
        if demo.bpm < 20
            || demo.time_signature.trim().is_empty()
            || demo.key.trim().is_empty()
            || demo.description.trim().is_empty()
            || demo.style_ideas.is_empty()
            || demo.public_domain_reasoning.len() < 80
            || demo.sources.is_empty()
            || demo
                .sources
                .iter()
                .any(|source| !source.starts_with("https://"))
            || !is_sha256(&demo.sha256.midi)
            || !is_sha256(&demo.sha256.project)
        {
            bail!("demo metadata or provenance is incomplete");
        }

        let midi = fs::read(root.join(&demo.midi))?;
        validate_midi(&midi, demo.tracks.len())
            .with_context(|| format!("invalid demo MIDI: {}", demo.id))?;
        let project_text = fs::read_to_string(root.join(&demo.project))?;
        if !project_text.starts_with(&format!("SHSYNTH-SONG {}\n", sequencer::SONG_VERSION)) {
            bail!("demo Project is not saved in the current format");
        }
        let song = sequencer::decode(&project_text)
            .with_context(|| format!("invalid native demo Project: {}", demo.id))?;
        if song.name != demo.title || song.patterns.len() != 1 {
            bail!("native demo title or pattern count differs from its manifest");
        }
        let pattern = song
            .patterns
            .values()
            .next()
            .context("missing demo pattern")?;
        if pattern.tempo != demo.bpm || pattern.pages.len() != demo.tracks.len() {
            bail!("native demo tempo or page count differs from its manifest");
        }
        for (page, declared) in pattern.pages.iter().zip(&demo.tracks) {
            if !matches!(page.target, PageTarget::Default)
                || page.name != declared.to_ascii_uppercase()
                || page
                    .columns
                    .iter()
                    .any(|column| *column != Default::default())
                || !page.setup.is_empty()
            {
                bail!("demo Projects must use canonical portable routing");
            }
        }
        if !pattern
            .rows
            .iter()
            .flatten()
            .any(|cell| matches!(cell.note, Note::On(_)))
        {
            bail!("native demo contains no notes");
        }
        expected.insert(PathBuf::from(&demo.midi));
        expected.insert(PathBuf::from(&demo.project));
    }

    let present = fs::read_dir(root)?
        .map(|entry| Ok(PathBuf::from(entry?.file_name())))
        .collect::<Result<BTreeSet<_>>>()?;
    if present != expected {
        bail!("demo directory contains missing or unlisted files");
    }
    Ok(())
}

#[test]
fn bundled_demo_manifest_is_complete_current_and_loadable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let generator = Command::new("python3")
        .arg(root.join("scripts/generate_demo_songs.py"))
        .current_dir(root)
        .output()
        .expect("run deterministic demo validator");
    assert!(
        generator.status.success(),
        "{}{}",
        String::from_utf8_lossy(&generator.stdout),
        String::from_utf8_lossy(&generator.stderr)
    );
    validate_demo_dir(&root.join("demos")).expect("validate cleared demo corpus");
}
