//! Native Windows file picker & reader for Zekrima Chat attachments.
//!
//! Supports:
//! - Text & source code files (`.txt`, `.md`, `.rs`, `.py`, `.js`, `.ts`, `.json`,
//!   `.csv`, `.log`, `.html`, `.xml`, `.toml`, `.yaml`, etc.)
//! - Images (`.jpg`, `.jpeg`, `.png`, `.webp`, `.bmp`) -> Vision multimodal
//! - PDF documents (`.pdf`) -> native `application/pdf` inline_data for Gemini 3.8 Flash
//!
//! Opens the Windows File Explorer dialog (`GetOpenFileNameW` from `comdlg32.dll`)
//! on a dedicated background thread so the Dynamic Island UI never blocks or re-enters.

use std::ffi::c_void;
use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::Mutex;
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_USER};

/// Posted to the main window when the user selects a file from the native picker.
pub const WM_FILE_ATTACHED: u32 = WM_USER + 225;

const MAX_TEXT_BYTES: usize = 24_000;
const MAX_BINARY_BYTES: usize = 6 * 1024 * 1024; // 6 MB limit for PDF/image attachments

#[derive(Clone, Debug)]
pub enum AttachmentData {
    Text(String),
    Binary { mime_type: String, bytes: Vec<u8> },
}

#[derive(Clone, Debug)]
pub struct FileAttachment {
    pub file_name: String,
    pub size_bytes: usize,
    pub data: AttachmentData,
}

impl FileAttachment {
    pub fn short_label(&self) -> String {
        let name: String = self.file_name.chars().take(24).collect();
        let kb = (self.size_bytes + 1023) / 1024;
        format!("{name} ({kb} KB)")
    }
}

static PENDING_ATTACHMENT: Mutex<Option<Result<FileAttachment, String>>> = Mutex::new(None);

/// Retrieves the file chosen by the user once `WM_FILE_ATTACHED` arrives.
pub fn take_pending_attachment() -> Option<Result<FileAttachment, String>> {
    PENDING_ATTACHMENT.lock().ok()?.take()
}

/// Opens the native Windows Open File dialog on a background thread.
pub fn open_file_picker(main_hwnd: HWND) {
    let hwnd_raw = main_hwnd.0 as isize;
    let _ = thread::Builder::new()
        .name("zekrima-file-picker".to_owned())
        .spawn(move || {
            if let Some(path) = show_open_dialog() {
                let loaded = load_attachment(&path);
                if let Ok(mut slot) = PENDING_ATTACHMENT.lock() {
                    *slot = Some(loaded);
                }
                if hwnd_raw != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            HWND(hwnd_raw as *mut c_void),
                            WM_FILE_ATTACHED,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
        });
}

/// Reads a file from disk into a `FileAttachment` ready for the LLM pipeline.
pub fn load_attachment(path: &Path) -> Result<FileAttachment, String> {
    let meta = fs::metadata(path).map_err(|e| format!("Gagal membaca file: {e}"))?;
    let size_bytes = meta.len() as usize;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let binary_mime = match ext.as_str() {
        "pdf" => Some("application/pdf"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        _ => None,
    };

    if let Some(mime) = binary_mime {
        if size_bytes > MAX_BINARY_BYTES {
            return Err(format!(
                "File terlalu besar ({:.1} MB). Maksimal 6 MB.",
                size_bytes as f64 / (1024.0 * 1024.0)
            ));
        }
        let bytes = fs::read(path).map_err(|e| format!("Gagal membaca file: {e}"))?;
        return Ok(FileAttachment {
            file_name,
            size_bytes,
            data: AttachmentData::Binary {
                mime_type: mime.to_string(),
                bytes,
            },
        });
    }

    // Text / code / document file
    let raw = fs::read(path).map_err(|e| format!("Gagal membaca file: {e}"))?;
    let slice = if raw.len() > MAX_TEXT_BYTES {
        &raw[..MAX_TEXT_BYTES]
    } else {
        &raw[..]
    };
    let mut text = String::from_utf8_lossy(slice).to_string();
    if raw.len() > MAX_TEXT_BYTES {
        text.push_str("\n...[terpotong karena melebihi 24 KB]...");
    }

    Ok(FileAttachment {
        file_name,
        size_bytes,
        data: AttachmentData::Text(text),
    })
}

// =============================================================================
// Native Win32 GetOpenFileNameW binding (comdlg32.dll, zero extra crates)
// =============================================================================

const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
const OFN_NOCHANGEDIR: u32 = 0x0000_0008;

#[repr(C)]
struct OpenFileNameW {
    l_struct_size: u32,
    hwnd_owner: *mut c_void,
    h_instance: *mut c_void,
    lpstr_filter: *const u16,
    lpstr_custom_filter: *mut u16,
    n_max_cust_filter: u32,
    n_filter_index: u32,
    lpstr_file: *mut u16,
    n_max_file: u32,
    lpstr_file_title: *mut u16,
    n_max_file_title: u32,
    lpstr_initial_dir: *const u16,
    lpstr_title: *const u16,
    flags: u32,
    n_file_offset: u16,
    n_file_extension: u16,
    lpstr_def_ext: *const u16,
    l_cust_data: isize,
    lpfn_hook: *const c_void,
    lp_template_name: *const u16,
    pv_reserved: *mut c_void,
    dw_reserved: u32,
    flags_ex: u32,
}

#[link(name = "comdlg32")]
extern "system" {
    fn GetOpenFileNameW(lpofn: *mut OpenFileNameW) -> i32;
}

fn show_open_dialog() -> Option<PathBuf> {
    let filter_str = "All Supported (*.txt;*.md;*.pdf;*.png;*.jpg;*.rs;*.py;*.js;*.json;*.csv)\0*.txt;*.md;*.pdf;*.png;*.jpg;*.jpeg;*.webp;*.rs;*.py;*.js;*.ts;*.json;*.csv;*.log;*.html;*.xml;*.toml;*.yaml;*.yml;*.c;*.cpp;*.java\0PDF & Images (*.pdf;*.png;*.jpg;*.webp)\0*.pdf;*.png;*.jpg;*.jpeg;*.webp;*.bmp\0All Files (*.*)\0*.*\0\0";
    let filter_w: Vec<u16> = filter_str.encode_utf16().collect();
    let title_w: Vec<u16> = "Pilih file untuk dibaca Zeki\0".encode_utf16().collect();
    let mut file_buf = vec![0u16; 1024];

    let mut ofn = OpenFileNameW {
        l_struct_size: size_of::<OpenFileNameW>() as u32,
        hwnd_owner: null_mut(),
        h_instance: null_mut(),
        lpstr_filter: filter_w.as_ptr(),
        lpstr_custom_filter: null_mut(),
        n_max_cust_filter: 0,
        n_filter_index: 1,
        lpstr_file: file_buf.as_mut_ptr(),
        n_max_file: file_buf.len() as u32,
        lpstr_file_title: null_mut(),
        n_max_file_title: 0,
        lpstr_initial_dir: null_mut(),
        lpstr_title: title_w.as_ptr(),
        flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        n_file_offset: 0,
        n_file_extension: 0,
        lpstr_def_ext: null_mut(),
        l_cust_data: 0,
        lpfn_hook: null_mut(),
        lp_template_name: null_mut(),
        pv_reserved: null_mut(),
        dw_reserved: 0,
        flags_ex: 0,
    };

    let ok = unsafe { GetOpenFileNameW(&mut ofn) };
    if ok == 0 {
        return None;
    }

    let len = file_buf
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(file_buf.len());
    if len == 0 {
        return None;
    }
    let path_str = String::from_utf16_lossy(&file_buf[..len]);
    Some(PathBuf::from(path_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_text_attachment_cleanly() {
        let tmp = std::env::temp_dir().join("zekrima_test_attach.txt");
        fs::write(&tmp, "Halo Zeki, ini isi file contoh.").unwrap();
        let att = load_attachment(&tmp).expect("should read text file");
        let _ = fs::remove_file(&tmp);

        assert_eq!(att.file_name, "zekrima_test_attach.txt");
        match att.data {
            AttachmentData::Text(t) => assert!(t.contains("Halo Zeki")),
            _ => panic!("expected text attachment"),
        }
    }
}
