//! Engine-independent synchronized JACK capture. One callback transfers every
//! armed channel into one bounded interleaved SPSC ring; a non-real-time worker
//! writes one mono WAV per track and atomically publishes the take directory.
use crate::config::{AudioCaptureConfig, CaptureTrackConfig, CaptureTrackRole};
use crate::dsp::StereoFrame;
use crate::jack::{Client as JackClient, Port as JackPort, PortDirection, PortGetBuffer};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::cell::UnsafeCell;
use std::ffi::{c_int, c_uint, c_void, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const MAX_CAPTURE_TRACKS: usize = 64;
const MANIFEST_VERSION: u32 = 1;
const WAV_MAX_DATA_BYTES: u64 = u32::MAX as u64 - 36;
const MONO_WAV_MAX_FRAMES: u64 = WAV_MAX_DATA_BYTES / 3;
const STEREO_WAV_MAX_FRAMES: u64 = WAV_MAX_DATA_BYTES / 6;
const MIN_FREE_BYTES: u64 = 64 * 1024 * 1024;
const STATUS_UPDATE_FRAMES: u64 = 4096;

const FAULT_NONE: u32 = 0;
const FAULT_OVERFLOW: u32 = 1;
const FAULT_CALLBACK_SIZE: u32 = 2;
const FAULT_JACK_SHUTDOWN: u32 = 3;
const FAULT_SOURCE_LOST: u32 = 4;
const FAULT_XRUN: u32 = 5;
const FAULT_NULL_BUFFER: u32 = 6;
const FAULT_WRITER: u32 = 7;

struct InterleavedRing {
    samples: Box<[UnsafeCell<f32>]>,
    channels: usize,
    capacity_frames: usize,
    read: AtomicUsize,
    write: AtomicUsize,
    dropped: AtomicU64,
    overflows: AtomicU64,
    high_water: AtomicUsize,
}

// Exactly one callback producer and one disk consumer access disjoint frames,
// coordinated by acquire/release frame indices.
unsafe impl Sync for InterleavedRing {}

impl InterleavedRing {
    fn new(channels: usize, capacity_frames: usize) -> Result<Self> {
        if !(1..=MAX_CAPTURE_TRACKS).contains(&channels) {
            bail!("capture needs 1..={MAX_CAPTURE_TRACKS} armed tracks");
        }
        let capacity_frames = capacity_frames.max(2).saturating_add(1);
        let samples = channels
            .checked_mul(capacity_frames)
            .context("capture ring size overflow")?;
        Ok(Self {
            samples: (0..samples).map(|_| UnsafeCell::new(0.0)).collect(),
            channels,
            capacity_frames,
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
            overflows: AtomicU64::new(0),
            high_water: AtomicUsize::new(0),
        })
    }

    fn used_frames(&self, read: usize, write: usize) -> usize {
        if write >= read {
            write - read
        } else {
            self.capacity_frames - read + write
        }
    }

    /// Copies a whole callback or none of it, preserving equal track lengths.
    /// `buffers` contains preallocated callback-local source pointers.
    unsafe fn push_raw(&self, buffers: &[*const f32], frames: usize) -> bool {
        if buffers.len() != self.channels || buffers.iter().any(|buffer| buffer.is_null()) {
            self.dropped.fetch_add(frames as u64, Ordering::Relaxed);
            return false;
        }
        self.push_with(frames, |channel, frame| unsafe {
            *buffers[channel].add(frame)
        })
    }

    unsafe fn push_cells(&self, buffers: &[UnsafeCell<*const f32>], frames: usize) -> bool {
        if buffers.len() != self.channels
            || buffers
                .iter()
                .any(|buffer| unsafe { (*buffer.get()).is_null() })
        {
            self.dropped.fetch_add(frames as u64, Ordering::Relaxed);
            return false;
        }
        self.push_with(frames, |channel, frame| unsafe {
            *(*buffers[channel].get()).add(frame)
        })
    }

    fn push_with(&self, frames: usize, mut sample: impl FnMut(usize, usize) -> f32) -> bool {
        let read = self.read.load(Ordering::Acquire);
        let mut write = self.write.load(Ordering::Relaxed);
        let used = self.used_frames(read, write);
        let free = self.capacity_frames - 1 - used;
        if frames > free {
            self.dropped.fetch_add(frames as u64, Ordering::Relaxed);
            self.overflows.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        for frame in 0..frames {
            let destination = write * self.channels;
            for channel in 0..self.channels {
                // SAFETY: the producer exclusively owns the unpublished frame.
                unsafe { *self.samples[destination + channel].get() = sample(channel, frame) };
            }
            write = (write + 1) % self.capacity_frames;
        }
        self.write.store(write, Ordering::Release);
        let now_used = used + frames;
        self.high_water.fetch_max(now_used, Ordering::Relaxed);
        true
    }

    #[inline]
    fn push_stereo(&self, frames: &[StereoFrame]) -> bool {
        if self.channels != 2 {
            self.dropped
                .fetch_add(frames.len() as u64, Ordering::Relaxed);
            return false;
        }
        self.push_with(frames.len(), |channel, frame| match channel {
            0 => frames[frame].left,
            _ => frames[frame].right,
        })
    }

    fn pop_interleaved(&self, output: &mut [f32]) -> usize {
        let maximum_frames = output.len() / self.channels;
        let mut read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        let frames = self.used_frames(read, write).min(maximum_frames);
        for frame in 0..frames {
            let source = read * self.channels;
            let destination = frame * self.channels;
            for channel in 0..self.channels {
                // SAFETY: the consumer exclusively owns every published frame
                // before the read index advances.
                output[destination + channel] = unsafe { *self.samples[source + channel].get() };
            }
            read = (read + 1) % self.capacity_frames;
        }
        if frames > 0 {
            self.read.store(read, Ordering::Release);
        }
        frames
    }

    fn is_empty(&self) -> bool {
        self.read.load(Ordering::Acquire) == self.write.load(Ordering::Acquire)
    }
}

const FINAL_CAPTURE_IDLE: u32 = 0;
const FINAL_CAPTURE_ARMED: u32 = 1;
const FINAL_CAPTURE_ACTIVE: u32 = 2;
const FINAL_CAPTURE_STOP: u32 = 3;

#[derive(Clone, Debug, Default)]
pub struct FinalMixRecorderStatus {
    pub recording: bool,
    pub elapsed: Duration,
    pub bytes: u64,
    pub total_frames: u64,
    pub dropped_frames: u64,
    pub overflow_events: u64,
    pub writer_high_water_frames: usize,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}

struct FinalMixShared {
    ring: InterleavedRing,
    mode: AtomicU32,
    writer_running: AtomicBool,
    fault: AtomicU32,
    accepted_frames: AtomicU64,
    written_frames: AtomicU64,
    sample_rate: u32,
    maximum_callback_frames: usize,
}

#[derive(Clone)]
pub(crate) struct FinalMixCapture {
    shared: Arc<FinalMixShared>,
}

impl FinalMixCapture {
    /// Called once with the exact final playback frames. The transition from
    /// armed to active and from stop-requested to idle can therefore occur
    /// only at a complete callback boundary.
    #[inline]
    pub(crate) fn capture(&self, frames: &[StereoFrame]) {
        let mut mode = self.shared.mode.load(Ordering::Acquire);
        if mode == FINAL_CAPTURE_ARMED {
            let _ = self.shared.mode.compare_exchange(
                FINAL_CAPTURE_ARMED,
                FINAL_CAPTURE_ACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            mode = self.shared.mode.load(Ordering::Acquire);
        }
        if mode == FINAL_CAPTURE_STOP {
            self.shared
                .mode
                .store(FINAL_CAPTURE_IDLE, Ordering::Release);
            self.shared.writer_running.store(false, Ordering::Release);
            return;
        }
        if mode != FINAL_CAPTURE_ACTIVE {
            return;
        }
        if frames.is_empty() || frames.len() > self.shared.maximum_callback_frames {
            self.fault(FAULT_CALLBACK_SIZE);
            return;
        }
        if !self.shared.ring.push_stereo(frames) {
            self.fault(FAULT_OVERFLOW);
            return;
        }
        self.shared
            .accepted_frames
            .fetch_add(frames.len() as u64, Ordering::Relaxed);
    }

    pub(crate) fn source_lost(&self) {
        if self.recording() {
            self.fault(FAULT_SOURCE_LOST);
        }
    }

    pub(crate) fn xrun(&self) {
        if self.recording() {
            self.fault(FAULT_XRUN);
        }
    }

    pub(crate) fn jack_shutdown(&self) {
        if self.recording() {
            self.fault(FAULT_JACK_SHUTDOWN);
        }
    }

    pub(crate) fn callback_violation(&self) {
        if self.recording() {
            self.fault(FAULT_CALLBACK_SIZE);
        }
    }

    pub(crate) fn invalid_buffer(&self) {
        if self.recording() {
            self.fault(FAULT_NULL_BUFFER);
        }
    }

    fn recording(&self) -> bool {
        matches!(
            self.shared.mode.load(Ordering::Acquire),
            FINAL_CAPTURE_ARMED | FINAL_CAPTURE_ACTIVE | FINAL_CAPTURE_STOP
        )
    }

    fn fault(&self, code: u32) {
        self.shared
            .fault
            .compare_exchange(FAULT_NONE, code, Ordering::AcqRel, Ordering::Relaxed)
            .ok();
        self.shared
            .mode
            .store(FINAL_CAPTURE_IDLE, Ordering::Release);
        self.shared.writer_running.store(false, Ordering::Release);
    }
}

pub(crate) struct FinalMixRecorder {
    directory: PathBuf,
    shared: Arc<FinalMixShared>,
    status: Arc<Mutex<FinalMixRecorderStatus>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl FinalMixRecorder {
    pub(crate) fn new(
        directory: PathBuf,
        sample_rate: u32,
        ring_frames: usize,
        maximum_callback_frames: usize,
    ) -> Result<Self> {
        if !(8_000..=384_000).contains(&sample_rate) {
            bail!("invalid final-mix sample rate");
        }
        if maximum_callback_frames == 0 {
            bail!("invalid final-mix callback capacity");
        }
        Ok(Self {
            directory,
            shared: Arc::new(FinalMixShared {
                ring: InterleavedRing::new(2, ring_frames)?,
                mode: AtomicU32::new(FINAL_CAPTURE_IDLE),
                writer_running: AtomicBool::new(false),
                fault: AtomicU32::new(FAULT_NONE),
                accepted_frames: AtomicU64::new(0),
                written_frames: AtomicU64::new(0),
                sample_rate,
                maximum_callback_frames,
            }),
            status: Arc::new(Mutex::new(FinalMixRecorderStatus::default())),
            worker: None,
        })
    }

    pub(crate) fn capture_handle(&self) -> FinalMixCapture {
        FinalMixCapture {
            shared: Arc::clone(&self.shared),
        }
    }

    pub(crate) fn is_recording(&self) -> bool {
        self.worker.is_some() && self.shared.mode.load(Ordering::Acquire) != FINAL_CAPTURE_IDLE
    }

    pub(crate) fn start(&mut self, optional_name: Option<&str>) -> Result<()> {
        self.reap_finished()?;
        if self.worker.is_some()
            || self.shared.mode.load(Ordering::Acquire) != FINAL_CAPTURE_IDLE
            || !self.shared.ring.is_empty()
        {
            bail!("final-mix recording is already active");
        }
        fs::create_dir_all(&self.directory)?;
        let recovered = recover_interrupted(&self.directory)?;
        if available_bytes(&self.directory)? < MIN_FREE_BYTES {
            bail!("less than 64 MiB free in recording directory");
        }
        let stem = final_mix_stem(optional_name);
        let final_path = unique_file(&self.directory, &stem, "wav")?;
        let temporary = final_path.with_extension("wav.part");
        self.shared.fault.store(FAULT_NONE, Ordering::Release);
        self.shared.accepted_frames.store(0, Ordering::Release);
        self.shared.written_frames.store(0, Ordering::Release);
        self.shared.ring.dropped.store(0, Ordering::Release);
        self.shared.ring.overflows.store(0, Ordering::Release);
        self.shared.ring.high_water.store(0, Ordering::Release);
        self.shared.writer_running.store(true, Ordering::Release);
        if let Ok(mut status) = self.status.lock() {
            *status = FinalMixRecorderStatus {
                recording: true,
                path: Some(final_path.clone()),
                error: (!recovered.is_empty())
                    .then(|| format!("recovered {} interrupted recording(s)", recovered.len())),
                ..FinalMixRecorderStatus::default()
            };
        }
        let shared = Arc::clone(&self.shared);
        let status = Arc::clone(&self.status);
        let worker_final = final_path.clone();
        let worker_temporary = temporary.clone();
        self.worker = Some(
            thread::Builder::new()
                .name("shr-final-mix-writer".into())
                .spawn(move || {
                    let result = write_final_mix(
                        &worker_temporary,
                        &worker_final,
                        &shared,
                        &status,
                        WriterBehavior::default(),
                    );
                    shared.writer_running.store(false, Ordering::Release);
                    shared.mode.store(FINAL_CAPTURE_IDLE, Ordering::Release);
                    if let Err(error) = result {
                        shared
                            .fault
                            .compare_exchange(
                                FAULT_NONE,
                                FAULT_WRITER,
                                Ordering::AcqRel,
                                Ordering::Relaxed,
                            )
                            .ok();
                        if let Ok(mut public) = status.lock() {
                            public.recording = false;
                            public.error = Some(error.to_string());
                            public.path = worker_temporary.exists().then_some(worker_temporary);
                        }
                    }
                })
                .context("start final-mix writer thread")?,
        );
        self.shared
            .mode
            .store(FINAL_CAPTURE_ARMED, Ordering::Release);
        Ok(())
    }

    pub(crate) fn request_stop(&self) {
        let mode = self.shared.mode.load(Ordering::Acquire);
        if matches!(mode, FINAL_CAPTURE_ARMED | FINAL_CAPTURE_ACTIVE) {
            self.shared
                .mode
                .store(FINAL_CAPTURE_STOP, Ordering::Release);
        }
    }

    /// Use after JACK deactivation: the last completed callback is already a
    /// deterministic boundary, so no future callback is needed to consume the
    /// stop request.
    pub(crate) fn stop_after_deactivate(&mut self) -> Result<()> {
        if self.worker.is_none() {
            return Ok(());
        }
        self.shared
            .mode
            .store(FINAL_CAPTURE_IDLE, Ordering::Release);
        self.shared.writer_running.store(false, Ordering::Release);
        self.join_worker()
    }

    pub(crate) fn finish_stop(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.shared.mode.load(Ordering::Acquire) == FINAL_CAPTURE_STOP
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(1));
        }
        if self.shared.mode.load(Ordering::Acquire) == FINAL_CAPTURE_STOP {
            bail!("final-mix stop boundary timed out; audio callback is unavailable");
        }
        self.join_worker()
    }

    pub(crate) fn status(&mut self) -> FinalMixRecorderStatus {
        let _ = self.reap_finished();
        let mut public = self
            .status
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default();
        public.recording =
            self.worker.is_some() && self.shared.mode.load(Ordering::Acquire) != FINAL_CAPTURE_IDLE;
        public.total_frames = if public.recording {
            self.shared.accepted_frames.load(Ordering::Relaxed)
        } else {
            self.shared.written_frames.load(Ordering::Relaxed)
        };
        public.elapsed = Duration::from_secs_f64(
            public.total_frames as f64 / f64::from(self.shared.sample_rate),
        );
        let written_frames = self.shared.written_frames.load(Ordering::Relaxed);
        public.bytes = if written_frames == 0 && public.path.is_none() {
            0
        } else {
            44 + written_frames.saturating_mul(6)
        };
        public.dropped_frames = self.shared.ring.dropped.load(Ordering::Relaxed);
        public.overflow_events = self.shared.ring.overflows.load(Ordering::Relaxed);
        public.writer_high_water_frames = self.shared.ring.high_water.load(Ordering::Relaxed);
        let fault = self.shared.fault.load(Ordering::Acquire);
        if fault != FAULT_NONE && public.error.is_none() {
            public.error = Some(fault_message(fault).into());
        }
        public
    }

    fn reap_finished(&mut self) -> Result<()> {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            self.join_worker()?;
        }
        Ok(())
    }

    fn join_worker(&mut self) -> Result<()> {
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow!("final-mix writer thread panicked"))?;
        }
        let fault = self.shared.fault.load(Ordering::Acquire);
        if fault != FAULT_NONE {
            bail!(fault_message(fault));
        }
        Ok(())
    }
}

impl Drop for FinalMixRecorder {
    fn drop(&mut self) {
        let _ = self.stop_after_deactivate();
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecorderTrackStatus {
    pub label: String,
    pub armed: bool,
    pub preferred_source: String,
    pub resolved: bool,
    pub peak_dbfs: Option<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct RecorderStatus {
    pub recording: bool,
    pub incomplete: bool,
    pub elapsed: Duration,
    pub bytes: u64,
    pub sample_rate: u32,
    pub total_frames: u64,
    pub dropped_frames: u64,
    pub overflow_events: u64,
    pub callback_violations: u64,
    pub xruns: u64,
    pub writer_high_water_frames: usize,
    pub active_tracks: usize,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    pub tracks: Vec<RecorderTrackStatus>,
}

struct SharedStatus {
    started: Instant,
    public: RecorderStatus,
}

struct CallbackData {
    ring: Arc<InterleavedRing>,
    running: Arc<AtomicBool>,
    capture_enabled: Arc<AtomicBool>,
    fault: Arc<AtomicU32>,
    xruns: Arc<AtomicU64>,
    callback_violations: Arc<AtomicU64>,
    accepted_frames: Arc<AtomicU64>,
    ports: Box<[*mut JackPort]>,
    port_ids: Box<[u32]>,
    buffers: Box<[UnsafeCell<*const f32>]>,
    peaks: Arc<[AtomicU32]>,
    maximum_callback_frames: usize,
    port_get_buffer: PortGetBuffer,
}
unsafe impl Send for CallbackData {}

pub struct AudioRecorder {
    config: AudioCaptureConfig,
    available_sources: Vec<String>,
    status: Arc<Mutex<SharedStatus>>,
    active: Option<Active>,
}

struct Active {
    jack: JackClient,
    running: Arc<AtomicBool>,
    capture_enabled: Arc<AtomicBool>,
    fault: Arc<AtomicU32>,
    xruns: Arc<AtomicU64>,
    callback_violations: Arc<AtomicU64>,
    accepted_frames: Arc<AtomicU64>,
    ring: Arc<InterleavedRing>,
    peaks: Arc<[AtomicU32]>,
    worker: Option<thread::JoinHandle<()>>,
    callback_data: Box<CallbackData>,
}

impl AudioRecorder {
    pub fn new(config: AudioCaptureConfig, available_sources: Vec<String>) -> Self {
        let public = idle_status(&config, &available_sources);
        Self {
            config,
            available_sources,
            status: Arc::new(Mutex::new(SharedStatus {
                started: Instant::now(),
                public,
            })),
            active: None,
        }
    }

    pub fn status(&self) -> RecorderStatus {
        let mut public = self
            .status
            .lock()
            .map(|status| {
                let mut public = status.public.clone();
                if public.recording {
                    public.elapsed = status.started.elapsed();
                }
                public
            })
            .unwrap_or_default();
        if let Some(active) = &self.active {
            public.dropped_frames = active.ring.dropped.load(Ordering::Relaxed);
            public.overflow_events = active.ring.overflows.load(Ordering::Relaxed);
            public.writer_high_water_frames = active.ring.high_water.load(Ordering::Relaxed);
            public.callback_violations = active.callback_violations.load(Ordering::Relaxed);
            public.xruns = active.xruns.load(Ordering::Relaxed);
            public.total_frames = active.accepted_frames.load(Ordering::Relaxed);
            if !active.running.load(Ordering::Acquire) {
                public.recording = false;
            }
            for (track, peak) in public
                .tracks
                .iter_mut()
                .filter(|track| track.armed)
                .zip(active.peaks.iter())
            {
                let value = f32::from_bits(peak.load(Ordering::Relaxed));
                track.peak_dbfs = (value > 0.0).then(|| 20.0 * value.log10());
            }
            let fault = active.fault.load(Ordering::Acquire);
            if fault != FAULT_NONE && public.error.is_none() {
                public.incomplete = true;
                public.error = Some(fault_message(fault).into());
            }
        }
        public
    }

    pub fn update_configuration(
        &mut self,
        config: AudioCaptureConfig,
        available_sources: Vec<String>,
    ) -> Result<()> {
        if self.active.is_some() {
            bail!("stop recording before changing capture tracks");
        }
        self.config = config;
        self.available_sources = available_sources;
        if let Ok(mut status) = self.status.lock() {
            status.started = Instant::now();
            status.public = idle_status(&self.config, &self.available_sources);
        }
        Ok(())
    }

    #[doc(hidden)]
    pub(crate) fn set_preview_status(&self, status: RecorderStatus) {
        if let Ok(mut shared) = self.status.lock() {
            shared.started = Instant::now() - status.elapsed;
            shared.public = status;
        }
    }

    pub fn start(&mut self, optional_name: Option<&str>) -> Result<()> {
        if self
            .active
            .as_ref()
            .and_then(|active| active.worker.as_ref())
            .is_some_and(|worker| worker.is_finished())
        {
            self.stop()?;
        }
        if self.active.is_some() {
            bail!("audio recording is already active");
        }
        let configured = self.config.effective_tracks();
        let armed = configured
            .iter()
            .filter(|track| track.armed)
            .cloned()
            .collect::<Vec<_>>();
        if armed.is_empty() {
            bail!("no recording tracks are armed");
        }
        if armed.len() > MAX_CAPTURE_TRACKS {
            bail!("capture supports at most {MAX_CAPTURE_TRACKS} armed tracks");
        }
        let missing = armed
            .iter()
            .filter(|track| {
                track.preferred_source.is_empty()
                    || !self
                        .available_sources
                        .iter()
                        .any(|source| source == &track.preferred_source)
            })
            .map(|track| track.label.as_str())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            bail!(
                "armed source missing: {}; assign it or disarm that track",
                missing.join(", ")
            );
        }
        fs::create_dir_all(&self.config.directory)?;
        let recovered = recover_interrupted(&self.config.directory)?;
        if available_bytes(&self.config.directory)? < MIN_FREE_BYTES {
            bail!("less than 64 MiB free in recording directory");
        }
        let stem = recording_stem(optional_name);
        let paths = unique_session_paths(&self.config.directory, &stem)?;
        let ring = Arc::new(InterleavedRing::new(armed.len(), self.config.ring_frames)?);
        let running = Arc::new(AtomicBool::new(true));
        let capture_enabled = Arc::new(AtomicBool::new(false));
        let publish = Arc::new(AtomicBool::new(false));
        let fault = Arc::new(AtomicU32::new(FAULT_NONE));
        let xruns = Arc::new(AtomicU64::new(0));
        let callback_violations = Arc::new(AtomicU64::new(0));
        let accepted_frames = Arc::new(AtomicU64::new(0));
        let peaks: Arc<[AtomicU32]> = (0..armed.len())
            .map(|_| AtomicU32::new(0.0f32.to_bits()))
            .collect::<Vec<_>>()
            .into();

        let mut jack = JackClient::open(&self.config.client_name)?;
        let sample_rate = jack.sample_rate();
        if !(8_000..=384_000).contains(&sample_rate) {
            bail!("JACK reported invalid sample rate {sample_rate}");
        }
        let mut ports = Vec::with_capacity(armed.len());
        let mut port_ids = Vec::with_capacity(armed.len());
        for index in 0..armed.len() {
            let port =
                jack.register_audio_port(&format!("track_{:02}", index + 1), PortDirection::Input)?;
            port_ids.push(jack.port_id(port)?);
            ports.push(port);
        }
        let mut callback_data = Box::new(CallbackData {
            ring: Arc::clone(&ring),
            running: Arc::clone(&running),
            capture_enabled: Arc::clone(&capture_enabled),
            fault: Arc::clone(&fault),
            xruns: Arc::clone(&xruns),
            callback_violations: Arc::clone(&callback_violations),
            accepted_frames: Arc::clone(&accepted_frames),
            ports: ports.into_boxed_slice(),
            port_ids: port_ids.into_boxed_slice(),
            buffers: (0..armed.len())
                .map(|_| UnsafeCell::new(std::ptr::null()))
                .collect(),
            peaks: Arc::clone(&peaks),
            maximum_callback_frames: self.config.maximum_callback_frames,
            port_get_buffer: jack.port_get_buffer(),
        });
        let callback_pointer = ((&mut *callback_data) as *mut CallbackData).cast();
        // SAFETY: callback data stays boxed until after JACK deactivation.
        unsafe {
            jack.set_process_callback(process_callback, callback_pointer)?;
            jack.set_shutdown_callback(shutdown_callback, callback_pointer);
            jack.set_xrun_callback(xrun_callback, callback_pointer)?;
            jack.set_port_connect_callback(port_connect_callback, callback_pointer)?;
        }

        let initial_tracks = status_tracks(&configured, &self.available_sources);
        if let Ok(mut status) = self.status.lock() {
            status.started = Instant::now();
            status.public = RecorderStatus {
                recording: true,
                sample_rate,
                active_tracks: armed.len(),
                path: Some(paths.final_path.clone()),
                tracks: initial_tracks,
                error: (!recovered.is_empty()).then(|| {
                    format!(
                        "recovered/reported {} interrupted recording(s)",
                        recovered.len()
                    )
                }),
                ..RecorderStatus::default()
            };
        }

        let worker_status = Arc::clone(&self.status);
        let worker_ring = Arc::clone(&ring);
        let worker_running = Arc::clone(&running);
        let worker_capture = Arc::clone(&capture_enabled);
        let worker_publish = Arc::clone(&publish);
        let worker_fault = Arc::clone(&fault);
        let worker_xruns = Arc::clone(&xruns);
        let worker_callback_violations = Arc::clone(&callback_violations);
        let worker_paths = paths.clone();
        let worker_tracks = armed.clone();
        let maximum_callback_frames = self.config.maximum_callback_frames;
        let worker = thread::Builder::new()
            .name("shr-multistem-writer".into())
            .spawn(move || {
                let result = write_session(
                    &worker_paths,
                    sample_rate,
                    &worker_tracks,
                    &worker_ring,
                    &worker_running,
                    &worker_publish,
                    &worker_fault,
                    &worker_xruns,
                    &worker_callback_violations,
                    &worker_status,
                    maximum_callback_frames,
                    WriterBehavior::default(),
                );
                worker_capture.store(false, Ordering::Release);
                worker_running.store(false, Ordering::Release);
                if let Err(error) = result {
                    worker_fault
                        .compare_exchange(
                            FAULT_NONE,
                            FAULT_WRITER,
                            Ordering::AcqRel,
                            Ordering::Relaxed,
                        )
                        .ok();
                    if let Ok(mut status) = worker_status.lock() {
                        status.public.error = Some(error.to_string());
                        status.public.incomplete = true;
                        status.public.recording = false;
                        status.public.path = Some(worker_paths.temporary.clone());
                    }
                }
            })
            .context("start multistem writer thread")?;

        if let Err(error) = activate_and_connect(&mut jack, &armed, &callback_data.ports) {
            capture_enabled.store(false, Ordering::Release);
            jack.deactivate();
            running.store(false, Ordering::Release);
            let _ = worker.join();
            if let Ok(mut status) = self.status.lock() {
                status.public.recording = false;
                status.public.error = Some(error.to_string());
            }
            return Err(error);
        }
        publish.store(true, Ordering::Release);
        capture_enabled.store(true, Ordering::Release);
        if worker.is_finished() {
            capture_enabled.store(false, Ordering::Release);
            jack.deactivate();
            running.store(false, Ordering::Release);
            let _ = worker.join();
            let error = self
                .status()
                .error
                .unwrap_or_else(|| "multistem writer stopped during startup".into());
            bail!(error);
        }
        self.active = Some(Active {
            jack,
            running,
            capture_enabled,
            fault,
            xruns,
            callback_violations,
            accepted_frames,
            ring,
            peaks,
            worker: Some(worker),
            callback_data,
        });
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        // The last completed process callback is the common stop boundary for
        // every channel. Deactivation waits for any callback already in flight.
        active.capture_enabled.store(false, Ordering::Release);
        active.jack.deactivate();
        active.running.store(false, Ordering::Release);
        active
            .worker
            .take()
            .map(|worker| worker.join())
            .transpose()
            .map_err(|_| anyhow!("multistem writer thread panicked"))?;
        let fault = active.fault.load(Ordering::Acquire);
        if let Ok(mut status) = self.status.lock() {
            status.public.recording = false;
            status.public.dropped_frames = active.ring.dropped.load(Ordering::Relaxed);
            status.public.overflow_events = active.ring.overflows.load(Ordering::Relaxed);
            status.public.writer_high_water_frames = active.ring.high_water.load(Ordering::Relaxed);
            status.public.callback_violations = active.callback_violations.load(Ordering::Relaxed);
            status.public.xruns = active.xruns.load(Ordering::Relaxed);
            status.public.total_frames = active.accepted_frames.load(Ordering::Relaxed);
        }
        drop(active.callback_data);
        if fault != FAULT_NONE {
            bail!(fault_message(fault));
        }
        if let Some(error) = self.status().error {
            bail!(error);
        }
        Ok(())
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn idle_status(config: &AudioCaptureConfig, sources: &[String]) -> RecorderStatus {
    let tracks = config.effective_tracks();
    RecorderStatus {
        active_tracks: tracks.iter().filter(|track| track.armed).count(),
        tracks: status_tracks(&tracks, sources),
        ..RecorderStatus::default()
    }
}

fn status_tracks(tracks: &[CaptureTrackConfig], sources: &[String]) -> Vec<RecorderTrackStatus> {
    tracks
        .iter()
        .map(|track| RecorderTrackStatus {
            label: track.label.clone(),
            armed: track.armed,
            preferred_source: track.preferred_source.clone(),
            resolved: !track.preferred_source.is_empty()
                && sources
                    .iter()
                    .any(|source| source == &track.preferred_source),
            peak_dbfs: None,
        })
        .collect()
}

#[derive(Clone)]
struct SessionPaths {
    temporary: PathBuf,
    final_path: PathBuf,
    incomplete: PathBuf,
}

#[derive(Clone, Copy, Default)]
struct WriterBehavior {
    delay: Duration,
    fail_after_frames: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SessionManifest {
    #[serde(alias = "version")]
    format_version: u32,
    session_name: String,
    sample_rate: u32,
    total_frames: u64,
    duration_seconds: f64,
    completeness: String,
    finalization: String,
    dropped_frames: u64,
    overflow_events: u64,
    callback_violations: u64,
    xruns: u64,
    writer_high_water_frames: usize,
    recovery: Vec<String>,
    tracks: Vec<ManifestTrack>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ManifestTrack {
    id: String,
    label: String,
    group: Option<String>,
    role: String,
    preferred_source: Option<String>,
    actual_jack_port: String,
    wav_file: String,
    frames: u64,
    finalized: bool,
}

fn manifest_for(
    paths: &SessionPaths,
    tracks: &[CaptureTrackConfig],
    sample_rate: u32,
) -> SessionManifest {
    let session_name = paths
        .final_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("recording")
        .to_owned();
    SessionManifest {
        format_version: MANIFEST_VERSION,
        session_name,
        sample_rate,
        total_frames: 0,
        duration_seconds: 0.0,
        completeness: "recording".into(),
        finalization: "pending".into(),
        dropped_frames: 0,
        overflow_events: 0,
        callback_violations: 0,
        xruns: 0,
        writer_high_water_frames: 0,
        recovery: Vec::new(),
        tracks: tracks
            .iter()
            .enumerate()
            .map(|(index, track)| ManifestTrack {
                id: track.id.clone(),
                label: track.label.clone(),
                group: (!track.group.is_empty()).then(|| track.group.clone()),
                role: match track.role {
                    CaptureTrackRole::Mono => "mono",
                    CaptureTrackRole::StereoLeft => "stereo-left",
                    CaptureTrackRole::StereoRight => "stereo-right",
                }
                .into(),
                preferred_source: (!track.preferred_source.is_empty())
                    .then(|| track.preferred_source.clone()),
                actual_jack_port: track.preferred_source.clone(),
                wav_file: track_filename(index, track),
                frames: 0,
                finalized: false,
            })
            .collect(),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_session(
    paths: &SessionPaths,
    sample_rate: u32,
    tracks: &[CaptureTrackConfig],
    ring: &InterleavedRing,
    running: &AtomicBool,
    publish: &AtomicBool,
    fault: &AtomicU32,
    xruns: &AtomicU64,
    callback_violations: &AtomicU64,
    status: &Mutex<SharedStatus>,
    maximum_callback_frames: usize,
    behavior: WriterBehavior,
) -> Result<()> {
    fs::create_dir(&paths.temporary)
        .with_context(|| format!("create take directory {}", paths.temporary.display()))?;
    let mut manifest = manifest_for(paths, tracks, sample_rate);
    write_manifest(&paths.temporary, &manifest)?;
    let mut files = Vec::with_capacity(tracks.len());
    for track in &manifest.tracks {
        let part = paths.temporary.join(format!("{}.part", track.wav_file));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .with_context(|| format!("create {}", part.display()))?;
        let mut file = BufWriter::with_capacity(64 * 1024, file);
        write_mono_wav_header(&mut file, sample_rate, 0)?;
        files.push(file);
    }
    let scratch_frames = maximum_callback_frames.max(16);
    let mut scratch = vec![0.0f32; scratch_frames * tracks.len()];
    let mut encoded = (0..tracks.len())
        .map(|_| Vec::with_capacity(scratch_frames * 3))
        .collect::<Vec<_>>();
    let mut frames = 0u64;
    while running.load(Ordering::Acquire) || !ring.is_empty() {
        let count = ring.pop_interleaved(&mut scratch);
        if count == 0 {
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        if let Some(limit) = behavior.fail_after_frames {
            if frames >= limit {
                bail!("simulated recording storage failure");
            }
        }
        if frames.saturating_add(count as u64) > MONO_WAV_MAX_FRAMES {
            fault
                .compare_exchange(
                    FAULT_NONE,
                    FAULT_WRITER,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .ok();
            running.store(false, Ordering::Release);
            bail!("mono WAV reached the RIFF 4 GiB limit");
        }
        for channel in 0..tracks.len() {
            let bytes = &mut encoded[channel];
            bytes.clear();
            for frame in 0..count {
                append_i24(bytes, scratch[frame * tracks.len() + channel]);
            }
            files[channel].write_all(bytes)?;
        }
        frames += count as u64;
        if !behavior.delay.is_zero() {
            thread::sleep(behavior.delay);
        }
        if frames % STATUS_UPDATE_FRAMES < count as u64 {
            if let Ok(mut shared) = status.lock() {
                shared.public.bytes = 44 * tracks.len() as u64 + frames * tracks.len() as u64 * 3;
                shared.public.total_frames = frames;
                shared.public.dropped_frames = ring.dropped.load(Ordering::Relaxed);
                shared.public.overflow_events = ring.overflows.load(Ordering::Relaxed);
                shared.public.writer_high_water_frames = ring.high_water.load(Ordering::Relaxed);
                shared.public.callback_violations = callback_violations.load(Ordering::Relaxed);
                shared.public.xruns = xruns.load(Ordering::Relaxed);
            }
        }
    }
    if !publish.load(Ordering::Acquire) {
        drop(files);
        fs::remove_dir_all(&paths.temporary)?;
        if let Ok(mut shared) = status.lock() {
            shared.public.recording = false;
            shared.public.bytes = 0;
            shared.public.path = None;
        }
        return Ok(());
    }
    for file in &mut files {
        finalize_mono_wav(file, sample_rate, frames)?;
        file.flush()?;
        file.get_ref().sync_all()?;
    }
    drop(files);
    for track in &mut manifest.tracks {
        let part = paths.temporary.join(format!("{}.part", track.wav_file));
        let final_path = paths.temporary.join(&track.wav_file);
        crate::fsutil::rename_noreplace(&part, &final_path)?;
        track.frames = frames;
        track.finalized = true;
    }
    manifest.total_frames = frames;
    manifest.duration_seconds = frames as f64 / f64::from(sample_rate);
    manifest.dropped_frames = ring.dropped.load(Ordering::Relaxed);
    manifest.overflow_events = ring.overflows.load(Ordering::Relaxed);
    manifest.callback_violations = callback_violations.load(Ordering::Relaxed);
    manifest.xruns = xruns.load(Ordering::Relaxed);
    manifest.writer_high_water_frames = ring.high_water.load(Ordering::Relaxed);
    let complete = frames > 0
        && fault.load(Ordering::Acquire) == FAULT_NONE
        && manifest.dropped_frames == 0
        && manifest.callback_violations == 0
        && manifest.xruns == 0
        && manifest.tracks.iter().all(|track| track.frames == frames);
    let destination = if complete {
        manifest.completeness = "complete".into();
        manifest.finalization = "finalized".into();
        &paths.final_path
    } else {
        manifest.completeness = "incomplete".into();
        manifest.finalization = "finalized-with-error".into();
        manifest.recovery.push(if frames == 0 {
            "Recording contains no captured frames".into()
        } else {
            fault_message(fault.load(Ordering::Acquire)).into()
        });
        &paths.incomplete
    };
    write_manifest(&paths.temporary, &manifest)?;
    fs::File::open(&paths.temporary)?.sync_all()?;
    crate::fsutil::rename_noreplace(&paths.temporary, destination)?;
    if let Ok(mut shared) = status.lock() {
        shared.public.recording = false;
        shared.public.incomplete = !complete;
        shared.public.bytes = 44 * tracks.len() as u64 + frames * tracks.len() as u64 * 3;
        shared.public.total_frames = frames;
        shared.public.dropped_frames = manifest.dropped_frames;
        shared.public.overflow_events = manifest.overflow_events;
        shared.public.callback_violations = manifest.callback_violations;
        shared.public.xruns = manifest.xruns;
        shared.public.writer_high_water_frames = manifest.writer_high_water_frames;
        shared.public.path = Some(destination.clone());
        shared.public.error = if complete {
            None
        } else if frames == 0 {
            Some("recording contains no captured frames".into())
        } else {
            Some(fault_message(fault.load(Ordering::Acquire)).into())
        };
    }
    Ok(())
}

fn write_manifest(directory: &Path, manifest: &SessionManifest) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    crate::fsutil::atomic_write(&directory.join("session.json"), &bytes)
        .context("write recording session manifest")
}

fn append_i24(output: &mut Vec<u8>, value: f32) {
    let sample = (value.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
    output.extend_from_slice(&sample.to_le_bytes()[..3]);
}

fn write_final_mix(
    temporary: &Path,
    final_path: &Path,
    shared: &FinalMixShared,
    status: &Mutex<FinalMixRecorderStatus>,
    behavior: WriterBehavior,
) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
    let mut file = BufWriter::with_capacity(64 * 1024, file);
    finalize_legacy_stereo_wav(&mut file, shared.sample_rate, 0)?;
    let scratch_frames = shared.maximum_callback_frames.max(16);
    let mut scratch = vec![0.0f32; scratch_frames * 2];
    let mut encoded = Vec::with_capacity(scratch_frames * 6);
    let mut frames = 0u64;
    while shared.writer_running.load(Ordering::Acquire) || !shared.ring.is_empty() {
        let count = shared.ring.pop_interleaved(&mut scratch);
        if count == 0 {
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        if let Some(limit) = behavior.fail_after_frames {
            if frames >= limit {
                bail!("simulated final-mix storage failure");
            }
        }
        if frames.saturating_add(count as u64) > STEREO_WAV_MAX_FRAMES {
            shared
                .fault
                .compare_exchange(
                    FAULT_NONE,
                    FAULT_WRITER,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .ok();
            shared.mode.store(FINAL_CAPTURE_IDLE, Ordering::Release);
            shared.writer_running.store(false, Ordering::Release);
            bail!("stereo WAV reached the RIFF 4 GiB limit");
        }
        encoded.clear();
        for sample in scratch.iter().take(count * 2) {
            append_i24(&mut encoded, *sample);
        }
        file.write_all(&encoded)?;
        frames += count as u64;
        shared.written_frames.store(frames, Ordering::Release);
        if !behavior.delay.is_zero() {
            thread::sleep(behavior.delay);
        }
        if frames % STATUS_UPDATE_FRAMES < count as u64 {
            if let Ok(mut public) = status.lock() {
                public.bytes = 44 + frames * 6;
                public.total_frames = frames;
                public.dropped_frames = shared.ring.dropped.load(Ordering::Relaxed);
                public.overflow_events = shared.ring.overflows.load(Ordering::Relaxed);
                public.writer_high_water_frames = shared.ring.high_water.load(Ordering::Relaxed);
            }
        }
    }
    if frames == 0 {
        drop(file);
        let _ = fs::remove_file(temporary);
        if let Ok(mut public) = status.lock() {
            public.recording = false;
            public.bytes = 0;
            public.total_frames = 0;
            public.path = None;
            public.error = Some("final-mix recording contains no frames".into());
        }
        bail!("final-mix recording contains no frames");
    }
    finalize_legacy_stereo_wav(&mut file, shared.sample_rate, frames)?;
    file.flush()?;
    file.get_ref().sync_all()?;
    drop(file);
    let fault = shared.fault.load(Ordering::Acquire);
    if fault == FAULT_NONE
        && shared.ring.dropped.load(Ordering::Acquire) == 0
        && shared.ring.overflows.load(Ordering::Acquire) == 0
    {
        crate::fsutil::rename_noreplace(temporary, final_path)?;
        if let Some(parent) = final_path.parent() {
            File::open(parent)?.sync_all()?;
        }
        if let Ok(mut public) = status.lock() {
            public.recording = false;
            public.bytes = 44 + frames * 6;
            public.total_frames = frames;
            public.path = Some(final_path.to_path_buf());
            public.error = None;
        }
        Ok(())
    } else {
        if let Ok(mut public) = status.lock() {
            public.recording = false;
            public.bytes = 44 + frames * 6;
            public.total_frames = frames;
            public.path = Some(temporary.to_path_buf());
            public.error = Some(fault_message(fault).into());
        }
        bail!(fault_message(fault));
    }
}

fn write_mono_wav_header(file: &mut impl Write, rate: u32, data: u32) -> Result<()> {
    let riff_size = 36u32.checked_add(data).context("WAV size overflow")?;
    let byte_rate = rate.checked_mul(3).context("WAV sample rate overflow")?;
    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&3u16.to_le_bytes())?;
    file.write_all(&24u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data.to_le_bytes())?;
    Ok(())
}

fn finalize_mono_wav(file: &mut (impl Write + Seek), rate: u32, frames: u64) -> Result<()> {
    let data = frames
        .checked_mul(3)
        .filter(|bytes| *bytes <= WAV_MAX_DATA_BYTES)
        .context("mono WAV exceeded 4 GiB PCM limit")? as u32;
    file.seek(SeekFrom::Start(0))?;
    write_mono_wav_header(file, rate, data)
}

fn track_filename(index: usize, track: &CaptureTrackConfig) -> String {
    format!(
        "{:02}-{}.wav",
        index + 1,
        crate::sequencer::safe_name(&track.label)
    )
}

fn activate_and_connect(
    jack: &mut JackClient,
    tracks: &[CaptureTrackConfig],
    ports: &[*mut JackPort],
) -> Result<()> {
    jack.activate().context("activate JACK recorder")?;
    for (track, port) in tracks.iter().zip(ports) {
        if let Err(error) = jack.connect_external_to_port(&track.preferred_source, *port) {
            jack.deactivate();
            return Err(error).with_context(|| {
                format!(
                    "connect required JACK source {} for {}",
                    track.preferred_source, track.label
                )
            });
        }
    }
    Ok(())
}

unsafe extern "C" fn process_callback(frames: c_uint, argument: *mut c_void) -> c_int {
    // SAFETY: the owner keeps callback data pinned until deactivation returns.
    let data = unsafe { &*argument.cast::<CallbackData>() };
    if !data.capture_enabled.load(Ordering::Acquire) {
        return 0;
    }
    let frames = frames as usize;
    if frames == 0 || frames > data.maximum_callback_frames {
        data.callback_violations.fetch_add(1, Ordering::Relaxed);
        set_fault(data, FAULT_CALLBACK_SIZE);
        return 0;
    }
    for (index, port) in data.ports.iter().enumerate() {
        unsafe {
            *data.buffers[index].get() =
                (data.port_get_buffer)(*port, frames as c_uint).cast::<f32>();
        }
    }
    if data
        .buffers
        .iter()
        .any(|buffer| unsafe { (*buffer.get()).is_null() })
    {
        data.ring
            .dropped
            .fetch_add(frames as u64, Ordering::Relaxed);
        set_fault(data, FAULT_NULL_BUFFER);
        return 0;
    }
    for (channel, buffer) in data.buffers.iter().enumerate() {
        let buffer = unsafe { *buffer.get() };
        let mut peak = 0.0f32;
        for frame in 0..frames {
            peak = peak.max(unsafe { *buffer.add(frame) }.abs());
        }
        data.peaks[channel].fetch_max(peak.to_bits(), Ordering::Relaxed);
    }
    if !unsafe { data.ring.push_cells(&data.buffers, frames) } {
        set_fault(data, FAULT_OVERFLOW);
        return 0;
    }
    data.accepted_frames
        .fetch_add(frames as u64, Ordering::Relaxed);
    0
}

unsafe extern "C" fn shutdown_callback(argument: *mut c_void) {
    let data = unsafe { &*argument.cast::<CallbackData>() };
    if data.capture_enabled.load(Ordering::Acquire) {
        set_fault(data, FAULT_JACK_SHUTDOWN);
    }
}

unsafe extern "C" fn xrun_callback(argument: *mut c_void) -> c_int {
    let data = unsafe { &*argument.cast::<CallbackData>() };
    if data.capture_enabled.load(Ordering::Acquire) {
        data.xruns.fetch_add(1, Ordering::Relaxed);
        set_fault(data, FAULT_XRUN);
    }
    0
}

unsafe extern "C" fn port_connect_callback(
    first: c_uint,
    second: c_uint,
    connected: c_int,
    argument: *mut c_void,
) {
    if connected != 0 {
        return;
    }
    let data = unsafe { &*argument.cast::<CallbackData>() };
    if data.capture_enabled.load(Ordering::Acquire)
        && data
            .port_ids
            .iter()
            .any(|port| *port == first || *port == second)
    {
        set_fault(data, FAULT_SOURCE_LOST);
    }
}

fn set_fault(data: &CallbackData, code: u32) {
    data.fault
        .compare_exchange(FAULT_NONE, code, Ordering::AcqRel, Ordering::Relaxed)
        .ok();
    data.capture_enabled.store(false, Ordering::Release);
    data.running.store(false, Ordering::Release);
}

fn fault_message(code: u32) -> &'static str {
    match code {
        FAULT_NONE => "no recorder fault",
        FAULT_OVERFLOW => "capture ring overflow; take is incomplete",
        FAULT_CALLBACK_SIZE => "JACK callback exceeded configured recorder capacity",
        FAULT_JACK_SHUTDOWN => "JACK shut down during recording",
        FAULT_SOURCE_LOST => "a required recording source disappeared",
        FAULT_XRUN => "JACK xrun during recording; take is incomplete",
        FAULT_NULL_BUFFER => "JACK returned an invalid recording buffer",
        FAULT_WRITER => "recording writer or finalization failed",
        _ => "unknown recorder fault",
    }
}

#[derive(Clone, Debug)]
pub struct StressReport {
    pub session: PathBuf,
    pub channels: usize,
    pub sample_rate: u32,
    pub callback_frames: usize,
    pub total_frames: u64,
    pub elapsed: Duration,
    pub throughput_bytes_per_second: f64,
    pub writer_high_water_frames: usize,
    pub dropped_frames: u64,
    pub overflow_events: u64,
    pub channel_identity_verified: bool,
}

#[derive(Clone, Debug)]
pub struct FinalMixStressReport {
    pub wav: PathBuf,
    pub sample_rate: u32,
    pub callback_frames: usize,
    pub total_frames: u64,
    pub elapsed: Duration,
    pub callback_mean_nanoseconds: u64,
    pub callback_p95_nanoseconds: u64,
    pub callback_p99_nanoseconds: u64,
    pub callback_maximum_nanoseconds: u64,
    pub maximum_gain_reduction_db: f32,
    pub writer_high_water_frames: usize,
    pub dropped_frames: u64,
    pub overflow_events: u64,
    pub output_file_equal: bool,
}

/// JACK-free, non-audible soak of the production three-source faders, final
/// limiter/meter, callback recorder handoff, stereo writer, and publication.
pub fn run_final_mix_stress(
    destination: &Path,
    duration: Duration,
    sample_rate: u32,
    callback_frames: usize,
) -> Result<FinalMixStressReport> {
    if destination.as_os_str().is_empty() || destination == Path::new("/") {
        bail!("stress destination must be an explicit non-root directory");
    }
    if duration.is_zero() || duration > Duration::from_secs(24 * 60 * 60) {
        bail!("stress duration must be greater than zero and at most 24 hours");
    }
    if !(44_100..=48_000).contains(&sample_rate) || !matches!(sample_rate, 44_100 | 48_000) {
        bail!("final-mix stress rate must be 44100 or 48000 Hz");
    }
    if !(16..=4096).contains(&callback_frames) {
        bail!("final-mix stress callback must be 16..=4096 frames");
    }
    let target_frames = (duration.as_secs_f64() * f64::from(sample_rate)).round() as u64;
    if target_frames == 0 {
        bail!("stress duration is shorter than one sample frame");
    }
    fs::create_dir_all(destination)?;
    let controls = Arc::new(crate::final_bus::BusControls::default());
    let meters = Arc::new(crate::final_bus::FinalBusMeters::default());
    let mut bus = crate::final_bus::FinalBusProcessor::new(
        sample_rate,
        callback_frames,
        controls,
        Arc::clone(&meters),
    )
    .map_err(anyhow::Error::msg)?;
    let ring_frames = (sample_rate as usize * 2).max(callback_frames * 8);
    let mut recorder = FinalMixRecorder::new(
        destination.to_path_buf(),
        sample_rate,
        ring_frames,
        callback_frames,
    )?;
    let capture = recorder.capture_handle();
    recorder.start(Some("stress"))?;
    let mut sources: [Vec<StereoFrame>; 3] =
        std::array::from_fn(|_| vec![StereoFrame::SILENCE; callback_frames]);
    let mut output = vec![StereoFrame::SILENCE; callback_frames];
    let callbacks =
        target_frames.saturating_add(callback_frames as u64 - 1) / callback_frames as u64;
    let mut timings = Vec::with_capacity(callbacks as usize);
    let mut expected_hash = 0xcbf29ce484222325u64;
    let mut maximum_gain_reduction_db = 0.0f32;
    let mut produced = 0u64;
    let started = Instant::now();
    while produced < target_frames {
        let count = (target_frames - produced).min(callback_frames as u64) as usize;
        for (source, buffer) in sources.iter_mut().enumerate() {
            for (offset, frame) in buffer.iter_mut().take(count).enumerate() {
                *frame = final_stress_source(source, produced + offset as u64);
            }
        }
        let callback_started = Instant::now();
        for (index, source) in crate::final_bus::BusSource::ALL.iter().copied().enumerate() {
            bus.process_source(source, &mut sources[index][..count]);
        }
        for (frame, output_frame) in output.iter_mut().take(count).enumerate() {
            *output_frame = StereoFrame::new(
                sources[0][frame].left + sources[1][frame].left + sources[2][frame].left,
                sources[0][frame].right + sources[1][frame].right + sources[2][frame].right,
            );
        }
        bus.process_final(&mut output[..count]);
        capture.capture(&output[..count]);
        maximum_gain_reduction_db =
            maximum_gain_reduction_db.max(meters.snapshot().limiter_gain_reduction_db);
        timings.push(
            callback_started
                .elapsed()
                .as_nanos()
                .min(u128::from(u64::MAX)) as u64,
        );
        let ceiling = 10.0f32.powf(crate::final_bus::LIMITER_CEILING_DBFS / 20.0) + 1e-6;
        for frame in &output[..count] {
            if !frame.left.is_finite()
                || !frame.right.is_finite()
                || frame.left.abs() > ceiling
                || frame.right.abs() > ceiling
            {
                bail!("final bus produced an unsafe sample");
            }
            hash_i24(&mut expected_hash, frame.left);
            hash_i24(&mut expected_hash, frame.right);
        }
        produced += count as u64;
        let deadline = started + Duration::from_secs_f64(produced as f64 / f64::from(sample_rate));
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining > Duration::from_millis(1) {
                thread::sleep(remaining.min(Duration::from_millis(2)));
            } else {
                thread::yield_now();
            }
        }
    }
    recorder.request_stop();
    capture.capture(&[]);
    recorder.finish_stop()?;
    let status = recorder.status();
    let wav = status
        .path
        .context("final-mix stress did not publish a WAV")?;
    let mut file = File::open(&wav)?;
    let mut header = [0u8; 44];
    file.read_exact(&mut header)?;
    let mut actual_hash = 0xcbf29ce484222325u64;
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        for byte in &bytes[..count] {
            actual_hash ^= u64::from(*byte);
            actual_hash = actual_hash.wrapping_mul(0x100000001b3);
        }
    }
    timings.sort_unstable();
    let percentile = |percent: usize| timings[(timings.len().saturating_sub(1) * percent) / 100];
    let total_time = timings.iter().copied().sum::<u64>();
    Ok(FinalMixStressReport {
        wav,
        sample_rate,
        callback_frames,
        total_frames: produced,
        elapsed: started.elapsed(),
        callback_mean_nanoseconds: total_time / timings.len() as u64,
        callback_p95_nanoseconds: percentile(95),
        callback_p99_nanoseconds: percentile(99),
        callback_maximum_nanoseconds: timings.last().copied().unwrap_or(0),
        maximum_gain_reduction_db,
        writer_high_water_frames: status.writer_high_water_frames,
        dropped_frames: status.dropped_frames,
        overflow_events: status.overflow_events,
        output_file_equal: expected_hash == actual_hash,
    })
}

fn final_stress_source(source: usize, frame: u64) -> StereoFrame {
    if frame % 8192 == 0 {
        return StereoFrame::new(0.8, 0.72);
    }
    let (left_prime, right_prime, scale) = match source {
        0 => (257, 263, 0.20),
        1 => (509, 521, 0.16),
        _ => (997, 991, 0.12),
    };
    StereoFrame::new(
        ((frame % left_prime) as f32 / (left_prime - 1) as f32 - 0.5) * scale,
        ((frame % right_prime) as f32 / (right_prime - 1) as f32 - 0.5) * scale,
    )
}

fn hash_i24(hash: &mut u64, value: f32) {
    let sample = (value.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
    for byte in &sample.to_le_bytes()[..3] {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

/// Non-audible, JACK-free soak using the production ring, writer, manifest,
/// finalization, and publication path.
pub fn run_synthetic_stress(
    destination: &Path,
    duration: Duration,
    channels: usize,
    sample_rate: u32,
    callback_frames: usize,
) -> Result<StressReport> {
    if destination.as_os_str().is_empty() || destination == Path::new("/") {
        bail!("stress destination must be an explicit non-root directory");
    }
    if !(1..=MAX_CAPTURE_TRACKS).contains(&channels) {
        bail!("stress channels must be 1..={MAX_CAPTURE_TRACKS}");
    }
    if !(8_000..=384_000).contains(&sample_rate) {
        bail!("stress sample rate must be 8000..=384000 Hz");
    }
    if !(16..=65_536).contains(&callback_frames) {
        bail!("stress callback must be 16..=65536 frames");
    }
    if duration.is_zero() || duration > Duration::from_secs(24 * 60 * 60) {
        bail!("stress duration must be greater than zero and at most 24 hours");
    }
    let target_frames = (duration.as_secs_f64() * f64::from(sample_rate)).round() as u64;
    if target_frames == 0 {
        bail!("stress duration is shorter than one sample frame");
    }
    fs::create_dir_all(destination)?;
    if available_bytes(destination)? < MIN_FREE_BYTES {
        bail!("less than 64 MiB free in stress destination");
    }
    let paths = unique_session_paths(destination, "synthetic-multitrack")?;
    let tracks = (0..channels)
        .map(|index| CaptureTrackConfig {
            id: format!("synthetic-{}", index + 1),
            label: format!("Synthetic Input {}", index + 1),
            group: String::new(),
            role: CaptureTrackRole::Mono,
            armed: true,
            preferred_source: format!("synthetic:channel_{}", index + 1),
        })
        .collect::<Vec<_>>();
    let ring_capacity = (sample_rate as usize * 2)
        .max(callback_frames.saturating_mul(8))
        .min(4_194_304);
    let ring = Arc::new(InterleavedRing::new(channels, ring_capacity)?);
    let running = Arc::new(AtomicBool::new(true));
    let publish = Arc::new(AtomicBool::new(true));
    let fault = Arc::new(AtomicU32::new(FAULT_NONE));
    let xruns = Arc::new(AtomicU64::new(0));
    let violations = Arc::new(AtomicU64::new(0));
    let status = Arc::new(Mutex::new(SharedStatus {
        started: Instant::now(),
        public: RecorderStatus::default(),
    }));
    let worker_paths = paths.clone();
    let worker_tracks = tracks.clone();
    let worker_ring = Arc::clone(&ring);
    let worker_running = Arc::clone(&running);
    let worker_publish = Arc::clone(&publish);
    let worker_fault = Arc::clone(&fault);
    let worker_xruns = Arc::clone(&xruns);
    let worker_violations = Arc::clone(&violations);
    let worker_status = Arc::clone(&status);
    let worker = thread::Builder::new()
        .name("shr-stress-writer".into())
        .spawn(move || {
            write_session(
                &worker_paths,
                sample_rate,
                &worker_tracks,
                &worker_ring,
                &worker_running,
                &worker_publish,
                &worker_fault,
                &worker_xruns,
                &worker_violations,
                &worker_status,
                callback_frames,
                WriterBehavior::default(),
            )
        })?;
    let mut planar = vec![vec![0.0f32; callback_frames]; channels];
    let pointers = planar
        .iter()
        .map(|samples| samples.as_ptr())
        .collect::<Vec<_>>();
    let mut produced = 0u64;
    let started = Instant::now();
    while produced < target_frames {
        let count = (target_frames - produced).min(callback_frames as u64) as usize;
        for (channel, samples) in planar.iter_mut().enumerate() {
            for (offset, sample) in samples.iter_mut().take(count).enumerate() {
                *sample = synthetic_sample(channel, channels, produced + offset as u64);
            }
        }
        if !unsafe { ring.push_raw(&pointers, count) } {
            fault.store(FAULT_OVERFLOW, Ordering::Release);
            break;
        }
        produced += count as u64;
        let deadline = started + Duration::from_secs_f64(produced as f64 / f64::from(sample_rate));
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(now);
            if remaining > Duration::from_millis(1) {
                thread::sleep(remaining.min(Duration::from_millis(2)));
            } else {
                thread::yield_now();
            }
        }
    }
    running.store(false, Ordering::Release);
    let writer_result = worker
        .join()
        .map_err(|_| anyhow!("synthetic multitrack writer panicked"))?;
    writer_result?;
    let elapsed = started.elapsed();
    let manifest: SessionManifest =
        serde_json::from_slice(&fs::read(paths.final_path.join("session.json"))?)?;
    if manifest.completeness != "complete"
        || manifest.total_frames != target_frames
        || manifest
            .tracks
            .iter()
            .any(|track| track.frames != target_frames)
    {
        bail!("synthetic take failed frame-count or completion verification");
    }
    let mut identity_verified = true;
    for (channel, track) in manifest.tracks.iter().enumerate() {
        let wav = paths.final_path.join(&track.wav_file);
        let expected_len = target_frames
            .checked_mul(3)
            .and_then(|bytes| 44u64.checked_add(bytes))
            .context("synthetic WAV length overflow")?;
        let metadata = fs::symlink_metadata(&wav)?;
        if !metadata.file_type().is_file() || metadata.len() != expected_len {
            identity_verified = false;
            break;
        }
        for frame in [0, 1, target_frames / 2, target_frames - 1] {
            let actual = read_mono_i24_at(&wav, frame)?;
            let expected = synthetic_sample(channel, channels, frame);
            if (actual - expected).abs() > 0.000_001 {
                identity_verified = false;
                break;
            }
        }
    }
    if !identity_verified {
        bail!("synthetic channel identity verification failed");
    }
    let bytes = 44 * channels as u64 + target_frames * channels as u64 * 3;
    Ok(StressReport {
        session: paths.final_path,
        channels,
        sample_rate,
        callback_frames,
        total_frames: target_frames,
        elapsed,
        throughput_bytes_per_second: bytes as f64 / elapsed.as_secs_f64().max(f64::EPSILON),
        writer_high_water_frames: ring.high_water.load(Ordering::Relaxed),
        dropped_frames: ring.dropped.load(Ordering::Relaxed),
        overflow_events: ring.overflows.load(Ordering::Relaxed),
        channel_identity_verified: identity_verified,
    })
}

fn synthetic_sample(channel: usize, channels: usize, frame: u64) -> f32 {
    if frame == 0 {
        return (channel + 1) as f32 / (channels + 1) as f32 * 0.8;
    }
    let channel_offset = (channel + 1) as f32 / (channels + 1) as f32 * 0.25;
    let ramp = (frame % 997) as f32 / 996.0 * 0.5;
    channel_offset + ramp - 0.375
}

fn read_mono_i24_at(path: &Path, frame: u64) -> Result<f32> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.seek(SeekFrom::Start(44 + frame.saturating_mul(3)))?;
    let mut bytes = [0u8; 3];
    file.read_exact(&mut bytes)?;
    let raw = i32::from(bytes[0]) | (i32::from(bytes[1]) << 8) | (i32::from(bytes[2]) << 16);
    let signed = if raw & 0x80_0000 != 0 {
        raw | !0xff_ffff
    } else {
        raw
    };
    Ok(signed as f32 / 8_388_607.0)
}

pub fn recover_interrupted(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut found = recover_legacy_stereo_parts(directory)?;
    let Ok(entries) = fs::read_dir(directory) else {
        return Ok(found);
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".take.part") {
            continue;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            found.push(path);
            continue;
        }
        match recover_session_directory(&path) {
            Ok(recovered) => found.push(recovered),
            Err(_) => found.push(path),
        }
    }
    Ok(found)
}

fn recover_session_directory(path: &Path) -> Result<PathBuf> {
    let manifest_path = path.join("session.json");
    let metadata = fs::symlink_metadata(&manifest_path)?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 * 1024 {
        bail!("invalid interrupted session manifest");
    }
    let mut manifest: SessionManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.format_version != MANIFEST_VERSION
        || !(8_000..=384_000).contains(&manifest.sample_rate)
        || manifest.tracks.is_empty()
        || manifest.tracks.len() > MAX_CAPTURE_TRACKS
    {
        bail!("unsupported interrupted session manifest");
    }
    let mut wav_names = std::collections::BTreeSet::new();
    if manifest
        .tracks
        .iter()
        .any(|track| !safe_manifest_wav_name(&track.wav_file) || !wav_names.insert(&track.wav_file))
    {
        bail!("unsafe or duplicate interrupted stem filename");
    }
    let mut frame_counts = Vec::with_capacity(manifest.tracks.len());
    for track in &manifest.tracks {
        let part = path.join(format!("{}.part", track.wav_file));
        let metadata = fs::symlink_metadata(&part)?;
        if !metadata.file_type().is_file() {
            bail!("interrupted stem is not a regular file");
        }
        let len = metadata.len();
        if len < 44 || !is_mono_wav_part(&part) {
            bail!("invalid interrupted mono WAV");
        }
        frame_counts.push(((len - 44) / 3).min(MONO_WAV_MAX_FRAMES));
    }
    let common_frames = frame_counts
        .into_iter()
        .min()
        .context("no interrupted stems")?;
    for track in &mut manifest.tracks {
        let part = path.join(format!("{}.part", track.wav_file));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&part)?;
        file.set_len(44 + common_frames * 3)?;
        finalize_mono_wav(&mut file, manifest.sample_rate, common_frames)?;
        file.sync_all()?;
        drop(file);
        crate::fsutil::rename_noreplace(&part, &path.join(&track.wav_file))?;
        track.frames = common_frames;
        track.finalized = true;
    }
    manifest.total_frames = common_frames;
    manifest.duration_seconds = common_frames as f64 / f64::from(manifest.sample_rate);
    manifest.completeness = "recovered-incomplete".into();
    manifest.finalization = "recovered".into();
    manifest.recovery.push(
        "Recovered complete common frames after interrupted finalization; review before use".into(),
    );
    write_manifest(path, &manifest)?;
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording.take.part")
        .trim_end_matches(".take.part");
    let recovered = unique_directory(
        path.parent().unwrap_or(Path::new(".")),
        stem,
        "recovered.take",
    )?;
    crate::fsutil::rename_noreplace(path, &recovered)?;
    Ok(recovered)
}

fn safe_manifest_wav_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
        && name.ends_with(".wav")
        && !name.chars().any(char::is_control)
}

fn is_mono_wav_part(path: &Path) -> bool {
    let mut header = [0u8; 44];
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header[..4] == b"RIFF"
        && &header[8..12] == b"WAVE"
        && &header[12..20] == b"fmt \x10\0\0\0"
        && &header[20..24] == b"\x01\0\x01\0"
        && &header[32..36] == b"\x03\0\x18\0"
        && &header[36..40] == b"data"
        && read_wav_rate_bytes(&header, 3).is_some()
}

fn recover_legacy_stereo_parts(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return Ok(found);
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !entry.file_name().to_string_lossy().ends_with(".wav.part") {
            continue;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            found.push(path);
            continue;
        }
        let len = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        if len >= 44 && is_legacy_stereo_part(&path) {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)?;
            let frames = ((len - 44) / 6).min(STEREO_WAV_MAX_FRAMES);
            file.set_len(44 + frames * 6)?;
            let rate = read_wav_rate(&mut file, 6).context("invalid interrupted WAV header")?;
            finalize_legacy_stereo_wav(&mut file, rate, frames)?;
            file.sync_all()?;
            drop(file);
            let stem = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("interrupted.wav.part")
                .trim_end_matches(".wav.part");
            let recovered = unique_file(directory, &format!("{stem}-recovered"), "wav")?;
            crate::fsutil::rename_noreplace(&path, &recovered)?;
            found.push(recovered);
        } else {
            found.push(path);
        }
    }
    Ok(found)
}

fn is_legacy_stereo_part(path: &Path) -> bool {
    let mut header = [0u8; 44];
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header[..4] == b"RIFF"
        && &header[8..12] == b"WAVE"
        && &header[12..20] == b"fmt \x10\0\0\0"
        && &header[20..24] == b"\x01\0\x02\0"
        && &header[32..36] == b"\x06\0\x18\0"
        && &header[36..40] == b"data"
        && read_wav_rate_bytes(&header, 6).is_some()
}

fn read_wav_rate(file: &mut File, bytes_per_frame: u32) -> Option<u32> {
    let mut bytes = [0u8; 4];
    file.seek(SeekFrom::Start(24)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    let rate = u32::from_le_bytes(bytes);
    (rate.checked_mul(bytes_per_frame).is_some() && (8_000..=384_000).contains(&rate))
        .then_some(rate)
}

fn read_wav_rate_bytes(header: &[u8; 44], bytes_per_frame: u32) -> Option<u32> {
    let rate = u32::from_le_bytes(header[24..28].try_into().ok()?);
    let byte_rate = u32::from_le_bytes(header[28..32].try_into().ok()?);
    ((8_000..=384_000).contains(&rate) && rate.checked_mul(bytes_per_frame) == Some(byte_rate))
        .then_some(rate)
}

fn finalize_legacy_stereo_wav(
    file: &mut (impl Write + Seek),
    rate: u32,
    frames: u64,
) -> Result<()> {
    let data = frames
        .checked_mul(6)
        .filter(|bytes| *bytes <= WAV_MAX_DATA_BYTES)
        .context("WAV exceeded 4 GiB PCM limit")? as u32;
    file.seek(SeekFrom::Start(0))?;
    let riff_size = 36u32.checked_add(data).context("WAV size overflow")?;
    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(
        &rate
            .checked_mul(6)
            .context("WAV rate overflow")?
            .to_le_bytes(),
    )?;
    file.write_all(&6u16.to_le_bytes())?;
    file.write_all(&24u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data.to_le_bytes())?;
    Ok(())
}

fn available_bytes(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).context("recording disk space");
    }
    let value = unsafe { value.assume_init() };
    Ok(value.f_bavail.saturating_mul(value.f_frsize))
}

fn recording_stem(name: Option<&str>) -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let suffix = name
        .map(crate::sequencer::safe_name)
        .filter(|name| name != "untitled");
    suffix.map_or_else(
        || format!("recording-{seconds}"),
        |name| format!("recording-{seconds}-{name}"),
    )
}

fn final_mix_stem(name: Option<&str>) -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let suffix = name
        .map(crate::sequencer::safe_name)
        .filter(|name| name != "untitled");
    suffix.map_or_else(
        || format!("final-mix-{seconds}"),
        |name| format!("final-mix-{seconds}-{name}"),
    )
}

fn unique_session_paths(directory: &Path, stem: &str) -> Result<SessionPaths> {
    for suffix in 0..10_000 {
        let stem = if suffix == 0 {
            stem.to_owned()
        } else {
            format!("{stem}-{suffix}")
        };
        let final_path = directory.join(format!("{stem}.take"));
        let temporary = directory.join(format!("{stem}.take.part"));
        let incomplete = directory.join(format!("{stem}.incomplete.take"));
        if !final_path.exists() && !temporary.exists() && !incomplete.exists() {
            return Ok(SessionPaths {
                temporary,
                final_path,
                incomplete,
            });
        }
    }
    bail!("could not choose a unique recording session name")
}

fn unique_directory(directory: &Path, stem: &str, suffix: &str) -> Result<PathBuf> {
    for number in 0..10_000 {
        let name = if number == 0 {
            format!("{stem}-{suffix}")
        } else {
            format!("{stem}-{number}-{suffix}")
        };
        let path = directory.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    bail!("could not choose a unique recovered recording directory")
}

fn unique_file(directory: &Path, stem: &str, extension: &str) -> Result<PathBuf> {
    for number in 0..10_000 {
        let name = if number == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{number}.{extension}")
        };
        let path = directory.join(name);
        if !path.exists() && !path.with_extension(format!("{extension}.part")).exists() {
            return Ok(path);
        }
    }
    bail!("could not choose a unique recording filename")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::allocation_test::assert_no_allocations;

    fn tracks(count: usize) -> Vec<CaptureTrackConfig> {
        (0..count)
            .map(|index| CaptureTrackConfig {
                id: format!("input-{}", index + 1),
                label: format!("Input {}", index + 1),
                group: String::new(),
                role: CaptureTrackRole::Mono,
                armed: true,
                preferred_source: format!("test:capture_{}", index + 1),
            })
            .collect()
    }

    fn test_paths(base: &Path, name: &str) -> SessionPaths {
        SessionPaths {
            temporary: base.join(format!("{name}.take.part")),
            final_path: base.join(format!("{name}.take")),
            incomplete: base.join(format!("{name}.incomplete.take")),
        }
    }

    fn feed(ring: &InterleavedRing, channels: usize, frames: usize, start: usize) {
        let data = (0..channels)
            .map(|channel| {
                (0..frames)
                    .map(|frame| ((channel + 1) * 1000 + start + frame) as f32 / 100_000.0)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let pointers = data
            .iter()
            .map(|channel| channel.as_ptr())
            .collect::<Vec<_>>();
        assert!(unsafe { ring.push_raw(&pointers, frames) });
    }

    fn run_session_at(
        count: usize,
        frames: usize,
        sample_rate: u32,
        callback_frames: usize,
    ) -> (PathBuf, SessionManifest) {
        let base = std::env::temp_dir().join(format!(
            "shr-multitrack-{}-{count}-{sample_rate}-{callback_frames}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let paths = test_paths(&base, "take");
        let ring = InterleavedRing::new(count, frames + 8).unwrap();
        feed(&ring, count, frames, 0);
        let running = AtomicBool::new(false);
        let publish = AtomicBool::new(true);
        let fault = AtomicU32::new(FAULT_NONE);
        let xruns = AtomicU64::new(0);
        let violations = AtomicU64::new(0);
        let status = Mutex::new(SharedStatus {
            started: Instant::now(),
            public: RecorderStatus::default(),
        });
        write_session(
            &paths,
            sample_rate,
            &tracks(count),
            &ring,
            &running,
            &publish,
            &fault,
            &xruns,
            &violations,
            &status,
            callback_frames,
            WriterBehavior::default(),
        )
        .unwrap();
        let manifest =
            serde_json::from_slice(&fs::read(paths.final_path.join("session.json")).unwrap())
                .unwrap();
        (base, manifest)
    }

    fn run_session(count: usize, frames: usize) -> (PathBuf, SessionManifest) {
        run_session_at(count, frames, 48_000, 256)
    }

    #[test]
    fn supported_channel_counts_are_sample_aligned_and_identity_safe() {
        for count in [1, 2, 4, 8, 12, 16, 18, 23] {
            let (base, manifest) = run_session(count, 513);
            assert_eq!(manifest.completeness, "complete");
            assert_eq!(manifest.total_frames, 513);
            assert!(manifest.tracks.iter().all(|track| track.frames == 513));
            for (channel, track) in manifest.tracks.iter().enumerate() {
                let bytes = fs::read(base.join("take.take").join(&track.wav_file)).unwrap();
                assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 1);
                assert_eq!(
                    u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
                    48_000
                );
                assert_eq!(
                    u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
                    513 * 3
                );
                let raw = [bytes[44], bytes[45], bytes[46], 0];
                let sample = i32::from_le_bytes(raw) as f32 / 8_388_607.0;
                let expected = ((channel + 1) * 1000) as f32 / 100_000.0;
                assert!((sample - expected).abs() < 0.000_001);
            }
            let _ = fs::remove_dir_all(base);
        }
    }

    #[test]
    fn supported_rates_and_realistic_callback_sizes_keep_one_timeline() {
        for sample_rate in [44_100, 48_000] {
            for callback_frames in [64, 128, 256, 1024] {
                let frames = callback_frames * 2 + 17;
                let (base, manifest) = run_session_at(4, frames, sample_rate, callback_frames);
                assert_eq!(manifest.sample_rate, sample_rate);
                assert_eq!(manifest.total_frames, frames as u64);
                assert!(manifest
                    .tracks
                    .iter()
                    .all(|track| track.frames == frames as u64 && track.finalized));
                let _ = fs::remove_dir_all(base);
            }
        }
    }

    #[test]
    fn whole_callback_overflow_never_skews_channels() {
        let ring = InterleavedRing::new(4, 4).unwrap();
        feed(&ring, 4, 4, 0);
        let data = vec![vec![0.0; 2]; 4];
        let pointers = data
            .iter()
            .map(|channel| channel.as_ptr())
            .collect::<Vec<_>>();
        assert!(!unsafe { ring.push_raw(&pointers, 2) });
        assert_eq!(ring.dropped.load(Ordering::Relaxed), 2);
        assert_eq!(ring.overflows.load(Ordering::Relaxed), 1);
        let mut out = vec![0.0; 32];
        assert_eq!(ring.pop_interleaved(&mut out), 4);
    }

    #[test]
    fn slow_writer_high_water_and_failure_are_reported() {
        let base = std::env::temp_dir().join(format!("shr-slow-writer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let paths = test_paths(&base, "failure");
        let ring = InterleavedRing::new(18, 2048).unwrap();
        feed(&ring, 18, 1024, 0);
        let running = AtomicBool::new(false);
        let publish = AtomicBool::new(true);
        let fault = AtomicU32::new(FAULT_NONE);
        let status = Mutex::new(SharedStatus {
            started: Instant::now(),
            public: RecorderStatus::default(),
        });
        assert!(write_session(
            &paths,
            48_000,
            &tracks(18),
            &ring,
            &running,
            &publish,
            &fault,
            &AtomicU64::new(0),
            &AtomicU64::new(0),
            &status,
            128,
            WriterBehavior {
                delay: Duration::from_millis(1),
                fail_after_frames: Some(128),
            },
        )
        .is_err());
        assert!(paths.temporary.exists());
        assert!(ring.high_water.load(Ordering::Relaxed) >= 1024);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn faulted_take_finalizes_stems_but_publishes_only_as_incomplete() {
        let base = std::env::temp_dir().join(format!("shr-partial-final-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let paths = test_paths(&base, "faulted");
        let ring = InterleavedRing::new(4, 128).unwrap();
        feed(&ring, 4, 64, 0);
        let status = Mutex::new(SharedStatus {
            started: Instant::now(),
            public: RecorderStatus::default(),
        });
        write_session(
            &paths,
            44_100,
            &tracks(4),
            &ring,
            &AtomicBool::new(false),
            &AtomicBool::new(true),
            &AtomicU32::new(FAULT_SOURCE_LOST),
            &AtomicU64::new(0),
            &AtomicU64::new(0),
            &status,
            64,
            WriterBehavior::default(),
        )
        .unwrap();
        assert!(!paths.final_path.exists());
        assert!(paths.incomplete.exists());
        let manifest: SessionManifest =
            serde_json::from_slice(&fs::read(paths.incomplete.join("session.json")).unwrap())
                .unwrap();
        assert_eq!(manifest.sample_rate, 44_100);
        assert_eq!(manifest.completeness, "incomplete");
        assert!(manifest
            .tracks
            .iter()
            .all(|track| track.frames == 64 && track.finalized));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn zero_frame_take_is_never_published_as_complete() {
        let base = std::env::temp_dir().join(format!("shr-empty-take-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let paths = test_paths(&base, "empty");
        write_session(
            &paths,
            48_000,
            &tracks(2),
            &InterleavedRing::new(2, 64).unwrap(),
            &AtomicBool::new(false),
            &AtomicBool::new(true),
            &AtomicU32::new(FAULT_NONE),
            &AtomicU64::new(0),
            &AtomicU64::new(0),
            &Mutex::new(SharedStatus {
                started: Instant::now(),
                public: RecorderStatus::default(),
            }),
            64,
            WriterBehavior::default(),
        )
        .unwrap();
        assert!(!paths.final_path.exists());
        let manifest: SessionManifest =
            serde_json::from_slice(&fs::read(paths.incomplete.join("session.json")).unwrap())
                .unwrap();
        assert_eq!(manifest.completeness, "incomplete");
        assert!(manifest.recovery[0].contains("no captured frames"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn missing_preference_stays_missing_then_resolves_by_exact_name() {
        let config = AudioCaptureConfig {
            client_name: "test".into(),
            directory: PathBuf::from("/tmp"),
            inputs: Vec::new(),
            tracks: tracks(2),
            ring_frames: 1024,
            maximum_callback_frames: 256,
        };
        let missing = idle_status(&config, &[]);
        assert!(missing.tracks.iter().all(|track| !track.resolved));
        assert_eq!(missing.tracks[0].preferred_source, "test:capture_1");
        let returned = idle_status(
            &config,
            &["test:capture_1".into(), "nearby:capture_2".into()],
        );
        assert!(returned.tracks[0].resolved);
        assert!(!returned.tracks[1].resolved);
        assert_eq!(returned.tracks[1].preferred_source, "test:capture_2");
    }

    #[test]
    fn interrupted_multistem_recovery_uses_common_shortest_length() {
        let base = std::env::temp_dir().join(format!("shr-recover-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let paths = test_paths(&base, "lost");
        fs::create_dir(&paths.temporary).unwrap();
        let configured = tracks(2);
        let manifest = manifest_for(&paths, &configured, 48_000);
        write_manifest(&paths.temporary, &manifest).unwrap();
        for (index, track) in manifest.tracks.iter().enumerate() {
            let mut file =
                File::create(paths.temporary.join(format!("{}.part", track.wav_file))).unwrap();
            write_mono_wav_header(&mut file, 48_000, 0).unwrap();
            file.write_all(&vec![0; (10 - index) * 3]).unwrap();
        }
        let recovered = recover_interrupted(&base).unwrap();
        assert_eq!(recovered.len(), 1);
        let manifest: SessionManifest =
            serde_json::from_slice(&fs::read(recovered[0].join("session.json")).unwrap()).unwrap();
        assert_eq!(manifest.total_frames, 9);
        assert_eq!(manifest.completeness, "recovered-incomplete");
        assert!(manifest.tracks.iter().all(|track| track.frames == 9));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn recovery_refuses_future_manifests_and_unsafe_stem_paths() {
        let outer = std::env::temp_dir().join(format!("shr-safe-recovery-{}", std::process::id()));
        let base = outer.join("recordings");
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(&base).unwrap();

        let future = test_paths(&base, "future");
        fs::create_dir(&future.temporary).unwrap();
        let mut future_manifest = manifest_for(&future, &tracks(1), 48_000);
        future_manifest.format_version = MANIFEST_VERSION + 1;
        write_manifest(&future.temporary, &future_manifest).unwrap();
        assert!(recover_session_directory(&future.temporary).is_err());
        assert!(future.temporary.exists());

        let unsafe_paths = test_paths(&base, "unsafe");
        fs::create_dir(&unsafe_paths.temporary).unwrap();
        let mut unsafe_manifest = manifest_for(&unsafe_paths, &tracks(1), 48_000);
        unsafe_manifest.tracks[0].wav_file = "../../outside.wav".into();
        write_manifest(&unsafe_paths.temporary, &unsafe_manifest).unwrap();
        let outside = outer.join("outside.wav.part");
        fs::write(&outside, b"must stay untouched").unwrap();
        assert!(recover_session_directory(&unsafe_paths.temporary).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"must stay untouched");
        assert!(unsafe_paths.temporary.exists());
        let _ = fs::remove_dir_all(outer);
    }

    #[test]
    fn recovery_never_follows_part_symlinks_and_names_never_replace() {
        let base = std::env::temp_dir().join(format!("shr-safe-take-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let target = base.join("target.bin");
        fs::write(&target, b"private").unwrap();
        let link = base.join("linked.wav.part");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(recover_interrupted(&base).unwrap(), [link]);
        assert_eq!(fs::read(&target).unwrap(), b"private");
        let first = unique_session_paths(&base, "safe").unwrap();
        fs::create_dir(&first.final_path).unwrap();
        assert_ne!(
            unique_session_paths(&base, "safe").unwrap().final_path,
            first.final_path
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn callback_faults_are_lock_free_atomic_state_transitions() {
        let ring = Arc::new(InterleavedRing::new(1, 8).unwrap());
        let running = Arc::new(AtomicBool::new(true));
        let capture = Arc::new(AtomicBool::new(true));
        let fault = Arc::new(AtomicU32::new(FAULT_NONE));
        let data = CallbackData {
            ring,
            running: Arc::clone(&running),
            capture_enabled: Arc::clone(&capture),
            fault: Arc::clone(&fault),
            xruns: Arc::new(AtomicU64::new(0)),
            callback_violations: Arc::new(AtomicU64::new(0)),
            accepted_frames: Arc::new(AtomicU64::new(0)),
            ports: Vec::new().into_boxed_slice(),
            port_ids: vec![42].into_boxed_slice(),
            buffers: Vec::<UnsafeCell<*const f32>>::new().into_boxed_slice(),
            peaks: Vec::<AtomicU32>::new().into(),
            maximum_callback_frames: 256,
            port_get_buffer: dummy_buffer,
        };
        unsafe {
            process_callback(
                257,
                (&data as *const CallbackData).cast_mut().cast::<c_void>(),
            )
        };
        assert_eq!(fault.load(Ordering::Acquire), FAULT_CALLBACK_SIZE);
        assert_eq!(data.callback_violations.load(Ordering::Relaxed), 1);
        running.store(true, Ordering::Release);
        capture.store(true, Ordering::Release);
        fault.store(FAULT_NONE, Ordering::Release);
        set_fault(&data, FAULT_SOURCE_LOST);
        assert_eq!(fault.load(Ordering::Acquire), FAULT_SOURCE_LOST);
        assert!(!running.load(Ordering::Acquire));
        assert!(!capture.load(Ordering::Acquire));

        running.store(true, Ordering::Release);
        capture.store(true, Ordering::Release);
        fault.store(FAULT_NONE, Ordering::Release);
        unsafe {
            port_connect_callback(7, 42, 0, (&data as *const CallbackData).cast_mut().cast())
        };
        assert_eq!(fault.load(Ordering::Acquire), FAULT_SOURCE_LOST);
    }

    #[test]
    fn jack_free_synthetic_18_channel_path_publishes_and_verifies() {
        let base = std::env::temp_dir().join(format!("shr-stress-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let report =
            run_synthetic_stress(&base, Duration::from_millis(25), 18, 48_000, 128).unwrap();
        assert_eq!(report.channels, 18);
        assert_eq!(report.total_frames, 1200);
        assert_eq!(report.dropped_frames, 0);
        assert!(report.channel_identity_verified);
        assert!(report.session.join("session.json").is_file());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn jack_free_final_mix_stress_uses_three_sources_and_verifies_full_pcm() {
        let base =
            std::env::temp_dir().join(format!("shr-final-stress-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let report = run_final_mix_stress(&base, Duration::from_millis(25), 48_000, 128).unwrap();
        assert_eq!(report.total_frames, 1200);
        assert_eq!(report.dropped_frames, 0);
        assert_eq!(report.overflow_events, 0);
        assert!(report.maximum_gain_reduction_db > 0.0);
        assert!(report.output_file_equal);
        assert_eq!(read_stereo_i24(&report.wav).len(), 1200);
        let _ = fs::remove_dir_all(base);
    }

    unsafe extern "C" fn dummy_buffer(_: *mut JackPort, _: c_uint) -> *mut c_void {
        std::ptr::null_mut()
    }

    fn read_stereo_i24(path: &Path) -> Vec<StereoFrame> {
        let bytes = fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u16::from_le_bytes(bytes[22..24].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(bytes[34..36].try_into().unwrap()), 24);
        bytes[44..]
            .chunks_exact(6)
            .map(|frame| {
                let decode = |sample: &[u8]| {
                    let raw = i32::from(sample[0])
                        | (i32::from(sample[1]) << 8)
                        | (i32::from(sample[2]) << 16);
                    let signed = if raw & 0x80_0000 != 0 {
                        raw | !0xff_ffff
                    } else {
                        raw
                    };
                    signed as f32 / 8_388_607.0
                };
                StereoFrame::new(decode(&frame[..3]), decode(&frame[3..]))
            })
            .collect()
    }

    #[test]
    fn final_mix_callback_boundaries_and_pcm_equal_playback_quantization() {
        let base = std::env::temp_dir().join(format!("shr-final-mix-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let mut recorder = FinalMixRecorder::new(base.clone(), 48_000, 4096, 256).unwrap();
        let capture = recorder.capture_handle();
        recorder.start(Some("equality")).unwrap();
        let first = (0..128)
            .map(|index| StereoFrame::new(index as f32 / 512.0, -(index as f32) / 1024.0))
            .collect::<Vec<_>>();
        let second = (0..64)
            .map(|index| StereoFrame::new(-0.25 + index as f32 / 1024.0, 0.125))
            .collect::<Vec<_>>();
        assert_no_allocations(|| capture.capture(&first));
        assert_no_allocations(|| capture.capture(&second));
        recorder.request_stop();
        assert_no_allocations(|| capture.capture(&[]));
        recorder.finish_stop().unwrap();
        let status = recorder.status();
        assert_eq!(status.total_frames, 192);
        assert_eq!(status.dropped_frames, 0);
        assert_eq!(status.overflow_events, 0);
        let path = status.path.unwrap();
        let decoded = read_stereo_i24(&path);
        let expected = first.iter().chain(&second).copied().collect::<Vec<_>>();
        assert_eq!(decoded.len(), expected.len());
        for (actual, expected) in decoded.iter().zip(expected) {
            assert!((actual.left - expected.left).abs() <= 1.0 / 8_388_607.0);
            assert!((actual.right - expected.right).abs() <= 1.0 / 8_388_607.0);
        }
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn final_mix_source_loss_is_recoverable_and_never_looks_successful() {
        let base =
            std::env::temp_dir().join(format!("shr-final-source-loss-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let mut recorder = FinalMixRecorder::new(base.clone(), 44_100, 512, 128).unwrap();
        let capture = recorder.capture_handle();
        recorder.start(None).unwrap();
        capture.capture(&[StereoFrame::new(0.1, -0.2); 128]);
        capture.source_lost();
        assert!(recorder.finish_stop().is_err());
        let status = recorder.status();
        assert!(!status.recording);
        assert!(status.error.unwrap().contains("source disappeared"));
        let part = status.path.unwrap();
        assert_eq!(
            part.extension().and_then(|value| value.to_str()),
            Some("part")
        );
        assert!(!base.join("final-mix.wav").exists());
        let recovered = recover_interrupted(&base).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(read_stereo_i24(&recovered[0]).len(), 128);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn final_mix_zero_frame_failure_and_publication_collision_are_safe() {
        let base = std::env::temp_dir().join(format!("shr-final-safe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let temporary = base.join("empty.wav.part");
        let destination = base.join("empty.wav");
        let shared = FinalMixShared {
            ring: InterleavedRing::new(2, 64).unwrap(),
            mode: AtomicU32::new(FINAL_CAPTURE_IDLE),
            writer_running: AtomicBool::new(false),
            fault: AtomicU32::new(FAULT_NONE),
            accepted_frames: AtomicU64::new(0),
            written_frames: AtomicU64::new(0),
            sample_rate: 48_000,
            maximum_callback_frames: 64,
        };
        assert!(write_final_mix(
            &temporary,
            &destination,
            &shared,
            &Mutex::new(FinalMixRecorderStatus::default()),
            WriterBehavior::default(),
        )
        .is_err());
        assert!(!temporary.exists());
        fs::write(&destination, b"owned by someone else").unwrap();
        let frames = [StereoFrame::new(0.1, 0.2); 16];
        assert!(shared.ring.push_stereo(&frames));
        let collision_part = base.join("collision.wav.part");
        assert!(write_final_mix(
            &collision_part,
            &destination,
            &shared,
            &Mutex::new(FinalMixRecorderStatus::default()),
            WriterBehavior::default(),
        )
        .is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"owned by someone else");
        assert!(collision_part.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn final_mix_overflow_writer_failure_and_disk_error_are_honest() {
        let shared = Arc::new(FinalMixShared {
            ring: InterleavedRing::new(2, 17).unwrap(),
            mode: AtomicU32::new(FINAL_CAPTURE_ACTIVE),
            writer_running: AtomicBool::new(true),
            fault: AtomicU32::new(FAULT_NONE),
            accepted_frames: AtomicU64::new(0),
            written_frames: AtomicU64::new(0),
            sample_rate: 48_000,
            maximum_callback_frames: 16,
        });
        let capture = FinalMixCapture {
            shared: Arc::clone(&shared),
        };
        capture.capture(&[StereoFrame::new(0.1, -0.1); 16]);
        capture.capture(&[StereoFrame::new(0.2, -0.2); 16]);
        assert_eq!(shared.fault.load(Ordering::Acquire), FAULT_OVERFLOW);
        assert_eq!(shared.mode.load(Ordering::Acquire), FINAL_CAPTURE_IDLE);
        assert_eq!(shared.ring.dropped.load(Ordering::Relaxed), 16);
        assert_eq!(shared.ring.overflows.load(Ordering::Relaxed), 1);

        let base =
            std::env::temp_dir().join(format!("shr-final-writer-errors-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let slow_shared = Arc::new(FinalMixShared {
            ring: InterleavedRing::new(2, 129).unwrap(),
            mode: AtomicU32::new(FINAL_CAPTURE_ACTIVE),
            writer_running: AtomicBool::new(true),
            fault: AtomicU32::new(FAULT_NONE),
            accepted_frames: AtomicU64::new(0),
            written_frames: AtomicU64::new(0),
            sample_rate: 48_000,
            maximum_callback_frames: 16,
        });
        let slow_status = Arc::new(Mutex::new(FinalMixRecorderStatus::default()));
        let slow_part = base.join("slow.wav.part");
        let slow_final = base.join("slow.wav");
        let worker_shared = Arc::clone(&slow_shared);
        let worker_status = Arc::clone(&slow_status);
        let worker_part = slow_part.clone();
        let worker_final = slow_final.clone();
        let worker = thread::spawn(move || {
            write_final_mix(
                &worker_part,
                &worker_final,
                &worker_shared,
                &worker_status,
                WriterBehavior {
                    delay: Duration::from_millis(2),
                    fail_after_frames: None,
                },
            )
        });
        for block in 0..8 {
            let sample = 0.01 * (block + 1) as f32;
            assert!(slow_shared
                .ring
                .push_stereo(&[StereoFrame::new(sample, -sample); 16]));
        }
        slow_shared.writer_running.store(false, Ordering::Release);
        worker.join().unwrap().unwrap();
        assert_eq!(read_stereo_i24(&slow_final).len(), 128);
        assert!(slow_shared.ring.high_water.load(Ordering::Relaxed) >= 16);

        let writer_shared = FinalMixShared {
            ring: InterleavedRing::new(2, 64).unwrap(),
            mode: AtomicU32::new(FINAL_CAPTURE_IDLE),
            writer_running: AtomicBool::new(false),
            fault: AtomicU32::new(FAULT_NONE),
            accepted_frames: AtomicU64::new(0),
            written_frames: AtomicU64::new(0),
            sample_rate: 48_000,
            maximum_callback_frames: 16,
        };
        assert!(writer_shared
            .ring
            .push_stereo(&[StereoFrame::new(0.1, 0.2); 16]));
        let failed_part = base.join("failed.wav.part");
        assert!(write_final_mix(
            &failed_part,
            &base.join("failed.wav"),
            &writer_shared,
            &Mutex::new(FinalMixRecorderStatus::default()),
            WriterBehavior {
                delay: Duration::from_millis(1),
                fail_after_frames: Some(0),
            },
        )
        .is_err());
        assert!(failed_part.exists());

        let missing_parent = base.join("missing");
        assert!(write_final_mix(
            &missing_parent.join("disk.wav.part"),
            &missing_parent.join("disk.wav"),
            &writer_shared,
            &Mutex::new(FinalMixRecorderStatus::default()),
            WriterBehavior::default(),
        )
        .is_err());
        assert!(!missing_parent.exists());
        let _ = fs::remove_dir_all(base);
    }
}
