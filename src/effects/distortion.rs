use super::{smooth, EffectError, PARAMETER_SMOOTH_SAMPLES};
use crate::audio_graph::EffectInstance;
use crate::dsp::{db_to_gain, DcBlocker, OnePole, OnePoleMode, SmoothedValue, StereoFrame};
use crate::effect_schema;

const DC_BLOCK_HZ: f32 = 10.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    SoftCubic,
    HardClip,
    AsymmetricDiodeLike,
}

impl Mode {
    fn from_parameter(value: f32) -> Self {
        match value as u8 {
            0 => Self::SoftCubic,
            1 => Self::HardClip,
            _ => Self::AsymmetricDiodeLike,
        }
    }
}

#[derive(Clone, Copy)]
struct ModeBranch {
    mode: Mode,
    dc_left: DcBlocker,
    dc_right: DcBlocker,
}

impl ModeBranch {
    fn new(mode: Mode, sample_rate: f32) -> Result<Self, EffectError> {
        Ok(Self {
            mode,
            dc_left: DcBlocker::new(DC_BLOCK_HZ, sample_rate)?,
            dc_right: DcBlocker::new(DC_BLOCK_HZ, sample_rate)?,
        })
    }

    fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.dc_left.reset();
        self.dc_right.reset();
    }

    #[inline]
    fn process(&mut self, frame: StereoFrame, bias: f32) -> StereoFrame {
        let left = transfer(self.mode, frame.left, bias);
        let right = transfer(self.mode, frame.right, bias);
        if self.mode == Mode::AsymmetricDiodeLike {
            StereoFrame::new(self.dc_left.process(left), self.dc_right.process(right))
        } else {
            StereoFrame::new(left, right)
        }
    }
}

pub(super) struct Distortion {
    sample_rate: f32,
    current: ModeBranch,
    next: ModeBranch,
    mode_mix: SmoothedValue,
    transitioning: bool,
    pending_mode: Option<Mode>,
    drive: SmoothedValue,
    bias: SmoothedValue,
    tone_left: OnePole,
    tone_right: OnePole,
    output: SmoothedValue,
    mix: SmoothedValue,
}

impl Distortion {
    pub(super) fn compile(effect: &EffectInstance, sample_rate: u32) -> Result<Self, EffectError> {
        let value = |name| {
            effect_schema::parameter(effect, name)
                .map_err(|error| EffectError::new(error.to_string()))
        };
        let sample_rate = sample_rate as f32;
        let mode = Mode::from_parameter(value("mode")?);
        let tone_hz = value("tone_hz")?;
        Ok(Self {
            sample_rate,
            current: ModeBranch::new(mode, sample_rate)?,
            next: ModeBranch::new(mode, sample_rate)?,
            mode_mix: smooth(0.0),
            transitioning: false,
            pending_mode: None,
            drive: smooth(db_to_gain(value("drive_db")?)?),
            bias: smooth(value("bias")?),
            tone_left: OnePole::new(OnePoleMode::LowPass, tone_hz, sample_rate)?,
            tone_right: OnePole::new(OnePoleMode::LowPass, tone_hz, sample_rate)?,
            output: smooth(db_to_gain(value("output_db")?)?),
            mix: smooth(value("mix_percent")? * 0.01),
        })
    }

    #[inline]
    pub(super) fn process(&mut self, frame: StereoFrame) -> StereoFrame {
        let drive = self.drive.next_value();
        let bias = self.bias.next_value();
        let driven = StereoFrame::new(frame.left * drive, frame.right * drive);
        let mut shaped = self.current.process(driven, bias);
        if self.transitioning {
            let next = self.next.process(driven, bias);
            let mix = self.mode_mix.next_value();
            shaped = StereoFrame::new(
                shaped.left + (next.left - shaped.left) * mix,
                shaped.right + (next.right - shaped.right) * mix,
            );
            if mix >= 1.0 {
                self.current = self.next;
                if let Some(mode) = self.pending_mode.take() {
                    self.begin_mode_transition(mode);
                } else {
                    self.transitioning = false;
                }
            }
        }
        let shaped = StereoFrame::new(
            self.tone_left.process(shaped.left),
            self.tone_right.process(shaped.right),
        );
        let output = self.output.next_value();
        let mix = self.mix.next_value();
        StereoFrame::new(
            frame.left + (shaped.left * output - frame.left) * mix,
            frame.right + (shaped.right * output - frame.right) * mix,
        )
        .finite_or_silence()
    }

    pub(super) fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        match name {
            "mode" => self.set_mode(Mode::from_parameter(value)),
            "drive_db" => {
                self.drive
                    .set_target(db_to_gain(value)?, PARAMETER_SMOOTH_SAMPLES)?;
            }
            "bias" => self.bias.set_target(value, PARAMETER_SMOOTH_SAMPLES)?,
            "tone_hz" => {
                self.tone_left.configure(value, self.sample_rate)?;
                self.tone_right.configure(value, self.sample_rate)?;
            }
            "output_db" => {
                self.output
                    .set_target(db_to_gain(value)?, PARAMETER_SMOOTH_SAMPLES)?;
            }
            "mix_percent" => self
                .mix
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            _ => {
                return Err(EffectError::new(format!(
                    "unknown Distortion parameter {name}"
                )))
            }
        }
        Ok(())
    }

    fn set_mode(&mut self, mode: Mode) {
        if self.transitioning {
            self.pending_mode = Some(mode);
        } else if mode != self.current.mode {
            self.begin_mode_transition(mode);
        }
    }

    fn begin_mode_transition(&mut self, mode: Mode) {
        self.next = self.current;
        self.next.set_mode(mode);
        if self.mode_mix.reset(0.0).is_ok()
            && self
                .mode_mix
                .set_target(1.0, PARAMETER_SMOOTH_SAMPLES)
                .is_ok()
        {
            self.transitioning = true;
        }
    }

    pub(super) fn reset(&mut self) {
        let mode = self.pending_mode.unwrap_or(if self.transitioning {
            self.next.mode
        } else {
            self.current.mode
        });
        self.current.set_mode(mode);
        self.next = self.current;
        let _ = self.mode_mix.reset(0.0);
        self.transitioning = false;
        self.tone_left.reset();
        self.tone_right.reset();
        self.pending_mode = None;
    }
}

#[inline]
fn transfer(mode: Mode, input: f32, bias: f32) -> f32 {
    match mode {
        Mode::SoftCubic => soft_cubic(input),
        Mode::HardClip => input.clamp(-1.0, 1.0),
        Mode::AsymmetricDiodeLike => {
            let input = input + bias;
            if input >= 0.0 {
                soft_cubic(input * 1.4)
            } else {
                -0.8 * soft_cubic(-input * 0.9)
            }
        }
    }
}

#[inline]
fn soft_cubic(input: f32) -> f32 {
    if input >= 1.0 {
        1.0
    } else if input <= -1.0 {
        -1.0
    } else {
        1.5 * (input - input * input * input / 3.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_graph::{EffectKind, EFFECT_FORMAT_VERSION};
    use crate::dsp::allocation_test::assert_no_allocations;
    use crate::effects::EffectSlot;
    use std::f32::consts::PI;

    fn effect(parameters: impl IntoIterator<Item = (&'static str, f32)>) -> EffectInstance {
        EffectInstance {
            id: 4,
            kind: EffectKind::Distortion,
            version: EFFECT_FORMAT_VERSION,
            bypass: false,
            parameters: parameters
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
            owned_memory_bytes: 0,
        }
    }

    fn harmonic(samples: &[f32], number: usize) -> f32 {
        let length = samples.len() as f32;
        let mut real = 0.0;
        let mut imaginary = 0.0;
        for (index, sample) in samples.iter().copied().enumerate() {
            let phase = 2.0 * PI * number as f32 * index as f32 / length;
            real += sample * phase.cos();
            imaginary -= sample * phase.sin();
        }
        2.0 * (real * real + imaginary * imaginary).sqrt() / length
    }

    fn spectral_bin(samples: &[f32], bin: usize) -> f32 {
        let length = samples.len() as f32;
        let mut real = 0.0;
        let mut imaginary = 0.0;
        for (index, sample) in samples.iter().copied().enumerate() {
            let phase = 2.0 * PI * bin as f32 * index as f32 / length;
            real += sample * phase.cos();
            imaginary -= sample * phase.sin();
        }
        2.0 * (real * real + imaginary * imaginary).sqrt() / length
    }

    #[test]
    fn named_transfer_curves_are_bounded_and_distinct() {
        for index in 0..=20_000 {
            let input = -10.0 + index as f32 * 0.001;
            for mode in [Mode::SoftCubic, Mode::HardClip, Mode::AsymmetricDiodeLike] {
                assert!(transfer(mode, input, 0.0).abs() <= 1.0);
            }
            assert!(
                (transfer(Mode::SoftCubic, input, 0.0) + transfer(Mode::SoftCubic, -input, 0.0))
                    .abs()
                    < 1.0e-6
            );
            assert!(
                (transfer(Mode::HardClip, input, 0.0) + transfer(Mode::HardClip, -input, 0.0))
                    .abs()
                    < 1.0e-6
            );
        }
        assert_eq!(transfer(Mode::HardClip, 0.5, 0.0), 0.5);
        assert!(transfer(Mode::SoftCubic, 0.5, 0.0) > 0.5);
        assert_ne!(
            transfer(Mode::AsymmetricDiodeLike, 0.5, 0.0),
            -transfer(Mode::AsymmetricDiodeLike, -0.5, 0.0)
        );
    }

    #[test]
    fn symmetric_cubic_has_third_but_negligible_even_harmonics() {
        let length = 4_096;
        let samples = (0..length)
            .map(|index| {
                let input = (2.0 * PI * index as f32 / length as f32).sin() * 0.8;
                transfer(Mode::SoftCubic, input, 0.0)
            })
            .collect::<Vec<_>>();
        assert!(harmonic(&samples, 3) > 0.02);
        assert!(harmonic(&samples, 2) < 1.0e-5);
        assert!(harmonic(&samples, 4) < 1.0e-5);

        let asymmetric = (0..length)
            .map(|index| {
                let input = (2.0 * PI * index as f32 / length as f32).sin() * 0.8;
                transfer(Mode::AsymmetricDiodeLike, input, 0.0)
            })
            .collect::<Vec<_>>();
        assert!(harmonic(&asymmetric, 2) > 0.01);
    }

    #[test]
    fn non_oversampled_high_frequency_alias_is_measured() {
        let length = 4_096;
        let fundamental_bin = 900;
        let third_harmonic_alias_bin = length - fundamental_bin * 3;
        let samples = (0..length)
            .map(|index| {
                let phase = 2.0 * PI * fundamental_bin as f32 * index as f32 / length as f32;
                transfer(Mode::SoftCubic, phase.sin() * 0.8, 0.0)
            })
            .collect::<Vec<_>>();
        let fundamental = spectral_bin(&samples, fundamental_bin);
        let alias = spectral_bin(&samples, third_harmonic_alias_bin);
        let alias_db = 20.0 * (alias / fundamental).log10();

        // At 48 kHz this models a 10.55 kHz input whose 31.64 kHz third
        // harmonic folds to 16.36 kHz. The explicit bound documents the
        // product cost of using this inexpensive transfer without oversampling.
        assert!((-24.1..=-23.8).contains(&alias_db), "{alias_db} dBc");
    }

    #[test]
    fn asymmetric_mode_automatically_rejects_dc() {
        let mut asymmetric = EffectSlot::compile(
            &effect([
                ("mode", 2.0),
                ("drive_db", 0.0),
                ("bias", 0.25),
                ("tone_hz", 18_000.0),
                ("output_db", 0.0),
            ]),
            48_000,
            128,
        )
        .unwrap();
        let mut last = StereoFrame::SILENCE;
        for _ in 0..400 {
            let mut block = [StereoFrame::new(0.25, 0.25); 128];
            asymmetric.process(&mut block);
            last = block[127];
        }
        assert!(last.left.abs() < 0.001 && last.right.abs() < 0.001);

        let mut symmetric = EffectSlot::compile(
            &effect([
                ("mode", 1.0),
                ("drive_db", 0.0),
                ("tone_hz", 18_000.0),
                ("output_db", 0.0),
            ]),
            48_000,
            128,
        )
        .unwrap();
        let mut block = [StereoFrame::new(0.25, 0.25); 128];
        for _ in 0..400 {
            symmetric.process(&mut block);
            block.fill(StereoFrame::new(0.25, 0.25));
        }
        symmetric.process(&mut block);
        assert!(block[127].left > 0.24);
    }

    #[test]
    fn random_limits_modes_chunks_reset_bypass_and_allocation_are_safe() {
        let configured = effect([
            ("mode", 2.0),
            ("drive_db", 30.0),
            ("bias", 0.5),
            ("tone_hz", 800.0),
            ("output_db", 0.0),
            ("mix_percent", 100.0),
        ]);
        let input = (0..4_096)
            .map(|index| {
                let value = ((index * 43 % 257) as f32 / 128.0) - 1.0;
                StereoFrame::new(value, -value)
            })
            .collect::<Vec<_>>();
        let mut whole = EffectSlot::compile(&configured, 48_000, 256).unwrap();
        let mut expected = input.clone();
        assert_no_allocations(|| {
            for chunk in expected.chunks_mut(256) {
                whole.process(chunk);
            }
        });
        let mut odd = EffectSlot::compile(&configured, 48_000, 256).unwrap();
        let mut actual = input;
        for chunk in actual.chunks_mut(37) {
            odd.process(chunk);
        }
        assert_eq!(actual, expected);
        assert!(actual.iter().all(|frame| {
            frame.left.is_finite()
                && frame.right.is_finite()
                && frame.left.abs() <= 2.0
                && frame.right.abs() <= 2.0
        }));

        for index in 0..100 {
            odd.set_parameter("mode", (index % 3) as f32).unwrap();
            odd.set_parameter("drive_db", (index % 31) as f32).unwrap();
            let mut block = [StereoFrame::new(1.0, -1.0); 17];
            odd.process(&mut block);
            assert!(block
                .iter()
                .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
        }
        odd.reset();
        odd.set_bypass(true).unwrap();
        let mut dry = [StereoFrame::new(0.25, -0.5); 256];
        odd.process(&mut dry);
        assert_eq!(dry[255], StereoFrame::new(0.25, -0.5));
    }

    #[test]
    fn all_supported_sample_rates_compile_and_silence_stays_silent() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000, 384_000] {
            for mode in 0..=2 {
                let mut slot = EffectSlot::compile(
                    &effect([("mode", mode as f32), ("tone_hz", 18_000.0)]),
                    sample_rate,
                    64,
                )
                .unwrap();
                let mut silence = [StereoFrame::SILENCE; 64];
                slot.process(&mut silence);
                assert_eq!(silence, [StereoFrame::SILENCE; 64]);
            }
        }
    }
}
