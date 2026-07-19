//! Bounded, UI-side performance meter sampling and presentation state.

use crate::dsp::MeterSnapshot;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

pub const VISIBLE_CPU_CORES: usize = 4;
const MAX_CPU_CORES: usize = 64;
pub const CPU_POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const AUDIO_FLOOR_DBFS: f32 = -60.0;
const AUDIO_HOLD: Duration = Duration::from_millis(900);
const CLIP_HOLD: Duration = Duration::from_secs(2);
const PEAK_DECAY_DB_PER_SECOND: f32 = 24.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeterColor {
    Green,
    Yellow,
    Red,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarCell {
    pub symbol: char,
    pub color: MeterColor,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CpuCounter {
    index: usize,
    total: u64,
    idle: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CpuSnapshot {
    cores: Vec<CpuCounter>,
}

impl CpuSnapshot {
    pub fn core_count(&self) -> usize {
        self.cores.len()
    }
}

/// Parses only numbered per-core lines and caps hostile or surprising input.
pub fn parse_proc_stat(input: &str) -> CpuSnapshot {
    let mut cores = Vec::with_capacity(VISIBLE_CPU_CORES);
    for line in input.lines() {
        if cores.len() == MAX_CPU_CORES {
            break;
        }
        let mut fields = line.split_whitespace();
        let Some(label) = fields.next() else {
            continue;
        };
        let Some(index) = label.strip_prefix("cpu").and_then(|value| {
            (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| value.parse::<usize>().ok())
                .flatten()
        }) else {
            continue;
        };
        let values = fields
            // Linux reports guest and guest_nice after these eight counters,
            // but they are already included in user and nice.
            .take(8)
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>();
        let Ok(values) = values else {
            continue;
        };
        if values.len() < 4 {
            continue;
        }
        let total = values.iter().copied().fold(0_u64, u64::saturating_add);
        let idle = values[3].saturating_add(values.get(4).copied().unwrap_or(0));
        cores.push(CpuCounter { index, total, idle });
    }
    cores.sort_unstable_by_key(|core| core.index);
    cores.dedup_by_key(|core| core.index);
    CpuSnapshot { cores }
}

pub fn cpu_percentages(
    previous: &CpuSnapshot,
    current: &CpuSnapshot,
) -> [Option<f32>; VISIBLE_CPU_CORES] {
    std::array::from_fn(|index| {
        let old = previous.cores.iter().find(|core| core.index == index)?;
        let new = current.cores.iter().find(|core| core.index == index)?;
        if new.total < old.total || new.idle < old.idle {
            return None;
        }
        let total = new.total - old.total;
        let idle = new.idle - old.idle;
        if total == 0 || idle > total {
            return None;
        }
        Some(100.0 * (total - idle) as f32 / total as f32)
    })
}

pub const fn cpu_color(percent: f32) -> MeterColor {
    if percent < 60.0 {
        MeterColor::Green
    } else if percent <= 85.0 {
        MeterColor::Yellow
    } else {
        MeterColor::Red
    }
}

pub const fn audio_color(dbfs: f32) -> MeterColor {
    if dbfs < -12.0 {
        MeterColor::Green
    } else if dbfs <= -3.0 {
        MeterColor::Yellow
    } else {
        MeterColor::Red
    }
}

pub fn level_dbfs(amplitude: f32) -> f32 {
    if amplitude.is_finite() && amplitude > 0.0 {
        (20.0 * amplitude.log10()).max(AUDIO_FLOOR_DBFS)
    } else {
        AUDIO_FLOOR_DBFS
    }
}

pub fn cpu_bar(width: usize, percent: f32) -> Vec<BarCell> {
    let filled = ((percent.clamp(0.0, 100.0) / 100.0) * width as f32).round() as usize;
    (0..width)
        .map(|index| {
            let scale_value = 100.0 * (index + 1) as f32 / width.max(1) as f32;
            BarCell {
                symbol: if index < filled { '█' } else { '·' },
                color: cpu_color(scale_value),
            }
        })
        .collect()
}

pub fn audio_bar(width: usize, rms_dbfs: f32, peak_dbfs: f32) -> Vec<BarCell> {
    let position = |value: f32| {
        ((value.clamp(AUDIO_FLOOR_DBFS, 0.0) - AUDIO_FLOOR_DBFS) / -AUDIO_FLOOR_DBFS * width as f32)
            .round() as usize
    };
    let filled = position(rms_dbfs).min(width);
    let peak = (peak_dbfs > AUDIO_FLOOR_DBFS).then(|| {
        position(peak_dbfs)
            .saturating_sub(1)
            .min(width.saturating_sub(1))
    });
    (0..width)
        .map(|index| {
            let scale_value =
                AUDIO_FLOOR_DBFS + -AUDIO_FLOOR_DBFS * (index + 1) as f32 / width.max(1) as f32;
            BarCell {
                symbol: if Some(index) == peak {
                    '│'
                } else if index < filled {
                    '█'
                } else {
                    '·'
                },
                color: audio_color(scale_value),
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioAvailability {
    Stopped,
    DirectUnavailable,
    GraphActive,
    Presentation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioLevel {
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
}

impl Default for AudioLevel {
    fn default() -> Self {
        Self {
            rms_dbfs: AUDIO_FLOOR_DBFS,
            peak_dbfs: AUDIO_FLOOR_DBFS,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct LevelState {
    display: AudioLevel,
    hold_until: Option<Instant>,
    initialized: bool,
}

#[derive(Debug)]
pub struct PerformanceMeter {
    cpu_previous: Option<CpuSnapshot>,
    cpu_last_read: Option<Instant>,
    cpu_loads: [Option<f32>; VISIBLE_CPU_CORES],
    cpu_cores: usize,
    cpu_available: bool,
    audio: [LevelState; 2],
    numeric_peak_dbfs: [f32; 2],
    audio_availability: AudioAvailability,
    audio_last_update: Option<Instant>,
    clip_count: Option<u64>,
    clip_until: Option<Instant>,
    non_finite: u64,
    presentation: bool,
}

impl Default for PerformanceMeter {
    fn default() -> Self {
        Self {
            cpu_previous: None,
            cpu_last_read: None,
            cpu_loads: [None; VISIBLE_CPU_CORES],
            cpu_cores: 0,
            cpu_available: false,
            audio: [LevelState::default(); 2],
            numeric_peak_dbfs: [AUDIO_FLOOR_DBFS; 2],
            audio_availability: AudioAvailability::Stopped,
            audio_last_update: None,
            clip_count: None,
            clip_until: None,
            non_finite: 0,
            presentation: false,
        }
    }
}

impl PerformanceMeter {
    pub fn poll_cpu(&mut self, now: Instant, path: &Path) {
        if self
            .cpu_last_read
            .is_some_and(|last| now.duration_since(last) < CPU_POLL_INTERVAL)
        {
            return;
        }
        self.cpu_last_read = Some(now);
        self.presentation = false;
        let Ok(text) = fs::read_to_string(path) else {
            self.cpu_available = false;
            self.cpu_loads = [None; VISIBLE_CPU_CORES];
            return;
        };
        self.update_cpu_text(&text);
    }

    pub fn update_cpu_text(&mut self, text: &str) {
        let current = parse_proc_stat(text);
        self.cpu_cores = current.core_count();
        self.cpu_available = self.cpu_cores > 0;
        self.cpu_loads = self
            .cpu_previous
            .as_ref()
            .map(|previous| cpu_percentages(previous, &current))
            .unwrap_or([None; VISIBLE_CPU_CORES]);
        self.cpu_previous = Some(current);
    }

    pub fn cpu_loads(&self) -> [Option<f32>; VISIBLE_CPU_CORES] {
        self.cpu_loads
    }

    pub fn cpu_cores(&self) -> usize {
        self.cpu_cores
    }

    pub fn cpu_available(&self) -> bool {
        self.cpu_available
    }

    pub fn set_audio_unavailable(&mut self, availability: AudioAvailability) {
        debug_assert!(!matches!(
            availability,
            AudioAvailability::GraphActive | AudioAvailability::Presentation
        ));
        self.presentation = false;
        self.audio_availability = availability;
        self.audio_last_update = None;
        self.clip_count = None;
        self.clip_until = None;
        self.non_finite = 0;
        self.audio = [LevelState::default(); 2];
        self.numeric_peak_dbfs = [AUDIO_FLOOR_DBFS; 2];
    }

    pub fn update_audio(&mut self, snapshot: MeterSnapshot, now: Instant) {
        self.audio_availability = AudioAvailability::GraphActive;
        self.presentation = false;
        let elapsed = self
            .audio_last_update
            .map(|last| now.saturating_duration_since(last))
            .unwrap_or_default();
        self.audio_last_update = Some(now);
        let inputs = [
            (snapshot.rms.left, snapshot.peak.left),
            (snapshot.rms.right, snapshot.peak.right),
        ];
        for ((state, numeric_peak), (rms, peak)) in self
            .audio
            .iter_mut()
            .zip(&mut self.numeric_peak_dbfs)
            .zip(inputs)
        {
            let target_rms = level_dbfs(rms);
            let target_peak = level_dbfs(peak);
            *numeric_peak = (*numeric_peak).max(target_peak);
            if !state.initialized {
                state.display.rms_dbfs = target_rms;
                state.display.peak_dbfs = target_peak;
                state.hold_until = Some(now + AUDIO_HOLD);
                state.initialized = true;
            } else {
                let tau = if target_rms > state.display.rms_dbfs {
                    0.08
                } else {
                    0.35
                };
                let alpha = 1.0 - (-elapsed.as_secs_f32() / tau).exp();
                state.display.rms_dbfs += alpha * (target_rms - state.display.rms_dbfs);
                if target_peak >= state.display.peak_dbfs {
                    state.display.peak_dbfs = target_peak;
                    state.hold_until = Some(now + AUDIO_HOLD);
                } else if state.hold_until.is_none_or(|until| now >= until) {
                    state.display.peak_dbfs = (state.display.peak_dbfs
                        - PEAK_DECAY_DB_PER_SECOND * elapsed.as_secs_f32())
                    .max(target_peak);
                }
            }
        }
        let new_clip = snapshot.peak.left >= 1.0
            || snapshot.peak.right >= 1.0
            || self.clip_count.is_some_and(|old| {
                snapshot.clips > old || (snapshot.clips < old && snapshot.clips > 0)
            })
            || (self.clip_count.is_none() && snapshot.clips > 0);
        if new_clip {
            self.clip_until = Some(now + CLIP_HOLD);
        }
        self.clip_count = Some(snapshot.clips);
        self.non_finite = snapshot.non_finite;
    }

    pub fn audio_availability(&self) -> AudioAvailability {
        self.audio_availability
    }

    pub fn audio_levels(&self) -> [AudioLevel; 2] {
        [self.audio[0].display, self.audio[1].display]
    }

    pub fn numeric_peak_dbfs(&self) -> [f32; 2] {
        self.numeric_peak_dbfs
    }

    pub fn clipping(&self, now: Instant) -> bool {
        self.clip_until.is_some_and(|until| now < until)
    }

    pub fn non_finite(&self) -> u64 {
        self.non_finite
    }

    pub fn clear_holds(&mut self) {
        for state in &mut self.audio {
            state.display.peak_dbfs = state.display.rms_dbfs;
            state.hold_until = None;
        }
        self.clear_numeric_peaks();
        self.clip_until = None;
    }

    pub fn clear_numeric_peaks(&mut self) {
        self.numeric_peak_dbfs = [AUDIO_FLOOR_DBFS; 2];
    }

    pub fn seed_presentation(
        &mut self,
        cpu: [Option<f32>; VISIBLE_CPU_CORES],
        audio: [AudioLevel; 2],
        numeric_peak_dbfs: [f32; 2],
        now: Instant,
    ) {
        self.cpu_loads = cpu;
        self.cpu_cores = VISIBLE_CPU_CORES;
        self.cpu_available = true;
        for (state, level) in self.audio.iter_mut().zip(audio) {
            state.display = level;
            state.initialized = true;
            state.hold_until = Some(now + AUDIO_HOLD);
        }
        self.numeric_peak_dbfs = numeric_peak_dbfs;
        self.audio_availability = AudioAvailability::Presentation;
        self.audio_last_update = Some(now);
        self.presentation = true;
    }

    pub fn is_presentation(&self) -> bool {
        self.presentation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::StereoFrame;

    const FIRST: &str = "cpu  100 0 100 800 0 0 0 0 0 0\n\
cpu0 25 0 25 200 0 0 0 0 0 0\n\
cpu1 20 0 30 200 0 0 0 0 0 0\n\
cpu2 30 0 20 200 0 0 0 0 0 0\n\
cpu3 25 0 25 200 0 0 0 0 0 0\n\
intr 123\n";
    const SECOND: &str = "cpu  180 0 180 1040 0 0 0 0 0 0\n\
cpu0 45 0 45 210 0 0 0 0 0 0\n\
cpu1 30 0 40 230 0 0 0 0 0 0\n\
cpu2 70 0 40 210 0 0 0 0 0 0\n\
cpu3 35 0 55 390 0 0 0 0 0 0\n";

    #[test]
    fn proc_stat_parser_ignores_aggregate_and_calculates_per_core_deltas() {
        let first = parse_proc_stat(FIRST);
        let second = parse_proc_stat(SECOND);
        assert_eq!(first.core_count(), 4);
        let loads = cpu_percentages(&first, &second);
        for (actual, expected) in loads.into_iter().zip([80.0, 40.0, 85.714_29, 17.391_304]) {
            assert!((actual.unwrap() - expected).abs() < 0.001);
        }
    }

    #[test]
    fn proc_stat_handles_missing_malformed_and_reset_counters() {
        let first = parse_proc_stat(FIRST);
        let reset = parse_proc_stat("cpu0 1 0 1 2\ncpu1 broken 0 0 0\ncpu8 10 0 10 80\n");
        assert_eq!(reset.core_count(), 2);
        assert_eq!(cpu_percentages(&first, &reset), [None, None, None, None]);
    }

    #[test]
    fn threshold_colors_use_documented_boundaries() {
        assert_eq!(cpu_color(59.999), MeterColor::Green);
        assert_eq!(cpu_color(60.0), MeterColor::Yellow);
        assert_eq!(cpu_color(84.999), MeterColor::Yellow);
        assert_eq!(cpu_color(85.0), MeterColor::Yellow);
        assert_eq!(cpu_color(85.001), MeterColor::Red);
        assert_eq!(audio_color(-12.001), MeterColor::Green);
        assert_eq!(audio_color(-12.0), MeterColor::Yellow);
        assert_eq!(audio_color(-3.001), MeterColor::Yellow);
        assert_eq!(audio_color(-3.0), MeterColor::Yellow);
        assert_eq!(audio_color(-2.999), MeterColor::Red);
    }

    #[test]
    fn db_conversion_and_bars_handle_silence_and_clipping() {
        assert_eq!(level_dbfs(0.0), AUDIO_FLOOR_DBFS);
        assert!((level_dbfs(0.5) + 6.0206).abs() < 0.001);
        assert!((level_dbfs(2.0) - 6.0206).abs() < 0.001);
        assert!(audio_bar(20, AUDIO_FLOOR_DBFS, AUDIO_FLOOR_DBFS)
            .iter()
            .all(|cell| cell.symbol == '·'));
        assert_eq!(audio_bar(20, -6.0, 0.0).last().unwrap().symbol, '│');

        let now = Instant::now();
        let mut meter = PerformanceMeter::default();
        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(1.0, 0.5),
                rms: StereoFrame::new(0.5, 0.25),
                clips: 1,
                non_finite: 0,
            },
            now,
        );
        assert!(meter.clipping(now));
        assert!(!meter.clipping(now + CLIP_HOLD));
    }

    #[test]
    fn rms_release_is_smoothed_and_peak_is_held_then_decays() {
        let now = Instant::now();
        let mut meter = PerformanceMeter::default();
        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(0.5, 0.5),
                rms: StereoFrame::new(0.5, 0.5),
                ..MeterSnapshot::default()
            },
            now,
        );
        meter.update_audio(MeterSnapshot::default(), now + Duration::from_millis(100));
        let held = meter.audio_levels()[0];
        assert!(held.rms_dbfs > AUDIO_FLOOR_DBFS);
        assert!((held.peak_dbfs + 6.0206).abs() < 0.01);
        meter.update_audio(MeterSnapshot::default(), now + Duration::from_secs(1));
        assert!(meter.audio_levels()[0].peak_dbfs < held.peak_dbfs);
    }

    #[test]
    fn numeric_peak_maximum_rises_and_never_decays() {
        let now = Instant::now();
        let mut meter = PerformanceMeter::default();
        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(0.25, 0.25),
                ..MeterSnapshot::default()
            },
            now,
        );
        let first = meter.numeric_peak_dbfs();
        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(0.75, 0.5),
                ..MeterSnapshot::default()
            },
            now + Duration::from_millis(10),
        );
        let louder = meter.numeric_peak_dbfs();
        assert!(louder[0] > first[0]);
        assert!(louder[1] > first[1]);

        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(0.01, 0.02),
                ..MeterSnapshot::default()
            },
            now + Duration::from_secs(60),
        );
        assert_eq!(meter.numeric_peak_dbfs(), louder);
    }

    #[test]
    fn numeric_peak_maxima_are_independent_and_manual_reset_clears_both() {
        let now = Instant::now();
        let mut meter = PerformanceMeter::default();
        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(0.8, 0.2),
                ..MeterSnapshot::default()
            },
            now,
        );
        let first = meter.numeric_peak_dbfs();
        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(0.4, 0.9),
                ..MeterSnapshot::default()
            },
            now + Duration::from_millis(20),
        );
        let second = meter.numeric_peak_dbfs();
        assert_eq!(second[0], first[0]);
        assert!(second[1] > first[1]);

        meter.clear_holds();
        assert_eq!(
            meter.numeric_peak_dbfs(),
            [AUDIO_FLOOR_DBFS, AUDIO_FLOOR_DBFS]
        );
    }

    #[test]
    fn unavailable_and_new_meter_lifecycle_cannot_leak_an_old_maximum() {
        let now = Instant::now();
        let mut meter = PerformanceMeter::default();
        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(0.95, 0.85),
                ..MeterSnapshot::default()
            },
            now,
        );
        assert!(meter.numeric_peak_dbfs()[0] > -1.0);

        meter.set_audio_unavailable(AudioAvailability::Stopped);
        assert_eq!(
            meter.numeric_peak_dbfs(),
            [AUDIO_FLOOR_DBFS, AUDIO_FLOOR_DBFS]
        );
        meter.update_audio(
            MeterSnapshot {
                peak: StereoFrame::new(0.1, 0.2),
                ..MeterSnapshot::default()
            },
            now + Duration::from_secs(1),
        );
        let fresh = meter.numeric_peak_dbfs();
        assert!((fresh[0] - level_dbfs(0.1)).abs() < 0.001);
        assert!((fresh[1] - level_dbfs(0.2)).abs() < 0.001);

        meter.set_audio_unavailable(AudioAvailability::DirectUnavailable);
        assert_eq!(
            meter.numeric_peak_dbfs(),
            [AUDIO_FLOOR_DBFS, AUDIO_FLOOR_DBFS]
        );
    }
}
