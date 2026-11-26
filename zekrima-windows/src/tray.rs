//! System Tray native Windows 11 menggunakan `Shell_NotifyIconW` dan menu popup.
//!
//! - Ikon di area notifikasi dengan tooltip `"Zekrima"`.
//! - Klik kanan → menu: Run at Startup (centang), Reset Position, Separator, Exit.
//! - Registry `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` untuk opsi Startup.
//! - `SetForegroundWindow` sebelum `TrackPopupMenu` dan `PostMessageW(hwnd, WM_NULL, ...)`
//!   setelahnya (pola standar Win32 agar menu bisa ditutup dengan benar).
//! - Pilihan menu diposting ke jendela utama via `WM_TRAY_COMMAND`.

use std::cell::Cell;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use windows::core::{w, PCWSTR, Result};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HINSTANCE, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, LoadIconW, PostMessageW, RegisterClassW, SetForegroundWindow, TrackPopupMenu,
    IDI_APPLICATION, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_USER, WNDCLASSW,
    WS_POPUP,
};

/// Pesan kustom dari tray window ke main window (membawa ID menu di `wparam`).
pub const WM_TRAY_COMMAND: u32 = WM_USER + 100;

/// ID menu tray.
pub const IDM_STARTUP: usize = 1;
pub const IDM_RESET: usize = 2;
pub const IDM_EXIT: usize = 3;
pub const IDM_NOTIF_SETTINGS: usize = 4;
pub const IDM_ASSISTANT_CONFIG: usize = 5;
pub const IDM_TOGGLE_HIDE: usize = 6;
pub const IDM_AUTO_HIDE: usize = 7;

static TRAY_AUTO_HIDE_CHECKED: AtomicBool = AtomicBool::new(false);
static TRAY_ISLAND_HIDDEN: AtomicBool = AtomicBool::new(false);
static TRAY_HIDE_HOTKEY: AtomicU8 = AtomicU8::new(0);

pub fn sync_tray_state(auto_hide: bool, island_hidden: bool, hide_hotkey_u8: u8) {
    TRAY_AUTO_HIDE_CHECKED.store(auto_hide, Ordering::Relaxed);
    TRAY_ISLAND_HIDDEN.store(island_hidden, Ordering::Relaxed);
    TRAY_HIDE_HOTKEY.store(hide_hotkey_u8, Ordering::Relaxed);
}

const TRAY_CLASS_NAME: PCWSTR = w!("ZekrimaTrayWindow");
const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("Zekrima");

thread_local! {
    /// HWND jendela utama, dipakai tray window untuk memposting pilihan menu.
    static APP_HWND: Cell<HWND> = const { Cell::new(HWND(null_mut())) };
}

/// Membangun UTF-16 null-terminated dari string runtime.
fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn tray_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    match message {
        WM_TRAY_COMMAND => {
            // `lparam` membawa pesan mouse dari ikon tray.
            let event = lparam.0 as u32;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                show_tray_menu(hwnd);
            } else if event == WM_LBUTTONUP {
                let app = APP_HWND.with(|slot| slot.get());
                if !app.is_invalid() {
                    let _ = PostMessageW(app, WM_TRAY_COMMAND, WPARAM(IDM_TOGGLE_HIDE), LPARAM(0));
                }
            }
            windows::Win32::Foundation::LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// Menampilkan menu popup tray dan memposting pilihan ke jendela utama.
unsafe fn show_tray_menu(hwnd: HWND) {
    let menu = match CreatePopupMenu() {
        Ok(menu) => menu,
        Err(_) => return,
    };

    let checked = if is_startup_enabled() {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };
    let hotkey_label = crate::services::config::HideHotkey::from_u8(
        TRAY_HIDE_HOTKEY.load(Ordering::Relaxed),
    )
    .label();
    let hide_wide = to_wide(&if TRAY_ISLAND_HIDDEN.load(Ordering::Relaxed) {
        format!("Show Dynamic Island ({hotkey_label})")
    } else {
        format!("Hide Dynamic Island ({hotkey_label})")
    });
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        IDM_TOGGLE_HIDE,
        PCWSTR(hide_wide.as_ptr()),
    );
    let auto_hide_checked = if TRAY_AUTO_HIDE_CHECKED.load(Ordering::Relaxed) {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };
    let _ = AppendMenuW(
        menu,
        MF_STRING | auto_hide_checked,
        IDM_AUTO_HIDE,
        w!("Auto-Hide when Idle"),
    );
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, w!(""));
    let _ = AppendMenuW(
        menu,
        MF_STRING | checked,
        IDM_STARTUP,
        w!("Run at Windows Startup"),
    );
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        IDM_ASSISTANT_CONFIG,
        w!("Assistant Config (API Keys)"),
    );
    let _ = AppendMenuW(menu, MF_STRING, IDM_NOTIF_SETTINGS, w!("Notification Settings"));
    let _ = AppendMenuW(menu, MF_STRING, IDM_RESET, w!("Reset Position"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, w!(""));
    let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT, w!("Exit"));

    let _ = SetForegroundWindow(hwnd);

    let mut cursor = POINT::default();
    let _ = GetCursorPos(&mut cursor);

    let command = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_RETURNCMD,
        cursor.x,
        cursor.y,
        0,
        hwnd,
        None,
    );

    let _ = DestroyMenu(menu);
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));

    if command.0 != 0 {
        let app = APP_HWND.with(|slot| slot.get());
        if !app.is_invalid() {
            let _ = PostMessageW(app, WM_TRAY_COMMAND, WPARAM(command.0 as usize), LPARAM(0));
        }
    }
}

/// Membaca apakah opsi "Run at Windows Startup" aktif untuk executable ini.
pub fn is_startup_enabled() -> bool {
    unsafe {
        let mut handle = HKEY(null_mut());
        let status = RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_READ, &mut handle);
        if status.0 != 0 {
            return false;
        }

        let mut buffer = [0u16; 1024];
        let mut length = (buffer.len() * 2) as u32;
        let query = RegQueryValueExW(
            handle,
            RUN_VALUE,
            None,
            None,
            Some(buffer.as_mut_ptr() as *mut u8),
            Some(&mut length),
        );
        let _ = RegCloseKey(handle);

        if query.0 != 0 {
            return false;
        }

        let stored = String::from_utf16_lossy(&buffer);
        let stored = stored
            .split('\0')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        match std::env::current_exe() {
            Ok(exe) => stored.eq_ignore_ascii_case(&exe.to_string_lossy()),
            Err(_) => false,
        }
    }
}

/// Menulis atau menghapus entri startup di `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub fn set_startup_registry(enabled: bool) -> Result<()> {
    unsafe {
        if enabled {
            let exe = std::env::current_exe()?;
            let quoted = format!("\"{}\"", exe.to_string_lossy());
            let wide = to_wide(&quoted);
            let mut bytes: Vec<u8> = Vec::with_capacity(wide.len() * 2);
            for unit in wide {
                bytes.extend_from_slice(&unit.to_ne_bytes());
            }

            let mut handle = HKEY(null_mut());
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                RUN_SUBKEY,
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut handle,
                None,
            )
            .ok()?;

            let result = RegSetValueExW(handle, RUN_VALUE, 0, REG_SZ, Some(&bytes)).ok();
            let _ = RegCloseKey(handle);
            result
        } else {
            let mut handle = HKEY(null_mut());
            let open = RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, 0, KEY_SET_VALUE, &mut handle);
            if open == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            open.ok()?;

            let delete = RegDeleteValueW(handle, RUN_VALUE);
            let _ = RegCloseKey(handle);
            if delete == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                delete.ok()
            }
        }
    }
}

/// Membuat jendela pesan tersembunyi dan mengelola ikon tray-nya.
pub struct Tray {
    hwnd: HWND,
    notify: NOTIFYICONDATAW,
    visible: bool,
}

impl Tray {
    pub unsafe fn new(app_hwnd: HWND) -> Result<Self> {
        APP_HWND.with(|slot| slot.set(app_hwnd));

        let instance = HINSTANCE(GetModuleHandleW(None)?.0);

        let window_class = WNDCLASSW {
            lpfnWndProc: Some(tray_wndproc),
            hInstance: instance,
            lpszClassName: TRAY_CLASS_NAME,
            ..Default::default()
        };

        if RegisterClassW(&window_class) == 0 {
            return Err(windows::core::Error::from_win32());
        }

        let tray_hwnd = CreateWindowExW(
            Default::default(),
            TRAY_CLASS_NAME,
            w!("ZekrimaTray"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            instance,
            None,
        )?;

        let hicon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();

        let mut notify = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: tray_hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY_COMMAND,
            hIcon: hicon,
            ..Default::default()
        };

        let tip = to_wide("Zekrima");
        let count = tip.len().min(notify.szTip.len());
        notify.szTip[..count].copy_from_slice(&tip[..count]);

        Ok(Self {
            hwnd: tray_hwnd,
            notify,
            visible: false,
        })
    }

    pub unsafe fn set_visible(&mut self, visible: bool) -> Result<()> {
        if self.visible == visible {
            return Ok(());
        }
        let message = if visible { NIM_ADD } else { NIM_DELETE };
        let _ = Shell_NotifyIconW(message, &self.notify);
        self.visible = visible;
        Ok(())
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            if self.visible {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.notify);
                self.visible = false;
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
