//! Engine-independent stereo JACK capture. The JACK callback only copies into
//! a fixed SPSC ring; WAV conversion and disk I/O happen on a worker thread.
use crate::config::{AudioCaptureConfig, StereoInputConfig};
use anyhow::{anyhow, bail, Context, Result};
use std::cell::UnsafeCell;
use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const JACK_DEFAULT_AUDIO_TYPE: &[u8] = b"32 bit float mono audio\0";
const JACK_PORT_IS_INPUT: u64 = 1;
const JACK_NO_START_SERVER: c_uint = 1;
const WAV_MAX_DATA_BYTES: u64 = u32::MAX as u64 - 36;
const WAV_MAX_FRAMES: u64 = WAV_MAX_DATA_BYTES / 6;

struct StereoRing {
    frames: Box<[UnsafeCell<[f32; 2]>]>,
    read: AtomicUsize,
    write: AtomicUsize,
    dropped: AtomicU64,
}
// One JACK producer and one disk consumer access disjoint slots, coordinated
// by acquire/release indices.
unsafe impl Sync for StereoRing {}

impl StereoRing {
    fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2) + 1;
        Self {
            frames: (0..capacity).map(|_| UnsafeCell::new([0.0; 2])).collect(),
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }
    /// Allocation-free, nonblocking producer operation for the RT callback.
    fn push_slices(&self, left: &[f32], right: &[f32]) {
        let mut write = self.write.load(Ordering::Relaxed);
        let mut dropped = 0;
        for (&left, &right) in left.iter().zip(right) {
            let next = (write + 1) % self.frames.len();
            if next == self.read.load(Ordering::Acquire) {
                dropped += 1;
                continue;
            }
            // SAFETY: only the single producer writes the slot at `write`, and
            // it publishes that slot after the write with Release ordering.
            unsafe {
                *self.frames[write].get() = [left, right];
            }
            write = next;
            self.write.store(write, Ordering::Release);
        }
        if dropped > 0 {
            self.dropped.fetch_add(dropped, Ordering::Relaxed);
        }
    }
    fn pop(&self) -> Option<[f32; 2]> {
        let read = self.read.load(Ordering::Relaxed);
        if read == self.write.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: the single consumer reads only a slot published by producer.
        let frame = unsafe { *self.frames[read].get() };
        self.read
            .store((read + 1) % self.frames.len(), Ordering::Release);
        Some(frame)
    }
    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    fn is_empty(&self) -> bool {
        self.read.load(Ordering::Acquire) == self.write.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecorderStatus {
    pub recording: bool,
    pub elapsed: Duration,
    pub bytes: u64,
    pub sample_rate: u32,
    pub dropped_frames: u64,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}

struct SharedStatus {
    started: Instant,
    public: RecorderStatus,
}
struct CallbackData {
    ring: Arc<StereoRing>,
    running: Arc<AtomicBool>,
    left: *mut JackPort,
    right: *mut JackPort,
    port_get_buffer: PortGetBuffer,
}
unsafe impl Send for CallbackData {}

pub struct AudioRecorder {
    config: AudioCaptureConfig,
    status: Arc<Mutex<SharedStatus>>,
    active: Option<Active>,
}
struct Active {
    jack: JackClient,
    running: Arc<AtomicBool>,
    ring: Arc<StereoRing>,
    worker: Option<thread::JoinHandle<()>>,
    callback_data: Box<CallbackData>,
}

impl AudioRecorder {
    pub fn new(config: AudioCaptureConfig) -> Self {
        Self {
            config,
            status: Arc::new(Mutex::new(SharedStatus {
                started: Instant::now(),
                public: RecorderStatus::default(),
            })),
            active: None,
        }
    }
    pub fn status(&self) -> RecorderStatus {
        self.status
            .lock()
            .map(|s| {
                let mut p = s.public.clone();
                if p.recording {
                    p.elapsed = s.started.elapsed();
                }
                p
            })
            .unwrap_or_default()
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
        let input = self
            .config
            .inputs
            .first()
            .context("no stereo capture.input configured")?
            .clone();
        fs::create_dir_all(&self.config.directory)?;
        let recovered = recover_interrupted(&self.config.directory)?;
        if available_bytes(&self.config.directory)? < 64 * 1024 * 1024 {
            bail!("less than 64 MiB free in recording directory");
        }
        let stem = recording_stem(optional_name);
        let final_path = unique_path(&self.config.directory, &stem, "wav")?;
        let tmp_path = final_path.with_extension("wav.part");
        let ring = Arc::new(StereoRing::new(self.config.ring_frames));
        let running = Arc::new(AtomicBool::new(true));
        let publish = Arc::new(AtomicBool::new(false));
        let owns_temporary = Arc::new(AtomicBool::new(false));
        let mut jack = JackClient::open(&self.config.client_name)?;
        let sample_rate = jack.sample_rate();
        if !(8_000..=384_000).contains(&sample_rate) {
            bail!("JACK reported invalid sample rate {sample_rate}");
        }
        let left = jack.register_input("input_l")?;
        let right = jack.register_input("input_r")?;
        let mut callback_data = Box::new(CallbackData {
            ring: Arc::clone(&ring),
            running: Arc::clone(&running),
            left,
            right,
            port_get_buffer: jack.api.port_get_buffer,
        });
        jack.set_callback((&mut *callback_data) as *mut CallbackData)?;
        let worker_status = Arc::clone(&self.status);
        let worker_ring = Arc::clone(&ring);
        let worker_running = Arc::clone(&running);
        let worker_publish = Arc::clone(&publish);
        let worker_owns_temporary = Arc::clone(&owns_temporary);
        let worker_final = final_path.clone();
        if let Ok(mut s) = self.status.lock() {
            s.started = Instant::now();
            s.public = RecorderStatus {
                recording: true,
                sample_rate,
                path: Some(final_path.clone()),
                error: (!recovered.is_empty()).then(|| {
                    format!(
                        "recovered/reported {} interrupted recording(s)",
                        recovered.len()
                    )
                }),
                ..RecorderStatus::default()
            };
        }
        let worker_tmp = tmp_path.clone();
        let worker = thread::Builder::new()
            .name("shsynth-wav-writer".into())
            .spawn(move || {
                let result = write_worker(
                    RecordingPaths {
                        temporary: &worker_tmp,
                        final_path: &worker_final,
                    },
                    sample_rate,
                    &worker_ring,
                    &worker_running,
                    &worker_publish,
                    &worker_owns_temporary,
                    &worker_status,
                );
                worker_running.store(false, Ordering::Release);
                if let Err(error) = result {
                    if worker_owns_temporary.load(Ordering::Acquire)
                        && !worker_publish.load(Ordering::Acquire)
                    {
                        let _ = fs::remove_file(&worker_tmp);
                    }
                    if let Ok(mut s) = worker_status.lock() {
                        s.public.error = Some(error.to_string());
                        s.public.recording = false;
                    }
                }
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                running.store(false, Ordering::Release);
                if let Ok(mut s) = self.status.lock() {
                    s.public.recording = false;
                    s.public.error = Some(error.to_string());
                }
                return Err(error).context("start WAV writer thread");
            }
        };
        if let Err(error) = jack.activate_and_connect(&input, left, right) {
            jack.deactivate();
            running.store(false, Ordering::Release);
            let _ = worker.join();
            if let Ok(mut s) = self.status.lock() {
                s.public.recording = false;
                s.public.error = Some(error.to_string());
            }
            return Err(error);
        }
        publish.store(true, Ordering::Release);
        if worker.is_finished() {
            jack.deactivate();
            running.store(false, Ordering::Release);
            let _ = worker.join();
            let error = self
                .status()
                .error
                .unwrap_or_else(|| "WAV writer stopped during startup".into());
            bail!(error);
        }
        self.active = Some(Active {
            jack,
            running,
            ring,
            worker: Some(worker),
            callback_data,
        });
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        active.jack.deactivate();
        active.running.store(false, Ordering::Release);
        let join_result = active
            .worker
            .take()
            .map(|worker| worker.join())
            .transpose()
            .map_err(|_| anyhow!("WAV writer thread panicked"));
        if let Ok(mut s) = self.status.lock() {
            s.public.recording = false;
            s.public.dropped_frames = active.ring.dropped();
        }
        drop(active.callback_data);
        join_result.map(|_| ())
    }
}
impl Drop for AudioRecorder {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct RecordingPaths<'a> {
    temporary: &'a Path,
    final_path: &'a Path,
}

fn write_worker(
    paths: RecordingPaths<'_>,
    sample_rate: u32,
    ring: &StereoRing,
    running: &AtomicBool,
    publish: &AtomicBool,
    owns_temporary: &AtomicBool,
    status: &Mutex<SharedStatus>,
) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(paths.temporary)
        .with_context(|| format!("create {}", paths.temporary.display()))?;
    owns_temporary.store(true, Ordering::Release);
    let mut file = BufWriter::with_capacity(64 * 1024, file);
    write_wav_header(&mut file, sample_rate, 0)?;
    let mut frames = 0u64;
    let mut limit_reached = false;
    'capture: while running.load(Ordering::Acquire) || !ring.is_empty() {
        let mut wrote = false;
        while let Some([left, right]) = ring.pop() {
            if frames >= WAV_MAX_FRAMES {
                limit_reached = true;
                running.store(false, Ordering::Release);
                while ring.pop().is_some() {}
                break 'capture;
            }
            write_i24(&mut file, left)?;
            write_i24(&mut file, right)?;
            frames += 1;
            wrote = true;
            if frames % 4096 == 0 {
                if let Ok(mut s) = status.lock() {
                    s.public.bytes = 44 + frames * 6;
                    s.public.dropped_frames = ring.dropped();
                }
            }
        }
        if !wrote {
            thread::sleep(Duration::from_millis(2));
        }
    }
    if !publish.load(Ordering::Acquire) {
        drop(file);
        fs::remove_file(paths.temporary)?;
        if let Ok(mut s) = status.lock() {
            s.public.recording = false;
            s.public.bytes = 0;
        }
        return Ok(());
    }
    finalize_wav(&mut file, sample_rate, frames)?;
    file.flush()?;
    file.get_ref().sync_all()?;
    drop(file);
    crate::fsutil::rename_noreplace(paths.temporary, paths.final_path).with_context(|| {
        format!(
            "recording destination exists; kept {}",
            paths.temporary.display()
        )
    })?;
    if let Ok(mut s) = status.lock() {
        s.public.bytes = 44 + frames * 6;
        s.public.dropped_frames = ring.dropped();
        s.public.recording = false;
        if limit_reached {
            s.public.error = Some("WAV reached the 4 GiB RIFF limit and was finalized".into());
        }
    }
    Ok(())
}

fn write_i24(file: &mut impl Write, value: f32) -> Result<()> {
    let sample = (value.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
    let bytes = sample.to_le_bytes();
    file.write_all(&bytes[..3])?;
    Ok(())
}
fn write_wav_header(file: &mut impl Write, rate: u32, data: u32) -> Result<()> {
    let riff_size = 36u32.checked_add(data).context("WAV size overflow")?;
    let byte_rate = rate.checked_mul(6).context("WAV sample rate overflow")?;
    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&6u16.to_le_bytes())?;
    file.write_all(&24u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data.to_le_bytes())?;
    Ok(())
}
fn finalize_wav(file: &mut (impl Write + Seek), rate: u32, frames: u64) -> Result<()> {
    let data = frames
        .checked_mul(6)
        .filter(|n| *n <= WAV_MAX_DATA_BYTES)
        .context("WAV exceeded 4 GiB PCM limit")? as u32;
    file.seek(SeekFrom::Start(0))?;
    write_wav_header(file, rate, data)
}

pub fn recover_interrupted(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(found);
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("part") {
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if len >= 44 && is_wav_part(&path) {
                let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
                let frames = ((len - 44) / 6).min(WAV_MAX_FRAMES);
                file.set_len(44 + frames * 6)?;
                let rate = read_wav_rate(&mut file).context("invalid interrupted WAV header")?;
                finalize_wav(&mut file, rate, frames)?;
                file.sync_all()?;
                drop(file);
                let stem = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("interrupted.wav.part")
                    .trim_end_matches(".wav.part");
                let recovered = unique_path(dir, &format!("{stem}-recovered"), "wav")?;
                crate::fsutil::rename_noreplace(&path, &recovered)?;
                found.push(recovered);
            } else {
                found.push(path);
            }
        }
    }
    Ok(found)
}
fn read_wav_rate(file: &mut File) -> Option<u32> {
    use std::io::Read;
    let mut bytes = [0u8; 4];
    file.seek(SeekFrom::Start(24)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    let rate = u32::from_le_bytes(bytes);
    (8_000..=384_000).contains(&rate).then_some(rate)
}
fn is_wav_part(path: &Path) -> bool {
    use std::io::Read;
    let mut header = [0u8; 44];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header[..4] == b"RIFF"
        && &header[8..12] == b"WAVE"
        && &header[12..20] == b"fmt \x10\0\0\0"
        && &header[20..24] == b"\x01\0\x02\0"
        && &header[32..36] == b"\x06\0\x18\0"
        && &header[36..40] == b"data"
        && read_wav_rate_bytes(&header).is_some()
}
fn read_wav_rate_bytes(header: &[u8; 44]) -> Option<u32> {
    let rate = u32::from_le_bytes(header[24..28].try_into().ok()?);
    let byte_rate = u32::from_le_bytes(header[28..32].try_into().ok()?);
    ((8_000..=384_000).contains(&rate) && rate.checked_mul(6) == Some(byte_rate)).then_some(rate)
}
fn available_bytes(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).context("recording disk space");
    };
    let value = unsafe { value.assume_init() };
    Ok(value.f_bavail.saturating_mul(value.f_frsize))
}
fn recording_stem(name: Option<&str>) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let suffix = name
        .map(crate::sequencer::safe_name)
        .filter(|s| s != "untitled");
    suffix.map_or_else(
        || format!("recording-{secs}"),
        |s| format!("recording-{secs}-{s}"),
    )
}
fn unique_path(dir: &Path, stem: &str, ext: &str) -> Result<PathBuf> {
    for n in 0..10_000 {
        let name = if n == 0 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        let p = dir.join(name);
        if !p.exists() && !p.with_extension(format!("{ext}.part")).exists() {
            return Ok(p);
        }
    }
    bail!("could not choose a unique recording filename")
}

type ProcessCallback = unsafe extern "C" fn(c_uint, *mut c_void) -> c_int;
type PortGetBuffer = unsafe extern "C" fn(*mut JackPort, c_uint) -> *mut c_void;
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
type PortRegister =
    unsafe extern "C" fn(*mut JackOpaque, *const c_char, *const c_char, u64, u64) -> *mut JackPort;
type SetProcess = unsafe extern "C" fn(*mut JackOpaque, ProcessCallback, *mut c_void) -> c_int;
type Activate = unsafe extern "C" fn(*mut JackOpaque) -> c_int;
type Deactivate = unsafe extern "C" fn(*mut JackOpaque) -> c_int;
type Connect = unsafe extern "C" fn(*mut JackOpaque, *const c_char, *const c_char) -> c_int;
type PortName = unsafe extern "C" fn(*const JackPort) -> *const c_char;
type SampleRate = unsafe extern "C" fn(*mut JackOpaque) -> c_uint;
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
unsafe impl Send for JackApi {}
struct JackClient {
    client: *mut JackOpaque,
    api: JackApi,
    active: bool,
}
unsafe impl Send for JackClient {}
impl JackClient {
    fn open(name: &str) -> Result<Self> {
        let name = CString::new(name)?;
        unsafe {
            let handle = libc::dlopen(c"libjack.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                bail!("JACK library unavailable")
            };
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
                bail!("JACK server unavailable (status {status})")
            };
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
    fn register_input(&self, name: &str) -> Result<*mut JackPort> {
        let name = CString::new(name)?;
        let p = unsafe {
            (self.api.port_register)(
                self.client,
                name.as_ptr(),
                JACK_DEFAULT_AUDIO_TYPE.as_ptr().cast(),
                JACK_PORT_IS_INPUT,
                0,
            )
        };
        if p.is_null() {
            bail!("register JACK input {name:?}")
        }
        Ok(p)
    }
    fn set_callback(&self, data: *mut CallbackData) -> Result<()> {
        if unsafe { (self.api.set_process)(self.client, process_callback, data.cast()) } != 0 {
            bail!("set JACK capture callback")
        }
        Ok(())
    }
    fn activate_and_connect(
        &mut self,
        input: &StereoInputConfig,
        left: *mut JackPort,
        right: *mut JackPort,
    ) -> Result<()> {
        if unsafe { (self.api.activate)(self.client) } != 0 {
            bail!("activate JACK recorder")
        };
        self.active = true;
        let left_name = unsafe { (self.api.port_name)(left) };
        let right_name = unsafe { (self.api.port_name)(right) };
        if left_name.is_null() || right_name.is_null() {
            self.deactivate();
            bail!("JACK recorder returned an unnamed port")
        }
        let lp = unsafe { std::ffi::CStr::from_ptr(left_name) };
        let rp = unsafe { std::ffi::CStr::from_ptr(right_name) };
        let l = CString::new(input.left_port.as_str())?;
        let r = CString::new(input.right_port.as_str())?;
        if unsafe { (self.api.connect)(self.client, l.as_ptr(), lp.as_ptr()) } != 0 {
            self.deactivate();
            bail!("connect JACK input {}", input.left_port)
        };
        if unsafe { (self.api.connect)(self.client, r.as_ptr(), rp.as_ptr()) } != 0 {
            self.deactivate();
            bail!("connect JACK input {}", input.right_port)
        };
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
    let p = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
    if p.is_null() {
        bail!("JACK symbol unavailable")
    };
    Ok(unsafe { std::mem::transmute_copy(&p) })
}
unsafe extern "C" fn process_callback(frames: c_uint, arg: *mut c_void) -> c_int {
    let data = unsafe { &*(arg.cast::<CallbackData>()) };
    if !data.running.load(Ordering::Acquire) {
        return 0;
    }
    let left = unsafe { (data.port_get_buffer)(data.left, frames) }.cast::<f32>();
    let right = unsafe { (data.port_get_buffer)(data.right, frames) }.cast::<f32>();
    if left.is_null() || right.is_null() {
        return 0;
    }
    let l = unsafe { std::slice::from_raw_parts(left, frames as usize) };
    let r = unsafe { std::slice::from_raw_parts(right, frames as usize) };
    data.ring.push_slices(l, r);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_overflow_is_accounted_without_blocking() {
        let r = StereoRing::new(2);
        r.push_slices(&[1., 2., 3.], &[4., 5., 6.]);
        assert_eq!(r.dropped(), 1);
        assert_eq!(r.pop(), Some([1., 4.]));
        assert_eq!(r.pop(), Some([2., 5.]));
    }
    #[test]
    fn wav_finalizes_and_part_is_recoverable() {
        let d = std::env::temp_dir().join(format!("shwav-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let p = d.join("x.wav.part");
        let f = d.join("x.wav");
        let ring = StereoRing::new(8);
        ring.push_slices(&[0.0, 0.5], &[0.0, -0.5]);
        let run = AtomicBool::new(false);
        let publish = AtomicBool::new(true);
        let owns_temporary = AtomicBool::new(false);
        let status = Mutex::new(SharedStatus {
            started: Instant::now(),
            public: RecorderStatus::default(),
        });
        write_worker(
            RecordingPaths {
                temporary: &p,
                final_path: &f,
            },
            48_000,
            &ring,
            &run,
            &publish,
            &owns_temporary,
            &status,
        )
        .unwrap();
        let b = fs::read(&f).unwrap();
        assert_eq!(&b[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 12);
        fs::write(d.join("lost.wav.part"), b"partial").unwrap();
        assert_eq!(recover_interrupted(&d).unwrap().len(), 1);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn riff_limit_accepts_only_the_last_valid_frame() {
        let mut wav = std::io::Cursor::new(Vec::new());
        finalize_wav(&mut wav, 48_000, WAV_MAX_FRAMES).unwrap();
        let bytes = wav.into_inner();
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            (WAV_MAX_FRAMES * 6) as u32
        );

        let mut oversized = std::io::Cursor::new(Vec::new());
        assert!(finalize_wav(&mut oversized, 48_000, WAV_MAX_FRAMES + 1).is_err());
    }

    #[test]
    fn failed_jack_start_does_not_publish_an_empty_recording() {
        let d = std::env::temp_dir().join(format!("shwav-unpublished-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let part = d.join("x.wav.part");
        let final_path = d.join("x.wav");
        let ring = StereoRing::new(8);
        let run = AtomicBool::new(false);
        let publish = AtomicBool::new(false);
        let owns_temporary = AtomicBool::new(false);
        let status = Mutex::new(SharedStatus {
            started: Instant::now(),
            public: RecorderStatus::default(),
        });

        write_worker(
            RecordingPaths {
                temporary: &part,
                final_path: &final_path,
            },
            48_000,
            &ring,
            &run,
            &publish,
            &owns_temporary,
            &status,
        )
        .unwrap();
        assert!(!part.exists());
        assert!(!final_path.exists());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn writer_never_removes_a_temporary_file_it_did_not_create() {
        let d = std::env::temp_dir().join(format!("shwav-existing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let part = d.join("x.wav.part");
        let final_path = d.join("x.wav");
        fs::write(&part, b"other recorder").unwrap();
        let ring = StereoRing::new(8);
        let run = AtomicBool::new(false);
        let publish = AtomicBool::new(false);
        let owns_temporary = AtomicBool::new(false);
        let status = Mutex::new(SharedStatus {
            started: Instant::now(),
            public: RecorderStatus::default(),
        });

        assert!(write_worker(
            RecordingPaths {
                temporary: &part,
                final_path: &final_path,
            },
            48_000,
            &ring,
            &run,
            &publish,
            &owns_temporary,
            &status,
        )
        .is_err());
        assert_eq!(fs::read(&part).unwrap(), b"other recorder");
        assert!(!owns_temporary.load(Ordering::Acquire));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn filenames_are_safe_and_never_replace() {
        let d = std::env::temp_dir();
        let stem = recording_stem(Some("../../ loud take"));
        assert!(!stem.contains('/'));
        let p = unique_path(&d, &stem, "wav").unwrap();
        fs::write(&p, []).unwrap();
        assert_ne!(unique_path(&d, &stem, "wav").unwrap(), p);
        let _ = fs::remove_file(p);
    }
}
