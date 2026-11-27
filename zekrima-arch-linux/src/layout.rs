//! Shared layout constants in logical pixels (DIPs) plus dynamic monitor scaling.

pub const CANVAS_WIDTH_LOGICAL: f32 = 680.0;
pub const CANVAS_HEIGHT_LOGICAL: f32 = 500.0;
pub const PILL_TOP_LOGICAL: f32 = 8.0;

/// Computes a dynamic UI scale factor from the active monitor's logical resolution
/// so the Dynamic Island scales cleanly on 720p/768p laptops, 1080p, 1440p, and 4K displays.
pub fn compute_ui_scale(mon_w: i32, mon_h: i32) -> f32 {
    if mon_w <= 0 || mon_h <= 0 {
        return 1.0;
    }
    let fit_w = (mon_w as f32 * 0.92) / CANVAS_WIDTH_LOGICAL;
    let fit_h = (mon_h as f32 * 0.72) / CANVAS_HEIGHT_LOGICAL;
    let cap = fit_w.min(fit_h);
    let boost: f32 = if mon_w >= 3000 && mon_h >= 1600 {
        1.22
    } else if mon_w >= 2400 && mon_h >= 1300 {
        1.10
    } else {
        1.0
    };
    boost.min(cap).clamp(0.72, 1.25)
}
