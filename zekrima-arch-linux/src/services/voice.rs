//! PipeWire / PulseAudio microphone recorder (`parec`) with real-time RMS level,
//! Push-to-Talk & Hands-Free Energy VAD, and pure-Rust 16 kHz mono WAV encoding.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

const TARGET_SAMPLE_RATE: u32 = 16_000;
const MAX_RECORD_SECS: f32 = 25.0;
const VAD_SPEECH_THRESHOLD: f32 = 0.022;
const VAD_SILENCE_STOP_SECS: f32 = 1.35;
const VAD_NO_SPEECH_TIMEOUT_SECS: f32 = 6.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    /// Records until `stop()` is called (or `MAX_RECORD_SECS`).
    PushToTalk,
    /// Automatically stops after brief silence following detected speech (VAD).
    HandsFreeVad,
}

#[derive(Clone, Debug)]
pub struct RecordingResult {
    pub wav_bytes: Vec<u8>,
    pub duration_secs: f32,
    pub had_speech: bool,
    pub error: Option<String>,
}

pub struct VoiceRecorder {
    stop_flag: Arc<AtomicBool>,
    child_pid: Arc<AtomicU32>,
    level_bits: Arc<AtomicU32>,
    worker: Option<JoinHandle<()>>,
}

impl VoiceRecorder {
    pub fn start<F>(mode: CaptureMode, on_finish: F) -> Self
    where
        F: FnOnce(RecordingResult) + Send + 'static,
    {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let child_pid = Arc::new(AtomicU32::new(0));
        let level_bits = Arc::new(AtomicU32::new(0));

        let w_stop = Arc::clone(&stop_flag);
        let w_pid = Arc::clone(&child_pid);
        let w_level = Arc::clone(&level_bits);

        let worker = thread::Builder::new()
            .name("zekrima-mic".to_owned())
            .spawn(move || {
                let result = record_pipewire_mic(mode, &w_stop, &w_pid, &w_level);
                w_level.store(0.0f32.to_bits(), Ordering::Relaxed);
                on_finish(result);
            })
            .ok();

        Self {
            stop_flag,
            child_pid,
            level_bits,
            worker,
        }
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let pid = self.child_pid.swap(0, Ordering::SeqCst);
        if pid != 0 {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    pub fn live_level(&self) -> f32 {
        f32::from_bits(self.level_bits.load(Ordering::Relaxed)).clamp(0.0, 1.0)
    }
}

impl Drop for VoiceRecorder {
    fn drop(&mut self) {
        self.stop();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

fn record_pipewire_mic(
    mode: CaptureMode,
    stop_flag: &AtomicBool,
    child_pid: &AtomicU32,
    level_bits: &AtomicU32,
) -> RecordingResult {
    // Capture 16 kHz mono signed 16-bit LE directly from default PipeWire mic source.
    let mut child = match Command::new("parec")
        .args([
            "--format=s16le",
            "--rate=16000",
            "--channels=1",
            "--latency-msec=40",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => {
            return RecordingResult {
                wav_bytes: Vec::new(),
                duration_secs: 0.0,
                had_speech: false,
                error: Some(
                    "Mikrofon tidak bisa direkam: perintah `parec` (pipewire-pulse) tidak ditemukan."
                        .to_string(),
                ),
            };
        }
    };

    child_pid.store(child.id(), Ordering::SeqCst);

    let mut samples: Vec<i16> = Vec::with_capacity(TARGET_SAMPLE_RATE as usize * 5);
    let started = Instant::now();
    let mut speech_detected = false;
    let mut last_speech_at = Instant::now();

    if let Some(mut stdout) = child.stdout.take() {
        // 40 ms chunks at 16 kHz * 2 bytes = 1280 bytes
        let mut buf = [0u8; 1280];
        let mut leftover: Option<u8> = None;

        while !stop_flag.load(Ordering::SeqCst) {
            if started.elapsed().as_secs_f32() >= MAX_RECORD_SECS {
                break;
            }

            let n = match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };

            let mut chunk_sq_sum = 0.0f64;
            let mut chunk_count = 0usize;

            let mut idx = 0usize;
            if let Some(prev) = leftover.take() {
                if n > 0 {
                    let s = i16::from_le_bytes([prev, buf[0]]);
                    samples.push(s);
                    let f = s as f64 / 32768.0;
                    chunk_sq_sum += f * f;
                    chunk_count += 1;
                    idx = 1;
                }
            }

            while idx + 1 < n {
                let s = i16::from_le_bytes([buf[idx], buf[idx + 1]]);
                samples.push(s);
                let f = s as f64 / 32768.0;
                chunk_sq_sum += f * f;
                chunk_count += 1;
                idx += 2;
            }
            if idx < n {
                leftover = Some(buf[idx]);
            }

            if chunk_count > 0 {
                let rms = (chunk_sq_sum / chunk_count as f64).sqrt() as f32;
                let normalized = (rms * 5.2).clamp(0.0, 1.0);
                level_bits.store(normalized.to_bits(), Ordering::Relaxed);

                if rms >= VAD_SPEECH_THRESHOLD {
                    speech_detected = true;
                    last_speech_at = Instant::now();
                }
            }

            if mode == CaptureMode::HandsFreeVad {
                if speech_detected {
                    if last_speech_at.elapsed().as_secs_f32() >= VAD_SILENCE_STOP_SECS {
                        break;
                    }
                } else if started.elapsed().as_secs_f32() >= VAD_NO_SPEECH_TIMEOUT_SECS {
                    break;
                }
            }
        }
    }

    child_pid.store(0, Ordering::SeqCst);
    let _ = child.kill();
    let _ = child.wait();

    let duration_secs = samples.len() as f32 / TARGET_SAMPLE_RATE as f32;
    let had_speech = speech_detected || (mode == CaptureMode::PushToTalk && duration_secs >= 0.35);
    let wav_bytes = encode_wav_i16(&samples, TARGET_SAMPLE_RATE);

    RecordingResult {
        wav_bytes,
        duration_secs,
        had_speech,
        error: None,
    }
}

pub fn encode_wav_i16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_bytes = (samples.len() * 2) as u32;
    let riff_size = 36u32.saturating_add(data_bytes);
    let byte_rate = sample_rate * 2;

    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_valid_wav_header_and_payload() {
        let samples = vec![0i16, 1200, -1200, 32767, -32768];
        let wav = encode_wav_i16(&samples, 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + samples.len() * 2);
    }
}
