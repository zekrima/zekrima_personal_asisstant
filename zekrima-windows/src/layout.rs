//! Shared layout constants. All values are in device-independent pixels (DIPs)
//! and are scaled by the current monitor DPI before touching Win32/Direct2D.

/// Fixed transparent host canvas width in logical pixels (fits 640px expanded chat).
pub const CANVAS_WIDTH_LOGICAL: f32 = 680.0;
/// Fixed transparent host canvas height in logical pixels (fits 470px large chat).
pub const CANVAS_HEIGHT_LOGICAL: f32 = 500.0;
/// The island floats a few pixels below the top edge of the screen.
pub const PILL_TOP_LOGICAL: f32 = 8.0;

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
