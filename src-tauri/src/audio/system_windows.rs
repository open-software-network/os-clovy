use crate::{
    audio::{
        active_silence::ActiveSilence, pause_state::PauseState, system_audio::SystemAudioStopResult,
    },
    domain::types::{AppError, AudioLevelDto, RecordingSource, SourceReadinessDto},
};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::{
    fs::File,
    io::BufWriter,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use windows::core::GUID;

pub const SYSTEM_AUDIO_PERMISSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const STARTUP_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default)]
struct WindowsSystemAudioStats {
    level: AudioLevelDto,
    max_level: f32,
    bytes_written: i64,
    last_error: Option<String>,
}

pub struct SystemAudioCapture {
    stop_requested: Arc<AtomicBool>,
    pause_state: Arc<PauseState>,
    stats: Arc<Mutex<WindowsSystemAudioStats>>,
    partial_path: PathBuf,
    final_path: PathBuf,
    worker: Option<JoinHandle<Result<(), AppError>>>,
}

impl SystemAudioCapture {
    pub fn start(
        partial_path: PathBuf,
        final_path: PathBuf,
        _timeline_offset: Duration,
        authorization: Option<crate::audio::capture::CaptureStartAuthorization>,
    ) -> Result<Self, AppError> {
        let stop_requested = Arc::new(AtomicBool::new(false));
        let pause_state = Arc::new(PauseState::default());
        let stats = Arc::new(Mutex::new(WindowsSystemAudioStats::default()));
        let (startup_sender, startup_receiver) = mpsc::channel();
        let worker = {
            let stop_requested = Arc::clone(&stop_requested);
            let pause_state = Arc::clone(&pause_state);
            let stats = Arc::clone(&stats);
            let partial_path = partial_path.clone();
            thread::Builder::new()
                .name("clovy-windows-system-audio".to_string())
                .spawn(move || {
                    capture_loop(
                        partial_path,
                        stop_requested,
                        pause_state,
                        stats,
                        startup_sender,
                        authorization,
                    )
                })
                .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))?
        };
        match startup_receiver.recv_timeout(STARTUP_HANDSHAKE_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                stop_requested.store(true, Ordering::Release);
                let _ = worker.join();
                let _ = std::fs::remove_file(&partial_path);
                return Err(error);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                stop_requested.store(true, Ordering::Release);
                return Err(AppError::new(
                    "system_audio_start_timeout",
                    "Could not start system audio capture. Try again, or check the selected output device.",
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let worker_result = worker.join().map_err(|_| {
                    AppError::new("system_audio_unavailable", "System audio worker panicked.")
                })?;
                return Err(worker_result.err().unwrap_or_else(|| {
                    AppError::new(
                        "system_audio_unavailable",
                        "System audio worker stopped before capture was ready.",
                    )
                }));
            }
        }
        Ok(Self {
            stop_requested,
            pause_state,
            stats,
            partial_path,
            final_path,
            worker: Some(worker),
        })
    }

    pub fn pause(&mut self) {
        self.pause_state.pause(Instant::now());
    }

    pub fn resume(&mut self) {
        self.pause_state.resume(Instant::now());
    }

    pub fn status(&self) -> (AudioLevelDto, i64, Option<String>) {
        self.stats
            .lock()
            .map(|stats| {
                (
                    stats.level.clone(),
                    stats.bytes_written,
                    stats.last_error.clone(),
                )
            })
            .unwrap_or_default()
    }

    pub fn stop(mut self) -> SystemAudioStopResult {
        self.stop_requested.store(true, Ordering::Release);
        let worker_result = match self.worker.take() {
            Some(worker) => worker.join().map_err(|_| {
                AppError::new("system_audio_unavailable", "System audio worker panicked.")
            }),
            None => Err(AppError::new(
                "system_audio_unavailable",
                "System audio worker is missing.",
            )),
        };
        let stop_error = worker_result.and_then(|result| result).err();
        let finalization_error =
            finalize_partial_system_audio(&self.partial_path, &self.final_path)
                .err()
                .map(|error| AppError::new("audio_finalization_failed", error.to_string()));
        match (stop_error, finalization_error) {
            (Some(error), Some(finalization_error)) => SystemAudioStopResult::Failed(
                AppError::new(
                    error.code,
                    format!(
                        "{} System audio file finalization also failed: {}",
                        error.message, finalization_error.message
                    ),
                )
                .into(),
            ),
            (Some(error), None) => SystemAudioStopResult::Failed(error.into()),
            (None, Some(error)) => SystemAudioStopResult::Failed(error.into()),
            (None, None) => SystemAudioStopResult::Stopped(self.final_path),
        }
    }
}

fn finalize_partial_system_audio(
    partial_path: &PathBuf,
    final_path: &PathBuf,
) -> std::io::Result<()> {
    if partial_path.exists() {
        std::fs::rename(partial_path, final_path)?;
    }
    Ok(())
}

pub fn system_audio_readiness() -> SourceReadinessDto {
    match default_render_endpoint_available() {
        Ok(()) => SourceReadinessDto {
            source: RecordingSource::System,
            required: true,
            ready: true,
            permission_state: "granted".to_string(),
            device_available: true,
            capture_available: true,
            recovery_action: None,
            message: None,
        },
        Err(error) => SourceReadinessDto {
            source: RecordingSource::System,
            required: true,
            ready: false,
            permission_state: "unsupported".to_string(),
            device_available: false,
            capture_available: false,
            recovery_action: Some("openSoundSettings".to_string()),
            message: Some(error.message),
        },
    }
}

pub fn helper_permission_check() -> Result<(), AppError> {
    default_render_endpoint_available()
}

fn capture_loop(
    partial_path: PathBuf,
    stop_requested: Arc<AtomicBool>,
    pause_state: Arc<PauseState>,
    stats: Arc<Mutex<WindowsSystemAudioStats>>,
    startup_sender: Sender<Result<(), AppError>>,
    authorization: Option<crate::audio::capture::CaptureStartAuthorization>,
) -> Result<(), AppError> {
    let result = run_capture_loop(
        partial_path,
        stop_requested,
        pause_state,
        Arc::clone(&stats),
        startup_sender,
        authorization,
    );
    if let Err(error) = result.as_ref() {
        latch_error(&stats, error);
    }
    result
}

fn run_capture_loop(
    partial_path: PathBuf,
    stop_requested: Arc<AtomicBool>,
    pause_state: Arc<PauseState>,
    stats: Arc<Mutex<WindowsSystemAudioStats>>,
    startup_sender: Sender<Result<(), AppError>>,
    authorization: Option<crate::audio::capture::CaptureStartAuthorization>,
) -> Result<(), AppError> {
    let mut backend = match WasapiLoopbackBackend::new(authorization) {
        Ok(backend) => backend,
        Err(error) => {
            let _ = startup_sender.send(Err(error.clone()));
            return Err(error);
        }
    };
    let spec = WavSpec {
        channels: backend.channels,
        sample_rate: backend.sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = match WavWriter::create(&partial_path, spec) {
        Ok(writer) => writer,
        Err(error) => {
            let error = AppError::new("audio_writer_failed", error.to_string());
            let _ = startup_sender.send(Err(error.clone()));
            return Err(error);
        }
    };
    let _ = startup_sender.send(Ok(()));
    let mut silence = ActiveSilence::new(pause_state.snapshot().active_time);
    let mut was_paused = false;
    while !stop_requested.load(Ordering::Acquire) {
        let pause = pause_state.snapshot();
        if pause.paused {
            if !was_paused {
                backend.pause()?;
                let frames = silence.pause(pause.active_time, backend.sample_rate);
                if frames > 0 {
                    write_silence(&mut writer, frames, backend.channels, &stats)?;
                }
                was_paused = true;
            }
            thread::sleep(CAPTURE_POLL_INTERVAL);
            continue;
        }
        if was_paused {
            backend.resume()?;
            silence.resume();
            was_paused = false;
        }
        let wrote_packet = backend.write_next_packet(&mut writer, &stats)?;
        if wrote_packet {
            silence.packet(pause_state.snapshot().active_time);
            continue;
        }
        // A zero packet size means WASAPI has not delivered a packet yet, not
        // proof that a gap belongs in the WAV at this exact poll. Interior
        // gaps are filled from the next packet's device position; only a
        // trailing idle tail falls back to wall-clock padding on stop.
        thread::sleep(CAPTURE_POLL_INTERVAL);
    }
    let end = pause_state.snapshot().active_time;
    let trailing_silence_frames = silence.finish(end, backend.sample_rate);
    if trailing_silence_frames > 0 {
        write_silence(
            &mut writer,
            trailing_silence_frames,
            backend.channels,
            &stats,
        )?;
    }
    writer
        .finalize()
        .map_err(|error| AppError::new("audio_finalization_failed", error.to_string()))
}

fn default_render_endpoint_available() -> Result<(), AppError> {
    WasapiLoopbackBackend::probe_default_endpoint()
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MixSampleFormat {
    PcmUnsigned8,
    PcmSigned16,
    PcmSigned24,
    PcmSigned32,
    /// WAVEFORMATEXTENSIBLE stores valid PCM bits left-aligned in a wider
    /// signed container. Keep both widths so decoding can remove padding.
    PcmSignedExtensible {
        container_bits: u16,
        valid_bits: u16,
    },
    Float32,
}

#[cfg(target_os = "windows")]
struct WasapiLoopbackBackend {
    audio_client: windows::Win32::Media::Audio::IAudioClient,
    capture_client: windows::Win32::Media::Audio::IAudioCaptureClient,
    _com: ComApartment,
    channels: u16,
    sample_rate: u32,
    bytes_per_frame: usize,
    sample_format: MixSampleFormat,
    expected_device_position: Option<u64>,
    started: bool,
}

#[cfg(target_os = "windows")]
struct MixFormatPtr(*mut windows::Win32::Media::Audio::WAVEFORMATEX);

#[cfg(target_os = "windows")]
impl MixFormatPtr {
    fn new(ptr: *mut windows::Win32::Media::Audio::WAVEFORMATEX) -> Self {
        Self(ptr)
    }

    fn as_ptr(&self) -> *mut windows::Win32::Media::Audio::WAVEFORMATEX {
        self.0
    }
}

#[cfg(target_os = "windows")]
impl Drop for MixFormatPtr {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(self.0.cast())) };
    }
}

#[cfg(target_os = "windows")]
impl WasapiLoopbackBackend {
    fn probe_default_endpoint() -> Result<(), AppError> {
        let backend = Self::new(None)?;
        drop(backend);
        Ok(())
    }

    fn new(
        authorization: Option<crate::audio::capture::CaptureStartAuthorization>,
    ) -> Result<Self, AppError> {
        use windows::Win32::Media::Audio::{
            IAudioCaptureClient, IAudioClient, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
        };
        use windows::Win32::System::Com::CLSCTX_ALL;

        let com = ComApartment::new()?;
        let device = default_render_endpoint()?;
        let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))?;
        let mix_format = MixFormatPtr::new(
            unsafe { audio_client.GetMixFormat() }
                .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))?,
        );
        let format = unsafe { *mix_format.as_ptr() };
        let channels = format.nChannels;
        let sample_rate = format.nSamplesPerSec;
        let bytes_per_frame = format.nBlockAlign as usize;
        let sample_format = mix_sample_format(mix_format.as_ptr())?;
        if channels == 0 || sample_rate == 0 || bytes_per_frame == 0 {
            return Err(AppError::new(
                "system_audio_unavailable",
                "Default output device reported an unusable audio format.",
            ));
        }
        unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                10_000_000,
                0,
                mix_format.as_ptr(),
                None,
            )
        }
        .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))?;
        let capture_client: IAudioCaptureClient = unsafe { audio_client.GetService() }
            .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))?;
        crate::audio::system_audio::admit_system_audio_start(authorization.as_ref(), || {
            unsafe { audio_client.Start() }
                .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))
        })?;
        Ok(Self {
            audio_client,
            capture_client,
            _com: com,
            channels,
            sample_rate,
            bytes_per_frame,
            sample_format,
            expected_device_position: None,
            started: true,
        })
    }

    fn write_next_packet(
        &mut self,
        writer: &mut WavWriter<BufWriter<File>>,
        stats: &Arc<Mutex<WindowsSystemAudioStats>>,
    ) -> Result<bool, AppError> {
        use windows::Win32::Media::Audio::AUDCLNT_BUFFERFLAGS_SILENT;
        let packet_frames =
            unsafe { self.capture_client.GetNextPacketSize() }.map_err(|error| {
                AppError::new("system_audio_capture_unavailable", error.to_string())
            })?;
        if packet_frames == 0 {
            return Ok(false);
        }
        let mut data = std::ptr::null_mut();
        let mut frames = 0;
        let mut flags = 0;
        let mut device_position = 0_u64;
        unsafe {
            self.capture_client.GetBuffer(
                &mut data,
                &mut frames,
                &mut flags,
                Some(&mut device_position),
                None,
            )
        }
        .map_err(|error| AppError::new("system_audio_capture_unavailable", error.to_string()))?;
        let is_silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
        let gap_frames = self.device_gap_frames(device_position);
        let write_result = (|| {
            if gap_frames > 0 {
                write_silence(writer, gap_frames, self.channels, stats)?;
            }
            if is_silent {
                write_silence(writer, frames, self.channels, stats)
            } else {
                let bytes = frames as usize * self.bytes_per_frame;
                let samples = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), bytes) };
                write_pcm16_from_mix_format(
                    writer,
                    samples,
                    self.channels,
                    self.bytes_per_frame,
                    self.sample_format,
                    stats,
                )
            }
        })();
        self.expected_device_position = Some(device_position.saturating_add(frames as u64));
        let release_result = unsafe { self.capture_client.ReleaseBuffer(frames) }
            .map_err(|error| AppError::new("system_audio_capture_unavailable", error.to_string()));
        write_result?;
        release_result?;
        Ok(true)
    }
    fn pause(&mut self) -> Result<(), AppError> {
        if self.started {
            unsafe { self.audio_client.Stop() }.map_err(|error| {
                AppError::new("system_audio_capture_unavailable", error.to_string())
            })?;
            unsafe { self.audio_client.Reset() }.map_err(|error| {
                AppError::new("system_audio_capture_unavailable", error.to_string())
            })?;
            self.expected_device_position = None;
            self.started = false;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), AppError> {
        if !self.started {
            unsafe { self.audio_client.Start() }.map_err(|error| {
                AppError::new("system_audio_capture_unavailable", error.to_string())
            })?;
            self.expected_device_position = None;
            self.started = true;
        }
        Ok(())
    }

    fn device_gap_frames(&mut self, device_position: u64) -> u32 {
        let expected_position = self.expected_device_position.unwrap_or(0);
        let gap_frames = device_position.saturating_sub(expected_position);
        gap_frames.min(u32::MAX as u64) as u32
    }
}

#[cfg(target_os = "windows")]
impl Drop for WasapiLoopbackBackend {
    fn drop(&mut self) {
        if self.started {
            let _ = unsafe { self.audio_client.Stop() };
        }
    }
}

#[cfg(target_os = "windows")]
struct ComApartment;

#[cfg(target_os = "windows")]
impl ComApartment {
    fn new() -> Result<Self, AppError> {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))?;
        Ok(Self)
    }
}

#[cfg(target_os = "windows")]
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
}

#[cfg(target_os = "windows")]
fn default_render_endpoint() -> Result<windows::Win32::Media::Audio::IMMDevice, AppError> {
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))?;
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .map_err(|error| AppError::new("system_audio_unavailable", error.to_string()))
}

#[cfg(not(target_os = "windows"))]
struct WasapiLoopbackBackend {
    channels: u16,
    sample_rate: u32,
}

#[cfg(not(target_os = "windows"))]
impl WasapiLoopbackBackend {
    fn probe_default_endpoint() -> Result<(), AppError> {
        Err(AppError::new(
            "system_audio_unsupported",
            "System audio capture is only supported on Windows in this backend.",
        ))
    }

    fn new(
        _authorization: Option<crate::audio::capture::CaptureStartAuthorization>,
    ) -> Result<Self, AppError> {
        Self::probe_default_endpoint()?;
        Ok(Self {
            channels: 2,
            sample_rate: 48_000,
        })
    }

    fn write_next_packet(
        &mut self,
        _writer: &mut WavWriter<BufWriter<File>>,
        _stats: &Arc<Mutex<WindowsSystemAudioStats>>,
    ) -> Result<bool, AppError> {
        Ok(false)
    }
}

fn write_silence(
    writer: &mut WavWriter<BufWriter<File>>,
    frames: u32,
    channels: u16,
    stats: &Arc<Mutex<WindowsSystemAudioStats>>,
) -> Result<(), AppError> {
    let samples = frames as usize * channels as usize;
    for _ in 0..samples {
        writer
            .write_sample(0_i16)
            .map_err(|error| AppError::new("audio_writer_failed", error.to_string()))?;
    }
    update_stats(stats, 0.0, 0.0, samples as i64 * 2, None);
    Ok(())
}

fn write_pcm16_from_mix_format(
    writer: &mut WavWriter<BufWriter<File>>,
    bytes: &[u8],
    channels: u16,
    bytes_per_frame: usize,
    sample_format: MixSampleFormat,
    stats: &Arc<Mutex<WindowsSystemAudioStats>>,
) -> Result<(), AppError> {
    let channels = channels as usize;
    if channels == 0 {
        return Err(AppError::new(
            "system_audio_unavailable",
            "Default output device reported an unusable audio format.",
        ));
    }
    let bytes_per_sample = bytes_per_frame / channels;
    if bytes_per_sample == 0 {
        return Err(AppError::new(
            "system_audio_unavailable",
            "Default output device reported an unusable audio format.",
        ));
    }
    let mut peak = 0.0_f32;
    let mut sum_square = 0.0_f64;
    let mut samples_written = 0_i64;
    for frame in bytes.chunks_exact(bytes_per_frame) {
        for sample in frame.chunks_exact(bytes_per_sample).take(channels) {
            let normalized = sample_to_f32(sample, sample_format);
            let pcm_sample = (normalized.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            let magnitude = normalized.abs();
            peak = peak.max(magnitude);
            sum_square += (magnitude as f64).powi(2);
            samples_written += 1;
            writer
                .write_sample(pcm_sample)
                .map_err(|error| AppError::new("audio_writer_failed", error.to_string()))?;
        }
    }
    let rms = if samples_written == 0 {
        0.0
    } else {
        (sum_square / samples_written as f64).sqrt() as f32
    };
    update_stats(stats, peak, rms, samples_written * 2, None);
    Ok(())
}

fn sample_to_f32(sample: &[u8], sample_format: MixSampleFormat) -> f32 {
    match sample_format {
        MixSampleFormat::Float32 => {
            f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]])
        }
        MixSampleFormat::PcmSigned32 => {
            i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as f32
                / i32::MAX as f32
        }
        MixSampleFormat::PcmSigned24 => {
            let sign = if sample[2] & 0x80 == 0 { 0x00 } else { 0xff };
            i32::from_le_bytes([sample[0], sample[1], sample[2], sign]) as f32 / 8_388_608.0
        }
        MixSampleFormat::PcmSignedExtensible {
            container_bits,
            valid_bits,
        } => left_aligned_extensible_sample_to_f32(sample, container_bits, valid_bits),
        MixSampleFormat::PcmSigned16 => {
            i16::from_le_bytes([sample[0], sample[1]]) as f32 / i16::MAX as f32
        }
        MixSampleFormat::PcmUnsigned8 => (sample[0] as f32 - 128.0) / 128.0,
    }
}

#[cfg(target_os = "windows")]
fn mix_sample_format(
    format_ptr: *mut windows::Win32::Media::Audio::WAVEFORMATEX,
) -> Result<MixSampleFormat, AppError> {
    const WAVE_FORMAT_PCM: u16 = 0x0001;
    const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
    const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

    let format = unsafe { &*format_ptr };
    match format.wFormatTag {
        WAVE_FORMAT_PCM => pcm_sample_format(format.wBitsPerSample),
        WAVE_FORMAT_IEEE_FLOAT => {
            if format.wBitsPerSample == 32 {
                Ok(MixSampleFormat::Float32)
            } else {
                Err(unusable_format_error())
            }
        }
        WAVE_FORMAT_EXTENSIBLE => mix_sample_format_extensible(format_ptr),
        _ => Err(unusable_format_error()),
    }
}

#[cfg(target_os = "windows")]
fn mix_sample_format_extensible(
    format_ptr: *mut windows::Win32::Media::Audio::WAVEFORMATEX,
) -> Result<MixSampleFormat, AppError> {
    use windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE;

    let format = unsafe { &*(format_ptr as *const WAVEFORMATEXTENSIBLE) };
    let sub_format = unsafe { std::ptr::addr_of!(format.SubFormat).read_unaligned() };
    let bits_per_sample = format.Format.wBitsPerSample;
    let valid_bits_per_sample = unsafe { format.Samples.wValidBitsPerSample };
    if bits_per_sample == 32 && sub_format == ksdataformat_subtype_ieee_float() {
        return Ok(MixSampleFormat::Float32);
    }
    if sub_format == ksdataformat_subtype_pcm() {
        return extensible_pcm_sample_format(valid_bits_per_sample, bits_per_sample);
    }
    Err(unusable_format_error())
}

#[cfg(target_os = "windows")]
fn extensible_pcm_sample_format(
    valid_bits_per_sample: u16,
    bits_per_sample: u16,
) -> Result<MixSampleFormat, AppError> {
    let valid_bits = if valid_bits_per_sample == 0 {
        bits_per_sample
    } else {
        valid_bits_per_sample
    };
    if valid_bits == 0 || valid_bits > bits_per_sample {
        return Err(unusable_format_error());
    }
    if valid_bits == bits_per_sample {
        return pcm_sample_format(bits_per_sample);
    }
    match bits_per_sample {
        16 | 24 | 32 => Ok(MixSampleFormat::PcmSignedExtensible {
            container_bits: bits_per_sample,
            valid_bits,
        }),
        _ => Err(unusable_format_error()),
    }
}

#[cfg(target_os = "windows")]
fn pcm_sample_format(bits_per_sample: u16) -> Result<MixSampleFormat, AppError> {
    match bits_per_sample {
        8 => Ok(MixSampleFormat::PcmUnsigned8),
        16 => Ok(MixSampleFormat::PcmSigned16),
        24 => Ok(MixSampleFormat::PcmSigned24),
        32 => Ok(MixSampleFormat::PcmSigned32),
        _ => Err(unusable_format_error()),
    }
}

#[cfg(target_os = "windows")]
fn ksdataformat_subtype_pcm() -> GUID {
    GUID::from_values(
        0x00000001,
        0x0000,
        0x0010,
        [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    )
}

#[cfg(target_os = "windows")]
fn ksdataformat_subtype_ieee_float() -> GUID {
    GUID::from_values(
        0x00000003,
        0x0000,
        0x0010,
        [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
    )
}

fn left_aligned_extensible_sample_to_f32(
    sample: &[u8],
    container_bits: u16,
    valid_bits: u16,
) -> f32 {
    debug_assert!(valid_bits > 0 && valid_bits < container_bits);
    let container = match container_bits {
        16 => i16::from_le_bytes([sample[0], sample[1]]) as i64,
        24 => {
            let sign = if sample[2] & 0x80 == 0 { 0x00 } else { 0xff };
            i32::from_le_bytes([sample[0], sample[1], sample[2], sign]) as i64
        }
        32 => i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as i64,
        _ => unreachable!("validated extensible PCM container width"),
    };
    let sample = container >> (container_bits - valid_bits);
    sample as f32 / (1_i64 << (valid_bits - 1)) as f32
}

fn unusable_format_error() -> AppError {
    AppError::new(
        "system_audio_unavailable",
        "Default output device reported an unusable audio format.",
    )
}

fn latch_error(stats: &Arc<Mutex<WindowsSystemAudioStats>>, error: &AppError) {
    let Ok(mut stats) = stats.lock() else {
        return;
    };
    stats.last_error = Some(error.message.clone());
}

fn update_stats(
    stats: &Arc<Mutex<WindowsSystemAudioStats>>,
    peak: f32,
    rms: f32,
    bytes_written_delta: i64,
    last_error: Option<String>,
) {
    let Ok(mut stats) = stats.lock() else {
        return;
    };
    stats.max_level = stats.max_level.max(peak);
    stats.bytes_written += bytes_written_delta;
    stats.level = AudioLevelDto {
        peak,
        rms,
        recent_peaks: vec![peak],
    };
    stats.last_error = last_error;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_tail_writes_only_active_silence_to_system_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 1000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&path, spec).unwrap();
        let stats = Arc::new(Mutex::new(WindowsSystemAudioStats::default()));
        let start = Instant::now();
        let state = PauseState::default();
        state.pause(start + Duration::from_secs(2));
        let snapshot = state.snapshot_at(start + Duration::from_secs(102));
        assert!(snapshot.paused);
        let mut silence = ActiveSilence::new(start);
        let frames = silence.pause(snapshot.active_time, spec.sample_rate);
        write_silence(&mut writer, frames, spec.channels, &stats).unwrap();
        let tail = silence.finish(snapshot.active_time, spec.sample_rate);
        assert_eq!(tail, 0);
        writer.finalize().unwrap();
        assert_eq!(hound::WavReader::open(path).unwrap().duration(), 2000);
    }

    #[test]
    fn captured_pause_snapshot_survives_resume_before_wav_padding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system-stereo.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 1000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&path, spec).unwrap();
        let stats = Arc::new(Mutex::new(WindowsSystemAudioStats::default()));
        let start = Instant::now();
        let state = PauseState::default();
        state.pause(start + Duration::from_secs(2));
        let snapshot = state.snapshot_at(start + Duration::from_secs(102));
        assert!(snapshot.paused);
        state.resume(start + Duration::from_secs(102));
        let mut silence = ActiveSilence::new(start);
        let frames = silence.pause(snapshot.active_time, spec.sample_rate);
        write_silence(&mut writer, frames, spec.channels, &stats).unwrap();
        writer.finalize().unwrap();
        let reader = hound::WavReader::open(path).unwrap();
        assert_eq!(reader.duration(), 2000);
        assert_eq!(reader.spec().channels, 2);
    }

    #[test]
    fn sample_conversion_handles_float_and_integer_mix_formats() {
        assert!(
            (sample_to_f32(&1.0_f32.to_le_bytes(), MixSampleFormat::Float32) - 1.0).abs()
                < f32::EPSILON
        );
        assert_eq!(
            sample_to_f32(&0_i16.to_le_bytes(), MixSampleFormat::PcmSigned16),
            0.0
        );
        assert!(
            (sample_to_f32(&i16::MAX.to_le_bytes(), MixSampleFormat::PcmSigned16) - 1.0).abs()
                < 0.0001
        );
        assert!(
            (sample_to_f32(&i32::MAX.to_le_bytes(), MixSampleFormat::PcmSigned32) - 1.0).abs()
                < 0.0001
        );
        assert!(
            (sample_to_f32(&[0xff, 0xff, 0x7f], MixSampleFormat::PcmSigned24) - 1.0).abs() < 0.0001
        );
    }

    #[test]
    fn sample_conversion_handles_left_aligned_extensible_pcm() {
        let cases = [
            (
                "16 valid bits in a 24-bit container",
                &[0x00, 0xff, 0x7f][..],
                MixSampleFormat::PcmSignedExtensible {
                    container_bits: 24,
                    valid_bits: 16,
                },
                1.0,
            ),
            (
                "20 valid bits in a 24-bit container",
                &[0xf0, 0xff, 0x7f][..],
                MixSampleFormat::PcmSignedExtensible {
                    container_bits: 24,
                    valid_bits: 20,
                },
                1.0,
            ),
            (
                "16 valid bits in a 32-bit container",
                &[0x00, 0x00, 0xff, 0x7f][..],
                MixSampleFormat::PcmSignedExtensible {
                    container_bits: 32,
                    valid_bits: 16,
                },
                1.0,
            ),
            (
                "24 valid bits in a 32-bit container",
                &[0x00, 0xff, 0xff, 0x7f][..],
                MixSampleFormat::PcmSignedExtensible {
                    container_bits: 32,
                    valid_bits: 24,
                },
                1.0,
            ),
            (
                "24 valid bits in a 32-bit container at minimum",
                &[0x00, 0x00, 0x00, 0x80][..],
                MixSampleFormat::PcmSignedExtensible {
                    container_bits: 32,
                    valid_bits: 24,
                },
                -1.0,
            ),
        ];

        for (name, bytes, format, expected) in cases {
            assert!(
                (sample_to_f32(bytes, format) - expected).abs() < 0.0001,
                "{name}"
            );
        }
    }

    #[test]
    fn extensible_pcm_format_preserves_container_and_valid_bits() {
        assert_eq!(
            extensible_pcm_sample_format(16, 24).unwrap(),
            MixSampleFormat::PcmSignedExtensible {
                container_bits: 24,
                valid_bits: 16,
            }
        );
        assert_eq!(
            extensible_pcm_sample_format(20, 24).unwrap(),
            MixSampleFormat::PcmSignedExtensible {
                container_bits: 24,
                valid_bits: 20,
            }
        );
        assert_eq!(
            extensible_pcm_sample_format(16, 32).unwrap(),
            MixSampleFormat::PcmSignedExtensible {
                container_bits: 32,
                valid_bits: 16,
            }
        );
        assert_eq!(
            extensible_pcm_sample_format(24, 32).unwrap(),
            MixSampleFormat::PcmSignedExtensible {
                container_bits: 32,
                valid_bits: 24,
            }
        );
        assert_eq!(
            extensible_pcm_sample_format(0, 16).unwrap(),
            MixSampleFormat::PcmSigned16
        );
        assert!(extensible_pcm_sample_format(25, 24).is_err());
    }
}
