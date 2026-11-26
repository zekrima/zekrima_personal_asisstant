//! Native GTK4 FileDialog picker & attachment loader for Zekrima Chat on Linux.
//!
//! Uses XDG Desktop Portal via `gtk4::FileDialog` (Wayland & KDE/GNOME/Hyprland native)
//! and supports text/code files, images (Vision), and PDF documents.

use std::fs;
use std::path::Path;

use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{FileDialog, FileFilter, Window};

const MAX_TEXT_BYTES: usize = 24_000;
const MAX_BINARY_BYTES: usize = 6 * 1024 * 1024; // 6 MB limit

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
        let kb = self.size_bytes.div_ceil(1024);
        format!("{name} ({kb} KB)")
    }
}

/// Opens the native GTK4 / XDG Portal file chooser asynchronously without blocking the UI.
pub fn open_file_picker<F>(parent: Option<&Window>, on_result: F)
where
    F: FnOnce(Result<FileAttachment, String>) + 'static,
{
    let dialog = FileDialog::builder()
        .title("Pilih file untuk dibaca Zeki")
        .modal(false)
        .build();

    let supported = FileFilter::new();
    supported.set_name(Some("Supported (Text, Code, PDF, Images)"));
    for pat in [
        "*.txt", "*.md", "*.pdf", "*.png", "*.jpg", "*.jpeg", "*.webp", "*.bmp", "*.rs", "*.py",
        "*.js", "*.ts", "*.json", "*.csv", "*.log", "*.html", "*.xml", "*.toml", "*.yaml", "*.yml",
        "*.c", "*.cpp", "*.java", "*.sh",
    ] {
        supported.add_pattern(pat);
    }

    let all_files = FileFilter::new();
    all_files.set_name(Some("All Files"));
    all_files.add_pattern("*");

    let filters = gio::ListStore::new::<FileFilter>();
    filters.append(&supported);
    filters.append(&all_files);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&supported));

    dialog.open(parent, gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(path) = file.path() {
                on_result(load_attachment(&path));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_text_attachment_cleanly() {
        let tmp = std::env::temp_dir().join("zekrima_linux_test_attach.txt");
        fs::write(&tmp, "Halo Zeki Linux, ini isi file contoh.").unwrap();
        let att = load_attachment(&tmp).expect("should read text file");
        let _ = fs::remove_file(&tmp);

        assert_eq!(att.file_name, "zekrima_linux_test_attach.txt");
        match att.data {
            AttachmentData::Text(t) => assert!(t.contains("Halo Zeki Linux")),
            _ => panic!("expected text attachment"),
        }
    }
}
