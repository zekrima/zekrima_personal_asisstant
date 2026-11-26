//! System Control Tool Calling for Zekrima Assistant.
//!
//! Connects Zeki's AI brain directly to Windows system controls:
//! - `set_volume(level, mute)` — Core Audio (`IAudioEndpointVolume`)
//! - `set_brightness(level)`   — DDC/CI (`SetMonitorBrightness`) + WMI fallback
//! - `open_app(name)`          — Smart Windows app launcher (non-blocking)
//! - `notifications` context   — Injected from `NotificationSnapshot` so Zeki
//!                               can read & summarize recent desktop notifications
//!
//! Works across ALL LLM providers (Gemini 3.8 Flash, OpenRouter, OpenCode,
//! NVIDIA NIM, Groq, OpenAI) via structured inline `[[TOOL:...]]` tags emitted
//! by the model and stripped cleanly before displaying the final reply.

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::thread;
use std::time::Duration;

use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, SetMonitorBrightness, PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, IsWindow, SetForegroundWindow, ASFW_ANY,
};

use crate::services::notifications::NotificationSnapshot;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Instruction block appended to the assistant's system prompt so any LLM can
/// invoke Windows system controls reliably.
pub const TOOL_SYSTEM_INSTRUCTIONS: &str = "\n\n\
[SYSTEM CONTROL TOOLS]\n\
Kamu terhubung langsung ke kontrol sistem Windows pengguna. Jika pengguna meminta aksi sistem, sertakan tag aksi berikut di akhir jawabanmu (boleh lebih dari satu) sambil tetap menjawab singkat & ramah:\n\
- Atur volume/mute: [[TOOL:set_volume|level=0..100|mute=true/false]] (boleh isi level saja atau mute saja)\n\
- Atur kecerahan layar: [[TOOL:set_brightness|level=0..100]]\n\
- Buka aplikasi Windows: [[TOOL:open_app|name=nama_aplikasi]] (contoh: notepad, calculator, chrome, edge, explorer, terminal, vscode, spotify, discord, whatsapp, settings, taskmgr, paint)\n\
Jangan gunakan tag [[TOOL:...]] jika pengguna tidak meminta aksi sistem.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolAction {
    SetVolume { level: Option<u32>, mute: Option<bool> },
    SetBrightness { level: u32 },
    OpenApp { name: String },
}

/// Builds a concise text summary of current system state + recent notifications
/// to inject into the LLM context.
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

/// Parses `[[TOOL:...]]` tags from an LLM reply, executes each requested Windows
/// action, and returns `(cleaned_reply, executed_action_badges)`.
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

/// Strips `[[TOOL:...]]` markers from streaming text so raw tool syntax never
/// flashes awkwardly in the chat box.
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
            // Incomplete tag while streaming — hide the partial tag tail
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
        ToolAction::OpenApp { name } => open_windows_app(name),
    }
}

/// Sets Windows master audio volume (`0..=100`) and/or mute state via Core Audio.
pub fn set_system_volume(level: Option<u32>, mute: Option<bool>) -> Result<String, String> {
    unsafe {
        let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let res = (|| -> Result<String, String> {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|_| "Audio device tidak tersedia".to_string())?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(|_| "Speaker default tidak ditemukan".to_string())?;
            let endpoint: IAudioEndpointVolume = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|_| "Gagal mengakses kontrol volume".to_string())?;

            let mut summary = Vec::new();
            if let Some(lvl) = level {
                let clamped = lvl.min(100);
                let scalar = clamped as f32 / 100.0;
                endpoint
                    .SetMasterVolumeLevelScalar(scalar, std::ptr::null())
                    .map_err(|_| "Gagal mengubah level volume".to_string())?;
                // Auto-unmute if user explicitly raises volume > 0 without requesting mute
                if clamped > 0 && mute.is_none() {
                    let _ = endpoint.SetMute(false, std::ptr::null());
                }
                summary.push(format!("Volume {clamped}%"));
            }
            if let Some(m) = mute {
                endpoint
                    .SetMute(m, std::ptr::null())
                    .map_err(|_| "Gagal mengubah status mute".to_string())?;
                summary.push(if m { "Muted".to_string() } else { "Unmuted".to_string() });
            }
            Ok(summary.join(" • "))
        })();

        if com_ok {
            CoUninitialize();
        }
        res
    }
}

/// Sets monitor brightness (`0..=100`) via DDC/CI with WMI fallback for laptop panels.
pub fn set_system_brightness(level: u32) -> Result<String, String> {
    let clamped = level.min(100);

    // 1. Try DDC/CI for external displays
    if try_set_ddcci_brightness(clamped) {
        return Ok(format!("Brightness {clamped}%"));
    }

    // 2. Fallback to WMI `WmiMonitorBrightnessMethods` (works on built-in laptop panels)
    let ps_cmd = format!(
        "(Get-WmiObject -Namespace root/WMI -Class WmiMonitorBrightnessMethods).WmiSetBrightness(1, {clamped})"
    );
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps_cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    match status {
        Ok(s) if s.success() => Ok(format!("Brightness {clamped}%")),
        _ => Err(format!("Monitor tidak mendukung ubah brightness ({clamped}%)")),
    }
}

fn try_set_ddcci_brightness(percent: u32) -> bool {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        if hmon.is_invalid() {
            return false;
        }
        let mut count = 0u32;
        if GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut count).is_err() || count == 0 {
            return false;
        }
        let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
        if GetPhysicalMonitorsFromHMONITOR(hmon, &mut monitors).is_err() {
            return false;
        }
        let mut any_ok = false;
        for mon in &monitors {
            let mut min_b = 0u32;
            let mut cur_b = 0u32;
            let mut max_b = 0u32;
            if GetMonitorBrightness(mon.hPhysicalMonitor, &mut min_b, &mut cur_b, &mut max_b) != 0
                && max_b > min_b
            {
                let target = min_b + ((max_b - min_b) * percent) / 100;
                if SetMonitorBrightness(mon.hPhysicalMonitor, target) != 0 {
                    any_ok = true;
                }
            }
        }
        let _ = DestroyPhysicalMonitors(&monitors);
        any_ok
    }
}

/// Launches a Windows application cleanly without blocking the UI.
pub fn open_windows_app(app_name: &str) -> Result<String, String> {
    let raw = app_name.trim();
    if raw.is_empty() {
        return Err("Nama aplikasi kosong".to_string());
    }
    let lower = raw.to_ascii_lowercase();
    let (label, program, args): (&str, &str, &[&str]) = match lower.as_str() {
        "notepad" | "catatan" => ("Notepad", "notepad.exe", &[]),
        "calc" | "calculator" | "kalkulator" => ("Calculator", "calc.exe", &[]),
        "paint" | "mspaint" => ("Paint", "mspaint.exe", &[]),
        "explorer" | "files" | "file explorer" => ("File Explorer", "explorer.exe", &[]),
        "cmd" | "command prompt" => ("Command Prompt", "cmd.exe", &[]),
        "terminal" | "windows terminal" | "wt" => ("Terminal", "wt.exe", &[]),
        "powershell" => ("PowerShell", "powershell.exe", &[]),
        "taskmgr" | "task manager" => ("Task Manager", "taskmgr.exe", &[]),
        "settings" | "pengaturan" => ("Settings", "cmd.exe", &["/C", "start", "", "ms-settings:"]),
        "chrome" | "google chrome" => ("Chrome", "cmd.exe", &["/C", "start", "", "chrome"]),
        "edge" | "microsoft edge" => ("Edge", "cmd.exe", &["/C", "start", "", "msedge"]),
        "firefox" => ("Firefox", "cmd.exe", &["/C", "start", "", "firefox"]),
        "vscode" | "vs code" | "code" | "visual studio code" => {
            ("VS Code", "cmd.exe", &["/C", "start", "", "code"])
        }
        "spotify" => ("Spotify", "cmd.exe", &["/C", "start", "", "spotify:"]),
        "discord" => ("Discord", "cmd.exe", &["/C", "start", "", "discord:"]),
        "whatsapp" | "wa" => ("WhatsApp", "cmd.exe", &["/C", "start", "", "whatsapp:"]),
        "snipping" | "screenshot" => {
            ("Snipping Tool", "cmd.exe", &["/C", "start", "", "ms-screenclip:"])
        }
        _ => {
            // Safe sanitized fallback for simple single-token app names (no shell injection)
            let safe: String = raw
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                .collect();
            if safe.is_empty() {
                return Err(format!("Aplikasi '{raw}' tidak dikenali"));
            }
            unsafe {
                let _ = AllowSetForegroundWindow(ASFW_ANY);
            }
            return Command::new("cmd.exe")
                .args(["/C", "start", "", &safe])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map(|_| format!("Opened {safe}"))
                .map_err(|_| format!("Gagal membuka {safe}"));
        }
    };

    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }

    let mut cmd = Command::new(program);
    cmd.args(args);
    if program.eq_ignore_ascii_case("cmd.exe") {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map(|_| format!("Opened {label}"))
        .map_err(|_| format!("Gagal membuka {label}"))
}

/// User-initiated helper: restores focus to the previously active window and
/// pastes the clipboard text at the active cursor (`Ctrl+V`). Only invoked when
/// the user explicitly clicks the `Insert` button.
pub fn paste_into_window(target_hwnd: HWND) {
    let hwnd_raw = target_hwnd.0 as isize;
    if hwnd_raw == 0 {
        return;
    }
    let _ = thread::Builder::new()
        .name("zekrima-insert-text".to_owned())
        .spawn(move || unsafe {
            let hwnd = HWND(hwnd_raw as *mut c_void);
            if !IsWindow(hwnd).as_bool() {
                return;
            }
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            let _ = SetForegroundWindow(hwnd);
            thread::sleep(Duration::from_millis(95));

            let inputs = [
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_V,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_V,
                            dwFlags: KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            dwFlags: KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                },
            ];
            let _ = SendInput(&inputs, size_of::<INPUT>() as i32);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_tool_tags_and_cleans_reply() {
        let raw = "Siap, suara sudah dikecilkan dan Notepad dibuka!\n[[TOOL:set_volume|level=20|mute=false]] [[TOOL:open_app|name=notepad]]";
        let (cleaned, actions) = extract_tool_actions(raw);
        assert_eq!(cleaned, "Siap, suara sudah dikecilkan dan Notepad dibuka!");
        assert_eq!(
            actions,
            vec![
                ToolAction::SetVolume {
                    level: Some(20),
                    mute: Some(false),
                },
                ToolAction::OpenApp {
                    name: "notepad".to_string(),
                }
            ]
        );
    }

    #[test]
    fn hides_incomplete_tool_tag_during_streaming() {
        let partial = "Oke, mengubah kecerahan layar [[TOOL:set_bright";
        assert_eq!(strip_tool_tags(partial), "Oke, mengubah kecerahan layar");
    }
}
