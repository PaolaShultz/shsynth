use super::{smooth, EffectError, PARAMETER_SMOOTH_SAMPLES};
use crate::audio_graph::EffectInstance;
use crate::dsp::{FractionalDelayLine, OnePole, OnePoleMode, SmoothedValue, StereoFrame};
use crate::effect_schema;

const PREDELAY_CAPACITY_MILLISECONDS: f32 = 200.0;
const FDN_CAPACITY_MILLISECONDS: f32 = 100.0;
const EMERGENCY_LEVEL: f32 = 64.0;
const VOICING_LENGTHS_MS: [[f32; 4]; 3] = [
    [23.3, 29.7, 31.1, 37.9],
    [37.1, 41.1, 43.7, 47.9],
    [53.1, 61.7, 67.3, 71.9],
];

pub(super) struct Reverb {
    sample_rate: f32,
    predelay_left: FractionalDelayLine,
    predelay_right: FractionalDelayLine,
    predelay_samples: SmoothedValue,
    input_low_cut_left: OnePole,
    input_low_cut_right: OnePole,
    lines: [FractionalDelayLine; 4],
    damping: [OnePole; 4],
    lengths: [f32; 4],
    feedback: [f32; 4],
    voicing: usize,
    decay_seconds: f32,
    size_percent: f32,
    damping_percent: f32,
    width: SmoothedValue,
    wet: SmoothedValue,
    dry: SmoothedValue,
}

impl Reverb {
    pub(super) fn compile(effect: &EffectInstance, sample_rate: u32) -> Result<Self, EffectError> {
        let value = |name| {
            effect_schema::parameter(effect, name)
                .map_err(|error| EffectError::new(error.to_string()))
        };
        let sample_rate = sample_rate as f32;
        let predelay_capacity =
            (sample_rate * PREDELAY_CAPACITY_MILLISECONDS / 1_000.0).ceil() as usize;
        let fdn_capacity = (sample_rate * FDN_CAPACITY_MILLISECONDS / 1_000.0).ceil() as usize;
        let damping_hz = damping_frequency(value("damping_percent")?);
        let line = || FractionalDelayLine::new(fdn_capacity).map_err(EffectError::from);
        let damp = || {
            OnePole::new(OnePoleMode::LowPass, damping_hz, sample_rate).map_err(EffectError::from)
        };
        let mut reverb = Self {
            sample_rate,
            predelay_left: FractionalDelayLine::new(predelay_capacity)?,
            predelay_right: FractionalDelayLine::new(predelay_capacity)?,
            predelay_samples: smooth(value("predelay_ms")? * sample_rate / 1_000.0),
            input_low_cut_left: OnePole::new(
                OnePoleMode::HighPass,
                value("input_low_cut_hz")?,
                sample_rate,
            )?,
            input_low_cut_right: OnePole::new(
                OnePoleMode::HighPass,
                value("input_low_cut_hz")?,
                sample_rate,
            )?,
            lines: [line()?, line()?, line()?, line()?],
            damping: [damp()?, damp()?, damp()?, damp()?],
            lengths: [1.0; 4],
            feedback: [0.0; 4],
            voicing: value("type")? as usize,
            decay_seconds: value("decay_seconds")?,
            size_percent: value("size_percent")?,
            damping_percent: value("damping_percent")?,
            width: smooth(value("width_percent")? * 0.01),
            wet: smooth(value("wet_percent")? * 0.01),
            dry: smooth(value("dry_percent")? * 0.01),
        };
        reverb.update_lengths_and_feedback();
        Ok(reverb)
    }

    #[inline]
    pub(super) fn process(&mut self, frame: StereoFrame) -> StereoFrame {
        let predelay = self.predelay_samples.next_value();
        let (predelayed_left, predelayed_right) = if predelay < 1.0 {
            (frame.left, frame.right)
        } else {
            (
                self.predelay_left.read(predelay),
                self.predelay_right.read(predelay),
            )
        };
        self.predelay_left.push(frame.left);
        self.predelay_right.push(frame.right);

        let input_left = self.input_low_cut_left.process(predelayed_left);
        let input_right = self.input_low_cut_right.process(predelayed_right);
        let mono = (input_left + input_right) * 0.25;
        let side = (input_left - input_right) * 0.25;
        let delayed = [
            self.lines[0].read(self.lengths[0]),
            self.lines[1].read(self.lengths[1]),
            self.lines[2].read(self.lengths[2]),
            self.lines[3].read(self.lengths[3]),
        ];
        let mixed = [
            (delayed[0] + delayed[1] + delayed[2] + delayed[3]) * 0.5,
            (delayed[0] - delayed[1] + delayed[2] - delayed[3]) * 0.5,
            (delayed[0] + delayed[1] - delayed[2] - delayed[3]) * 0.5,
            (delayed[0] - delayed[1] - delayed[2] + delayed[3]) * 0.5,
        ];
        let injection = [mono + side, mono - side, -mono + side, mono + side];
        let mut poisoned = false;
        for index in 0..4 {
            let feedback = self.damping[index].process(mixed[index]) * self.feedback[index];
            let write = injection[index] + feedback;
            if !write.is_finite() || write.abs() > EMERGENCY_LEVEL {
                poisoned = true;
                break;
            }
            self.lines[index].push(write);
        }
        if poisoned {
            self.reset();
            return frame.finite_or_silence();
        }

        let wet_left = (delayed[0] + delayed[1] - delayed[2] - delayed[3]) * 0.5;
        let wet_right = (delayed[0] - delayed[1] + delayed[2] - delayed[3]) * 0.5;
        let mid = (wet_left + wet_right) * 0.5;
        let side = (wet_left - wet_right) * 0.5 * self.width.next_value();
        let wet = self.wet.next_value();
        let dry = self.dry.next_value();
        StereoFrame::new(
            frame.left * dry + (mid + side) * wet,
            frame.right * dry + (mid - side) * wet,
        )
        .finite_or_silence()
    }

    pub(super) fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        match name {
            "type" => {
                self.voicing = value as usize;
                self.update_lengths_and_feedback();
            }
            "predelay_ms" => self
                .predelay_samples
                .set_target(value * self.sample_rate / 1_000.0, PARAMETER_SMOOTH_SAMPLES)?,
            "decay_seconds" => {
                self.decay_seconds = value;
                self.update_lengths_and_feedback();
            }
            "size_percent" => {
                self.size_percent = value;
                self.update_lengths_and_feedback();
            }
            "damping_percent" => {
                self.damping_percent = value;
                let frequency = damping_frequency(value);
                for filter in &mut self.damping {
                    filter.configure(frequency, self.sample_rate)?;
                }
            }
            "input_low_cut_hz" => {
                self.input_low_cut_left.configure(value, self.sample_rate)?;
                self.input_low_cut_right
                    .configure(value, self.sample_rate)?;
            }
            "width_percent" => self
                .width
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            "wet_percent" => self
                .wet
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            "dry_percent" => self
                .dry
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            _ => return Err(EffectError::new(format!("unknown Reverb parameter {name}"))),
        }
        Ok(())
    }

    fn update_lengths_and_feedback(&mut self) {
        let size = 0.7 + self.size_percent * 0.006;
        for index in 0..4 {
            let milliseconds = VOICING_LENGTHS_MS[self.voicing][index] * size;
            let samples = milliseconds * self.sample_rate / 1_000.0;
            self.lengths[index] = samples.clamp(1.0, self.lines[index].maximum_delay() as f32);
            let delay_seconds = self.lengths[index] / self.sample_rate;
            self.feedback[index] = 10.0_f32
                .powf(-3.0 * delay_seconds / self.decay_seconds)
                .clamp(0.0, 0.999_9);
        }
    }

    pub(super) fn reset(&mut self) {
        self.predelay_left.reset();
        self.predelay_right.reset();
        self.input_low_cut_left.reset();
        self.input_low_cut_right.reset();
        for line in &mut self.lines {
            line.reset();
        }
        for filter in &mut self.damping {
            filter.reset();
        }
    }
}

fn damping_frequency(percent: f32) -> f32 {
    18_000.0 * (1.0 - percent * 0.01) + 1_500.0 * percent * 0.01
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
            id: 34,
            kind: EffectKind::Reverb,
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
    fn feedback_is_mathematically_bounded_and_tracks_declared_rt60() {
        for voicing in 0..=2 {
            for decay in [0.2, 1.5, 8.0] {
                for size in [0.0, 50.0, 100.0] {
                    let reverb = Reverb::compile(
                        &effect([
                            ("type", voicing as f32),
                            ("decay_seconds", decay),
                            ("size_percent", size),
                        ]),
                        48_000,
                    )
                    .unwrap();
                    for index in 0..4 {
                        assert!((0.0..1.0).contains(&reverb.feedback[index]));
                        let cycles = decay * reverb.sample_rate / reverb.lengths[index];
                        let rt60_gain = reverb.feedback[index].powf(cycles);
                        assert!((20.0 * rt60_gain.log10() + 60.0).abs() < 0.01);
                    }
                }
            }
        }
    }

    #[test]
    fn predelay_and_fdn_lengths_bound_first_wet_arrival() {
        let mut slot = EffectSlot::compile(
            &effect([
                ("type", 0.0),
                ("predelay_ms", 20.0),
                ("size_percent", 50.0),
                ("wet_percent", 100.0),
                ("dry_percent", 0.0),
            ]),
            48_000,
            128,
        )
        .unwrap();
        let mut samples = vec![StereoFrame::SILENCE; 4_000];
        samples[0] = StereoFrame::new(1.0, 0.0);
        for chunk in samples.chunks_mut(73) {
            slot.process(chunk);
        }
        let first = samples
            .iter()
            .position(|frame| frame.left.abs() + frame.right.abs() > 1.0e-5)
            .unwrap();
        assert!((2_070..=2_090).contains(&first), "first arrival {first}");
    }

    #[test]
    fn three_voicings_are_distinct_stereo_and_decay_over_time() {
        let mut signatures = Vec::new();
        for voicing in 0..=2 {
            let mut slot = EffectSlot::compile(
                &effect([
                    ("type", voicing as f32),
                    ("predelay_ms", 0.0),
                    ("decay_seconds", 0.5),
                    ("wet_percent", 100.0),
                    ("dry_percent", 0.0),
                ]),
                48_000,
                128,
            )
            .unwrap();
            let mut samples = vec![StereoFrame::SILENCE; 48_000];
            samples[0] = StereoFrame::new(1.0, 0.25);
            for chunk in samples.chunks_mut(127) {
                slot.process(chunk);
            }
            let early = samples[2_000..12_000]
                .iter()
                .map(|frame| frame.left * frame.left + frame.right * frame.right)
                .sum::<f32>();
            let late = samples[38_000..48_000]
                .iter()
                .map(|frame| frame.left * frame.left + frame.right * frame.right)
                .sum::<f32>();
            assert!(early > late * 5.0, "early {early}, late {late}");
            assert!(samples
                .iter()
                .any(|frame| (frame.left - frame.right).abs() > 1.0e-4));
            signatures.push(samples[8_000]);
        }
        assert_ne!(signatures[0], signatures[1]);
        assert_ne!(signatures[1], signatures[2]);
    }

    #[test]
    fn silence_rates_limits_moves_reset_bypass_and_allocation_are_safe() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000, 384_000] {
            let mut slot = EffectSlot::compile(
                &effect([
                    ("type", 2.0),
                    ("decay_seconds", 8.0),
                    ("size_percent", 100.0),
                    ("damping_percent", 0.0),
                ]),
                sample_rate,
                128,
            )
            .unwrap();
            let mut silence = [StereoFrame::SILENCE; 512];
            assert_no_allocations(|| {
                for chunk in silence.chunks_mut(37) {
                    slot.process(chunk);
                }
            });
            assert_eq!(silence, [StereoFrame::SILENCE; 512]);
            for index in 0..30 {
                slot.set_parameter("type", (index % 3) as f32).unwrap();
                slot.set_parameter("size_percent", (index * 3) as f32)
                    .unwrap();
                let mut block = [StereoFrame::new(1.0, -1.0); 31];
                slot.process(&mut block);
                assert!(block
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
