//! Zero-Retention Screen Vision & active window title detection for Arch Linux / CachyOS.
//!
//! Captures the active screen directly into RAM (zero persistent disk files):
//! - Wayland wlroots (Hyprland / Sway / Niri): `grim -t jpeg -q 80 -` (pure stdout pipe)
//! - KDE Plasma Wayland: `spectacle -b -n -f -o /dev/stdout`
//! - X11 fallback: `import -window root jpeg:-`
//! Resizes in-memory via `gdk-pixbuf` to the configured `max_width` (`1024px` Standard or `1600px` HD).

use std::process::{Command, Stdio};

use gtk4::gdk_pixbuf::{InterpType, PixbufLoader};
use gtk4::prelude::*;

/// Returns the title of the currently focused desktop window (best-effort across Wayland/X11).
pub fn active_window_title() -> String {
    // 1. Hyprland (`hyprctl activewindow -j`)
    if let Ok(out) = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(title) =
                crate::services::assistant::extract_json_string_field(&text, "title")
            {
                let trimmed = title.trim();
                if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("Zekrima") {
                    return trimmed.to_string();
                }
            }
        }
    }

    // 2. X11 / XWayland fallback (`xdotool getactivewindow getwindowname`)
    if let Ok(out) = Command::new("xdotool")
        .args(["getactivewindow", "getwindowname"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        if out.status.success() {
            let title = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !title.is_empty() && !title.eq_ignore_ascii_case("Zekrima") {
                return title;
            }
        }
    }

    String::new()
}

/// Captures the current screen in RAM as a resized JPEG (`max_width` clamped to `640..=2560`).
/// Never stores persistent files on disk (Zero Retention).
pub fn capture_screen_jpeg(max_width: u32) -> Option<Vec<u8>> {
    let raw = capture_raw_image_bytes()?;
    if raw.len() < 64 {
        return None;
    }

    let max_w = max_width.clamp(640, 2560) as i32;
    resize_to_jpeg(&raw, max_w).or_else(|| {
        // If input is already JPEG and pixbuf scaling failed, return raw JPEG directly.
        if raw.starts_with(&[0xFF, 0xD8]) {
            Some(raw)
        } else {
            None
        }
    })
}

fn capture_raw_image_bytes() -> Option<Vec<u8>> {
    // 1. Wayland wlroots (Hyprland, Sway, Niri, Wayfire, River)
    if let Some(bytes) = run_capture("grim", &["-t", "jpeg", "-q", "80", "-"]) {
        return Some(bytes);
    }

    // 2. KDE Plasma Wayland (CachyOS default DE)
    if let Some(bytes) = run_capture("spectacle", &["-b", "-n", "-f", "-o", "/dev/stdout"]) {
        return Some(bytes);
    }

    // 3. ImageMagick X11 fallback
    if let Some(bytes) = run_capture("import", &["-window", "root", "jpeg:-"]) {
        return Some(bytes);
    }

    None
}

fn run_capture(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if output.status.success() && output.stdout.len() > 64 {
        Some(output.stdout)
    } else {
        None
    }
}

fn resize_to_jpeg(raw: &[u8], max_w: i32) -> Option<Vec<u8>> {
    let loader = PixbufLoader::new();
    loader.write(raw).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;

    let w = pixbuf.width();
    let h = pixbuf.height();
    if w <= 0 || h <= 0 {
        return None;
    }

    let scaled = if w > max_w {
        let new_h = ((h as f64 * max_w as f64) / w as f64).round().max(1.0) as i32;
        pixbuf
            .scale_simple(max_w, new_h, InterpType::Bilinear)
            .unwrap_or(pixbuf)
    } else {
        pixbuf
    };

    scaled
        .save_to_bufferv("jpeg", &[("quality", "78")])
        .ok()
}
