//! JACK-synchronized FT2 WAV loops. Decode/import work happens before JACK
//! activation; the process callback only reads immutable PCM and writes two
//! bounded output buffers.

use crate::config::LoopPlayerConfig;
use crate::sequencer::{LoopSettings, Song};
use anyhow::{bail, Context, Result};
use libc::{c_char, c_int, c_uint, c_ulong, c_void};
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const JACK_DEFAULT_AUDIO_TYPE: &[u8] = b"32 bit float mono audio\0";
const JACK_PORT_IS_OUTPUT: c_ulong = 2;
const JACK_NO_START_SERVER: c_uint = 1;
const BEAT_UNITS: f64 = 1_000_000.0;
const ANALYSIS_HOP: usize = 1024;

#[derive(Debug)]
pub struct TransportClock {
    playing: AtomicBool,
    generation: AtomicU64,
    origin_beat: AtomicU64,
}

impl Default for TransportClock {
    fn default() -> Self {
        Self {
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            origin_beat: AtomicU64::new(0),
        }
    }
}

impl TransportClock {
    pub fn play(&self, origin_beat: f64, _bpm: u16) {
        self.origin_beat.store(
            (origin_beat.max(0.0) * BEAT_UNITS) as u64,
            Ordering::Release,
        );
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.playing.store(true, Ordering::Release);
    }

    pub fn stop(&self) {
        self.playing.store(false, Ordering::Release);
    }

    pub fn tempo(&self, _bpm: f64) {}
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
        let raw = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .map(|sample| {
                    let sample = sample.context("malformed float WAV sample")?;
                    if !sample.is_finite() {
                        bail!("WAV contains a non-finite float sample");
                    }
                    Ok(sample.clamp(-1.0, 1.0))
                })
                .collect::<Result<Vec<_>>>()?,
            hound::SampleFormat::Int => {
                let bits = u32::from(spec.bits_per_sample);
                if bits == 0 || bits > 32 {
                    bail!("unsupported WAV integer depth {}", spec.bits_per_sample);
                }
                let divisor = 2_f32.powi(bits.saturating_sub(1) as i32);
                reader
                    .samples::<i32>()
                    .map(|sample| {
                        sample
                            .map(|value| value as f32 / divisor)
                            .context("malformed integer WAV sample")
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        };
        if raw.is_empty() || raw.len() % usize::from(spec.channels) != 0 {
            bail!("WAV has no complete audio frames");
        }
        let samples = if spec.channels == 1 {
            raw.into_iter().map(|sample| [sample, sample]).collect()
        } else {
            raw.chunks_exact(2).map(|pair| [pair[0], pair[1]]).collect()
        };
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
}

struct Active {
    jack: JackClient,
    callback: Box<CallbackData>,
}

impl LoopPlayer {
    pub fn new(config: &LoopPlayerConfig, clock: Arc<TransportClock>) -> Self {
        Self {
            config: config.clone(),
            clock,
            active: None,
            status: LoopStatus::default(),
            position: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn load(&mut self, decoded: DecodedLoop, settings: &LoopSettings) -> Result<()> {
        if decoded.samples.is_empty()
            || !(8_000..=384_000).contains(&decoded.sample_rate)
            || !matches!(decoded.channels, 1 | 2)
            || decoded
                .samples
                .iter()
                .flatten()
                .any(|sample| !sample.is_finite())
        {
            bail!("invalid decoded WAV loop");
        }
        if !(2_000..=30_000).contains(&settings.source_bpm_x100)
            || settings.length_beats == 0
            || !(-16_384..=16_384).contains(&settings.offset_beats)
        {
            bail!("invalid private loop settings");
        }
        self.stop_backend();
        self.position.store(0, Ordering::Release);
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
            let left = jack.register_output("output_l")?;
            let right = jack.register_output("output_r")?;
            let interpreted = settings.interpreted_bpm();
            let start = beat_to_frame(f64::from(settings.start_beat), interpreted, source_rate)
                .min(decoded.samples.len().saturating_sub(1));
            let requested =
                beat_to_frame(f64::from(settings.length_beats), interpreted, source_rate);
            let length = requested
                .max(1)
                .min(decoded.samples.len().saturating_sub(start));
            let mut callback = Box::new(CallbackData {
                left,
                right,
                port_get_buffer: jack.api.port_get_buffer,
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
            });
            jack.set_callback((&mut *callback) as *mut CallbackData)?;
            jack.activate_and_connect(&self.config.outputs, left, right)?;
            Ok((Active { jack, callback }, length))
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
                self.status.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub fn status(&self) -> LoopStatus {
        let mut status = self.status.clone();
        status.playing = status.loaded && self.clock.playing.load(Ordering::Acquire);
        if status.source_rate > 0 {
            status.elapsed = Duration::from_secs_f64(
                self.position.load(Ordering::Acquire) as f64 / f64::from(status.source_rate),
            );
        }
        status
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
    }

    pub fn stop(&self) {
        self.clock.stop();
    }

    pub fn unload(&mut self) {
        self.stop_backend();
        self.position.store(0, Ordering::Release);
        self.status = LoopStatus::default();
    }

    fn stop_backend(&mut self) {
        if let Some(mut active) = self.active.take() {
            active.jack.deactivate();
            // Keep the callback allocation alive until JACK is inactive.
            drop(active.callback);
        }
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
}

#[repr(C)]
struct JackOpaque {
    _private: [u8; 0],
}
#[repr(C)]
struct JackPort {
    _private: [u8; 0],
}
type ClientOpen = unsafe extern "C" fn(*const c_char, c_uint, *mut c_uint) -> *mut JackOpaque;
type ClientClose = unsafe extern "C" fn(*mut JackOpaque) -> c_int;
type PortRegister = unsafe extern "C" fn(
    *mut JackOpaque,
    *const c_char,
    *const c_char,
    c_ulong,
    c_ulong,
) -> *mut JackPort;
type SetProcess = unsafe extern "C" fn(
    *mut JackOpaque,
    unsafe extern "C" fn(c_uint, *mut c_void) -> c_int,
    *mut c_void,
) -> c_int;
type Activate = unsafe extern "C" fn(*mut JackOpaque) -> c_int;
type Deactivate = unsafe extern "C" fn(*mut JackOpaque) -> c_int;
type Connect = unsafe extern "C" fn(*mut JackOpaque, *const c_char, *const c_char) -> c_int;
type PortName = unsafe extern "C" fn(*const JackPort) -> *const c_char;
type SampleRate = unsafe extern "C" fn(*const JackOpaque) -> c_uint;
type PortGetBuffer = unsafe extern "C" fn(*mut JackPort, c_uint) -> *mut c_void;

struct JackApi {
    handle: *mut c_void,
    client_close: ClientClose,
    port_register: PortRegister,
    set_process: SetProcess,
    activate: Activate,
    deactivate: Deactivate,
    connect: Connect,
    port_name: PortName,
    sample_rate: SampleRate,
    port_get_buffer: PortGetBuffer,
}

struct JackClient {
    client: *mut JackOpaque,
    api: JackApi,
    active: bool,
}

impl JackClient {
    fn open(name: &str) -> Result<Self> {
        let name = CString::new(name)?;
        unsafe {
            let handle = libc::dlopen(c"libjack.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                bail!("JACK library unavailable");
            }
            let loaded = (|| -> Result<(ClientOpen, JackApi)> {
                Ok((
                    symbol(handle, b"jack_client_open\0")?,
                    JackApi {
                        handle,
                        client_close: symbol(handle, b"jack_client_close\0")?,
                        port_register: symbol(handle, b"jack_port_register\0")?,
                        set_process: symbol(handle, b"jack_set_process_callback\0")?,
                        activate: symbol(handle, b"jack_activate\0")?,
                        deactivate: symbol(handle, b"jack_deactivate\0")?,
                        connect: symbol(handle, b"jack_connect\0")?,
                        port_name: symbol(handle, b"jack_port_name\0")?,
                        sample_rate: symbol(handle, b"jack_get_sample_rate\0")?,
                        port_get_buffer: symbol(handle, b"jack_port_get_buffer\0")?,
                    },
                ))
            })();
            let (open, api) = match loaded {
                Ok(loaded) => loaded,
                Err(error) => {
                    libc::dlclose(handle);
                    return Err(error);
                }
            };
            let mut status = 0;
            let client = open(name.as_ptr(), JACK_NO_START_SERVER, &mut status);
            if client.is_null() {
                libc::dlclose(handle);
                bail!("JACK server unavailable (status {status})");
            }
            Ok(Self {
                client,
                api,
                active: false,
            })
        }
    }

    fn sample_rate(&self) -> u32 {
        unsafe { (self.api.sample_rate)(self.client) }
    }

    fn register_output(&mut self, name: &str) -> Result<*mut JackPort> {
        let name = CString::new(name)?;
        let port = unsafe {
            (self.api.port_register)(
                self.client,
                name.as_ptr(),
                JACK_DEFAULT_AUDIO_TYPE.as_ptr().cast(),
                JACK_PORT_IS_OUTPUT,
                0,
            )
        };
        if port.is_null() {
            bail!("register JACK loop output {name:?}");
        }
        Ok(port)
    }

    fn set_callback(&mut self, data: *mut CallbackData) -> Result<()> {
        if unsafe { (self.api.set_process)(self.client, process_callback, data.cast()) } != 0 {
            bail!("set JACK loop callback");
        }
        Ok(())
    }

    fn activate_and_connect(
        &mut self,
        outputs: &[String],
        left: *mut JackPort,
        right: *mut JackPort,
    ) -> Result<()> {
        if outputs.len() != 2 {
            bail!("loop.output requires exactly two JACK destination ports");
        }
        let destinations = [
            CString::new(outputs[0].as_str())?,
            CString::new(outputs[1].as_str())?,
        ];
        if unsafe { (self.api.activate)(self.client) } != 0 {
            bail!("activate JACK loop player");
        }
        self.active = true;
        for (port, destination) in [(left, &destinations[0]), (right, &destinations[1])] {
            let source = unsafe { (self.api.port_name)(port) };
            if source.is_null() {
                self.deactivate();
                bail!("JACK loop player returned an unnamed port");
            }
            let source = unsafe { CStr::from_ptr(source) };
            if unsafe { (self.api.connect)(self.client, source.as_ptr(), destination.as_ptr()) }
                != 0
            {
                let label = destination.to_string_lossy().into_owned();
                self.deactivate();
                bail!("connect JACK loop output to {label}");
            }
        }
        Ok(())
    }

    fn deactivate(&mut self) {
        if self.active {
            unsafe { (self.api.deactivate)(self.client) };
            self.active = false;
        }
    }
}

impl Drop for JackClient {
    fn drop(&mut self) {
        self.deactivate();
        unsafe {
            (self.api.client_close)(self.client);
            libc::dlclose(self.api.handle);
        }
    }
}

unsafe fn symbol<T: Copy>(handle: *mut c_void, name: &[u8]) -> Result<T> {
    let pointer = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
    if pointer.is_null() {
        bail!("JACK symbol unavailable");
    }
    Ok(unsafe { std::mem::transmute_copy(&pointer) })
}

unsafe extern "C" fn process_callback(frames: c_uint, argument: *mut c_void) -> c_int {
    let data = unsafe { &mut *(argument.cast::<CallbackData>()) };
    let left = unsafe { (data.port_get_buffer)(data.left, frames) }.cast::<f32>();
    let right = unsafe { (data.port_get_buffer)(data.right, frames) }.cast::<f32>();
    if left.is_null() || right.is_null() {
        return 0;
    }
    let left = unsafe { std::slice::from_raw_parts_mut(left, frames as usize) };
    let right = unsafe { std::slice::from_raw_parts_mut(right, frames as usize) };
    left.fill(0.0);
    right.fill(0.0);
    if !data.clock.playing.load(Ordering::Acquire) || data.region_len == 0 {
        return 0;
    }
    let generation = data.clock.generation.load(Ordering::Acquire);
    if generation != data.seen_generation {
        data.seen_generation = generation;
        let origin = data.clock.origin_beat.load(Ordering::Acquire) as f64 / BEAT_UNITS;
        let loop_beats =
            data.region_len as f64 * data.interpreted_bpm / (60.0 * f64::from(data.source_rate));
        let beat_phase = loop_phase_from_song(origin, data.offset_beats, loop_beats);
        data.phase = data.region_start as f64 + beat_phase * data.region_len as f64;
    }
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
        *left_out = sample[0];
        *right_out = sample[1];
        data.phase += 1.0;
    }
    data.position.store(
        (data.phase - data.region_start as f64).max(0.0) as u64,
        Ordering::Release,
    );
    0
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
    use hound::{SampleFormat, WavSpec, WavWriter};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("shr-loop-{name}-{}-{nanos}", std::process::id()))
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
