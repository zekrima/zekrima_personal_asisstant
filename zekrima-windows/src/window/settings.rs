//! Native Win32 Settings window for Zekrima Assistant (BYOK API keys & free models).
//!
//! Runs on its own dedicated UI thread with an independent Win32 message loop so
//! it NEVER re-enters the main island's `RefCell<App>` and keeps the Dynamic
//! Island running at 60 fps while configuring providers & API keys.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::thread;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, GetStockObject, SetBkColor, SetBkMode,
    SetTextColor, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DEFAULT_QUALITY,
    FF_SWISS, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS,
    TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, GetSystemMetrics, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW,
    IsWindow, LoadCursorW, LoadIconW, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowTextW, ShowWindow,
    TranslateMessage, ASFW_ANY, CS_HREDRAW, CS_VREDRAW, HMENU, IDC_ARROW, IDI_APPLICATION,
    MB_ICONERROR, MB_OK, MSG, SM_CXSCREEN, SM_CYSCREEN, SW_RESTORE, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DESTROY, WM_SETFONT, WM_USER,
    WNDCLASSW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_MINIMIZEBOX,
    WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::services::config::{
    AssistantConfig, LlmProvider, ALL_HIDE_HOTKEYS, ALL_LLM_PROVIDERS, ALL_PTT_HOTKEYS,
    ALL_SCREEN_RESOLUTIONS, ALL_STT_PROVIDERS, GEMINI_TTS_MODELS, GEMINI_TTS_VOICES,
};

/// Posted back to the main Dynamic Island window once settings are saved.
pub const WM_SETTINGS_SAVED: u32 = WM_USER + 220;

const SETTINGS_CLASS: PCWSTR = w!("ZekrimaSettingsWindow");

// Control IDs
const IDC_PROVIDER_COMBO: usize = 101;
const IDC_MODEL_COMBO: usize = 102;
const IDC_API_KEY_LABEL: usize = 103;
const IDC_API_KEY_EDIT: usize = 104;
const IDC_SHOW_KEYS_CHK: usize = 105;
const IDC_BASE_URL_EDIT: usize = 106;
const IDC_STT_COMBO: usize = 107;
const IDC_GROQ_KEY_EDIT: usize = 108;
const IDC_LANG_COMBO: usize = 109;
const IDC_PROMPT_EDIT: usize = 110;
const IDC_TTS_ENABLED_CHK: usize = 111;
const IDC_TTS_MODEL_COMBO: usize = 112;
const IDC_AUTO_HIDE_CHK: usize = 113;
const IDC_TTS_VOICE_COMBO: usize = 114;
const IDC_PTT_HOTKEY_COMBO: usize = 115;
const IDC_HIDE_HOTKEY_COMBO: usize = 116;
const IDC_SCREEN_RES_COMBO: usize = 117;
const IDC_SAVE_BTN: usize = 120;
const IDC_OPEN_INI_BTN: usize = 121;
const IDC_CANCEL_BTN: usize = 122;

// Standard Win32 control style / message constants
const CBS_DROPDOWN: u32 = 0x0002;
const CBS_DROPDOWNLIST: u32 = 0x0003;
const CBS_AUTOHSCROLL: u32 = 0x0040;
const CBS_HASSTRINGS: u32 = 0x0200;
const CB_ADDSTRING: u32 = 0x0143;
const CB_RESETCONTENT: u32 = 0x014B;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const CBN_SELCHANGE: u16 = 1;

const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_PASSWORD: u32 = 0x0020;
const EM_SETPASSWORDCHAR: u32 = 0x00CC;

const BS_PUSHBUTTON: u32 = 0x0000;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
const BS_AUTOCHECKBOX: u32 = 0x0003;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: usize = 1;
const BN_CLICKED: u16 = 0;

const LANG_OPTIONS: [(&str, &str); 3] = [
    ("id", "id — Bahasa Indonesia"),
    ("en", "en — English"),
    ("auto", "auto — Auto-detect"),
];

/// Tracks currently open Settings HWND across threads (single-instance).
static ACTIVE_SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static STATE: RefCell<Option<SettingsState>> = const { RefCell::new(None) };
}

struct SettingsState {
    main_hwnd: isize,
    cfg: AssistantConfig,
    active_provider: LlmProvider,
    font_regular: HFONT,
    font_bold: HFONT,
    bg_brush: HBRUSH,
    hwnd_provider: HWND,
    hwnd_model: HWND,
    hwnd_key_label: HWND,
    hwnd_api_key: HWND,
    hwnd_show_keys: HWND,
    hwnd_base_url: HWND,
    hwnd_stt: HWND,
    hwnd_groq_key: HWND,
    hwnd_tts_enabled: HWND,
    hwnd_tts_model: HWND,
    hwnd_tts_voice: HWND,
    hwnd_ptt_hotkey: HWND,
    hwnd_hide_hotkey: HWND,
    hwnd_screen_res: HWND,
    hwnd_auto_hide: HWND,
    hwnd_lang: HWND,
    hwnd_prompt: HWND,
}

impl Drop for SettingsState {
    fn drop(&mut self) {
        unsafe {
            if !self.font_regular.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font_regular.0));
            }
            if !self.font_bold.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font_bold.0));
            }
            if !self.bg_brush.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bg_brush.0));
            }
        }
    }
}

/// Opens (or brings to front) the native Zekrima Assistant Settings window.
/// Always non-blocking so calling from the Dynamic Island UI thread is 100% safe.
pub fn open(main_hwnd: HWND) {
    let existing = ACTIVE_SETTINGS_HWND.load(Ordering::SeqCst);
    if existing != 0 {
        let hwnd = HWND(existing as *mut c_void);
        unsafe {
            if IsWindow(hwnd).as_bool() {
                let _ = AllowSetForegroundWindow(ASFW_ANY);
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
                return;
            }
        }
        ACTIVE_SETTINGS_HWND.store(0, Ordering::SeqCst);
    }

    let parent_raw = main_hwnd.0 as isize;
    let _ = thread::Builder::new()
        .name("zekrima-settings-ui".to_owned())
        .spawn(move || unsafe {
            run_settings_window(parent_raw);
        });
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn scale(v: i32, dpi: i32) -> i32 {
    (v * dpi + 48) / 96
}

unsafe fn set_text(hwnd: HWND, text: &str) {
    let w = to_wide(text);
    let _ = SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
}

unsafe fn get_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len + 1) as usize];
    let copied = GetWindowTextW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..copied as usize])
}

unsafe fn combo_add(hwnd: HWND, item: &str) {
    let w = to_wide(item);
    SendMessageW(hwnd, CB_ADDSTRING, WPARAM(0), LPARAM(w.as_ptr() as isize));
}

unsafe fn combo_clear(hwnd: HWND) {
    SendMessageW(hwnd, CB_RESETCONTENT, WPARAM(0), LPARAM(0));
}

unsafe fn combo_select(hwnd: HWND, idx: usize) {
    SendMessageW(hwnd, CB_SETCURSEL, WPARAM(idx), LPARAM(0));
}

unsafe fn combo_cursel(hwnd: HWND) -> Option<usize> {
    let idx = SendMessageW(hwnd, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
    if idx >= 0 {
        Some(idx as usize)
    } else {
        None
    }
}

unsafe fn apply_font(hwnd: HWND, font: HFONT) {
    SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
}

unsafe fn create_ui_font(dpi: i32, size_pt: i32, bold: bool) -> HFONT {
    let height = -((size_pt * dpi) / 72);
    CreateFontW(
        height,
        0,
        0,
        0,
        if bold { FW_SEMIBOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        DEFAULT_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
        w!("Segoe UI"),
    )
}

unsafe fn run_settings_window(main_hwnd_raw: isize) {
    let Ok(module) = GetModuleHandleW(None) else {
        return;
    };
    let instance = HINSTANCE(module.0);

    let bg_color = COLORREF(0x00F6F6F6); // Clean light Windows 11 surface (#F6F6F6)
    let bg_brush = CreateSolidBrush(bg_color);

    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(settings_wndproc),
        hInstance: instance,
        hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hbrBackground: if bg_brush.is_invalid() {
            HBRUSH(GetStockObject(WHITE_BRUSH).0)
        } else {
            bg_brush
        },
        lpszClassName: SETTINGS_CLASS,
        ..Default::default()
    };
    // Ignore ERROR_CLASS_ALREADY_EXISTS on subsequent opens
    let _ = RegisterClassW(&wc);

    let dpi = GetDpiForSystem().max(96) as i32;
    let win_w = scale(530, dpi);
    let win_h = scale(755, dpi);
    let screen_w = GetSystemMetrics(SM_CXSCREEN);
    let screen_h = GetSystemMetrics(SM_CYSCREEN);
    let win_x = (screen_w - win_w) / 2;
    let win_y = (screen_h - win_h) / 2;

    let style = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    let Ok(hwnd) = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        SETTINGS_CLASS,
        w!("Zekrima Assistant — Settings & Free Models"),
        style,
        win_x,
        win_y,
        win_w,
        win_h,
        None,
        None,
        instance,
        None,
    ) else {
        if !bg_brush.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(bg_brush.0));
        }
        return;
    };

    ACTIVE_SETTINGS_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

    let font_regular = create_ui_font(dpi, 9, false);
    let font_bold = create_ui_font(dpi, 10, true);
    let cfg = AssistantConfig::load();
    let active_provider = cfg.llm_provider;

    let pad = scale(20, dpi);
    let full_w = scale(464, dpi);
    let lbl_h = scale(18, dpi);
    let ctl_h = scale(26, dpi);
    let combo_drop_h = scale(220, dpi);
    let mut y = scale(16, dpi);

    let make_label = |text: &str, x: i32, y: i32, w: i32, h: i32, id: usize, bold: bool| -> HWND {
        let wt = to_wide(text);
        let h_ctrl = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(wt.as_ptr()),
            WS_CHILD | WS_VISIBLE,
            x,
            y,
            w,
            h,
            hwnd,
            HMENU(id as *mut c_void),
            instance,
            None,
        )
        .unwrap_or_default();
        apply_font(h_ctrl, if bold { font_bold } else { font_regular });
        h_ctrl
    };

    // Title section
    make_label("AI Brain (LLM Provider & Free Models)", pad, y, full_w, scale(22, dpi), 0, true);
    y += scale(28, dpi);

    // 1. LLM Provider
    make_label("LLM Provider:", pad, y, full_w, lbl_h, 0, false);
    y += scale(20, dpi);
    let hwnd_provider = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
        pad,
        y,
        full_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_PROVIDER_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_provider, font_regular);
    for (idx, p) in ALL_LLM_PROVIDERS.iter().enumerate() {
        combo_add(hwnd_provider, p.display_name());
        if *p == active_provider {
            combo_select(hwnd_provider, idx);
        }
    }
    y += scale(34, dpi);

    // 2. Free Model Preset (editable combo!)
    make_label(
        "Model (pilih preset gratis atau ketik ID model custom):",
        pad,
        y,
        full_w,
        lbl_h,
        0,
        false,
    );
    y += scale(20, dpi);
    let hwnd_model = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | WINDOW_STYLE(CBS_DROPDOWN | CBS_AUTOHSCROLL | CBS_HASSTRINGS),
        pad,
        y,
        full_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_MODEL_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_model, font_regular);
    y += scale(34, dpi);

    // 3. LLM API Key + Show checkbox
    let hwnd_key_label = make_label(active_provider.key_hint(), pad, y, full_w, lbl_h, IDC_API_KEY_LABEL, false);
    y += scale(20, dpi);
    let key_w = scale(384, dpi);
    let hwnd_api_key = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        w!("EDIT"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL | ES_PASSWORD),
        pad,
        y,
        key_w,
        ctl_h,
        hwnd,
        HMENU(IDC_API_KEY_EDIT as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_api_key, font_regular);

    let hwnd_show_keys = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("BUTTON"),
        w!("Show"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX),
        pad + key_w + scale(10, dpi),
        y + scale(3, dpi),
        scale(70, dpi),
        scale(20, dpi),
        hwnd,
        HMENU(IDC_SHOW_KEYS_CHK as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_show_keys, font_regular);
    y += scale(34, dpi);

    // 4. Endpoint Base URL
    make_label("Endpoint Base URL (editable saat pilih OpenAI / Custom):", pad, y, full_w, lbl_h, 0, false);
    y += scale(20, dpi);
    let hwnd_base_url = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        w!("EDIT"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL),
        pad,
        y,
        full_w,
        ctl_h,
        hwnd,
        HMENU(IDC_BASE_URL_EDIT as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_base_url, font_regular);
    y += scale(38, dpi);

    // Section 2: Voice & STT
    make_label("Voice Input (Speech-to-Text & Bahasa)", pad, y, full_w, scale(22, dpi), 0, true);
    y += scale(26, dpi);

    // 5. STT Provider + Language side-by-side
    let left_w = scale(292, dpi);
    let right_x = pad + left_w + scale(12, dpi);
    let right_w = full_w - left_w - scale(12, dpi);
    make_label("Voice STT Provider:", pad, y, left_w, lbl_h, 0, false);
    make_label("Language:", right_x, y, right_w, lbl_h, 0, false);
    y += scale(20, dpi);

    let hwnd_stt = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
        pad,
        y,
        left_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_STT_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_stt, font_regular);
    for (idx, s) in ALL_STT_PROVIDERS.iter().enumerate() {
        combo_add(hwnd_stt, s.display_name());
        if *s == cfg.stt_provider {
            combo_select(hwnd_stt, idx);
        }
    }

    let hwnd_lang = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
        right_x,
        y,
        right_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_LANG_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_lang, font_regular);
    let mut selected_lang = 0usize;
    for (idx, (code, label)) in LANG_OPTIONS.iter().enumerate() {
        combo_add(hwnd_lang, label);
        if cfg.language.eq_ignore_ascii_case(code) {
            selected_lang = idx;
        }
    }
    combo_select(hwnd_lang, selected_lang);
    y += scale(34, dpi);

    // 6. Groq STT Key (helpful whenever LLM is OpenRouter / OpenCode / NVIDIA)
    make_label(
        "Groq API Key untuk Voice STT gratis (opsional bila pakai Gemini):",
        pad,
        y,
        full_w,
        lbl_h,
        0,
        false,
    );
    y += scale(20, dpi);
    let hwnd_groq_key = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        w!("EDIT"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL | ES_PASSWORD),
        pad,
        y,
        full_w,
        ctl_h,
        hwnd,
        HMENU(IDC_GROQ_KEY_EDIT as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_groq_key, font_regular);
    set_text(hwnd_groq_key, &cfg.groq_api_key);
    y += scale(34, dpi);

    // 6b. Gemini 3.8 Flash TTS (Audio Output) + Voice selector
    let tts_chk_w = scale(182, dpi);
    let hwnd_tts_enabled = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("BUTTON"),
        w!("Voice Reply (3.8 TTS)"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX),
        pad,
        y + scale(3, dpi),
        tts_chk_w,
        scale(20, dpi),
        hwnd,
        HMENU(IDC_TTS_ENABLED_CHK as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_tts_enabled, font_regular);
    if cfg.gemini_tts_enabled {
        SendMessageW(hwnd_tts_enabled, BM_SETCHECK, WPARAM(BST_CHECKED), LPARAM(0));
    }

    let tts_combo_x = pad + tts_chk_w + scale(6, dpi);
    let tts_combo_w = scale(178, dpi);
    let hwnd_tts_model = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWN | CBS_AUTOHSCROLL | CBS_HASSTRINGS),
        tts_combo_x,
        y,
        tts_combo_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_TTS_MODEL_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_tts_model, font_regular);
    let mut matched_tts = None;
    for (idx, m) in GEMINI_TTS_MODELS.iter().enumerate() {
        combo_add(hwnd_tts_model, m);
        if *m == cfg.gemini_tts_model {
            matched_tts = Some(idx);
        }
    }
    if let Some(idx) = matched_tts {
        combo_select(hwnd_tts_model, idx);
    } else {
        set_text(hwnd_tts_model, &cfg.gemini_tts_model);
    }

    let voice_x = tts_combo_x + tts_combo_w + scale(8, dpi);
    let voice_w = full_w - (voice_x - pad);
    let hwnd_tts_voice = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
        voice_x,
        y,
        voice_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_TTS_VOICE_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_tts_voice, font_regular);
    let mut matched_voice = 0usize;
    for (idx, v) in GEMINI_TTS_VOICES.iter().enumerate() {
        combo_add(hwnd_tts_voice, v);
        if v.eq_ignore_ascii_case(&cfg.gemini_tts_voice) {
            matched_voice = idx;
        }
    }
    combo_select(hwnd_tts_voice, matched_voice);
    y += scale(36, dpi);

    // Section 3: Hotkeys & Vision Quality
    let half_w = (full_w - scale(12, dpi)) / 2;
    let col2_x = pad + half_w + scale(12, dpi);
    make_label("Push-to-Talk Hotkey:", pad, y, half_w, lbl_h, 0, false);
    make_label("Hide / Show Hotkey:", col2_x, y, half_w, lbl_h, 0, false);
    y += scale(20, dpi);

    let hwnd_ptt_hotkey = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
        pad,
        y,
        half_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_PTT_HOTKEY_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_ptt_hotkey, font_regular);
    for (idx, hk) in ALL_PTT_HOTKEYS.iter().enumerate() {
        combo_add(hwnd_ptt_hotkey, hk.label());
        if *hk == cfg.ptt_hotkey {
            combo_select(hwnd_ptt_hotkey, idx);
        }
    }

    let hwnd_hide_hotkey = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
        col2_x,
        y,
        half_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_HIDE_HOTKEY_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_hide_hotkey, font_regular);
    for (idx, hk) in ALL_HIDE_HOTKEYS.iter().enumerate() {
        combo_add(hwnd_hide_hotkey, hk.label());
        if *hk == cfg.hide_hotkey {
            combo_select(hwnd_hide_hotkey, idx);
        }
    }
    y += scale(32, dpi);

    make_label("Screen Vision Resolution:", pad, y, full_w, lbl_h, 0, false);
    y += scale(20, dpi);
    let hwnd_screen_res = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("COMBOBOX"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST | CBS_HASSTRINGS),
        pad,
        y,
        full_w,
        combo_drop_h,
        hwnd,
        HMENU(IDC_SCREEN_RES_COMBO as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_screen_res, font_regular);
    for (idx, r) in ALL_SCREEN_RESOLUTIONS.iter().enumerate() {
        combo_add(hwnd_screen_res, r.display_name());
        if *r == cfg.screen_resolution {
            combo_select(hwnd_screen_res, idx);
        }
    }
    y += scale(34, dpi);

    // 7. System Prompt
    make_label("System Prompt (Kepribadian Zeki):", pad, y, full_w, lbl_h, 0, false);
    y += scale(20, dpi);
    let prompt_h = scale(54, dpi);
    let hwnd_prompt = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        w!("EDIT"),
        w!(""),
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | WINDOW_STYLE(ES_MULTILINE | ES_AUTOVSCROLL),
        pad,
        y,
        full_w,
        prompt_h,
        hwnd,
        HMENU(IDC_PROMPT_EDIT as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_prompt, font_regular);
    set_text(hwnd_prompt, &cfg.system_prompt);
    y += prompt_h + scale(10, dpi);

    // 8. Auto-Hide toggle + Hide shortcut info
    let hwnd_auto_hide = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("BUTTON"),
        w!("Auto-Hide Island saat idle (sentuh tepi atas untuk muncul • Shortcut Hide: Alt+H)"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX),
        pad,
        y,
        full_w,
        scale(22, dpi),
        hwnd,
        HMENU(IDC_AUTO_HIDE_CHK as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(hwnd_auto_hide, font_regular);
    if cfg.auto_hide {
        SendMessageW(hwnd_auto_hide, BM_SETCHECK, WPARAM(BST_CHECKED), LPARAM(0));
    }
    y += scale(28, dpi);

    // Bottom Action Buttons
    let btn_h = scale(32, dpi);
    let open_ini_btn = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("BUTTON"),
        w!("Open .ini"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON),
        pad,
        y,
        scale(96, dpi),
        btn_h,
        hwnd,
        HMENU(IDC_OPEN_INI_BTN as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(open_ini_btn, font_regular);

    let cancel_w = scale(90, dpi);
    let save_w = scale(126, dpi);
    let save_x = pad + full_w - save_w;
    let cancel_x = save_x - cancel_w - scale(10, dpi);

    let cancel_btn = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("BUTTON"),
        w!("Cancel"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON),
        cancel_x,
        y,
        cancel_w,
        btn_h,
        hwnd,
        HMENU(IDC_CANCEL_BTN as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(cancel_btn, font_regular);

    let save_btn = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("BUTTON"),
        w!("Save && Apply"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON),
        save_x,
        y,
        save_w,
        btn_h,
        hwnd,
        HMENU(IDC_SAVE_BTN as *mut c_void),
        instance,
        None,
    )
    .unwrap_or_default();
    apply_font(save_btn, font_bold);

    let state = SettingsState {
        main_hwnd: main_hwnd_raw,
        cfg,
        active_provider,
        font_regular,
        font_bold,
        bg_brush,
        hwnd_provider,
        hwnd_model,
        hwnd_key_label,
        hwnd_api_key,
        hwnd_show_keys,
        hwnd_base_url,
        hwnd_stt,
        hwnd_groq_key,
        hwnd_tts_enabled,
        hwnd_tts_model,
        hwnd_tts_voice,
        hwnd_ptt_hotkey,
        hwnd_hide_hotkey,
        hwnd_screen_res,
        hwnd_auto_hide,
        hwnd_lang,
        hwnd_prompt,
    };
    populate_provider_fields(&state, active_provider);

    STATE.with(|slot| {
        *slot.borrow_mut() = Some(state);
    });

    let _ = AllowSetForegroundWindow(ASFW_ANY);
    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
        if !IsDialogMessageW(hwnd, &msg).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    STATE.with(|slot| {
        slot.borrow_mut().take();
    });
    ACTIVE_SETTINGS_HWND.store(0, Ordering::SeqCst);
}

unsafe fn populate_provider_fields(st: &SettingsState, provider: LlmProvider) {
    set_text(st.hwnd_key_label, provider.key_hint());
    set_text(st.hwnd_api_key, st.cfg.api_key_for(provider));

    combo_clear(st.hwnd_model);
    let saved_model = st.cfg.model_for(provider);
    let mut matched = None;
    for (idx, preset) in provider.model_presets().iter().enumerate() {
        combo_add(st.hwnd_model, preset);
        if *preset == saved_model {
            matched = Some(idx);
        }
    }
    if let Some(idx) = matched {
        combo_select(st.hwnd_model, idx);
    } else {
        set_text(st.hwnd_model, saved_model);
    }

    let url = if provider == LlmProvider::OpenAi {
        st.cfg.openai_base_url.as_str()
    } else {
        provider.default_base_url()
    };
    set_text(st.hwnd_base_url, url);
    let _ = EnableWindow(st.hwnd_base_url, provider == LlmProvider::OpenAi);

    // Keep Groq STT field synchronized when switching to/from Groq as main LLM
    set_text(st.hwnd_groq_key, &st.cfg.groq_api_key);
    let _ = EnableWindow(st.hwnd_groq_key, provider != LlmProvider::Groq);
}

unsafe fn flush_current_provider_into_cfg(st: &mut SettingsState) {
    let provider = st.active_provider;
    let key = get_text(st.hwnd_api_key);
    let model = get_text(st.hwnd_model);
    st.cfg.set_api_key_for(provider, key);
    st.cfg.set_model_for(provider, model);

    if provider == LlmProvider::OpenAi {
        let url = get_text(st.hwnd_base_url).trim().trim_end_matches('/').to_string();
        if !url.is_empty() {
            st.cfg.openai_base_url = url;
        }
    } else if provider != LlmProvider::Groq {
        let groq_stt_key = get_text(st.hwnd_groq_key).trim().to_string();
        st.cfg.groq_api_key = groq_stt_key;
    }
}

unsafe fn toggle_password_mask(st: &SettingsState) {
    let checked = SendMessageW(st.hwnd_show_keys, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 as usize
        == BST_CHECKED;
    let mask_char: usize = if checked { 0 } else { '•' as usize };
    SendMessageW(st.hwnd_api_key, EM_SETPASSWORDCHAR, WPARAM(mask_char), LPARAM(0));
    SendMessageW(st.hwnd_groq_key, EM_SETPASSWORDCHAR, WPARAM(mask_char), LPARAM(0));
    // Force immediate redraw of both edit boxes
    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(st.hwnd_api_key, None, true);
    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(st.hwnd_groq_key, None, true);
}

unsafe fn save_and_close(hwnd: HWND, st: &mut SettingsState) {
    flush_current_provider_into_cfg(st);
    st.cfg.llm_provider = st.active_provider;

    if let Some(stt_idx) = combo_cursel(st.hwnd_stt) {
        if let Some(&stt) = ALL_STT_PROVIDERS.get(stt_idx) {
            st.cfg.stt_provider = stt;
        }
    }
    if let Some(lang_idx) = combo_cursel(st.hwnd_lang) {
        if let Some((code, _)) = LANG_OPTIONS.get(lang_idx) {
            st.cfg.language = (*code).to_string();
        }
    }
    let tts_checked =
        SendMessageW(st.hwnd_tts_enabled, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 as usize
            == BST_CHECKED;
    st.cfg.gemini_tts_enabled = tts_checked;
    let tts_model = get_text(st.hwnd_tts_model).trim().to_string();
    if !tts_model.is_empty() {
        st.cfg.gemini_tts_model = tts_model;
    }
    if st.active_provider == LlmProvider::Gemini {
        // Keep Gemini audio understanding model synced with active Gemini chat model
        st.cfg.gemini_audio_model = st.cfg.gemini_model.clone();
    }

    if let Some(v_idx) = combo_cursel(st.hwnd_tts_voice) {
        if let Some(&voice) = GEMINI_TTS_VOICES.get(v_idx) {
            st.cfg.gemini_tts_voice = voice.to_string();
        }
    }
    if let Some(p_idx) = combo_cursel(st.hwnd_ptt_hotkey) {
        if let Some(&hk) = ALL_PTT_HOTKEYS.get(p_idx) {
            st.cfg.ptt_hotkey = hk;
        }
    }
    if let Some(h_idx) = combo_cursel(st.hwnd_hide_hotkey) {
        if let Some(&hk) = ALL_HIDE_HOTKEYS.get(h_idx) {
            st.cfg.hide_hotkey = hk;
        }
    }
    if let Some(r_idx) = combo_cursel(st.hwnd_screen_res) {
        if let Some(&res) = ALL_SCREEN_RESOLUTIONS.get(r_idx) {
            st.cfg.screen_resolution = res;
        }
    }
    let auto_hide_checked =
        SendMessageW(st.hwnd_auto_hide, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 as usize
            == BST_CHECKED;
    st.cfg.auto_hide = auto_hide_checked;

    let prompt = get_text(st.hwnd_prompt).trim().to_string();
    if !prompt.is_empty() {
        st.cfg.system_prompt = prompt;
    }

    match st.cfg.save() {
        Ok(()) => {
            if st.main_hwnd != 0 {
                let _ = PostMessageW(
                    HWND(st.main_hwnd as *mut c_void),
                    WM_SETTINGS_SAVED,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            let _ = DestroyWindow(hwnd);
        }
        Err(err) => {
            let msg = to_wide(&format!("Gagal menyimpan konfigurasi:\n{err}"));
            let _ = MessageBoxW(
                hwnd,
                PCWSTR(msg.as_ptr()),
                w!("Zekrima Settings"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

unsafe extern "system" fn settings_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut c_void);
            SetBkMode(hdc, TRANSPARENT);
            SetBkColor(hdc, COLORREF(0x00F6F6F6));
            SetTextColor(hdc, COLORREF(0x0018181B));
            let brush = STATE.with(|slot| {
                slot.try_borrow()
                    .ok()
                    .and_then(|s| s.as_ref().map(|st| st.bg_brush))
            });
            if let Some(b) = brush {
                if !b.is_invalid() {
                    return LRESULT(b.0 as isize);
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as usize;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u16;
            match (id, code) {
                (IDC_PROVIDER_COMBO, CBN_SELCHANGE) => {
                    STATE.with(|slot| {
                        if let Ok(mut guard) = slot.try_borrow_mut() {
                            if let Some(st) = guard.as_mut() {
                                flush_current_provider_into_cfg(st);
                                if let Some(idx) = combo_cursel(st.hwnd_provider) {
                                    if let Some(&next) = ALL_LLM_PROVIDERS.get(idx) {
                                        st.active_provider = next;
                                        populate_provider_fields(st, next);
                                    }
                                }
                            }
                        }
                    });
                    LRESULT(0)
                }
                (IDC_SHOW_KEYS_CHK, BN_CLICKED) => {
                    STATE.with(|slot| {
                        if let Ok(guard) = slot.try_borrow() {
                            if let Some(st) = guard.as_ref() {
                                toggle_password_mask(st);
                            }
                        }
                    });
                    LRESULT(0)
                }
                (IDC_SAVE_BTN, BN_CLICKED) => {
                    STATE.with(|slot| {
                        if let Ok(mut guard) = slot.try_borrow_mut() {
                            if let Some(st) = guard.as_mut() {
                                save_and_close(hwnd, st);
                            }
                        }
                    });
                    LRESULT(0)
                }
                (IDC_OPEN_INI_BTN, BN_CLICKED) => {
                    AssistantConfig::open_in_editor();
                    LRESULT(0)
                }
                (IDC_CANCEL_BTN, BN_CLICKED) => {
                    let _ = DestroyWindow(hwnd);
                    LRESULT(0)
                }
                // Enter / Esc standard dialog command IDs (IDOK = 1, IDCANCEL = 2)
                (1, _) => {
                    STATE.with(|slot| {
                        if let Ok(mut guard) = slot.try_borrow_mut() {
                            if let Some(st) = guard.as_mut() {
                                save_and_close(hwnd, st);
                            }
                        }
                    });
                    LRESULT(0)
                }
                (2, _) => {
                    let _ = DestroyWindow(hwnd);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
