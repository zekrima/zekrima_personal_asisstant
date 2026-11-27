//! Cicada Mode — Real-Time Live Meeting Translator for Zekrima Assistant (Windows).
//!
//! Architecture (inspired by Cicada & `CIcada_planning.md`):
//! - **Direct System Audio Loopback (No Meeting Bot)**: captures speaker audio via
//!   WASAPI `AUDCLNT_STREAMFLAGS_LOOPBACK` -> 16 kHz mono PCM -> pure-Rust smart
//!   `SpeechSegmenter` (30 ms frames, 10 ms onset check, 100 Hz high-pass rumble
//!   rejection, steady-noise rejection, 0.55 s pause endpointing, 7.5 s quietest-frame
//!   forced cut).
//! - **Dual-Channel Separation & Echo Deduplication**: optional microphone channel
//!   ("You") runs alongside system audio ("Online") with adaptive noise-floor gating
//!   and Cicada's `EchoFilter` (timing gate + bigram overlap) so speaker bleed is
//!   never duplicated.
//! - **Bidirectional Quick-Reply**: hold/click `Reply` to speak in your language
//!   (e.g. Indonesian) and get an instant translation into `cicada_reply_lang`
//!   (e.g. English) ready to read aloud or paste into Zoom/Meet/Teams chat.
//! - **Fast Context-Aware Translation**:
//!   - 1-step Gemini Native Audio (`LANG:` / `ORIGINAL:` / `TRANSLATION:`) when Gemini
//!     is configured, OR Groq/OpenAI Whisper STT + active fast LLM (with optional
//!     Google Translate fallback).
//!   - Sliding-window context (last 4 sentences) + Custom Glossary from Settings.
//! - **0.0% Idle CPU**: zero audio threads run when Cicada Mode is stopped.

use std::collections::{HashSet, VecDeque};
use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eMultimedia, eRender, IAudioCaptureClient, IAudioClient,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_USER};

use crate::services::assistant::{
    base64_encode, effective_llm_provider, extract_json_string_field, http_get_raw, http_post_raw,
    json_escape, parse_url, transcribe_whisper_custom,
};
use crate::services::config::{
    cicada_lang_english_name, cicada_lang_short_badge, AssistantConfig, LlmProvider, SttProvider,
    CICADA_LANGUAGES,
};
use crate::services::voice::{CaptureMode, VoiceRecorder};

/// Posted to the main Dynamic Island window whenever Cicada state or transcripts update.
pub const WM_CICADA_UPDATED: u32 = WM_USER + 230;

const TARGET_RATE: usize = 16_000;
const FRAME_SAMPLES: usize = 480; // 30 ms at 16 kHz
const STEP10_SAMPLES: usize = 160; // 10 ms sub-step for click/onset rejection
const ONSET_STEPS: usize = 3; // 30 ms continuous sound before opening a segment
const FIXED_THRESHOLD: f64 = 0.011; // ~ -39 dBFS
const PAUSE_SECS: f64 = 0.55; // Snappy endpointing (<1.5s latency target)
const MAX_SEGMENT_SECS: f64 = 7.5;
const MIN_SPEECH_SECS: f64 = 0.24;
const PRE_ROLL_SECS: f64 = 0.15;
const TAIL_SECS: f64 = 0.15;
const CUT_WINDOW_SECS: f64 = 2.0;
const MAX_TRANSCRIPT_ENTRIES: usize = 200;

const WAVE_FORMAT_PCM_TAG: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
const SUBFORMAT_PCM: u32 = 0x0000_0001;
const SUBFORMAT_IEEE_FLOAT: u32 = 0x0000_0003;

/// Who spoke a translated line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeakerChannel {
    /// Remote speaker captured via WASAPI System Audio Loopback.
    Online,
    /// Local user / meeting room captured via Microphone (Hybrid mode).
    You,
    /// User's Bidirectional Quick-Reply translated into the meeting's language.
    QuickReply,
}

impl SpeakerChannel {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::You => "You",
            Self::QuickReply => "Reply",
        }
    }
}

/// One bilingual meeting transcript entry.
#[derive(Clone, Debug)]
pub struct TranscriptEntry {
    #[allow(dead_code)]
    pub id: u64,
    pub elapsed_secs: u64,
    pub speaker: SpeakerChannel,
    pub source_lang: String,
    pub target_lang: String,
    pub original: String,
    pub translation: String,
}

impl TranscriptEntry {
    pub fn timestamp_label(&self) -> String {
        let mins = self.elapsed_secs / 60;
        let secs = self.elapsed_secs % 60;
        format!("{mins:02}:{secs:02}")
    }
}

/// Immutable snapshot read by the UI renderer & hit-tester on each frame.
#[derive(Clone, Debug)]
pub struct CicadaSnapshot {
    pub active: bool,
    pub mic_enabled: bool,
    pub quick_reply_recording: bool,
    pub quick_reply_busy: bool,
    pub pending_segments: usize,
    pub detected_lang: Option<String>,
    pub target_lang: String,
    pub reply_lang: String,
    pub show_original: bool,
    pub glossary_count: usize,
    pub entries: Vec<TranscriptEntry>,
    pub last_quick_reply: Option<TranscriptEntry>,
    pub error: Option<String>,
    /// Monotonically incremented whenever a new translated entry arrives (triggers Zeki nod).
    pub arrival_seq: u64,
}

impl Default for CicadaSnapshot {
    fn default() -> Self {
        Self {
            active: false,
            mic_enabled: false,
            quick_reply_recording: false,
            quick_reply_busy: false,
            pending_segments: 0,
            detected_lang: None,
            target_lang: "id".to_string(),
            reply_lang: "en".to_string(),
            show_original: true,
            glossary_count: 0,
            entries: Vec::new(),
            last_quick_reply: None,
            error: None,
            arrival_seq: 0,
        }
    }
}

impl CicadaSnapshot {
    pub fn lang_pair_badge(&self) -> String {
        let src = self
            .detected_lang
            .as_deref()
            .map(cicada_lang_short_badge)
            .unwrap_or("AUTO");
        let dst = cicada_lang_short_badge(&self.target_lang);
        format!("{src}→{dst}")
    }

    pub fn has_entries(&self) -> bool {
        !self.entries.is_empty()
    }
}

// =============================================================================
// CicadaService — manages background capture threads & translation workers
// =============================================================================

struct SharedCicada {
    hwnd_raw: isize,
    snapshot: Mutex<CicadaSnapshot>,
    started_at: Mutex<Instant>,
    next_id: AtomicU64,
    generation: AtomicU64,
    online_level: AtomicU32,
    mic_level: AtomicU32,
    echo_filter: Mutex<EchoFilter>,
    /// Recently translated sentences (for sliding-window context).
    recent_context: Mutex<VecDeque<String>>,
}

impl SharedCicada {
    fn notify_ui(&self) {
        if self.hwnd_raw != 0 {
            unsafe {
                let _ = PostMessageW(
                    HWND(self.hwnd_raw as *mut c_void),
                    WM_CICADA_UPDATED,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    fn update(&self, f: impl FnOnce(&mut CicadaSnapshot)) {
        if let Ok(mut snap) = self.snapshot.lock() {
            f(&mut snap);
        }
        self.notify_ui();
    }

    fn elapsed_secs(&self) -> u64 {
        self.started_at
            .lock()
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }

    fn push_context(&self, original: &str, translation: &str) {
        if let Ok(mut ctx) = self.recent_context.lock() {
            ctx.push_back(format!("{original} => {translation}"));
            while ctx.len() > 4 {
                ctx.pop_front();
            }
        }
    }

    fn get_context(&self) -> Vec<String> {
        self.recent_context
            .lock()
            .map(|ctx| ctx.iter().cloned().collect())
            .unwrap_or_default()
    }
}

struct CaptureThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl CaptureThread {
    fn stop_async(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = thread::Builder::new()
                .name("zekrima-cicada-join".to_owned())
                .spawn(move || {
                    let _ = h.join();
                });
        }
    }
}

impl Drop for CaptureThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub struct CicadaService {
    shared: Arc<SharedCicada>,
    loopback_worker: Mutex<Option<CaptureThread>>,
    mic_worker: Mutex<Option<CaptureThread>>,
    quick_reply_rec: Mutex<Option<VoiceRecorder>>,
}

impl CicadaService {
    pub fn start(hwnd: HWND) -> Self {
        let cfg = AssistantConfig::load();
        let initial = CicadaSnapshot {
            target_lang: cfg.cicada_target_lang.clone(),
            reply_lang: cfg.cicada_reply_lang.clone(),
            show_original: cfg.cicada_show_original,
            glossary_count: parse_glossary(&cfg.cicada_glossary).len(),
            ..Default::default()
        };
        Self {
            shared: Arc::new(SharedCicada {
                hwnd_raw: hwnd.0 as isize,
                snapshot: Mutex::new(initial),
                started_at: Mutex::new(Instant::now()),
                next_id: AtomicU64::new(1),
                generation: AtomicU64::new(0),
                online_level: AtomicU32::new(0.0f32.to_bits()),
                mic_level: AtomicU32::new(0.0f32.to_bits()),
                echo_filter: Mutex::new(EchoFilter::new()),
                recent_context: Mutex::new(VecDeque::new()),
            }),
            loopback_worker: Mutex::new(None),
            mic_worker: Mutex::new(None),
            quick_reply_rec: Mutex::new(None),
        }
    }

    pub fn snapshot(&self) -> CicadaSnapshot {
        self.shared
            .snapshot
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub fn is_active(&self) -> bool {
        self.shared
            .snapshot
            .lock()
            .map(|s| s.active)
            .unwrap_or(false)
    }

    /// Combined live normalized audio level (`0.0..=1.0`) for Zeki's headset & waveform.
    pub fn live_audio_level(&self) -> f32 {
        let a = f32::from_bits(self.shared.online_level.load(Ordering::Relaxed));
        let b = f32::from_bits(self.shared.mic_level.load(Ordering::Relaxed));
        let qr = self
            .quick_reply_rec
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.live_level()))
            .unwrap_or(0.0);
        a.max(b).max(qr).clamp(0.0, 1.0)
    }

    /// Reloads target/reply language, glossary, and subtitle options from `assistant.ini`.
    pub fn refresh_from_config(&self) {
        let cfg = AssistantConfig::load();
        self.shared.update(|s| {
            s.target_lang = cfg.cicada_target_lang.clone();
            s.reply_lang = cfg.cicada_reply_lang.clone();
            s.show_original = cfg.cicada_show_original;
            s.glossary_count = parse_glossary(&cfg.cicada_glossary).len();
        });
    }

    /// Starts Live Meeting Translation (WASAPI System Audio Loopback + optional Hybrid Mic).
    pub fn start_listening(&self) {
        if self.is_active() {
            return;
        }
        let cfg = AssistantConfig::load();
        let gen = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut ts) = self.shared.started_at.lock() {
            *ts = Instant::now();
        }
        if let Ok(mut ef) = self.shared.echo_filter.lock() {
            ef.reset();
        }

        let want_mic = self
            .shared
            .snapshot
            .lock()
            .map(|s| s.mic_enabled)
            .unwrap_or(false);

        self.shared.update(|s| {
            s.active = true;
            s.pending_segments = 0;
            s.error = None;
            s.target_lang = cfg.cicada_target_lang.clone();
            s.reply_lang = cfg.cicada_reply_lang.clone();
            s.show_original = cfg.cicada_show_original;
            s.glossary_count = parse_glossary(&cfg.cicada_glossary).len();
        });

        self.spawn_loopback(gen);
        if want_mic {
            self.spawn_mic(gen);
        }
    }

    /// Stops all live audio capture threads cleanly (returns to 0.0% idle CPU).
    pub fn stop_listening(&self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut slot) = self.loopback_worker.lock() {
            if let Some(w) = slot.take() {
                w.stop_async();
            }
        }
        if let Ok(mut slot) = self.mic_worker.lock() {
            if let Some(w) = slot.take() {
                w.stop_async();
            }
        }
        self.shared
            .online_level
            .store(0.0f32.to_bits(), Ordering::Relaxed);
        self.shared
            .mic_level
            .store(0.0f32.to_bits(), Ordering::Relaxed);

        self.shared.update(|s| {
            s.active = false;
            s.pending_segments = 0;
        });
    }

    pub fn toggle_listening(&self) {
        if self.is_active() {
            self.stop_listening();
        } else {
            self.start_listening();
        }
    }

    /// Toggles Dual-Channel Hybrid Microphone capture ("You") live without stopping the meeting.
    pub fn toggle_mic(&self) {
        let mut enabled = false;
        self.shared.update(|s| {
            s.mic_enabled = !s.mic_enabled;
            enabled = s.mic_enabled;
        });
        if self.is_active() {
            if enabled {
                let gen = self.shared.generation.load(Ordering::SeqCst);
                self.spawn_mic(gen);
            } else {
                if let Ok(mut slot) = self.mic_worker.lock() {
                    if let Some(w) = slot.take() {
                        w.stop_async();
                    }
                }
                self.shared
                    .mic_level
                    .store(0.0f32.to_bits(), Ordering::Relaxed);
            }
        }
    }

    /// Cycles the target translation language (`id -> en -> ja -> zh-CN -> ko -> ...`) and saves.
    pub fn cycle_target_language(&self) {
        let mut cfg = AssistantConfig::load();
        let cur = CICADA_LANGUAGES
            .iter()
            .position(|(code, _)| code.eq_ignore_ascii_case(&cfg.cicada_target_lang))
            .unwrap_or(0);
        let next_idx = (cur + 1) % CICADA_LANGUAGES.len();
        let next_code = CICADA_LANGUAGES[next_idx].0.to_string();
        cfg.cicada_target_lang = next_code.clone();
        let _ = cfg.save();

        self.shared.update(|s| {
            s.target_lang = next_code;
        });
    }

    /// Toggles Bilingual (Original + Translation) vs Translation-Only subtitle mode and saves.
    pub fn toggle_show_original(&self) {
        let mut cfg = AssistantConfig::load();
        cfg.cicada_show_original = !cfg.cicada_show_original;
        let next = cfg.cicada_show_original;
        let _ = cfg.save();
        self.shared.update(|s| {
            s.show_original = next;
        });
    }

    /// Clears the current meeting transcript history.
    pub fn clear_transcript(&self) {
        if let Ok(mut ctx) = self.shared.recent_context.lock() {
            ctx.clear();
        }
        self.shared.update(|s| {
            s.entries.clear();
            s.last_quick_reply = None;
            s.detected_lang = None;
            s.error = None;
        });
    }

    /// Bidirectional Quick-Reply: click once to speak in your language (auto-stops after
    /// pause via VAD, or click again to stop immediately) and translate into `cicada_reply_lang`.
    pub fn toggle_quick_reply(&self) {
        // If already recording Quick-Reply, stop now so it translates immediately.
        if let Ok(mut slot) = self.quick_reply_rec.lock() {
            if let Some(rec) = slot.take() {
                rec.stop();
                let _ = thread::Builder::new()
                    .name("zekrima-qr-stop".to_owned())
                    .spawn(move || drop(rec));
                self.shared.update(|s| {
                    s.quick_reply_recording = false;
                });
                return;
            }
        }

        let cfg = AssistantConfig::load();
        let reply_lang = cfg.cicada_reply_lang.clone();
        self.shared.update(|s| {
            s.quick_reply_recording = true;
            s.quick_reply_busy = false;
            s.error = None;
        });

        let shared = Arc::clone(&self.shared);
        let rec = VoiceRecorder::start(CaptureMode::HandsFreeVad, move |result| {
            shared.update(|s| {
                s.quick_reply_recording = false;
                s.quick_reply_busy = true;
            });

            if let Some(err) = result.error {
                shared.update(|s| {
                    s.quick_reply_busy = false;
                    s.error = Some(err);
                });
                return;
            }
            if !result.had_speech || result.wav_bytes.len() <= 44 {
                shared.update(|s| {
                    s.quick_reply_busy = false;
                    s.error = Some("Quick-Reply: tidak ada suara terdeteksi.".to_string());
                });
                return;
            }

            let cfg = AssistantConfig::load();
            let glossary = parse_glossary(&cfg.cicada_glossary);
            let ctx = shared.get_context();
            let res = hear_and_translate(
                &cfg,
                &result.wav_bytes,
                result.duration_secs as f64,
                &reply_lang,
                None,
                &ctx,
                &glossary,
            );

            match res {
                Ok(Some(heard)) => {
                    let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
                    let entry = TranscriptEntry {
                        id,
                        elapsed_secs: shared.elapsed_secs(),
                        speaker: SpeakerChannel::QuickReply,
                        source_lang: heard.lang,
                        target_lang: reply_lang.clone(),
                        original: heard.original.clone(),
                        translation: heard.translation.clone(),
                    };
                    shared.push_context(&entry.original, &entry.translation);
                    // Copy Quick-Reply translation directly to Windows clipboard for instant paste
                    crate::services::clipboard::set_clipboard_text(&entry.translation);

                    shared.update(|s| {
                        s.quick_reply_busy = false;
                        s.error = None;
                        s.last_quick_reply = Some(entry.clone());
                        s.entries.push(entry);
                        if s.entries.len() > MAX_TRANSCRIPT_ENTRIES {
                            let excess = s.entries.len() - MAX_TRANSCRIPT_ENTRIES;
                            s.entries.drain(0..excess);
                        }
                        s.arrival_seq = s.arrival_seq.saturating_add(1);
                    });
                }
                Ok(None) => {
                    shared.update(|s| {
                        s.quick_reply_busy = false;
                    });
                }
                Err(err) => {
                    shared.update(|s| {
                        s.quick_reply_busy = false;
                        s.error = Some(err);
                    });
                }
            }
        });

        if let Ok(mut slot) = self.quick_reply_rec.lock() {
            *slot = Some(rec);
        }
    }

    /// Formats the full bilingual transcript as Markdown (ready to copy or save).
    pub fn format_markdown(&self) -> String {
        let snap = self.snapshot();
        if snap.entries.is_empty() {
            return String::new();
        }
        let mut out = String::from("# Zekrima Cicada — Live Meeting Transcript\n\n");
        for e in &snap.entries {
            out.push_str(&format!(
                "- **[{}] ({} · {}→{})** {}\n",
                e.timestamp_label(),
                e.speaker.tag(),
                cicada_lang_short_badge(&e.source_lang),
                cicada_lang_short_badge(&e.target_lang),
                e.translation
            ));
            if !e.original.eq_ignore_ascii_case(&e.translation) {
                out.push_str(&format!("  - *Original:* {}\n", e.original));
            }
        }
        out
    }

    /// Saves the current transcript to `%APPDATA%\Zekrima\meetings\meeting-<ts>.md`.
    pub fn save_markdown_file(&self) -> Option<PathBuf> {
        let md = self.format_markdown();
        if md.is_empty() {
            return None;
        }
        let dir = meetings_dir()?;
        let _ = fs::create_dir_all(&dir);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("meeting-{ts}.md"));
        fs::write(&path, md).ok()?;
        Some(path)
    }

    /// Compact transcript context injected into Zekrima Assistant for Live Q&A.
    pub fn build_qa_context(&self) -> Option<String> {
        let snap = self.snapshot();
        if snap.entries.is_empty() {
            return None;
        }
        // Keep the latest 45 lines so context stays fast and relevant
        let start = snap.entries.len().saturating_sub(45);
        let mut out = String::new();
        for e in &snap.entries[start..] {
            out.push_str(&format!(
                "[{}] {}: {} (Original: {})\n",
                e.timestamp_label(),
                e.speaker.tag(),
                e.translation,
                e.original
            ));
        }
        Some(out)
    }

    /// Builds a structured Auto-Summary prompt to send directly to Zekrima Assistant.
    pub fn build_summary_prompt(&self) -> Option<String> {
        let snap = self.snapshot();
        if snap.entries.is_empty() {
            return None;
        }
        Some(
            "Buat ringkasan rapat dari transkrip Cicada Mode yang terlampir dalam format Markdown singkat dan rapi:\n\
             1. **Ringkasan Singkat** (2-3 kalimat)\n\
             2. **Poin Utama & Keputusan**\n\
             3. **Action Items / Tugas Lanjutan**"
                .to_string(),
        )
    }

    fn spawn_loopback(&self, gen: u64) {
        let stop = Arc::new(AtomicBool::new(false));
        let w_stop = Arc::clone(&stop);
        let shared = Arc::clone(&self.shared);
        let handle = thread::Builder::new()
            .name("zekrima-cicada-loopback".to_owned())
            .spawn(move || unsafe {
                run_wasapi_channel(SpeakerChannel::Online, gen, w_stop, shared);
            })
            .ok();
        if let Ok(mut slot) = self.loopback_worker.lock() {
            if let Some(old) = slot.replace(CaptureThread { stop, handle }) {
                old.stop_async();
            }
        }
    }

    fn spawn_mic(&self, gen: u64) {
        let stop = Arc::new(AtomicBool::new(false));
        let w_stop = Arc::clone(&stop);
        let shared = Arc::clone(&self.shared);
        let handle = thread::Builder::new()
            .name("zekrima-cicada-mic".to_owned())
            .spawn(move || unsafe {
                run_wasapi_channel(SpeakerChannel::You, gen, w_stop, shared);
            })
            .ok();
        if let Ok(mut slot) = self.mic_worker.lock() {
            if let Some(old) = slot.replace(CaptureThread { stop, handle }) {
                old.stop_async();
            }
        }
    }
}

impl Drop for CicadaService {
    fn drop(&mut self) {
        self.stop_listening();
    }
}

fn meetings_dir() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(PathBuf::from(appdata).join("Zekrima").join("meetings"));
    }
    None
}

// =============================================================================
// WASAPI Dual-Channel Capture Loop (Loopback for Online, Capture for You)
// =============================================================================

#[derive(Clone, Copy)]
enum SampleKind {
    Float32,
    Pcm16,
}

struct WasapiStream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    channels: usize,
    sample_rate: u32,
    sample_kind: SampleKind,
}

impl WasapiStream {
    unsafe fn open(is_loopback: bool) -> Result<Self, String> {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|_| "Windows audio enumerator tidak tersedia.".to_string())?;

        let device = if is_loopback {
            enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(|_| "Speaker default tidak ditemukan.".to_string())?
        } else {
            enumerator
                .GetDefaultAudioEndpoint(eCapture, eCommunications)
                .or_else(|_| enumerator.GetDefaultAudioEndpoint(eCapture, eConsole))
                .map_err(|_| "Mikrofon tidak ditemukan.".to_string())?
        };

        let client: IAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|_| "Gagal mengaktifkan perangkat audio.".to_string())?;

        let mix_ptr: *mut WAVEFORMATEX = client
            .GetMixFormat()
            .map_err(|_| "Gagal membaca format audio.".to_string())?;
        if mix_ptr.is_null() {
            return Err("Format audio tidak valid.".to_string());
        }

        let mix = *mix_ptr;
        let channels = (mix.nChannels as usize).max(1);
        let sample_rate = mix.nSamplesPerSec.max(8_000);
        let sample_kind = detect_sample_kind(mix_ptr, &mix);

        let flags = if is_loopback {
            AUDCLNT_STREAMFLAGS_LOOPBACK
        } else {
            0
        };
        // 200 ms shared-mode buffer (in 100-ns units)
        let init_res = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            flags,
            2_000_000,
            0,
            mix_ptr,
            None,
        );
        CoTaskMemFree(Some(mix_ptr as *const _));
        init_res.map_err(|_| {
            if is_loopback {
                "Gagal membuka WASAPI Loopback speaker.".to_string()
            } else {
                "Akses mikrofon ditolak atau sedang dipakai.".to_string()
            }
        })?;

        let capture: IAudioCaptureClient = client
            .GetService()
            .map_err(|_| "Gagal membuka capture client.".to_string())?;
        client
            .Start()
            .map_err(|_| "Gagal memulai audio stream.".to_string())?;

        let sample_kind =
            sample_kind.ok_or_else(|| "Format PCM perangkat audio tidak didukung.".to_string())?;

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

unsafe fn run_wasapi_channel(
    channel: SpeakerChannel,
    gen: u64,
    stop: Arc<AtomicBool>,
    shared: Arc<SharedCicada>,
) {
    let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
    let is_loopback = channel == SpeakerChannel::Online;

    let stream = match WasapiStream::open(is_loopback) {
        Ok(s) => s,
        Err(err) => {
            shared.update(|s| {
                s.error = Some(err);
                if is_loopback {
                    s.active = false;
                }
            });
            if com_ok {
                CoUninitialize();
            }
            return;
        }
    };

    let mut segmenter = SpeechSegmenter::new(!is_loopback);
    let mut mono_raw: Vec<f32> = Vec::with_capacity(4096);
    let mut smoothed_level = 0.0f32;
    let mut last_packet = Instant::now();
    let mut last_silence_fed = Instant::now();

    while !stop.load(Ordering::SeqCst)
        && shared.generation.load(Ordering::SeqCst) == gen
    {
        mono_raw.clear();
        let mut got_packet = false;

        loop {
            let packet_frames = match stream.capture.GetNextPacketSize() {
                Ok(n) => n,
                Err(_) => break,
            };
            if packet_frames == 0 {
                break;
            }

            let mut data_ptr: *mut u8 = std::ptr::null_mut();
            let mut frames_read: u32 = 0;
            let mut flags: u32 = 0;
            if stream
                .capture
                .GetBuffer(&mut data_ptr, &mut frames_read, &mut flags, None, None)
                .is_err()
            {
                break;
            }

            let silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
            let frames = frames_read as usize;
            let ch = stream.channels;
            if silent || data_ptr.is_null() {
                mono_raw.resize(mono_raw.len() + frames, 0.0);
            } else {
                match stream.sample_kind {
                    SampleKind::Float32 => {
                        let slice = std::slice::from_raw_parts(data_ptr as *const f32, frames * ch);
                        for frame in slice.chunks_exact(ch) {
                            let sample = frame.iter().copied().sum::<f32>() / ch as f32;
                            mono_raw.push(sample);
                        }
                    }
                    SampleKind::Pcm16 => {
                        let slice = std::slice::from_raw_parts(data_ptr as *const i16, frames * ch);
                        for frame in slice.chunks_exact(ch) {
                            let sample =
                                frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / ch as f32;
                            mono_raw.push(sample);
                        }
                    }
                }
            }
            let _ = stream.capture.ReleaseBuffer(frames_read);
            got_packet = true;
        }

        let pcm16: Vec<i16> = if got_packet && !mono_raw.is_empty() {
            last_packet = Instant::now();
            last_silence_fed = last_packet;
            resample_to_i16(&mono_raw, stream.sample_rate, TARGET_RATE as u32)
        } else if is_loopback && last_packet.elapsed() >= Duration::from_millis(350) {
            // WASAPI loopback stops emitting packets during complete silence; feed silence
            // frames so the segmenter cleanly closes the active phrase!
            let dt = last_silence_fed.elapsed().as_secs_f64().min(0.2);
            if dt >= 0.03 {
                last_silence_fed = Instant::now();
                vec![0i16; (dt * TARGET_RATE as f64) as usize]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        if !pcm16.is_empty() {
            let rms = rms_i16(&pcm16);
            let norm = (rms as f32 * 8.5).clamp(0.0, 1.0).powf(0.65);
            let alpha = if norm > smoothed_level { 0.55 } else { 0.22 };
            smoothed_level += (norm - smoothed_level) * alpha;
            if is_loopback {
                shared
                    .online_level
                    .store(smoothed_level.to_bits(), Ordering::Relaxed);
                if rms >= FIXED_THRESHOLD {
                    if let Ok(mut ef) = shared.echo_filter.lock() {
                        ef.mark_online_loud();
                    }
                }
            } else {
                shared
                    .mic_level
                    .store(smoothed_level.to_bits(), Ordering::Relaxed);
            }

            let emitted = segmenter.feed(&pcm16);
            for seg in emitted {
                dispatch_segment(channel, gen, seg, Arc::clone(&shared));
            }
        } else {
            smoothed_level *= 0.82;
            if smoothed_level < 0.01 {
                smoothed_level = 0.0;
            }
            if is_loopback {
                shared
                    .online_level
                    .store(smoothed_level.to_bits(), Ordering::Relaxed);
            } else {
                shared
                    .mic_level
                    .store(smoothed_level.to_bits(), Ordering::Relaxed);
            }
        }

        thread::sleep(Duration::from_millis(20));
    }

    if let Some(seg) = segmenter.flush() {
        if shared.generation.load(Ordering::SeqCst) == gen {
            dispatch_segment(channel, gen, seg, Arc::clone(&shared));
        }
    }

    stream.close();
    if com_ok {
        CoUninitialize();
    }
}

fn dispatch_segment(
    channel: SpeakerChannel,
    gen: u64,
    seg: SpeechSegment,
    shared: Arc<SharedCicada>,
) {
    let seg_end_secs = shared.elapsed_secs();
    let seg_dur = seg.samples.len() as f64 / TARGET_RATE as f64;
    let wav = encode_wav_pcm16(&seg.samples, TARGET_RATE as u32);
    let wall_now = Instant::now();

    shared.update(|s| {
        s.pending_segments = s.pending_segments.saturating_add(1);
    });

    let _ = thread::Builder::new()
        .name("zekrima-cicada-tx".to_owned())
        .spawn(move || {
            let cfg = AssistantConfig::load();
            let target_lang = cfg.cicada_target_lang.clone();
            let hint = shared
                .snapshot
                .lock()
                .ok()
                .and_then(|s| s.detected_lang.clone());
            let glossary = parse_glossary(&cfg.cicada_glossary);
            let ctx = shared.get_context();

            let result = hear_and_translate(
                &cfg,
                &wav,
                seg_dur,
                &target_lang,
                hint.as_deref(),
                &ctx,
                &glossary,
            );

            if shared.generation.load(Ordering::SeqCst) != gen {
                shared.update(|s| {
                    s.pending_segments = s.pending_segments.saturating_sub(1);
                });
                return;
            }

            match result {
                Ok(Some(heard)) => {
                    // Echo deduplication for Dual-Channel Hybrid mic
                    if channel == SpeakerChannel::You {
                        let is_echo = shared
                            .echo_filter
                            .lock()
                            .map(|ef| ef.is_echo(&heard.original, wall_now))
                            .unwrap_or(false);
                        if is_echo {
                            shared.update(|s| {
                                s.pending_segments = s.pending_segments.saturating_sub(1);
                            });
                            return;
                        }
                    } else if channel == SpeakerChannel::Online {
                        if let Ok(mut ef) = shared.echo_filter.lock() {
                            ef.record_online(&heard.original, wall_now);
                        }
                    }

                    let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
                    let entry = TranscriptEntry {
                        id,
                        elapsed_secs: seg_end_secs,
                        speaker: channel,
                        source_lang: heard.lang.clone(),
                        target_lang: target_lang.clone(),
                        original: heard.original.clone(),
                        translation: heard.translation.clone(),
                    };
                    shared.push_context(&entry.original, &entry.translation);

                    shared.update(|s| {
                        s.pending_segments = s.pending_segments.saturating_sub(1);
                        s.error = None;
                        if channel == SpeakerChannel::Online {
                            s.detected_lang = Some(heard.lang);
                        }
                        s.entries.push(entry);
                        if s.entries.len() > MAX_TRANSCRIPT_ENTRIES {
                            let excess = s.entries.len() - MAX_TRANSCRIPT_ENTRIES;
                            s.entries.drain(0..excess);
                        }
                        s.arrival_seq = s.arrival_seq.saturating_add(1);
                    });
                }
                Ok(None) => {
                    shared.update(|s| {
                        s.pending_segments = s.pending_segments.saturating_sub(1);
                    });
                }
                Err(err) => {
                    shared.update(|s| {
                        s.pending_segments = s.pending_segments.saturating_sub(1);
                        s.error = Some(err);
                    });
                }
            }
        });
}

// =============================================================================
// Smart Pure-Rust Speech Segmenter (ported from Cicada `Segmenter.cs`)
// =============================================================================

#[derive(Debug)]
struct SpeechSegment {
    samples: Vec<i16>,
}

struct SpeechSegmenter {
    adaptive: bool,
    noise_floor: f64,
    highpass_a: f64,
    hx: f64,
    hy: f64,
    pending: Vec<i16>,
    current: Vec<i16>,
    levels: Vec<f64>,
    highs: Vec<f64>,
    speech: Vec<bool>,
    before: VecDeque<(Vec<i16>, f64, f64)>,
    quiet_frames: usize,
    loud_frames: usize,
    sounding_steps: usize,
}

impl SpeechSegmenter {
    fn new(adaptive: bool) -> Self {
        let highpass_a = 1.0 / (1.0 + 2.0 * std::f64::consts::PI * 100.0 / TARGET_RATE as f64);
        Self {
            adaptive,
            noise_floor: 0.008,
            highpass_a,
            hx: 0.0,
            hy: 0.0,
            pending: Vec::with_capacity(FRAME_SAMPLES * 4),
            current: Vec::with_capacity(TARGET_RATE * 4),
            levels: Vec::new(),
            highs: Vec::new(),
            speech: Vec::new(),
            before: VecDeque::new(),
            quiet_frames: 0,
            loud_frames: 0,
            sounding_steps: 0,
        }
    }

    fn feed(&mut self, samples: &[i16]) -> Vec<SpeechSegment> {
        self.pending.extend_from_slice(samples);
        let mut out = Vec::new();
        let whole = (self.pending.len() / FRAME_SAMPLES) * FRAME_SAMPLES;
        if whole == 0 {
            return out;
        }
        let frames: Vec<i16> = self.pending.drain(..whole).collect();
        for chunk in frames.chunks_exact(FRAME_SAMPLES) {
            if let Some(seg) = self.step(chunk) {
                out.push(seg);
            }
        }
        out
    }

    fn flush(&mut self) -> Option<SpeechSegment> {
        if self.current.is_empty() {
            return None;
        }
        let end = self
            .current
            .len()
            .saturating_sub(self.quiet_frames * FRAME_SAMPLES)
            + (TAIL_SECS * TARGET_RATE as f64) as usize;
        self.close(end)
    }

    fn gate_threshold(&self) -> f64 {
        if self.adaptive {
            (self.noise_floor * 2.6).max(0.015)
        } else {
            FIXED_THRESHOLD
        }
    }

    fn highpass_rms(&mut self, frame: &[i16]) -> f64 {
        let a = self.highpass_a;
        let mut x0 = self.hx;
        let mut y = self.hy;
        let mut sum = 0.0f64;
        for &s in frame {
            let x = s as f64;
            y = a * (y + x - x0);
            x0 = x;
            sum += y * y;
        }
        self.hx = x0;
        self.hy = y;
        (sum / frame.len().max(1) as f64).sqrt() / 32768.0
    }

    fn is_speech(&mut self, frame: &[i16], level: f64) -> bool {
        let line = self.gate_threshold();
        if self.adaptive {
            if level < line {
                self.noise_floor = self.noise_floor * 0.96 + level * 0.04;
            }
            let on = level >= line;
            if on {
                self.sounding_steps += FRAME_SAMPLES / STEP10_SAMPLES;
            } else {
                self.sounding_steps = 0;
            }
            return on;
        }

        // 10 ms sub-step onset gate rejects isolated keyboard clicks
        let mut said = false;
        for step in frame.chunks(STEP10_SAMPLES) {
            let step_rms = rms_i16(step);
            if step_rms >= line {
                self.sounding_steps += 1;
                if self.sounding_steps >= ONSET_STEPS {
                    said = true;
                }
            } else {
                self.sounding_steps = 0;
            }
        }
        said
    }

    fn step(&mut self, frame: &[i16]) -> Option<SpeechSegment> {
        let level = rms_i16(frame);
        let high = self.highpass_rms(frame);
        let loud = self.is_speech(frame, level);
        let pre_roll_frames = (PRE_ROLL_SECS * TARGET_RATE as f64 / FRAME_SAMPLES as f64) as usize;

        if self.current.is_empty() {
            if !loud {
                self.before.push_back((frame.to_vec(), level, high));
                while self.before.len() > pre_roll_frames + 1 {
                    self.before.pop_front();
                }
                return None;
            }
            while let Some((f, l, h)) = self.before.pop_front() {
                self.current.extend_from_slice(&f);
                self.levels.push(l);
                self.highs.push(h);
                self.speech.push(false);
            }
            self.quiet_frames = 0;
            self.loud_frames = 0;
        }

        self.current.extend_from_slice(frame);
        self.levels.push(level);
        self.highs.push(high);
        self.speech.push(loud);
        if loud {
            self.quiet_frames = 0;
            self.loud_frames += 1;
        } else {
            self.quiet_frames += 1;
        }

        if (self.quiet_frames * FRAME_SAMPLES) as f64 >= PAUSE_SECS * TARGET_RATE as f64 {
            let end = self
                .current
                .len()
                .saturating_sub(self.quiet_frames * FRAME_SAMPLES)
                + (TAIL_SECS * TARGET_RATE as f64) as usize;
            return self.close(end);
        }

        if self.current.len() as f64 >= MAX_SEGMENT_SECS * TARGET_RATE as f64 {
            let window_frames = (CUT_WINDOW_SECS * TARGET_RATE as f64 / FRAME_SAMPLES as f64) as usize;
            let from = self.levels.len().saturating_sub(window_frames).max(1);
            let mut cut = from;
            for i in from..self.levels.len() {
                if self.levels[i] < self.levels[cut] {
                    cut = i;
                }
            }
            return self.close(cut * FRAME_SAMPLES);
        }

        None
    }

    fn close(&mut self, end_raw: usize) -> Option<SpeechSegment> {
        let end = end_raw.min(self.current.len());
        let cut_frames = (end / FRAME_SAMPLES).min(self.levels.len());
        let samples: Vec<i16> = self.current[..end].to_vec();
        let rest: Vec<i16> = self.current[end..].to_vec();
        let rest_levels: Vec<f64> = self.levels[cut_frames..].to_vec();
        let rest_highs: Vec<f64> = self.highs[cut_frames..].to_vec();
        let rest_speech: Vec<bool> = self.speech[cut_frames..].to_vec();

        let line = self.gate_threshold();
        let sent_levels = &self.levels[..cut_frames];
        let sent_highs = &self.highs[..cut_frames];

        let enough_speech =
            (self.loud_frames * FRAME_SAMPLES) as f64 >= MIN_SPEECH_SECS * TARGET_RATE as f64;
        let valid = enough_speech
            && !is_steady_noise(sent_levels, line)
            && !is_low_rumble(sent_levels, sent_highs, line);

        self.current.clear();
        self.levels.clear();
        self.highs.clear();
        self.speech.clear();
        self.quiet_frames = 0;
        self.loud_frames = 0;

        if rest_speech.iter().any(|&v| v) {
            self.current = rest;
            self.levels = rest_levels;
            self.highs = rest_highs;
            self.speech = rest_speech;
            self.quiet_frames = self.speech.iter().rev().take_while(|&&v| !v).count();
            self.loud_frames = self.speech.iter().filter(|&&v| v).count();
        }

        if valid && samples.len() >= (MIN_SPEECH_SECS * TARGET_RATE as f64) as usize {
            Some(SpeechSegment { samples })
        } else {
            None
        }
    }
}

/// Steady fan/hum rejection (90th / 10th percentile < 2.0 and no syllable dips).
fn is_steady_noise(levels: &[f64], gate: f64) -> bool {
    let first = match levels.iter().position(|&l| l >= gate) {
        Some(i) => i,
        None => return true,
    };
    let last = match levels.iter().rposition(|&l| l >= gate) {
        Some(i) => i,
        None => return true,
    };
    if last <= first + 2 {
        return false;
    }
    let body = &levels[first..=last];
    let mut top: Vec<f64> = body.iter().map(|&l| l.max(gate)).collect();
    top.sort_by(|a, b| a.total_cmp(b));
    let p90 = top[(0.9 * (top.len() - 1) as f64) as usize];
    let p10 = top[(0.1 * (top.len() - 1) as f64) as usize];
    if p90 >= 2.0 * p10 {
        return false;
    }
    let mut low: Vec<f64> = body.iter().map(|&l| l.max(gate / 8.0)).collect();
    low.sort_by(|a, b| a.total_cmp(b));
    let p50_low = low[(0.5 * (low.len() - 1) as f64) as usize];
    let p10_low = low[(0.1 * (low.len() - 1) as f64) as usize];
    p50_low < 2.5 * p10_low
}

/// Low-frequency rumble rejection (<25% of energy above 100 Hz).
fn is_low_rumble(levels: &[f64], highs: &[f64], gate: f64) -> bool {
    let mut full = 0.0f64;
    let mut above = 0.0f64;
    for (&l, &h) in levels.iter().zip(highs.iter()) {
        if l < gate {
            continue;
        }
        full += l * l;
        above += h * h;
    }
    full > 0.0 && above < 0.25 * full
}

// =============================================================================
// EchoFilter — Dual-Channel Deduplication (ported from Cicada `Hybrid.cs`)
// =============================================================================

struct EchoFilter {
    last_online_loud: Option<Instant>,
    recent_online: VecDeque<(Instant, String)>,
}

impl EchoFilter {
    fn new() -> Self {
        Self {
            last_online_loud: None,
            recent_online: VecDeque::new(),
        }
    }

    fn reset(&mut self) {
        self.last_online_loud = None;
        self.recent_online.clear();
    }

    fn mark_online_loud(&mut self) {
        self.last_online_loud = Some(Instant::now());
    }

    fn record_online(&mut self, text: &str, at: Instant) {
        self.recent_online.push_back((at, text.to_string()));
        while let Some((t, _)) = self.recent_online.front() {
            if at.saturating_duration_since(*t) > Duration::from_secs(20) {
                self.recent_online.pop_front();
            } else {
                break;
            }
        }
    }

    fn is_echo(&self, room_text: &str, at: Instant) -> bool {
        // If speakers were silent recently, room speech is genuinely local
        if let Some(last_loud) = self.last_online_loud {
            if at.saturating_duration_since(last_loud) > Duration::from_secs(5) {
                return false;
            }
        } else {
            return false;
        }

        let near: Vec<&str> = self
            .recent_online
            .iter()
            .filter(|(t, _)| {
                let dt = if at >= *t {
                    at.duration_since(*t)
                } else {
                    t.duration_since(at)
                };
                dt <= Duration::from_secs(4)
            })
            .map(|(_, s)| s.as_str())
            .collect();

        is_echo_match(room_text, &near)
    }
}

fn is_echo_match(room_text: &str, online_texts: &[&str]) -> bool {
    if online_texts.is_empty() {
        return false;
    }
    let r = text_units(room_text);
    if r.is_empty() {
        return false;
    }
    let online_units: Vec<Vec<String>> = online_texts.iter().map(|s| text_units(s)).collect();
    if r.len() <= 2 {
        return online_units.iter().any(|o| o == &r);
    }
    let mine: HashSet<String> = r.windows(2).map(|w| format!("{} {}", w[0], w[1])).collect();
    if mine.is_empty() {
        return false;
    }
    let theirs: HashSet<String> = online_units
        .iter()
        .flat_map(|u| u.windows(2).map(|w| format!("{} {}", w[0], w[1])))
        .collect();
    let shared = mine.iter().filter(|p| theirs.contains(*p)).count();
    (shared as f64 / mine.len() as f64) >= 0.60
}

fn text_units(s: &str) -> Vec<String> {
    let mut units = Vec::new();
    let mut word = String::new();
    for c in s.to_lowercase().chars() {
        if is_cjk(c) {
            if !word.is_empty() {
                units.push(std::mem::take(&mut word));
            }
            units.push(c.to_string());
        } else if c.is_alphanumeric() {
            word.push(c);
        } else if !word.is_empty() {
            units.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        units.push(word);
    }
    units
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF
    )
}

// =============================================================================
// Audio Resampling & WAV Encoding
// =============================================================================

fn rms_i16(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / samples.len() as f64).sqrt() / 32768.0
}

fn resample_to_i16(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<i16> {
    if input.is_empty() {
        return Vec::new();
    }
    if src_rate == dst_rate {
        return input
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = ((input.len() as f64) / ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let s0 = input.get(idx).copied().unwrap_or(0.0);
        let s1 = input.get(idx + 1).copied().unwrap_or(s0);
        let val = s0 + (s1 - s0) * frac;
        out.push((val.clamp(-1.0, 1.0) * 32767.0) as i16);
    }
    out
}

fn encode_wav_pcm16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_bytes = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

// =============================================================================
// Fast AI Hear + Translate Engine (1-Step Gemini Audio OR Whisper + Fast LLM)
// =============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
struct HeardLine {
    lang: String,
    original: String,
    translation: String,
}

fn parse_glossary(raw: &str) -> Vec<String> {
    raw.split([';', ',', '\n'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .take(60)
        .map(ToOwned::to_owned)
        .collect()
}

fn hear_and_translate(
    cfg: &AssistantConfig,
    wav_bytes: &[u8],
    seconds: f64,
    target_lang: &str,
    hint_lang: Option<&str>,
    recent_context: &[String],
    glossary: &[String],
) -> Result<Option<HeardLine>, String> {
    if !cfg.has_any_key() {
        return Err("Atur API key di Config (Gemini / Groq / OpenRouter) untuk Cicada Mode.".to_string());
    }

    // Route 1 (Lowest latency 1-step): Gemini Native Audio when preferred or only Gemini key exists
    let prefer_gemini_1step = match cfg.stt_provider {
        SttProvider::Gemini => !cfg.gemini_api_key.is_empty(),
        SttProvider::Auto => {
            (!cfg.gemini_api_key.is_empty() && cfg.llm_provider == LlmProvider::Gemini)
                || (cfg.groq_api_key.is_empty()
                    && cfg.openai_api_key.is_empty()
                    && !cfg.gemini_api_key.is_empty())
        }
        SttProvider::Groq | SttProvider::OpenAi => false,
    };

    if prefer_gemini_1step {
        if let Ok(reply) = gemini_hear_one_step(
            cfg,
            wav_bytes,
            target_lang,
            hint_lang,
            recent_context,
            glossary,
        ) {
            return Ok(parse_three_line_heard(&reply, target_lang, hint_lang, seconds));
        }
    }

    // Route 2: Ultra-fast Whisper STT (Groq <300ms or OpenAI) -> Fast LLM / Google Translate
    let transcript = if !cfg.groq_api_key.is_empty() && cfg.stt_provider != SttProvider::OpenAi {
        transcribe_whisper_custom(
            "api.groq.com",
            443,
            "/openai/v1/audio/transcriptions",
            true,
            &cfg.groq_api_key,
            "whisper-large-v3-turbo",
            "auto",
            wav_bytes,
        )?
    } else if !cfg.openai_api_key.is_empty() {
        let (host, port, base_path, secure) = parse_url(&cfg.openai_base_url);
        let path = format!("{}/audio/transcriptions", base_path.trim_end_matches('/'));
        transcribe_whisper_custom(
            &host,
            port,
            &path,
            secure,
            &cfg.openai_api_key,
            "whisper-1",
            "auto",
            wav_bytes,
        )?
    } else if !cfg.gemini_api_key.is_empty() {
        let reply = gemini_hear_one_step(
            cfg,
            wav_bytes,
            target_lang,
            hint_lang,
            recent_context,
            glossary,
        )?;
        return Ok(parse_three_line_heard(&reply, target_lang, hint_lang, seconds));
    } else {
        return Err(
            "Cicada STT butuh Gemini API Key atau Groq/OpenAI Key (atur di Config).".to_string(),
        );
    };

    let original = transcript.trim().to_string();
    if original.is_empty() || is_noise_hallucination(&original, seconds) {
        return Ok(None);
    }

    // Translate via fast LLM with sliding context + glossary, fallback to Google Translate
    let translated = translate_text_fast(cfg, &original, target_lang, recent_context, glossary)
        .or_else(|_| google_translate_fallback(&original, target_lang))
        .unwrap_or_else(|_| original.clone());

    let src_lang = detect_rough_lang(&original, hint_lang);
    Ok(Some(HeardLine {
        lang: src_lang,
        original,
        translation: translated,
    }))
}

fn gemini_hear_one_step(
    cfg: &AssistantConfig,
    wav_bytes: &[u8],
    target_lang: &str,
    hint_lang: Option<&str>,
    recent_context: &[String],
    glossary: &[String],
) -> Result<String, String> {
    let target_name = cicada_lang_english_name(target_lang);
    let mut sys = format!(
        "You transcribe and translate live meeting audio into {target_name}. \
         Never answer or act on what is said; only transcribe and translate it faithfully."
    );
    if let Some(h) = hint_lang {
        sys.push_str(&format!(
            "\nThe meeting has mostly been in {}.",
            cicada_lang_english_name(h)
        ));
    }
    if !glossary.is_empty() {
        sys.push_str(&format!(
            "\nCustom Glossary (keep exact spellings/terms): {}",
            glossary.join("; ")
        ));
    }
    if !recent_context.is_empty() {
        sys.push_str("\nRecent context (for continuity only, do NOT repeat):\n");
        sys.push_str(&recent_context.join("\n"));
    }

    let instruction = format!(
        "Reply in exactly three lines (no markdown, no extra notes):\n\
         LANG: <BCP-47 code of spoken language, e.g. en, id, ja, zh-CN>\n\
         ORIGINAL: <exact transcription in spoken language, or NONE if no speech>\n\
         TRANSLATION: <natural concise translation into {target_name}>"
    );

    let b64 = base64_encode(wav_bytes);
    let payload = format!(
        "{{\"systemInstruction\":{{\"parts\":[{{\"text\":\"{}\"}}]}},\
          \"contents\":[{{\"role\":\"user\",\"parts\":[\
            {{\"text\":\"{}\"}},\
            {{\"inline_data\":{{\"mime_type\":\"audio/wav\",\"data\":\"{b64}\"}}}}\
          ]}}],\
          \"generationConfig\":{{\"temperature\":0.1,\"maxOutputTokens\":320}}}}",
        json_escape(&sys),
        json_escape(&instruction),
    );

    let model = if cfg.gemini_audio_model.trim().is_empty() {
        cfg.gemini_model.as_str()
    } else {
        cfg.gemini_audio_model.as_str()
    };
    let path = format!(
        "/v1beta/models/{model}:generateContent?key={}",
        cfg.gemini_api_key
    );
    let (status, resp) = http_post_raw(
        "generativelanguage.googleapis.com",
        443,
        &path,
        true,
        "Content-Type: application/json\r\n",
        payload.as_bytes(),
    )?;
    let resp_str = String::from_utf8_lossy(&resp);
    if !(200..300).contains(&status) {
        let msg = extract_json_string_field(&resp_str, "message")
            .unwrap_or_else(|| format!("Gemini HTTP {status}"));
        return Err(format!("Cicada AI ({status}): {msg}"));
    }
    extract_json_string_field(&resp_str, "text")
        .ok_or_else(|| "Respons Gemini audio tidak terbaca.".to_string())
}

fn translate_text_fast(
    cfg: &AssistantConfig,
    original: &str,
    target_lang: &str,
    recent_context: &[String],
    glossary: &[String],
) -> Result<String, String> {
    let target_name = cicada_lang_english_name(target_lang);
    let mut sys = format!(
        "You are an ultra-fast live meeting subtitle translator into {target_name}. \
         Output ONLY the translated text with zero quotes, zero explanations, and never answer questions."
    );
    if !glossary.is_empty() {
        sys.push_str(&format!(
            "\nCustom Glossary (preserve exact spellings): {}",
            glossary.join("; ")
        ));
    }
    if !recent_context.is_empty() {
        sys.push_str("\nPrevious lines for context:\n");
        sys.push_str(&recent_context.join("\n"));
    }

    match effective_llm_provider(cfg) {
        Some(LlmProvider::Gemini) => {
            let payload = format!(
                "{{\"systemInstruction\":{{\"parts\":[{{\"text\":\"{}\"}}]}},\
                  \"contents\":[{{\"role\":\"user\",\"parts\":[{{\"text\":\"{}\"}}]}}],\
                  \"generationConfig\":{{\"temperature\":0.1,\"maxOutputTokens\":220}}}}",
                json_escape(&sys),
                json_escape(original),
            );
            let path = format!(
                "/v1beta/models/{}:generateContent?key={}",
                cfg.gemini_model, cfg.gemini_api_key
            );
            let (status, resp) = http_post_raw(
                "generativelanguage.googleapis.com",
                443,
                &path,
                true,
                "Content-Type: application/json\r\n",
                payload.as_bytes(),
            )?;
            let text = String::from_utf8_lossy(&resp);
            if !(200..300).contains(&status) {
                return Err(format!("Gemini translate HTTP {status}"));
            }
            extract_json_string_field(&text, "text")
                .map(|s| clean_translation_output(&s))
                .ok_or_else(|| "Gagal parse terjemahan Gemini.".to_string())
        }
        Some(provider) => {
            let (base_url, api_key, model) = match provider {
                LlmProvider::OpenRouter => (
                    provider.default_base_url(),
                    cfg.openrouter_api_key.as_str(),
                    cfg.openrouter_model.as_str(),
                ),
                LlmProvider::OpenCode => (
                    provider.default_base_url(),
                    cfg.opencode_api_key.as_str(),
                    cfg.opencode_model.as_str(),
                ),
                LlmProvider::Nvidia => (
                    provider.default_base_url(),
                    cfg.nvidia_api_key.as_str(),
                    cfg.nvidia_model.as_str(),
                ),
                LlmProvider::Groq => (
                    provider.default_base_url(),
                    cfg.groq_api_key.as_str(),
                    cfg.groq_model.as_str(),
                ),
                LlmProvider::OpenAi => (
                    cfg.openai_base_url.as_str(),
                    cfg.openai_api_key.as_str(),
                    cfg.openai_model.as_str(),
                ),
                LlmProvider::Gemini => unreachable!(),
            };
            let (host, port, base_path, secure) = parse_url(base_url);
            let path = format!("{}/chat/completions", base_path.trim_end_matches('/'));
            let payload = format!(
                "{{\"model\":\"{}\",\"temperature\":0.1,\"max_tokens\":220,\"messages\":[\
                  {{\"role\":\"system\",\"content\":\"{}\"}},\
                  {{\"role\":\"user\",\"content\":\"{}\"}}\
                ]}}",
                json_escape(model),
                json_escape(&sys),
                json_escape(original),
            );
            let headers = format!(
                "Authorization: Bearer {api_key}\r\nContent-Type: application/json\r\n"
            );
            let (status, resp) =
                http_post_raw(&host, port, &path, secure, &headers, payload.as_bytes())?;
            let text = String::from_utf8_lossy(&resp);
            if !(200..300).contains(&status) {
                return Err(format!("LLM translate HTTP {status}"));
            }
            extract_json_string_field(&text, "content")
                .map(|s| clean_translation_output(&s))
                .ok_or_else(|| "Gagal parse terjemahan LLM.".to_string())
        }
        None => Err("Tidak ada LLM key.".to_string()),
    }
}

/// Free lightweight fallback (`translate.googleapis.com/translate_a/single?client=gtx`),
/// matching Cicada's fallback chain when an LLM times out or fails.
fn google_translate_fallback(text: &str, target_lang: &str) -> Result<String, String> {
    let encoded = url_encode(text);
    let path = format!(
        "/translate_a/single?client=gtx&sl=auto&tl={}&dt=t&q={encoded}",
        url_encode(target_lang)
    );
    let (status, resp) = http_get_raw("translate.googleapis.com", 443, &path, true, "")?;
    if !(200..300).contains(&status) {
        return Err(format!("Google Translate HTTP {status}"));
    }
    let body = String::from_utf8_lossy(&resp);
    // Response starts with [[["translated","original",...],...]]
    parse_google_gtx_response(&body).ok_or_else(|| "Format Google Translate tidak dikenali".to_string())
}

fn parse_google_gtx_response(body: &str) -> Option<String> {
    // Extract first string of each inner `[ "..." , "..." ]` array inside the first outer array.
    let bytes = body.as_bytes();
    let mut i = 0usize;
    let mut out = String::new();
    // Expect `[[[`
    while i < bytes.len() && bytes[i] != b'[' {
        i += 1;
    }
    i += 1;
    while i < bytes.len() && bytes[i] != b'[' {
        i += 1;
    }
    i += 1;

    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b']' {
            break;
        }
        if bytes[i] == b'[' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
                let (seg, next_i) = read_simple_json_str(body, i)?;
                out.push_str(&seg);
                i = next_i;
            }
            // Skip to matching `]`
            let mut depth = 1usize;
            let mut in_str = false;
            let mut esc = false;
            while i < bytes.len() && depth > 0 {
                let b = bytes[i];
                if in_str {
                    if esc {
                        esc = false;
                    } else if b == b'\\' {
                        esc = true;
                    } else if b == b'"' {
                        in_str = false;
                    }
                } else if b == b'"' {
                    in_str = true;
                } else if b == b'[' {
                    depth += 1;
                } else if b == b']' {
                    depth -= 1;
                }
                i += 1;
            }
        } else {
            break;
        }
    }

    if out.trim().is_empty() {
        None
    } else {
        Some(out.trim().to_string())
    }
}

fn read_simple_json_str(s: &str, start: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut chars = s[start..].char_indices().peekable();
    while let Some((off, ch)) = chars.next() {
        match ch {
            '"' => return Some((out, start + off + 1)),
            '\\' => match chars.next()?.1 {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn parse_three_line_heard(
    reply: &str,
    _target_lang: &str,
    hint: Option<&str>,
    seconds: f64,
) -> Option<HeardLine> {
    let clean = reply.rsplit("</think>").next().unwrap_or(reply);
    let mut lang: Option<String> = None;
    let mut original: Option<String> = None;
    let mut translation: Option<String> = None;

    for raw in clean.lines() {
        let line = raw.trim();
        if let Some(rest) = strip_prefix_ci(line, "LANG:") {
            lang = Some(rest.trim().to_string());
        } else if let Some(rest) = strip_prefix_ci(line, "ORIGINAL:") {
            original = Some(rest.trim().to_string());
        } else if let Some(rest) = strip_prefix_ci(line, "TRANSLATION:") {
            translation = Some(rest.trim().to_string());
        }
    }

    let orig = original?.trim().to_string();
    let orig_clean = orig.trim_matches(|c: char| c == '.' || c == '。' || c.is_whitespace());
    if orig_clean.is_empty() || orig_clean.eq_ignore_ascii_case("NONE") {
        return None;
    }
    if !orig.chars().any(|c| c.is_alphanumeric()) {
        return None;
    }
    if is_noise_hallucination(&orig, seconds) {
        return None;
    }

    let code = normalize_lang_code(lang.as_deref(), hint.unwrap_or("en"));
    let trans = match translation {
        Some(t) if !t.trim().is_empty() && !t.trim().eq_ignore_ascii_case("NONE") => {
            clean_translation_output(&t)
        }
        _ => orig.clone(),
    };

    Some(HeardLine {
        lang: code,
        original: orig,
        translation: trans,
    })
}

fn strip_prefix_ci<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    if line.len() >= prefix.len() && line[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&line[prefix.len()..])
    } else {
        None
    }
}

fn clean_translation_output(raw: &str) -> String {
    let no_think = raw.rsplit("</think>").next().unwrap_or(raw).trim();
    let stripped = strip_prefix_ci(no_think, "translation:")
        .unwrap_or(no_think)
        .trim()
        .trim_matches('"')
        .trim();
    stripped.to_string()
}

fn normalize_lang_code(raw: Option<&str>, fallback: &str) -> String {
    let s = raw.unwrap_or("").trim().to_ascii_lowercase();
    if s.is_empty() {
        return fallback.to_string();
    }
    if s.starts_with("id") || s.contains("indones") {
        "id".to_string()
    } else if s.starts_with("en") || s.contains("english") {
        "en".to_string()
    } else if s.starts_with("ja") || s.starts_with("jp") || s.contains("japan") {
        "ja".to_string()
    } else if s.starts_with("zh") || s.contains("chin") {
        "zh-CN".to_string()
    } else if s.starts_with("ko") || s.starts_with("kr") || s.contains("korea") {
        "ko".to_string()
    } else if s.starts_with("es") {
        "es".to_string()
    } else if s.starts_with("fr") {
        "fr".to_string()
    } else if s.starts_with("de") {
        "de".to_string()
    } else {
        s.chars().take(5).collect()
    }
}

fn detect_rough_lang(text: &str, hint: Option<&str>) -> String {
    if text.chars().any(|c| matches!(c as u32, 0x3040..=0x30FF)) {
        return "ja".to_string();
    }
    if text.chars().any(|c| matches!(c as u32, 0xAC00..=0xD7AF)) {
        return "ko".to_string();
    }
    if text.chars().any(|c| matches!(c as u32, 0x4E00..=0x9FFF)) {
        return "zh-CN".to_string();
    }
    hint.unwrap_or("en").to_string()
}

/// Port of Cicada's `Translator.IsNoise`: drops silence/music descriptions, timestamps,
/// filler-only sounds ("um", "hmm", "uh"), and runaway token repetitions.
fn is_noise_hallucination(original: &str, seconds: f64) -> bool {
    let lower = original.to_lowercase();
    const NO_SPEECH: &[&str] = &[
        "no speech",
        "no spoken",
        "no one is speaking",
        "nobody is speaking",
        "no audible speech",
        "no dialogue",
        "silence",
        "tidak ada suara",
        "tidak ada ucapan",
        "hening",
        "没有说话",
        "无语音",
        "没有语音",
        "音声がありません",
        "음성이 없습니다",
    ];
    if NO_SPEECH.iter().any(|p| lower.contains(p)) {
        return true;
    }
    // Strip bracketed annotations like [Music], (applause), ♪
    let mut cleaned = String::with_capacity(lower.len());
    let mut depth = 0usize;
    for ch in lower.chars() {
        match ch {
            '[' | '(' | '（' | '【' => depth += 1,
            ']' | ')' | '）' | '】' => depth = depth.saturating_sub(1),
            '♪' => {}
            _ if depth == 0 => cleaned.push(ch),
            _ => {}
        }
    }
    let units = text_units(&cleaned);
    if units.is_empty() {
        return true;
    }

    const FILLERS: &[&str] = &[
        "uh", "uhm", "um", "umm", "hmm", "hm", "mm", "mhm", "ah", "oh", "eh", "er", "erm", "huh",
        "hmm", "嗯", "啊", "呃", "哦", "え", "あ", "うん", "음", "어",
    ];
    if units.iter().all(|u| {
        let shortened = collapse_triple_chars(u);
        FILLERS.contains(&shortened.as_str())
    }) {
        return true;
    }

    // Runaway repetition (e.g. keyboard clicking hallucinated as 6x same token)
    let n = units.len();
    for p in 1..=3 {
        if n >= 6 * p && (p..n).all(|i| units[i] == units[i - p]) {
            return true;
        }
    }

    seconds > 0.0 && n as f64 > (14.0 * seconds).max(16.0)
}

fn collapse_triple_chars(s: &str) -> String {
    let mut out = String::new();
    let mut prev1: Option<char> = None;
    let mut prev2: Option<char> = None;
    for c in s.chars() {
        if prev1 == Some(c) && prev2 == Some(c) {
            continue;
        }
        out.push(c);
        prev2 = prev1;
        prev1 = Some(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_line_heard_reply() {
        let reply = "LANG: en\nORIGINAL: Let's ship the release tomorrow.\nTRANSLATION: Mari kita rilis besok.";
        let h = parse_three_line_heard(reply, "id", None, 2.0).unwrap();
        assert_eq!(h.lang, "en");
        assert_eq!(h.original, "Let's ship the release tomorrow.");
        assert_eq!(h.translation, "Mari kita rilis besok.");
    }

    #[test]
    fn filters_fillers_and_music_hallucinations() {
        assert!(is_noise_hallucination("[Music]", 2.0));
        assert!(is_noise_hallucination("ummm hmm", 1.0));
        assert!(is_noise_hallucination("la la la la la la", 2.0));
        assert!(!is_noise_hallucination("Can you hear me clearly?", 2.0));
    }

    #[test]
    fn echo_match_drops_speaker_bleed_and_keeps_local_speech() {
        let online = ["We need to update the deployment pipeline before Friday."];
        assert!(is_echo_match(
            "need to update the deployment pipeline before Friday",
            &online
        ));
        assert!(!is_echo_match(
            "Oke siap, nanti sore saya kirim revisinya.",
            &online
        ));
    }

    #[test]
    fn parses_google_gtx_json() {
        let sample = r#"[[["Halo dunia. ","Hello world. ",null,null,1],["Apa kabar?","How are you?",null,null,1]],null,"en"]"#;
        assert_eq!(
            parse_google_gtx_response(sample).as_deref(),
            Some("Halo dunia. Apa kabar?")
        );
    }
}
