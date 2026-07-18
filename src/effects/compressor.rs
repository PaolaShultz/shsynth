use super::{smooth, EffectError, PARAMETER_SMOOTH_SAMPLES};
use crate::audio_graph::EffectInstance;
use crate::dsp::{db_to_gain, finite_or_zero, OnePole, OnePoleMode, SmoothedValue, StereoFrame};
use crate::effect_schema;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

const MIN_DETECTOR_DB: f32 = -120.0;
const MIN_GAIN_DB: f32 = -96.0;
const GAIN_TABLE_STEPS: usize = 2_048;

#[derive(Debug, Default)]
pub struct AtomicGainReduction {
    bits: AtomicU32,
}

impl AtomicGainReduction {
    fn publish(&self, decibels: f32) {
        self.bits
            .store(decibels.max(0.0).to_bits(), Ordering::Release);
    }

    pub fn load(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Acquire))
    }
}

struct GainTable {
    values: Box<[f32]>,
}

impl GainTable {
    fn new() -> Result<Self, EffectError> {
        let mut values = Vec::with_capacity(GAIN_TABLE_STEPS + 1);
        for index in 0..=GAIN_TABLE_STEPS {
            let db = MIN_GAIN_DB * (1.0 - index as f32 / GAIN_TABLE_STEPS as f32);
            values.push(db_to_gain(db)?);
        }
        Ok(Self {
            values: values.into_boxed_slice(),
        })
    }

    #[inline]
    fn gain(&self, db: f32) -> f32 {
        let normalized =
            ((db.clamp(MIN_GAIN_DB, 0.0) - MIN_GAIN_DB) / -MIN_GAIN_DB) * GAIN_TABLE_STEPS as f32;
        let first = normalized.floor() as usize;
        let second = (first + 1).min(GAIN_TABLE_STEPS);
        let fraction = normalized - first as f32;
        self.values[first] + (self.values[second] - self.values[first]) * fraction
    }
}

pub(super) struct Compressor {
    sample_rate: f32,
    threshold_db: f32,
    ratio: f32,
    knee_db: f32,
    attack_ms: f32,
    release_ms: f32,
    attack_coefficient: f32,
    release_coefficient: f32,
    gain_change_db: f32,
    makeup: SmoothedValue,
    mix: SmoothedValue,
    sidechain_left: OnePole,
    sidechain_right: OnePole,
    gain_table: GainTable,
    published_gain_reduction: Arc<AtomicGainReduction>,
}

impl Compressor {
    pub(super) fn compile(effect: &EffectInstance, sample_rate: u32) -> Result<Self, EffectError> {
        let value = |name| {
            effect_schema::parameter(effect, name)
                .map_err(|error| EffectError::new(error.to_string()))
        };
        let sample_rate = sample_rate as f32;
        let attack_ms = value("attack_ms")?;
        let release_ms = value("release_ms")?;
        let sidechain_hz = value("sidechain_highpass_hz")?;
        Ok(Self {
            sample_rate,
            threshold_db: value("threshold_db")?,
            ratio: value("ratio")?,
            knee_db: value("knee_db")?,
            attack_ms,
            release_ms,
            attack_coefficient: time_coefficient(attack_ms, sample_rate),
            release_coefficient: time_coefficient(release_ms, sample_rate),
            gain_change_db: 0.0,
            makeup: smooth(db_to_gain(value("makeup_db")?)?),
            mix: smooth(value("mix_percent")? * 0.01),
            sidechain_left: OnePole::new(OnePoleMode::HighPass, sidechain_hz, sample_rate)?,
            sidechain_right: OnePole::new(OnePoleMode::HighPass, sidechain_hz, sample_rate)?,
            gain_table: GainTable::new()?,
            published_gain_reduction: Arc::new(AtomicGainReduction::default()),
        })
    }

    #[inline]
    pub(super) fn process(&mut self, frame: StereoFrame) -> StereoFrame {
        let left = self.sidechain_left.process(frame.left).abs();
        let right = self.sidechain_right.process(frame.right).abs();
        let detector = left.max(right);
        let level_db = if detector > 0.0 {
            20.0 * detector.log10()
        } else {
            MIN_DETECTOR_DB
        };
        let target = curve_gain_db(level_db, self.threshold_db, self.ratio, self.knee_db);
        self.follow_gain_change(target);
        let gain = self.gain_table.gain(self.gain_change_db) * self.makeup.next_value();
        let mix = self.mix.next_value();
        StereoFrame::new(
            frame.left + (frame.left * gain - frame.left) * mix,
            frame.right + (frame.right * gain - frame.right) * mix,
        )
        .finite_or_silence()
    }

    #[inline]
    fn follow_gain_change(&mut self, target_db: f32) {
        let coefficient = if target_db < self.gain_change_db {
            self.attack_coefficient
        } else {
            self.release_coefficient
        };
        self.gain_change_db =
            finite_or_zero(target_db + coefficient * (self.gain_change_db - target_db))
                .clamp(MIN_GAIN_DB, 0.0);
    }

    pub(super) fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        match name {
            "threshold_db" => self.threshold_db = value,
            "ratio" => self.ratio = value,
            "knee_db" => self.knee_db = value,
            "attack_ms" => {
                self.attack_ms = value;
                self.attack_coefficient = time_coefficient(value, self.sample_rate);
            }
            "release_ms" => {
                self.release_ms = value;
                self.release_coefficient = time_coefficient(value, self.sample_rate);
            }
            "makeup_db" => {
                self.makeup
                    .set_target(db_to_gain(value)?, PARAMETER_SMOOTH_SAMPLES)?;
            }
            "mix_percent" => {
                self.mix
                    .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?;
            }
            "sidechain_highpass_hz" => {
                self.sidechain_left.configure(value, self.sample_rate)?;
                self.sidechain_right.configure(value, self.sample_rate)?;
            }
            _ => {
                return Err(EffectError::new(format!(
                    "unknown Compressor parameter {name}"
                )))
            }
        }
        Ok(())
    }

    pub(super) fn reset(&mut self) {
        self.gain_change_db = 0.0;
        self.sidechain_left.reset();
        self.sidechain_right.reset();
        self.published_gain_reduction.publish(0.0);
    }

    pub(super) fn gain_reduction(&self) -> Arc<AtomicGainReduction> {
        Arc::clone(&self.published_gain_reduction)
    }

    pub(super) fn publish(&self) {
        self.published_gain_reduction.publish(-self.gain_change_db);
    }
}

fn time_coefficient(milliseconds: f32, sample_rate: f32) -> f32 {
    (-1.0 / (milliseconds * 0.001 * sample_rate)).exp()
}

fn curve_gain_db(level_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
    let slope = ratio.recip() - 1.0;
    let over = level_db - threshold_db;
    if knee_db > 0.0 && over.abs() <= knee_db * 0.5 {
        slope * (over + knee_db * 0.5).powi(2) / (2.0 * knee_db)
    } else if over > 0.0 {
        slope * over
    } else {
        0.0
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
            id: 3,
            kind: EffectKind::Compressor,
            version: EFFECT_FORMAT_VERSION,
            bypass: false,
            parameters: parameters
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
            owned_memory_bytes: 0,
        }
    }

    #[test]
    fn hard_and_soft_knee_curves_match_the_declared_equations() {
        assert_eq!(curve_gain_db(-40.0, -20.0, 4.0, 0.0), 0.0);
        assert_eq!(curve_gain_db(-20.0, -20.0, 4.0, 0.0), 0.0);
        assert!((curve_gain_db(0.0, -20.0, 4.0, 0.0) + 15.0).abs() < 1.0e-6);

        let below = curve_gain_db(-26.0, -20.0, 4.0, 12.0);
        let center = curve_gain_db(-20.0, -20.0, 4.0, 12.0);
        let above = curve_gain_db(-14.0, -20.0, 4.0, 12.0);
        assert_eq!(below, 0.0);
        assert!((center + 1.125).abs() < 1.0e-6);
        assert!((above + 4.5).abs() < 1.0e-6);
        let epsilon = 0.001;
        assert!(
            (curve_gain_db(-26.0 - epsilon, -20.0, 4.0, 12.0)
                - curve_gain_db(-26.0 + epsilon, -20.0, 4.0, 12.0))
            .abs()
                < 0.001
        );
    }

    #[test]
    fn callback_gain_lookup_stays_within_one_thousandth_db() {
        let table = GainTable::new().unwrap();
        let mut maximum_error = 0.0_f32;
        for index in 0..=96_000 {
            let expected_db = -96.0 + index as f32 * 0.001;
            let actual_db = 20.0 * table.gain(expected_db).log10();
            maximum_error = maximum_error.max((actual_db - expected_db).abs());
        }
        assert!(maximum_error < 0.001, "{maximum_error} dB");
    }

    #[test]
    fn attack_and_release_are_monotonic_and_hit_their_time_constants() {
        let mut compressor = Compressor::compile(
            &effect([("attack_ms", 10.0), ("release_ms", 100.0)]),
            48_000,
        )
        .unwrap();
        let target = -12.0;
        let mut previous = 0.0;
        for _ in 0..480 {
            compressor.follow_gain_change(target);
            assert!(compressor.gain_change_db <= previous);
            previous = compressor.gain_change_db;
        }
        assert!((compressor.gain_change_db - target * (1.0 - (-1.0_f32).exp())).abs() < 0.01);
        for _ in 0..4_800 {
            let before = compressor.gain_change_db;
            compressor.follow_gain_change(0.0);
            assert!(compressor.gain_change_db >= before);
        }
        assert!(compressor.gain_change_db.abs() < 4.5);
    }

    #[test]
    fn louder_channel_drives_one_linked_gain_and_meter() {
        let configured = effect([
            ("threshold_db", -24.0),
            ("ratio", 4.0),
            ("knee_db", 0.0),
            ("attack_ms", 0.1),
            ("release_ms", 20.0),
        ]);
        let mut slot = EffectSlot::compile(&configured, 48_000, 128).unwrap();
        let meter = slot.meters().gain_reduction.unwrap();
        let mut left_ratio = 0.0;
        let mut right_ratio = 0.0;
        for block_index in 0..400 {
            let mut block = [StereoFrame::SILENCE; 128];
            for (index, frame) in block.iter_mut().enumerate() {
                let phase = 2.0 * PI * 1_000.0 * (block_index * 128 + index) as f32 / 48_000.0;
                let sample = phase.sin();
                *frame = StereoFrame::new(sample, sample * 0.1);
            }
            let input = block;
            slot.process(&mut block);
            if block_index == 399 {
                let index = input
                    .iter()
                    .position(|frame| frame.left.abs() > 0.9)
                    .unwrap();
                left_ratio = block[index].left / input[index].left;
                right_ratio = block[index].right / input[index].right;
            }
        }
        assert!((left_ratio - right_ratio).abs() < 1.0e-6);
        assert!(left_ratio < 0.5);
        assert!(meter.load() > 6.0);
    }

    #[test]
    fn zero_lookahead_preserves_the_first_impulse_sample() {
        let mut slot = EffectSlot::compile(
            &effect([
                ("threshold_db", -48.0),
                ("ratio", 20.0),
                ("knee_db", 0.0),
                ("attack_ms", 100.0),
            ]),
            48_000,
            64,
        )
        .unwrap();
        let mut block = [StereoFrame::SILENCE; 64];
        block[0] = StereoFrame::new(1.0, -1.0);
        slot.process(&mut block);
        assert!(block[0].left > 0.99);
        assert!(block[1..]
            .iter()
            .all(|frame| *frame == StereoFrame::SILENCE));
    }

    #[test]
    fn silence_random_limits_reset_chunks_and_allocation_are_safe() {
        let configured = effect([
            ("threshold_db", -30.0),
            ("ratio", 8.0),
            ("knee_db", 8.0),
            ("attack_ms", 2.0),
            ("release_ms", 80.0),
            ("makeup_db", 6.0),
            ("mix_percent", 75.0),
            ("sidechain_highpass_hz", 100.0),
        ]);
        let input = (0..4_096)
            .map(|index| {
                let value = ((index * 31 % 251) as f32 / 125.0) - 1.0;
                StereoFrame::new(value, value * -0.31)
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
        assert!(actual
            .iter()
            .all(|frame| frame.left.is_finite() && frame.right.is_finite()));

        odd.reset();
        let mut silence = [StereoFrame::SILENCE; 256];
        odd.process(&mut silence);
        assert_eq!(silence, [StereoFrame::SILENCE; 256]);
        assert_eq!(odd.meters().gain_reduction.unwrap().load(), 0.0);
    }

    #[test]
    fn every_sample_rate_and_rapid_parameter_move_remains_finite() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000, 384_000] {
            let mut slot = EffectSlot::compile(&effect([]), sample_rate, 63).unwrap();
            for index in 0..200 {
                slot.set_parameter("threshold_db", -48.0 + (index % 49) as f32)
                    .unwrap();
                slot.set_parameter("ratio", 1.0 + (index % 20) as f32)
                    .unwrap();
                slot.set_parameter("knee_db", (index % 13) as f32).unwrap();
                let mut block = [StereoFrame::new(1.0, -1.0); 63];
                slot.process(&mut block);
                assert!(block
                    .iter()
                    .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
            }
            slot.set_bypass(true).unwrap();
            let mut block = [StereoFrame::new(0.25, -0.5); 2_000];
            slot.process(&mut block);
            assert_eq!(block[1_999], StereoFrame::new(0.25, -0.5));
        }
    }
}
