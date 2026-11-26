# Zekrima untuk Arch Linux / CachyOS

Dynamic Island native untuk **Arch Linux** & **CachyOS**, dibangun dengan **Rust + GTK4 + `gtk4-layer-shell` + Cairo/PangoCairo**. Dirancang agar ringan, native Wayland, click-through, tidak mencuri fokus, dan `0.0%` CPU saat idle.

---

## 1. Kompatibilitas Desktop Environment
- **Wayland (Direkomendasikan — native `zwlr_layer_shell_v1`)**:
  - **Hyprland** ✅
  - **KDE Plasma 6 (Wayland)** ✅ *(DE default CachyOS)*
  - **Sway / Niri / Wayfire / River** ✅
- **X11 / GNOME**:
  - Berjalan dengan mode fallback jendela floating transparan tanpa dekorasi.

---

## 2. Cara Instalasi di CachyOS / Arch Linux

### Cara A — Instalasi Otomatis (Paling Mudah)
Salin folder `zekrima-arch-linux/` (atau ekstrak `zekrima-arch-cachyos.tar.gz`) ke CachyOS, lalu jalankan:

```bash
cd zekrima-arch-linux
chmod +x install-cachyos.sh
./install-cachyos.sh
```

Script ini akan otomatis:
1. Memasang dependency yang dibutuhkan lewat `pacman -S --needed` (`gtk4`, `gtk4-layer-shell`, `playerctl`, `pipewire-pulse`, `wireplumber`, `dbus`, `rust`, dll.).
2. Mem-build binary release teroptimasi dan memasangnya sebagai paket Arch native via `makepkg -si` ke `/usr/bin/zekrima` dan `/usr/share/applications/zekrima.desktop`.

### Cara B — Manual via `makepkg` (Paket Arch Native)
```bash
cd zekrima-arch-linux
makepkg -si
```

### Cara C — Jalankan Tanpa Install Global (Dev / Testing)
```bash
sudo pacman -S --needed base-devel rust cargo pkgconf gtk4 gtk4-layer-shell pango cairo gdk-pixbuf2 playerctl pipewire-pulse wireplumber dbus curl
cd zekrima-arch-linux
cargo run --release
```

### Cara Uninstall
Karena terpasang lewat `pacman`/`makepkg`, menghapusnya sangat bersih:
```bash
sudo pacman -R zekrima
```
*(Jika autostart pernah diaktifkan, hapus juga `~/.config/autostart/zekrima.desktop`.)*

---

## 3. Menjalankan & Autostart

### Menjalankan Manual
```bash
zekrima &
```
Atau cari **Zekrima** di Application Launcher (KDE Kickoff / Rofi / Wofi / Fuzzel).

### Autostart Saat Login
- **KDE Plasma / GNOME / XFCE (XDG Autostart)**:
  - Klik kanan pada pill → pilih **`Run at Login`** (otomatis membuat `~/.config/autostart/zekrima.desktop`).
- **Hyprland**:
  - Tambahkan baris berikut ke `~/.config/hypr/hyprland.conf`:
    ```ini
    exec-once = zekrima
    ```
- **Sway**:
  - Tambahkan ke `~/.config/sway/config`:
    ```ini
    exec zekrima
    ```

---

## 4. Panduan Penggunaan

### Interaksi Dasar
| Aksi | Fungsi |
| :--- | :--- |
| **Hover (arahkan kursor) ke pill** | Pill mengembang halus dengan animasi *spring*. |
| **Geser kursor keluar pill** | Pill menyusut kembali ke bentuk ringkas. |
| **Klik kiri pada pill** | Toggle mengembang/menyusut (tidak mencuri fokus keyboard). |
| **Klik di luar area pill** | Klik tembus (*click-through*) via `GdkSurface::set_input_region`. |
| **Klik kanan pada pill** | Menu konteks: `Run at Login`, `Test Notification`, `Quit Zekrima`. |

### 6 Tab Produktivitas & Maskot Zeki (`v0.3.0`)
Saat pill mengembang (`440 × 198`, atau `520 × 310` / `640 × 470` saat Mode Chat), tersedia 6 tab di bagian atas serta **Zeki Companion Dock** yang selalu hadir di bawah seluruh sub-menu:
- **`Zeki`**: maskot vektor prosedural 7-state (`Idle`, `Sleepy`, `Listening`, `Thinking`, `Speaking`, `Happy`, `Error`), **Mode Chat** scrollable (klik ganda card untuk perbesar `520×310` ↔ `640×470`), lampiran file (`+ File` via XDG Desktop Portal `FileDialog` untuk teks/kode/gambar/PDF), tombol `Insert`/`Copy`/`Clear`, toggle privasi `Screen: ON` / `Incognito`, dan tombol **`Config`** (jendela GTK4 Settings untuk API key BYOK: **Google Gemini 3.8 Flash + TTS**, **OpenRouter Free**, **OpenCode Zen Free**, **NVIDIA NIM Free**, **Groq**, **OpenAI/Custom**, pilihan suara TTS, resolusi Screen Vision Standard/HD, & Auto-Hide).
- **`Media`**: logo media di pojok kiri atas tepat di atas jam tebal, judul/artis, kontrol MPRIS2, dan visualizer PipeWire FFT.
- **`Timer`**: Focus / Pomodoro Timer dengan preset **`5m`**, **`15m`**, **`25m`**, **`45m`**, tombol **`Start`/`Pause`** & **`Reset`**, progress bar, indikator countdown di collapsed pill, serta toast notifikasi otomatis saat waktu habis.
- **`Clip`**: riwayat teks clipboard (`Ctrl+C`) event-driven via GTK4 `gdk::Clipboard`. Klik baris mana saja untuk copy ulang secara instan, atau klik **`Clear`**.
- **`Sys`**: System Monitor ringkas (**CPU %** dari `/proc/stat`, **Memory** dari `/proc/meminfo`, **Battery/Power** dari `/sys/class/power_supply`) — hanya di-sample saat tab `Sys` dibuka (`0.0%` idle CPU).
- **`Notifs`**: daftar notifikasi D-Bus bergaya Pillar.

### Global Hotkeys Wayland (Hyprland / KDE Plasma / Sway) & Auto-Hide
Zekrima Linux mendukung remote CLI IPC melalui `gio::Application` sehingga Anda bisa mengikat shortcut global di compositor Wayland mana pun tanpa membuka jendela ganda:
- **Toggle Hide / Show**: `zekrima --toggle-hide`
- **Voice Listening**: `zekrima --voice`
- **Buka Chat Mode**: `zekrima --chat`
- **Buka Settings**: `zekrima --settings`

Contoh di `~/.config/hypr/hyprland.conf`:
```ini
bind = ALT, H, exec, zekrima --toggle-hide
bind = ALT, SPACE, exec, zekrima --voice
bind = ALT, C, exec, zekrima --chat
```
Atau aktifkan **Auto-Hide when Idle** lewat menu klik kanan / Settings (pill memudar setelah 4 detik idle dan muncul kembali saat kursor menyentuh tepi tengah atas layar).

### Jam Tegas & Jelas
- **Collapsed Pill**: jam **`HH:MM`** berukuran **`16.5px` Bold** putih terang di sebelah label Zekrima / album art.
- **Expanded Idle Card**: jam besar **`32px` Bold** + tanggal singkat (`Thu, 26 Nov`) + level volume sistem.
- **Expanded Media Card**: jam **`18.5px` Bold** di pojok kanan bawah sejajar dengan kontrol media.

### Kontrol Media (MPRIS2) & Bar Musik Mengikuti Irama
- Bekerja otomatis dengan **Spotify, Firefox, Chromium/Brave, VLC, mpv, Cider, YouTube Music**, dan semua player yang mendukung MPRIS2 (via `playerctl --follow`).
- **Equalizer 5 Bar Real-Time**: menangkap stream audio monitor PipeWire (`parec --device=@DEFAULT_MONITOR@`) dan menganalisisnya dengan **512-point FFT** murni Rust (Sub-bass, Bass, Vokal, Melodi, Treble).
- Saat musik di-pause atau ditutup, proses `parec` langsung dihentikan sehingga penggunaan CPU kembali `0.0%`.

### Notifikasi Bergaya Pillar (D-Bus)
- Memantau `org.freedesktop.Notifications.Notify` secara event-driven via `dbus-monitor` — bekerja berdampingan dengan **KDE Plasma Notifications, SwayNC, Dunst, dan Mako** tanpa bentrok.
- **Incoming Toast**: muncul ~3,8 detik dengan warna aksen per aplikasi, avatar inisial, judul, isi, tombol `×`, dan progress bar countdown di bawah.
- **Collapsed Badge**: avatar warna aplikasi + badge merah jumlah notifikasi (`1`–`9+`).
- **Expanded Notifications Panel**: klik **`Notifs`** di pojok kanan atas saat expanded untuk melihat daftar notifikasi, menghapus satuan, atau **`Clear all`**.
- Untuk mengetes cepat, klik kanan pill → **`Test Notification`**, atau jalankan dari terminal:
  ```bash
  notify-send -a "Discord" "Pesan Baru" "Halo dari CachyOS!"
  ```

### Volume & Brightness HUD
- **Volume**: event-driven via `pactl subscribe` + `wpctl get-volume @DEFAULT_AUDIO_SINK@` (mendukung tombol keyboard, scroll bar, dan perangkat Bluetooth).
- **Brightness**: event-driven via `udevadm monitor --subsystem-match=backlight` membaca `/sys/class/backlight/*/brightness` (untuk laptop).

---

## 5. Pemecahan Masalah (Troubleshooting)
- **Error saat `cargo build` terkait `gtk4-layer-shell`**: Pastikan paket `gtk4-layer-shell` dan `pkgconf` sudah terinstall (`sudo pacman -S gtk4 gtk4-layer-shell pkgconf`).
- **Bar musik tidak bergerak saat lagu diputar**: Pastikan `pipewire-pulse` aktif (`systemctl --user status pipewire-pulse`) dan perintah `parec --version` tersedia.
- **Media tidak terdeteksi**: Uji dengan `playerctl status`; pastikan `playerctl` terinstall dan browser/pemutar musik mengaktifkan MPRIS.
- **Volume HUD tidak muncul**: Uji dengan `wpctl get-volume @DEFAULT_AUDIO_SINK@` dan `pactl info`.
- **Menjalankan di GNOME Wayland**: GNOME belum mendukung protokol `wlr-layer-shell`, sehingga jendela berjalan dalam mode fallback (floating undecorated). Untuk pengalaman Dynamic Island terbaik di CachyOS, gunakan **KDE Plasma 6 Wayland** atau **Hyprland**.
