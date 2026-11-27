//! Background Voice-to-AI pipeline: WASAPI Mic + VAD -> STT -> Streaming LLM.
//!
//! Uses native Windows WinHTTP (`Win32_Networking_WinHttp`) for HTTPS & SSE
//! streaming with zero external TLS/HTTP crates, keeping binary size tiny and
//! respecting Windows system proxies automatically.
//!
//! Supported pipelines (BYOK via `%APPDATA%\Zekrima\assistant.ini`):
//! 1. Groq Whisper (`whisper-large-v3-turbo`) or OpenAI Whisper (`whisper-1`) STT
//!    -> Gemini 2.5 Flash / Groq Llama / OpenAI GPT-4o-mini SSE streaming.
//! 2. Zero-extra-key Gemini Native Audio: if only `gemini_api_key` is set, sends
//!    the recorded 16kHz WAV directly to Gemini 2.5 Flash `:streamGenerateContent`
//!    in a single low-latency streaming request!

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[link(name = "winmm")]
extern "system" {
    fn PlaySoundW(psz_sound: *const u8, hmod: *mut c_void, fdw_sound: u32) -> i32;
}

const SND_ASYNC: u32 = 0x0001;
const SND_MEMORY: u32 = 0x0004;
const SND_NODEFAULT: u32 = 0x0002;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_USER};

use crate::services::config::{AssistantConfig, LlmProvider, SttProvider};
use crate::services::files::{AttachmentData, FileAttachment};
use crate::services::notifications::NotificationSnapshot;
use crate::services::screen;
use crate::services::tools;
use crate::services::voice::{CaptureMode, RecordingResult, VoiceRecorder};

/// Posted to the main window whenever the assistant state or streaming text changes.
pub const WM_ASSISTANT_UPDATED: u32 = WM_USER + 205;

const MAX_HISTORY_TURNS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantPhase {
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
    Done,
    Error,
}

impl AssistantPhase {
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Listening | Self::Transcribing | Self::Thinking | Self::Speaking
        )
    }

    pub fn is_listening(self) -> bool {
        self == Self::Listening
    }
}

/// Live context passed into each voice or chat request (notifications, optional screen/clipboard/file).
#[derive(Clone, Default)]
pub struct RequestContext {
    pub notifications: NotificationSnapshot,
    pub clipboard_preview: Option<String>,
    pub attachment: Option<FileAttachment>,
    /// Live or recent Cicada meeting bilingual transcript for Live Q&A & Auto-Summary.
    pub meeting_transcript: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub is_user: bool,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct AssistantSnapshot {
    pub phase: AssistantPhase,
    pub transcript: String,
    pub reply: String,
    pub messages: Vec<ChatMessage>,
    pub error: Option<String>,
    pub provider_label: String,
    pub has_config_key: bool,
    /// Privacy toggle: true = Screen Vision & clipboard context ON, false = Incognito Mode.
    pub screen_access_enabled: bool,
    /// True strictly while a screenshot is in flight (drives visual eye indicator).
    pub screen_capturing_now: bool,
    /// Summaries of system control actions executed on the last turn.
    pub last_tool_badges: Vec<String>,
}

impl Default for AssistantSnapshot {
    fn default() -> Self {
        Self {
            phase: AssistantPhase::Idle,
            transcript: String::new(),
            reply: String::new(),
            messages: Vec::new(),
            error: None,
            provider_label: "Gemini".to_string(),
            has_config_key: false,
            screen_access_enabled: true,
            screen_capturing_now: false,
            last_tool_badges: Vec::new(),
        }
    }
}

impl AssistantSnapshot {
    /// True when the expanded Zeki tab should show the chat/response view.
    pub fn has_conversation_view(&self) -> bool {
        self.phase != AssistantPhase::Idle
            || !self.messages.is_empty()
            || !self.reply.is_empty()
            || !self.transcript.is_empty()
            || self.error.is_some()
    }
}

#[derive(Clone, Debug)]
struct ChatTurn {
    role: &'static str, // "user" or "assistant"
    text: String,
}

struct SharedState {
    hwnd_raw: isize,
    snapshot: Mutex<AssistantSnapshot>,
    history: Mutex<Vec<ChatTurn>>,
    generation: AtomicU64,
    cancel_flag: Mutex<Arc<AtomicBool>>,
}

impl SharedState {
    fn notify_ui(&self) {
        if self.hwnd_raw != 0 {
            unsafe {
                let _ = PostMessageW(
                    HWND(self.hwnd_raw as *mut c_void),
                    WM_ASSISTANT_UPDATED,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    fn update_snapshot(&self, f: impl FnOnce(&mut AssistantSnapshot)) {
        if let Ok(mut snap) = self.snapshot.lock() {
            f(&mut snap);
        }
        self.notify_ui();
    }

    fn is_current(&self, gen: u64, cancel: &AtomicBool) -> bool {
        self.generation.load(Ordering::SeqCst) == gen && !cancel.load(Ordering::SeqCst)
    }
}

pub struct AssistantService {
    shared: Arc<SharedState>,
    recorder: Mutex<Option<VoiceRecorder>>,
}

impl AssistantService {
    pub fn start(hwnd: HWND) -> Self {
        // Ensure a starter template exists on first launch so Config is instant.
        let cfg = AssistantConfig::load();
        let initial = AssistantSnapshot {
            phase: AssistantPhase::Idle,
            transcript: String::new(),
            reply: String::new(),
            messages: Vec::new(),
            error: None,
            provider_label: provider_badge(&cfg),
            has_config_key: cfg.has_any_key(),
            screen_access_enabled: true,
            screen_capturing_now: false,
            last_tool_badges: Vec::new(),
        };

        Self {
            shared: Arc::new(SharedState {
                hwnd_raw: hwnd.0 as isize,
                snapshot: Mutex::new(initial),
                history: Mutex::new(Vec::new()),
                generation: AtomicU64::new(0),
                cancel_flag: Mutex::new(Arc::new(AtomicBool::new(false))),
            }),
            recorder: Mutex::new(None),
        }
    }

    pub fn snapshot(&self) -> AssistantSnapshot {
        self.shared
            .snapshot
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Live normalized microphone level (`0.0..=1.0`) while recording.
    pub fn live_voice_level(&self) -> f32 {
        self.recorder
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.live_level()))
            .unwrap_or(0.0)
    }

    pub fn is_listening(&self) -> bool {
        self.shared
            .snapshot
            .lock()
            .map(|s| s.phase == AssistantPhase::Listening)
            .unwrap_or(false)
    }

    pub fn is_screen_enabled(&self) -> bool {
        self.shared
            .snapshot
            .lock()
            .map(|s| s.screen_access_enabled)
            .unwrap_or(true)
    }

    /// Toggles Screen Vision / Incognito privacy mode. Returns the new state (`true` = Screen ON).
    pub fn toggle_screen_access(&self) -> bool {
        let mut enabled = true;
        self.shared.update_snapshot(|snap| {
            snap.screen_access_enabled = !snap.screen_access_enabled;
            if !snap.screen_access_enabled {
                snap.screen_capturing_now = false;
            }
            enabled = snap.screen_access_enabled;
        });
        enabled
    }

    /// Begins microphone recording (Push-to-Talk or Click-to-Talk with auto-VAD).
    pub fn start_listening(&self, mode: CaptureMode, req_ctx: RequestContext) {
        // Cancel any previous in-flight request or recording
        self.cancel_inflight_only();

        let cfg = AssistantConfig::load();
        if !cfg.has_any_key() {
            AssistantConfig::ensure_template_file();
            self.shared.update_snapshot(|snap| {
                snap.phase = AssistantPhase::Error;
                snap.has_config_key = false;
                snap.provider_label = provider_badge(&cfg);
                snap.error = Some(
                    "API key belum diatur. Klik tombol Config untuk memilih Gemini / OpenRouter / OpenCode / NVIDIA."
                        .to_string(),
                );
            });
            return;
        }

        let gen = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut slot) = self.shared.cancel_flag.lock() {
            *slot = Arc::clone(&cancel);
        }

        let screen_on = self.is_screen_enabled();
        self.shared.update_snapshot(|snap| {
            snap.phase = AssistantPhase::Listening;
            snap.transcript.clear();
            snap.reply.clear();
            snap.error = None;
            snap.last_tool_badges.clear();
            snap.screen_capturing_now = false;
            snap.has_config_key = true;
            snap.provider_label = provider_badge(&cfg);
        });

        let shared = Arc::clone(&self.shared);
        let rec = VoiceRecorder::start(mode, move |recording| {
            if !shared.is_current(gen, &cancel) {
                return;
            }
            process_recording(shared, gen, cancel, cfg, recording, req_ctx, screen_on);
        });

        if let Ok(mut slot) = self.recorder.lock() {
            *slot = Some(rec);
        }
    }

    /// Sends a typed text message directly to the configured streaming LLM
    /// with optional Screen Vision & System Control tool calling.
    pub fn send_text(&self, prompt: String, req_ctx: RequestContext) {
        let clean = prompt.trim().to_string();
        if clean.is_empty() {
            return;
        }

        self.cancel_inflight_only();

        let cfg = AssistantConfig::load();
        if !cfg.has_any_key() {
            AssistantConfig::ensure_template_file();
            self.shared.update_snapshot(|snap| {
                snap.phase = AssistantPhase::Error;
                snap.has_config_key = false;
                snap.provider_label = provider_badge(&cfg);
                snap.error = Some(
                    "API key belum diatur. Klik tombol Config untuk memilih Gemini / OpenRouter / OpenCode / NVIDIA."
                        .to_string(),
                );
            });
            return;
        }

        let gen = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut slot) = self.shared.cancel_flag.lock() {
            *slot = Arc::clone(&cancel);
        }

        let screen_on = self.is_screen_enabled();
        self.shared.update_snapshot(|snap| {
            snap.phase = AssistantPhase::Thinking;
            snap.transcript = clean.clone();
            snap.reply.clear();
            snap.error = None;
            snap.last_tool_badges.clear();
            snap.screen_capturing_now = screen_on;
            snap.has_config_key = true;
            snap.provider_label = provider_badge(&cfg);
        });

        let shared = Arc::clone(&self.shared);
        let _ = thread::Builder::new()
            .name("zekrima-chat-llm".to_owned())
            .spawn(move || {
                if !shared.is_current(gen, &cancel) {
                    return;
                }
                let (sys_prompt, jpeg_opt, bin_att) =
                    prepare_context_and_screen(&cfg, &req_ctx, screen_on);
                let history = shared
                    .history
                    .lock()
                    .map(|h| h.clone())
                    .unwrap_or_default();
                let res = run_llm_streaming(
                    &cfg,
                    &sys_prompt,
                    &clean,
                    jpeg_opt.as_deref(),
                    bin_att.as_ref(),
                    &history,
                    &shared,
                    gen,
                    &cancel,
                );
                // Zero Retention: drop screenshot from RAM immediately after request finishes
                drop(jpeg_opt);
                drop(bin_att);
                shared.update_snapshot(|snap| snap.screen_capturing_now = false);
                finalize_llm_result(&shared, gen, &cancel, &cfg, clean, res);
            });
    }

    /// Stops an active microphone capture so it immediately flows into STT + LLM.
    pub fn stop_listening(&self) {
        if let Ok(slot) = self.recorder.lock() {
            if let Some(rec) = slot.as_ref() {
                rec.stop();
            }
        }
    }

    /// Cancels active recording or streaming request and returns to Idle.
    pub fn cancel(&self) {
        self.cancel_inflight_only();
        self.shared.update_snapshot(|snap| {
            if snap.phase.is_busy() {
                snap.phase = if snap.reply.is_empty() {
                    AssistantPhase::Idle
                } else {
                    AssistantPhase::Done
                };
            }
        });
    }

    /// Clears chat history and hides the response card.
    pub fn clear_conversation(&self) {
        self.cancel_inflight_only();
        if let Ok(mut hist) = self.shared.history.lock() {
            hist.clear();
        }
        let cfg = AssistantConfig::load();
        self.shared.update_snapshot(|snap| {
            snap.phase = AssistantPhase::Idle;
            snap.transcript.clear();
            snap.reply.clear();
            snap.messages.clear();
            snap.error = None;
            snap.screen_capturing_now = false;
            snap.last_tool_badges.clear();
            snap.has_config_key = cfg.has_any_key();
            snap.provider_label = provider_badge(&cfg);
        });
    }

    /// Re-reads `assistant.ini` (e.g. after user clicked Config).
    pub fn refresh_config_status(&self) {
        let cfg = AssistantConfig::load();
        self.shared.update_snapshot(|snap| {
            snap.has_config_key = cfg.has_any_key();
            snap.provider_label = provider_badge(&cfg);
        });
    }

    fn cancel_inflight_only(&self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(slot) = self.shared.cancel_flag.lock() {
            slot.store(true, Ordering::SeqCst);
        }
        stop_tts_playback();
        if let Ok(mut rec_slot) = self.recorder.lock() {
            if let Some(rec) = rec_slot.take() {
                rec.stop();
                // Drop on a helper thread if needed so UI never waits for WASAPI stop
                let _ = thread::Builder::new()
                    .name("zekrima-mic-drop".to_owned())
                    .spawn(move || drop(rec));
            }
        }
    }
}

impl Drop for AssistantService {
    fn drop(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(slot) = self.shared.cancel_flag.lock() {
            slot.store(true, Ordering::SeqCst);
        }
        if let Ok(mut rec_slot) = self.recorder.lock() {
            if let Some(rec) = rec_slot.take() {
                rec.stop();
                drop(rec);
            }
        }
    }
}

fn provider_badge(cfg: &AssistantConfig) -> String {
    cfg.llm_provider.short_badge().to_string()
}

struct BinaryAttachment {
    mime_type: String,
    bytes: Vec<u8>,
}

fn prepare_context_and_screen(
    cfg: &AssistantConfig,
    req_ctx: &RequestContext,
    screen_enabled: bool,
) -> (String, Option<Vec<u8>>, Option<BinaryAttachment>) {
    let active_win = if screen_enabled {
        screen::active_window_title()
    } else {
        String::new()
    };
    let clip = if screen_enabled {
        req_ctx.clipboard_preview.as_deref()
    } else {
        None
    };
    let context_note = tools::build_context_note(&req_ctx.notifications, &active_win, clip);
    let privacy_note = if screen_enabled {
        "\n[SCREEN VISION AKTIF]: Screenshot layar aktif pengguna dilampirkan jika didukung model. Gunakan isi layar hanya untuk menjawab permintaan pengguna."
    } else {
        "\n[INCOGNITO MODE]: Akses layar & clipboard dimatikan pengguna demi privasi."
    };

    let mut file_text_section = String::new();
    let mut bin_att: Option<BinaryAttachment> = None;
    if let Some(att) = &req_ctx.attachment {
        match &att.data {
            AttachmentData::Text(text) => {
                file_text_section = format!(
                    "\n\n[ATTACHED FILE: {}]\n{}\n[END ATTACHED FILE]",
                    att.file_name, text
                );
            }
            AttachmentData::Binary { mime_type, bytes } => {
                file_text_section = format!(
                    "\n\n[ATTACHED FILE: {} ({}) dilampirkan untuk dibaca/dianalisis]",
                    att.file_name, mime_type
                );
                bin_att = Some(BinaryAttachment {
                    mime_type: mime_type.clone(),
                    bytes: bytes.clone(),
                });
            }
        }
    }

    let meeting_section = match &req_ctx.meeting_transcript {
        Some(tr) if !tr.trim().is_empty() => format!(
            "\n\n[LIVE MEETING TRANSCRIPT (CICADA MODE)]\n{}\n[END MEETING TRANSCRIPT]\nGunakan transkrip rapat ini bila pengguna menanyakan isi rapat, revisi, keputusan, atau ringkasan.",
            tr
        ),
        _ => String::new(),
    };

    let sys_prompt = format!(
        "{}{}{}\n\n{}{}{}",
        cfg.system_prompt,
        tools::TOOL_SYSTEM_INSTRUCTIONS,
        privacy_note,
        context_note,
        file_text_section,
        meeting_section
    );

    let jpeg = if screen_enabled {
        screen::capture_screen_jpeg(cfg.screen_resolution.max_width_px())
    } else {
        None
    };
    (sys_prompt, jpeg, bin_att)
}

fn process_recording(
    shared: Arc<SharedState>,
    gen: u64,
    cancel: Arc<AtomicBool>,
    cfg: AssistantConfig,
    rec: RecordingResult,
    req_ctx: RequestContext,
    screen_enabled: bool,
) {
    if let Some(err) = rec.error {
        shared.update_snapshot(|snap| {
            snap.phase = AssistantPhase::Error;
            snap.error = Some(err);
        });
        return;
    }

    if !rec.had_speech || rec.wav_bytes.len() <= 44 {
        shared.update_snapshot(|snap| {
            snap.phase = AssistantPhase::Error;
            snap.error = Some("Tidak ada suara terdeteksi. Tahan Alt+Space sambil bicara.".to_string());
        });
        return;
    }

    // Decide STT route
    let use_gemini_direct_audio = match cfg.stt_provider {
        SttProvider::Gemini => !cfg.gemini_api_key.is_empty(),
        SttProvider::Auto => {
            cfg.groq_api_key.is_empty()
                && cfg.openai_api_key.is_empty()
                && !cfg.gemini_api_key.is_empty()
        }
        SttProvider::Groq | SttProvider::OpenAi => false,
    };

    if use_gemini_direct_audio {
        shared.update_snapshot(|snap| {
            snap.phase = AssistantPhase::Thinking;
            snap.screen_capturing_now = screen_enabled;
            snap.transcript = format!("Voice ({:.1}s)", rec.duration_secs);
        });
        let (sys_prompt, jpeg_opt, bin_att) =
            prepare_context_and_screen(&cfg, &req_ctx, screen_enabled);
        let history = shared
            .history
            .lock()
            .map(|h| h.clone())
            .unwrap_or_default();
        let res = stream_gemini_audio(
            &cfg,
            &sys_prompt,
            &rec.wav_bytes,
            jpeg_opt.as_deref(),
            bin_att.as_ref(),
            &history,
            &shared,
            gen,
            &cancel,
        );
        // Zero Retention: immediately drop screenshot bytes from memory
        drop(jpeg_opt);
        drop(bin_att);
        shared.update_snapshot(|snap| snap.screen_capturing_now = false);
        finalize_llm_result(
            &shared,
            gen,
            &cancel,
            &cfg,
            format!("[Voice {:.1}s]", rec.duration_secs),
            res,
        );
        return;
    }

    // Step 1: Whisper STT (Groq or OpenAI)
    shared.update_snapshot(|snap| {
        snap.phase = AssistantPhase::Transcribing;
    });

    let stt_result = match cfg.stt_provider {
        SttProvider::Groq => transcribe_whisper(
            "api.groq.com",
            "/openai/v1/audio/transcriptions",
            &cfg.groq_api_key,
            "whisper-large-v3-turbo",
            &cfg.language,
            &rec.wav_bytes,
        ),
        SttProvider::OpenAi => {
            let (host, port, base_path, secure) = parse_url(&cfg.openai_base_url);
            transcribe_whisper_custom(
                &host,
                port,
                &format!("{base_path}/audio/transcriptions"),
                secure,
                &cfg.openai_api_key,
                "whisper-1",
                &cfg.language,
                &rec.wav_bytes,
            )
        }
        SttProvider::Auto => {
            if !cfg.groq_api_key.is_empty() {
                transcribe_whisper(
                    "api.groq.com",
                    "/openai/v1/audio/transcriptions",
                    &cfg.groq_api_key,
                    "whisper-large-v3-turbo",
                    &cfg.language,
                    &rec.wav_bytes,
                )
            } else if !cfg.openai_api_key.is_empty() {
                let (host, port, base_path, secure) = parse_url(&cfg.openai_base_url);
                transcribe_whisper_custom(
                    &host,
                    port,
                    &format!("{base_path}/audio/transcriptions"),
                    secure,
                    &cfg.openai_api_key,
                    "whisper-1",
                    &cfg.language,
                    &rec.wav_bytes,
                )
            } else {
                Err("Voice STT butuh Groq API key gratis (atau Gemini/OpenAI key) di Config -> Voice STT.".to_string())
            }
        }
        SttProvider::Gemini => Err("gemini_api_key belum diisi di Config.".to_string()),
    };

    if !shared.is_current(gen, &cancel) {
        return;
    }

    let transcript = match stt_result {
        Ok(text) => {
            let clean = text.trim().to_string();
            if clean.is_empty() {
                shared.update_snapshot(|snap| {
                    snap.phase = AssistantPhase::Error;
                    snap.error = Some("Suara tidak jelas — coba ucapkan ulang.".to_string());
                });
                return;
            }
            clean
        }
        Err(err) => {
            shared.update_snapshot(|snap| {
                snap.phase = AssistantPhase::Error;
                snap.error = Some(err);
            });
            return;
        }
    };

    // Step 2: Streaming LLM response (with optional Screen Vision & System Control tools)
    shared.update_snapshot(|snap| {
        snap.phase = AssistantPhase::Thinking;
        snap.screen_capturing_now = screen_enabled;
        snap.transcript = transcript.clone();
    });

    let (sys_prompt, jpeg_opt, bin_att) =
        prepare_context_and_screen(&cfg, &req_ctx, screen_enabled);
    let history = shared
        .history
        .lock()
        .map(|h| h.clone())
        .unwrap_or_default();

    let llm_result = run_llm_streaming(
        &cfg,
        &sys_prompt,
        &transcript,
        jpeg_opt.as_deref(),
        bin_att.as_ref(),
        &history,
        &shared,
        gen,
        &cancel,
    );
    drop(jpeg_opt);
    drop(bin_att);
    shared.update_snapshot(|snap| snap.screen_capturing_now = false);
    finalize_llm_result(&shared, gen, &cancel, &cfg, transcript, llm_result);
}

#[allow(clippy::too_many_arguments)]
fn run_llm_streaming(
    cfg: &AssistantConfig,
    sys_prompt: &str,
    transcript: &str,
    jpeg_opt: Option<&[u8]>,
    bin_att: Option<&BinaryAttachment>,
    history: &[ChatTurn],
    shared: &Arc<SharedState>,
    gen: u64,
    cancel: &AtomicBool,
) -> Result<String, String> {
    // Prefer an attached image over the screen capture for single-image OpenAI-compatible models
    let openai_img: Option<(&str, &[u8])> = match bin_att {
        Some(att) if att.mime_type.starts_with("image/") => {
            Some((att.mime_type.as_str(), att.bytes.as_slice()))
        }
        _ => jpeg_opt.map(|j| ("image/jpeg", j)),
    };

    match effective_llm_provider(cfg) {
        Some(LlmProvider::Gemini) => stream_gemini_text(
            cfg,
            sys_prompt,
            transcript,
            jpeg_opt,
            bin_att,
            history,
            shared,
            gen,
            cancel,
        ),
        Some(LlmProvider::OpenRouter) => stream_openai_with_vision_fallback(
            LlmProvider::OpenRouter.default_base_url(),
            &cfg.openrouter_api_key,
            &cfg.openrouter_model,
            sys_prompt,
            "HTTP-Referer: https://zekrima.local\r\nX-Title: Zekrima Assistant\r\n",
            transcript,
            openai_img,
            history,
            shared,
            gen,
            cancel,
        ),
        Some(LlmProvider::OpenCode) => stream_openai_with_vision_fallback(
            LlmProvider::OpenCode.default_base_url(),
            &cfg.opencode_api_key,
            &cfg.opencode_model,
            sys_prompt,
            "",
            transcript,
            openai_img,
            history,
            shared,
            gen,
            cancel,
        ),
        Some(LlmProvider::Nvidia) => stream_openai_with_vision_fallback(
            LlmProvider::Nvidia.default_base_url(),
            &cfg.nvidia_api_key,
            &cfg.nvidia_model,
            sys_prompt,
            "",
            transcript,
            openai_img,
            history,
            shared,
            gen,
            cancel,
        ),
        Some(LlmProvider::Groq) => stream_openai_with_vision_fallback(
            LlmProvider::Groq.default_base_url(),
            &cfg.groq_api_key,
            &cfg.groq_model,
            sys_prompt,
            "",
            transcript,
            openai_img,
            history,
            shared,
            gen,
            cancel,
        ),
        Some(LlmProvider::OpenAi) => stream_openai_with_vision_fallback(
            &cfg.openai_base_url,
            &cfg.openai_api_key,
            &cfg.openai_model,
            sys_prompt,
            "",
            transcript,
            openai_img,
            history,
            shared,
            gen,
            cancel,
        ),
        None => Err("API key LLM belum diisi. Klik Config untuk mengaturnya.".to_string()),
    }
}

/// Attempts multimodal vision first when an image is attached; if a text-only
/// free model rejects the `image_url` block (HTTP 400/422), retries seamlessly as text-only.
#[allow(clippy::too_many_arguments)]
fn stream_openai_with_vision_fallback(
    base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    extra_headers: &str,
    user_text: &str,
    img_opt: Option<(&str, &[u8])>,
    history: &[ChatTurn],
    shared: &Arc<SharedState>,
    gen: u64,
    cancel: &AtomicBool,
) -> Result<String, String> {
    if img_opt.is_some() {
        let first = stream_openai_compatible(
            base_url,
            api_key,
            model,
            system_prompt,
            extra_headers,
            user_text,
            img_opt,
            history,
            shared,
            gen,
            cancel,
        );
        match first {
            Ok(ans) => return Ok(ans),
            Err(err)
                if (err.contains("(400)") || err.contains("(422)") || err.contains("image"))
                    && shared.is_current(gen, cancel) =>
            {
                // Seamless fallback to text-only for non-vision free models
            }
            Err(err) => return Err(err),
        }
    }
    stream_openai_compatible(
        base_url,
        api_key,
        model,
        system_prompt,
        extra_headers,
        user_text,
        None,
        history,
        shared,
        gen,
        cancel,
    )
}

pub(crate) fn effective_llm_provider(cfg: &AssistantConfig) -> Option<LlmProvider> {
    if !cfg.api_key_for(cfg.llm_provider).is_empty() {
        return Some(cfg.llm_provider);
    }
    crate::services::config::ALL_LLM_PROVIDERS
        .into_iter()
        .find(|&p| !cfg.api_key_for(p).is_empty())
}

fn finalize_llm_result(
    shared: &Arc<SharedState>,
    gen: u64,
    cancel: &AtomicBool,
    cfg: &AssistantConfig,
    user_text: String,
    result: Result<String, String>,
) {
    if !shared.is_current(gen, cancel) {
        return;
    }
    match result {
        Ok(full_reply) => {
            let (cleaned_speech, tool_badges) = tools::parse_and_execute_tools(&full_reply);
            let display_reply = if tool_badges.is_empty() {
                cleaned_speech.clone()
            } else if cleaned_speech.is_empty() {
                tool_badges.join("  •  ")
            } else {
                format!("{cleaned_speech}\n[{}]", tool_badges.join(" • "))
            };
            let trimmed = display_reply.trim().to_string();

            let updated_messages: Vec<ChatMessage> = if !trimmed.is_empty() {
                if let Ok(mut hist) = shared.history.lock() {
                    hist.push(ChatTurn {
                        role: "user",
                        text: user_text,
                    });
                    hist.push(ChatTurn {
                        role: "assistant",
                        text: trimmed.clone(),
                    });
                    let max_entries = MAX_HISTORY_TURNS * 2;
                    if hist.len() > max_entries {
                        let drain = hist.len() - max_entries;
                        hist.drain(0..drain);
                    }
                    hist.iter()
                        .map(|t| ChatMessage {
                            is_user: t.role == "user",
                            text: t.text.clone(),
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // Optional Gemini 3.8 Flash TTS playback while keeping Zeki in `Speaking` state
            if cfg.gemini_tts_enabled
                && !cfg.gemini_api_key.is_empty()
                && !cleaned_speech.trim().is_empty()
            {
                let msgs = updated_messages.clone();
                let badges = tool_badges.clone();
                shared.update_snapshot(|snap| {
                    snap.phase = AssistantPhase::Speaking;
                    snap.reply = trimmed.clone();
                    snap.last_tool_badges = badges;
                    if !msgs.is_empty() {
                        snap.messages = msgs;
                    }
                    snap.error = None;
                });
                speak_with_gemini_tts(cfg, &cleaned_speech, cancel);
            }

            if !shared.is_current(gen, cancel) {
                return;
            }
            shared.update_snapshot(|snap| {
                snap.phase = AssistantPhase::Done;
                snap.last_tool_badges = tool_badges;
                if !trimmed.is_empty() {
                    snap.reply = trimmed;
                }
                if !updated_messages.is_empty() {
                    snap.messages = updated_messages;
                }
                snap.error = None;
            });
        }
        Err(err) => {
            shared.update_snapshot(|snap| {
                snap.phase = AssistantPhase::Error;
                snap.error = Some(err);
            });
        }
    }
}

// =============================================================================
// Whisper STT (Groq / OpenAI multipart/form-data)
// =============================================================================

fn transcribe_whisper(
    host: &str,
    path: &str,
    api_key: &str,
    model: &str,
    language: &str,
    wav_bytes: &[u8],
) -> Result<String, String> {
    transcribe_whisper_custom(host, 443, path, true, api_key, model, language, wav_bytes)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn transcribe_whisper_custom(
    host: &str,
    port: u16,
    path: &str,
    secure: bool,
    api_key: &str,
    model: &str,
    language: &str,
    wav_bytes: &[u8],
) -> Result<String, String> {
    let boundary = "----ZekrimaVoiceBoundary7f9a2c";
    let mut body: Vec<u8> = Vec::with_capacity(wav_bytes.len() + 512);

    // model field
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(model.as_bytes());
    body.extend_from_slice(b"\r\n");

    // optional language hint
    if !language.trim().is_empty() && !language.eq_ignore_ascii_case("auto") {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"language\"\r\n\r\n");
        body.extend_from_slice(language.trim().as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    // audio file field
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"voice.wav\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
    body.extend_from_slice(wav_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let headers = format!(
        "Authorization: Bearer {api_key}\r\nContent-Type: multipart/form-data; boundary={boundary}\r\n"
    );

    let (status, resp_bytes) = http_post_raw(host, port, path, secure, &headers, &body)?;
    let resp_str = String::from_utf8_lossy(&resp_bytes);
    if !(200..300).contains(&status) {
        let msg = extract_json_string_field(&resp_str, "message")
            .unwrap_or_else(|| format!("STT HTTP {status}"));
        return Err(format!("STT gagal: {msg}"));
    }

    extract_json_string_field(&resp_str, "text")
        .ok_or_else(|| "Format respons STT tidak dikenali.".to_string())
}

// =============================================================================
// Gemini 3.8 Flash Streaming (Chat Text, Native Audio Input, and TTS Output)
// =============================================================================

#[allow(clippy::too_many_arguments)]
fn stream_gemini_text(
    cfg: &AssistantConfig,
    sys_prompt: &str,
    user_text: &str,
    jpeg_opt: Option<&[u8]>,
    bin_att: Option<&BinaryAttachment>,
    history: &[ChatTurn],
    shared: &Arc<SharedState>,
    gen: u64,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut contents = String::from("[");
    for turn in history {
        let role = if turn.role == "assistant" { "model" } else { "user" };
        contents.push_str(&format!(
            "{{\"role\":\"{role}\",\"parts\":[{{\"text\":\"{}\"}}]}},",
            json_escape(&turn.text)
        ));
    }
    let mut parts = vec![format!("{{\"text\":\"{}\"}}", json_escape(user_text))];
    if let Some(jpeg) = jpeg_opt {
        parts.push(format!(
            "{{\"inline_data\":{{\"mime_type\":\"image/jpeg\",\"data\":\"{}\"}}}}",
            base64_encode(jpeg)
        ));
    }
    if let Some(att) = bin_att {
        parts.push(format!(
            "{{\"inline_data\":{{\"mime_type\":\"{}\",\"data\":\"{}\"}}}}",
            json_escape(&att.mime_type),
            base64_encode(&att.bytes)
        ));
    }
    contents.push_str(&format!("{{\"role\":\"user\",\"parts\":[{}]}}", parts.join(",")));
    contents.push(']');

    let payload = format!(
        "{{\"systemInstruction\":{{\"parts\":[{{\"text\":\"{}\"}}]}},\"contents\":{contents}}}",
        json_escape(sys_prompt)
    );

    stream_gemini_payload(cfg, &cfg.gemini_model, &payload, shared, gen, cancel)
}

#[allow(clippy::too_many_arguments)]
fn stream_gemini_audio(
    cfg: &AssistantConfig,
    sys_prompt: &str,
    wav_bytes: &[u8],
    jpeg_opt: Option<&[u8]>,
    bin_att: Option<&BinaryAttachment>,
    history: &[ChatTurn],
    shared: &Arc<SharedState>,
    gen: u64,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let b64 = base64_encode(wav_bytes);
    let mut contents = String::from("[");
    for turn in history {
        let role = if turn.role == "assistant" { "model" } else { "user" };
        contents.push_str(&format!(
            "{{\"role\":\"{role}\",\"parts\":[{{\"text\":\"{}\"}}]}},",
            json_escape(&turn.text)
        ));
    }
    let prompt_hint = if cfg.language.eq_ignore_ascii_case("en") {
        "Listen to this voice message and answer directly and concisely."
    } else {
        "Dengarkan pesan suara ini dan jawab langsung dengan singkat dan ramah."
    };
    let mut parts = vec![
        format!("{{\"text\":\"{}\"}}", json_escape(prompt_hint)),
        format!("{{\"inline_data\":{{\"mime_type\":\"audio/wav\",\"data\":\"{b64}\"}}}}"),
    ];
    if let Some(jpeg) = jpeg_opt {
        parts.push(format!(
            "{{\"inline_data\":{{\"mime_type\":\"image/jpeg\",\"data\":\"{}\"}}}}",
            base64_encode(jpeg)
        ));
    }
    if let Some(att) = bin_att {
        parts.push(format!(
            "{{\"inline_data\":{{\"mime_type\":\"{}\",\"data\":\"{}\"}}}}",
            json_escape(&att.mime_type),
            base64_encode(&att.bytes)
        ));
    }
    contents.push_str(&format!("{{\"role\":\"user\",\"parts\":[{}]}}", parts.join(",")));
    contents.push(']');

    let payload = format!(
        "{{\"systemInstruction\":{{\"parts\":[{{\"text\":\"{}\"}}]}},\"contents\":{contents}}}",
        json_escape(sys_prompt)
    );

    let audio_model = if cfg.gemini_audio_model.trim().is_empty() {
        cfg.gemini_model.as_str()
    } else {
        cfg.gemini_audio_model.as_str()
    };
    stream_gemini_payload(cfg, audio_model, &payload, shared, gen, cancel)
}

fn stream_gemini_payload(
    cfg: &AssistantConfig,
    model: &str,
    payload: &str,
    shared: &Arc<SharedState>,
    gen: u64,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let path = format!(
        "/v1beta/models/{model}:streamGenerateContent?alt=sse&key={}",
        cfg.gemini_api_key
    );
    let headers = "Content-Type: application/json\r\nAccept: text/event-stream\r\n";

    let mut accumulated = String::new();
    http_post_sse(
        "generativelanguage.googleapis.com",
        443,
        &path,
        true,
        headers,
        payload.as_bytes(),
        cancel,
        |data_line| {
            if !shared.is_current(gen, cancel) {
                return;
            }
            if let Some(chunk) = extract_json_string_field(data_line, "text") {
                if !chunk.is_empty() {
                    accumulated.push_str(&chunk);
                    let visible = tools::strip_tool_tags(&accumulated);
                    shared.update_snapshot(|snap| {
                        snap.phase = AssistantPhase::Speaking;
                        snap.reply = visible;
                    });
                }
            }
        },
    )?;

    if accumulated.trim().is_empty() {
        return Err("Gemini tidak mengembalikan teks jawaban.".to_string());
    }
    Ok(accumulated)
}

/// Synthesizes & plays speech using Gemini 3.8 Flash TTS (`gemini-3.8-flash-tts`),
/// which returns complete 24kHz WAV (`audio/wav`) played natively via `winmm`.
fn speak_with_gemini_tts(cfg: &AssistantConfig, text: &str, cancel: &AtomicBool) {
    if cancel.load(Ordering::SeqCst) {
        return;
    }

    // Keep TTS snappy by capping very long replies to the first ~380 chars
    let speak_text: String = text
        .replace(['*', '#', '`'], "")
        .chars()
        .take(380)
        .collect();
    if speak_text.trim().is_empty() {
        return;
    }

    let tts_model = if cfg.gemini_tts_model.trim().is_empty() {
        "gemini-3.8-flash-tts"
    } else {
        cfg.gemini_tts_model.as_str()
    };
    let voice = if cfg.gemini_tts_voice.trim().is_empty() {
        "Kore"
    } else {
        cfg.gemini_tts_voice.as_str()
    };

    let path = format!(
        "/v1beta/models/{tts_model}:generateContent?key={}",
        cfg.gemini_api_key
    );
    let payload = format!(
        "{{\"contents\":[{{\"role\":\"user\",\"parts\":[{{\"text\":\"{}\"}}]}}],\
          \"generationConfig\":{{\"responseModalities\":[\"AUDIO\"],\
          \"speechConfig\":{{\"voiceConfig\":{{\"prebuiltVoiceConfig\":{{\"voiceName\":\"{}\"}}}}}}}}}}",
        json_escape(&speak_text),
        json_escape(voice),
    );
    let headers = "Content-Type: application/json\r\n";

    let Ok((status, resp_bytes)) = http_post_raw(
        "generativelanguage.googleapis.com",
        443,
        &path,
        true,
        headers,
        payload.as_bytes(),
    ) else {
        return;
    };
    if !(200..300).contains(&status) || cancel.load(Ordering::SeqCst) {
        return;
    }

    let resp_str = String::from_utf8_lossy(&resp_bytes);
    let Some(b64_audio) = extract_json_string_field(&resp_str, "data") else {
        return;
    };
    let Some(raw_bytes) = base64_decode(&b64_audio) else {
        return;
    };
    if raw_bytes.len() < 44 || cancel.load(Ordering::SeqCst) {
        return;
    }

    // Gemini 3.8 TTS returns complete RIFF WAV; wrap raw 24kHz PCM16 if header is absent.
    let wav = if raw_bytes.starts_with(b"RIFF") {
        raw_bytes
    } else {
        wrap_pcm16_as_wav(&raw_bytes, 24_000)
    };

    let duration_ms = wav_duration_ms(&wav).clamp(400, 25_000);
    unsafe {
        let _ = PlaySoundW(
            wav.as_ptr(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }

    // Keep the WAV buffer alive until playback completes or is cancelled
    let started = Instant::now();
    while started.elapsed().as_millis() < duration_ms as u128 {
        if cancel.load(Ordering::SeqCst) {
            stop_tts_playback();
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    stop_tts_playback();
}

fn stop_tts_playback() {
    unsafe {
        let _ = PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0);
    }
}

fn wrap_pcm16_as_wav(pcm_bytes: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm_bytes.len() as u32;
    let riff_len = 36u32.saturating_add(data_len);
    let byte_rate = sample_rate * 2;
    let mut out = Vec::with_capacity(44 + pcm_bytes.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
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
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm_bytes);
    out
}

fn wav_duration_ms(wav: &[u8]) -> u64 {
    if wav.len() < 44 {
        return 0;
    }
    let byte_rate = u32::from_le_bytes([wav[28], wav[29], wav[30], wav[31]]).max(1) as u64;
    let payload_bytes = wav.len().saturating_sub(44) as u64;
    (payload_bytes * 1000) / byte_rate + 180
}

// =============================================================================
// OpenAI / Groq Compatible Chat Completions Streaming
// =============================================================================

#[allow(clippy::too_many_arguments)]
fn stream_openai_compatible(
    base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    extra_headers: &str,
    user_text: &str,
    img_opt: Option<(&str, &[u8])>,
    history: &[ChatTurn],
    shared: &Arc<SharedState>,
    gen: u64,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let (host, port, base_path, secure) = parse_url(base_url);
    let path = format!("{base_path}/chat/completions");

    let mut messages = format!(
        "[{{\"role\":\"system\",\"content\":\"{}\"}}",
        json_escape(system_prompt)
    );
    for turn in history {
        messages.push_str(&format!(
            ",{{\"role\":\"{}\",\"content\":\"{}\"}}",
            turn.role,
            json_escape(&turn.text)
        ));
    }
    if let Some((mime, bytes)) = img_opt {
        let img_b64 = base64_encode(bytes);
        messages.push_str(&format!(
            ",{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}},{{\"type\":\"image_url\",\"image_url\":{{\"url\":\"data:{mime};base64,{img_b64}\"}}}}]}}]",
            json_escape(user_text)
        ));
    } else {
        messages.push_str(&format!(
            ",{{\"role\":\"user\",\"content\":\"{}\"}}]",
            json_escape(user_text)
        ));
    }

    let payload = format!(
        "{{\"model\":\"{}\",\"stream\":true,\"messages\":{messages}}}",
        json_escape(model)
    );
    let headers = format!(
        "Authorization: Bearer {api_key}\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\n{extra_headers}"
    );

    let mut accumulated = String::new();
    http_post_sse(
        &host,
        port,
        &path,
        secure,
        &headers,
        payload.as_bytes(),
        cancel,
        |data_line| {
            if !shared.is_current(gen, cancel) || data_line == "[DONE]" {
                return;
            }
            if let Some(delta) = extract_json_string_field(data_line, "content") {
                if !delta.is_empty() {
                    accumulated.push_str(&delta);
                    let visible = tools::strip_tool_tags(&accumulated);
                    shared.update_snapshot(|snap| {
                        snap.phase = AssistantPhase::Speaking;
                        snap.reply = visible;
                    });
                }
            }
        },
    )?;

    if accumulated.trim().is_empty() {
        return Err("AI tidak mengembalikan teks jawaban.".to_string());
    }
    Ok(accumulated)
}

// =============================================================================
// Native WinHTTP Client (HTTPS + SSE Streaming)
// =============================================================================

struct WinHttpHandle(*mut c_void);

impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn parse_url(url: &str) -> (String, u16, String, bool) {
    let trimmed = url.trim().trim_end_matches('/');
    let (secure, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = trimmed.strip_prefix("http://") {
        (false, r)
    } else {
        (true, trimmed)
    };
    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], rest[idx..].to_string()),
        None => (rest, String::new()),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().unwrap_or(if secure { 443 } else { 80 }),
        ),
        None => (host_port.to_string(), if secure { 443 } else { 80 }),
    };
    (host, port, path, secure)
}

fn open_winhttp_request(
    host: &str,
    port: u16,
    path: &str,
    secure: bool,
    headers: &str,
    body: &[u8],
) -> Result<(WinHttpHandle, WinHttpHandle, WinHttpHandle, u32), String> {
    unsafe {
        let session_raw = WinHttpOpen(
            w!("ZekrimaAssistant/0.3"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session_raw.is_null() {
            return Err("WinHttpOpen gagal".to_string());
        }
        let session = WinHttpHandle(session_raw);
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 20_000, 45_000);

        let host_w = to_wide(host);
        let conn_raw = WinHttpConnect(session.0, PCWSTR(host_w.as_ptr()), port, 0);
        if conn_raw.is_null() {
            return Err(format!("Tidak bisa terhubung ke {host}"));
        }
        let conn = WinHttpHandle(conn_raw);

        let path_w = to_wide(path);
        let flags = if secure { WINHTTP_FLAG_SECURE } else { Default::default() };
        let req_raw = WinHttpOpenRequest(
            conn.0,
            w!("POST"),
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        );
        if req_raw.is_null() {
            return Err("WinHttpOpenRequest gagal".to_string());
        }
        let req = WinHttpHandle(req_raw);

        let headers_w: Vec<u16> = headers.encode_utf16().collect();
        let body_ptr = if body.is_empty() {
            None
        } else {
            Some(body.as_ptr() as *const c_void)
        };

        WinHttpSendRequest(
            req.0,
            Some(&headers_w),
            body_ptr,
            body.len() as u32,
            body.len() as u32,
            0,
        )
        .map_err(|_| "Gagal mengirim request (cek koneksi internet).".to_string())?;

        WinHttpReceiveResponse(req.0, std::ptr::null_mut())
            .map_err(|_| "Tidak ada respons dari server AI.".to_string())?;

        let mut status_code: u32 = 0;
        let mut status_size = std::mem::size_of::<u32>() as u32;
        let _ = WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status_code as *mut u32 as *mut c_void),
            &mut status_size,
            std::ptr::null_mut(),
        );

        Ok((session, conn, req, status_code))
    }
}

pub(crate) fn http_post_raw(
    host: &str,
    port: u16,
    path: &str,
    secure: bool,
    headers: &str,
    body: &[u8],
) -> Result<(u32, Vec<u8>), String> {
    let (_session, _conn, req, status) =
        open_winhttp_request(host, port, path, secure, headers, body)?;

    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let mut read = 0u32;
        unsafe {
            if WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read)
                .is_err()
            {
                break;
            }
        }
        if read == 0 {
            break;
        }
        out.extend_from_slice(&buf[..read as usize]);
    }

    Ok((status, out))
}

pub(crate) fn http_get_raw(
    host: &str,
    port: u16,
    path: &str,
    secure: bool,
    headers: &str,
) -> Result<(u32, Vec<u8>), String> {
    unsafe {
        let session_raw = WinHttpOpen(
            w!("ZekrimaCicada/0.4"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session_raw.is_null() {
            return Err("WinHttpOpen gagal".to_string());
        }
        let session = WinHttpHandle(session_raw);
        let _ = WinHttpSetTimeouts(session.0, 6_000, 6_000, 8_000, 12_000);

        let host_w = to_wide(host);
        let conn_raw = WinHttpConnect(session.0, PCWSTR(host_w.as_ptr()), port, 0);
        if conn_raw.is_null() {
            return Err(format!("Tidak bisa terhubung ke {host}"));
        }
        let conn = WinHttpHandle(conn_raw);

        let path_w = to_wide(path);
        let flags = if secure { WINHTTP_FLAG_SECURE } else { Default::default() };
        let req_raw = WinHttpOpenRequest(
            conn.0,
            w!("GET"),
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        );
        if req_raw.is_null() {
            return Err("WinHttpOpenRequest gagal".to_string());
        }
        let req = WinHttpHandle(req_raw);

        let headers_w: Vec<u16> = headers.encode_utf16().collect();
        WinHttpSendRequest(
            req.0,
            if headers_w.is_empty() { None } else { Some(&headers_w) },
            None,
            0,
            0,
            0,
        )
        .map_err(|_| "Gagal mengirim HTTP GET".to_string())?;

        WinHttpReceiveResponse(req.0, std::ptr::null_mut())
            .map_err(|_| "Tidak ada respons HTTP GET".to_string())?;

        let mut status_code: u32 = 0;
        let mut status_size = std::mem::size_of::<u32>() as u32;
        let _ = WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status_code as *mut u32 as *mut c_void),
            &mut status_size,
            std::ptr::null_mut(),
        );

        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read)
                .is_err()
                || read == 0
            {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
        }
        Ok((status_code, out))
    }
}

#[allow(clippy::too_many_arguments)]
fn http_post_sse<F>(
    host: &str,
    port: u16,
    path: &str,
    secure: bool,
    headers: &str,
    body: &[u8],
    cancel: &AtomicBool,
    mut on_data: F,
) -> Result<(), String>
where
    F: FnMut(&str),
{
    let (_session, _conn, req, status) =
        open_winhttp_request(host, port, path, secure, headers, body)?;

    let mut buf = [0u8; 4096];
    if !(200..300).contains(&status) {
        let mut err_body = Vec::new();
        loop {
            let mut read = 0u32;
            unsafe {
                if WinHttpReadData(
                    req.0,
                    buf.as_mut_ptr() as *mut c_void,
                    buf.len() as u32,
                    &mut read,
                )
                .is_err()
                    || read == 0
                {
                    break;
                }
            }
            err_body.extend_from_slice(&buf[..read as usize]);
            if err_body.len() > 8192 {
                break;
            }
        }
        let err_text = String::from_utf8_lossy(&err_body);
        let msg = extract_json_string_field(&err_text, "message")
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(format!("AI error ({status}): {msg}"));
    }

    let mut pending = String::new();
    loop {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let mut read = 0u32;
        unsafe {
            if WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read)
                .is_err()
            {
                break;
            }
        }
        if read == 0 {
            break;
        }
        pending.push_str(&String::from_utf8_lossy(&buf[..read as usize]));

        while let Some(nl) = pending.find('\n') {
            let line = pending[..nl].trim_end_matches('\r').to_string();
            pending.drain(..=nl);
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if !data.is_empty() {
                    on_data(data);
                }
            }
        }
    }

    if !pending.is_empty() && !cancel.load(Ordering::SeqCst) {
        if let Some(data) = pending.trim().strip_prefix("data:") {
            let data = data.trim();
            if !data.is_empty() {
                on_data(data);
            }
        }
    }

    Ok(())
}

// =============================================================================
// Lightweight JSON & Base64 helpers (zero external dependencies)
// =============================================================================

pub(crate) fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Extracts the decoded string value of the first matching `"key": "..."` field in JSON.
pub fn extract_json_string_field(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let bytes = json.as_bytes();
    let mut search_from = 0usize;

    while let Some(rel) = json[search_from..].find(&pattern) {
        let mut idx = search_from + rel + pattern.len();
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() || bytes[idx] != b':' {
            search_from = idx;
            continue;
        }
        idx += 1;
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx < bytes.len() && bytes[idx] == b'"' {
            return parse_json_string(&json[idx + 1..]);
        }
        search_from = idx;
    }
    None
}

fn parse_json_string(rest: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{0008}'),
                'f' => out.push('\u{000C}'),
                'u' => {
                    let code = read_hex4(&mut chars)?;
                    if (0xD800..=0xDBFF).contains(&code) {
                        // UTF-16 surrogate pair
                        if chars.next() == Some('\\') && chars.next() == Some('u') {
                            if let Some(low) = read_hex4(&mut chars) {
                                let combined = 0x10000
                                    + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                if let Some(ch) = char::from_u32(combined) {
                                    out.push(ch);
                                }
                            }
                        }
                    } else if let Some(ch) = char::from_u32(code as u32) {
                        out.push(ch);
                    }
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn read_hex4<I: Iterator<Item = char>>(iter: &mut I) -> Option<u16> {
    let mut val = 0u16;
    for _ in 0..4 {
        let d = iter.next()?.to_digit(16)? as u16;
        val = (val << 4) | d;
    }
    Some(val)
}

pub(crate) fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for b in input.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' | b'\t' => continue,
            _ => return None,
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

// =============================================================================
// Live API Model Discovery (Auto-detect available models in Settings)
// =============================================================================

/// Queries the provider's `/models` API using the given key/base URL and returns
/// a clean, prioritized list of available chat/multimodal model IDs.
pub fn fetch_available_models(
    provider: LlmProvider,
    api_key: &str,
    custom_base_url: &str,
) -> Result<Vec<String>, String> {
    let key = api_key.trim();
    if key.is_empty() && !matches!(provider, LlmProvider::OpenRouter | LlmProvider::OpenAi) {
        return Err("Masukkan API key untuk mendeteksi model otomatis.".to_string());
    }

    let (status, resp_bytes) = match provider {
        LlmProvider::Gemini => {
            let path = format!("/v1beta/models?pageSize=200&key={key}");
            http_get_raw("generativelanguage.googleapis.com", 443, &path, true, "")?
        }
        LlmProvider::OpenRouter => {
            let headers = if key.is_empty() {
                "Accept: application/json\r\n".to_string()
            } else {
                format!("Authorization: Bearer {key}\r\nAccept: application/json\r\n")
            };
            http_get_raw("openrouter.ai", 443, "/api/v1/models", true, &headers)?
        }
        LlmProvider::OpenCode | LlmProvider::Nvidia | LlmProvider::Groq | LlmProvider::OpenAi => {
            let base = if provider == LlmProvider::OpenAi && !custom_base_url.trim().is_empty() {
                custom_base_url.trim()
            } else {
                provider.default_base_url()
            };
            let (host, port, base_path, secure) = parse_url(base);
            let path = format!("{}/models", base_path.trim_end_matches('/'));
            let headers = if key.is_empty() {
                "Accept: application/json\r\n".to_string()
            } else {
                format!("Authorization: Bearer {key}\r\nAccept: application/json\r\n")
            };
            http_get_raw(&host, port, &path, secure, &headers)?
        }
    };

    let body = String::from_utf8_lossy(&resp_bytes);
    if !(200..300).contains(&status) {
        let msg = extract_json_string_field(&body, "message")
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(if status == 401 || status == 403 {
            format!("API key tidak valid ({status}): {msg}")
        } else {
            format!("Gagal memuat model ({status}): {msg}")
        });
    }

    let models = match provider {
        LlmProvider::Gemini => parse_gemini_models_json(&body),
        _ => parse_openai_models_json(&body, provider),
    };

    if models.is_empty() {
        return Err("API merespons sukses, tetapi daftar model kosong.".to_string());
    }
    Ok(models)
}

fn parse_gemini_models_json(json: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let needle = "\"name\"";
    let mut positions = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = json[from..].find(needle) {
        let pos = from + rel;
        positions.push(pos);
        from = pos + needle.len();
    }

    for (idx, &start) in positions.iter().enumerate() {
        let end = positions.get(idx + 1).copied().unwrap_or(json.len());
        let block = &json[start..end];
        let Some(raw_name) = extract_json_string_field(block, "name") else {
            continue;
        };
        let Some(model_id) = raw_name.strip_prefix("models/") else {
            continue;
        };
        // Only keep generative chat/multimodal models (skip pure embedding / video / image-only)
        let lower = model_id.to_ascii_lowercase();
        let supports_gen = block.contains("generateContent") || !block.contains("supportedGenerationMethods");
        let is_non_chat = lower.contains("embedding")
            || lower.contains("aqa")
            || lower.contains("imagen")
            || lower.contains("veo")
            || lower.contains("lyria");
        if supports_gen && !is_non_chat && seen.insert(model_id.to_string()) {
            found.push(model_id.to_string());
        }
    }

    // Prioritize newer Gemini Flash / Pro models at the top
    found.sort_by(|a, b| {
        let score = |s: &str| -> (u8, String) {
            let l = s.to_ascii_lowercase();
            let tier = if l.contains("tts") {
                3
            } else if l.starts_with("gemini-") && l.contains("flash") && !l.contains("1.5") {
                0
            } else if l.starts_with("gemini-") {
                1
            } else {
                2
            };
            (tier, l)
        };
        let sa = score(a);
        let sb = score(b);
        sa.0.cmp(&sb.0).then_with(|| sb.1.cmp(&sa.1))
    });
    found.truncate(120);
    found
}

fn parse_openai_models_json(json: &str, provider: LlmProvider) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Extract every `"id": "..."` field (standard OpenAI / OpenRouter / Groq / NVIDIA / OpenCode)
    let bytes = json.as_bytes();
    let pattern = "\"id\"";
    let mut search_from = 0usize;
    while let Some(rel) = json[search_from..].find(pattern) {
        let mut idx = search_from + rel + pattern.len();
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx < bytes.len() && bytes[idx] == b':' {
            idx += 1;
            while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
                idx += 1;
            }
            if idx < bytes.len() && bytes[idx] == b'"' {
                if let Some(id) = parse_json_string(&json[idx + 1..]) {
                    let clean = id.trim();
                    let lower = clean.to_ascii_lowercase();
                    let skip = clean.is_empty()
                        || clean.len() > 120
                        || lower.contains("whisper")
                        || lower.contains("embedding")
                        || lower.contains("moderation")
                        || lower.contains("dall-e")
                        || lower.contains("tts-");
                    if !skip && seen.insert(clean.to_string()) {
                        found.push(clean.to_string());
                    }
                }
            }
        }
        search_from = idx.max(search_from + rel + pattern.len());
    }

    match provider {
        LlmProvider::OpenRouter => {
            // Put `openrouter/free` and `:free` models first so free models are easy to pick
            found.sort_by(|a, b| {
                let rank = |s: &str| -> u8 {
                    if s == "openrouter/free" {
                        0
                    } else if s.ends_with(":free") {
                        1
                    } else {
                        2
                    }
                };
                rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
            });
            if !found.iter().any(|m| m == "openrouter/free") {
                found.insert(0, "openrouter/free".to_string());
            }
        }
        _ => {
            found.sort();
        }
    }

    found.truncate(160);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_escaped_json_strings_and_unicode() {
        let sample = r#"{"candidates":[{"content":{"parts":[{"text":"Halo!\nIni \"Zeki\" \u2728"}]}}]}"#;
        assert_eq!(
            extract_json_string_field(sample, "text").as_deref(),
            Some("Halo!\nIni \"Zeki\" ✨")
        );
    }

    #[test]
    fn base64_encodes_and_decodes_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"Zeki"), "WmVraQ==");
        assert_eq!(base64_decode("WmVraQ==").as_deref(), Some(b"Zeki".as_slice()));
        assert_eq!(base64_decode("Zm9v").as_deref(), Some(b"foo".as_slice()));
    }

    #[test]
    fn parses_custom_openai_urls() {
        assert_eq!(
            parse_url("https://api.openai.com/v1"),
            ("api.openai.com".to_string(), 443, "/v1".to_string(), true)
        );
        assert_eq!(
            parse_url("http://localhost:11434/v1/"),
            ("localhost".to_string(), 11434, "/v1".to_string(), false)
        );
    }

    #[test]
    fn parses_gemini_and_openrouter_model_lists() {
        let gem = r#"{"models":[{"name":"models/text-embedding-004","supportedGenerationMethods":["embedContent"]},{"name":"models/gemini-3.8-flash","supportedGenerationMethods":["generateContent"]},{"name":"models/gemini-2.5-pro","supportedGenerationMethods":["generateContent"]}]}"#;
        let gm = parse_gemini_models_json(gem);
        assert_eq!(gm, vec!["gemini-3.8-flash", "gemini-2.5-pro"]);

        let or = r#"{"data":[{"id":"openai/gpt-4o"},{"id":"google/gemma-3-27b-it:free"},{"id":"openrouter/free"}]}"#;
        let om = parse_openai_models_json(or, LlmProvider::OpenRouter);
        assert_eq!(
            om,
            vec!["openrouter/free", "google/gemma-3-27b-it:free", "openai/gpt-4o"]
        );
    }
}
