//! BYOK (Bring Your Own Key) configuration for Zekrima Assistant.
//!
//! Stored in `%APPDATA%\Zekrima\assistant.ini` (simple `key = value` format,
//! zero external dependencies). Supports:
//! - Google Gemini (Free tier + native audio)
//! - OpenRouter (Free `:free` models & `openrouter/free` auto-router)
//! - OpenCode Zen (Free models)
//! - NVIDIA NIM (Free open-weight frontier models)
//! - Groq (Free Whisper STT & Llama/Qwen LLMs)
//! - OpenAI / Custom OpenAI-compatible endpoints

use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::thread;

use windows::core::{w, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, ASFW_ANY, SW_SHOWNORMAL,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmProvider {
    Gemini,
    OpenRouter,
    OpenCode,
    Nvidia,
    Groq,
    OpenAi,
}

pub const ALL_LLM_PROVIDERS: [LlmProvider; 6] = [
    LlmProvider::Gemini,
    LlmProvider::OpenRouter,
    LlmProvider::OpenCode,
    LlmProvider::Nvidia,
    LlmProvider::Groq,
    LlmProvider::OpenAi,
];

impl LlmProvider {
    pub fn id(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::OpenRouter => "openrouter",
            Self::OpenCode => "opencode",
            Self::Nvidia => "nvidia",
            Self::Groq => "groq",
            Self::OpenAi => "openai",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Self::OpenRouter,
            "opencode" => Self::OpenCode,
            "nvidia" | "nim" => Self::Nvidia,
            "groq" => Self::Groq,
            "openai" | "custom" => Self::OpenAi,
            _ => Self::Gemini,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Gemini => "Google Gemini (Free Tier + Native Audio)",
            Self::OpenRouter => "OpenRouter (Free Models)",
            Self::OpenCode => "OpenCode Zen (Free Models)",
            Self::Nvidia => "NVIDIA NIM (Free Models)",
            Self::Groq => "Groq (Free Tier)",
            Self::OpenAi => "OpenAI / Custom Endpoint",
        }
    }

    pub fn short_badge(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini",
            Self::OpenRouter => "OpenRouter",
            Self::OpenCode => "OpenCode",
            Self::Nvidia => "NVIDIA",
            Self::Groq => "Groq",
            Self::OpenAi => "OpenAI",
        }
    }

    pub fn key_hint(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini API Key (aistudio.google.com):",
            Self::OpenRouter => "OpenRouter API Key (openrouter.ai/keys):",
            Self::OpenCode => "OpenCode API Key (opencode.ai/auth):",
            Self::Nvidia => "NVIDIA NIM API Key (build.nvidia.com):",
            Self::Groq => "Groq API Key (console.groq.com):",
            Self::OpenAi => "OpenAI / Custom API Key:",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::OpenCode => "https://opencode.ai/zen/v1",
            Self::Nvidia => "https://integrate.api.nvidia.com/v1",
            Self::Groq => "https://api.groq.com/openai/v1",
            Self::OpenAi => "https://api.openai.com/v1",
        }
    }

    /// Curated active free-tier model presets for each provider (user can also type any custom ID).
    pub fn model_presets(self) -> &'static [&'static str] {
        match self {
            Self::Gemini => &[
                "gemini-3.8-flash",
                "gemini-3.8-flash-lite",
                "gemini-4-hydrogen",
                "gemini-3.6-flash",
                "gemini-3.5-flash-lite",
            ],
            Self::OpenRouter => &[
                "openrouter/free",
                "google/gemma-4-31b-it:free",
                "google/gemma-3-27b-it:free",
                "meta-llama/llama-3.3-70b-instruct:free",
                "deepseek/deepseek-r1-0528:free",
                "qwen/qwen3-coder:free",
                "nvidia/nemotron-3-super-120b-a12b:free",
            ],
            Self::OpenCode => &[
                "big-pickle",
                "opencode/big-pickle",
                "opencode/deepseek-v4.1-flash-free",
                "opencode/beam-free",
                "gpt-5-nano",
                "glm-4.7-free",
            ],
            Self::Nvidia => &[
                "meta/llama-3.3-70b-instruct",
                "deepseek-ai/deepseek-r1",
                "deepseek-ai/deepseek-r1-distill-qwen-32b",
                "qwen/qwen3-coder-480b",
                "qwen/qwen2.5-72b-instruct",
                "meta/llama-3.1-405b-instruct",
                "nvidia/llama-3.1-nemotron-70b-instruct",
            ],
            Self::Groq => &[
                "llama-3.3-70b-versatile",
                "llama-3.1-8b-instant",
                "qwen/qwen3-32b",
                "gemma2-9b-it",
            ],
            Self::OpenAi => &["gpt-4o-mini", "gpt-4.1-mini", "gpt-4o"],
        }
    }

    pub fn default_model(self) -> &'static str {
        self.model_presets()[0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttProvider {
    Auto,
    Groq,
    Gemini,
    OpenAi,
}

pub const ALL_STT_PROVIDERS: [SttProvider; 4] = [
    SttProvider::Auto,
    SttProvider::Groq,
    SttProvider::Gemini,
    SttProvider::OpenAi,
];

impl SttProvider {
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Groq => "groq",
            Self::Gemini => "gemini",
            Self::OpenAi => "openai",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "groq" => Self::Groq,
            "gemini" => Self::Gemini,
            "openai" => Self::OpenAi,
            _ => Self::Auto,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "Auto (Groq Whisper -> OpenAI -> Gemini Audio)",
            Self::Groq => "Groq Whisper (Free, Ultra-fast <300ms)",
            Self::Gemini => "Gemini Native Audio (Uses Gemini Key)",
            Self::OpenAi => "OpenAI Whisper (whisper-1)",
        }
    }
}

pub const GEMINI_TTS_MODELS: [&str; 2] = [
    "gemini-3.8-flash-tts",
    "gemini-3.8-flash-lite-tts",
];

pub const GEMINI_TTS_VOICES: [&str; 6] = [
    "Kore",
    "Puck",
    "Aoede",
    "Charon",
    "Fenrir",
    "Zephyr",
];

/// Supported target / reply languages for Cicada Mode (Live Meeting Translator).
pub const CICADA_LANGUAGES: [(&str, &str); 8] = [
    ("id", "Indonesian (ID)"),
    ("en", "English (EN)"),
    ("ja", "Japanese (JA)"),
    ("zh-CN", "Chinese (ZH)"),
    ("ko", "Korean (KO)"),
    ("es", "Spanish (ES)"),
    ("fr", "French (FR)"),
    ("de", "German (DE)"),
];

pub fn cicada_lang_english_name(code: &str) -> &'static str {
    match code.trim().to_ascii_lowercase().as_str() {
        "id" | "ind" => "Indonesian",
        "en" | "en-us" | "en-gb" => "English",
        "ja" | "jp" => "Japanese",
        "zh" | "zh-cn" | "zh-hans" => "Simplified Chinese",
        "zh-tw" | "zh-hant" => "Traditional Chinese",
        "ko" | "kr" => "Korean",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        _ => "Indonesian",
    }
}

pub fn cicada_lang_short_badge(code: &str) -> &'static str {
    match code.trim().to_ascii_lowercase().as_str() {
        "id" | "ind" => "ID",
        "en" | "en-us" | "en-gb" => "EN",
        "ja" | "jp" => "JA",
        "zh" | "zh-cn" | "zh-hans" | "zh-tw" => "ZH",
        "ko" | "kr" => "KO",
        "es" => "ES",
        "fr" => "FR",
        "de" => "DE",
        _ => "ID",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PttHotkey {
    AltSpace,
    CtrlSpace,
    AltZ,
    CtrlShiftSpace,
}

pub const ALL_PTT_HOTKEYS: [PttHotkey; 4] = [
    PttHotkey::AltSpace,
    PttHotkey::CtrlSpace,
    PttHotkey::AltZ,
    PttHotkey::CtrlShiftSpace,
];

impl PttHotkey {
    pub fn id(self) -> &'static str {
        match self {
            Self::AltSpace => "alt+space",
            Self::CtrlSpace => "ctrl+space",
            Self::AltZ => "alt+z",
            Self::CtrlShiftSpace => "ctrl+shift+space",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ctrl+space" => Self::CtrlSpace,
            "alt+z" => Self::AltZ,
            "ctrl+shift+space" => Self::CtrlShiftSpace,
            _ => Self::AltSpace,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AltSpace => "Alt+Space",
            Self::CtrlSpace => "Ctrl+Space",
            Self::AltZ => "Alt+Z",
            Self::CtrlShiftSpace => "Ctrl+Shift+Space",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::AltSpace => "Alt+Spc",
            Self::CtrlSpace => "Ctrl+Spc",
            Self::AltZ => "Alt+Z",
            Self::CtrlShiftSpace => "C+S+Spc",
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::AltSpace => 0,
            Self::CtrlSpace => 1,
            Self::AltZ => 2,
            Self::CtrlShiftSpace => 3,
        }
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::CtrlSpace,
            2 => Self::AltZ,
            3 => Self::CtrlShiftSpace,
            _ => Self::AltSpace,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HideHotkey {
    AltH,
    CtrlShiftH,
    AltX,
}

pub const ALL_HIDE_HOTKEYS: [HideHotkey; 3] = [
    HideHotkey::AltH,
    HideHotkey::CtrlShiftH,
    HideHotkey::AltX,
];

impl HideHotkey {
    pub fn id(self) -> &'static str {
        match self {
            Self::AltH => "alt+h",
            Self::CtrlShiftH => "ctrl+shift+h",
            Self::AltX => "alt+x",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ctrl+shift+h" => Self::CtrlShiftH,
            "alt+x" => Self::AltX,
            _ => Self::AltH,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AltH => "Alt+H",
            Self::CtrlShiftH => "Ctrl+Shift+H",
            Self::AltX => "Alt+X",
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::AltH => 0,
            Self::CtrlShiftH => 1,
            Self::AltX => 2,
        }
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::CtrlShiftH,
            2 => Self::AltX,
            _ => Self::AltH,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenResolution {
    Standard, // 1024px wide (fast & low bandwidth)
    Hd,       // 1600px wide (crisp for small code/PDF text)
}

pub const ALL_SCREEN_RESOLUTIONS: [ScreenResolution; 2] =
    [ScreenResolution::Standard, ScreenResolution::Hd];

impl ScreenResolution {
    pub fn id(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Hd => "hd",
        }
    }

    pub fn from_id(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "hd" | "1600" | "high" => Self::Hd,
            _ => Self::Standard,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Standard => "Standard (1024px — Cepat & Hemat)",
            Self::Hd => "HD (1600px — Teks Kecil & PDF Tajam)",
        }
    }

    pub fn max_width_px(self) -> u32 {
        match self {
            Self::Standard => 1024,
            Self::Hd => 1600,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AssistantConfig {
    pub llm_provider: LlmProvider,
    pub stt_provider: SttProvider,

    pub gemini_api_key: String,
    pub gemini_model: String,
    pub gemini_audio_model: String,
    pub gemini_tts_model: String,
    pub gemini_tts_voice: String,
    pub gemini_tts_enabled: bool,

    pub openrouter_api_key: String,
    pub openrouter_model: String,

    pub opencode_api_key: String,
    pub opencode_model: String,

    pub nvidia_api_key: String,
    pub nvidia_model: String,

    pub groq_api_key: String,
    pub groq_model: String,

    pub openai_api_key: String,
    pub openai_base_url: String,
    pub openai_model: String,

    pub language: String,
    pub system_prompt: String,
    pub auto_hide: bool,
    pub ptt_hotkey: PttHotkey,
    pub hide_hotkey: HideHotkey,
    pub screen_resolution: ScreenResolution,

    // Cicada Mode (Live Meeting Translator)
    pub cicada_target_lang: String,
    pub cicada_reply_lang: String,
    pub cicada_show_original: bool,
    pub cicada_glossary: String,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            llm_provider: LlmProvider::Gemini,
            stt_provider: SttProvider::Auto,
            gemini_api_key: String::new(),
            gemini_model: LlmProvider::Gemini.default_model().to_string(),
            gemini_audio_model: "gemini-3.8-flash".to_string(),
            gemini_tts_model: GEMINI_TTS_MODELS[0].to_string(),
            gemini_tts_voice: "Kore".to_string(),
            gemini_tts_enabled: true,
            openrouter_api_key: String::new(),
            openrouter_model: LlmProvider::OpenRouter.default_model().to_string(),
            opencode_api_key: String::new(),
            opencode_model: LlmProvider::OpenCode.default_model().to_string(),
            nvidia_api_key: String::new(),
            nvidia_model: LlmProvider::Nvidia.default_model().to_string(),
            groq_api_key: String::new(),
            groq_model: LlmProvider::Groq.default_model().to_string(),
            openai_api_key: String::new(),
            openai_base_url: LlmProvider::OpenAi.default_base_url().to_string(),
            openai_model: LlmProvider::OpenAi.default_model().to_string(),
            language: "id".to_string(),
            system_prompt: "Kamu adalah Zeki, asisten AI personal yang ramah, ringkas, dan cerdas di dalam Dynamic Island Windows. Jawab singkat (maksimal 2-4 kalimat kecuali diminta detail), jelas, dan langsung pada intinya.".to_string(),
            auto_hide: false,
            ptt_hotkey: PttHotkey::AltSpace,
            hide_hotkey: HideHotkey::AltH,
            screen_resolution: ScreenResolution::Standard,
            cicada_target_lang: "id".to_string(),
            cicada_reply_lang: "en".to_string(),
            cicada_show_original: true,
            cicada_glossary: String::new(),
        }
    }
}

impl AssistantConfig {
    /// Returns `%APPDATA%\Zekrima\assistant.ini` (or executable dir fallback).
    pub fn config_path() -> PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("Zekrima").join("assistant.ini");
        }
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("assistant.ini")))
            .unwrap_or_else(|| PathBuf::from("assistant.ini"))
    }

    /// Ensures `assistant.ini` exists on disk.
    pub fn ensure_template_file() -> PathBuf {
        let path = Self::config_path();
        if !path.exists() {
            let _ = Self::default().save();
        }
        path
    }

    pub fn api_key_for(&self, provider: LlmProvider) -> &str {
        match provider {
            LlmProvider::Gemini => &self.gemini_api_key,
            LlmProvider::OpenRouter => &self.openrouter_api_key,
            LlmProvider::OpenCode => &self.opencode_api_key,
            LlmProvider::Nvidia => &self.nvidia_api_key,
            LlmProvider::Groq => &self.groq_api_key,
            LlmProvider::OpenAi => &self.openai_api_key,
        }
    }

    pub fn set_api_key_for(&mut self, provider: LlmProvider, value: String) {
        let clean = value.trim().to_string();
        match provider {
            LlmProvider::Gemini => self.gemini_api_key = clean,
            LlmProvider::OpenRouter => self.openrouter_api_key = clean,
            LlmProvider::OpenCode => self.opencode_api_key = clean,
            LlmProvider::Nvidia => self.nvidia_api_key = clean,
            LlmProvider::Groq => self.groq_api_key = clean,
            LlmProvider::OpenAi => self.openai_api_key = clean,
        }
    }

    pub fn model_for(&self, provider: LlmProvider) -> &str {
        match provider {
            LlmProvider::Gemini => &self.gemini_model,
            LlmProvider::OpenRouter => &self.openrouter_model,
            LlmProvider::OpenCode => &self.opencode_model,
            LlmProvider::Nvidia => &self.nvidia_model,
            LlmProvider::Groq => &self.groq_model,
            LlmProvider::OpenAi => &self.openai_model,
        }
    }

    pub fn set_model_for(&mut self, provider: LlmProvider, value: String) {
        let clean = value.trim().to_string();
        let final_val = if clean.is_empty() {
            provider.default_model().to_string()
        } else {
            clean
        };
        match provider {
            LlmProvider::Gemini => self.gemini_model = final_val,
            LlmProvider::OpenRouter => self.openrouter_model = final_val,
            LlmProvider::OpenCode => self.opencode_model = final_val,
            LlmProvider::Nvidia => self.nvidia_model = final_val,
            LlmProvider::Groq => self.groq_model = final_val,
            LlmProvider::OpenAi => self.openai_model = final_val,
        }
    }

    /// Loads config from `%APPDATA%\Zekrima\assistant.ini` with env-var fallbacks.
    pub fn load() -> Self {
        let mut cfg = Self::default();
        let path = Self::config_path();

        if let Ok(contents) = fs::read_to_string(&path) {
            cfg.apply_ini(&contents);
        }

        // Environment variable fallbacks when file keys are left blank
        let env_fill = |slot: &mut String, var: &str| {
            if slot.is_empty() {
                if let Ok(val) = std::env::var(var) {
                    *slot = val.trim().to_string();
                }
            }
        };
        env_fill(&mut cfg.gemini_api_key, "GEMINI_API_KEY");
        env_fill(&mut cfg.openrouter_api_key, "OPENROUTER_API_KEY");
        env_fill(&mut cfg.opencode_api_key, "OPENCODE_API_KEY");
        env_fill(&mut cfg.nvidia_api_key, "NVIDIA_API_KEY");
        env_fill(&mut cfg.groq_api_key, "GROQ_API_KEY");
        env_fill(&mut cfg.openai_api_key, "OPENAI_API_KEY");

        cfg
    }

    /// Saves the current configuration to `%APPDATA%\Zekrima\assistant.ini`.
    pub fn save(&self) -> io::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = self.to_ini();
        fs::write(&path, content)
    }

    pub fn to_ini(&self) -> String {
        let clean_prompt = self.system_prompt.replace(['\r', '\n'], " ");
        format!(
            "# Zekrima Assistant Configuration (BYOK - Bring Your Own Key)\n\
             # Managed automatically by the Zekrima Settings window (you can also edit manually).\n\n\
             # LLM provider: gemini | openrouter | opencode | nvidia | groq | openai\n\
             llm_provider = {}\n\n\
             # STT provider: auto | groq | gemini | openai\n\
             stt_provider = {}\n\n\
             # --- Google Gemini (Active 3.8 Flash for Chat & Audio/TTS) ---\n\
             gemini_api_key = {}\n\
             gemini_model = {}\n\
             gemini_audio_model = {}\n\
             gemini_tts_model = {}\n\
             gemini_tts_voice = {}\n\
             gemini_tts_enabled = {}\n\n\
             # --- OpenRouter (Free :free models & openrouter/free) ---\n\
             openrouter_api_key = {}\n\
             openrouter_model = {}\n\n\
             # --- OpenCode Zen (Free Models) ---\n\
             opencode_api_key = {}\n\
             opencode_model = {}\n\n\
             # --- NVIDIA NIM (Free Models at build.nvidia.com) ---\n\
             nvidia_api_key = {}\n\
             nvidia_model = {}\n\n\
             # --- Groq (Ultra-fast Free Whisper STT & LLM) ---\n\
             groq_api_key = {}\n\
             groq_model = {}\n\n\
             # --- OpenAI / Custom OpenAI-Compatible ---\n\
             openai_api_key = {}\n\
             openai_base_url = {}\n\
             openai_model = {}\n\n\
             # Speech language hint (id, en, or auto)\n\
             language = {}\n\n\
             # Assistant personality\n\
             system_prompt = {}\n\n\
             # Auto-hide Dynamic Island when idle (true | false)\n\
             auto_hide = {}\n\
             # Push-to-Talk hotkey: alt+space | ctrl+space | alt+z | ctrl+shift+space\n\
             ptt_hotkey = {}\n\
             # Hide/Show hotkey: alt+h | ctrl+shift+h | alt+x\n\
             hide_hotkey = {}\n\
             # Screen Vision resolution: standard (1024px) | hd (1600px)\n\
             screen_resolution = {}\n\n\
             # --- Cicada Mode (Live Meeting Translator) ---\n\
             cicada_target_lang = {}\n\
             cicada_reply_lang = {}\n\
             cicada_show_original = {}\n\
             cicada_glossary = {}\n",
            self.llm_provider.id(),
            self.stt_provider.id(),
            self.gemini_api_key,
            self.gemini_model,
            self.gemini_audio_model,
            self.gemini_tts_model,
            self.gemini_tts_voice,
            if self.gemini_tts_enabled { "true" } else { "false" },
            self.openrouter_api_key,
            self.openrouter_model,
            self.opencode_api_key,
            self.opencode_model,
            self.nvidia_api_key,
            self.nvidia_model,
            self.groq_api_key,
            self.groq_model,
            self.openai_api_key,
            self.openai_base_url,
            self.openai_model,
            self.language,
            clean_prompt,
            if self.auto_hide { "true" } else { "false" },
            self.ptt_hotkey.id(),
            self.hide_hotkey.id(),
            self.screen_resolution.id(),
            self.cicada_target_lang,
            self.cicada_reply_lang,
            if self.cicada_show_original { "true" } else { "false" },
            self.cicada_glossary.replace(['\r', '\n'], "; "),
        )
    }

    fn apply_ini(&mut self, contents: &str) {
        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with(';')
                || line.starts_with('[')
            {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().trim_matches('"').to_string();
            if val.is_empty() {
                continue;
            }
            match key.as_str() {
                "llm_provider" => self.llm_provider = LlmProvider::from_id(&val),
                "stt_provider" => self.stt_provider = SttProvider::from_id(&val),
                "gemini_api_key" => self.gemini_api_key = val,
                "gemini_model" => {
                    // Auto-migrate deprecated Gemini 2.0 / 2.5 models to active Gemini 3.8 Flash
                    self.gemini_model = if is_deprecated_gemini_model(&val) {
                        "gemini-3.8-flash".to_string()
                    } else {
                        val
                    };
                }
                "gemini_audio_model" => {
                    self.gemini_audio_model = if is_deprecated_gemini_model(&val) {
                        "gemini-3.8-flash".to_string()
                    } else {
                        val
                    };
                }
                "gemini_tts_model" => self.gemini_tts_model = val,
                "gemini_tts_voice" => self.gemini_tts_voice = val,
                "gemini_tts_enabled" => {
                    self.gemini_tts_enabled =
                        matches!(val.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
                }
                "openrouter_api_key" => self.openrouter_api_key = val,
                "openrouter_model" => self.openrouter_model = val,
                "opencode_api_key" => self.opencode_api_key = val,
                "opencode_model" => self.opencode_model = val,
                "nvidia_api_key" => self.nvidia_api_key = val,
                "nvidia_model" => self.nvidia_model = val,
                "groq_api_key" => self.groq_api_key = val,
                "groq_model" => self.groq_model = val,
                "openai_api_key" => self.openai_api_key = val,
                "openai_base_url" => {
                    self.openai_base_url = val.trim_end_matches('/').to_string()
                }
                "openai_model" => self.openai_model = val,
                "language" => self.language = val,
                "system_prompt" => self.system_prompt = val,
                "auto_hide" => {
                    self.auto_hide =
                        matches!(val.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
                }
                "ptt_hotkey" => self.ptt_hotkey = PttHotkey::from_id(&val),
                "hide_hotkey" => self.hide_hotkey = HideHotkey::from_id(&val),
                "screen_resolution" => self.screen_resolution = ScreenResolution::from_id(&val),
                "cicada_target_lang" => self.cicada_target_lang = val,
                "cicada_reply_lang" => self.cicada_reply_lang = val,
                "cicada_show_original" => {
                    self.cicada_show_original =
                        matches!(val.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
                }
                "cicada_glossary" => self.cicada_glossary = val,
                _ => {}
            }
        }
    }

    pub fn has_any_key(&self) -> bool {
        ALL_LLM_PROVIDERS
            .iter()
            .any(|&p| !self.api_key_for(p).is_empty())
    }

    /// Opens raw `assistant.ini` in Notepad (non-blocking).
    pub fn open_in_editor() {
        let path = Self::ensure_template_file();
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }

        if Command::new("notepad.exe").arg(&path).spawn().is_ok() {
            return;
        }

        let _ = thread::Builder::new()
            .name("zekrima-open-config".to_owned())
            .spawn(move || {
                let wide: Vec<u16> = path
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                unsafe {
                    let _ = ShellExecuteW(
                        None,
                        w!("open"),
                        PCWSTR(wide.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    );
                }
            });
    }
}

fn is_deprecated_gemini_model(model: &str) -> bool {
    let lower = model.trim().to_ascii_lowercase();
    lower.starts_with("gemini-2.5-")
        || lower.starts_with("gemini-2.0-")
        || lower.starts_with("gemini-1.5-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_roundtrips_all_providers() {
        let mut cfg = AssistantConfig::default();
        cfg.llm_provider = LlmProvider::OpenRouter;
        cfg.stt_provider = SttProvider::Groq;
        cfg.openrouter_api_key = "sk-or-test".to_string();
        cfg.openrouter_model = "openrouter/free".to_string();
        cfg.opencode_api_key = "oc-test".to_string();
        cfg.opencode_model = "big-pickle".to_string();
        cfg.nvidia_api_key = "nvapi-test".to_string();
        cfg.nvidia_model = "meta/llama-3.3-70b-instruct".to_string();
        cfg.groq_api_key = "gsk-test".to_string();
        cfg.ptt_hotkey = PttHotkey::CtrlSpace;
        cfg.hide_hotkey = HideHotkey::CtrlShiftH;
        cfg.screen_resolution = ScreenResolution::Hd;
        cfg.gemini_tts_voice = "Puck".to_string();
        cfg.auto_hide = true;

        let ini = cfg.to_ini();
        let mut loaded = AssistantConfig::default();
        loaded.apply_ini(&ini);

        assert_eq!(loaded.llm_provider, LlmProvider::OpenRouter);
        assert_eq!(loaded.stt_provider, SttProvider::Groq);
        assert_eq!(loaded.openrouter_api_key, "sk-or-test");
        assert_eq!(loaded.openrouter_model, "openrouter/free");
        assert_eq!(loaded.opencode_api_key, "oc-test");
        assert_eq!(loaded.opencode_model, "big-pickle");
        assert_eq!(loaded.nvidia_api_key, "nvapi-test");
        assert_eq!(loaded.nvidia_model, "meta/llama-3.3-70b-instruct");
        assert_eq!(loaded.groq_api_key, "gsk-test");
        assert_eq!(loaded.ptt_hotkey, PttHotkey::CtrlSpace);
        assert_eq!(loaded.hide_hotkey, HideHotkey::CtrlShiftH);
        assert_eq!(loaded.screen_resolution, ScreenResolution::Hd);
        assert_eq!(loaded.gemini_tts_voice, "Puck");
        assert!(loaded.auto_hide);
        assert!(loaded.has_any_key());
    }

    #[test]
    fn every_provider_has_free_model_presets() {
        for p in ALL_LLM_PROVIDERS {
            assert!(!p.model_presets().is_empty());
            assert!(!p.default_model().is_empty());
            assert!(!p.default_base_url().is_empty());
        }
        assert_eq!(LlmProvider::Gemini.default_model(), "gemini-3.8-flash");
    }

    #[test]
    fn auto_migrates_deprecated_gemini_2_5_to_3_8_flash() {
        let mut cfg = AssistantConfig::default();
        cfg.apply_ini("gemini_model = gemini-2.5-flash\ngemini_audio_model = gemini-2.0-flash\n");
        assert_eq!(cfg.gemini_model, "gemini-3.8-flash");
        assert_eq!(cfg.gemini_audio_model, "gemini-3.8-flash");
    }
}
