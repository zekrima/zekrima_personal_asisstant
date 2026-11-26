//! Event-driven Windows Clipboard history manager (`AddClipboardFormatListener`).
//!
//! Zero polling: Windows sends `WM_CLIPBOARDUPDATE` directly to the host window
//! whenever text is copied. Clicking an item in the island copies it back into
//! the system clipboard immediately.

use std::ptr::copy_nonoverlapping;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
    RemoveClipboardFormatListener, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

/// Win32 `CF_UNICODETEXT` clipboard format id.
const CF_UNICODETEXT: u32 = 13;
/// Win32 `WM_CLIPBOARDUPDATE` message id.
pub const WM_CLIPBOARDUPDATE: u32 = 0x031D;

const MAX_CLIP_ITEMS: usize = 6;
const MAX_CLIP_CHARS: usize = 2000;

pub struct ClipboardHistory {
    hwnd: HWND,
    listening: bool,
    pub items: Vec<String>,
    pub copied_flash_until: Option<Instant>,
    ignore_next_update: bool,
}

impl ClipboardHistory {
    pub unsafe fn new(hwnd: HWND) -> Self {
        let listening = AddClipboardFormatListener(hwnd).is_ok();
        let mut history = Self {
            hwnd,
            listening,
            items: Vec::with_capacity(MAX_CLIP_ITEMS),
            copied_flash_until: None,
            ignore_next_update: false,
        };
        // Seed initial clipboard text if present.
        history.capture_current();
        history
    }

    /// Called on `WM_CLIPBOARDUPDATE`.
    pub unsafe fn on_clipboard_update(&mut self) -> bool {
        if self.ignore_next_update {
            self.ignore_next_update = false;
            return false;
        }
        self.capture_current()
    }

    unsafe fn capture_current(&mut self) -> bool {
        let Some(text) = read_clipboard_text(self.hwnd) else {
            return false;
        };
        let normalized: String = text
            .trim()
            .chars()
            .take(MAX_CLIP_CHARS)
            .collect();
        if normalized.is_empty() {
            return false;
        }

        if self.items.first().map(|s| s.as_str()) == Some(normalized.as_str()) {
            return false;
        }

        self.items.retain(|existing| existing != &normalized);
        self.items.insert(0, normalized);
        self.items.truncate(MAX_CLIP_ITEMS);
        true
    }

    /// Writes `items[index]` back to the Windows clipboard and moves it to top.
    pub unsafe fn copy_item(&mut self, index: usize) -> bool {
        let Some(text) = self.items.get(index).cloned() else {
            return false;
        };
        self.ignore_next_update = true;
        if write_clipboard_text(self.hwnd, &text) {
            self.items.remove(index);
            self.items.insert(0, text);
            self.copied_flash_until = Some(Instant::now() + Duration::from_millis(1400));
            true
        } else {
            self.ignore_next_update = false;
            false
        }
    }

    /// Writes arbitrary text (e.g. an AI reply) directly to the Windows clipboard.
    pub unsafe fn copy_text(&mut self, text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return false;
        }
        self.ignore_next_update = true;
        if write_clipboard_text(self.hwnd, trimmed) {
            self.copied_flash_until = Some(Instant::now() + Duration::from_millis(1400));
            true
        } else {
            self.ignore_next_update = false;
            false
        }
    }

    /// Reads current plain text from the Windows clipboard (for Ctrl+V paste in chat).
    pub unsafe fn current_text(&self) -> Option<String> {
        read_clipboard_text(self.hwnd)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.copied_flash_until = None;
    }

    pub fn is_copied_flash_active(&self) -> bool {
        self.copied_flash_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }
}

impl Drop for ClipboardHistory {
    fn drop(&mut self) {
        if self.listening {
            unsafe {
                let _ = RemoveClipboardFormatListener(self.hwnd);
            }
        }
    }
}

unsafe fn read_clipboard_text(hwnd: HWND) -> Option<String> {
    if OpenClipboard(hwnd).is_err() {
        return None;
    }
    let result = (|| -> Option<String> {
        let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
        let ptr = GlobalLock(HGLOBAL(handle.0)) as *const u16;
        if ptr.is_null() {
            return None;
        }
        let mut len = 0usize;
        while len < MAX_CLIP_CHARS && *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let text = String::from_utf16_lossy(slice);
        let _ = GlobalUnlock(HGLOBAL(handle.0));
        Some(text)
    })();
    let _ = CloseClipboard();
    result
}

unsafe fn write_clipboard_text(hwnd: HWND, text: &str) -> bool {
    if OpenClipboard(hwnd).is_err() {
        return false;
    }
    let ok = (|| -> bool {
        if EmptyClipboard().is_err() {
            return false;
        }
        let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let byte_len = utf16.len() * std::mem::size_of::<u16>();
        let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, byte_len) else {
            return false;
        };
        let dst = GlobalLock(hmem) as *mut u16;
        if dst.is_null() {
            return false;
        }
        copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
        let _ = GlobalUnlock(hmem);
        SetClipboardData(CF_UNICODETEXT, HANDLE(hmem.0)).is_ok()
    })();
    let _ = CloseClipboard();
    ok
}
