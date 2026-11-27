# Zekrima Dynamic Island — Android Edition (`v0.4.0`)

Native Kotlin + Android `Canvas` floating Dynamic Island, AI Personal Assistant (**Zeki**), and **Cicada Mode** Live Meeting Translator for **Android 10+ (API 29 – 36)**.

## Features
- **Floating Dynamic Island Overlay (`IslandOverlayService`)**:
  - Smooth 60/120 Hz damped spring physics with **0.0% idle CPU** when tucked into the top edge after 10 seconds idle.
  - Interactive procedural mascot **Zeki** (7 states, 15m auto-sleep, poke sparks, music dance, audio-reactive Cicada headset, gentle nod, & `TakingNotes`).
  - **7 Tabs**: `Zeki` | `Live` (Cicada) | `Media` | `Timer` | `Clip` | `Sys` | `Notifs`.
- **Cicada Mode (Live Meeting Translator)**:
  - Captures remote meeting audio directly on Android 10+ via **`AudioPlaybackCaptureConfiguration`** (`MediaProjection`) without joining any meeting bot, plus automatic **Speakerphone Mic fallback** for DRM-protected apps.
  - Pure-Kotlin `SpeechSegmenter`, dual-channel `EchoFilter`, 1-step Gemini Native Audio / Whisper STT + Google Translate fallback, Custom Glossary, **Bidirectional Quick-Reply** (auto-copied to clipboard), **Click-Through Live Subtitle Bar (`Pass` / `Unlock`)**, **Auto-Summary**, **Ask Zeki (Live Q&A)**, and Markdown export.
- **Zekrima AI Personal Assistant (BYOK)**:
  - Built-in support for **Google Gemini**, **OpenRouter (`:free`)**, **OpenCode Zen**, **NVIDIA NIM**, **Groq**, and **OpenAI / Custom / Ollama** with **live `/models` auto-detection** in Settings.
  - Zero-Retention **Screen Vision** (`MediaProjection` in-memory JPEG), hands-free **Voice Capture**, and Android **Tool Calling** (`set_volume`, `set_brightness`, `open_app`, notifications).
- **Quick Settings Tile**: Toggle Zekrima Dynamic Island directly from Android's Quick Settings shade.

## Build from Source
```bash
cd zekrima-android
./gradlew testDebugUnitTest assembleRelease
```
Output APK: `app/build/outputs/apk/release/app-release.apk`
