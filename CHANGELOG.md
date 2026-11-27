# Changelog

All notable changes to **Zekrima** will be documented in this file.

## [0.5.0] - 2026-11-27

### Added (Android Edition — `zekrima-android/`)
- **Native Android Dynamic Island (`API 29–36` / Android 10+)**:
  - Pure Kotlin + Android `Canvas` floating overlay (`IslandOverlayService`) with damped spring physics, **0.0% idle CPU** when tucked into the top edge after 10 s, **Quick Settings Tile**, and all 7 tabs (`Zeki`, `Live`, `Media`, `Timer`, `Clip`, `Sys`, `Notifs`).
  - Full **Zeki** procedural mascot (7 states, poke sparks, music dance, audio-reactive Cicada headset, gentle nod, & `TakingNotes`), multi-provider BYOK AI Assistant with **live `/models` auto-detection**, Zero-Retention **Screen Vision** (`MediaProjection`), and real Android hardware tools (`AudioManager`, `Settings.System`, `PackageManager`, `NotificationListenerService`, `MediaSessionManager`).
  - Full **Cicada Mode (Live Meeting Translator)** using Android 10+ **`AudioPlaybackCaptureConfiguration`** direct system audio loopback (+ automatic Speakerphone Mic fallback), pure-Kotlin `SpeechSegmenter` & `EchoFilter`, Click-Through **Live Subtitle Bar (`Pass` / `Unlock`)**, **Bidirectional Quick-Reply**, Custom Glossary, Auto-Summary, Live Q&A, and Markdown export.
  - Added `build-android` CI & Release workflow jobs to build, unit-test, and attach `Zekrima-Android.apk`.
- **Dynamic Resolution-Aware Scaling & Responsive Settings (Windows, Arch Linux & Android)**:
  - **Windows**: Added monitor-resolution + DPI-aware `effective_dpi()` scaling (`WM_DISPLAYCHANGE` + `WM_DPICHANGED`) for the Dynamic Island, and upgraded the Win32 Settings window to dynamically size against the monitor work area (`SPI_GETWORKAREA`) with resizing (`WS_THICKFRAME`) and smooth mouse-wheel/vertical scrollbar scrolling so controls and `Save & Apply` never clip on compact or high-DPI screens.
  - **Arch Linux / CachyOS**: Added `compute_ui_scale()` from active `gdk::Monitor::geometry()` scaling Cairo rendering, canvas, input regions, and pointer coordinates, plus dynamic monitor-proportioned GTK4 Settings window sizing.
  - **Android**: Added live `onConfigurationChanged` orientation/foldable/tablet reflow to `IslandOverlayService` (`maxWidthDp` & `maxHeightDp`) and centered responsive container layout + `fitsSystemWindows` to `MainActivity` Settings.
- **Privacy-First Default (`Incognito` ON by Default)**:
  - Screen Vision now defaults to **Incognito (OFF)** on startup across Windows, Arch Linux / CachyOS, and Android; users explicitly toggle `Screen ON` whenever they want Zeki to inspect the screen.

## [0.4.0] - 2026-11-27

### Added (Cicada Mode — Live Meeting Translator on Windows & Arch Linux / CachyOS)
- **Direct System Audio Capture (No Meeting Bots) & Smart Speech Segmenter**:
  - Captures remote meeting audio directly via **WASAPI Loopback** (`AUDCLNT_STREAMFLAGS_LOOPBACK`) on Windows and **PipeWire `@DEFAULT_MONITOR@`** (`parec`) on Arch Linux / CachyOS with **0.0% idle CPU** when stopped.
  - Pure-Rust Cicada `SpeechSegmenter`: 30 ms frames, 10 ms onset click rejection, 100 Hz high-pass rumble rejection, steady-noise rejection, 0.55 s pause endpointing, and 7.5 s quietest-frame forced cuts.
- **Dual-Channel Separation, Echo Deduplication & Bidirectional Quick-Reply**:
  - Live toggleable **`Mic: ON/OFF`** hybrid channel separates **"Online"** (remote speaker) and **"You"** (local mic) with Cicada's `EchoFilter` (timing gate + bigram overlap) to eliminate speaker-bleed duplicates.
  - **`Reply` (Bidirectional Quick-Reply)**: speak in your native language (e.g., Indonesian) and receive an instant translation into `cicada_reply_lang` (e.g., English), auto-copied to clipboard ready to read aloud or paste into meeting chat.
- **Context-Aware Translation & Custom Glossary**:
  - 1-step **Gemini Native Audio** (`LANG:` / `ORIGINAL:` / `TRANSLATION:`) or **Groq/OpenAI Whisper STT + Fast LLM** (with Google Translate fallback), powered by a 4-sentence sliding context window, noise/hallucination filter, and configurable **Custom Glossary** in Settings (`assistant.ini`).
- **3-State Dynamic Island UI & New Zeki Animations**:
  - **Standby / Listening (Collapsed Pill)**: blinking emerald live dot, language pair badge (`EN→ID`), 5-bar live speech waveform, and **Zeki wearing a glowing headset** with audio-reactive earcups.
  - **Live Subtitle Bar (`620×94`)**: semi-transparent bilingual subtitle bar with **Zeki nodding gently** on each new translated line, plus **Click-Through (`Pass` / `Unlock`)** so clicks pass directly to Zoom/Meet/Teams underneath.
  - **Expanded Transcript (`Live` tab, `560×340` ↔ `640×470`)**: **Zeki holding an animated notebook & pen** (`TakingNotes`), scrollable speaker-colored bilingual transcript, quick target-language & original-text toggles, Markdown export (`Copy MD` + auto-save to `meetings/`), one-click **Auto-Summary**, and **Live Q&A (`Ask Zeki`)** injecting the meeting transcript directly into Zekrima Assistant.
  - Added Wayland IPC CLI flags `zekrima --cicada` / `--live` and `zekrima --reply` on Arch Linux / CachyOS.

### Fixed & Improved
- **Reliable Auto-Minimize on Mouse Leave (Windows & Arch Linux)**:
  - Dynamic Island now reliably collapses back to compact pill whenever the cursor leaves the island (unless actively typing a chat draft or an AI turn is mid-flight), backed by a 320 ms cursor-bounds watchdog on Windows so fast cursor exits or missed `WM_MOUSELEAVE` events never leave the pill stuck open.
- **Auto-Detect Available Models from API in Settings (Windows & Arch Linux)**:
  - Settings automatically queries the provider's `/models` endpoint (`Gemini`, `OpenRouter`, `OpenCode Zen`, `NVIDIA NIM`, `Groq`, `OpenAI / Custom / Ollama`) on open, provider switch, and 650 ms after pasting/editing an API key (plus a manual `↻ Cek Model` button), populating the Model dropdown with live available models and clear status feedback.
- **Smooth 10-Second Top-Edge Tuck Auto-Hide & Single-Press `Alt+H` Reveal (Windows & Arch Linux)**:
  - Raised Auto-Hide idle timeout from 4 s to **10 s** and replaced the abrupt invisible fade with a smooth spring tuck into a subtle top-edge notch handle (`128 × 5`) that stays at **0.0% CPU** once settled.
  - Fixed Windows `UpdateLayeredWindow` per-pixel alpha hit-testing (`alpha = 2/255` invisible `240 × 12` top-center sensor + seamless top-bezel hover bridge) so moving the mouse to the top edge of the screen immediately springs the island back into view with zero bounce.
  - Fixed `Alt+H` (or custom Hide hotkey) requiring 2 presses while auto-hidden — pressing it once now immediately reveals the island.

## [0.3.0] - 2026-11-26

### Added (Zekrima Personal Assistant — Windows & Arch Linux / CachyOS Editions)
- **Interactive Mascot "Zeki" & 7-State Procedural Animation Engine**:
  - Pure Direct2D vector mascot (`Idle`, `Sleepy`, `Listening`, `Thinking`, `Speaking`, `Happy`, `Error`) with 15-minute auto-sleep, poke spark reactions, and audio-reactive music dance.
  - Persistent **Zeki Companion Dock** across all expanded sub-menus (`Media`, `Timer`, `Clip`, `Sys`, `Notifs`) with quick access to **Chat** and **Voice Listening**.
  - Media logo positioned top-left above the clock when media is playing.
- **Voice, Chat & Multi-Provider AI Core (BYOK)**:
  - Native **Mode Chat** directly inside Direct2D (`WM_CHAR`, `WM_KEYDOWN`, `Ctrl+V` paste, Enter send, Esc cancel), scrollable multi-turn conversation history (mouse wheel, drag, PgUp/PgDn, scrollbar), and **double-click expand** (`520×310` ↔ `640×470`).
  - Native Win32 **Assistant Settings App** (`Config`) with per-provider API key retention, editable model preset combobox, Show/Hide key toggle, and live `Save & Apply` reload (`%APPDATA%\Zekrima\assistant.ini`).
  - Built-in support for **Google Gemini** (`gemini-3.8-flash` for chat & audio input, `gemini-3.8-flash-tts` / `gemini-3.8-flash-lite-tts` for WAV TTS), **OpenRouter (Free)**, **OpenCode Zen (Free)**, **NVIDIA NIM (Free)**, **Groq**, and **OpenAI / Custom** endpoints.
  - WASAPI `eCapture` 16 kHz mono WAV recorder with Push-to-Talk & Hands-Free energy VAD.
- **Context Awareness, File Attachments & System Control**:
  - Zero-Retention in-memory **Screen Vision** (GDI `BitBlt` + WIC JPEG, configurable Standard `1024px` or HD `1600px`) with **Incognito / Screen ON** privacy toggle and live `● SCREEN` / `● MIC` indicators.
  - **File Attachments** (`Attach` button) for text/code, images, and PDFs.
  - Universal `[[TOOL:...]]` protocol for Volume, Brightness, Open App, and Notification context summary, plus explicit user-triggered `Insert` button to paste AI responses into the previous window.
- **Auto-Hide, Custom Hotkeys & Power Optimizations**:
  - **Auto-Hide when Idle** (fades out after 4s idle with a 5px top-center reveal strip, `0.0%` CPU when hidden) and customizable **Hide/Show** (`Alt+H`, `Ctrl+Shift+H`, `Alt+X`) & **Push-to-Talk** (`Alt+Space`, `Ctrl+Space`, `Alt+Z`, `Ctrl+Shift+Space`) global hotkeys.
  - Selectable **Gemini TTS Voices** (`Kore`, `Puck`, `Aoede`, `Charon`, `Fenrir`, `Zephyr`).
  - Pre-cached Direct2D `ID2D1StrokeStyle` (eliminating per-frame COM allocations) and low-fps (~8 fps) battery-saver mode when collapsed & `Sleepy`.

- **Arch Linux / CachyOS Full Parity (`zekrima-arch-linux/`)**:
  - Full Cairo port of the 7-state **Zeki** procedural mascot engine, persistent Companion Dock, 6-tab bar, top-left Media logo above clock, scrollable & double-click expandable Mode Chat (`KeyboardMode::Exclusive` only while typing).
  - Native GTK4 **Settings Window** (`~/.config/zekrima/assistant.ini` with `0600` perms), PipeWire mic capture (`parec`) + TTS playback (`paplay`/`pw-play`), Zero-Retention Screen Vision (`grim`/`spectacle` + `gdk-pixbuf`), XDG Portal `gtk4::FileDialog` attachments, Linux system tools (`wpctl`, `brightnessctl`, `xdg-open`), and Wayland IPC hotkey commands (`zekrima --toggle-hide`, `--voice`, `--chat`, `--settings`).

## [0.2.0] - 2026-11-26

### Added (Windows & Arch Linux / CachyOS)
- **Unified 5-Tab Expanded Island**: `Media` | `Timer` | `Clip` | `Sys` | `Notifs` bar across the top of the expanded pill (`440 × 162`).
- **Focus / Pomodoro Timer (`Timer` tab)**:
  - Presets (`5m`, `15m`, `25m`, `45m`), `Start`/`Pause`, `Reset`, bottom progress bar, live countdown indicator right in the collapsed pill while running, and automatic Pillar-style completion toast at `00:00`.
- **Quick Clipboard History (`Clip` tab)**:
  - Event-driven clipboard watcher (`AddClipboardFormatListener` / `WM_CLIPBOARDUPDATE` on Windows, native GTK4 `gdk::Clipboard::connect_changed` on Arch Linux/CachyOS) with 0% idle polling.
  - Remembers recent text snippets; click any row to re-copy immediately with green `"Copied to clipboard!"` feedback, plus `Clear` button.
- **Compact System Monitor (`Sys` tab)**:
  - Live **CPU %**, **Memory (Used / Total GB & %)**, and **Battery / AC Power** with color-coded bars (green / amber / red).
  - Sampled strictly on-demand while the `Sys` tab is open (`GetSystemTimes` / `GlobalMemoryStatusEx` / `GetSystemPowerStatus` on Windows; `/proc/stat`, `/proc/meminfo`, `/sys/class/power_supply` on Linux) — `0.0%` idle CPU when closed.

## [0.1.0] - 2026-11-26

### Added
- **Windows 11 / 10 Native Edition (`zekrima-windows/`)**:
  - Pure native Win32 + Direct2D + DirectWrite layered overlay (`~0.42 MB` release binary, `0.0%` idle CPU).
  - Damped harmonic spring physics, per-pixel transparency, click-through hit-testing (`WM_NCHITTEST`), and zero focus stealing (`WS_EX_NOACTIVATE`).
  - Clear bold clock (`HH:MM`) across collapsed pill, expanded idle hero card (+ date), and expanded media card.
  - Real-time 5-band audio-reactive music equalizer using WASAPI Loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`) + pure-Rust 512-point FFT.
  - Event-driven WinRT SMTC media controls (album art with hash caching, title, artist, Previous / Play-Pause / Next).
  - Pillar-style Windows notifications (`UserNotificationListener` with real-time `NotificationChanged`, fallback polling, colored app avatar, incoming toast countdown bar, collapsed badge, expanded list, click-to-activate app via AUMID, and Windows Settings shortcut).
  - Event-driven Volume HUD (`IAudioEndpointVolumeCallback` + `IMMNotificationClient`) and Brightness HUD (`WM_POWERBROADCAST` + WMI/DDC-CI fallback).
  - Native System Tray (`Run at Windows Startup`, `Notification Settings`, `Reset Position`, `Exit`), single-instance mutex, and fullscreen auto-hide (`SHQueryUserNotificationState`).
- **Arch Linux / CachyOS Native Edition (`zekrima-arch-linux/`)**:
  - Native Rust + GTK4 + `gtk4-layer-shell` (Wayland `Layer::Overlay` + X11 fallback) + Cairo/PangoCairo renderer.
  - Dynamic click-through input region (`GdkSurface::set_input_region`) and `KeyboardMode::None` non-activating overlay.
  - Real-time 5-band PipeWire `@DEFAULT_MONITOR@` FFT music visualizer (`parec`).
  - Event-driven MPRIS2 media controls (`playerctl --follow`), Volume HUD (`pactl subscribe` + `wpctl`), Brightness HUD (`udevadm monitor` + `/sys/class/backlight`), and Pillar-style D-Bus notification mirroring.
  - Native Arch `PKGBUILD`, `zekrima.desktop`, and one-command `install-cachyos.sh`.
- **GitHub Automation**:
  - CI build workflow for Windows and Arch Linux.
  - Automated GitHub Release workflow triggered on `v*` tags.
