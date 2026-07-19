use super::{smooth, EffectError, PARAMETER_SMOOTH_SAMPLES};
use crate::audio_graph::{EffectInstance, EffectKind};
use crate::dsp::{FractionalDelayLine, SineLfo, SmoothedValue, StereoFrame};
use crate::effect_schema;

const CHORUS_CAPACITY_MILLISECONDS: f32 = 45.0;
const FLANGER_CAPACITY_MILLISECONDS: f32 = 16.0;
const EMERGENCY_LEVEL: f32 = 64.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Chorus,
    Flanger,
}

pub(super) struct ModulatedDelay {
    kind: Kind,
    sample_rate: f32,
    left: FractionalDelayLine,
    right: FractionalDelayLine,
    left_lfo: SineLfo,
    right_lfo: SineLfo,
    rate_hz: f32,
    stereo_phase_degrees: f32,
    base_samples: SmoothedValue,
    depth: SmoothedValue,
    feedback: SmoothedValue,
    mix: SmoothedValue,
    dry: SmoothedValue,
}

impl ModulatedDelay {
    pub(super) fn compile(effect: &EffectInstance, sample_rate: u32) -> Result<Self, EffectError> {
        let value = |name| {
            effect_schema::parameter(effect, name)
                .map_err(|error| EffectError::new(error.to_string()))
        };
        let kind = match effect.kind {
            EffectKind::Chorus => Kind::Chorus,
            EffectKind::Flanger => Kind::Flanger,
            _ => return Err(EffectError::new("modulated delay kind mismatch")),
        };
        let sample_rate = sample_rate as f32;
        let capacity_ms = match kind {
            Kind::Chorus => CHORUS_CAPACITY_MILLISECONDS,
            Kind::Flanger => FLANGER_CAPACITY_MILLISECONDS,
        };
        let capacity = (sample_rate * capacity_ms / 1_000.0).ceil().max(2.0) as usize;
        let rate_hz = value("rate_hz")?;
        let stereo_phase_degrees = value("stereo_phase_degrees")?;
        Ok(Self {
            kind,
            sample_rate,
            left: FractionalDelayLine::new(capacity)?,
            right: FractionalDelayLine::new(capacity)?,
            left_lfo: SineLfo::new(rate_hz, 0.0, sample_rate)?,
            right_lfo: SineLfo::new(rate_hz, stereo_phase_degrees.to_radians(), sample_rate)?,
            rate_hz,
            stereo_phase_degrees,
            base_samples: smooth(value("base_delay_ms")? * sample_rate / 1_000.0),
            depth: smooth(value("depth_percent")? * 0.01),
            feedback: smooth(value("feedback_percent")? * 0.01),
            mix: smooth(value("mix_percent")? * 0.01),
            dry: smooth(value("dry_percent")? * 0.01),
        })
    }

    #[inline]
    pub(super) fn process(&mut self, frame: StereoFrame) -> StereoFrame {
        let base = self.base_samples.next_value();
        let depth = self.depth.next_value();
        let left_lfo = self.left_lfo.next_value();
        let right_lfo = self.right_lfo.next_value();
        let left_time = self.modulated_samples(base, depth, left_lfo);
        let right_time = self.modulated_samples(base, depth, right_lfo);
        let delayed_left = self.left.read(left_time);
        let delayed_right = self.right.read(right_time);
        let feedback = self.feedback.next_value();
        let write_left = frame.left + delayed_left * feedback;
        let write_right = frame.right + delayed_right * feedback;
        if write_left.abs() > EMERGENCY_LEVEL
            || write_right.abs() > EMERGENCY_LEVEL
            || !write_left.is_finite()
            || !write_right.is_finite()
        {
            self.reset();
        } else {
            self.left.push(write_left);
            self.right.push(write_right);
        }
        let mix = self.mix.next_value();
        let dry = self.dry.next_value();
        StereoFrame::new(
            frame.left * dry + delayed_left * mix,
            frame.right * dry + delayed_right * mix,
        )
        .finite_or_silence()
    }

    pub(super) fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        match name {
            "base_delay_ms" => self
                .base_samples
                .set_target(value * self.sample_rate / 1_000.0, PARAMETER_SMOOTH_SAMPLES)?,
            "rate_hz" => {
                self.rate_hz = value;
                self.configure_lfos()?;
            }
            "depth_percent" => self
                .depth
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            "stereo_phase_degrees" => {
                self.stereo_phase_degrees = value;
                self.configure_lfos()?;
            }
            "feedback_percent" => self
                .feedback
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            "mix_percent" => self
                .mix
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            "dry_percent" => self
                .dry
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            _ => {
                return Err(EffectError::new(format!(
                    "unknown modulated delay parameter {name}"
                )))
            }
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

    #[inline]
    fn modulated_samples(&self, base: f32, depth: f32, lfo: f32) -> f32 {
        let maximum = self.left.maximum_delay() as f32;
        let available = (base - 1.0).min(maximum - base).max(0.0);
        let kind_scale = match self.kind {
            Kind::Chorus => 0.9,
            Kind::Flanger => 1.0,
        };
        (base + lfo * available * depth * kind_scale).clamp(1.0, maximum)
    }

    pub(super) fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_graph::EFFECT_FORMAT_VERSION;
    use crate::dsp::allocation_test::assert_no_allocations;
    use crate::effects::EffectSlot;
    use std::collections::BTreeMap;

    fn effect(
        kind: EffectKind,
        parameters: impl IntoIterator<Item = (&'static str, f32)>,
    ) -> EffectInstance {
        EffectInstance {
            id: 31,
            kind,
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
    fn zero_depth_chorus_impulse_has_the_declared_base_delay() {
        let mut slot = EffectSlot::compile(
            &effect(
                EffectKind::Chorus,
                [
                    ("base_delay_ms", 15.0),
                    ("depth_percent", 0.0),
                    ("feedback_percent", 0.0),
                    ("mix_percent", 100.0),
                    ("dry_percent", 0.0),
                ],
            ),
            48_000,
            64,
        )
        .unwrap();
        let mut samples = vec![StereoFrame::SILENCE; 722];
        samples[0] = StereoFrame::new(1.0, 1.0);
        for chunk in samples.chunks_mut(43) {
            slot.process(chunk);
        }
        assert!(samples[..720].iter().all(|frame| frame.left.abs() < 1.0e-7));
        assert!((samples[720].left - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn modulation_read_heads_never_leave_allocated_history() {
        for kind in [EffectKind::Chorus, EffectKind::Flanger] {
            let configured = effect(
                kind,
                [("depth_percent", 100.0), ("stereo_phase_degrees", 180.0)],
            );
            let processor = ModulatedDelay::compile(&configured, 48_000).unwrap();
            for index in 0..=2_000 {
                let lfo = index as f32 / 1_000.0 - 1.0;
                let value = processor.modulated_samples(processor.base_samples.current(), 1.0, lfo);
                assert!((1.0..=processor.left.maximum_delay() as f32).contains(&value));
            }
        }
    }

    #[test]
    fn stereo_phase_and_signed_flanger_feedback_are_distinct_and_finite() {
        let mut slot = EffectSlot::compile(
            &effect(
                EffectKind::Flanger,
                [
                    ("depth_percent", 100.0),
                    ("feedback_percent", -80.0),
                    ("stereo_phase_degrees", 180.0),
                    ("mix_percent", 100.0),
                    ("dry_percent", 0.0),
                ],
            ),
            48_000,
            128,
        )
        .unwrap();
        let mut block = [StereoFrame::new(0.5, 0.5); 4_096];
        for chunk in block.chunks_mut(73) {
            slot.process(chunk);
        }
        assert!(block
            .iter()
            .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
        assert!(block
            .iter()
            .skip(500)
            .any(|frame| (frame.left - frame.right).abs() > 0.01));
    }

    #[test]
    fn rates_chunks_parameter_moves_reset_bypass_and_allocation_are_safe() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000, 384_000] {
            for kind in [EffectKind::Chorus, EffectKind::Flanger] {
                let configured = effect(kind, []);
                let mut slot = EffectSlot::compile(&configured, sample_rate, 128).unwrap();
                let mut block = [StereoFrame::new(1.0, -1.0); 512];
                assert_no_allocations(|| {
                    for chunk in block.chunks_mut(37) {
                        slot.process(chunk);
                    }
                });
                for index in 0..50 {
                    slot.set_parameter("depth_percent", (index * 2) as f32)
                        .unwrap();
                    slot.set_parameter("rate_hz", 0.05 + index as f32 * 0.05)
                        .unwrap();
                    let mut short = [StereoFrame::new(0.25, -0.25); 17];
                    slot.process(&mut short);
                    assert!(short
                        .iter()
                        .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
                }
                slot.reset();
                slot.set_bypass(true).unwrap();
                let mut dry = [StereoFrame::new(0.25, -0.5); 2_048];
                slot.process(&mut dry);
                assert_eq!(dry[2_047], StereoFrame::new(0.25, -0.5));
            }
        }
    }
}
