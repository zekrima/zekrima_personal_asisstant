#!/usr/bin/env bash
set -euo pipefail

echo "==> Installing build & runtime dependencies for Zekrima Personal Assistant v0.3.0 on CachyOS / Arch Linux..."
sudo pacman -S --needed --noconfirm \
  base-devel rust cargo pkgconf \
  gtk4 gtk4-layer-shell pango cairo gdk-pixbuf2 \
  playerctl pipewire-pulse wireplumber dbus curl \
  grim brightnessctl wtype

echo "==> Building and installing Zekrima Arch package via makepkg..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
makepkg -si --noconfirm

echo ""
echo "==> Selesai! Zekrima Personal Assistant v0.3.0 sudah terinstall di /usr/bin/zekrima"
echo "    Jalankan dengan: zekrima &"
echo "    Hotkey Hyprland / KDE Wayland:"
echo "      - Toggle Hide/Show : zekrima --toggle-hide"
echo "      - Voice Listening  : zekrima --voice"
echo "      - Open Chat Mode   : zekrima --chat"
echo "      - Assistant Config : zekrima --settings"
