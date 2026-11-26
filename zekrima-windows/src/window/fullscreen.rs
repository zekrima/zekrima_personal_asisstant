//! Deteksi fullscreen otomatis menggunakan `SHQueryUserNotificationState`.
//!
//! Timer setiap 1000 ms memeriksa status notifikasi:
//! - Jika `QUNS_RUNNING_D3D_FULL_SCREEN` atau `QUNS_BUSY` → sembunyikan window.
//! - Jika `QUNS_ACCEPTS_NOTIFICATIONS` → tampilkan kembali tanpa mencuri fokus
//!   dan pastikan Z-order tetap `HWND_TOPMOST`.

use windows::core::Result;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_ACCEPTS_NOTIFICATIONS, QUNS_BUSY,
    QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE,
};

/// Timer id untuk pengecekan fullscreen berkala (1000 ms).
pub const TIMER_FULLSCREEN_CHECK: usize = 2;

pub struct FullscreenMonitor {
    hwnd: HWND,
    timer: usize,
    was_fullscreen: bool,
}

impl FullscreenMonitor {
    pub unsafe fn new(hwnd: HWND) -> Result<Self> {
        let _ = SetTimer(hwnd, TIMER_FULLSCREEN_CHECK, 1000, None);

        Ok(Self {
            hwnd,
            timer: TIMER_FULLSCREEN_CHECK,
            was_fullscreen: false,
        })
    }

    pub fn is_hidden(&self) -> bool {
        self.was_fullscreen
    }

    /// Panggil dari `wndproc` saat menerima `WM_TIMER`.
    pub unsafe fn check_and_react(&mut self, manual_hidden: bool) -> Result<()> {
        let state = SHQueryUserNotificationState()?;
        let is_fullscreen = state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_BUSY;
        let is_visible = state == QUNS_ACCEPTS_NOTIFICATIONS;

        if is_fullscreen && !self.was_fullscreen {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            self.was_fullscreen = true;
        } else if is_visible && self.was_fullscreen {
            if !manual_hidden {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            self.was_fullscreen = false;
        }

        Ok(())
    }
}

impl Drop for FullscreenMonitor {
    fn drop(&mut self) {
        let _ = unsafe { KillTimer(self.hwnd, self.timer) };
    }
}
