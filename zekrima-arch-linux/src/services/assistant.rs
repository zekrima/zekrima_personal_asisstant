//! Background Voice & Chat AI pipeline for Arch Linux / CachyOS.
//!
//! Uses system `curl` (already in `PKGBUILD`) with ephemeral `0600` header config
//! files so API keys NEVER appear in process command-line arguments (`ps aux`),
//! supporting HTTPS + real-time SSE streaming across:
//! - Google Gemini 3.8 Flash (Chat, Multimodal Audio/Image/PDF, and Gemini 3.8 Flash TTS)
//! - OpenRouter (Free), OpenCode Zen (Free), NVIDIA NIM (Free), Groq, OpenAI / Custom
//! - Groq / OpenAI Whisper STT
//! - PipeWire TTS audio playback (`paplay` / `pw-play` stdin pipe, zero temp audio files)

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::services::config::{AssistantConfig, LlmProvider, SttProvider};
use crate::services::files::{AttachmentData, FileAttachment};
use crate::services::notifications::NotificationSnapshot;
use crate::services::screen;
use crate::services::tools;
use crate::services::voice::{CaptureMode, RecordingResult, VoiceRecorder};

const MAX_HISTORY_TURNS: usize = 6;

static ACTIVE_TTS_PID: AtomicU32 = AtomicU32::new(0);
static TEMP_FILE_SEQ: AtomicU64 = AtomicU64::new(1);

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
    pub screen_access_enabled: bool,
    pub screen_capturing_now: bool,
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
            screen_access_enabled: false,
            screen_capturing_now: false,
            last_tool_badges: Vec::new(),
        }
    }
}

impl AssistantSnapshot {
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
    role: &'static str,
    text: String,
}

struct SharedState {
    on_update: Arc<dyn Fn() + Send + Sync>,
    snapshot: Mutex<AssistantSnapshot>,
    history: Mutex<Vec<ChatTurn>>,
    generation: AtomicU64,
    cancel_flag: Mutex<Arc<AtomicBool>>,
}

impl SharedState {
    fn notify_ui(&self) {
        (self.on_update)();
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
    pub fn start(on_update: Arc<dyn Fn() + Send + Sync>) -> Self {
        let cfg = AssistantConfig::load();
        let initial = AssistantSnapshot {
            phase: AssistantPhase::Idle,
            transcript: String::new(),
            reply: String::new(),
            messages: Vec::new(),
            error: None,
            provider_label: provider_badge(&cfg),
            has_config_key: cfg.has_any_key(),
            screen_access_enabled: false,
            screen_capturing_now: false,
            last_tool_badges: Vec::new(),
        };

        Self {
            shared: Arc::new(SharedState {
                on_update,
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
            .unwrap_or(false)
    }

    pub fn toggle_screen_access(&self) -> bool {
        let mut enabled = false;
        self.shared.update_snapshot(|snap| {
            snap.screen_access_enabled = !snap.screen_access_enabled;
            if !snap.screen_access_enabled {
                snap.screen_capturing_now = false;
            }
            enabled = snap.screen_access_enabled;
        });
        enabled
    }

    pub fn start_listening(&self, mode: CaptureMode, req_ctx: RequestContext) {
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
                // Zero Retention: drop screenshot from RAM immediately
                drop(jpeg_opt);
                drop(bin_att);
                shared.update_snapshot(|snap| snap.screen_capturing_now = false);
                finalize_llm_result(&shared, gen, &cancel, &cfg, clean, res);
            });
    }

    pub fn stop_listening(&self) {
        if let Ok(slot) = self.recorder.lock() {
            if let Some(rec) = slot.as_ref() {
                rec.stop();
            }
        }
    }

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
        stop_tts_playback();
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
            snap.error = Some(format!(
                "Tidak ada suara terdeteksi. Klik Voice atau tekan {} sambil bicara.",
                cfg.ptt_hotkey.label()
            ));
        });
        return;
    }

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

    shared.update_snapshot(|snap| {
        snap.phase = AssistantPhase::Transcribing;
    });

    let stt_result = match cfg.stt_provider {
        SttProvider::Groq => transcribe_whisper(
            "https://api.groq.com/openai/v1/audio/transcriptions",
            &cfg.groq_api_key,
            "whisper-large-v3-turbo",
            &cfg.language,
            &rec.wav_bytes,
        ),
        SttProvider::OpenAi => transcribe_whisper(
            &format!(
                "{}/audio/transcriptions",
                cfg.openai_base_url.trim_end_matches('/')
            ),
            &cfg.openai_api_key,
            "whisper-1",
            &cfg.language,
            &rec.wav_bytes,
        ),
        SttProvider::Auto => {
            if !cfg.groq_api_key.is_empty() {
                transcribe_whisper(
                    "https://api.groq.com/openai/v1/audio/transcriptions",
                    &cfg.groq_api_key,
                    "whisper-large-v3-turbo",
                    &cfg.language,
                    &rec.wav_bytes,
                )
            } else if !cfg.openai_api_key.is_empty() {
                transcribe_whisper(
                    &format!(
                        "{}/audio/transcriptions",
                        cfg.openai_base_url.trim_end_matches('/')
                    ),
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
            &[
                "HTTP-Referer: https://zekrima.local",
                "X-Title: Zekrima Assistant",
            ],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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

#[allow(clippy::too_many_arguments)]
fn stream_openai_with_vision_fallback(
    base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    extra_headers: &[&str],
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
                    && shared.is_current(gen, cancel) => {}
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

pub(crate) fn transcribe_whisper(
    url: &str,
    api_key: &str,
    model: &str,
    language: &str,
    wav_bytes: &[u8],
) -> Result<String, String> {
    let boundary = "----ZekrimaVoiceBoundary7f9a2c";
    let mut body: Vec<u8> = Vec::with_capacity(wav_bytes.len() + 512);

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
    body.extend_from_slice(model.as_bytes());
    body.extend_from_slice(b"\r\n");

    if !language.trim().is_empty() && !language.eq_ignore_ascii_case("auto") {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"language\"\r\n\r\n");
        body.extend_from_slice(language.trim().as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"voice.wav\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
    body.extend_from_slice(wav_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let auth = format!("Authorization: Bearer {api_key}");
    let ct = format!("Content-Type: multipart/form-data; boundary={boundary}");
    let (status, resp_bytes) = curl_post_raw(url, &[&auth, &ct], &body)?;
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
// Gemini 3.8 Flash Streaming (Chat, Multimodal Audio/Image/PDF, and TTS)
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
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse"
    );
    let api_hdr = format!("x-goog-api-key: {}", cfg.gemini_api_key);
    let headers = [
        api_hdr.as_str(),
        "Content-Type: application/json",
        "Accept: text/event-stream",
    ];

    let mut accumulated = String::new();
    curl_post_sse(&url, &headers, payload.as_bytes(), cancel, |data_line| {
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
    })?;

    if accumulated.trim().is_empty() {
        return Err("Gemini tidak mengembalikan teks jawaban.".to_string());
    }
    Ok(accumulated)
}

fn speak_with_gemini_tts(cfg: &AssistantConfig, text: &str, cancel: &AtomicBool) {
    if cancel.load(Ordering::SeqCst) {
        return;
    }

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

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{tts_model}:generateContent"
    );
    let payload = format!(
        "{{\"contents\":[{{\"role\":\"user\",\"parts\":[{{\"text\":\"{}\"}}]}}],\
          \"generationConfig\":{{\"responseModalities\":[\"AUDIO\"],\
          \"speechConfig\":{{\"voiceConfig\":{{\"prebuiltVoiceConfig\":{{\"voiceName\":\"{}\"}}}}}}}}}}",
        json_escape(&speak_text),
        json_escape(voice),
    );
    let api_hdr = format!("x-goog-api-key: {}", cfg.gemini_api_key);
    let Ok((status, resp_bytes)) = curl_post_raw(
        &url,
        &[api_hdr.as_str(), "Content-Type: application/json"],
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

    let wav = if raw_bytes.starts_with(b"RIFF") {
        raw_bytes
    } else {
        wrap_pcm16_as_wav(&raw_bytes, 24_000)
    };

    play_wav_linux(&wav, cancel);
}

fn play_wav_linux(wav: &[u8], cancel: &AtomicBool) {
    let spawn_player = || {
        Command::new("paplay")
            .args(["--file-format=wav"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .or_else(|_| {
                Command::new("pw-play")
                    .arg("-")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            })
            .or_else(|_| {
                Command::new("aplay")
                    .args(["-q", "-"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            })
    };

    let Ok(mut child) = spawn_player() else {
        return;
    };

    let pid = child.id();
    ACTIVE_TTS_PID.store(pid, Ordering::SeqCst);

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(wav);
    }

    let _ = child.wait();
    ACTIVE_TTS_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::Relaxed).ok();
    if cancel.load(Ordering::SeqCst) {
        stop_tts_playback();
    }
}

fn stop_tts_playback() {
    let pid = ACTIVE_TTS_PID.swap(0, Ordering::SeqCst);
    if pid != 0 {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
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

// =============================================================================
// OpenAI / OpenRouter / OpenCode / NVIDIA / Groq Streaming
// =============================================================================

#[allow(clippy::too_many_arguments)]
fn stream_openai_compatible(
    base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    extra_headers: &[&str],
    user_text: &str,
    img_opt: Option<(&str, &[u8])>,
    history: &[ChatTurn],
    shared: &Arc<SharedState>,
    gen: u64,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

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
    let auth = format!("Authorization: Bearer {api_key}");
    let mut headers: Vec<&str> = vec![
        auth.as_str(),
        "Content-Type: application/json",
        "Accept: text/event-stream",
    ];
    headers.extend_from_slice(extra_headers);

    let mut accumulated = String::new();
    curl_post_sse(&url, &headers, payload.as_bytes(), cancel, |data_line| {
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
    })?;

    if accumulated.trim().is_empty() {
        return Err("AI tidak mengembalikan teks jawaban.".to_string());
    }
    Ok(accumulated)
}

// =============================================================================
// Secure Curl HTTPS / SSE Transport (ephemeral 0600 header config, zero ps leaks)
// =============================================================================

struct EphemeralCurlConfig(PathBuf);

impl EphemeralCurlConfig {
    fn create(headers: &[&str]) -> Result<Self, String> {
        let dir = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let seq = TEMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = dir.join(format!("zekrima-curl-{pid}-{seq}.cfg"));

        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        opts.mode(0o600);

        let mut file = opts
            .open(&path)
            .map_err(|_| "Gagal membuat konfigurasi HTTP sementara.".to_string())?;

        for h in headers {
            let escaped = h.replace('\\', "\\\\").replace('"', "\\\"");
            writeln!(file, "header = \"{escaped}\"")
                .map_err(|_| "Gagal menulis header HTTP.".to_string())?;
        }

        Ok(Self(path))
    }
}

impl Drop for EphemeralCurlConfig {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

const STATUS_MARKER: &[u8] = b"\n__ZEKRIMA_HTTP_STATUS__:";

pub(crate) fn curl_post_raw(url: &str, headers: &[&str], body: &[u8]) -> Result<(u32, Vec<u8>), String> {
    let hdr_cfg = EphemeralCurlConfig::create(headers)?;
    let mut child = Command::new("curl")
        .args([
            "-sS",
            "-X",
            "POST",
            "--connect-timeout",
            "10",
            "--max-time",
            "45",
            "-K",
        ])
        .arg(&hdr_cfg.0)
        .args([
            "--data-binary",
            "@-",
            "-w",
            "\n__ZEKRIMA_HTTP_STATUS__:%{http_code}",
            url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Perintah `curl` tidak ditemukan di sistem.".to_string())?;

    let payload = body.to_vec();
    let writer = child.stdin.take().map(|mut stdin| {
        thread::spawn(move || {
            let _ = stdin.write_all(&payload);
        })
    });

    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut out);
    }
    if let Some(w) = writer {
        let _ = w.join();
    }
    let _ = child.wait();

    if let Some(pos) = find_subslice_rev(&out, STATUS_MARKER) {
        let code_slice = &out[pos + STATUS_MARKER.len()..];
        let code_str = String::from_utf8_lossy(code_slice).trim().to_string();
        let status = code_str.parse::<u32>().unwrap_or(0);
        out.truncate(pos);
        Ok((status, out))
    } else {
        Err("Tidak ada respons HTTP dari server AI (cek koneksi internet).".to_string())
    }
}

pub(crate) fn curl_get_raw(url: &str) -> Result<(u32, Vec<u8>), String> {
    const STATUS_MARKER: &[u8] = b"\n__ZEKRIMA_HTTP_STATUS__:";
    let output = Command::new("curl")
        .args([
            "-sS",
            "--connect-timeout",
            "6",
            "--max-time",
            "12",
            "-w",
            "\n__ZEKRIMA_HTTP_STATUS__:%{http_code}",
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Perintah `curl` tidak ditemukan di sistem.".to_string())?;
    let mut out = output.stdout;
    if let Some(pos) = find_subslice_rev(&out, STATUS_MARKER) {
        let code_slice = &out[pos + STATUS_MARKER.len()..];
        let status = String::from_utf8_lossy(code_slice)
            .trim()
            .parse::<u32>()
            .unwrap_or(0);
        out.truncate(pos);
        Ok((status, out))
    } else {
        Err("Tidak ada respons HTTP GET.".to_string())
    }
}

fn curl_post_sse<F>(
    url: &str,
    headers: &[&str],
    body: &[u8],
    cancel: &AtomicBool,
    mut on_data: F,
) -> Result<(), String>
where
    F: FnMut(&str),
{
    let hdr_cfg = EphemeralCurlConfig::create(headers)?;
    let mut child = Command::new("curl")
        .args([
            "-sS",
            "-N",
            "--no-buffer",
            "-i",
            "-X",
            "POST",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "-K",
        ])
        .arg(&hdr_cfg.0)
        .args(["--data-binary", "@-", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Perintah `curl` tidak ditemukan di sistem.".to_string())?;

    let payload = body.to_vec();
    let writer = child.stdin.take().map(|mut stdin| {
        thread::spawn(move || {
            let _ = stdin.write_all(&payload);
        })
    });

    let mut result = Ok(());
    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut status: u32 = 0;

        // Read HTTP response header block(s) (handles optional 100 Continue)
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).unwrap_or(0);
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.starts_with("HTTP/") {
                if let Some(code_str) = trimmed.split_whitespace().nth(1) {
                    if let Ok(code) = code_str.parse::<u32>() {
                        status = code;
                    }
                }
            } else if trimmed.is_empty() {
                if status == 100 {
                    status = 0;
                    continue;
                }
                break;
            }
        }

        if !(200..300).contains(&status) {
            let mut err_text = String::new();
            let _ = reader.take(8192).read_to_string(&mut err_text);
            let msg = extract_json_string_field(&err_text, "message")
                .unwrap_or_else(|| format!("HTTP {status}"));
            result = Err(format!("AI error ({status}): {msg}"));
        } else {
            let mut line = String::new();
            loop {
                if cancel.load(Ordering::SeqCst) {
                    let _ = child.kill();
                    break;
                }
                line.clear();
                let n = reader.read_line(&mut line).unwrap_or(0);
                if n == 0 {
                    break;
                }
                let trimmed = line.trim_end_matches(['\r', '\n']);
                if let Some(data) = trimmed.strip_prefix("data:") {
                    let data = data.trim();
                    if !data.is_empty() {
                        on_data(data);
                    }
                }
            }
        }
    }

    if cancel.load(Ordering::SeqCst) {
        let _ = child.kill();
    }
    let _ = child.wait();
    if let Some(w) = writer {
        let _ = w.join();
    }

    result
}

fn find_subslice_rev(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
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

pub(crate) fn curl_get_raw_headers(url: &str, headers: &[&str]) -> Result<(u32, Vec<u8>), String> {
    const STATUS_MARKER: &[u8] = b"\n__ZEKRIMA_HTTP_STATUS__:";
    let hdr_cfg = EphemeralCurlConfig::create(headers)?;
    let output = Command::new("curl")
        .args([
            "-sS",
            "--connect-timeout",
            "8",
            "--max-time",
            "15",
            "-K",
        ])
        .arg(&hdr_cfg.0)
        .args([
            "-w",
            "\n__ZEKRIMA_HTTP_STATUS__:%{http_code}",
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Perintah `curl` tidak ditemukan di sistem.".to_string())?;
    let mut out = output.stdout;
    if let Some(pos) = find_subslice_rev(&out, STATUS_MARKER) {
        let code_slice = &out[pos + STATUS_MARKER.len()..];
        let status = String::from_utf8_lossy(code_slice)
            .trim()
            .parse::<u32>()
            .unwrap_or(0);
        out.truncate(pos);
        Ok((status, out))
    } else {
        Err("Tidak ada respons HTTP GET.".to_string())
    }
}

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
            let key_hdr = format!("x-goog-api-key: {key}");
            curl_get_raw_headers(
                "https://generativelanguage.googleapis.com/v1beta/models?pageSize=200",
                &[&key_hdr],
            )?
        }
        LlmProvider::OpenRouter => {
            if key.is_empty() {
                curl_get_raw_headers("https://openrouter.ai/api/v1/models", &[])?
            } else {
                let auth = format!("Authorization: Bearer {key}");
                curl_get_raw_headers("https://openrouter.ai/api/v1/models", &[&auth])?
            }
        }
        LlmProvider::OpenCode | LlmProvider::Nvidia | LlmProvider::Groq | LlmProvider::OpenAi => {
            let base = if provider == LlmProvider::OpenAi && !custom_base_url.trim().is_empty() {
                custom_base_url.trim()
            } else {
                provider.default_base_url()
            };
            let url = format!("{}/models", base.trim_end_matches('/'));
            if key.is_empty() {
                curl_get_raw_headers(&url, &[])?
            } else {
                let auth = format!("Authorization: Bearer {key}");
                curl_get_raw_headers(&url, &[&auth])?
            }
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
        let lower = model_id.to_ascii_lowercase();
        let supports_gen =
            block.contains("generateContent") || !block.contains("supportedGenerationMethods");
        let is_non_chat = lower.contains("embedding")
            || lower.contains("aqa")
            || lower.contains("imagen")
            || lower.contains("veo")
            || lower.contains("lyria");
        if supports_gen && !is_non_chat && seen.insert(model_id.to_string()) {
            found.push(model_id.to_string());
        }
    }

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
        _ => found.sort(),
    }

    found.truncate(160);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_escaped_json_strings_and_unicode() {
        let sample =
            r#"{"candidates":[{"content":{"parts":[{"text":"Halo!\nIni \"Zeki\" \u2728"}]}}]}"#;
        assert_eq!(
            extract_json_string_field(sample, "text").as_deref(),
            Some("Halo!\nIni \"Zeki\" ✨")
        );
    }

    #[test]
    fn base64_encodes_and_decodes_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"Zeki"), "WmVraQ==");
        assert_eq!(base64_decode("WmVraQ==").as_deref(), Some(b"Zeki".as_slice()));
    }
}
