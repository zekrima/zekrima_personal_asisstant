//! On-demand Windows screen capture with in-memory JPEG compression (Zero Retention).
//!
//! - Captures the primary monitor via Win32 GDI `BitBlt` into a 32-bit BGRA DIB.
//! - Scales down to max 1280px width and compresses to JPEG (~72% quality) purely
//!   in memory via WIC (`IWICImagingFactory`) + COM `CreateStreamOnHGlobal`.
//! - Never writes a screenshot file to disk; the JPEG buffer is dropped as soon
//!   as the Vision LLM request completes.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use windows::core::GUID;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR,
    GUID_WICPixelFormat32bppBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapEncoderNoCache, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
};
use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    IStream, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowTextLengthW, GetWindowTextW, SM_CXSCREEN,
    SM_CYSCREEN,
};

/// Captures the active screen into a compact in-memory JPEG byte buffer scaled to `max_width`.
/// Returns `None` if capture or WIC encoding fails.
pub fn capture_screen_jpeg(max_width: u32) -> Option<Vec<u8>> {
    let max_w = max_width.clamp(640, 2560);
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return None;
        }
        let w = width as u32;
        let h = height as u32;

        // 1. Capture desktop pixels via GDI BitBlt into a top-down 32bpp BGRA DIB
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            let _ = ReleaseDC(None, screen_dc);
            return None;
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = null_mut();
        let Ok(dib) = CreateDIBSection(screen_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            return None;
        };
        if bits.is_null() {
            let _ = DeleteObject(HGDIOBJ(dib.0));
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            return None;
        }

        let old_obj: HGDIOBJ = SelectObject(mem_dc, HGDIOBJ(dib.0));
        let blit_ok = BitBlt(mem_dc, 0, 0, width, height, screen_dc, 0, 0, SRCCOPY).is_ok();

        let stride = w * 4;
        let pixel_bytes = if blit_ok {
            let slice = std::slice::from_raw_parts(bits as *const u8, (stride * h) as usize);
            Some(slice.to_vec())
        } else {
            None
        };

        SelectObject(mem_dc, old_obj);
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(None, screen_dc);

        let pixels = pixel_bytes?;
        encode_bgra_to_jpeg(&pixels, w, h, stride, max_w)
    }
}

/// Returns the title of the currently active foreground window (for lightweight context).
pub fn active_window_title() -> String {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return String::new();
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n as usize]);
        let trimmed = title.trim();
        if trimmed.eq_ignore_ascii_case("Zekrima") {
            String::new()
        } else {
            trimmed.chars().take(120).collect()
        }
    }
}

unsafe fn encode_bgra_to_jpeg(
    pixels: &[u8],
    src_w: u32,
    src_h: u32,
    stride: u32,
    max_width: u32,
) -> Option<Vec<u8>> {
    let com_ok = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
    let result = (|| -> Option<Vec<u8>> {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;

        let bitmap = factory
            .CreateBitmapFromMemory(src_w, src_h, &GUID_WICPixelFormat32bppBGRA, stride, pixels)
            .ok()?;

        let (dst_w, dst_h) = if src_w > max_width {
            let scale = max_width as f64 / src_w as f64;
            (max_width, ((src_h as f64) * scale).round().max(1.0) as u32)
        } else {
            (src_w, src_h)
        };

        let scaler = factory.CreateBitmapScaler().ok()?;
        scaler
            .Initialize(&bitmap, dst_w, dst_h, WICBitmapInterpolationModeFant)
            .ok()?;

        let converter = factory.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &scaler,
                &GUID_WICPixelFormat24bppBGR,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .ok()?;

        let stream: IStream = CreateStreamOnHGlobal(HGLOBAL(null_mut()), true).ok()?;
        let encoder = factory
            .CreateEncoder(&GUID_ContainerFormatJpeg, &GUID::zeroed())
            .ok()?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache).ok()?;

        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, null_mut()).ok()?;
        let frame = frame?;

        frame.Initialize(None).ok()?;
        frame.SetSize(dst_w, dst_h).ok()?;
        frame.WriteSource(&converter, null_mut()).ok()?;
        frame.Commit().ok()?;
        encoder.Commit().ok()?;

        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME).ok()?;
        let size = stat.cbSize as usize;
        if size == 0 || size > 4 * 1024 * 1024 {
            return None;
        }

        stream.Seek(0, STREAM_SEEK_SET, None).ok()?;
        let mut out = vec![0u8; size];
        let mut read = 0u32;
        let hr = stream.Read(out.as_mut_ptr() as *mut c_void, size as u32, Some(&mut read));
        if hr.is_err() || read == 0 {
            return None;
        }
        out.truncate(read as usize);
        Some(out)
    })();

    if com_ok {
        CoUninitialize();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_synthetic_bgra_frame_to_valid_jpeg() {
        let w = 64u32;
        let h = 48u32;
        let pixels = vec![120u8; (w * h * 4) as usize];
        let jpeg = unsafe { encode_bgra_to_jpeg(&pixels, w, h, w * 4, 1280) };
        let bytes = jpeg.expect("WIC should encode synthetic BGRA to JPEG");
        assert!(bytes.len() > 100);
        // JPEG SOI marker: FF D8
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8]);
    }
}
