//! Preallocated, allocation-free insert-effect runtime slots.

mod compressor;
mod eq;

use crate::audio_graph::{EffectId, EffectInstance, EffectKind};
use crate::dsp::{db_to_gain, AtomicMeter, MeterAccumulator, SmoothedValue, StereoFrame};
use crate::effect_schema;
use std::fmt;
use std::sync::Arc;

pub use compressor::AtomicGainReduction;
use compressor::Compressor;
use eq::Eq;

const PARAMETER_SMOOTH_SAMPLES: u32 = 64;
const BYPASS_FADE_MILLISECONDS: f32 = 5.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectError(String);

impl EffectError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for EffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EffectError {}

#[derive(Clone)]
pub struct MeterHandles {
    pub input: Arc<AtomicMeter>,
    pub output: Arc<AtomicMeter>,
    pub gain_reduction: Option<Arc<AtomicGainReduction>>,
}

enum Processor {
    Utility(Utility),
    Eq(Box<Eq>),
    Compressor(Box<Compressor>),
}

impl Processor {
    fn compile(effect: &EffectInstance, sample_rate: u32) -> Result<Self, EffectError> {
        match effect.kind {
            EffectKind::Utility => Ok(Self::Utility(Utility::compile(effect)?)),
            EffectKind::Eq => Ok(Self::Eq(Box::new(Eq::compile(effect, sample_rate)?))),
            EffectKind::Compressor => Ok(Self::Compressor(Box::new(Compressor::compile(
                effect,
                sample_rate,
            )?))),
            _ => Err(EffectError::new(format!(
                "{:?} processing is not implemented",
                effect.kind
            ))),
        }
    }

    #[inline]
    fn process(&mut self, frame: StereoFrame) -> StereoFrame {
        match self {
            Self::Utility(effect) => effect.process(frame),
            Self::Eq(effect) => effect.process(frame),
            Self::Compressor(effect) => effect.process(frame),
        }
    }

    fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        match self {
            Self::Utility(effect) => effect.set_parameter(name, value),
            Self::Eq(effect) => effect.set_parameter(name, value),
            Self::Compressor(effect) => effect.set_parameter(name, value),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Utility(effect) => effect.reset(),
            Self::Eq(effect) => effect.reset(),
            Self::Compressor(effect) => effect.reset(),
        }
    }

    fn gain_reduction(&self) -> Option<Arc<AtomicGainReduction>> {
        match self {
            Self::Compressor(effect) => Some(effect.gain_reduction()),
            Self::Utility(_) | Self::Eq(_) => None,
        }
    }

    fn publish(&self) {
        if let Self::Compressor(effect) = self {
            effect.publish();
        }
    }
}

pub struct EffectSlot {
    id: EffectId,
    kind: EffectKind,
    processor: Processor,
    processed_mix: SmoothedValue,
    bypass_fade_samples: u32,
    input_meter: MeterAccumulator,
    output_meter: MeterAccumulator,
    published_input: Arc<AtomicMeter>,
    published_output: Arc<AtomicMeter>,
}

impl EffectSlot {
    pub fn compile(
        effect: &EffectInstance,
        sample_rate: u32,
        meter_window: usize,
    ) -> Result<Self, EffectError> {
        effect_schema::validate(effect).map_err(|error| EffectError::new(error.to_string()))?;
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(EffectError::new("unsupported effect sample rate"));
        }
        let bypass_fade_samples =
            ((sample_rate as f32 * BYPASS_FADE_MILLISECONDS * 0.001).round() as u32).max(1);
        Ok(Self {
            id: effect.id,
            kind: effect.kind,
            processor: Processor::compile(effect, sample_rate)?,
            processed_mix: SmoothedValue::new(if effect.bypass { 0.0 } else { 1.0 })
                .map_err(|error| EffectError::new(error.to_string()))?,
            bypass_fade_samples,
            input_meter: MeterAccumulator::new(meter_window)
                .map_err(|error| EffectError::new(error.to_string()))?,
            output_meter: MeterAccumulator::new(meter_window)
                .map_err(|error| EffectError::new(error.to_string()))?,
            published_input: Arc::new(AtomicMeter::default()),
            published_output: Arc::new(AtomicMeter::default()),
        })
    }

    pub const fn id(&self) -> EffectId {
        self.id
    }

    pub const fn kind(&self) -> EffectKind {
        self.kind
    }

    pub fn meters(&self) -> MeterHandles {
        MeterHandles {
            input: Arc::clone(&self.published_input),
            output: Arc::clone(&self.published_output),
            gain_reduction: self.processor.gain_reduction(),
        }
    }

    pub fn set_bypass(&mut self, bypass: bool) -> Result<(), EffectError> {
        self.processed_mix
            .set_target(if bypass { 0.0 } else { 1.0 }, self.bypass_fade_samples)
            .map_err(|error| EffectError::new(error.to_string()))
    }

    pub fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        let spec = effect_schema::schema(self.kind)
            .iter()
            .find(|spec| spec.name == name)
            .ok_or_else(|| EffectError::new(format!("unknown {:?} parameter {name}", self.kind)))?;
        if !spec.accepts(value) {
            return Err(EffectError::new(format!(
                "invalid {:?} parameter {name}",
                self.kind
            )));
        }
        self.processor.set_parameter(name, value)
    }

    /// Process an in-place stereo block without allocation, locks, logging,
    /// I/O, or coefficient calculation.
    pub fn process(&mut self, frames: &mut [StereoFrame]) {
        for frame in frames.iter_mut() {
            let dry = self.input_meter.process(*frame);
            let processed = self.processor.process(dry);
            let processed = if processed.left.is_finite() && processed.right.is_finite() {
                processed
            } else {
                self.processor.reset();
                dry
            };
            let wet = self.processed_mix.next_value();
            let output = StereoFrame::new(
                dry.left + (processed.left - dry.left) * wet,
                dry.right + (processed.right - dry.right) * wet,
            )
            .finite_or_silence();
            *frame = self.output_meter.process(output);
        }
        self.published_input
            .publish(self.input_meter.snapshot_and_clear_peak());
        self.published_output
            .publish(self.output_meter.snapshot_and_clear_peak());
        self.processor.publish();
    }

    pub fn reset(&mut self) {
        self.processor.reset();
        self.input_meter.reset();
        self.output_meter.reset();
        self.published_input.publish(Default::default());
        self.published_output.publish(Default::default());
    }
}

struct Utility {
    trim: SmoothedValue,
    left_pan: SmoothedValue,
    right_pan: SmoothedValue,
    width: SmoothedValue,
    invert_left: SmoothedValue,
    invert_right: SmoothedValue,
    mute: SmoothedValue,
}

impl Utility {
    fn compile(effect: &EffectInstance) -> Result<Self, EffectError> {
        let value = |name| {
            effect_schema::parameter(effect, name)
                .map_err(|error| EffectError::new(error.to_string()))
        };
        let (left_pan, right_pan) = stereo_pan_gains(value("pan")?);
        Ok(Self {
            trim: smooth(db_to_gain(value("trim_db")?)?),
            left_pan: smooth(left_pan),
            right_pan: smooth(right_pan),
            width: smooth(value("width_percent")? * 0.01),
            invert_left: smooth(polarity(value("invert_left")?)),
            invert_right: smooth(polarity(value("invert_right")?)),
            mute: smooth(1.0 - value("mute")?),
        })
    }

    #[inline]
    fn process(&mut self, frame: StereoFrame) -> StereoFrame {
        let trim = self.trim.next_value() * self.mute.next_value();
        let left = frame.left * trim * self.left_pan.next_value() * self.invert_left.next_value();
        let right =
            frame.right * trim * self.right_pan.next_value() * self.invert_right.next_value();
        let mid = (left + right) * 0.5;
        let side = (left - right) * 0.5 * self.width.next_value();
        StereoFrame::new(mid + side, mid - side).finite_or_silence()
    }

    fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        if name == "pan" {
            let (left, right) = stereo_pan_gains(value);
            set_smooth(&mut self.left_pan, left)?;
            return set_smooth(&mut self.right_pan, right);
        }
        let (target, target_value) = match name {
            "trim_db" => (&mut self.trim, db_to_gain(value)?),
            "width_percent" => (&mut self.width, value * 0.01),
            "invert_left" => (&mut self.invert_left, polarity(value)),
            "invert_right" => (&mut self.invert_right, polarity(value)),
            "mute" => (&mut self.mute, 1.0 - value),
            _ => {
                return Err(EffectError::new(format!(
                    "unknown Utility parameter {name}"
                )))
            }
        };
        set_smooth(target, target_value)
    }

    fn reset(&mut self) {
        // Utility has no recursive state. Smoothers intentionally retain their
        // current values so reset never jumps a live gain or polarity target.
    }
}

fn smooth(value: f32) -> SmoothedValue {
    SmoothedValue::new(value).expect("validated finite effect parameter")
}

fn set_smooth(value: &mut SmoothedValue, target: f32) -> Result<(), EffectError> {
    value
        .set_target(target, PARAMETER_SMOOTH_SAMPLES)
        .map_err(|error| EffectError::new(error.to_string()))
}

fn polarity(value: f32) -> f32 {
    if value == 0.0 {
        1.0
    } else {
        -1.0
    }
}

fn stereo_pan_gains(pan: f32) -> (f32, f32) {
    if pan < 0.0 {
        (1.0, (-pan * std::f32::consts::FRAC_PI_2).cos())
    } else {
        ((pan * std::f32::consts::FRAC_PI_2).cos(), 1.0)
    }
}

impl From<crate::dsp::DspError> for EffectError {
    fn from(error: crate::dsp::DspError) -> Self {
        Self::new(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_graph::EFFECT_FORMAT_VERSION;
    use crate::dsp::allocation_test::assert_no_allocations;
    use std::collections::BTreeMap;

    fn utility(parameters: BTreeMap<String, f32>, bypass: bool) -> EffectInstance {
        EffectInstance {
            id: 7,
            kind: EffectKind::Utility,
            version: EFFECT_FORMAT_VERSION,
            bypass,
            parameters,
            owned_memory_bytes: 0,
        }
    }

    #[test]
    fn slot_preserves_identity_and_rejects_unimplemented_kinds() {
        let slot = EffectSlot::compile(&utility(BTreeMap::new(), false), 48_000, 128).unwrap();
        assert_eq!(slot.id(), 7);
        assert_eq!(slot.kind(), EffectKind::Utility);
        let mut effect = utility(BTreeMap::new(), false);
        effect.kind = EffectKind::Distortion;
        assert!(EffectSlot::compile(&effect, 48_000, 128).is_err());
    }

    #[test]
    fn utility_processes_stereo_parameters_without_allocating() {
        let effect = utility(
            BTreeMap::from([
                ("trim_db".into(), -6.0206),
                ("pan".into(), 1.0),
                ("width_percent".into(), 100.0),
            ]),
            false,
        );
        let mut slot = EffectSlot::compile(&effect, 48_000, 128).unwrap();
        let meters = slot.meters();
        let mut block = [StereoFrame::new(0.5, 0.5); 128];
        assert_no_allocations(|| slot.process(&mut block));
        assert!(block
            .iter()
            .all(|frame| frame.left.abs() < 0.001 && (frame.right - 0.25).abs() < 0.001));
        assert!(meters.input.load().peak.left >= 0.5);
        assert!(meters.output.load().peak.right >= 0.249);
    }

    #[test]
    fn bypass_crossfade_is_bounded_and_reaches_exact_dry() {
        let effect = utility(BTreeMap::from([("trim_db".into(), -12.0)]), false);
        let mut slot = EffectSlot::compile(&effect, 48_000, 256).unwrap();
        slot.set_bypass(true).unwrap();
        let mut block = [StereoFrame::new(0.5, -0.5); 256];
        slot.process(&mut block);
        assert!(block.iter().all(|frame| {
            frame.left.is_finite()
                && frame.right.is_finite()
                && (0.125..=0.5).contains(&frame.left)
                && (-0.5..=-0.125).contains(&frame.right)
        }));
        assert_eq!(block[255], StereoFrame::new(0.5, -0.5));
    }

    #[test]
    fn poison_is_metered_and_recovers_to_finite_output() {
        let mut slot = EffectSlot::compile(&utility(BTreeMap::new(), false), 48_000, 4).unwrap();
        let meters = slot.meters();
        let mut block = [
            StereoFrame::new(f32::NAN, 0.25),
            StereoFrame::new(0.5, f32::INFINITY),
            StereoFrame::new(0.25, -0.25),
            StereoFrame::SILENCE,
        ];
        slot.process(&mut block);
        assert!(block
            .iter()
            .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
        assert_eq!(meters.input.load().non_finite, 2);
        assert_eq!(meters.output.load().non_finite, 0);
        slot.reset();
        assert_eq!(meters.input.load(), Default::default());
        assert_eq!(meters.output.load(), Default::default());
    }

    #[test]
    fn rapid_valid_moves_are_smoothed_and_invalid_moves_are_refused() {
        let mut slot = EffectSlot::compile(&utility(BTreeMap::new(), false), 48_000, 64).unwrap();
        for index in 0..100 {
            slot.set_parameter("trim_db", if index % 2 == 0 { -60.0 } else { 12.0 })
                .unwrap();
            slot.set_parameter("invert_left", (index % 2) as f32)
                .unwrap();
            let mut block = [StereoFrame::new(1.0, -1.0); 17];
            slot.process(&mut block);
            assert!(block
                .iter()
                .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
        }
        assert!(slot.set_parameter("trim_db", f32::NAN).is_err());
        assert!(slot.set_parameter("future", 0.0).is_err());
    }
}
