//! Shared layout constants. All values are in device-independent pixels (DIPs)
//! and are scaled dynamically by both monitor DPI and physical screen resolution
//! before touching Win32/Direct2D.

/// Fixed transparent host canvas width in logical pixels (fits 640px expanded chat).
pub const CANVAS_WIDTH_LOGICAL: f32 = 680.0;
/// Fixed transparent host canvas height in logical pixels (fits 470px large chat).
pub const CANVAS_HEIGHT_LOGICAL: f32 = 500.0;
/// The island floats a few pixels below the top edge of the screen.
pub const PILL_TOP_LOGICAL: f32 = 8.0;

/// Computes an effective DPI that respects both Windows Display Scaling (`base_dpi`)
/// and the active monitor resolution (`screen_w_px × screen_h_px`), keeping the
/// Dynamic Island proportional on 720p/768p laptops, 1080p@150%, 1440p, and 4K monitors
/// without overflowing the viewport.
pub fn effective_dpi(base_dpi: u32, screen_w_px: i32, screen_h_px: i32) -> u32 {
    let base = base_dpi.max(96) as f32;
    if screen_w_px <= 0 || screen_h_px <= 0 {
        return base.round() as u32;
    }
    let max_canvas_w = (screen_w_px as f32) * 0.92;
    let max_canvas_h = (screen_h_px as f32) * 0.72;
    let fit_w_dpi = (max_canvas_w / CANVAS_WIDTH_LOGICAL) * 96.0;
    let fit_h_dpi = (max_canvas_h / CANVAS_HEIGHT_LOGICAL) * 96.0;
    let fit_cap = fit_w_dpi.min(fit_h_dpi);

    // Gently boost readability on high-resolution 1440p/4K monitors running at 100% scaling
    let logical_w = (screen_w_px as f32) * 96.0 / base;
    let logical_h = (screen_h_px as f32) * 96.0 / base;
    let large_boost = if logical_w >= 3000.0 && logical_h >= 1600.0 {
        1.22
    } else if logical_w >= 2400.0 && logical_h >= 1300.0 {
        1.10
    } else {
        1.0
    };

    (base * large_boost).min(fit_cap).clamp(68.0, 288.0).round() as u32
}

/// Converts logical pixels to physical pixels for a given DPI.
pub fn to_physical(logical: f32, dpi: u32) -> f32 {
    logical * dpi as f32 / 96.0
}

/// Physical size of the host canvas for a given DPI.
pub fn canvas_size_px(dpi: u32) -> (i32, i32) {
    let width = to_physical(CANVAS_WIDTH_LOGICAL, dpi).round() as i32;
    let height = to_physical(CANVAS_HEIGHT_LOGICAL, dpi).round() as i32;
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_dpi_adapts_across_resolutions() {
        assert_eq!(effective_dpi(96, 1920, 1080), 96);
        // 1080p at 175% (168 DPI) caps cleanly so expanded island fits within 72% screen height
        assert!(effective_dpi(168, 1920, 1080) <= 150);
        // 4K at 100% gets a subtle readability boost
        assert!(effective_dpi(96, 3840, 2160) > 105);
        // 1280x720 laptop at 125% caps to stay inside viewport
        assert!(effective_dpi(120, 1280, 720) <= 102);
    }
}
