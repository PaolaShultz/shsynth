//! JACK-synchronized FT2 WAV loops. Decode/import work happens before JACK
//! activation; the process callback only reads immutable PCM and writes two
//! bounded output buffers.

use crate::config::{ControllerClockConfig, LoopPlayerConfig};
use crate::dsp::{AtomicMeter, MeterAccumulator, MeterSnapshot, StereoFrame, MAX_METER_WINDOW};
use crate::jack::{Client as JackClient, Port as JackPort, PortDirection, PortGetBuffer};
use crate::sequencer::{LoopSettings, Song};
use alsa::seq::{Addr, EvQueueControl, Event, EventType, PortCap, PortType, Seq};
use alsa::Direction;
use anyhow::{bail, Context, Result};
use libc::{c_int, c_uint, c_void};
use midir::MidiOutput;
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const BEAT_UNITS: f64 = 1_000_000.0;
const ANALYSIS_HOP: usize = 1024;
const LOOP_OUTPUT_PORT_NAMES: [&str; 2] = ["output_l", "output_r"];

pub(crate) fn configured_output_ports(config: &LoopPlayerConfig) -> [String; 2] {
    [
        format!("{}:{}", config.client_name, LOOP_OUTPUT_PORT_NAMES[0]),
        format!("{}:{}", config.client_name, LOOP_OUTPUT_PORT_NAMES[1]),
    ]
}
const MAX_LOOP_CALLBACK_FRAMES: usize = MAX_METER_WINDOW;
// Decoding is deliberately bounded because the whole loop stays resident in
// memory for the lock-free JACK callback. Six million stereo frames use about
// 46 MiB and cover 125 seconds at 48 kHz.
const MAX_DECODED_LOOP_FRAMES: u32 = 6_000_000;

#[derive(Debug)]
pub struct TransportClock {
    playing: AtomicBool,
    generation: AtomicU64,
    origin_beat: AtomicU64,
    controller_tx: Option<mpsc::Sender<ControllerClockCommand>>,
    controller_thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Default for TransportClock {
    fn default() -> Self {
        Self::new(
            &ControllerClockConfig {
                enabled: false,
                client_name: String::new(),
                output_match: String::new(),
            },
            120,
        )
    }
}

impl TransportClock {
    pub fn new(config: &ControllerClockConfig, initial_bpm: u16) -> Self {
        let (controller_tx, controller_thread) = if config.enabled {
            let (tx, rx) = mpsc::channel();
            let output = AlsaControllerClockOutput::new(config.clone());
            let handle = thread::Builder::new()
                .name("shsynth-controller-clock".into())
                .spawn(move || run_controller_clock(rx, Box::new(output), f64::from(initial_bpm)))
                .ok();
            match handle {
                Some(handle) => (Some(tx), Some(handle)),
                None => (None, None),
            }
        } else {
            (None, None)
        };
        Self {
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            origin_beat: AtomicU64::new(0),
            controller_tx,
            controller_thread: Mutex::new(controller_thread),
        }
    }

    pub fn play(&self, origin_beat: f64, bpm: u16) {
        self.origin_beat.store(
            (origin_beat.max(0.0) * BEAT_UNITS) as u64,
            Ordering::Release,
        );
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.playing.store(true, Ordering::Release);
        if let Some(tx) = &self.controller_tx {
            let _ = tx.send(ControllerClockCommand::Start(f64::from(bpm)));
        }
    }

    pub fn stop(&self) {
        if self.playing.swap(false, Ordering::AcqRel) {
            if let Some(tx) = &self.controller_tx {
                let _ = tx.send(ControllerClockCommand::Stop);
            }
        }
    }

    pub fn tempo(&self, bpm: f64) {
        if let Some(tx) = &self.controller_tx {
            let _ = tx.send(ControllerClockCommand::Tempo(bpm.clamp(20.0, 300.0)));
        }
    }

    /// Reposition the loop at a repeated Project boundary without emitting a
    /// second MIDI Start. Controller transport remains one continuous run.
    pub fn restart_cycle(&self, origin_beat: f64) {
        self.origin_beat.store(
            (origin_beat.max(0.0) * BEAT_UNITS) as u64,
            Ordering::Release,
        );
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

impl Drop for TransportClock {
    fn drop(&mut self) {
        self.stop();
        if let Some(tx) = &self.controller_tx {
            let _ = tx.send(ControllerClockCommand::Shutdown);
        }
        if let Ok(mut handle) = self.controller_thread.lock() {
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ControllerClockCommand {
    Start(f64),
    Tempo(f64),
    Stop,
    Shutdown,
}

#[derive(Clone, Copy, Debug)]
enum ControllerClockMessage {
    TimingClock,
    Start,
    Stop,
}

impl ControllerClockMessage {
    #[cfg(test)]
    const fn bytes(self) -> &'static [u8] {
        match self {
            Self::TimingClock => &[0xf8],
            Self::Start => &[0xfa],
            Self::Stop => &[0xfc],
        }
    }
}

trait ControllerClockOutput: Send {
    fn send(&mut self, message: ControllerClockMessage) -> std::result::Result<(), String>;
}

struct AlsaControllerClockOutput {
    config: ControllerClockConfig,
    connection: Option<AlsaDirectClockConnection>,
}

struct AlsaDirectClockConnection {
    sequencer: Seq,
    source_port: i32,
    destination: Addr,
}

impl AlsaControllerClockOutput {
    fn new(config: ControllerClockConfig) -> Self {
        Self {
            config,
            connection: None,
        }
    }

    fn connect(&mut self) -> std::result::Result<(), String> {
        let output =
            MidiOutput::new(&self.config.client_name).map_err(|error| error.to_string())?;
        let ports = output.ports();
        let names = ports
            .iter()
            .map(|port| output.port_name(port).unwrap_or_default())
            .collect::<Vec<_>>();
        let index = matching_controller_output_index(&names, &self.config.output_match)?;
        let destination = alsa_address_from_midir_name(&names[index])?;
        drop(output);

        let sequencer =
            Seq::open(None, Some(Direction::Playback), false).map_err(|error| error.to_string())?;
        let client_name = CString::new(self.config.client_name.as_str())
            .map_err(|_| "controller clock client name contains a NUL byte".to_owned())?;
        sequencer
            .set_client_name(&client_name)
            .map_err(|error| error.to_string())?;
        let destination_info = sequencer
            .get_any_port_info(destination)
            .map_err(|error| error.to_string())?;
        if !destination_info.get_capability().contains(PortCap::WRITE) {
            return Err(format!(
                "controller clock destination {}:{} is not writable",
                destination.client, destination.port
            ));
        }
        let port_name = CString::new("SHR-DAW controller clock only").expect("static port name");
        let source_port = sequencer
            .create_simple_port(
                &port_name,
                controller_clock_source_capabilities(),
                PortType::MIDI_GENERIC | PortType::APPLICATION,
            )
            .map_err(|error| error.to_string())?;
        self.connection = Some(AlsaDirectClockConnection {
            sequencer,
            source_port,
            destination,
        });
        Ok(())
    }
}

fn controller_clock_source_capabilities() -> PortCap {
    PortCap::READ | PortCap::NO_EXPORT
}

fn alsa_address_from_midir_name(name: &str) -> std::result::Result<Addr, String> {
    let address = name
        .rsplit_once(' ')
        .map(|(_, address)| address)
        .ok_or_else(|| format!("ALSA output {name:?} has no client:port address"))?;
    address
        .parse::<Addr>()
        .map_err(|_| format!("ALSA output {name:?} has an invalid client:port address"))
}

pub(crate) fn matching_controller_output_index(
    names: &[String],
    wanted: &str,
) -> std::result::Result<usize, String> {
    if wanted.trim().is_empty() || wanted.trim() != wanted {
        return Err("controller clock output must be one exact ALSA port name".into());
    }
    let stable_wanted = crate::controller_learn::stable_input_match(wanted);
    let matches = names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            (name == wanted || crate::controller_learn::stable_input_match(name) == stable_wanted)
                .then_some(index)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => Err(format!("controller clock output {wanted:?} is offline")),
        _ => Err(format!(
            "controller clock output {wanted:?} is ambiguous ({} exact stable-name matches)",
            matches.len()
        )),
    }
}

pub fn controller_clock_outputs(client_name: &str) -> Result<Vec<String>> {
    let output = MidiOutput::new(client_name)?;
    let mut names = output
        .ports()
        .iter()
        .filter_map(|port| output.port_name(port).ok())
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

impl ControllerClockOutput for AlsaControllerClockOutput {
    fn send(&mut self, message: ControllerClockMessage) -> std::result::Result<(), String> {
        if self.connection.is_none() {
            self.connect()?;
        }
        let connection = self
            .connection
            .as_mut()
            .expect("controller clock connection was established");
        let event_type = match message {
            ControllerClockMessage::TimingClock => EventType::Clock,
            ControllerClockMessage::Start => EventType::Start,
            ControllerClockMessage::Stop => EventType::Stop,
        };
        let mut event = Event::new(
            event_type,
            &EvQueueControl {
                queue: 0,
                value: (),
            },
        );
        event.set_source(connection.source_port);
        event.set_dest(connection.destination);
        event.set_direct();
        let result = connection
            .sequencer
            .event_output_direct(&mut event)
            .map(|_| ())
            .map_err(|error| error.to_string());
        if result.is_err() {
            self.connection = None;
        }
        result
    }
}

#[derive(Clone, Copy, Debug)]
struct ControllerClockPhase {
    interval_seconds: f64,
    next_tick_seconds: f64,
}

impl ControllerClockPhase {
    fn start(now: Duration, bpm: f64) -> Self {
        Self {
            interval_seconds: controller_clock_interval_seconds(bpm),
            next_tick_seconds: now.as_secs_f64(),
        }
    }

    fn tempo(&mut self, now: Duration, bpm: f64) {
        let now = now.as_secs_f64();
        let new_interval = controller_clock_interval_seconds(bpm);
        let remaining = (self.next_tick_seconds - now).max(0.0);
        let phase_remaining = (remaining / self.interval_seconds).clamp(0.0, 1.0);
        self.interval_seconds = new_interval;
        self.next_tick_seconds = now + new_interval * phase_remaining;
    }

    /// Return at most one due pulse. If scheduling was delayed, advance to
    /// the first future phase instead of sending a catch-up burst.
    fn take_due(&mut self, now: Duration) -> bool {
        let now = now.as_secs_f64();
        // `Duration` has nanosecond resolution while phase is retained as a
        // fractional second to avoid cumulative rounding. Treat conversion
        // to the nearest nanosecond as the same deadline.
        if now + 0.000_000_001 < self.next_tick_seconds {
            return false;
        }
        self.next_tick_seconds += self.interval_seconds;
        if self.next_tick_seconds <= now {
            let skipped = ((now - self.next_tick_seconds) / self.interval_seconds).floor() + 1.0;
            self.next_tick_seconds += skipped * self.interval_seconds;
        }
        true
    }

    fn next_tick(&self) -> Duration {
        Duration::from_secs_f64(self.next_tick_seconds)
    }
}

#[cfg(test)]
fn controller_clock_interval(bpm: f64) -> Duration {
    Duration::from_secs_f64(controller_clock_interval_seconds(bpm))
}

fn controller_clock_interval_seconds(bpm: f64) -> f64 {
    60.0 / bpm.clamp(20.0, 300.0) / 24.0
}

fn run_controller_clock(
    receiver: mpsc::Receiver<ControllerClockCommand>,
    mut output: Box<dyn ControllerClockOutput>,
    initial_bpm: f64,
) {
    let origin = Instant::now();
    let elapsed = || origin.elapsed();
    let mut phase = ControllerClockPhase::start(elapsed(), initial_bpm);
    let mut output_available = true;
    let mut clock_sent = false;
    let mut transport_running = false;
    loop {
        let timeout = phase.next_tick().saturating_sub(elapsed());
        match receiver.recv_timeout(timeout) {
            Ok(ControllerClockCommand::Start(bpm)) => {
                phase.tempo(elapsed(), bpm);
                if transport_running && output_available {
                    let _ = output.send(ControllerClockMessage::Stop);
                }
                if !clock_sent {
                    output_available = output.send(ControllerClockMessage::TimingClock).is_ok();
                    clock_sent = output_available;
                    let _ = phase.take_due(elapsed());
                }
                if output_available {
                    output_available = output.send(ControllerClockMessage::Start).is_ok();
                }
                transport_running = true;
            }
            Ok(ControllerClockCommand::Tempo(bpm)) => {
                phase.tempo(elapsed(), bpm);
            }
            Ok(ControllerClockCommand::Stop) => {
                if transport_running && output_available {
                    let _ = output.send(ControllerClockMessage::Stop);
                }
                transport_running = false;
            }
            Ok(ControllerClockCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if transport_running && output_available {
                    let _ = output.send(ControllerClockMessage::Stop);
                }
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if phase.take_due(elapsed()) && output_available {
                    output_available = output.send(ControllerClockMessage::TimingClock).is_ok();
                    clock_sent = output_available;
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct DecodedLoop {
    pub samples: Vec<[f32; 2]>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl DecodedLoop {
    pub fn open(path: &Path) -> Result<Self> {
        let mut reader =
            hound::WavReader::open(path).with_context(|| format!("open WAV {}", path.display()))?;
        let spec = reader.spec();
        if !matches!(spec.channels, 1 | 2) {
            bail!(
                "WAV must be mono or stereo (found {} channels)",
                spec.channels
            );
        }
        if !(8_000..=384_000).contains(&spec.sample_rate) {
            bail!("unsupported WAV sample rate {} Hz", spec.sample_rate);
        }
        let frames = checked_loop_frames(reader.duration())?;
        let mut samples = Vec::with_capacity(frames);
        match spec.sample_format {
            hound::SampleFormat::Float => {
                let mut raw = reader.samples::<f32>();
                while let Some(left) = raw.next() {
                    let left = checked_float_sample(left)?;
                    let right = if spec.channels == 1 {
                        left
                    } else {
                        checked_float_sample(raw.next().context("incomplete stereo WAV frame")?)?
                    };
                    samples.push([left, right]);
                }
            }
            hound::SampleFormat::Int => {
                let bits = u32::from(spec.bits_per_sample);
                if bits == 0 || bits > 32 {
                    bail!("unsupported WAV integer depth {}", spec.bits_per_sample);
                }
                let divisor = 2_f32.powi(bits.saturating_sub(1) as i32);
                let mut raw = reader.samples::<i32>();
                while let Some(left) = raw.next() {
                    let left = left.context("malformed integer WAV sample")? as f32 / divisor;
                    let right = if spec.channels == 1 {
                        left
                    } else {
                        raw.next()
                            .context("incomplete stereo WAV frame")?
                            .context("malformed integer WAV sample")? as f32
                            / divisor
                    };
                    samples.push([left, right]);
                }
            }
        }
        if samples.is_empty() || samples.len() != frames {
            bail!("WAV has no complete audio frames");
        }
        Ok(Self {
            samples,
            sample_rate: spec.sample_rate,
            channels: spec.channels,
        })
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.sample_rate))
    }
}

fn checked_loop_frames(frames: u32) -> Result<usize> {
    if frames > MAX_DECODED_LOOP_FRAMES {
        bail!("WAV has {frames} frames; the safe loop limit is {MAX_DECODED_LOOP_FRAMES} frames");
    }
    Ok(frames as usize)
}

fn checked_float_sample(sample: hound::Result<f32>) -> Result<f32> {
    let sample = sample.context("malformed float WAV sample")?;
    if !sample.is_finite() {
        bail!("WAV contains a non-finite float sample");
    }
    Ok(sample.clamp(-1.0, 1.0))
}

pub fn loops_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
                .join(".local/share")
        })
        .join("shsynth/loops")
}

pub fn list_wavs(directory: &Path) -> Vec<PathBuf> {
    let mut files = fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LibraryEntry {
    pub file: String,
    pub current: bool,
    pub saved_references: usize,
}

pub fn library_entries(
    directory: &Path,
    current: Option<&LoopSettings>,
    projects: &Path,
) -> Result<Vec<LibraryEntry>> {
    let mut references = std::collections::BTreeMap::<String, usize>::new();
    for name in crate::sequencer::list(projects) {
        let song = crate::sequencer::load(projects, &name)
            .with_context(|| format!("inspect saved Project {name}"))?;
        if let Some(settings) = song.audio_loop {
            *references.entry(settings.file).or_default() += 1;
        }
    }
    Ok(list_wavs(directory)
        .into_iter()
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .map(|file| LibraryEntry {
            current: current.is_some_and(|settings| settings.file == file),
            saved_references: references.get(&file).copied().unwrap_or(0),
            file,
        })
        .collect())
}

pub fn delete_library_file(
    directory: &Path,
    file: &str,
    current: Option<&LoopSettings>,
    projects: &Path,
) -> Result<()> {
    if Path::new(file).file_name().and_then(|name| name.to_str()) != Some(file)
        || !Path::new(file)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
    {
        bail!("unsafe private loop path");
    }
    let entry = library_entries(directory, current, projects)?
        .into_iter()
        .find(|entry| entry.file == file)
        .context("private loop is missing or is not a regular WAV file")?;
    if entry.current || entry.saved_references != 0 {
        bail!(
            "loop is referenced by the current Project ({}) and {} saved Project(s)",
            usize::from(entry.current),
            entry.saved_references
        );
    }
    let path = directory.join(file);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("private loop is not a regular file");
    }
    fs::remove_file(path)?;
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

pub fn import(source: &Path, destination: &Path) -> Result<(PathBuf, DecodedLoop)> {
    let decoded = DecodedLoop::open(source)?;
    fs::create_dir_all(destination)?;
    let original = source
        .file_name()
        .and_then(|name| name.to_str())
        .context("WAV filename is not valid UTF-8")?;
    let stem = Path::new(original)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("loop");
    let safe = crate::sequencer::safe_name(stem);
    for suffix in 1..=9999 {
        let target = if suffix == 1 {
            destination.join(format!("{safe}.wav"))
        } else {
            destination.join(format!("{safe}-{suffix}.wav"))
        };
        let mut output = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(output) => output,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("create {}", target.display()))
            }
        };
        let result = (|| -> Result<()> {
            let mut input = File::open(source)?;
            io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            drop(output);
            let _ = fs::remove_file(&target);
            return Err(error)
                .with_context(|| format!("copy private loop to {}", target.display()));
        }
        drop(output);
        fs::File::open(destination)?.sync_all()?;
        return Ok((target, decoded));
    }
    bail!("too many imported loops named {safe}")
}

pub fn bpm_candidates(measured: f64) -> [f64; 3] {
    [measured / 2.0, measured, measured * 2.0]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopAlignment {
    pub source_bpm: f64,
    pub length_beats: u32,
    pub bars: u32,
    pub transient_detected: bool,
}

pub fn analyze_alignment(decoded: &DecodedLoop, project_bpm: u16, meter: u8) -> LoopAlignment {
    let duration = decoded.duration().as_secs_f64().max(0.001);
    let meter = u32::from(meter.clamp(1, 16));
    let estimated = estimate_bpm(decoded);
    let measured_bpm = estimated.unwrap_or(f64::from(project_bpm));
    let measured_beats = (duration * measured_bpm / 60.0).round().max(1.0) as u32;
    let bars = ((measured_beats as f64 / f64::from(meter)).round() as u32).max(1);
    let length_beats = bars.saturating_mul(meter).max(1);
    LoopAlignment {
        source_bpm: (f64::from(length_beats) * 60.0 / duration).clamp(20.0, 300.0),
        length_beats,
        bars,
        transient_detected: estimated.is_some(),
    }
}

fn estimate_bpm(decoded: &DecodedLoop) -> Option<f64> {
    if decoded.samples.len() < ANALYSIS_HOP * 4 {
        return None;
    }
    let envelope = onset_envelope(decoded);
    let energy = envelope.iter().sum::<f64>();
    if energy <= f64::EPSILON {
        return None;
    }
    let windows_per_second = f64::from(decoded.sample_rate) / ANALYSIS_HOP as f64;
    let mut best = None;
    for bpm in (60..=200).rev() {
        let lag = (windows_per_second * 60.0 / f64::from(bpm)).round() as usize;
        if lag == 0 || lag >= envelope.len() {
            continue;
        }
        let score = envelope
            .iter()
            .skip(lag)
            .zip(envelope.iter())
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / (envelope.len() - lag) as f64;
        if score > best.map_or(0.0, |(_, score)| score) {
            best = Some((f64::from(bpm), score));
        }
    }
    best.filter(|(_, score)| *score > 1.0e-8)
        .map(|(bpm, _)| bpm)
}

fn onset_envelope(decoded: &DecodedLoop) -> Vec<f64> {
    let mut previous = 0.0;
    decoded
        .samples
        .chunks(ANALYSIS_HOP)
        .map(|chunk| {
            let energy = chunk
                .iter()
                .map(|sample| {
                    let mono = (f64::from(sample[0]) + f64::from(sample[1])) * 0.5;
                    mono * mono
                })
                .sum::<f64>()
                / chunk.len().max(1) as f64;
            let onset = (energy - previous).max(0.0);
            previous = energy;
            onset
        })
        .collect()
}

pub fn beat_to_frame(beat: f64, bpm: f64, sample_rate: u32) -> usize {
    (beat.max(0.0) * 60.0 / bpm.max(0.01) * f64::from(sample_rate)).round() as usize
}

pub fn bar_to_beat(bars: u32, meter: u8) -> u32 {
    bars.saturating_mul(u32::from(meter.clamp(1, 16)))
}

pub fn fade_frames(sample_rate: u32, slice_frames: usize) -> usize {
    ((f64::from(sample_rate) * 0.005).round() as usize)
        .max(1)
        .min(slice_frames.saturating_div(4).max(1))
}

pub fn render_sample(
    samples: &[[f32; 2]],
    region_start: usize,
    region_len: usize,
    phase: f64,
    fade: usize,
) -> [f32; 2] {
    if region_len == 0 || samples.is_empty() {
        return [0.0; 2];
    }
    let relative = (phase - region_start as f64).rem_euclid(region_len as f64);
    let positioned = region_start as f64 + relative;
    let index = positioned.floor() as usize;
    let next = if index + 1 < region_start + region_len {
        index + 1
    } else {
        region_start
    };
    let fraction = (positioned - index as f64) as f32;
    let a = samples.get(index).copied().unwrap_or([0.0; 2]);
    let b = samples.get(next).copied().unwrap_or(a);
    let edge = relative.min(region_len as f64 - relative);
    let envelope = (edge / fade.max(1) as f64).clamp(0.0, 1.0) as f32;
    [
        (a[0] + (b[0] - a[0]) * fraction) * envelope,
        (a[1] + (b[1] - a[1]) * fraction) * envelope,
    ]
}

pub fn song_position_beats(song: &Song, order: usize, row: usize) -> f64 {
    let prior_rows = song
        .order
        .iter()
        .take(order)
        .filter_map(|number| song.patterns.get(number))
        .map(|pattern| pattern.rows.len())
        .sum::<usize>();
    (prior_rows + row) as f64 / f64::from(song.steps_per_beat)
}

pub fn loop_phase_from_song(origin_beat: f64, offset_beats: i32, loop_beats: f64) -> f64 {
    if loop_beats > 0.0 {
        (origin_beat - f64::from(offset_beats)).rem_euclid(loop_beats) / loop_beats
    } else {
        0.0
    }
}

#[derive(Clone, Debug, Default)]
pub struct LoopStatus {
    pub loaded: bool,
    pub playing: bool,
    pub file: Option<String>,
    pub source_rate: u32,
    pub source_channels: u16,
    pub duration: Duration,
    pub elapsed: Duration,
    pub error: Option<String>,
}

pub struct LoopPlayer {
    config: LoopPlayerConfig,
    clock: Arc<TransportClock>,
    active: Option<Active>,
    status: LoopStatus,
    position: Arc<AtomicU64>,
    meter: Arc<AtomicMeter>,
    preview: bool,
}

struct Active {
    jack: JackClient,
    callback: Box<CallbackData>,
    client_state: Arc<LoopClientState>,
}

impl LoopPlayer {
    pub fn new(config: &LoopPlayerConfig, clock: Arc<TransportClock>) -> Self {
        Self {
            config: config.clone(),
            clock,
            active: None,
            status: LoopStatus::default(),
            position: Arc::new(AtomicU64::new(0)),
            meter: Arc::new(AtomicMeter::default()),
            preview: false,
        }
    }

    pub fn load(&mut self, decoded: DecodedLoop, settings: &LoopSettings) -> Result<()> {
        self.stop_backend();
        self.position.store(0, Ordering::Release);
        self.clear_meter();
        self.status = LoopStatus::default();
        self.preview = false;
        if decoded.samples.is_empty()
            || !(8_000..=384_000).contains(&decoded.sample_rate)
            || !matches!(decoded.channels, 1 | 2)
            || decoded
                .samples
                .iter()
                .flatten()
                .any(|sample| !sample.is_finite())
        {
            self.status.error = Some("invalid decoded WAV loop".into());
            bail!("invalid decoded WAV loop");
        }
        if !(2_000..=30_000).contains(&settings.source_bpm_x100)
            || settings.length_beats == 0
            || !(-16_384..=16_384).contains(&settings.offset_beats)
        {
            self.status.error = Some("invalid private loop settings".into());
            bail!("invalid private loop settings");
        }
        let source_rate = decoded.sample_rate;
        let source_channels = decoded.channels;
        self.status = LoopStatus {
            file: Some(settings.file.clone()),
            source_rate,
            source_channels,
            ..LoopStatus::default()
        };
        let result = (|| -> Result<(Active, usize)> {
            let mut jack = JackClient::open(&self.config.client_name)?;
            let jack_rate = jack.sample_rate();
            require_native_rate(source_rate, jack_rate)?;
            let left =
                jack.register_audio_port(LOOP_OUTPUT_PORT_NAMES[0], PortDirection::Output)?;
            let right =
                jack.register_audio_port(LOOP_OUTPUT_PORT_NAMES[1], PortDirection::Output)?;
            let interpreted = settings.interpreted_bpm();
            let start = beat_to_frame(f64::from(settings.start_beat), interpreted, source_rate)
                .min(decoded.samples.len().saturating_sub(1));
            let requested =
                beat_to_frame(f64::from(settings.length_beats), interpreted, source_rate);
            let length = requested
                .max(1)
                .min(decoded.samples.len().saturating_sub(start));
            let client_state = Arc::new(LoopClientState {
                active: AtomicBool::new(true),
                published_meter: Arc::clone(&self.meter),
            });
            let mut callback = Box::new(CallbackData {
                left,
                right,
                port_get_buffer: jack.port_get_buffer(),
                renderer: LoopRenderer {
                    samples: decoded.samples,
                    source_rate,
                    interpreted_bpm: interpreted,
                    region_start: start,
                    region_len: length,
                    offset_beats: settings.offset_beats,
                    fade: fade_frames(source_rate, length),
                    phase: start as f64,
                    seen_generation: u64::MAX,
                    clock: Arc::clone(&self.clock),
                    position: Arc::clone(&self.position),
                    meter: MeterAccumulator::new(MAX_LOOP_CALLBACK_FRAMES)?,
                    client_state: Arc::clone(&client_state),
                    meter_active: false,
                },
            });
            // SAFETY: `callback` stays boxed until after JACK is deactivated.
            unsafe {
                jack.set_process_callback(
                    process_callback,
                    ((&mut *callback) as *mut CallbackData).cast(),
                )?;
                jack.set_shutdown_callback(
                    shutdown_callback,
                    Arc::as_ptr(&callback.renderer.client_state)
                        .cast_mut()
                        .cast(),
                );
            }
            activate_and_connect(&mut jack, &self.config.outputs, left, right)?;
            Ok((
                Active {
                    jack,
                    callback,
                    client_state,
                },
                length,
            ))
        })();
        match result {
            Ok((active, length)) => {
                self.status.loaded = true;
                self.status.duration =
                    Duration::from_secs_f64(length as f64 / f64::from(source_rate));
                self.active = Some(active);
                Ok(())
            }
            Err(error) => {
                self.clear_meter();
                self.status.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub fn status(&self) -> LoopStatus {
        let mut status = self.status.clone();
        let client_active = self
            .active
            .as_ref()
            .is_some_and(|active| active.client_state.active.load(Ordering::Acquire));
        status.playing = status.loaded
            && self.clock.playing.load(Ordering::Acquire)
            && (client_active || self.preview);
        if status.loaded && !client_active && !self.preview {
            status.error = Some("JACK loop client inactive".into());
        }
        if status.source_rate > 0 {
            status.elapsed = Duration::from_secs_f64(
                self.position.load(Ordering::Acquire) as f64 / f64::from(status.source_rate),
            );
        }
        status
    }

    pub fn meter_snapshot(&self) -> Option<MeterSnapshot> {
        let active = self.active.as_ref()?;
        (active.client_state.active.load(Ordering::Acquire)
            && self.clock.playing.load(Ordering::Acquire))
        .then(|| self.meter.load())
    }

    #[doc(hidden)]
    pub(crate) fn set_preview_status(&mut self, status: LoopStatus) {
        if status.source_rate > 0 {
            self.position.store(
                (status.elapsed.as_secs_f64() * f64::from(status.source_rate)).round() as u64,
                Ordering::Release,
            );
        }
        self.status = status;
        self.preview = true;
    }

    pub fn stop(&self) {
        self.clock.stop();
        self.clear_meter();
    }

    pub fn unload(&mut self) {
        self.stop_backend();
        self.position.store(0, Ordering::Release);
        self.clear_meter();
        self.status = LoopStatus::default();
        self.preview = false;
    }

    fn stop_backend(&mut self) {
        if let Some(mut active) = self.active.take() {
            active.jack.deactivate();
            // Keep the callback allocation alive until JACK is inactive.
            drop(active.callback);
        }
        self.clear_meter();
    }

    fn clear_meter(&self) {
        self.meter.publish(MeterSnapshot::default());
    }
}

impl Drop for LoopPlayer {
    fn drop(&mut self) {
        self.stop_backend();
    }
}

struct CallbackData {
    left: *mut JackPort,
    right: *mut JackPort,
    port_get_buffer: PortGetBuffer,
    renderer: LoopRenderer,
}

struct LoopRenderer {
    samples: Vec<[f32; 2]>,
    source_rate: u32,
    interpreted_bpm: f64,
    region_start: usize,
    region_len: usize,
    offset_beats: i32,
    fade: usize,
    phase: f64,
    seen_generation: u64,
    clock: Arc<TransportClock>,
    position: Arc<AtomicU64>,
    meter: MeterAccumulator,
    client_state: Arc<LoopClientState>,
    meter_active: bool,
}

struct LoopClientState {
    active: AtomicBool,
    published_meter: Arc<AtomicMeter>,
}

fn activate_and_connect(
    jack: &mut JackClient,
    outputs: &[String],
    left: *mut JackPort,
    right: *mut JackPort,
) -> Result<()> {
    let destinations = loop_destinations(outputs)?;
    jack.activate().context("activate JACK loop player")?;
    for (port, destination) in [(left, destinations[0]), (right, destinations[1])] {
        if let Err(error) = jack.connect_port_to_external(port, destination) {
            jack.deactivate();
            return Err(error)
                .with_context(|| format!("connect JACK loop output to {destination}"));
        }
    }
    Ok(())
}

fn loop_destinations(outputs: &[String]) -> Result<[&str; 2]> {
    let [left, right] = outputs else {
        bail!("loop.output requires exactly two JACK destination ports");
    };
    Ok([left, right])
}

unsafe extern "C" fn process_callback(frames: c_uint, argument: *mut c_void) -> c_int {
    let data = unsafe { &mut *(argument.cast::<CallbackData>()) };
    let left = unsafe { (data.port_get_buffer)(data.left, frames) }.cast::<f32>();
    let right = unsafe { (data.port_get_buffer)(data.right, frames) }.cast::<f32>();
    if left.is_null() || right.is_null() {
        clear_renderer_meter(&mut data.renderer);
        return 0;
    }
    let left = unsafe { std::slice::from_raw_parts_mut(left, frames as usize) };
    let right = unsafe { std::slice::from_raw_parts_mut(right, frames as usize) };
    render_output(&mut data.renderer, left, right);
    0
}

unsafe extern "C" fn shutdown_callback(argument: *mut c_void) {
    let state = unsafe { &*(argument.cast::<LoopClientState>()) };
    state.active.store(false, Ordering::Release);
    state.published_meter.publish(MeterSnapshot::default());
}

#[inline]
fn clear_renderer_meter(data: &mut LoopRenderer) {
    if data.meter_active {
        data.meter.reset();
        data.meter_active = false;
    }
    data.client_state
        .published_meter
        .publish(MeterSnapshot::default());
}

#[inline]
fn render_output(data: &mut LoopRenderer, left: &mut [f32], right: &mut [f32]) {
    left.fill(0.0);
    right.fill(0.0);
    if left.len() != right.len()
        || left.len() > MAX_LOOP_CALLBACK_FRAMES
        || !data.client_state.active.load(Ordering::Acquire)
        || !data.clock.playing.load(Ordering::Acquire)
        || data.region_len == 0
    {
        clear_renderer_meter(data);
        return;
    }
    let generation = data.clock.generation.load(Ordering::Acquire);
    if generation != data.seen_generation {
        data.meter.reset();
        data.seen_generation = generation;
        let origin = data.clock.origin_beat.load(Ordering::Acquire) as f64 / BEAT_UNITS;
        let loop_beats =
            data.region_len as f64 * data.interpreted_bpm / (60.0 * f64::from(data.source_rate));
        let beat_phase = loop_phase_from_song(origin, data.offset_beats, loop_beats);
        data.phase = data.region_start as f64 + beat_phase * data.region_len as f64;
    }
    data.meter_active = true;
    let end = (data.region_start + data.region_len) as f64;
    for (left_out, right_out) in left.iter_mut().zip(right.iter_mut()) {
        while data.phase >= end {
            data.phase -= data.region_len as f64;
        }
        let sample = render_sample(
            &data.samples,
            data.region_start,
            data.region_len,
            data.phase,
            data.fade,
        );
        let sample = data.meter.process(StereoFrame::new(sample[0], sample[1]));
        *left_out = sample.left;
        *right_out = sample.right;
        data.phase += 1.0;
    }
    data.client_state
        .published_meter
        .publish(data.meter.snapshot_and_clear_peak());
    data.position.store(
        (data.phase - data.region_start as f64).max(0.0) as u64,
        Ordering::Release,
    );
}

fn require_native_rate(source_rate: u32, jack_rate: u32) -> Result<()> {
    if source_rate != jack_rate {
        bail!(
            "WAV is {source_rate} Hz but JACK is {jack_rate} Hz; restart JACK at {source_rate} Hz for native loop playback"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::allocation_test::assert_no_allocations;
    use hound::{SampleFormat, WavSpec, WavWriter};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("shr-loop-{name}-{}-{nanos}", std::process::id()))
    }

    fn quarter_note(bpm: f64) -> Duration {
        Duration::from_secs_f64(60.0 / bpm)
    }

    #[test]
    fn controller_clock_is_exactly_twenty_four_ppqn_at_representative_tempos() {
        for bpm in [20.0, 60.0, 120.0, 173.0, 300.0] {
            let mut phase = ControllerClockPhase::start(Duration::ZERO, bpm);
            let end = quarter_note(bpm);
            let mut pulses = Vec::new();
            while phase.next_tick() < end {
                let at = phase.next_tick();
                assert!(phase.take_due(at));
                pulses.push(at);
            }
            assert_eq!(pulses.len(), 24, "wrong PPQN at {bpm} BPM");
            for pair in pulses.windows(2) {
                let actual = pair[1] - pair[0];
                assert!(actual.abs_diff(controller_clock_interval(bpm)) <= Duration::from_nanos(1));
            }
        }
    }

    #[test]
    fn controller_clock_tempo_change_keeps_phase_and_never_catches_up_in_a_burst() {
        let mut phase = ControllerClockPhase::start(Duration::ZERO, 120.0);
        assert!(phase.take_due(Duration::ZERO));
        let old_next = phase.next_tick();
        let change = old_next / 2;
        phase.tempo(change, 60.0);
        let expected = change + controller_clock_interval(60.0) / 2;
        assert!(phase.next_tick().abs_diff(expected) <= Duration::from_nanos(2));
        let deadline = phase.next_tick();
        assert!(!phase.take_due(deadline - Duration::from_nanos(5)));
        assert!(phase.take_due(deadline));
        assert!(!phase.take_due(deadline));

        let delayed = deadline + Duration::from_secs(2);
        assert!(phase.take_due(delayed));
        assert!(!phase.take_due(delayed));
        assert!(phase.next_tick() > delayed);
    }

    #[test]
    fn controller_clock_is_independent_of_swing_pages_and_destinations() {
        let pulses = |irrelevant_event_offsets: &[Duration]| {
            let mut phase = ControllerClockPhase::start(Duration::ZERO, 100.0);
            let mut result = Vec::new();
            for _ in 0..48 {
                let at = phase.next_tick();
                assert!(phase.take_due(at));
                result.push(at);
            }
            let _ = irrelevant_event_offsets;
            result
        };
        let straight = pulses(&[]);
        let swung_many_destinations = pulses(&[
            Duration::from_millis(17),
            Duration::from_millis(211),
            Duration::from_millis(499),
        ]);
        assert_eq!(straight, swung_many_destinations);
    }

    #[test]
    fn clock_only_protocol_has_no_channel_voice_sysex_continue_or_song_position() {
        let bytes = [
            ControllerClockMessage::TimingClock.bytes(),
            ControllerClockMessage::Start.bytes(),
            ControllerClockMessage::Stop.bytes(),
        ];
        assert_eq!(bytes, [&[0xf8][..], &[0xfa][..], &[0xfc][..]]);
        assert!(bytes
            .iter()
            .flat_map(|message| message.iter())
            .all(|byte| !matches!(byte, 0x80..=0xef | 0xf0 | 0xf2 | 0xfb)));
        let capabilities = controller_clock_source_capabilities();
        assert!(capabilities.contains(PortCap::NO_EXPORT));
        assert!(!capabilities.contains(PortCap::SUBS_READ));
    }

    #[test]
    fn controller_clock_output_uses_one_exact_stable_alsa_port_name() {
        let names = vec![
            "Minilab3:Minilab3 MIDI 32:0".to_owned(),
            "Minilab3:Minilab3 DIN THRU 32:1".to_owned(),
            "Other:Other MIDI 40:0".to_owned(),
        ];
        assert_eq!(
            matching_controller_output_index(&names, "Minilab3:Minilab3 MIDI").unwrap(),
            0
        );
        assert!(matching_controller_output_index(&names, "Minilab3").is_err());
        let ambiguous = vec![
            "Minilab3:Minilab3 MIDI 32:0".to_owned(),
            "Minilab3:Minilab3 MIDI 41:0".to_owned(),
        ];
        assert!(matching_controller_output_index(&ambiguous, "Minilab3:Minilab3 MIDI").is_err());
        assert_eq!(
            alsa_address_from_midir_name(&names[0]).unwrap(),
            Addr {
                client: 32,
                port: 0
            }
        );
        assert!(alsa_address_from_midir_name("Minilab3:Minilab3 MIDI").is_err());
    }

    struct RecordingClockOutput {
        messages: Arc<Mutex<Vec<Vec<u8>>>>,
        fail: bool,
    }

    impl ControllerClockOutput for RecordingClockOutput {
        fn send(&mut self, message: ControllerClockMessage) -> std::result::Result<(), String> {
            if self.fail {
                return Err("offline".into());
            }
            self.messages.lock().unwrap().push(message.bytes().to_vec());
            Ok(())
        }
    }

    #[test]
    fn controller_transport_sends_one_start_and_stop_and_offline_shutdown_joins() {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let recorded = Arc::clone(&messages);
        let worker = thread::spawn(move || {
            run_controller_clock(
                rx,
                Box::new(RecordingClockOutput {
                    messages: recorded,
                    fail: false,
                }),
                120.0,
            )
        });
        tx.send(ControllerClockCommand::Start(120.0)).unwrap();
        thread::sleep(Duration::from_millis(55));
        tx.send(ControllerClockCommand::Tempo(90.0)).unwrap();
        tx.send(ControllerClockCommand::Stop).unwrap();
        tx.send(ControllerClockCommand::Stop).unwrap();
        thread::sleep(Duration::from_millis(30));
        tx.send(ControllerClockCommand::Shutdown).unwrap();
        worker.join().unwrap();
        let messages = messages.lock().unwrap();
        assert_eq!(
            messages.iter().filter(|m| m.as_slice() == [0xfa]).count(),
            1
        );
        assert_eq!(
            messages.iter().filter(|m| m.as_slice() == [0xfc]).count(),
            1
        );
        assert!(messages.iter().any(|m| m.as_slice() == [0xf8]));
        let start = messages
            .iter()
            .position(|m| m.as_slice() == [0xfa])
            .unwrap();
        let stop = messages
            .iter()
            .position(|m| m.as_slice() == [0xfc])
            .unwrap();
        assert!(messages[..start].iter().any(|m| m.as_slice() == [0xf8]));
        assert!(messages[stop + 1..].iter().any(|m| m.as_slice() == [0xf8]));
        assert!(messages
            .iter()
            .all(|message| matches!(message.as_slice(), [0xf8] | [0xfa] | [0xfc])));

        let (tx, rx) = mpsc::channel();
        let offline = thread::spawn(move || {
            run_controller_clock(
                rx,
                Box::new(RecordingClockOutput {
                    messages: Arc::new(Mutex::new(Vec::new())),
                    fail: true,
                }),
                120.0,
            )
        });
        tx.send(ControllerClockCommand::Start(120.0)).unwrap();
        tx.send(ControllerClockCommand::Stop).unwrap();
        tx.send(ControllerClockCommand::Shutdown).unwrap();
        offline.join().unwrap();

        let messages = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let recorded = Arc::clone(&messages);
        let shutdown = thread::spawn(move || {
            run_controller_clock(
                rx,
                Box::new(RecordingClockOutput {
                    messages: recorded,
                    fail: false,
                }),
                120.0,
            )
        });
        tx.send(ControllerClockCommand::Start(120.0)).unwrap();
        tx.send(ControllerClockCommand::Shutdown).unwrap();
        shutdown.join().unwrap();
        let messages = messages.lock().unwrap();
        assert_eq!(
            messages.iter().filter(|m| m.as_slice() == [0xfa]).count(),
            1
        );
        assert_eq!(
            messages.iter().filter(|m| m.as_slice() == [0xfc]).count(),
            1
        );
    }

    #[test]
    fn disabled_controller_clock_starts_no_worker_and_sends_nothing() {
        let clock = TransportClock::default();
        assert!(clock.controller_tx.is_none());
        assert!(clock.controller_thread.lock().unwrap().is_none());
        clock.play(0.0, 120);
        clock.tempo(90.0);
        clock.stop();
    }

    fn test_renderer(samples: Vec<[f32; 2]>) -> (LoopRenderer, Arc<AtomicMeter>) {
        let clock = Arc::new(TransportClock::default());
        let position = Arc::new(AtomicU64::new(0));
        let published_meter = Arc::new(AtomicMeter::default());
        (
            LoopRenderer {
                source_rate: 48_000,
                interpreted_bpm: 120.0,
                region_start: 0,
                region_len: samples.len(),
                offset_beats: 0,
                fade: 0,
                phase: 0.0,
                seen_generation: u64::MAX,
                clock,
                position,
                meter: MeterAccumulator::new(4).unwrap(),
                client_state: Arc::new(LoopClientState {
                    active: AtomicBool::new(true),
                    published_meter: Arc::clone(&published_meter),
                }),
                meter_active: false,
                samples,
            },
            published_meter,
        )
    }

    #[test]
    fn bpm_interpretations_and_musical_frame_math() {
        assert_eq!(bpm_candidates(120.0), [60.0, 120.0, 240.0]);
        assert_eq!(beat_to_frame(4.0, 120.0, 48_000), 96_000);
        assert_eq!(bar_to_beat(2, 3), 6);
        assert_eq!(fade_frames(48_000, 100), 25);
        assert_eq!(fade_frames(48_000, 48_000), 240);
    }

    #[test]
    fn order_and_row_convert_to_absolute_beats() {
        let config = crate::config::RuntimeConfig::default().external_midi;
        let mut song = Song::new(&config);
        let setup = song.patterns[&0].clone();
        song.patterns
            .insert(1, crate::sequencer::Pattern::empty_like_setup(8, &setup));
        song.order = vec![0, 1];
        assert_eq!(song_position_beats(&song, 1, 4), 17.0);
    }

    #[test]
    fn auto_alignment_estimates_pulses_and_snaps_to_bars() {
        let sample_rate = 48_000;
        let mut samples = vec![[0.0, 0.0]; sample_rate as usize * 2];
        for beat in 0..4 {
            let start = beat * 24_000;
            for frame in start..start + 512 {
                samples[frame] = [1.0, 1.0];
            }
        }
        let decoded = DecodedLoop {
            samples,
            sample_rate,
            channels: 1,
        };
        let alignment = analyze_alignment(&decoded, 90, 4);
        assert!(alignment.transient_detected);
        assert_eq!(alignment.length_beats, 4);
        assert_eq!(alignment.bars, 1);
        assert!((alignment.source_bpm - 120.0).abs() < 0.01);
    }

    #[test]
    fn auto_alignment_falls_back_to_project_tempo_for_flat_audio() {
        let decoded = DecodedLoop {
            samples: vec![[0.0, 0.0]; 48_000 * 3],
            sample_rate: 48_000,
            channels: 2,
        };
        let alignment = analyze_alignment(&decoded, 100, 3);
        assert!(!alignment.transient_detected);
        assert_eq!(alignment.length_beats, 6);
        assert_eq!(alignment.bars, 2);
        assert!((alignment.source_bpm - 120.0).abs() < 0.01);
    }

    #[test]
    fn song_phase_accounts_for_bar_offsets() {
        assert_eq!(loop_phase_from_song(0.0, 0, 16.0), 0.0);
        assert_eq!(loop_phase_from_song(4.0, 4, 16.0), 0.0);
        assert_eq!(loop_phase_from_song(0.0, 4, 16.0), 0.75);
        assert_eq!(loop_phase_from_song(0.0, -4, 16.0), 0.25);
    }

    #[test]
    fn mono_and_stereo_decode_and_malformed_files_are_safe() {
        let base = temp_dir("decode");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let mono = base.join("mono.wav");
        let mut writer = WavWriter::create(
            &mono,
            WavSpec {
                channels: 1,
                sample_rate: 44_100,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        )
        .unwrap();
        writer.write_sample::<i16>(16_384).unwrap();
        writer.write_sample::<i16>(-16_384).unwrap();
        writer.finalize().unwrap();
        let decoded = DecodedLoop::open(&mono).unwrap();
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.samples[0], [0.5, 0.5]);

        let stereo = base.join("stereo.wav");
        let mut writer = WavWriter::create(
            &stereo,
            WavSpec {
                channels: 2,
                sample_rate: 44_100,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        )
        .unwrap();
        writer.write_sample::<i16>(8192).unwrap();
        writer.write_sample::<i16>(-8192).unwrap();
        writer.finalize().unwrap();
        let decoded = DecodedLoop::open(&stereo).unwrap();
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.samples[0], [0.25, -0.25]);

        let bad = base.join("bad.wav");
        fs::write(&bad, b"not a wave").unwrap();
        assert!(DecodedLoop::open(&bad).is_err());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn decoded_loop_frame_limit_is_explicit_and_bounded() {
        assert_eq!(
            checked_loop_frames(MAX_DECODED_LOOP_FRAMES).unwrap(),
            MAX_DECODED_LOOP_FRAMES as usize
        );
        assert!(checked_loop_frames(MAX_DECODED_LOOP_FRAMES + 1)
            .unwrap_err()
            .to_string()
            .contains("safe loop limit"));
    }

    #[test]
    fn import_copies_wavs_to_private_storage_without_replacing_existing_files() {
        let base = temp_dir("import");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let source = base.join("My Loop!.wav");
        let mut writer = WavWriter::create(
            &source,
            WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        )
        .unwrap();
        writer.write_sample::<i16>(0).unwrap();
        writer.finalize().unwrap();
        let destination = base.join("private");

        let (first, decoded) = import(&source, &destination).unwrap();
        let (second, _) = import(&source, &destination).unwrap();

        assert_eq!(decoded.sample_rate, 48_000);
        assert!(first.starts_with(&destination));
        assert!(second.starts_with(&destination));
        assert_ne!(first, second);
        assert!(first.exists());
        assert!(second.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn listing_ignores_directories_and_symlinks_named_like_wavs() {
        let base = temp_dir("list");
        fs::create_dir_all(base.join("directory.wav")).unwrap();
        fs::write(base.join("real.wav"), []).unwrap();
        std::os::unix::fs::symlink(base.join("real.wav"), base.join("alias.wav")).unwrap();

        assert_eq!(list_wavs(&base), [base.join("real.wav")]);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn invalid_decoded_loop_is_rejected_before_opening_jack() {
        let config = crate::config::RuntimeConfig::default().loop_player;
        let mut player = LoopPlayer::new(&config, Arc::new(TransportClock::default()));
        let settings = LoopSettings {
            file: "empty.wav".into(),
            source_bpm_x100: 12_000,
            interpretation: crate::sequencer::BpmInterpretation::Normal,
            start_beat: 0,
            length_beats: 4,
            offset_beats: 0,
        };

        let error = player
            .load(
                DecodedLoop {
                    samples: Vec::new(),
                    sample_rate: 48_000,
                    channels: 2,
                },
                &settings,
            )
            .unwrap_err();
        assert!(error.to_string().contains("invalid decoded WAV loop"));
    }

    #[test]
    fn transport_clock_tracks_play_restart_tempo_and_stop() {
        let clock = TransportClock::default();
        clock.play(3.5, 120);
        assert!(clock.playing.load(Ordering::Acquire));
        assert_eq!(clock.origin_beat.load(Ordering::Acquire), 3_500_000);
        let first_generation = clock.generation.load(Ordering::Acquire);

        clock.tempo(150.25);
        clock.play(1.0, 90);
        assert!(clock.generation.load(Ordering::Acquire) > first_generation);
        assert_eq!(clock.origin_beat.load(Ordering::Acquire), 1_000_000);

        clock.stop();
        assert!(!clock.playing.load(Ordering::Acquire));
    }

    #[test]
    fn native_sample_rendering_wraps_with_bounded_fades() {
        let data = [[1.0, 1.0], [0.5, 0.5], [-1.0, -1.0], [0.0, 0.0]];
        assert_eq!(render_sample(&data, 0, 4, 0.0, 1), [0.0, 0.0]);
        assert!((render_sample(&data, 0, 4, 1.5, 1)[0] + 0.25).abs() < 0.0001);
        assert!((render_sample(&data, 0, 4, 4.5, 1)[0] - 0.375).abs() < 0.0001);
        assert!(fade_frames(48_000, 4) <= 1);
    }

    #[test]
    fn loop_callback_meter_accumulates_publishes_and_separates_stereo() {
        let (mut renderer, published) = test_renderer(vec![[0.5, 0.25]; 4]);
        renderer.clock.play(0.0, 120);
        let mut left = [0.0; 4];
        let mut right = [0.0; 4];

        assert_no_allocations(|| render_output(&mut renderer, &mut left, &mut right));

        assert_eq!(left, [0.0, 0.5, 0.5, 0.5]);
        assert_eq!(right, [0.0, 0.25, 0.25, 0.25]);
        let snapshot = published.load();
        assert_eq!(snapshot.peak, StereoFrame::new(0.5, 0.25));
        assert!((snapshot.rms.left - 0.433_012_7).abs() < 0.000_001);
        assert!((snapshot.rms.right - 0.216_506_35).abs() < 0.000_001);
    }

    #[test]
    fn stopped_silent_and_restarted_loop_cannot_leave_stale_meter_levels() {
        let (mut renderer, published) = test_renderer(vec![[0.8, 0.4]; 4]);
        renderer.clock.play(0.0, 120);
        let mut left = [0.0; 4];
        let mut right = [0.0; 4];
        render_output(&mut renderer, &mut left, &mut right);
        assert_eq!(published.load().peak, StereoFrame::new(0.8, 0.4));

        renderer.clock.stop();
        render_output(&mut renderer, &mut left, &mut right);
        assert_eq!(left, [0.0; 4]);
        assert_eq!(right, [0.0; 4]);
        assert_eq!(published.load(), MeterSnapshot::default());

        published.publish(MeterSnapshot {
            peak: StereoFrame::new(0.6, 0.3),
            ..MeterSnapshot::default()
        });
        unsafe {
            shutdown_callback(
                Arc::as_ptr(&renderer.client_state)
                    .cast_mut()
                    .cast::<c_void>(),
            )
        };
        assert_eq!(published.load(), MeterSnapshot::default());
        left.fill(1.0);
        right.fill(1.0);
        render_output(&mut renderer, &mut left, &mut right);
        assert_eq!(left, [0.0; 4]);
        assert_eq!(right, [0.0; 4]);
        assert_eq!(published.load(), MeterSnapshot::default());

        renderer.samples.fill([0.1, 0.2]);
        renderer.client_state.active.store(true, Ordering::Release);
        renderer.clock.play(0.0, 120);
        render_output(&mut renderer, &mut left, &mut right);
        assert_eq!(published.load().peak, StereoFrame::new(0.1, 0.2));
    }

    #[test]
    fn unloaded_failed_and_oversized_loop_states_clear_meter_availability() {
        let config = crate::config::RuntimeConfig::default().loop_player;
        let mut player = LoopPlayer::new(&config, Arc::new(TransportClock::default()));
        player.meter.publish(MeterSnapshot {
            peak: StereoFrame::new(0.9, 0.7),
            ..MeterSnapshot::default()
        });
        player.unload();
        assert!(player.meter_snapshot().is_none());
        assert_eq!(player.meter.load(), MeterSnapshot::default());

        let settings = LoopSettings {
            file: "empty.wav".into(),
            source_bpm_x100: 12_000,
            interpretation: crate::sequencer::BpmInterpretation::Normal,
            start_beat: 0,
            length_beats: 4,
            offset_beats: 0,
        };
        assert!(player
            .load(
                DecodedLoop {
                    samples: Vec::new(),
                    sample_rate: 48_000,
                    channels: 2,
                },
                &settings,
            )
            .is_err());
        assert!(player.meter_snapshot().is_none());
        assert_eq!(player.meter.load(), MeterSnapshot::default());

        let (mut renderer, published) = test_renderer(vec![[0.5, 0.5]; 4]);
        renderer.clock.play(0.0, 120);
        let mut left = vec![1.0; MAX_LOOP_CALLBACK_FRAMES + 1];
        let mut right = vec![1.0; MAX_LOOP_CALLBACK_FRAMES + 1];
        render_output(&mut renderer, &mut left, &mut right);
        assert!(left.iter().all(|sample| *sample == 0.0));
        assert!(right.iter().all(|sample| *sample == 0.0));
        assert_eq!(published.load(), MeterSnapshot::default());
    }

    #[test]
    fn loop_meter_keeps_the_existing_owned_stereo_route() {
        let config = crate::config::RuntimeConfig::default().loop_player;
        assert_eq!(LOOP_OUTPUT_PORT_NAMES, ["output_l", "output_r"]);
        let destinations = loop_destinations(&config.outputs).unwrap();
        assert_eq!(
            destinations,
            [config.outputs[0].as_str(), config.outputs[1].as_str()]
        );
        assert!(loop_destinations(&config.outputs[..1]).is_err());
    }

    #[test]
    fn native_loop_playback_requires_matching_jack_rate() {
        assert!(require_native_rate(44_100, 44_100).is_ok());
        assert!(require_native_rate(44_100, 48_000)
            .unwrap_err()
            .to_string()
            .contains("restart JACK at 44100 Hz"));
    }

    #[test]
    fn loop_library_deletes_only_unreferenced_regular_wavs() {
        let base = temp_dir("library-delete");
        let loops = base.join("loops");
        let projects = base.join("projects");
        fs::create_dir_all(&loops).unwrap();
        fs::write(loops.join("free.wav"), b"private").unwrap();
        fs::write(loops.join("used.wav"), b"private").unwrap();
        std::os::unix::fs::symlink(loops.join("free.wav"), loops.join("alias.wav")).unwrap();

        let mut song = Song::new(&crate::config::RuntimeConfig::default().external_midi);
        song.name = "saved".into();
        song.audio_loop = Some(LoopSettings {
            file: "used.wav".into(),
            source_bpm_x100: 12_000,
            interpretation: crate::sequencer::BpmInterpretation::Normal,
            start_beat: 0,
            length_beats: 4,
            offset_beats: 0,
        });
        crate::sequencer::save(&projects, &song, false).unwrap();

        let entries = library_entries(&loops, None, &projects).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.file == "used.wav")
                .unwrap()
                .saved_references,
            1
        );
        assert!(delete_library_file(&loops, "used.wav", None, &projects).is_err());
        assert!(delete_library_file(&loops, "alias.wav", None, &projects).is_err());
        assert!(delete_library_file(&loops, "../free.wav", None, &projects).is_err());
        delete_library_file(&loops, "free.wav", None, &projects).unwrap();
        assert!(!loops.join("free.wav").exists());
        assert!(loops.join("used.wav").exists());
        let _ = fs::remove_dir_all(base);
    }
}
