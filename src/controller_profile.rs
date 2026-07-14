//! Discoverable, data-driven input-controller profiles.

use crate::pads::{ControllerLayout, PadAction, PadConfig};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
    #[serde(default)]
    pub cc_buttons: HashMap<u8, String>,
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
        for &target in self.controls.values() {
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
        config.cc_buttons = self
            .cc_buttons
            .iter()
            .map(|(&number, action)| Ok((number, action.parse()?)))
            .collect::<Result<_>>()?;
        Ok(())
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
        self.profiles
            .iter()
            .filter(|profile| profile.matches(port_name))
            .max_by_key(|profile| {
                profile
                    .match_names
                    .iter()
                    .map(|name| normalize(name))
                    .filter(|name| normalized_port.contains(name))
                    .map(|name| name.len())
                    .max()
                    .unwrap_or(0)
            })
    }

    pub fn profiles(&self) -> &[ControllerProfile] {
        &self.profiles
    }
}

pub fn validate_catalog(path: &Path) -> Result<usize> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let profiles: Vec<ControllerProfile> =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
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
    }
}
