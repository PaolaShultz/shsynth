use super::{smooth, EffectError, PARAMETER_SMOOTH_SAMPLES};
use crate::audio_graph::EffectInstance;
use crate::dsp::{finite_or_zero, SineLfo, SmoothedValue, StereoFrame};
use crate::effect_schema;

const TABLE_STEPS: usize = 1_024;
const MAX_STAGES: usize = 6;

#[derive(Clone, Copy, Debug, Default)]
struct AllpassState {
    previous_input: f32,
    previous_output: f32,
}

impl AllpassState {
    #[inline]
    fn process(&mut self, input: f32, coefficient: f32) -> f32 {
        let output = coefficient * (input - self.previous_output) + self.previous_input;
        self.previous_input = input;
        self.previous_output = finite_or_zero(output);
        self.previous_output
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub(super) struct Phaser {
    sample_rate: f32,
    left_lfo: SineLfo,
    right_lfo: SineLfo,
    rate_hz: f32,
    stereo_phase_degrees: f32,
    center_hz: f32,
    range_octaves: f32,
    coefficients: Box<[f32]>,
    left_stages: [AllpassState; MAX_STAGES],
    right_stages: [AllpassState; MAX_STAGES],
    stages: usize,
    left_feedback: f32,
    right_feedback: f32,
    feedback: SmoothedValue,
    mix: SmoothedValue,
    dry: SmoothedValue,
}

impl Phaser {
    pub(super) fn compile(effect: &EffectInstance, sample_rate: u32) -> Result<Self, EffectError> {
        let value = |name| {
            effect_schema::parameter(effect, name)
                .map_err(|error| EffectError::new(error.to_string()))
        };
        let sample_rate = sample_rate as f32;
        let rate_hz = value("rate_hz")?;
        let stereo_phase_degrees = value("stereo_phase_degrees")?;
        let center_hz = value("center_hz")?;
        let range_octaves = value("range_octaves")?;
        Ok(Self {
            sample_rate,
            left_lfo: SineLfo::new(rate_hz, 0.0, sample_rate)?,
            right_lfo: SineLfo::new(rate_hz, stereo_phase_degrees.to_radians(), sample_rate)?,
            rate_hz,
            stereo_phase_degrees,
            center_hz,
            range_octaves,
            coefficients: coefficient_table(center_hz, range_octaves, sample_rate),
            left_stages: [AllpassState::default(); MAX_STAGES],
            right_stages: [AllpassState::default(); MAX_STAGES],
            stages: value("stages")? as usize,
            left_feedback: 0.0,
            right_feedback: 0.0,
            feedback: smooth(value("feedback_percent")? * 0.01),
            mix: smooth(value("mix_percent")? * 0.01),
            dry: smooth(value("dry_percent")? * 0.01),
        })
    }

    #[inline]
    pub(super) fn process(&mut self, frame: StereoFrame) -> StereoFrame {
        let left_coefficient = table_value(&self.coefficients, self.left_lfo.next_value());
        let right_coefficient = table_value(&self.coefficients, self.right_lfo.next_value());
        let feedback = self.feedback.next_value();
        let mut left = finite_or_zero(frame.left + self.left_feedback * feedback);
        let mut right = finite_or_zero(frame.right + self.right_feedback * feedback);
        for stage in 0..self.stages {
            left = self.left_stages[stage].process(left, left_coefficient);
            right = self.right_stages[stage].process(right, right_coefficient);
        }
        if !left.is_finite() || !right.is_finite() || left.abs() > 64.0 || right.abs() > 64.0 {
            self.reset();
            left = 0.0;
            right = 0.0;
        }
        self.left_feedback = left;
        self.right_feedback = right;
        let mix = self.mix.next_value();
        let dry = self.dry.next_value();
        StereoFrame::new(
            frame.left * dry + left * mix,
            frame.right * dry + right * mix,
        )
        .finite_or_silence()
    }

    pub(super) fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        match name {
            "stages" => self.stages = value as usize,
            "rate_hz" => {
                self.rate_hz = value;
                self.configure_lfos()?;
            }
            "center_hz" => {
                self.center_hz = value;
                self.rebuild_table();
            }
            "range_octaves" => {
                self.range_octaves = value;
                self.rebuild_table();
            }
            "feedback_percent" => self
                .feedback
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            "stereo_phase_degrees" => {
                self.stereo_phase_degrees = value;
                self.configure_lfos()?;
            }
            "mix_percent" => self
                .mix
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            "dry_percent" => self
                .dry
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            _ => return Err(EffectError::new(format!("unknown Phaser parameter {name}"))),
        }
        Ok(())
    }

    fn configure_lfos(&mut self) -> Result<(), EffectError> {
        self.left_lfo
            .configure(self.rate_hz, 0.0, self.sample_rate)?;
        self.right_lfo.configure(
            self.rate_hz,
            self.stereo_phase_degrees.to_radians(),
            self.sample_rate,
        )?;
        Ok(())
    }

    fn rebuild_table(&mut self) {
        self.coefficients = coefficient_table(self.center_hz, self.range_octaves, self.sample_rate);
    }

    pub(super) fn reset(&mut self) {
        for stage in &mut self.left_stages {
            stage.reset();
        }
        for stage in &mut self.right_stages {
            stage.reset();
        }
        self.left_feedback = 0.0;
        self.right_feedback = 0.0;
    }
}

fn coefficient_table(center_hz: f32, range_octaves: f32, sample_rate: f32) -> Box<[f32]> {
    (0..=TABLE_STEPS)
        .map(|index| {
            let lfo = index as f32 / TABLE_STEPS as f32 * 2.0 - 1.0;
            let frequency = (center_hz * 2.0_f32.powf(lfo * range_octaves * 0.5))
                .clamp(20.0, sample_rate * 0.45);
            let tangent = (std::f32::consts::PI * frequency / sample_rate).tan();
            ((1.0 - tangent) / (1.0 + tangent)).clamp(-0.999_9, 0.999_9)
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

#[inline]
fn table_value(table: &[f32], lfo: f32) -> f32 {
    let position = (lfo * 0.5 + 0.5).clamp(0.0, 1.0) * TABLE_STEPS as f32;
    let first = position.floor() as usize;
    let second = (first + 1).min(TABLE_STEPS);
    let fraction = position - first as f32;
    table[first] + (table[second] - table[first]) * fraction
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_graph::{EffectKind, EFFECT_FORMAT_VERSION};
    use crate::dsp::allocation_test::assert_no_allocations;
    use crate::effects::EffectSlot;
    use std::collections::BTreeMap;

    fn effect(parameters: impl IntoIterator<Item = (&'static str, f32)>) -> EffectInstance {
        EffectInstance {
            id: 32,
            kind: EffectKind::Phaser,
            version: EFFECT_FORMAT_VERSION,
            bypass: false,
            parameters: parameters
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect::<BTreeMap<_, _>>(),
            owned_memory_bytes: 0,
        }
    }

    #[test]
    fn coefficient_sweep_is_strictly_stable_at_all_rates() {
        for sample_rate in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 384_000.0] {
            for center in [100.0, 1_000.0, 5_000.0] {
                let table = coefficient_table(center, 6.0, sample_rate);
                assert!(table
                    .iter()
                    .all(|coefficient| coefficient.is_finite() && coefficient.abs() < 1.0));
            }
        }
    }

    #[test]
    fn four_and_six_stage_curves_are_distinct_and_stereo_phase_decorrelates() {
        let input = (0..8_192)
            .map(|index| {
                let value = (index as f32 * 0.173).sin() * 0.5;
                StereoFrame::new(value, value)
            })
            .collect::<Vec<_>>();
        let mut four = EffectSlot::compile(
            &effect([
                ("stages", 4.0),
                ("stereo_phase_degrees", 180.0),
                ("mix_percent", 100.0),
                ("dry_percent", 0.0),
            ]),
            48_000,
            128,
        )
        .unwrap();
        let mut six = EffectSlot::compile(
            &effect([
                ("stages", 6.0),
                ("stereo_phase_degrees", 180.0),
                ("mix_percent", 100.0),
                ("dry_percent", 0.0),
            ]),
            48_000,
            128,
        )
        .unwrap();
        let mut output_four = input.clone();
        let mut output_six = input;
        for chunk in output_four.chunks_mut(97) {
            four.process(chunk);
        }
        for chunk in output_six.chunks_mut(97) {
            six.process(chunk);
        }
        assert_ne!(output_four, output_six);
        assert!(output_four
            .iter()
            .skip(1_000)
            .any(|frame| (frame.left - frame.right).abs() > 0.001));
    }

    #[test]
    fn limits_moves_chunks_reset_bypass_and_allocation_are_safe() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000, 384_000] {
            let mut slot = EffectSlot::compile(
                &effect([
                    ("stages", 6.0),
                    ("feedback_percent", 75.0),
                    ("range_octaves", 6.0),
                ]),
                sample_rate,
                128,
            )
            .unwrap();
            let mut block = [StereoFrame::new(1.0, -1.0); 2_048];
            assert_no_allocations(|| {
                for chunk in block.chunks_mut(31) {
                    slot.process(chunk);
                }
            });
            assert!(block
                .iter()
                .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
            for index in 0..40 {
                slot.set_parameter("center_hz", 100.0 + index as f32 * 100.0)
                    .unwrap();
                slot.set_parameter("feedback_percent", index as f32 - 20.0)
                    .unwrap();
                let mut short = [StereoFrame::new(0.5, -0.5); 13];
                slot.process(&mut short);
            }
            slot.reset();
            slot.set_bypass(true).unwrap();
            let mut dry = [StereoFrame::new(0.25, -0.5); 2_048];
            slot.process(&mut dry);
            assert_eq!(dry[2_047], StereoFrame::new(0.25, -0.5));
        }
    }
}
