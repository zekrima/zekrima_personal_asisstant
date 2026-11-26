//! Native Windows WASAPI microphone recorder with real-time RMS level & VAD.
//!
//! Power & CPU design:
//! - Only opens the WASAPI capture endpoint while actively recording.
//! - When idle, zero threads run and CPU usage is strictly 0.0%.
//! - Produces a clean 16 kHz, 16-bit mono WAV payload ready for Groq Whisper,
//!   OpenAI Whisper, or Gemini native audio.
//! - Supports both Push-to-Talk (`Alt + Space`) and Click-to-Talk with automatic
//!   energy-based Voice Activity Detection (auto-stops after ~1.3s of silence
//!   once speech has been heard).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const MAX_RECORD_SECS: u64 = 20;
const VAD_SPEECH_THRESHOLD: f32 = 0.022;
const VAD_SILENCE_STOP_MS: u128 = 1300;
const VAD_NO_SPEECH_TIMEOUT_MS: u128 = 5500;

const WAVE_FORMAT_PCM_TAG: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
const SUBFORMAT_PCM: u32 = 0x0000_0001;
const SUBFORMAT_IEEE_FLOAT: u32 = 0x0000_0003;

/// Result produced when a recording session finishes.
pub struct RecordingResult {
    pub wav_bytes: Vec<u8>,
    pub duration_secs: f32,
    pub had_speech: bool,
    pub error: Option<String>,
}

/// Recording trigger mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    /// Hold `Alt + Space`: records until `stop()` is called (or max 20s).
    PushToTalk,
    /// Click `Mic`: automatically stops after ~1.3s of silence following speech.
    HandsFreeVad,
}

#[derive(Clone, Copy)]
enum SampleKind {
    Float32,
    Pcm16,
}

/// Active microphone recording session. Drop or call `stop()` to finalize.
pub struct VoiceRecorder {
    stop_flag: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    worker: Option<JoinHandle<()>>,
}

impl VoiceRecorder {
    /// Starts capturing microphone audio on a dedicated thread.
    /// Invokes `on_finished` on the capture thread once stopped (manually or by VAD).
    pub fn start<F>(mode: CaptureMode, on_finished: F) -> Self
    where
        F: FnOnce(RecordingResult) + Send + 'static,
    {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0.0f32.to_bits()));

        let worker_stop = Arc::clone(&stop_flag);
        let worker_level = Arc::clone(&level);

        let worker = thread::Builder::new()
            .name("zekrima-mic-capture".to_owned())
            .spawn(move || unsafe {
                let result = capture_loop(mode, worker_stop, worker_level);
                on_finished(result);
            })
            .ok();

        Self {
            stop_flag,
            level,
            worker,
        }
    }

    /// Signals the recorder to finish and deliver the captured WAV.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    /// Current live normalized microphone amplitude (`0.0..=1.0`) for Zeki's waveform.
    pub fn live_level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

impl Drop for VoiceRecorder {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

struct MicSession {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    channels: usize,
    sample_rate: u32,
    sample_kind: SampleKind,
}

impl MicSession {
    unsafe fn open() -> Result<Self, String> {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|_| "Audio enumerator unavailable".to_string())?;

        // Prefer communications mic, fall back to default console capture device
        let device = enumerator
            .GetDefaultAudioEndpoint(eCapture, eCommunications)
            .or_else(|_| enumerator.GetDefaultAudioEndpoint(eCapture, eConsole))
            .map_err(|_| "No microphone detected".to_string())?;

        let client: IAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|_| "Could not activate microphone".to_string())?;

        let mix_ptr: *mut WAVEFORMATEX = client
            .GetMixFormat()
            .map_err(|_| "Could not read microphone format".to_string())?;
        if mix_ptr.is_null() {
            return Err("Invalid microphone format".to_string());
        }

        let mix = *mix_ptr;
        let channels = (mix.nChannels as usize).max(1);
        let sample_rate = mix.nSamplesPerSec.max(8_000);
        let sample_kind = detect_sample_kind(mix_ptr, &mix);

        // 100 ms shared-mode capture buffer (100-ns units)
        let init_res = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            0,
            1_000_000,
            0,
            mix_ptr,
            None,
        );
        CoTaskMemFree(Some(mix_ptr as *const _));
        init_res.map_err(|_| "Microphone access denied or in use".to_string())?;

        let capture: IAudioCaptureClient = client
            .GetService()
            .map_err(|_| "Could not open capture stream".to_string())?;
        client
            .Start()
            .map_err(|_| "Could not start microphone".to_string())?;

        let sample_kind =
            sample_kind.ok_or_else(|| "Unsupported microphone PCM format".to_string())?;

        Ok(Self {
            client,
            capture,
            channels,
            sample_rate,
            sample_kind,
        })
    }

    unsafe fn close(self) {
        let _ = self.client.Stop();
    }
}

unsafe fn detect_sample_kind(ptr: *const WAVEFORMATEX, wf: &WAVEFORMATEX) -> Option<SampleKind> {
    match wf.wFormatTag {
        WAVE_FORMAT_IEEE_FLOAT_TAG if wf.wBitsPerSample == 32 => Some(SampleKind::Float32),
        WAVE_FORMAT_PCM_TAG if wf.wBitsPerSample == 16 => Some(SampleKind::Pcm16),
        WAVE_FORMAT_EXTENSIBLE_TAG if wf.cbSize >= 22 => {
            let subformat_ptr =
                (ptr as *const u8).add(std::mem::size_of::<WAVEFORMATEX>() + 6) as *const u32;
            let data1 = std::ptr::read_unaligned(subformat_ptr);
            match (data1, wf.wBitsPerSample) {
                (SUBFORMAT_IEEE_FLOAT, 32) => Some(SampleKind::Float32),
                (SUBFORMAT_PCM, 16) => Some(SampleKind::Pcm16),
                _ => None,
            }
        }
        _ => None,
    }
}

unsafe fn capture_loop(
    mode: CaptureMode,
    stop_flag: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
) -> RecordingResult {
    let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();

    let session = match MicSession::open() {
        Ok(s) => s,
        Err(err) => {
            if com_ok {
                CoUninitialize();
            }
            return RecordingResult {
                wav_bytes: Vec::new(),
                duration_secs: 0.0,
                had_speech: false,
                error: Some(err),
            };
        }
    };

    let src_rate = session.sample_rate;
    let mut mono_raw: Vec<f32> = Vec::with_capacity(src_rate as usize * 5);
    let started_at = Instant::now();
    let mut speech_detected = false;
    let mut last_speech_at = Instant::now();
    let mut smoothed_level = 0.0f32;
    let mut adaptive_floor = 0.008f32;

    while !stop_flag.load(Ordering::SeqCst) {
        if started_at.elapsed().as_secs() >= MAX_RECORD_SECS {
            break;
        }

        let mut chunk_sq_sum = 0.0f32;
        let mut chunk_samples = 0usize;

        loop {
            let packet_frames = match session.capture.GetNextPacketSize() {
                Ok(n) => n,
                Err(_) => break,
            };
            if packet_frames == 0 {
                break;
            }

            let mut data_ptr: *mut u8 = std::ptr::null_mut();
            let mut frames_read: u32 = 0;
            let mut flags: u32 = 0;
            if session
                .capture
                .GetBuffer(&mut data_ptr, &mut frames_read, &mut flags, None, None)
                .is_err()
            {
                break;
            }

            let silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
            let frames = frames_read as usize;
            let ch = session.channels;

            if silent || data_ptr.is_null() {
                mono_raw.resize(mono_raw.len() + frames, 0.0);
                chunk_samples += frames;
            } else {
                match session.sample_kind {
                    SampleKind::Float32 => {
                        let slice = std::slice::from_raw_parts(data_ptr as *const f32, frames * ch);
                        for frame in slice.chunks_exact(ch) {
                            let sample = frame.iter().copied().sum::<f32>() / ch as f32;
                            mono_raw.push(sample);
                            chunk_sq_sum += sample * sample;
                            chunk_samples += 1;
                        }
                    }
                    SampleKind::Pcm16 => {
                        let slice = std::slice::from_raw_parts(data_ptr as *const i16, frames * ch);
                        for frame in slice.chunks_exact(ch) {
                            let sample =
                                frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / ch as f32;
                            mono_raw.push(sample);
                            chunk_sq_sum += sample * sample;
                            chunk_samples += 1;
                        }
                    }
                }
            }

            let _ = session.capture.ReleaseBuffer(frames_read);
        }

        if chunk_samples > 0 {
            let rms = (chunk_sq_sum / chunk_samples as f32).sqrt();
            // Track gentle ambient noise floor when below speech threshold
            if rms < VAD_SPEECH_THRESHOLD {
                adaptive_floor = adaptive_floor * 0.96 + rms * 0.04;
            }
            let threshold = VAD_SPEECH_THRESHOLD.max(adaptive_floor * 2.4);
            if rms >= threshold {
                speech_detected = true;
                last_speech_at = Instant::now();
            }

            // Visual level for Zeki's animated mouth waveform (0..1)
            let norm = ((rms - adaptive_floor * 0.6).max(0.0) * 9.5)
                .clamp(0.0, 1.0)
                .powf(0.65);
            let alpha = if norm > smoothed_level { 0.55 } else { 0.25 };
            smoothed_level += (norm - smoothed_level) * alpha;
            level.store(smoothed_level.to_bits(), Ordering::Relaxed);
        }

        // Auto-stop VAD only in HandsFreeVad mode
        if mode == CaptureMode::HandsFreeVad {
            if speech_detected && last_speech_at.elapsed().as_millis() >= VAD_SILENCE_STOP_MS {
                break;
            }
            if !speech_detected && started_at.elapsed().as_millis() >= VAD_NO_SPEECH_TIMEOUT_MS {
                break;
            }
        }

        thread::sleep(Duration::from_millis(16));
    }

    level.store(0.0f32.to_bits(), Ordering::Relaxed);
    session.close();
    if com_ok {
        CoUninitialize();
    }

    // Resample to 16 kHz mono and trim leading/trailing dead silence
    let resampled = resample_linear(&mono_raw, src_rate, TARGET_SAMPLE_RATE);
    let trimmed = trim_silence(&resampled, TARGET_SAMPLE_RATE);
    let duration_secs = trimmed.len() as f32 / TARGET_SAMPLE_RATE as f32;
    let wav_bytes = encode_wav_pcm16(trimmed, TARGET_SAMPLE_RATE);

    RecordingResult {
        wav_bytes,
        duration_secs,
        had_speech: speech_detected && duration_secs >= 0.22,
        error: None,
    }
}

fn resample_linear(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if input.is_empty() || src_rate == dst_rate {
        return input.to_vec();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let s0 = input[idx];
        let s1 = input.get(idx + 1).copied().unwrap_or(s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    out
}

fn trim_silence(samples: &[f32], sample_rate: u32) -> &[f32] {
    let thresh = 0.012f32;
    let pad = (sample_rate as usize / 5).min(samples.len()); // 200ms padding
    let first = samples.iter().position(|s| s.abs() >= thresh);
    let last = samples.iter().rposition(|s| s.abs() >= thresh);
    match (first, last) {
        (Some(f), Some(l)) if f <= l => {
            let start = f.saturating_sub(pad);
            let end = (l + 1 + pad).min(samples.len());
            &samples[start..end]
        }
        _ => samples,
    }
}

pub fn encode_wav_pcm16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let riff_len = 36u32.saturating_add(data_len);
    let byte_rate = sample_rate * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format tag
    out.extend_from_slice(&1u16.to_le_bytes()); // Mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // Block align
    out.extend_from_slice(&16u16.to_le_bytes()); // Bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());

    for &s in samples {
        let clamped = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
        out.extend_from_slice(&clamped.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_valid_wav_header_and_payload() {
        let pcm = [0.0f32, 0.5, -0.5, 1.0];
        let wav = encode_wav_pcm16(&pcm, 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + pcm.len() * 2);
    }

    #[test]
    fn linear_resampler_preserves_duration() {
        let src = vec![0.25f32; 48_000];
        let dst = resample_linear(&src, 48_000, 16_000);
        assert_eq!(dst.len(), 16_000);
    }
}
