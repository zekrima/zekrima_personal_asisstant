//! `WM_NCHITTEST` and interactive target hit-testing.
//!
//! The host window is a large transparent canvas. Only the currently animated
//! pill rectangle is interactive; every other pixel returns `HTTRANSPARENT` so
//! Windows forwards the mouse message to the window underneath.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, HTCLIENT, HTTRANSPARENT};

use crate::layout::{to_physical, CANVAS_WIDTH_LOGICAL, PILL_TOP_LOGICAL};
use crate::render::spring::PillSprings;
use crate::render::widgets::{self, HitContext, HitTarget};

/// Current pill rectangle in physical client coordinates.
pub fn pill_rect(dpi: u32, springs: &PillSprings) -> RECT {
    let (width, height, _) = springs.geometry();
    let left = to_physical((CANVAS_WIDTH_LOGICAL - width) / 2.0, dpi);
    let top = to_physical(PILL_TOP_LOGICAL, dpi);
    let right = left + to_physical(width, dpi);
    let bottom = top + to_physical(height, dpi);

    RECT {
        left: left.floor() as i32,
        top: top.floor() as i32,
        right: right.ceil() as i32,
        bottom: bottom.ceil() as i32,
    }
}

fn contains(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

/// Handles `WM_NCHITTEST` for the pill (or a thin top-edge reveal strip when auto-hidden).
pub unsafe fn handle(
    hwnd: HWND,
    lparam: LPARAM,
    dpi: u32,
    springs: &PillSprings,
    auto_hidden: bool,
) -> LRESULT {
    let mut point = POINT {
        x: signed_loword(lparam.0),
        y: signed_hiword(lparam.0),
    };

    if !ScreenToClient(hwnd, &mut point).as_bool() {
        return LRESULT(HTTRANSPARENT as isize);
    }

    if auto_hidden {
        // Thin 5px top-center reveal strip so moving cursor to top edge brings the island back
        let strip_w = to_physical(220.0, dpi);
        let left = to_physical((CANVAS_WIDTH_LOGICAL - 220.0) / 2.0, dpi).floor() as i32;
        let right = (left as f32 + strip_w).ceil() as i32;
        let bottom = to_physical(5.0, dpi).ceil().max(4.0) as i32;
        let strip = RECT {
            left,
            top: 0,
            right,
            bottom,
        };
        if contains(&strip, point.x, point.y) {
            return LRESULT(HTCLIENT as isize);
        }
        return LRESULT(HTTRANSPARENT as isize);
    }

    if contains(&pill_rect(dpi, springs), point.x, point.y) {
        LRESULT(HTCLIENT as isize)
    } else {
        LRESULT(HTTRANSPARENT as isize)
    }
}

/// Used by the Escape-key hook: is the system cursor over the pill right now?
pub unsafe fn cursor_over_pill(hwnd: HWND, dpi: u32, springs: &PillSprings) -> bool {
    let mut point = POINT::default();
    if GetCursorPos(&mut point).is_err() {
        return false;
    }

    ScreenToClient(hwnd, &mut point).as_bool()
        && contains(&pill_rect(dpi, springs), point.x, point.y)
}

/// Maps client coordinates in `WM_LBUTTONUP` lParam to an interactive hit target.
pub unsafe fn target_at(
    lparam: LPARAM,
    dpi: u32,
    springs: &PillSprings,
    ctx: &HitContext,
) -> Option<HitTarget> {
    let x = signed_loword(lparam.0);
    let y = signed_hiword(lparam.0);
    target_at_client(x, y, dpi, springs, ctx)
}

/// Same as [`target_at`] but reads the current system cursor position.
pub unsafe fn target_at_cursor(
    hwnd: HWND,
    dpi: u32,
    springs: &PillSprings,
    ctx: &HitContext,
) -> Option<HitTarget> {
    let mut point = POINT::default();
    if GetCursorPos(&mut point).is_err() || !ScreenToClient(hwnd, &mut point).as_bool() {
        return None;
    }

    target_at_client(point.x, point.y, dpi, springs, ctx)
}

fn target_at_client(
    x: i32,
    y: i32,
    dpi: u32,
    springs: &PillSprings,
    ctx: &HitContext,
) -> Option<HitTarget> {
    let pill = pill_rect(dpi, springs);
    if !contains(&pill, x, y) {
        return None;
    }

    let scale = dpi.max(96) as f32 / 96.0;
    let local_x = (x as f32 - pill.left as f32) / scale;
    let local_y = (y as f32 - pill.top as f32) / scale;

    widgets::hit_target_at(local_x, local_y, ctx)
}

fn signed_loword(value: isize) -> i32 {
    (value as u32 & 0xFFFF) as u16 as i16 as i32
}

fn signed_hiword(value: isize) -> i32 {
    ((value as u32 >> 16) & 0xFFFF) as u16 as i16 as i32
}
