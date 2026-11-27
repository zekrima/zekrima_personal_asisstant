//! Top-level layered "island" host window.
//!
//! The window itself is a fixed 540x180 logical-pixel transparent canvas,
//! horizontally centered at the top of the primary monitor. It never appears
//! in Alt+Tab or the taskbar and never activates on click.

use windows::core::{w, Error, Result};
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, GetSystemMetrics, GetWindowLongPtrW, RegisterClassW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, ASFW_ANY, CS_DBLCLKS,
    CS_HREDRAW, CS_VREDRAW, GWL_EXSTYLE, HWND_TOPMOST, SM_CXSCREEN, SM_CYSCREEN, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WNDCLASSW,
    WNDPROC, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::layout::{canvas_size_px, effective_dpi};

pub struct HostWindow {
    hwnd: HWND,
    dpi: u32,
    width_px: u32,
    height_px: u32,
}

impl HostWindow {
    /// Registers the window class and creates the hidden layered canvas.
    pub unsafe fn create(wndproc: WNDPROC) -> Result<Self> {
        let module = GetModuleHandleW(None)?;
        let instance = HINSTANCE(module.0);
        let class_name = w!("ZekrimaHostWindow");

        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: wndproc,
            hInstance: instance,
            lpszClassName: class_name,
            ..Default::default()
        };

        if RegisterClassW(&window_class) == 0 {
            return Err(Error::from_win32());
        }

        let raw_dpi = GetDpiForSystem().max(96);
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);
        let dpi = effective_dpi(raw_dpi, screen_width, screen_height);
        let (width_px, height_px) = canvas_size_px(dpi);
        let x = (screen_width - width_px) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name,
            w!("Zekrima"),
            WS_POPUP,
            x,
            0,
            width_px,
            height_px,
            None,
            None,
            instance,
            None,
        )?;

        let actual_raw_dpi = GetDpiForWindow(hwnd).max(96);
        let actual_dpi = effective_dpi(actual_raw_dpi, screen_width, screen_height);
        let mut host = Self {
            hwnd,
            dpi,
            width_px: width_px as u32,
            height_px: height_px as u32,
        };

        if actual_dpi != dpi {
            host.apply_dpi(actual_raw_dpi)?;
        }

        Ok(host)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    pub fn width_px(&self) -> u32 {
        self.width_px
    }

    pub fn height_px(&self) -> u32 {
        self.height_px
    }

    /// Applies a new raw DPI + current monitor resolution, resizes the canvas and re-centers at `y = 0`.
    pub unsafe fn apply_dpi(&mut self, raw_dpi: u32) -> Result<()> {
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);
        let dpi = effective_dpi(raw_dpi.max(96), screen_width, screen_height);
        let (width_px, height_px) = canvas_size_px(dpi);
        let x = (screen_width - width_px) / 2;

        SetWindowPos(
            self.hwnd,
            HWND_TOPMOST,
            x,
            0,
            width_px,
            height_px,
            SWP_NOACTIVATE,
        )?;

        self.dpi = dpi;
        self.width_px = width_px as u32;
        self.height_px = height_px as u32;
        Ok(())
    }

    /// Re-queries active monitor resolution & DPI (called on `WM_DISPLAYCHANGE` or Reset Position).
    pub unsafe fn refresh_for_display(&mut self) -> Result<()> {
        let raw_dpi = GetDpiForWindow(self.hwnd).max(96);
        self.apply_dpi(raw_dpi)
    }

    /// Shows the window without activating it or stealing keyboard focus.
    pub unsafe fn show(&self) {
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

    /// Completely hides the island window (used by manual `Alt + H` hide shortcut).
    pub unsafe fn hide(&self) {
        let _ = ShowWindow(self.hwnd, SW_HIDE);
    }

    /// Toggles whether the island accepts keyboard focus (only enabled while
    /// typing inside Zeki's chat box so normal clicks never steal focus).
    pub unsafe fn set_keyboard_focus(&self, enable: bool) {
        let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
        let no_act = WS_EX_NOACTIVATE.0 as isize;
        let next = if enable { ex & !no_act } else { ex | no_act };
        if next != ex {
            let _ = SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next);
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        if enable {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(self.hwnd);
        }
    }

    /// Re-centers the canvas at the top edge of the primary monitor.
    #[allow(dead_code)]
    pub unsafe fn recenter(&self) -> Result<()> {
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let x = (screen_width - self.width_px as i32) / 2;

        SetWindowPos(
            self.hwnd,
            HWND_TOPMOST,
            x,
            0,
            self.width_px as i32,
            self.height_px as i32,
            SWP_NOACTIVATE,
        )?;

        Ok(())
    }
}
