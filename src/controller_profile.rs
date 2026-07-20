//! Discoverable, data-driven input-controller profiles.

use crate::pads::{ControllerLayout, PadAction, PadConfig};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

pub const UPDATE_URL: &str =
    "https://raw.githubusercontent.com/PaolaShultz/shr-daw/main/controller-profiles/catalog.json";

#[derive(Clone, Debug, Deserialize)]
pub struct ControllerProfile {
    pub id: String,
    pub name: String,
    pub match_names: Vec<String>,
    pub layout: u8,
    #[serde(default)]
    pub controls: HashMap<u8, u8>,
    #[serde(default)]
    pub encoder_relative_cc: Option<u8>,
    #[serde(default)]
    pub encoder_relative_reverse: bool,
    #[serde(default)]
    pub encoder_press_cc: Option<u8>,
    #[serde(default)]
    pub encoder_press_note: Option<u8>,
    #[serde(default)]
    pub lock_cc: Option<u8>,
    #[serde(default)]
    pub note_buttons: HashMap<u8, String>,
    /// Optional 1-based channel qualifiers keyed by command note.
    #[serde(default)]
    pub note_button_channels: HashMap<u8, u8>,
    #[serde(default)]
    pub cc_buttons: HashMap<u8, String>,
    /// Optional 1-based channel qualifiers keyed by command CC.
    #[serde(default)]
    pub cc_button_channels: HashMap<u8, u8>,
    #[serde(default)]
    pub source: String,
}

impl ControllerProfile {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            bail!("controller profile needs a non-empty id and name");
        }
        if self
            .match_names
            .iter()
            .all(|name| normalize(name).is_empty())
        {
            bail!("controller profile {} needs a match name", self.id);
        }
        if !matches!(self.layout, 4 | 5 | 8) {
            bail!("controller profile {} layout must be 4, 5, or 8", self.id);
        }
        for (&incoming, &target) in &self.controls {
            crate::pads::ensure_midi_number(incoming, "controller profile CC")?;
            if crate::control::by_cc(target).is_none() {
                bail!(
                    "controller profile {} has unknown target CC {target}",
                    self.id
                );
            }
        }
        for action in self.note_buttons.values().chain(self.cc_buttons.values()) {
            action.parse::<PadAction>().with_context(|| {
                format!("controller profile {} has invalid action {action}", self.id)
            })?;
        }
        for &note in self.note_buttons.keys() {
            crate::pads::ensure_midi_number(note, "controller profile button note")?;
        }
        for &cc in self.cc_buttons.keys() {
            crate::pads::ensure_midi_number(cc, "controller profile button CC")?;
        }
        if self.note_button_channels.iter().any(|(note, channel)| {
            !self.note_buttons.contains_key(note) || !(1..=16).contains(channel)
        }) {
            bail!(
                "controller profile {} has an invalid note-button channel qualifier",
                self.id
            );
        }
        if self
            .cc_button_channels
            .iter()
            .any(|(cc, channel)| !self.cc_buttons.contains_key(cc) || !(1..=16).contains(channel))
        {
            bail!(
                "controller profile {} has an invalid CC-button channel qualifier",
                self.id
            );
        }
        for (number, description) in [
            (self.encoder_relative_cc, "controller profile encoder CC"),
            (self.encoder_press_cc, "controller profile encoder press CC"),
            (
                self.encoder_press_note,
                "controller profile encoder press note",
            ),
            (self.lock_cc, "controller profile lock CC"),
        ] {
            if let Some(number) = number {
                crate::pads::ensure_midi_number(number, description)?;
            }
        }
        let mut used_cc = self.controls.keys().copied().collect::<HashSet<_>>();
        for cc in self.cc_buttons.keys().copied().chain(
            [
                self.encoder_relative_cc,
                self.encoder_press_cc,
                self.lock_cc,
            ]
            .into_iter()
            .flatten(),
        ) {
            if !used_cc.insert(cc) {
                bail!("controller profile {} reuses CC {cc}", self.id);
            }
        }
        if self.encoder_press_cc.is_some() && self.encoder_press_note.is_some() {
            bail!(
                "controller profile {} encoder press must use either a CC or a note",
                self.id
            );
        }
        if self
            .encoder_press_note
            .is_some_and(|note| self.note_buttons.contains_key(&note))
        {
            bail!(
                "controller profile {} reuses encoder press note as a button",
                self.id
            );
        }
        Ok(())
    }

    pub fn matches(&self, port_name: &str) -> bool {
        let port = normalize(port_name);
        self.match_names
            .iter()
            .map(|name| normalize(name))
            .any(|name| !name.is_empty() && port.contains(&name))
    }

    pub fn apply(&self, config: &mut PadConfig, input_name: &str) -> Result<()> {
        self.validate()?;
        config.input_match = Some(input_name.to_owned());
        config.layout = match self.layout {
            8 => ControllerLayout::Eight,
            5 => ControllerLayout::Five,
            4 => ControllerLayout::Four,
            _ => unreachable!(),
        };
        config.controls.clone_from(&self.controls);
        config.encoder_relative_cc = self.encoder_relative_cc;
        config.encoder_relative_reverse = self.encoder_relative_reverse;
        config.encoder_press_cc = self.encoder_press_cc;
        config.encoder_press_note = self.encoder_press_note;
        config.lock_cc = self.lock_cc;
        config.pads = self
            .note_buttons
            .iter()
            .map(|(&number, action)| Ok((number, action.parse()?)))
            .collect::<Result<_>>()?;
        config.pad_channels = self
            .note_button_channels
            .iter()
            .map(|(&number, &channel)| (number, channel - 1))
            .collect();
        config.cc_buttons = self
            .cc_buttons
            .iter()
            .map(|(&number, action)| Ok((number, action.parse()?)))
            .collect::<Result<_>>()?;
        config.cc_button_channels = self
            .cc_button_channels
            .iter()
            .map(|(&number, &channel)| (number, channel - 1))
            .collect();
        config.validate()
    }
}

#[derive(Default)]
pub struct Catalog {
    profiles: Vec<ControllerProfile>,
}

impl Catalog {
    pub fn discover() -> Self {
        let mut profiles = Vec::new();
        let mut ids = HashSet::new();
        for root in roots() {
            let path = root.join("catalog.json");
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(found) = serde_json::from_str::<Vec<ControllerProfile>>(&text) else {
                continue;
            };
            for profile in found {
                if profile.validate().is_ok() && ids.insert(profile.id.clone()) {
                    profiles.push(profile);
                }
            }
        }
        Self { profiles }
    }

    pub fn matching(&self, port_name: &str) -> Option<&ControllerProfile> {
        let normalized_port = normalize(port_name);
        let matches = self
            .profiles
            .iter()
            .filter(|profile| profile.matches(port_name))
            .map(|profile| {
                let specificity = profile
                    .match_names
                    .iter()
                    .map(|name| normalize(name))
                    .filter(|name| normalized_port.contains(name))
                    .map(|name| name.len())
                    .max()
                    .unwrap_or(0);
                (specificity, profile)
            })
            .collect::<Vec<_>>();
        let best = matches.iter().map(|(specificity, _)| *specificity).max()?;
        let mut best_matches = matches
            .into_iter()
            .filter(|(specificity, _)| *specificity == best);
        let (_, profile) = best_matches.next()?;
        best_matches.next().is_none().then_some(profile)
    }

    pub fn profiles(&self) -> &[ControllerProfile] {
        &self.profiles
    }
}

#[cfg(test)]
pub fn validate_catalog(path: &Path) -> Result<usize> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    validate_catalog_bytes(&bytes).with_context(|| format!("parse {}", path.display()))
}

pub fn validate_catalog_bytes(bytes: &[u8]) -> Result<usize> {
    let profiles: Vec<ControllerProfile> = serde_json::from_slice(bytes)?;
    let mut ids = HashSet::new();
    for profile in &profiles {
        profile.validate()?;
        if !ids.insert(&profile.id) {
            bail!("duplicate controller profile id {}", profile.id);
        }
    }
    Ok(profiles.len())
}

pub fn user_catalog_path() -> PathBuf {
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".local/share")
        })
        .join("shsynth/controller-profiles/catalog.json")
}

fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = env::var_os("SHSYNTH_CONTROLLER_PROFILE_DIR") {
        roots.push(PathBuf::from(path));
    }
    if let Some(parent) = user_catalog_path().parent() {
        roots.push(parent.to_path_buf());
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.join("../share/shsynth/controller-profiles"));
        }
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("controller-profiles"));
    roots
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_is_valid_and_matches_punctuation_insensitively() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("controller-profiles/catalog.json");
        assert!(validate_catalog(&path).unwrap() >= 1);
        let catalog = Catalog::discover();
        let profile = catalog.matching("20:0 Arturia MiniLab3 MIDI 1").unwrap();
        assert_eq!(profile.id, "arturia-minilab-3");
        let mut config = PadConfig::default();
        profile.apply(&mut config, "MiniLab3 MIDI").unwrap();
        assert_eq!(config.controls.len(), 12);
        assert_eq!(config.pads.len(), 8);
        assert_eq!(config.pad_channels.len(), 8);
        assert!(config.pad_channels.values().all(|channel| *channel == 9));
        assert_eq!(config.lock_cc, None);
        for (offset, action) in [
            PadAction::Page1,
            PadAction::Page2,
            PadAction::Page3,
            PadAction::Page4,
            PadAction::Item1,
            PadAction::Item2,
            PadAction::Item3,
            PadAction::Item4,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(config.pads.get(&(36 + offset as u8)), Some(&action));
        }
        assert_eq!(config.lock_action(&[0xb0, 27, 127]), (false, false));
    }

    fn minimal_profile() -> ControllerProfile {
        ControllerProfile {
            id: "test-controller".into(),
            name: "Test Controller".into(),
            match_names: vec!["test controller".into()],
            layout: 4,
            controls: HashMap::new(),
            encoder_relative_cc: None,
            encoder_relative_reverse: false,
            encoder_press_cc: None,
            encoder_press_note: None,
            lock_cc: None,
            note_buttons: HashMap::new(),
            note_button_channels: HashMap::new(),
            cc_buttons: HashMap::new(),
            cc_button_channels: HashMap::new(),
            source: "hardware verification".into(),
        }
    }

    #[test]
    fn catalog_rejects_out_of_range_and_conflicting_physical_messages() {
        let mut profile = minimal_profile();
        profile.controls.insert(128, 74);
        assert!(profile.validate().is_err());

        profile = minimal_profile();
        profile.encoder_press_note = Some(36);
        profile.note_buttons.insert(36, "item-1".into());
        assert!(profile.validate().is_err());

        profile = minimal_profile();
        profile.encoder_press_cc = Some(118);
        profile.encoder_press_note = Some(36);
        assert!(profile.validate().is_err());

        profile = minimal_profile();
        profile.note_button_channels.insert(36, 10);
        assert!(profile.validate().is_err());

        profile = minimal_profile();
        profile.note_buttons.insert(36, "item-1".into());
        profile.note_button_channels.insert(36, 17);
        assert!(profile.validate().is_err());

        profile = minimal_profile();
        profile.cc_buttons.insert(44, "item-1".into());
        profile.cc_button_channels.insert(44, 0);
        assert!(profile.validate().is_err());
    }

    #[test]
    fn qualified_profile_application_and_controller_save_retain_channels() {
        let mut profile = minimal_profile();
        profile.note_buttons.insert(36, "page-1".into());
        profile.note_button_channels.insert(36, 10);
        profile.cc_buttons.insert(44, "item-1".into());
        profile.cc_button_channels.insert(44, 3);
        let mut config = PadConfig::default();
        profile.apply(&mut config, "Test Controller MIDI").unwrap();
        assert_eq!(config.pad_channels, HashMap::from([(36, 9)]));
        assert_eq!(config.cc_button_channels, HashMap::from([(44, 2)]));

        let path = std::env::temp_dir().join(format!(
            "shsynth-profile-channel-roundtrip-{}.conf",
            std::process::id()
        ));
        config.save(&path).unwrap();
        let loaded = PadConfig::load(&path).unwrap();
        assert_eq!(loaded.pad_channels, config.pad_channels);
        assert_eq!(loaded.cc_button_channels, config.cc_button_channels);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn downloaded_catalog_bytes_are_fully_validated() {
        let profile = r#"{
            "id":"test-controller",
            "name":"Test Controller",
            "match_names":["test controller"],
            "layout":4
        }"#;
        let valid = format!("[{profile}]");
        assert_eq!(validate_catalog_bytes(valid.as_bytes()).unwrap(), 1);

        let duplicate = format!("[{profile},{profile}]");
        assert!(validate_catalog_bytes(duplicate.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("duplicate controller profile id"));
        assert!(validate_catalog_bytes(b"not json").is_err());
    }

    #[test]
    fn equally_specific_controller_profiles_do_not_auto_select() {
        let mut first = minimal_profile();
        first.id = "first".into();
        let mut second = minimal_profile();
        second.id = "second".into();
        let catalog = Catalog {
            profiles: vec![first, second],
        };

        assert!(catalog.matching("Test Controller MIDI").is_none());
    }
}
