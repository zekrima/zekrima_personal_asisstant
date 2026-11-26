//! System Control Tool Calling for Zekrima Assistant on Arch Linux / CachyOS.
//!
//! Connects Zeki's AI brain directly to Linux system controls:
//! - `set_volume(level, mute)` — WirePlumber (`wpctl`) / PipeWire
//! - `set_brightness(level)`   — `brightnessctl` with `/sys/class/backlight` fallback
//! - `open_app(name)`          — Smart Arch/CachyOS app launcher (`xdg-open` fallback)
//! - `notifications` context   — Injected from `NotificationSnapshot`
//!
//! Works across ALL LLM providers via universal `[[TOOL:...]]` tags stripped cleanly
//! before displaying the final reply.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::services::notifications::NotificationSnapshot;

pub const TOOL_SYSTEM_INSTRUCTIONS: &str = "\n\n\
[SYSTEM CONTROL TOOLS]\n\
Kamu terhubung langsung ke kontrol sistem Arch Linux / CachyOS pengguna. Jika pengguna meminta aksi sistem, sertakan tag aksi berikut di akhir jawabanmu (boleh lebih dari satu) sambil tetap menjawab singkat & ramah:\n\
- Atur volume/mute: [[TOOL:set_volume|level=0..100|mute=true/false]] (boleh isi level saja atau mute saja)\n\
- Atur kecerahan layar: [[TOOL:set_brightness|level=0..100]]\n\
- Buka aplikasi Linux: [[TOOL:open_app|name=nama_aplikasi]] (contoh: firefox, chromium, terminal, kitty, konsole, dolphin, files, vscode, spotify, discord, settings, calculator)\n\
Jangan gunakan tag [[TOOL:...]] jika pengguna tidak meminta aksi sistem.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolAction {
    SetVolume {
        level: Option<u32>,
        mute: Option<bool>,
    },
    SetBrightness {
        level: u32,
    },
    OpenApp {
        name: String,
    },
}

pub fn build_context_note(
    notifications: &NotificationSnapshot,
    active_window: &str,
    clipboard_preview: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !active_window.is_empty() {
        parts.push(format!("Jendela aktif: {active_window}"));
    }
    if let Some(clip) = clipboard_preview {
        let trimmed: String = clip.trim().chars().take(240).collect();
        if !trimmed.is_empty() {
            parts.push(format!("Clipboard terbaru: \"{trimmed}\""));
        }
    }
    if !notifications.items.is_empty() {
        let mut notif_lines = Vec::new();
        for item in notifications.items.iter().take(5) {
            let body: String = item.body.chars().take(80).collect();
            notif_lines.push(format!("- [{}] {}: {}", item.app, item.title, body));
        }
        parts.push(format!(
            "Notifikasi masuk terbaru ({}):\n{}",
            notifications.items.len(),
            notif_lines.join("\n")
        ));
    } else {
        parts.push("Notifikasi masuk terbaru: tidak ada notifikasi belum dibaca.".to_string());
    }

    format!("[LIVE CONTEXT]\n{}", parts.join("\n"))
}

pub fn parse_and_execute_tools(raw_reply: &str) -> (String, Vec<String>) {
    let (cleaned, actions) = extract_tool_actions(raw_reply);
    if actions.is_empty() {
        return (cleaned, Vec::new());
    }

    let mut badges = Vec::new();
    for action in actions {
        match execute_action(&action) {
            Ok(msg) => badges.push(format!("✓ {msg}")),
            Err(err) => badges.push(format!("⚠ {err}")),
        }
    }
    (cleaned, badges)
}

pub fn strip_tool_tags(text: &str) -> String {
    extract_tool_actions(text).0
}

pub fn extract_tool_actions(input: &str) -> (String, Vec<ToolAction>) {
    let mut actions = Vec::new();
    let mut cleaned = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("[[TOOL:") {
        cleaned.push_str(&rest[..start]);
        let after = &rest[start + 7..];
        if let Some(end) = after.find("]]") {
            let body = &after[..end];
            if let Some(act) = parse_tool_body(body) {
                actions.push(act);
            }
            rest = &after[end + 2..];
        } else {
            rest = "";
            break;
        }
    }
    cleaned.push_str(rest);
    (cleaned.trim().to_string(), actions)
}

fn parse_tool_body(body: &str) -> Option<ToolAction> {
    let mut parts = body.split('|');
    let name = parts.next()?.trim().to_ascii_lowercase();
    let mut level: Option<u32> = None;
    let mut mute: Option<bool> = None;
    let mut app_name: Option<String> = None;

    for part in parts {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim().trim_matches('"');
        match key.as_str() {
            "level" | "value" | "percent" => {
                let digits: String = val.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse::<u32>() {
                    level = Some(n.min(100));
                }
            }
            "mute" | "muted" => {
                mute = Some(matches!(
                    val.to_ascii_lowercase().as_str(),
                    "true" | "1" | "yes" | "on"
                ));
            }
            "name" | "app" | "app_name" => {
                if !val.is_empty() {
                    app_name = Some(val.to_string());
                }
            }
            _ => {}
        }
    }

    match name.as_str() {
        "set_volume" | "volume" if level.is_some() || mute.is_some() => {
            Some(ToolAction::SetVolume { level, mute })
        }
        "set_brightness" | "brightness" => level.map(|lvl| ToolAction::SetBrightness { level: lvl }),
        "open_app" | "launch_app" => app_name.map(|n| ToolAction::OpenApp { name: n }),
        _ => None,
    }
}

fn execute_action(action: &ToolAction) -> Result<String, String> {
    match action {
        ToolAction::SetVolume { level, mute } => set_system_volume(*level, *mute),
        ToolAction::SetBrightness { level } => set_system_brightness(*level),
        ToolAction::OpenApp { name } => open_linux_app(name),
    }
}

pub fn set_system_volume(level: Option<u32>, mute: Option<bool>) -> Result<String, String> {
    let mut summary = Vec::new();
    if let Some(lvl) = level {
        let clamped = lvl.min(100);
        let pct = format!("{clamped}%");
        let ok = Command::new("wpctl")
            .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &pct])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            || Command::new("pactl")
                .args(["set-sink-volume", "@DEFAULT_SINK@", &pct])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        if !ok {
            return Err("Gagal mengatur volume (wpctl/pactl)".to_string());
        }
        if clamped > 0 && mute.is_none() {
            let _ = Command::new("wpctl")
                .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        summary.push(format!("Volume {clamped}%"));
    }

    if let Some(m) = mute {
        let flag = if m { "1" } else { "0" };
        let ok = Command::new("wpctl")
            .args(["set-mute", "@DEFAULT_AUDIO_SINK@", flag])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
            || Command::new("pactl")
                .args(["set-sink-mute", "@DEFAULT_SINK@", flag])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        if !ok {
            return Err("Gagal mengubah status mute".to_string());
        }
        summary.push(if m {
            "Muted".to_string()
        } else {
            "Unmuted".to_string()
        });
    }

    Ok(summary.join(" • "))
}

pub fn set_system_brightness(level: u32) -> Result<String, String> {
    let clamped = level.clamp(1, 100);
    let pct = format!("{clamped}%");

    if Command::new("brightnessctl")
        .args(["set", &pct])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(format!("Brightness {clamped}%"));
    }

    // Fallback: direct sysfs write if user belongs to `video` group
    if let Some(dir) = find_backlight_dir() {
        if let Ok(max_str) = fs::read_to_string(dir.join("max_brightness")) {
            if let Ok(max_val) = max_str.trim().parse::<u32>() {
                let target = ((max_val as u64 * clamped as u64) / 100).max(1) as u32;
                if fs::write(dir.join("brightness"), target.to_string()).is_ok() {
                    return Ok(format!("Brightness {clamped}%"));
                }
            }
        }
    }

    Err(format!(
        "Gagal ubah brightness ({clamped}%) — pasang `brightnessctl`"
    ))
}

fn find_backlight_dir() -> Option<PathBuf> {
    let entries = fs::read_dir("/sys/class/backlight").ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("brightness").exists() && path.join("max_brightness").exists() {
            return Some(path);
        }
    }
    None
}

pub fn open_linux_app(app_name: &str) -> Result<String, String> {
    let raw = app_name.trim();
    if raw.is_empty() {
        return Err("Nama aplikasi kosong".to_string());
    }
    let lower = raw.to_ascii_lowercase();
    let (label, candidates): (&str, &[&str]) = match lower.as_str() {
        "firefox" => ("Firefox", &["firefox"]),
        "chrome" | "google chrome" | "chromium" => {
            ("Browser", &["chromium", "google-chrome-stable", "brave", "firefox"])
        }
        "terminal" | "kitty" | "konsole" | "alacritty" => {
            ("Terminal", &["kitty", "konsole", "alacritty", "foot", "wezterm", "gnome-terminal"])
        }
        "files" | "explorer" | "dolphin" | "nautilus" | "thunar" => {
            ("Files", &["dolphin", "nautilus", "thunar", "pcmanfm"])
        }
        "vscode" | "vs code" | "code" => ("VS Code", &["code", "codium"]),
        "spotify" => ("Spotify", &["spotify", "spotify-launcher"]),
        "discord" => ("Discord", &["discord", "vesktop"]),
        "calc" | "calculator" | "kalkulator" => {
            ("Calculator", &["kcalc", "gnome-calculator", "qalculate-gtk"])
        }
        "notepad" | "catatan" | "editor" => ("Editor", &["kate", "kwrite", "gedit", "mousepad"]),
        "settings" | "pengaturan" => ("Settings", &["systemsettings", "gnome-control-center"]),
        "taskmgr" | "task manager" => ("System Monitor", &["plasma-systemmonitor", "btop", "htop"]),
        _ => {
            let safe: String = raw
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                .collect();
            if safe.is_empty() {
                return Err(format!("Aplikasi '{raw}' tidak dikenali"));
            }
            if spawn_detached(&safe, &[]) {
                return Ok(format!("Opened {safe}"));
            }
            if spawn_detached("xdg-open", &[&safe]) {
                return Ok(format!("Opened {safe}"));
            }
            return Err(format!("Gagal membuka {safe}"));
        }
    };

    for prog in candidates {
        if spawn_detached(prog, &[]) {
            return Ok(format!("Opened {label}"));
        }
    }

    Err(format!("Aplikasi {label} tidak ditemukan"))
}

fn spawn_detached(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// User-initiated helper triggered ONLY when the user explicitly clicks `Insert`.
/// The text is already copied to the GTK4 clipboard by the caller; this optionally
/// sends a single `Ctrl+V` into the focused application if `wtype` or `xdotool` is installed.
pub fn trigger_paste_shortcut() {
    let _ = thread::Builder::new()
        .name("zekrima-insert-text".to_owned())
        .spawn(move || {
            thread::sleep(Duration::from_millis(120));
            // Wayland wlroots (`wtype -M ctrl v -m ctrl`)
            let ok = Command::new("wtype")
                .args(["-M", "ctrl", "v", "-m", "ctrl"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return;
            }
            // X11 fallback (`xdotool key --clearmodifiers ctrl+v`)
            let _ = Command::new("xdotool")
                .args(["key", "--clearmodifiers", "ctrl+v"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_tool_tags_and_cleans_reply() {
        let raw = "Siap, suara dikecilkan!\n[[TOOL:set_volume|level=25|mute=false]] [[TOOL:open_app|name=firefox]]";
        let (cleaned, actions) = extract_tool_actions(raw);
        assert_eq!(cleaned, "Siap, suara dikecilkan!");
        assert_eq!(
            actions,
            vec![
                ToolAction::SetVolume {
                    level: Some(25),
                    mute: Some(false),
                },
                ToolAction::OpenApp {
                    name: "firefox".to_string(),
                }
            ]
        );
    }

    #[test]
    fn hides_incomplete_tool_tag_during_streaming() {
        let partial = "Mengubah kecerahan layar [[TOOL:set_bright";
        assert_eq!(strip_tool_tags(partial), "Mengubah kecerahan layar");
    }
}
