#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "==> Installing runtime dependencies for Zekrima Personal Assistant (Arch / CachyOS)..."
if command -v pacman >/dev/null 2>&1; then
  sudo pacman -S --needed --noconfirm \
    gtk4 gtk4-layer-shell pango cairo gdk-pixbuf2 \
    playerctl pipewire-pulse wireplumber dbus curl \
    grim brightnessctl wtype
fi

echo "==> Installing pre-built zekrima binary to /usr/local/bin/zekrima ..."
sudo install -Dm755 "$SCRIPT_DIR/zekrima" /usr/local/bin/zekrima
sudo install -Dm644 "$SCRIPT_DIR/zekrima.desktop" /usr/share/applications/zekrima.desktop

echo ""
echo "==> Selesai! Zekrima Personal Assistant sudah terpasang di /usr/local/bin/zekrima"
echo "    Jalankan dengan: zekrima &"
echo "    Wayland Hotkeys:"
echo "      - Toggle Hide/Show : zekrima --toggle-hide"
echo "      - Voice Listening  : zekrima --voice"
echo "      - Open Chat Mode   : zekrima --chat"
echo "      - Assistant Config : zekrima --settings"
