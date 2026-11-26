# Changelog

All notable changes to **Zekrima** will be documented in this file.

## [0.3.0] - 2026-11-26

### Added (Zekrima Personal Assistant — Windows Edition)
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
