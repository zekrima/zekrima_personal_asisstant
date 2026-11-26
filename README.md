# Zekrima Personal Assistant — Native Dynamic Island Overlay

![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)
![Windows](https://img.shields.io/badge/Windows-11%20%2F%2010-0078D6?logo=windows11)
![Arch Linux](https://img.shields.io/badge/Arch%20Linux-CachyOS-1793D1?logo=archlinux)
![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)

**Zekrima Personal Assistant** adalah aplikasi **Dynamic Island & AI Personal Assistant native** super ringan yang dibangun murni dengan **Rust** — tanpa WebView, Electron, atau Tauri. Dirancang untuk menemani alur kerja harian di **Windows 11/10** maupun **Arch Linux / CachyOS** dengan maskot interaktif **Zeki**, chat & voice AI multimodal (BYOK), screen vision zero-retention, konsumsi `0.0%` idle CPU, animasi *damped harmonic spring*, serta integrasi sistem yang cepat dan tidak mengganggu fokus kerja.

---

## Fitur Saat Ini (`v0.3.0`)

- **Maskot Interaktif "Zeki" & Animasi Prosedural 7-State (Windows)**:
  - Render vektor Direct2D (`Idle`, `Sleepy`, `Listening`, `Thinking`, `Speaking`, `Happy`, `Error`) dengan auto-sleep 15 menit, reaksi klik (*poke sparks*), dan goyang mengikuti musik.
  - **Zeki Companion Dock** tampil di seluruh sub-menu (`Media`, `Timer`, `Clip`, `Sys`, `Notifs`) dengan akses cepat **Chat** & **Voice Listening** kapan saja.
- **AI Chat, Voice & File Assistant (BYOK — Multi-Provider)**:
  - **Mode Chat Native** langsung di Direct2D dengan riwayat multi-turn yang bisa di-scroll (wheel / drag / PgUp-PgDn) dan **klik ganda (double-click) untuk memperbesar chat box** (`520×310` ↔ `640×470`).
  - **Voice Input & TTS**: WASAPI Push-to-Talk & Hands-Free VAD + suara TTS native (`Kore`, `Puck`, `Aoede`, `Charon`, `Fenrir`, `Zephyr`).
  - **Dukungan Provider & Model Gratis**: **Google Gemini** (`gemini-3.8-flash` & `gemini-3.8-flash-tts`), **OpenRouter (Free)**, **OpenCode Zen (Free)**, **NVIDIA NIM (Free)**, **Groq**, dan **OpenAI / Custom** lewat aplikasi pengaturan native (`Config`).
  - **Screen Vision (Zero Retention) & File Attachment**: tangkapan layar in-memory (`Standard 1024px` / `HD 1600px`) dengan tombol **Incognito**, indikator live `● SCREEN` / `● MIC`, lampiran file (teks/kode/gambar/PDF), serta kontrol sistem (volume, brightness, buka aplikasi) & tombol `Insert` eksplisit.
- **Auto-Hide & Custom Global Hotkeys**:
  - **Auto-Hide saat Idle** (pudar setelah 4 detik idle, muncul saat kursor menyentuh tepi atas tengah) dan shortcut global kustom untuk **Hide/Show** (`Alt+H`, `Ctrl+Shift+H`, `Alt+X`) serta **Push-to-Talk** (`Alt+Space`, `Ctrl+Space`, `Alt+Z`, `Ctrl+Shift+Space`).
- **Unified 6-Tab Productivity Bar**: pindah cepat antara **`Zeki`**, **`Media`**, **`Timer`**, **`Clip`**, **`Sys`**, dan **`Notifs`** langsung di dalam pill expanded.
- **Focus / Pomodoro Timer (`Timer`)**
  - Preset `5m`, `15m`, `25m`, `45m`, tombol `Start`/`Pause` & `Reset`, progress bar countdown, indikator hitung mundur aktif di collapsed pill, serta toast notifikasi otomatis saat waktu selesai.
- **Quick Clipboard History (`Clip`)**
  - Event-driven tanpa polling (`WM_CLIPBOARDUPDATE` di Windows, GTK4 `gdk::Clipboard` di Linux). Menyimpan potongan teks terbaru yang di-copy (`Ctrl+C`) — klik baris mana saja untuk copy ulang secara instan.
- **Compact System Monitor (`Sys`)**
  - Memantau **CPU %**, **RAM (Used / Total GB)**, dan **Battery / AC Power** dengan bar berkode warna; hanya di-sample saat tab `Sys` dibuka (`0.0%` idle CPU).
- **Tampilan Jam Tegas & Jelas**
  - Jam **`HH:MM`** tebal dan mudah dibaca di mode collapsed pill (`16.5px` Bold), jam besar **`32px` Bold + tanggal** di mode expanded idle, serta jam **`18.5px` Bold** di pojok kanan kartu media.
- **Equalizer Musik Mengikuti Irama Audio Asli (Real-Time FFT)**
  - 5 band frekuensi musikal (Sub-bass, Bass, Vokal, Melodi, Treble) menggunakan **512-point FFT pure Rust**:
    - **Windows**: WASAPI Loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`).
    - **Arch Linux / CachyOS**: PipeWire `@DEFAULT_MONITOR@` (`parec`).
  - Capture audio & timer animasi otomatis berhenti saat musik di-pause (`0.0%` idle CPU).
- **Kontrol Media Native**
  - Menampilkan album art, judul lagu, artis, dan tombol **Previous / Play-Pause / Next** (WinRT SMTC di Windows, MPRIS2 `playerctl --follow` di Linux).
- **Notifikasi Desktop Bergaya Pillar**
  - **Incoming Toast**: warna aksen otomatis per aplikasi, avatar inisial, judul, isi, tombol dismiss `×`, dan **progress bar countdown ~3,8 detik**.
  - **Collapsed Pill Badge**: avatar warna aplikasi + badge merah jumlah notifikasi (`1`–`9+`).
  - **Expanded Notifications Panel**: daftar notifikasi, klik untuk membuka aplikasi pengirim (Windows AUMID), dismiss satuan, serta **`Clear all`**.
- **Volume & Brightness HUD Event-Driven**
  - Bar HUD sementara saat mengubah volume, mute, atau kecerahan layar tanpa polling terus-menerus.
- **Click-Through, Non-Focus-Stealing & Fullscreen Auto-Hide**
  - Klik di luar area pill selalu tembus ke jendela kerja di bawahnya; klik pill tidak mencuri fokus keyboard.
  - Di Windows, pill otomatis bersembunyi saat aplikasi/game fullscreen dan muncul kembali di lapisan teratas setelah keluar fullscreen.
- **System Tray & Autostart**
  - Dukungan autostart saat login (`HKCU\...\Run` di Windows, XDG `~/.config/autostart/zekrima.desktop` / Hyprland `exec-once` di Linux).

---

## Struktur Proyek

| Platform | Folder | Stack Native | Panduan Lengkap |
| :--- | :--- | :--- | :--- |
| **Windows 11 / 10** | [`zekrima-windows/`](zekrima-windows/) | Win32, Direct2D, DirectWrite, WinRT SMTC, WASAPI | [Panduan Windows](zekrima-windows/README.md) |
| **Arch Linux / CachyOS** | [`zekrima-arch-linux/`](zekrima-arch-linux/) | GTK4, `gtk4-layer-shell` (Wayland), Cairo/Pango, PipeWire, MPRIS2, D-Bus | [Panduan Arch / CachyOS](zekrima-arch-linux/README.md) |

---

## Quick Start & Download Installer

Unduh langsung installer siap pakai dari halaman **[GitHub Releases Terbaru](https://github.com/zekrima/zekrima_personal_asisstant/releases/latest)**:

| Platform | File Installer di GitHub Releases | Cara Pasang |
| :--- | :--- | :--- |
| **Windows 11 / 10 (Setup GUI)** | `Zekrima-Setup-x64.exe` | Klik ganda untuk install (Start Menu, Desktop shortcut, opsi Startup & Uninstaller) |
| **Windows 11 / 10 (Portable)** | `zekrima-windows-portable.zip` / `zekrima.exe` | Ekstrak & langsung jalankan `zekrima.exe` |
| **Windows (1-Perintah PowerShell)** | `install-windows.ps1` | `irm https://raw.githubusercontent.com/zekrima/zekrima_personal_asisstant/main/zekrima-windows/install-windows.ps1 \| iex` |
| **Arch Linux / CachyOS (Pacman)** | `zekrima-0.3.0-1-x86_64.pkg.tar.zst` | `sudo pacman -U zekrima-0.3.0-1-x86_64.pkg.tar.zst` *(tanpa compile)* |
| **Linux Pre-Built Binary** | `zekrima-linux-x86_64.tar.gz` | Ekstrak lalu jalankan `./install-binary.sh` |
| **Arch / CachyOS (Build Source)** | `zekrima-arch-cachyos-src.tar.gz` | Ekstrak lalu jalankan `./install-cachyos.sh` |

### Build Manual dari Source

#### Windows 11 / 10
```powershell
cd zekrima-windows
cargo build --release
.\target\release\zekrima.exe
```
Detail instalasi, izin notifikasi Windows, & pemakaian lengkap: [zekrima-windows/README.md](zekrima-windows/README.md).

#### Arch Linux / CachyOS
```bash
cd zekrima-arch-linux
chmod +x install-cachyos.sh
./install-cachyos.sh
zekrima &
```
Detail instalasi Hyprland / KDE Plasma 6 Wayland, hotkey IPC (`--toggle-hide`, `--voice`, `--chat`), & uninstall: [zekrima-arch-linux/README.md](zekrima-arch-linux/README.md).

---

## Roadmap Pengembangan (Aktif)

Fitur-fitur produktivitas yang direncanakan untuk rilis berikutnya agar semakin membantu pekerjaan harian:

- [x] **Focus / Pomodoro Timer** — timer kerja langsung di pill dengan preset (`5m/15m/25m/45m`), progress bar, & toast selesai.
- [x] **Quick Clipboard History** — akses cepat riwayat teks `Ctrl+C` (event-driven, klik untuk copy ulang instan).
- [x] **System Monitor Ringkas** — indikator CPU, RAM, & Battery/Power berkode warna (on-demand, 0% idle CPU).
- [ ] **Bluetooth Audio & Per-App Mixer** — status perangkat audio Bluetooth & kontrol volume per aplikasi.
- [ ] **Calendar & Meeting Reminder** — hitung mundur jadwal meeting berikutnya langsung di collapsed pill.
- [ ] **Kustomisasi Tema & Multi-Monitor** — pengaturan warna aksen, posisi monitor, dan file konfigurasi `config.toml`.
- [ ] **Auto-Update & AUR Package** — paket AUR (`zekrima-bin` / `zekrima-git`) untuk Arch Linux & CachyOS.

---

## CI/CD & Rilis Otomatis
- **CI** ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)): otomatis mengecek dan mem-build versi **Windows** dan **Arch Linux** pada setiap push/PR ke `main`.
- **Release** ([`.github/workflows/release.yml`](.github/workflows/release.yml)): otomatis mem-build dan menerbitkan aset `zekrima.exe`, `zekrima-windows-portable.zip`, dan `zekrima-arch-cachyos.tar.gz` ke **GitHub Releases** setiap kali tag versi (`v*`) di-push:
  ```bash
  git tag v0.1.0
  git push origin v0.1.0
  ```

## Lisensi
Dirilis di bawah lisensi [MIT](LICENSE).
