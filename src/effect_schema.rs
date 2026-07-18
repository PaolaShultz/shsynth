//! Stable, named parameter contracts for persisted audio effects.
//!
//! UI input may be clamped before it reaches this module. Persisted values are
//! deliberately stricter: unknown names, non-finite values, out-of-range
//! values, and invalid discrete choices reject the complete graph.

use crate::audio_graph::{EffectInstance, EffectKind};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterType {
    Continuous,
    Integer,
    Toggle,
    Choices(&'static [i16]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParameterSpec {
    pub name: &'static str,
    pub unit: &'static str,
    pub default: f32,
    pub minimum: f32,
    pub maximum: f32,
    pub value_type: ParameterType,
}

impl ParameterSpec {
    pub const fn continuous(
        name: &'static str,
        unit: &'static str,
        default: f32,
        minimum: f32,
        maximum: f32,
    ) -> Self {
        Self {
            name,
            unit,
            default,
            minimum,
            maximum,
            value_type: ParameterType::Continuous,
        }
    }

    pub const fn integer(
        name: &'static str,
        unit: &'static str,
        default: f32,
        minimum: f32,
        maximum: f32,
    ) -> Self {
        Self {
            name,
            unit,
            default,
            minimum,
            maximum,
            value_type: ParameterType::Integer,
        }
    }

    pub const fn toggle(name: &'static str, default: bool) -> Self {
        Self {
            name,
            unit: "on/off",
            default: if default { 1.0 } else { 0.0 },
            minimum: 0.0,
            maximum: 1.0,
            value_type: ParameterType::Toggle,
        }
    }

    pub const fn choices(
        name: &'static str,
        unit: &'static str,
        default: i16,
        choices: &'static [i16],
    ) -> Self {
        Self {
            name,
            unit,
            default: default as f32,
            minimum: 0.0,
            maximum: 0.0,
            value_type: ParameterType::Choices(choices),
        }
    }

    pub fn accepts(self, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match self.value_type {
            ParameterType::Continuous => (self.minimum..=self.maximum).contains(&value),
            ParameterType::Integer => {
                (self.minimum..=self.maximum).contains(&value) && value.fract() == 0.0
            }
            ParameterType::Toggle => value == 0.0 || value == 1.0,
            ParameterType::Choices(choices) => {
                value.fract() == 0.0 && choices.contains(&(value as i16))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaError(String);

impl SchemaError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SchemaError {}

const UTILITY: &[ParameterSpec] = &[
    ParameterSpec::continuous("trim_db", "dB", 0.0, -60.0, 12.0),
    ParameterSpec::continuous("pan", "L/R", 0.0, -1.0, 1.0),
    ParameterSpec::continuous("width_percent", "%", 100.0, 0.0, 200.0),
    ParameterSpec::toggle("invert_left", false),
    ParameterSpec::toggle("invert_right", false),
    ParameterSpec::toggle("mute", false),
];

const EQ: &[ParameterSpec] = &[
    ParameterSpec::toggle("low_cut_enabled", false),
    ParameterSpec::continuous("low_cut_hz", "Hz", 80.0, 20.0, 500.0),
    ParameterSpec::continuous("low_shelf_hz", "Hz", 120.0, 40.0, 800.0),
    ParameterSpec::continuous("low_shelf_db", "dB", 0.0, -18.0, 18.0),
    ParameterSpec::continuous("low_mid_hz", "Hz", 500.0, 80.0, 3_000.0),
    ParameterSpec::continuous("low_mid_db", "dB", 0.0, -18.0, 18.0),
    ParameterSpec::continuous("high_mid_hz", "Hz", 3_000.0, 400.0, 12_000.0),
    ParameterSpec::continuous("high_mid_db", "dB", 0.0, -18.0, 18.0),
    ParameterSpec::continuous("high_shelf_hz", "Hz", 8_000.0, 1_500.0, 20_000.0),
    ParameterSpec::continuous("high_shelf_db", "dB", 0.0, -18.0, 18.0),
    ParameterSpec::continuous("output_trim_db", "dB", 0.0, -18.0, 12.0),
];

const COMPRESSOR: &[ParameterSpec] = &[
    ParameterSpec::continuous("threshold_db", "dBFS", -18.0, -48.0, 0.0),
    ParameterSpec::continuous("ratio", ":1", 4.0, 1.0, 20.0),
    ParameterSpec::continuous("knee_db", "dB", 6.0, 0.0, 12.0),
    ParameterSpec::continuous("attack_ms", "ms", 10.0, 0.1, 100.0),
    ParameterSpec::continuous("release_ms", "ms", 150.0, 20.0, 1_500.0),
    ParameterSpec::continuous("makeup_db", "dB", 0.0, -12.0, 18.0),
    ParameterSpec::continuous("mix_percent", "%", 100.0, 0.0, 100.0),
    ParameterSpec::continuous("sidechain_highpass_hz", "Hz", 20.0, 20.0, 250.0),
];

const DISTORTION: &[ParameterSpec] = &[
    // 0 soft cubic, 1 hard clip, 2 asymmetric diode-like.
    ParameterSpec::integer("mode", "mode", 0.0, 0.0, 2.0),
    ParameterSpec::continuous("drive_db", "dB", 6.0, 0.0, 30.0),
    ParameterSpec::continuous("bias", "", 0.0, -0.5, 0.5),
    ParameterSpec::continuous("tone_hz", "Hz", 12_000.0, 800.0, 18_000.0),
    ParameterSpec::continuous("output_db", "dB", -6.0, -24.0, 0.0),
    ParameterSpec::continuous("mix_percent", "%", 100.0, 0.0, 100.0),
];

const DELAY: &[ParameterSpec] = &[
    ParameterSpec::integer("mode", "mode", 0.0, 0.0, 2.0),
    ParameterSpec::continuous("time_ms", "ms", 375.0, 1.0, 2_000.0),
    ParameterSpec::continuous("feedback_percent", "%", 30.0, 0.0, 92.0),
    ParameterSpec::continuous("stereo_ratio", "", 1.0, 0.5, 2.0),
    ParameterSpec::continuous("tone_hz", "Hz", 8_000.0, 500.0, 18_000.0),
    ParameterSpec::continuous("wet_percent", "%", 25.0, 0.0, 100.0),
    ParameterSpec::continuous("dry_percent", "%", 100.0, 0.0, 100.0),
    ParameterSpec::toggle("tail_on_bypass", false),
];

const REVERB: &[ParameterSpec] = &[
    ParameterSpec::integer("type", "type", 0.0, 0.0, 2.0),
    ParameterSpec::continuous("predelay_ms", "ms", 20.0, 0.0, 200.0),
    ParameterSpec::continuous("decay_seconds", "s", 1.5, 0.2, 8.0),
    ParameterSpec::continuous("size_percent", "%", 50.0, 0.0, 100.0),
    ParameterSpec::continuous("damping_percent", "%", 50.0, 0.0, 100.0),
    ParameterSpec::continuous("input_low_cut_hz", "Hz", 80.0, 20.0, 500.0),
    ParameterSpec::continuous("width_percent", "%", 100.0, 0.0, 100.0),
    ParameterSpec::continuous("wet_percent", "%", 25.0, 0.0, 100.0),
    ParameterSpec::continuous("dry_percent", "%", 100.0, 0.0, 100.0),
];

const CHORUS: &[ParameterSpec] = &[
    ParameterSpec::continuous("base_delay_ms", "ms", 15.0, 5.0, 30.0),
    ParameterSpec::continuous("rate_hz", "Hz", 0.5, 0.05, 5.0),
    ParameterSpec::continuous("depth_percent", "%", 35.0, 0.0, 100.0),
    ParameterSpec::continuous("stereo_phase_degrees", "deg", 90.0, 0.0, 180.0),
    ParameterSpec::continuous("feedback_percent", "%", 0.0, 0.0, 35.0),
    ParameterSpec::continuous("mix_percent", "%", 35.0, 0.0, 100.0),
    ParameterSpec::continuous("dry_percent", "%", 100.0, 0.0, 100.0),
];

const FLANGER: &[ParameterSpec] = &[
    ParameterSpec::continuous("base_delay_ms", "ms", 2.0, 0.2, 8.0),
    ParameterSpec::continuous("rate_hz", "Hz", 0.25, 0.03, 5.0),
    ParameterSpec::continuous("depth_percent", "%", 50.0, 0.0, 100.0),
    ParameterSpec::continuous("feedback_percent", "%", 25.0, -80.0, 80.0),
    ParameterSpec::continuous("stereo_phase_degrees", "deg", 90.0, 0.0, 180.0),
    ParameterSpec::continuous("mix_percent", "%", 50.0, 0.0, 100.0),
    ParameterSpec::continuous("dry_percent", "%", 100.0, 0.0, 100.0),
];

const PHASER: &[ParameterSpec] = &[
    ParameterSpec::choices("stages", "stages", 4, &[4, 6]),
    ParameterSpec::continuous("rate_hz", "Hz", 0.25, 0.03, 5.0),
    ParameterSpec::continuous("center_hz", "Hz", 1_000.0, 100.0, 5_000.0),
    ParameterSpec::continuous("range_octaves", "oct", 3.0, 0.5, 6.0),
    ParameterSpec::continuous("feedback_percent", "%", 0.0, -75.0, 75.0),
    ParameterSpec::continuous("stereo_phase_degrees", "deg", 90.0, 0.0, 180.0),
    ParameterSpec::continuous("mix_percent", "%", 50.0, 0.0, 100.0),
    ParameterSpec::continuous("dry_percent", "%", 100.0, 0.0, 100.0),
];

const TREMOLO_PAN: &[ParameterSpec] = &[
    ParameterSpec::integer("mode", "mode", 0.0, 0.0, 1.0),
    ParameterSpec::continuous("rate_hz", "Hz", 4.0, 0.05, 15.0),
    ParameterSpec::continuous("depth_percent", "%", 50.0, 0.0, 100.0),
    ParameterSpec::integer("shape", "shape", 0.0, 0.0, 2.0),
    ParameterSpec::continuous("stereo_phase_degrees", "deg", 180.0, 0.0, 180.0),
    ParameterSpec::continuous("output_trim_db", "dB", 0.0, -18.0, 12.0),
];

const FILTER: &[ParameterSpec] = &[
    // 0 low-pass, 1 band-pass, 2 high-pass.
    ParameterSpec::integer("mode", "mode", 0.0, 0.0, 2.0),
    ParameterSpec::continuous("cutoff_hz", "Hz", 1_000.0, 20.0, 20_000.0),
    ParameterSpec::continuous("resonance", "%", 20.0, 0.0, 90.0),
    ParameterSpec::continuous("drive_db", "dB", 0.0, 0.0, 12.0),
    ParameterSpec::continuous("mix_percent", "%", 100.0, 0.0, 100.0),
];

const GATE: &[ParameterSpec] = &[
    ParameterSpec::continuous("threshold_db", "dBFS", -48.0, -80.0, 0.0),
    ParameterSpec::continuous("hysteresis_db", "dB", 6.0, 0.0, 24.0),
    ParameterSpec::continuous("range_db", "dB", -60.0, -80.0, 0.0),
    ParameterSpec::continuous("attack_ms", "ms", 2.0, 0.1, 100.0),
    ParameterSpec::continuous("hold_ms", "ms", 40.0, 0.0, 500.0),
    ParameterSpec::continuous("release_ms", "ms", 150.0, 5.0, 2_000.0),
];

const CRUSHER: &[ParameterSpec] = &[
    ParameterSpec::integer("bit_depth", "bit", 12.0, 4.0, 16.0),
    ParameterSpec::integer("hold_factor", "x", 1.0, 1.0, 32.0),
    ParameterSpec::toggle("dither", false),
    ParameterSpec::continuous("mix_percent", "%", 100.0, 0.0, 100.0),
];

pub const fn schema(kind: EffectKind) -> &'static [ParameterSpec] {
    match kind {
        EffectKind::Utility => UTILITY,
        EffectKind::Eq => EQ,
        EffectKind::Compressor => COMPRESSOR,
        EffectKind::Distortion => DISTORTION,
        EffectKind::Delay => DELAY,
        EffectKind::Reverb => REVERB,
        EffectKind::Chorus => CHORUS,
        EffectKind::Flanger => FLANGER,
        EffectKind::Phaser => PHASER,
        EffectKind::TremoloPan => TREMOLO_PAN,
        EffectKind::Filter => FILTER,
        EffectKind::Gate => GATE,
        EffectKind::Crusher => CRUSHER,
    }
}

pub fn defaults(kind: EffectKind) -> BTreeMap<String, f32> {
    schema(kind)
        .iter()
        .map(|spec| (spec.name.to_owned(), spec.default))
        .collect()
}

pub fn parameter(effect: &EffectInstance, name: &str) -> Result<f32, SchemaError> {
    let spec = schema(effect.kind)
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| SchemaError::new(format!("unknown {:?} parameter {name}", effect.kind)))?;
    let value = effect.parameters.get(name).copied().unwrap_or(spec.default);
    if !spec.accepts(value) {
        return Err(SchemaError::new(format!(
            "invalid {:?} parameter {name}",
            effect.kind
        )));
    }
    Ok(value)
}

pub fn validate(effect: &EffectInstance) -> Result<(), SchemaError> {
    let specs = schema(effect.kind);
    for name in effect.parameters.keys() {
        if !specs.iter().any(|spec| spec.name == name) {
            return Err(SchemaError::new(format!(
                "unknown {:?} parameter {name}",
                effect.kind
            )));
        }
    }
    for spec in specs {
        parameter(effect, spec.name)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_graph::EFFECT_FORMAT_VERSION;

    fn instance(kind: EffectKind) -> EffectInstance {
        EffectInstance {
            id: 1,
            kind,
            version: EFFECT_FORMAT_VERSION,
            bypass: false,
            parameters: BTreeMap::new(),
            owned_memory_bytes: 0,
        }
    }

    #[test]
    fn every_kind_has_unique_valid_named_defaults() {
        for kind in [
            EffectKind::Utility,
            EffectKind::Eq,
            EffectKind::Compressor,
            EffectKind::Distortion,
            EffectKind::Delay,
            EffectKind::Reverb,
            EffectKind::Chorus,
            EffectKind::Flanger,
            EffectKind::Phaser,
            EffectKind::TremoloPan,
            EffectKind::Filter,
            EffectKind::Gate,
            EffectKind::Crusher,
        ] {
            let effect = instance(kind);
            validate(&effect).unwrap();
            let defaults = defaults(kind);
            assert_eq!(defaults.len(), schema(kind).len());
            assert!(schema(kind)
                .iter()
                .all(|spec| { !spec.name.is_empty() && spec.accepts(spec.default) }));
        }
    }

    #[test]
    fn persisted_values_are_strict_but_missing_values_use_defaults() {
        let mut effect = instance(EffectKind::Crusher);
        assert_eq!(parameter(&effect, "bit_depth").unwrap(), 12.0);
        effect.parameters.insert("bit_depth".into(), 8.5);
        assert!(validate(&effect)
            .unwrap_err()
            .to_string()
            .contains("bit_depth"));
        effect.parameters.insert("bit_depth".into(), 8.0);
        effect.parameters.insert("dither".into(), 0.5);
        assert!(validate(&effect)
            .unwrap_err()
            .to_string()
            .contains("dither"));
        effect.parameters.insert("dither".into(), 1.0);
        effect.parameters.insert("future_control".into(), 0.0);
        assert!(validate(&effect)
            .unwrap_err()
            .to_string()
            .contains("future_control"));
    }

    #[test]
    fn non_finite_and_out_of_range_values_are_rejected() {
        let mut effect = instance(EffectKind::Compressor);
        for bad in [f32::NAN, f32::INFINITY, -49.0, 1.0] {
            effect.parameters.insert("threshold_db".into(), bad);
            assert!(validate(&effect).is_err(), "accepted {bad}");
        }
        effect.parameters.insert("threshold_db".into(), -24.0);
        validate(&effect).unwrap();

        let mut phaser = instance(EffectKind::Phaser);
        phaser.parameters.insert("stages".into(), 5.0);
        assert!(validate(&phaser).is_err());
        phaser.parameters.insert("stages".into(), 6.0);
        validate(&phaser).unwrap();
    }
}
