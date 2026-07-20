//! Fixed three-source final performance bus.
//!
//! Construction and control changes happen away from the JACK callback. The
//! callback only reads atomics, advances bounded smoothing/delay state, and
//! publishes lock-free meters.

use crate::dsp::{
    db_to_gain, AtomicMeter, MeterAccumulator, MeterSnapshot, SmoothedValue, StereoFrame,
};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

pub const SOURCE_COUNT: usize = 3;
pub const SOURCE_GAIN_MIN_DB: f32 = -60.0;
pub const SOURCE_GAIN_MAX_DB: f32 = 6.0;
pub const MASTER_GAIN_MIN_DB: f32 = -60.0;
pub const MASTER_GAIN_MAX_DB: f32 = 0.0;
pub const DEFAULT_SOURCE_GAIN_DB: f32 = -6.0;
pub const LIMITER_CEILING_DBFS: f32 = -1.0;
pub const LIMITER_KNEE_DB: f32 = 3.0;
pub const LIMITER_LOOKAHEAD_SECONDS: f32 = 0.0025;
pub const LIMITER_RELEASE_SECONDS: f32 = 0.100;
const GAIN_SMOOTH_SECONDS: f32 = 0.010;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusSource {
    Synth = 0,
    Loop = 1,
    Input = 2,
}

impl BusSource {
    pub const ALL: [Self; SOURCE_COUNT] = [Self::Synth, Self::Loop, Self::Input];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Synth => "SYNTH",
            Self::Loop => "LOOP",
            Self::Input => "INPUT",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

struct AtomicFader {
    gain_db: AtomicU32,
    muted: AtomicBool,
}

impl AtomicFader {
    fn new(gain_db: f32) -> Self {
        Self {
            gain_db: AtomicU32::new(gain_db.to_bits()),
            muted: AtomicBool::new(false),
        }
    }

    fn gain_db(&self) -> f32 {
        f32::from_bits(self.gain_db.load(Ordering::Acquire))
    }
}

pub struct BusControls {
    sources: [AtomicFader; SOURCE_COUNT],
    master: AtomicFader,
}

impl Default for BusControls {
    fn default() -> Self {
        Self {
            sources: std::array::from_fn(|_| AtomicFader::new(DEFAULT_SOURCE_GAIN_DB)),
            master: AtomicFader::new(0.0),
        }
    }
}

impl BusControls {
    pub fn source_gain_db(&self, source: BusSource) -> f32 {
        self.sources[source.index()].gain_db()
    }

    pub fn set_source_gain_db(&self, source: BusSource, gain_db: f32) -> bool {
        if !gain_db.is_finite() || !(SOURCE_GAIN_MIN_DB..=SOURCE_GAIN_MAX_DB).contains(&gain_db) {
            return false;
        }
        self.sources[source.index()]
            .gain_db
            .store(gain_db.to_bits(), Ordering::Release);
        true
    }

    pub fn source_muted(&self, source: BusSource) -> bool {
        self.sources[source.index()].muted.load(Ordering::Acquire)
    }

    pub fn set_source_muted(&self, source: BusSource, muted: bool) {
        self.sources[source.index()]
            .muted
            .store(muted, Ordering::Release);
    }

    pub fn master_gain_db(&self) -> f32 {
        self.master.gain_db()
    }

    pub fn set_master_gain_db(&self, gain_db: f32) -> bool {
        if !gain_db.is_finite() || !(MASTER_GAIN_MIN_DB..=MASTER_GAIN_MAX_DB).contains(&gain_db) {
            return false;
        }
        self.master
            .gain_db
            .store(gain_db.to_bits(), Ordering::Release);
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FinalBusMeterSnapshot {
    pub limiter_input: MeterSnapshot,
    pub output: MeterSnapshot,
    pub limiter_gain_reduction_db: f32,
}

pub struct FinalBusMeters {
    limiter_input: AtomicMeter,
    output: AtomicMeter,
    gain_reduction_db: AtomicU32,
}

impl Default for FinalBusMeters {
    fn default() -> Self {
        Self {
            limiter_input: AtomicMeter::default(),
            output: AtomicMeter::default(),
            gain_reduction_db: AtomicU32::new(0.0f32.to_bits()),
        }
    }
}

impl FinalBusMeters {
    pub fn snapshot(&self) -> FinalBusMeterSnapshot {
        let gain_reduction_db = f32::from_bits(self.gain_reduction_db.load(Ordering::Acquire));
        FinalBusMeterSnapshot {
            limiter_input: self.limiter_input.load(),
            output: self.output.load(),
            limiter_gain_reduction_db: if gain_reduction_db.is_finite() {
                gain_reduction_db.clamp(0.0, 160.0)
            } else {
                0.0
            },
        }
    }

    pub fn clear(&self) {
        self.limiter_input.publish(MeterSnapshot::default());
        self.output.publish(MeterSnapshot::default());
        self.gain_reduction_db
            .store(0.0f32.to_bits(), Ordering::Release);
    }
}

struct RuntimeFader {
    value: SmoothedValue,
    last_target: f32,
    smoothing_samples: u32,
}

impl RuntimeFader {
    fn new(gain_db: f32, sample_rate: u32) -> Result<Self, String> {
        let gain = db_to_gain(gain_db).map_err(|error| error.to_string())?;
        Ok(Self {
            value: SmoothedValue::new(gain).map_err(|error| error.to_string())?,
            last_target: gain,
            smoothing_samples: ((sample_rate as f32 * GAIN_SMOOTH_SECONDS).round() as u32).max(1),
        })
    }

    #[inline]
    fn refresh(&mut self, gain_db: f32, muted: bool) {
        let target = if muted {
            0.0
        } else {
            db_to_gain(gain_db).unwrap_or(0.0)
        };
        if target != self.last_target {
            if self
                .value
                .set_target(target, self.smoothing_samples)
                .is_err()
            {
                let _ = self.value.reset(0.0);
            }
            self.last_target = target;
        }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        self.value.next_value()
    }
}

/// Stereo-linked, sample-peak, soft-knee limiter with a fixed 2.5 ms target
/// lookahead. This does not detect inter-sample peaks and is not true-peak.
pub struct FinalLimiter {
    delay: Box<[StereoFrame]>,
    write: usize,
    lookahead_samples: usize,
    ceiling: f32,
    gain: f32,
    held_gain: f32,
    hold_remaining: usize,
    release_step: f32,
}

impl FinalLimiter {
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err("unsupported limiter sample rate".into());
        }
        let lookahead_samples =
            ((sample_rate as f32 * LIMITER_LOOKAHEAD_SECONDS).round() as usize).max(1);
        let ceiling = db_to_gain(LIMITER_CEILING_DBFS).map_err(|error| error.to_string())?;
        let release_samples = (sample_rate as f32 * LIMITER_RELEASE_SECONDS).max(1.0);
        Ok(Self {
            delay: vec![StereoFrame::SILENCE; lookahead_samples].into_boxed_slice(),
            write: 0,
            lookahead_samples,
            ceiling,
            gain: 1.0,
            held_gain: 1.0,
            hold_remaining: 0,
            release_step: 1.0 - (-1.0 / release_samples).exp(),
        })
    }

    #[cfg(test)]
    pub fn lookahead_samples(&self) -> usize {
        self.lookahead_samples
    }

    #[cfg(test)]
    pub fn lookahead_seconds(&self, sample_rate: u32) -> f64 {
        self.lookahead_samples as f64 / f64::from(sample_rate)
    }

    #[cfg(test)]
    pub fn ceiling_linear(&self) -> f32 {
        self.ceiling
    }

    pub fn reset(&mut self) {
        self.delay.fill(StereoFrame::SILENCE);
        self.write = 0;
        self.gain = 1.0;
        self.held_gain = 1.0;
        self.hold_remaining = 0;
    }

    fn required_gain(&self, detector: f32) -> f32 {
        if !detector.is_finite() || detector <= 0.0 {
            return 1.0;
        }
        let input_db = 20.0 * detector.log10();
        let lower = LIMITER_CEILING_DBFS - LIMITER_KNEE_DB * 0.5;
        let upper = LIMITER_CEILING_DBFS + LIMITER_KNEE_DB * 0.5;
        let output_db = if input_db <= lower {
            input_db
        } else if input_db >= upper {
            LIMITER_CEILING_DBFS
        } else {
            let into_knee = input_db - lower;
            input_db - into_knee * into_knee / (2.0 * LIMITER_KNEE_DB)
        };
        10.0f32.powf((output_db - input_db) / 20.0).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn process(&mut self, input: StereoFrame) -> (StereoFrame, f32) {
        if !self.gain.is_finite()
            || !(0.0..=1.0).contains(&self.gain)
            || !self.held_gain.is_finite()
            || !(0.0..=1.0).contains(&self.held_gain)
            || self.write >= self.delay.len()
            || self.hold_remaining > self.lookahead_samples
        {
            self.reset();
        }
        let input = input.finite_or_silence();
        let delayed = self.delay[self.write].finite_or_silence();
        self.delay[self.write] = input;
        self.write += 1;
        if self.write == self.delay.len() {
            self.write = 0;
        }

        let detector = input.left.abs().max(input.right.abs());
        let required = self.required_gain(detector);
        if required < self.held_gain {
            self.held_gain = required;
            self.gain = self.gain.min(required);
            self.hold_remaining = self.lookahead_samples;
        } else if self.hold_remaining > 0 {
            self.hold_remaining -= 1;
            self.gain = self.gain.min(self.held_gain);
        } else {
            self.held_gain = 1.0;
            self.gain += (1.0 - self.gain) * self.release_step;
        }
        self.gain = self.gain.clamp(0.0, 1.0);
        let output = StereoFrame::new(
            (delayed.left * self.gain).clamp(-self.ceiling, self.ceiling),
            (delayed.right * self.gain).clamp(-self.ceiling, self.ceiling),
        )
        .finite_or_silence();
        let reduction = if self.gain > 0.0 {
            (-20.0 * self.gain.log10()).clamp(0.0, 160.0)
        } else {
            160.0
        };
        (output, reduction)
    }
}

pub struct FinalBusProcessor {
    controls: Arc<BusControls>,
    meters: Arc<FinalBusMeters>,
    source_faders: [RuntimeFader; SOURCE_COUNT],
    master_fader: RuntimeFader,
    limiter: FinalLimiter,
    limiter_input_meter: MeterAccumulator,
    output_meter: MeterAccumulator,
}

impl FinalBusProcessor {
    pub fn new(
        sample_rate: u32,
        maximum_frames: usize,
        controls: Arc<BusControls>,
        meters: Arc<FinalBusMeters>,
    ) -> Result<Self, String> {
        let source_fader = |source| {
            let gain_db = if controls.source_muted(source) {
                SOURCE_GAIN_MIN_DB
            } else {
                controls.source_gain_db(source)
            };
            RuntimeFader::new(gain_db, sample_rate)
        };
        let source_faders = [
            source_fader(BusSource::Synth)?,
            source_fader(BusSource::Loop)?,
            source_fader(BusSource::Input)?,
        ];
        let master_fader = RuntimeFader::new(controls.master_gain_db(), sample_rate)?;
        Ok(Self {
            source_faders,
            master_fader,
            limiter: FinalLimiter::new(sample_rate)?,
            limiter_input_meter: MeterAccumulator::new(maximum_frames)
                .map_err(|error| error.to_string())?,
            output_meter: MeterAccumulator::new(maximum_frames)
                .map_err(|error| error.to_string())?,
            controls,
            meters,
        })
    }

    #[cfg(test)]
    pub fn lookahead_samples(&self) -> usize {
        self.limiter.lookahead_samples()
    }

    #[inline]
    pub fn process_source(&mut self, source: BusSource, frames: &mut [StereoFrame]) {
        let index = source.index();
        self.source_faders[index].refresh(
            self.controls.source_gain_db(source),
            self.controls.source_muted(source),
        );
        for frame in frames {
            let gain = self.source_faders[index].next();
            *frame = StereoFrame::new(frame.left * gain, frame.right * gain).finite_or_silence();
        }
    }

    #[inline]
    pub fn process_final(&mut self, frames: &mut [StereoFrame]) {
        self.master_fader
            .refresh(self.controls.master_gain_db(), false);
        let mut maximum_reduction = 0.0f32;
        for frame in frames.iter_mut() {
            let master = self.master_fader.next();
            let input =
                StereoFrame::new(frame.left * master, frame.right * master).finite_or_silence();
            self.limiter_input_meter.process(input);
            let (output, reduction) = self.limiter.process(input);
            maximum_reduction = maximum_reduction.max(reduction);
            *frame = self.output_meter.process(output);
        }
        self.meters
            .limiter_input
            .publish(self.limiter_input_meter.snapshot_and_clear_peak());
        self.meters
            .output
            .publish(self.output_meter.snapshot_and_clear_peak());
        self.meters
            .gain_reduction_db
            .store(maximum_reduction.to_bits(), Ordering::Release);
    }

    pub fn reset(&mut self) {
        self.limiter.reset();
        self.limiter_input_meter.reset();
        self.output_meter.reset();
        self.meters.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::allocation_test::assert_no_allocations;

    fn processor(rate: u32, frames: usize) -> (FinalBusProcessor, Arc<BusControls>) {
        let controls = Arc::new(BusControls::default());
        for source in BusSource::ALL {
            assert!(controls.set_source_gain_db(source, 0.0));
        }
        let processor = FinalBusProcessor::new(
            rate,
            frames,
            Arc::clone(&controls),
            Arc::new(FinalBusMeters::default()),
        )
        .unwrap();
        (processor, controls)
    }

    #[test]
    fn limiter_contract_and_supported_rate_latency() {
        let at_44 = FinalLimiter::new(44_100).unwrap();
        let at_48 = FinalLimiter::new(48_000).unwrap();
        assert_eq!(at_44.lookahead_samples(), 110);
        assert_eq!(at_48.lookahead_samples(), 120);
        assert!((at_44.lookahead_seconds(44_100) - 0.002_494_331).abs() < 1e-8);
        assert_eq!(at_48.lookahead_seconds(48_000), 0.0025);
        assert!((at_48.ceiling_linear() - 10.0f32.powf(-1.0 / 20.0)).abs() < 1e-7);
    }

    #[test]
    fn limiter_impulse_is_delayed_linked_and_below_ceiling() {
        let mut limiter = FinalLimiter::new(48_000).unwrap();
        let delay = limiter.lookahead_samples();
        let mut output = Vec::new();
        for index in 0..=delay + 2 {
            let input = if index == 0 {
                StereoFrame::new(2.0, 1.0)
            } else {
                StereoFrame::SILENCE
            };
            output.push(limiter.process(input).0);
        }
        assert!(output[..delay]
            .iter()
            .all(|frame| *frame == StereoFrame::SILENCE));
        let impulse = output[delay];
        assert!(impulse.left.abs() <= limiter.ceiling_linear() + 1e-6);
        assert!((impulse.right / impulse.left - 0.5).abs() < 1e-5);
    }

    #[test]
    fn soft_knee_starts_three_db_wide() {
        let limiter = FinalLimiter::new(48_000).unwrap();
        let below = limiter.required_gain(10.0f32.powf((-2.6) / 20.0));
        let middle = limiter.required_gain(10.0f32.powf((-1.0) / 20.0));
        let above = limiter.required_gain(10.0f32.powf(0.6 / 20.0));
        assert_eq!(below, 1.0);
        assert!(middle < 1.0 && middle > above);
        assert!((above - 10.0f32.powf(-1.6 / 20.0)).abs() < 1e-6);
    }

    #[test]
    fn three_sources_sum_without_swap_bleed_omission_or_duplication() {
        let (mut bus, _) = processor(48_000, 256);
        let mut synth = [StereoFrame::new(0.01, 0.02); 256];
        let mut loop_frames = [StereoFrame::new(0.03, 0.04); 256];
        let mut input = [StereoFrame::new(0.05, 0.06); 256];
        bus.process_source(BusSource::Synth, &mut synth);
        bus.process_source(BusSource::Loop, &mut loop_frames);
        bus.process_source(BusSource::Input, &mut input);
        let mut sum = std::array::from_fn::<_, 256, _>(|index| {
            StereoFrame::new(
                synth[index].left + loop_frames[index].left + input[index].left,
                synth[index].right + loop_frames[index].right + input[index].right,
            )
        });
        bus.process_final(&mut sum);
        let delay = bus.lookahead_samples();
        assert!((sum[delay].left - 0.09).abs() < 1e-6);
        assert!((sum[delay].right - 0.12).abs() < 1e-6);
    }

    #[test]
    fn synth_loop_and_input_each_reach_only_the_expected_stereo_channels() {
        let expected = [
            (BusSource::Synth, StereoFrame::new(0.031, -0.047)),
            (BusSource::Loop, StereoFrame::new(-0.053, 0.071)),
            (BusSource::Input, StereoFrame::new(0.089, 0.097)),
        ];
        for (active, identity) in expected {
            let (mut bus, _) = processor(48_000, 256);
            let mut sum = [StereoFrame::SILENCE; 256];
            for source in BusSource::ALL {
                let mut frames = if source == active {
                    [identity; 256]
                } else {
                    [StereoFrame::SILENCE; 256]
                };
                bus.process_source(source, &mut frames);
                for (output, frame) in sum.iter_mut().zip(frames) {
                    output.left += frame.left;
                    output.right += frame.right;
                }
            }
            bus.process_final(&mut sum);
            let actual = sum[bus.lookahead_samples()];
            assert!((actual.left - identity.left).abs() < 1e-6);
            assert!((actual.right - identity.right).abs() < 1e-6);
        }
    }

    #[test]
    fn source_mute_and_master_level_are_smoothed_and_bounded() {
        let (mut bus, controls) = processor(48_000, 1024);
        assert!(!controls.set_source_gain_db(BusSource::Synth, 7.0));
        assert!(!controls.set_master_gain_db(0.1));
        controls.set_source_muted(BusSource::Synth, true);
        assert!(controls.set_master_gain_db(-6.0));
        let mut source = [StereoFrame::new(0.25, -0.25); 1024];
        bus.process_source(BusSource::Synth, &mut source);
        assert!(source[0].left > source[1023].left);
        assert!(source[1023].left.abs() < 1e-7);
    }

    #[test]
    fn finite_recovery_and_callback_processing_allocate_nothing() {
        let (mut bus, _) = processor(48_000, 256);
        let mut source = [StereoFrame::new(f32::NAN, f32::INFINITY); 256];
        let mut output = [StereoFrame::SILENCE; 256];
        assert_no_allocations(|| {
            bus.process_source(BusSource::Input, &mut source);
            output.copy_from_slice(&source);
            bus.process_final(&mut output);
        });
        assert!(output
            .iter()
            .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
    }

    #[test]
    fn limiter_is_chunk_size_invariant() {
        let sequence = (0..2048)
            .map(|index| {
                let left = ((index * 37 % 211) as f32 / 105.0) - 1.0;
                let right = ((index * 61 % 197) as f32 / 98.0) - 1.0;
                StereoFrame::new(left, right)
            })
            .collect::<Vec<_>>();
        let run = |chunks: &[usize]| {
            let (mut bus, _) = processor(44_100, 512);
            let mut output = Vec::new();
            let mut offset = 0;
            for &count in chunks {
                let mut block = sequence[offset..offset + count].to_vec();
                bus.process_final(&mut block);
                output.extend(block);
                offset += count;
            }
            output
        };
        assert_eq!(run(&[512, 512, 512, 512]), run(&[64; 32]));
    }

    #[test]
    fn release_recovers_after_roughly_one_tenth_second() {
        let mut limiter = FinalLimiter::new(48_000).unwrap();
        let mut last_reduction = 0.0;
        let mut at_one_time_constant = 0.0;
        for index in 0..14_400 {
            let input = if index == 0 {
                StereoFrame::new(2.0, 2.0)
            } else {
                StereoFrame::SILENCE
            };
            last_reduction = limiter.process(input).1;
            if index == 4_800 {
                at_one_time_constant = last_reduction;
            }
        }
        assert!((1.5..=2.5).contains(&at_one_time_constant));
        assert!(last_reduction < 0.5);
    }

    #[test]
    fn coherent_full_scale_sum_clips_before_limiter_but_final_meter_is_protected() {
        let (mut bus, _) = processor(48_000, 256);
        let mut sum = [StereoFrame::new(1.5, -1.5); 256];
        bus.process_final(&mut sum);
        let meter = bus.meters.snapshot();
        assert!(meter.limiter_input.clips > 0);
        assert_eq!(meter.output.clips, 0);
        assert!(meter.limiter_gain_reduction_db > 0.0);
        assert!(sum.iter().all(|frame| {
            frame.left.is_finite()
                && frame.right.is_finite()
                && frame.left.abs() <= bus.limiter.ceiling_linear() + 1e-6
                && frame.right.abs() <= bus.limiter.ceiling_linear() + 1e-6
        }));
    }

    #[test]
    fn supported_rates_and_callback_sizes_handle_steps_silence_and_alternating_channels() {
        for rate in [44_100, 48_000] {
            for callback in [64, 128, 256, 1024] {
                let (mut bus, _) = processor(rate, callback);
                let mut block = vec![StereoFrame::SILENCE; callback];
                for pass in 0..8 {
                    for (index, frame) in block.iter_mut().enumerate() {
                        *frame = if pass < 2 {
                            StereoFrame::SILENCE
                        } else if (pass * callback + index) % 2 == 0 {
                            StereoFrame::new(1.0, 0.25)
                        } else {
                            StereoFrame::new(-0.25, -1.0)
                        };
                    }
                    bus.process_final(&mut block);
                    assert!(block.iter().all(|frame| {
                        frame.left.is_finite()
                            && frame.right.is_finite()
                            && frame.left.abs() <= bus.limiter.ceiling_linear() + 1e-6
                            && frame.right.abs() <= bus.limiter.ceiling_linear() + 1e-6
                    }));
                }
            }
        }
    }

    #[test]
    fn corrupted_limiter_state_resets_to_finite_deterministic_output() {
        let mut limiter = FinalLimiter::new(48_000).unwrap();
        limiter.gain = f32::NAN;
        limiter.held_gain = f32::INFINITY;
        limiter.write = usize::MAX;
        limiter.hold_remaining = usize::MAX;
        limiter.delay[0] = StereoFrame::new(f32::NAN, f32::NEG_INFINITY);
        let (output, reduction) = limiter.process(StereoFrame::new(0.25, -0.5));
        assert_eq!(output, StereoFrame::SILENCE);
        assert!(reduction.is_finite());
        assert_eq!(limiter.write, 1);
    }
}
