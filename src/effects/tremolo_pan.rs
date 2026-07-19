use super::{smooth, EffectError, PARAMETER_SMOOTH_SAMPLES};
use crate::audio_graph::EffectInstance;
use crate::dsp::{db_to_gain, finite_or_zero, SineLfo, SmoothedValue, StereoFrame};
use crate::effect_schema;

const PAN_TABLE_STEPS: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Tremolo,
    AutoPan,
}

impl Mode {
    fn from_parameter(value: f32) -> Self {
        if value == 0.0 {
            Self::Tremolo
        } else {
            Self::AutoPan
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Shape {
    Sine,
    Triangle,
    SmoothedSquare,
}

impl Shape {
    fn from_parameter(value: f32) -> Self {
        match value as u8 {
            0 => Self::Sine,
            1 => Self::Triangle,
            _ => Self::SmoothedSquare,
        }
    }
}

struct ShapeLfo {
    sine: SineLfo,
    phase: f32,
    phase_step: f32,
    square: f32,
    square_coefficient: f32,
    sample_rate: f32,
}

impl ShapeLfo {
    fn new(rate_hz: f32, phase_degrees: f32, sample_rate: f32) -> Result<Self, EffectError> {
        let mut lfo = Self {
            sine: SineLfo::new(rate_hz, phase_degrees.to_radians(), sample_rate)?,
            phase: (phase_degrees / 360.0).rem_euclid(1.0),
            phase_step: rate_hz / sample_rate,
            square: 0.0,
            square_coefficient: 1.0 - (-1.0 / (0.005 * sample_rate)).exp(),
            sample_rate,
        };
        lfo.square = if lfo.sine.next_value() >= 0.0 {
            1.0
        } else {
            -1.0
        };
        Ok(lfo)
    }

    fn configure(&mut self, rate_hz: f32, phase_degrees: f32) -> Result<(), EffectError> {
        self.sine
            .configure(rate_hz, phase_degrees.to_radians(), self.sample_rate)?;
        self.phase = (phase_degrees / 360.0).rem_euclid(1.0);
        self.phase_step = rate_hz / self.sample_rate;
        Ok(())
    }

    #[inline]
    fn next(&mut self, shape: Shape) -> f32 {
        let sine = self.sine.next_value();
        let triangle = 1.0 - 4.0 * (self.phase - 0.5).abs();
        let target = if sine >= 0.0 { 1.0 } else { -1.0 };
        self.square =
            finite_or_zero(self.square + self.square_coefficient * (target - self.square));
        self.phase += self.phase_step;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        match shape {
            Shape::Sine => sine,
            Shape::Triangle => triangle.clamp(-1.0, 1.0),
            Shape::SmoothedSquare => self.square.clamp(-1.0, 1.0),
        }
    }
}

pub(super) struct TremoloPan {
    sample_rate: f32,
    left_lfo: ShapeLfo,
    right_lfo: ShapeLfo,
    rate_hz: f32,
    stereo_phase_degrees: f32,
    mode: Mode,
    shape: Shape,
    depth: SmoothedValue,
    output: SmoothedValue,
    pan_table: Box<[StereoFrame]>,
}

impl TremoloPan {
    pub(super) fn compile(effect: &EffectInstance, sample_rate: u32) -> Result<Self, EffectError> {
        let value = |name| {
            effect_schema::parameter(effect, name)
                .map_err(|error| EffectError::new(error.to_string()))
        };
        let sample_rate = sample_rate as f32;
        let rate_hz = value("rate_hz")?;
        let stereo_phase_degrees = value("stereo_phase_degrees")?;
        Ok(Self {
            sample_rate,
            left_lfo: ShapeLfo::new(rate_hz, 0.0, sample_rate)?,
            right_lfo: ShapeLfo::new(rate_hz, stereo_phase_degrees, sample_rate)?,
            rate_hz,
            stereo_phase_degrees,
            mode: Mode::from_parameter(value("mode")?),
            shape: Shape::from_parameter(value("shape")?),
            depth: smooth(value("depth_percent")? * 0.01),
            output: smooth(db_to_gain(value("output_trim_db")?)?),
            pan_table: pan_table(),
        })
    }

    #[inline]
    pub(super) fn process(&mut self, frame: StereoFrame) -> StereoFrame {
        let left_lfo = self.left_lfo.next(self.shape);
        let right_lfo = self.right_lfo.next(self.shape);
        let depth = self.depth.next_value();
        let output = self.output.next_value();
        match self.mode {
            Mode::Tremolo => {
                let left_gain = 1.0 - depth * (left_lfo * 0.5 + 0.5);
                let right_gain = 1.0 - depth * (right_lfo * 0.5 + 0.5);
                StereoFrame::new(
                    frame.left * left_gain * output,
                    frame.right * right_gain * output,
                )
            }
            Mode::AutoPan => {
                let movement = left_lfo * depth;
                let gains = table_value(&self.pan_table, movement);
                StereoFrame::new(
                    frame.left * gains.left * output,
                    frame.right * gains.right * output,
                )
            }
        }
        .finite_or_silence()
    }

    pub(super) fn set_parameter(&mut self, name: &str, value: f32) -> Result<(), EffectError> {
        match name {
            "mode" => self.mode = Mode::from_parameter(value),
            "rate_hz" => {
                self.rate_hz = value;
                self.configure_lfos()?;
            }
            "depth_percent" => self
                .depth
                .set_target(value * 0.01, PARAMETER_SMOOTH_SAMPLES)?,
            "shape" => self.shape = Shape::from_parameter(value),
            "stereo_phase_degrees" => {
                self.stereo_phase_degrees = value;
                self.configure_lfos()?;
            }
            "output_trim_db" => self
                .output
                .set_target(db_to_gain(value)?, PARAMETER_SMOOTH_SAMPLES)?,
            _ => {
                return Err(EffectError::new(format!(
                    "unknown TremoloPan parameter {name}"
                )))
            }
        }
        Ok(())
    }

    fn configure_lfos(&mut self) -> Result<(), EffectError> {
        self.left_lfo.configure(self.rate_hz, 0.0)?;
        self.right_lfo
            .configure(self.rate_hz, self.stereo_phase_degrees)?;
        Ok(())
    }

    pub(super) fn reset(&mut self) {
        if let Ok(left) = ShapeLfo::new(self.rate_hz, 0.0, self.sample_rate) {
            self.left_lfo = left;
        }
        if let Ok(right) = ShapeLfo::new(self.rate_hz, self.stereo_phase_degrees, self.sample_rate)
        {
            self.right_lfo = right;
        }
    }
}

fn pan_table() -> Box<[StereoFrame]> {
    let scale = std::f32::consts::SQRT_2;
    (0..=PAN_TABLE_STEPS)
        .map(|index| {
            let pan = index as f32 / PAN_TABLE_STEPS as f32 * 2.0 - 1.0;
            let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
            StereoFrame::new(angle.cos() * scale, angle.sin() * scale)
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

#[inline]
fn table_value(table: &[StereoFrame], movement: f32) -> StereoFrame {
    let position = (movement * 0.5 + 0.5).clamp(0.0, 1.0) * PAN_TABLE_STEPS as f32;
    let first = position.floor() as usize;
    let second = (first + 1).min(PAN_TABLE_STEPS);
    let fraction = position - first as f32;
    StereoFrame::new(
        table[first].left + (table[second].left - table[first].left) * fraction,
        table[first].right + (table[second].right - table[first].right) * fraction,
    )
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
            id: 33,
            kind: EffectKind::TremoloPan,
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
    fn depth_zero_is_unity_and_tremolo_shapes_are_bounded_and_distinct() {
        for mode in [0.0, 1.0] {
            let mut slot = EffectSlot::compile(
                &effect([("mode", mode), ("depth_percent", 0.0)]),
                48_000,
                128,
            )
            .unwrap();
            let mut block = [StereoFrame::new(0.5, -0.25); 128];
            slot.process(&mut block);
            assert!(block.iter().all(|frame| {
                (frame.left - 0.5).abs() < 1.0e-5 && (frame.right + 0.25).abs() < 1.0e-5
            }));
        }

        let mut outputs = Vec::new();
        for shape in 0..=2 {
            let mut slot = EffectSlot::compile(
                &effect([
                    ("shape", shape as f32),
                    ("depth_percent", 100.0),
                    ("stereo_phase_degrees", 0.0),
                ]),
                8_000,
                64,
            )
            .unwrap();
            let mut block = vec![StereoFrame::new(1.0, 1.0); 16_000];
            for chunk in block.chunks_mut(47) {
                slot.process(chunk);
            }
            assert!(block.iter().all(|frame| {
                frame.left.is_finite() && (0.0..=1.000_001).contains(&frame.left)
            }));
            outputs.push(block[4_000].left);
        }
        assert!(outputs
            .windows(2)
            .any(|pair| (pair[0] - pair[1]).abs() > 0.01));
    }

    #[test]
    fn autopan_table_has_constant_pair_power_and_no_runtime_trigonometry() {
        let table = pan_table();
        for gains in table.iter() {
            assert!((gains.left * gains.left + gains.right * gains.right - 2.0).abs() < 1.0e-5);
        }
        let mut slot = EffectSlot::compile(
            &effect([("mode", 1.0), ("depth_percent", 100.0)]),
            48_000,
            128,
        )
        .unwrap();
        let mut block = [StereoFrame::new(0.25, 0.25); 1_024];
        assert_no_allocations(|| slot.process(&mut block));
        assert!(block
            .iter()
            .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
    }

    #[test]
    fn limits_moves_chunks_reset_and_bypass_are_safe() {
        for sample_rate in [8_000, 44_100, 48_000, 96_000, 384_000] {
            let mut slot = EffectSlot::compile(
                &effect([
                    ("mode", 1.0),
                    ("rate_hz", 15.0),
                    ("depth_percent", 100.0),
                    ("shape", 2.0),
                ]),
                sample_rate,
                128,
            )
            .unwrap();
            for index in 0..50 {
                slot.set_parameter("shape", (index % 3) as f32).unwrap();
                slot.set_parameter("rate_hz", 0.05 + index as f32 * 0.1)
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
