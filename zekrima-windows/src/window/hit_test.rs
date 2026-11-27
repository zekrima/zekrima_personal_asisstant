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

/// Handles `WM_NCHITTEST` for the pill (or a thin top-edge reveal strip when auto-hidden,
/// or selective click-through when Cicada Live Subtitle Bar is locked for click-through).
pub unsafe fn handle(
    hwnd: HWND,
    lparam: LPARAM,
    dpi: u32,
    springs: &PillSprings,
    auto_hidden: bool,
    cicada_click_through: bool,
) -> LRESULT {
    let mut point = POINT {
        x: signed_loword(lparam.0),
        y: signed_hiword(lparam.0),
    };

    if !ScreenToClient(hwnd, &mut point).as_bool() {
        return LRESULT(HTTRANSPARENT as isize);
    }

    if auto_hidden {
        // Generous 12px top-center reveal strip matching the alpha-backed sensor in d2d.rs
        // so moving the cursor to the top edge reliably brings the island back.
        let strip_w = to_physical(240.0, dpi);
        let left = to_physical((CANVAS_WIDTH_LOGICAL - 240.0) / 2.0, dpi).floor() as i32;
        let right = (left as f32 + strip_w).ceil() as i32;
        let bottom = to_physical(12.0, dpi).ceil().max(10.0) as i32;
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

    let pill = pill_rect(dpi, springs);

    // In Cicada Click-Through Subtitle Bar mode, pass all clicks through to the meeting
    // app underneath EXCEPT the compact top-right control cluster so user can still unlock/stop.
    if cicada_click_through {
        if !contains(&pill, point.x, point.y) {
            return LRESULT(HTTRANSPARENT as isize);
        }
        let ctrl_w = to_physical(156.0, dpi).ceil() as i32;
        let ctrl_h = to_physical(34.0, dpi).ceil() as i32;
        let ctrl_zone = RECT {
            left: pill.right - ctrl_w,
            top: pill.top,
            right: pill.right,
            bottom: pill.top + ctrl_h,
        };
        if contains(&ctrl_zone, point.x, point.y) {
            return LRESULT(HTCLIENT as isize);
        }
        return LRESULT(HTTRANSPARENT as isize);
    }

    // Extend hover hit-zone flush to top screen edge (y=0) above the pill so pushing
    // the cursor all the way to the top bezel never causes a WM_MOUSELEAVE bounce.
    let hover_rect = RECT {
        left: pill.left,
        top: 0,
        right: pill.right,
        bottom: pill.bottom,
    };
    if !contains(&hover_rect, point.x, point.y) {
        return LRESULT(HTTRANSPARENT as isize);
    }

    LRESULT(HTCLIENT as isize)
}

/// Used by the Escape-key hook & mouse-leave watchdog: is the cursor over the pill or its top bezel?
pub unsafe fn cursor_over_pill(hwnd: HWND, dpi: u32, springs: &PillSprings) -> bool {
    let mut point = POINT::default();
    if GetCursorPos(&mut point).is_err() {
        return false;
    }

    let pill = pill_rect(dpi, springs);
    let hover_rect = RECT {
        left: pill.left,
        top: 0,
        right: pill.right,
        bottom: pill.bottom,
    };
    ScreenToClient(hwnd, &mut point).as_bool() && contains(&hover_rect, point.x, point.y)
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
