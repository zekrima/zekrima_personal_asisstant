#![windows_subsystem = "windows"]

mod layout;
mod render;
mod services;
mod tray;
mod window;

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;

use windows::core::{w, Error, Result, GUID, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY,
    POWERBROADCAST_SETTING,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_ESCAPE, VK_H, VK_LCONTROL, VK_LMENU,
    VK_LSHIFT, VK_NEXT, VK_PRIOR, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_SPACE, VK_UP,
    VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP, VK_X, VK_Z,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEHID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreatePopupMenu, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, KillTimer, MessageBoxW,
    PostMessageW, PostQuitMessage, SetForegroundWindow, SetTimer, SetWindowsHookExW,
    TrackPopupMenu, TranslateMessage, UnhookWindowsHookEx, DEVICE_NOTIFY_WINDOW_HANDLE, HC_ACTION,
    HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, MA_ACTIVATE, MA_NOACTIVATE, MB_ICONERROR, MB_OK,
    MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, PBT_POWERSETTINGCHANGE, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WH_KEYBOARD_LL, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_INPUT,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_NULL, WM_POWERBROADCAST,
    WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WM_USER,
};

use crate::render::d2d::{Frame, Renderer};
use crate::render::spring::{PillMode, PillSprings};
use crate::render::widgets::{ExpandedTab, HitContext, HitTarget};
use crate::render::zeki::{ZekiEngine, ZekiState};
use crate::services::assistant::{
    AssistantPhase, AssistantService, AssistantSnapshot, RequestContext, WM_ASSISTANT_UPDATED,
};
use crate::services::audio_viz::AudioVisualizer;
use crate::services::clipboard::{ClipboardHistory, WM_CLIPBOARDUPDATE};
use crate::services::config::{AssistantConfig, HideHotkey, PttHotkey};
use crate::services::files::{self, FileAttachment, WM_FILE_ATTACHED};
use crate::services::media::{MediaInfo, MediaService, WM_MEDIA_UPDATED};
use crate::services::notifications::{
    NotificationService, NotificationSnapshot, WM_NOTIFICATION_UPDATED,
};
use crate::services::pomodoro::PomodoroTimer;
use crate::services::sysmon::SystemMonitor;
use crate::services::system::{SystemService, SystemSnapshot, WM_SYSTEM_UPDATED};
use crate::services::voice::CaptureMode;
use crate::tray::{
    sync_tray_state, Tray, IDM_ASSISTANT_CONFIG, IDM_AUTO_HIDE, IDM_EXIT, IDM_NOTIF_SETTINGS,
    IDM_RESET, IDM_STARTUP, IDM_TOGGLE_HIDE, WM_TRAY_COMMAND,
};
use crate::window::fullscreen::{FullscreenMonitor, TIMER_FULLSCREEN_CHECK};
use crate::window::hit_test;
use crate::window::host::HostWindow;
use crate::window::settings::{self, WM_SETTINGS_SAVED};

const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_HOTKEY_PTT_DOWN: u32 = WM_USER + 210;
const WM_HOTKEY_PTT_UP: u32 = WM_USER + 211;
const WM_HOTKEY_CANCEL: u32 = WM_USER + 212;
const WM_HOTKEY_TOGGLE_HIDE: u32 = WM_USER + 213;
const AUTO_HIDE_DELAY_SECS: u64 = 4;
const TIMER_ANIMATION: usize = 1;
const ANIMATION_INTERVAL_FAST_MS: u32 = 16;
const ANIMATION_INTERVAL_IDLE_MS: u32 = 50;
const ANIMATION_INTERVAL_SLEEPY_MS: u32 = 120;
const TIMER_HUD_HIDE: usize = 3;
const TIMER_NOTIF_HIDE: usize = 4;
const HIDE_TIMER_MARGIN_MS: u32 = 30;
const MAX_FRAME_SECONDS: f32 = 0.12;

static ALT_DOWN: AtomicBool = AtomicBool::new(false);
static CTRL_DOWN: AtomicBool = AtomicBool::new(false);
static SHIFT_DOWN: AtomicBool = AtomicBool::new(false);
static PTT_ACTIVE: AtomicBool = AtomicBool::new(false);
static HIDE_KEY_HELD: AtomicBool = AtomicBool::new(false);
static ACTIVE_PTT_HOTKEY: AtomicU8 = AtomicU8::new(0);
static ACTIVE_HIDE_HOTKEY: AtomicU8 = AtomicU8::new(0);

// GUID_DEVICE_POWER_POLICY_VIDEO_BRIGHTNESS {aded5e82-b909-4619-9949-f5d71dac0bcb}
const GUID_MONITOR_BRIGHTNESS: GUID = GUID::from_u128(0xaded5e82_b909_4619_9949_f5d71dac0bcb);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct PowerNotify(HPOWERNOTIFY);

impl Drop for PowerNotify {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterPowerSettingNotification(self.0);
        }
    }
}

struct App {
    host: HostWindow,
    renderer: Renderer,
    springs: PillSprings,
    zeki: ZekiEngine,
    assistant: AssistantService,
    assistant_snapshot: AssistantSnapshot,
    assistant_pinned_open: bool,
    chat_mode: bool,
    chat_large: bool,
    chat_input: String,
    chat_focused: bool,
    chat_scroll_y: f32,
    chat_auto_scroll: bool,
    chat_dragging: bool,
    chat_drag_start_y: f32,
    chat_drag_start_scroll: f32,
    chat_drag_moved: bool,
    attached_file: Option<FileAttachment>,
    last_external_hwnd: Option<HWND>,
    manual_hidden: bool,
    auto_hide_enabled: bool,
    auto_hidden: bool,
    last_visible_activity: Instant,
    ptt_hotkey: PttHotkey,
    hide_hotkey: HideHotkey,
    media: MediaService,
    media_info: MediaInfo,
    audio_viz: AudioVisualizer,
    system: SystemService,
    system_snapshot: SystemSnapshot,
    notifications: NotificationService,
    notification_snapshot: NotificationSnapshot,
    timer: PomodoroTimer,
    clipboard: ClipboardHistory,
    sysmon: SystemMonitor,
    last_clock_minute: (u16, u16),
    expanded: bool,
    expanded_tab: ExpandedTab,
    hovered: bool,
    hovered_target: Option<HitTarget>,
    tracking_leave: bool,
    timer_interval_ms: Option<u32>,
    last_frame: Instant,
    tray: Tray,
    fullscreen_monitor: FullscreenMonitor,
    _power_notify: Option<PowerNotify>,
}

impl App {
    unsafe fn new() -> Result<Self> {
        let host = HostWindow::create(Some(wndproc))?;
        let media = MediaService::start(host.hwnd());
        let media_info = media.snapshot();
        let audio_viz = AudioVisualizer::start();
        audio_viz.set_active(media_info.has_session && media_info.is_playing);
        let system = SystemService::start(host.hwnd());
        let system_snapshot = system.snapshot();
        let notifications = NotificationService::start(host.hwnd());
        let notification_snapshot = notifications.snapshot();
        let clipboard = ClipboardHistory::new(host.hwnd());
        let mut sysmon = SystemMonitor::new();
        sysmon.sample();
        let renderer = Renderer::new(host.width_px(), host.height_px(), host.dpi() as f32)?;

        let tray = Tray::new(host.hwnd())?;
        let fullscreen_monitor = FullscreenMonitor::new(host.hwnd())?;
        let st = GetLocalTime();

        let power_notify = RegisterPowerSettingNotification(
            host.hwnd(),
            &GUID_MONITOR_BRIGHTNESS,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
        .ok()
        .map(PowerNotify);

        let cfg = AssistantConfig::load();
        let auto_hide_enabled = cfg.auto_hide;
        let ptt_hotkey = cfg.ptt_hotkey;
        let hide_hotkey = cfg.hide_hotkey;
        ACTIVE_PTT_HOTKEY.store(ptt_hotkey.to_u8(), Ordering::SeqCst);
        ACTIVE_HIDE_HOTKEY.store(hide_hotkey.to_u8(), Ordering::SeqCst);
        sync_tray_state(auto_hide_enabled, false, hide_hotkey.to_u8());

        let assistant = AssistantService::start(host.hwnd());
        let assistant_snapshot = assistant.snapshot();
        let mut zeki = ZekiEngine::new();
        zeki.set_dancing(media_info.has_session && media_info.is_playing);

        let mut app = Self {
            host,
            renderer,
            springs: PillSprings::new(),
            zeki,
            assistant,
            assistant_snapshot,
            assistant_pinned_open: false,
            chat_mode: false,
            chat_large: false,
            chat_input: String::new(),
            chat_focused: false,
            chat_scroll_y: 0.0,
            chat_auto_scroll: true,
            chat_dragging: false,
            chat_drag_start_y: 0.0,
            chat_drag_start_scroll: 0.0,
            chat_drag_moved: false,
            attached_file: None,
            last_external_hwnd: None,
            manual_hidden: false,
            auto_hide_enabled,
            auto_hidden: false,
            last_visible_activity: Instant::now(),
            ptt_hotkey,
            hide_hotkey,
            media,
            media_info,
            audio_viz,
            system,
            system_snapshot,
            notifications,
            notification_snapshot,
            timer: PomodoroTimer::default(),
            clipboard,
            sysmon,
            last_clock_minute: (st.wHour, st.wMinute),
            expanded: false,
            expanded_tab: ExpandedTab::Zeki,
            hovered: false,
            hovered_target: None,
            tracking_leave: false,
            timer_interval_ms: None,
            last_frame: Instant::now(),
            tray,
            fullscreen_monitor,
            _power_notify: power_notify,
        };

        app.render()?;
        app.host.show();
        app.tray.set_visible(true)?;
        app.update_timer();
        register_raw_input(app.host.hwnd());
        Ok(app)
    }

    unsafe fn render(&mut self) -> Result<()> {
        let (width, height, radius) = self.springs.geometry();
        let voice_level = if self.assistant.is_listening() {
            self.assistant.live_voice_level()
        } else {
            0.0
        };
        let attached_label = self.attached_file.as_ref().map(|f| f.short_label());
        let frame = Frame {
            hidden_visual: self.auto_hidden || self.manual_hidden,
            width,
            height,
            radius,
            expanded: self.expanded,
            expanded_tab: self.expanded_tab,
            zeki: &self.zeki,
            assistant: &self.assistant_snapshot,
            voice_level,
            chat_mode: self.chat_mode,
            chat_large: self.chat_large,
            chat_input: &self.chat_input,
            chat_focused: self.chat_focused,
            chat_scroll_y: self.chat_scroll_y,
            chat_auto_scroll: self.chat_auto_scroll,
            attached_file_label: attached_label.as_deref(),
            ptt_hotkey_short: self.ptt_hotkey.short_label(),
            media: &self.media_info,
            hovered_target: self.hovered_target,
            eq_levels: self.audio_viz.levels(),
            system: &self.system_snapshot,
            notification: &self.notification_snapshot,
            timer: &self.timer,
            clip_items: &self.clipboard.items,
            clip_copied_flash: self.clipboard.is_copied_flash_active(),
            sys_stats: &self.sysmon.stats,
        };
        self.renderer.render(self.host.hwnd(), &frame)
    }

    fn hit_context(&self) -> HitContext<'_> {
        let (pill_width, pill_height, _) = self.springs.geometry();
        HitContext {
            pill_width,
            pill_height,
            expanded: self.expanded,
            expanded_tab: self.expanded_tab,
            has_media: self.media_info.has_session,
            hud_visible: self.hud_visible(),
            toast_visible: self.toast_visible(),
            notifications: &self.notification_snapshot,
            clip_count: self.clipboard.items.len(),
            assistant_chat_open: self.chat_mode || self.assistant_snapshot.has_conversation_view(),
            assistant_has_reply: !self.assistant_snapshot.reply.is_empty(),
            assistant_has_error: self.assistant_snapshot.error.is_some(),
            assistant_has_file: self.attached_file.is_some(),
        }
    }

    fn equalizer_active(&self) -> bool {
        self.media_info.has_session
            && self.media_info.is_playing
            && !self.hud_visible()
            && !self.toast_visible()
            && (!self.expanded || self.expanded_tab == ExpandedTab::Media)
    }

    fn toast_progress_active(&self) -> bool {
        self.toast_visible() && !self.expanded && !self.hud_visible()
    }

    unsafe fn set_expanded(&mut self, expanded: bool) {
        if self.expanded == expanded {
            return;
        }

        self.expanded = expanded;
        if expanded {
            if self.expanded_tab == ExpandedTab::System {
                self.sysmon.sample();
            } else if self.expanded_tab == ExpandedTab::Zeki
                && !self.assistant_snapshot.has_config_key
            {
                self.assistant.refresh_config_status();
            }
        } else {
            self.hovered_target = None;
        }
        self.apply_pill_mode();
        self.update_timer();
    }

    unsafe fn set_expanded_tab(&mut self, tab: ExpandedTab) {
        if self.expanded_tab == tab {
            return;
        }
        self.expanded_tab = tab;
        self.hovered_target = None;
        if tab == ExpandedTab::System {
            self.sysmon.sample();
        }
        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    fn zeki_visible(&self) -> bool {
        if self.hud_visible() {
            return false;
        }
        if self.expanded {
            // Zeki is visible across ALL expanded sub-menus (Hero in Zeki tab + Companion Dock in all others)
            true
        } else if self.toast_visible() {
            false
        } else {
            self.zeki.state().is_assistant_active() || !self.media_info.has_session
        }
    }

    fn desired_timer_interval_ms(&self) -> Option<u32> {
        if self.manual_hidden || self.auto_hidden || self.fullscreen_monitor.is_hidden() {
            return None;
        }
        if !self.springs.is_settled()
            || self.equalizer_active()
            || self.toast_progress_active()
            || (self.zeki_visible()
                && (self.zeki.is_high_energy() || (self.expanded && self.expanded_tab == ExpandedTab::Zeki)))
        {
            Some(ANIMATION_INTERVAL_FAST_MS)
        } else if self.zeki_visible() {
            if !self.expanded && self.zeki.state() == ZekiState::Sleepy {
                Some(ANIMATION_INTERVAL_SLEEPY_MS)
            } else {
                Some(ANIMATION_INTERVAL_IDLE_MS)
            }
        } else {
            None
        }
    }

    unsafe fn update_timer(&mut self) {
        let desired = self.desired_timer_interval_ms();
        if desired == self.timer_interval_ms {
            return;
        }
        match desired {
            Some(ms) => {
                let timer = SetTimer(self.host.hwnd(), TIMER_ANIMATION, ms, None);
                if timer != 0 {
                    if self.timer_interval_ms.is_none() {
                        self.last_frame = Instant::now();
                    }
                    self.timer_interval_ms = Some(ms);
                }
            }
            None => self.stop_timer(),
        }
    }

    unsafe fn stop_timer(&mut self) {
        if self.timer_interval_ms.is_some() {
            let _ = KillTimer(self.host.hwnd(), TIMER_ANIMATION);
            self.timer_interval_ms = None;
        }
    }

    unsafe fn stop_all_timers(&mut self) {
        self.stop_timer();
        let _ = KillTimer(self.host.hwnd(), TIMER_HUD_HIDE);
        let _ = KillTimer(self.host.hwnd(), TIMER_NOTIF_HIDE);
    }

    unsafe fn on_timer(&mut self) -> Result<()> {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_frame)
            .as_secs_f32()
            .clamp(0.004, MAX_FRAME_SECONDS);
        self.last_frame = now;

        // Feed real-time microphone amplitude into Zeki's animated waveform while recording
        if self.assistant.is_listening() {
            self.zeki
                .set_live_voice_level(Some(self.assistant.live_voice_level()));
        } else {
            self.zeki.set_live_voice_level(None);
        }

        let prev_state = self.zeki.state();
        self.zeki.update(dt);
        if self.zeki.state() != prev_state {
            self.apply_pill_mode();
        }
        let _moving = self.springs.update(dt);

        self.render()?;
        self.update_timer();

        Ok(())
    }

    unsafe fn on_assistant_updated(&mut self) {
        let prev_phase = self.assistant_snapshot.phase;
        self.assistant_snapshot = self.assistant.snapshot();
        let phase = self.assistant_snapshot.phase;

        // Sync Zeki's character animation directly with the AI pipeline phase
        match phase {
            AssistantPhase::Listening => {
                self.zeki.set_state_from_assistant(ZekiState::Listening);
            }
            AssistantPhase::Transcribing | AssistantPhase::Thinking => {
                self.zeki.set_state_from_assistant(ZekiState::Thinking);
            }
            AssistantPhase::Speaking => {
                self.zeki.set_state_from_assistant(ZekiState::Speaking);
                if prev_phase != AssistantPhase::Speaking {
                    self.expanded_tab = ExpandedTab::Zeki;
                    self.assistant_pinned_open = true;
                    self.set_expanded(true);
                }
            }
            AssistantPhase::Done => {
                self.zeki.set_state_from_assistant(ZekiState::Happy);
                if !self.assistant_snapshot.reply.is_empty() {
                    self.expanded_tab = ExpandedTab::Zeki;
                    self.assistant_pinned_open = true;
                    self.set_expanded(true);
                }
            }
            AssistantPhase::Error => {
                self.zeki.set_state_from_assistant(ZekiState::Error);
                self.expanded_tab = ExpandedTab::Zeki;
                self.assistant_pinned_open = true;
                self.set_expanded(true);
            }
            AssistantPhase::Idle => {
                self.assistant_pinned_open = false;
                if prev_phase != AssistantPhase::Idle {
                    self.zeki.set_state_from_assistant(ZekiState::Idle);
                }
            }
        }

        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    fn build_request_context(&self) -> RequestContext {
        let clipboard_preview = if self.assistant.is_screen_enabled() {
            self.clipboard.items.first().cloned()
        } else {
            None
        };
        RequestContext {
            notifications: self.notification_snapshot.clone(),
            clipboard_preview,
            attachment: self.attached_file.clone(),
        }
    }

    unsafe fn remember_foreground_window(&mut self) {
        let fg = GetForegroundWindow();
        if !fg.is_invalid() && fg != self.host.hwnd() {
            self.last_external_hwnd = Some(fg);
        }
    }

    unsafe fn on_file_attached(&mut self) {
        if let Some(result) = files::take_pending_attachment() {
            match result {
                Ok(att) => {
                    self.attached_file = Some(att);
                    self.chat_mode = true;
                    self.assistant_pinned_open = true;
                    self.expanded_tab = ExpandedTab::Zeki;
                    self.set_expanded(true);
                    self.zeki.trigger_action_happy();
                }
                Err(err) => {
                    self.notifications.push_local_toast("Zeki File", "Gagal melampirkan file", &err);
                }
            }
            self.apply_pill_mode();
            self.update_timer();
            let _ = self.render();
        }
    }

    fn is_auto_hide_idle(&self) -> bool {
        !self.expanded
            && !self.hovered
            && !self.hud_visible()
            && !self.toast_visible()
            && !self.assistant_snapshot.phase.is_busy()
            && !self.zeki.state().is_assistant_active()
            && !self.timer.running
    }

    unsafe fn reveal_from_auto_hide(&mut self) {
        self.last_visible_activity = Instant::now();
        if self.auto_hidden {
            self.auto_hidden = false;
            self.apply_pill_mode();
            self.update_timer();
            let _ = self.render();
        }
    }

    unsafe fn set_manual_hidden(&mut self, hidden: bool) {
        if self.manual_hidden == hidden {
            return;
        }
        self.manual_hidden = hidden;
        sync_tray_state(
            self.auto_hide_enabled,
            self.manual_hidden,
            self.hide_hotkey.to_u8(),
        );
        if hidden {
            self.set_chat_focus(false);
            self.assistant_pinned_open = false;
            self.hovered = false;
            self.set_expanded(false);
            self.host.hide();
            self.stop_timer();
        } else {
            self.auto_hidden = false;
            self.last_visible_activity = Instant::now();
            self.apply_pill_mode();
            let _ = self.render();
            if !self.fullscreen_monitor.is_hidden() {
                self.host.show();
            }
            self.update_timer();
        }
    }

    unsafe fn toggle_manual_hide(&mut self) {
        let next = !self.manual_hidden;
        self.set_manual_hidden(next);
    }

    unsafe fn toggle_auto_hide(&mut self) {
        self.auto_hide_enabled = !self.auto_hide_enabled;
        sync_tray_state(
            self.auto_hide_enabled,
            self.manual_hidden,
            self.hide_hotkey.to_u8(),
        );
        let mut cfg = AssistantConfig::load();
        cfg.auto_hide = self.auto_hide_enabled;
        let _ = cfg.save();

        if !self.auto_hide_enabled {
            self.reveal_from_auto_hide();
        } else {
            self.last_visible_activity = Instant::now();
        }
    }

    unsafe fn on_hotkey_ptt_down(&mut self) {
        if self.manual_hidden {
            self.set_manual_hidden(false);
        }
        self.reveal_from_auto_hide();
        self.zeki.record_interaction();
        self.expanded_tab = ExpandedTab::Zeki;
        let req_ctx = self.build_request_context();
        self.assistant
            .start_listening(CaptureMode::PushToTalk, req_ctx);
    }

    unsafe fn on_hotkey_ptt_up(&mut self) {
        if self.assistant.is_listening() {
            self.assistant.stop_listening();
        }
    }

    unsafe fn on_hotkey_cancel(&mut self) {
        self.set_chat_focus(false);
        if self.assistant_snapshot.phase.is_busy() {
            self.assistant.cancel();
            return;
        }
        self.assistant_pinned_open = false;
        self.chat_mode = false;
        if !self.hovered {
            self.set_expanded(false);
        } else {
            self.apply_pill_mode();
            let _ = self.render();
        }
    }

    unsafe fn set_chat_focus(&mut self, focused: bool) {
        if focused {
            self.remember_foreground_window();
        }
        self.chat_focused = focused;
        self.host.set_keyboard_focus(focused);
        if focused {
            self.chat_mode = true;
            self.assistant_pinned_open = true;
            self.expanded_tab = ExpandedTab::Zeki;
            self.set_expanded(true);
        }
    }

    unsafe fn submit_chat_input(&mut self) {
        let prompt = self.chat_input.trim().to_string();
        if prompt.is_empty() {
            return;
        }
        self.chat_input.clear();
        self.chat_mode = true;
        self.chat_auto_scroll = true;
        self.assistant_pinned_open = true;
        let req_ctx = self.build_request_context();
        self.assistant.send_text(prompt, req_ctx);
        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    fn scroll_chat_by(&mut self, delta_y: f32) {
        let max_scroll = self.renderer.chat_max_scroll();
        if max_scroll <= 0.0 {
            self.chat_scroll_y = 0.0;
            self.chat_auto_scroll = true;
            return;
        }
        let current = if self.chat_auto_scroll {
            max_scroll
        } else {
            self.chat_scroll_y.clamp(0.0, max_scroll)
        };
        let next = (current + delta_y).clamp(0.0, max_scroll);
        self.chat_scroll_y = next;
        self.chat_auto_scroll = (max_scroll - next) <= 2.0;
    }

    unsafe fn toggle_chat_large(&mut self) {
        self.chat_large = !self.chat_large;
        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    unsafe fn on_mouse_wheel(&mut self, delta: i16) {
        if !self.expanded
            || self.expanded_tab != ExpandedTab::Zeki
            || !(self.chat_mode || self.assistant_snapshot.has_conversation_view())
        {
            return;
        }
        let steps = (delta as f32) / 120.0;
        self.scroll_chat_by(-steps * 36.0);
        let _ = self.render();
    }

    unsafe fn on_lbutton_down(&mut self, lparam: LPARAM) {
        let ctx = self.hit_context();
        if let Some(HitTarget::ZekiChatCard) =
            hit_test::target_at(lparam, self.host.dpi(), &self.springs, &ctx)
        {
            let y_px = ((lparam.0 as u32 >> 16) & 0xFFFF) as u16 as i16 as f32;
            let scale = self.host.dpi().max(96) as f32 / 96.0;
            let max_scroll = self.renderer.chat_max_scroll();
            let current = if self.chat_auto_scroll {
                max_scroll
            } else {
                self.chat_scroll_y.clamp(0.0, max_scroll)
            };
            self.chat_dragging = true;
            self.chat_drag_moved = false;
            self.chat_drag_start_y = y_px / scale;
            self.chat_drag_start_scroll = current;
        }
    }

    unsafe fn on_lbutton_dblclk(&mut self, lparam: LPARAM) {
        self.chat_dragging = false;
        self.chat_drag_moved = false;
        let ctx = self.hit_context();
        if let Some(HitTarget::ZekiChatCard) =
            hit_test::target_at(lparam, self.host.dpi(), &self.springs, &ctx)
        {
            self.toggle_chat_large();
        }
    }

    unsafe fn on_keydown(&mut self, vk: u16) {
        if !self.chat_focused {
            return;
        }
        let delta = match vk {
            x if x == VK_UP.0 => -28.0,
            x if x == VK_DOWN.0 => 28.0,
            x if x == VK_PRIOR.0 => -120.0,
            x if x == VK_NEXT.0 => 120.0,
            _ => return,
        };
        self.scroll_chat_by(delta);
        let _ = self.render();
    }

    unsafe fn on_char(&mut self, code: u32) {
        if !self.chat_focused {
            return;
        }
        self.zeki.record_interaction();
        match code {
            0x0D => {
                // Enter -> send chat message
                self.submit_chat_input();
                return;
            }
            0x08 => {
                // Backspace
                self.chat_input.pop();
            }
            0x7F => {
                // Ctrl+Backspace -> delete last word
                while self.chat_input.ends_with(' ') {
                    self.chat_input.pop();
                }
                while let Some(c) = self.chat_input.chars().last() {
                    if c.is_whitespace() {
                        break;
                    }
                    self.chat_input.pop();
                }
            }
            0x16 => {
                // Ctrl+V -> paste from Windows clipboard
                if let Some(clip) = self.clipboard.current_text() {
                    let clean = clip.replace(['\r', '\n'], " ");
                    for ch in clean.chars() {
                        if self.chat_input.chars().count() >= 500 {
                            break;
                        }
                        self.chat_input.push(ch);
                    }
                }
            }
            0x1B => {
                // Escape
                self.on_hotkey_cancel();
                return;
            }
            c if c >= 0x20 => {
                if self.chat_input.chars().count() < 500 {
                    if let Some(ch) = char::from_u32(c) {
                        self.chat_input.push(ch);
                    }
                }
            }
            _ => return,
        }
        let _ = self.render();
    }

    unsafe fn on_kill_focus(&mut self) {
        if self.chat_focused {
            self.chat_focused = false;
            self.host.set_keyboard_focus(false);
            if !self.hovered && !self.assistant_snapshot.phase.is_busy() {
                self.assistant_pinned_open = false;
                self.set_expanded(false);
            } else {
                let _ = self.render();
            }
        }
    }

    unsafe fn on_media_updated(&mut self) {
        self.media_info = self.media.snapshot();
        let playing = self.media_info.has_session && self.media_info.is_playing;
        self.audio_viz.set_active(playing);
        self.zeki.set_dancing(playing);
        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    /// 1-second housekeeping: minute clock refresh, Pomodoro tick, on-demand SysMon sample.
    unsafe fn on_one_second_tick(&mut self) {
        let mut needs_render = false;

        // 1. Local minute clock rollover
        let st = GetLocalTime();
        let current = (st.wHour, st.wMinute);
        if current != self.last_clock_minute {
            self.last_clock_minute = current;
            needs_render = true;
        }

        // 2. Pomodoro timer countdown (only when actively running)
        if self.timer.running {
            let finished = self.timer.tick();
            needs_render = true;
            if finished {
                self.zeki.trigger_action_happy();
                self.apply_pill_mode();
                self.notifications.push_local_toast(
                    "Pomodoro",
                    "Focus Session Complete!",
                    "Time's up — take a short break.",
                );
            }
        }

        // 2b. Zeki auto-sleep (15 min idle) & happy auto-revert
        if self.zeki.on_one_second_tick() {
            self.apply_pill_mode();
            self.update_timer();
            needs_render = true;
        }

        // 2c. Auto-Hide Dynamic Island after short idle period when enabled
        if self.auto_hide_enabled && !self.manual_hidden && !self.auto_hidden {
            if self.is_auto_hide_idle() {
                if self.last_visible_activity.elapsed().as_secs() >= AUTO_HIDE_DELAY_SECS {
                    self.auto_hidden = true;
                    self.stop_timer();
                    let _ = self.render();
                    return;
                }
            } else {
                self.last_visible_activity = Instant::now();
            }
        }

        // 3. System Monitor (ONLY sampled while user is looking at the Sys tab -> 0% idle CPU)
        if self.expanded && self.expanded_tab == ExpandedTab::System {
            self.sysmon.sample();
            needs_render = true;
        }

        // 4. Clear "Copied!" flash cleanly after ~1.4s
        if self.expanded
            && matches!(self.expanded_tab, ExpandedTab::Clipboard | ExpandedTab::Zeki)
            && self.clipboard.copied_flash_until.is_some()
            && !self.clipboard.is_copied_flash_active()
        {
            self.clipboard.copied_flash_until = None;
            needs_render = true;
        }

        if needs_render && self.timer_interval_ms.is_none() {
            let _ = self.render();
        }
    }

    unsafe fn on_clipboard_update(&mut self) {
        if self.clipboard.on_clipboard_update()
            && self.expanded
            && self.expanded_tab == ExpandedTab::Clipboard
            && self.timer_interval_ms.is_none()
        {
            let _ = self.render();
        }
    }

    unsafe fn on_system_updated(&mut self) {
        self.system_snapshot = self.system.snapshot();
        if !self.manual_hidden {
            self.reveal_from_auto_hide();
        }
        self.zeki.trigger_action_happy();
        self.schedule_hud_hide();
        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    unsafe fn on_notification_updated(&mut self) {
        self.notification_snapshot = self.notifications.snapshot();
        if self.toast_visible() {
            if !self.manual_hidden {
                self.reveal_from_auto_hide();
            }
            self.zeki.on_notification_peek();
        }
        self.schedule_notif_hide();
        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    unsafe fn schedule_hud_hide(&mut self) {
        let _ = KillTimer(self.host.hwnd(), TIMER_HUD_HIDE);
        let now = Instant::now();
        if let Some(until) = self.system_snapshot.hud.visible_until {
            if until > now {
                let wait_ms = (until - now).as_millis() as u32 + HIDE_TIMER_MARGIN_MS;
                let _ = SetTimer(self.host.hwnd(), TIMER_HUD_HIDE, wait_ms, None);
            }
        }
    }

    unsafe fn schedule_notif_hide(&mut self) {
        let _ = KillTimer(self.host.hwnd(), TIMER_NOTIF_HIDE);
        let now = Instant::now();
        if let Some(until) = self.notification_snapshot.toast_until {
            if until > now {
                let wait_ms = (until - now).as_millis() as u32 + HIDE_TIMER_MARGIN_MS;
                let _ = SetTimer(self.host.hwnd(), TIMER_NOTIF_HIDE, wait_ms, None);
            }
        }
    }

    unsafe fn on_hud_timer(&mut self) {
        let _ = KillTimer(self.host.hwnd(), TIMER_HUD_HIDE);
        self.system_snapshot = self.system.snapshot();
        if self.hud_visible() {
            self.schedule_hud_hide();
            return;
        }
        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    unsafe fn on_notif_timer(&mut self) {
        let _ = KillTimer(self.host.hwnd(), TIMER_NOTIF_HIDE);
        self.notification_snapshot = self.notifications.snapshot();
        if self.toast_visible() {
            self.schedule_notif_hide();
            return;
        }
        self.apply_pill_mode();
        self.update_timer();
        let _ = self.render();
    }

    fn hud_visible(&self) -> bool {
        self.system_snapshot.hud.is_visible(Instant::now())
    }

    fn toast_visible(&self) -> bool {
        self.notification_snapshot.is_toast_visible(Instant::now())
    }

    unsafe fn apply_pill_mode(&mut self) {
        let mode = if self.hud_visible() {
            PillMode::Hud
        } else if self.expanded {
            if self.expanded_tab == ExpandedTab::Zeki
                && (self.chat_mode || self.assistant_snapshot.has_conversation_view())
            {
                if self.chat_large {
                    PillMode::ExpandedChatLarge
                } else {
                    PillMode::ExpandedChat
                }
            } else {
                PillMode::Expanded
            }
        } else if self.toast_visible() {
            PillMode::Notification
        } else if self.zeki.state().is_assistant_active()
            || self.assistant_snapshot.phase.is_busy()
        {
            PillMode::Assistant
        } else if self.notification_snapshot.unread_count() > 0 {
            PillMode::CollapsedWithBadge
        } else {
            PillMode::Collapsed
        };
        self.springs.set_mode(mode);
    }

    unsafe fn on_mouse_move(&mut self, lparam: LPARAM) {
        self.reveal_from_auto_hide();
        self.zeki.record_interaction();

        if self.chat_dragging {
            let y_px = ((lparam.0 as u32 >> 16) & 0xFFFF) as u16 as i16 as f32;
            let scale = self.host.dpi().max(96) as f32 / 96.0;
            let dy = (y_px / scale) - self.chat_drag_start_y;
            if dy.abs() > 3.0 {
                self.chat_drag_moved = true;
            }
            let max_scroll = self.renderer.chat_max_scroll();
            let next = (self.chat_drag_start_scroll - dy).clamp(0.0, max_scroll);
            self.chat_scroll_y = next;
            self.chat_auto_scroll = max_scroll <= 0.0 || (max_scroll - next) <= 2.0;
            let _ = self.render();
            return;
        }

        if !self.tracking_leave {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.host.hwnd(),
                dwHoverTime: 0,
            };
            self.tracking_leave = TrackMouseEvent(&mut track).is_ok();
        }

        if !self.hovered {
            self.remember_foreground_window();
            self.hovered = true;
            if !self.toast_visible() {
                self.set_expanded(true);
            }
        }

        let ctx = self.hit_context();
        let target =
            hit_test::target_at_cursor(self.host.hwnd(), self.host.dpi(), &self.springs, &ctx);
        if target != self.hovered_target {
            self.hovered_target = target;
            if self.timer_interval_ms.is_none() {
                let _ = self.render();
            }
        }
    }

    unsafe fn on_mouse_leave(&mut self) {
        self.tracking_leave = false;
        self.hovered = false;
        self.hovered_target = None;
        self.chat_dragging = false;
        self.chat_drag_moved = false;
        // Keep the island open while listening, streaming, or pinned in Chat mode
        if self.assistant_pinned_open
            && (self.expanded_tab == ExpandedTab::Zeki || self.assistant_snapshot.phase.is_busy())
        {
            return;
        }
        self.set_expanded(false);
    }

    unsafe fn on_click(&mut self, lparam: LPARAM) {
        self.zeki.record_interaction();
        let was_drag = self.chat_dragging && self.chat_drag_moved;
        self.chat_dragging = false;
        self.chat_drag_moved = false;
        if was_drag {
            return;
        }
        let ctx = self.hit_context();
        if let Some(target) = hit_test::target_at(lparam, self.host.dpi(), &self.springs, &ctx) {
            match target {
                HitTarget::SelectTab(tab) => {
                    if tab != ExpandedTab::Zeki {
                        self.set_chat_focus(false);
                        self.assistant_pinned_open = false;
                    }
                    self.set_expanded_tab(tab);
                }
                HitTarget::ZekiPoke => {
                    self.zeki.poke();
                    self.apply_pill_mode();
                    self.update_timer();
                    let _ = self.render();
                }
                HitTarget::ZekiSelectState(state) => {
                    self.zeki.set_state_manual(state);
                    self.apply_pill_mode();
                    self.update_timer();
                    let _ = self.render();
                }
                HitTarget::ZekiOpenChat => {
                    self.chat_mode = true;
                    self.chat_auto_scroll = true;
                    self.assistant_pinned_open = true;
                    self.set_chat_focus(true);
                    self.apply_pill_mode();
                    self.update_timer();
                    let _ = self.render();
                }
                HitTarget::ZekiChatInput => {
                    self.set_chat_focus(true);
                    self.update_timer();
                    let _ = self.render();
                }
                HitTarget::ZekiChatCard => {
                    // Single click inside chat card keeps chat open without collapsing the pill
                    return;
                }
                HitTarget::ZekiChatToggleSize => {
                    self.toggle_chat_large();
                }
                HitTarget::ZekiChatSend => {
                    self.submit_chat_input();
                }
                HitTarget::ZekiMicToggle => {
                    self.set_chat_focus(false);
                    if self.assistant_snapshot.phase.is_listening() {
                        self.assistant.stop_listening();
                    } else if self.assistant_snapshot.phase.is_busy() {
                        self.assistant.cancel();
                    } else {
                        // Allow starting voice listening right from ANY sub-menu!
                        self.assistant_pinned_open = true;
                        if self.expanded_tab == ExpandedTab::Zeki {
                            self.chat_mode = true;
                        }
                        let req_ctx = self.build_request_context();
                        self.assistant
                            .start_listening(CaptureMode::HandsFreeVad, req_ctx);
                    }
                }
                HitTarget::ZekiTogglePrivacy => {
                    self.assistant.toggle_screen_access();
                    self.assistant_snapshot = self.assistant.snapshot();
                    let _ = self.render();
                }
                HitTarget::ZekiOpenConfig => {
                    self.set_chat_focus(false);
                    settings::open(self.host.hwnd());
                }
                HitTarget::ZekiAttachFile => {
                    self.set_chat_focus(false);
                    files::open_file_picker(self.host.hwnd());
                }
                HitTarget::ZekiRemoveFile => {
                    self.attached_file = None;
                    let _ = self.render();
                }
                HitTarget::ZekiInsertReply => {
                    let clean_reply = self.assistant_snapshot.reply.trim().to_string();
                    if !clean_reply.is_empty() && self.clipboard.copy_text(&clean_reply) {
                        self.set_chat_focus(false);
                        if let Some(target) = self.last_external_hwnd {
                            crate::services::tools::paste_into_window(target);
                        }
                        self.zeki.trigger_action_happy();
                        self.update_timer();
                        let _ = self.render();
                    }
                }
                HitTarget::ZekiCopyReply => {
                    if self.clipboard.copy_text(&self.assistant_snapshot.reply) {
                        self.zeki.trigger_action_happy();
                        self.update_timer();
                        let _ = self.render();
                    }
                }
                HitTarget::ZekiClearChat => {
                    self.set_chat_focus(false);
                    self.chat_mode = false;
                    self.chat_large = false;
                    self.chat_scroll_y = 0.0;
                    self.chat_auto_scroll = true;
                    self.chat_input.clear();
                    self.attached_file = None;
                    self.assistant_pinned_open = false;
                    self.assistant.clear_conversation();
                }
                HitTarget::MediaControl(control) => {
                    self.media.control(control);
                }
                HitTarget::ToastActivate(id) => {
                    self.notifications.activate(id);
                }
                HitTarget::ToastDismiss(id) => {
                    self.notifications.dismiss(id);
                }
                HitTarget::ClearAllNotifications => {
                    self.notifications.clear_all();
                }
                HitTarget::OpenNotificationSettings => {
                    self.notifications.open_windows_settings();
                }
                HitTarget::NotifActivate(id) => {
                    self.notifications.activate(id);
                }
                HitTarget::NotifDismiss(id) => {
                    self.notifications.dismiss(id);
                }
                HitTarget::TimerToggle => {
                    self.timer.toggle_start_pause();
                    let _ = self.render();
                }
                HitTarget::TimerReset => {
                    self.timer.reset();
                    let _ = self.render();
                }
                HitTarget::TimerPreset(secs) => {
                    self.timer.select_preset(secs);
                    let _ = self.render();
                }
                HitTarget::ClipCopy(index) => {
                    self.clipboard.copy_item(index);
                    self.zeki.trigger_action_happy();
                    self.update_timer();
                    let _ = self.render();
                }
                HitTarget::ClipClear => {
                    self.clipboard.clear();
                    let _ = self.render();
                }
            }
            return;
        }

        if self.toast_visible() && !self.expanded {
            self.notifications.clear_toast();
            return;
        }

        if self.expanded {
            self.set_chat_focus(false);
            self.assistant_pinned_open = false;
        }
        self.set_expanded(!self.expanded);
    }

    unsafe fn on_right_click(&mut self) -> Result<()> {
        let menu = CreatePopupMenu()?;
        let hide_text: Vec<u16> = format!("Hide Island ({})\0", self.hide_hotkey.label())
            .encode_utf16()
            .collect();
        AppendMenuW(menu, MF_STRING, IDM_TOGGLE_HIDE, PCWSTR(hide_text.as_ptr()))?;
        let ah_flag = if self.auto_hide_enabled {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        AppendMenuW(
            menu,
            MF_STRING | ah_flag,
            IDM_AUTO_HIDE,
            w!("Auto-Hide when Idle"),
        )?;
        AppendMenuW(menu, MF_SEPARATOR, 0, w!(""))?;
        AppendMenuW(
            menu,
            MF_STRING,
            IDM_ASSISTANT_CONFIG,
            w!("Assistant Config (API Keys)"),
        )?;
        AppendMenuW(menu, MF_STRING, IDM_NOTIF_SETTINGS, w!("Notification Settings"))?;
        AppendMenuW(menu, MF_STRING, IDM_RESET, w!("Reset Position"))?;
        AppendMenuW(menu, MF_STRING, IDM_EXIT, w!("Exit"))?;

        let _ = SetForegroundWindow(self.host.hwnd());

        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor)?;

        let command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            cursor.x,
            cursor.y,
            0,
            self.host.hwnd(),
            None,
        );

        let _ = DestroyMenu(menu);
        let _ = PostMessageW(self.host.hwnd(), WM_NULL, WPARAM(0), LPARAM(0));

        match command.0 as usize {
            IDM_TOGGLE_HIDE => {
                self.toggle_manual_hide();
            }
            IDM_AUTO_HIDE => {
                self.toggle_auto_hide();
            }
            IDM_ASSISTANT_CONFIG => {
                settings::open(self.host.hwnd());
            }
            IDM_NOTIF_SETTINGS => {
                self.notifications.open_windows_settings();
            }
            IDM_RESET => {
                let _ = self.host.recenter();
            }
            IDM_EXIT => {
                PostMessageW(self.host.hwnd(), WM_CLOSE, WPARAM(0), LPARAM(0))?;
            }
            _ => {}
        }

        Ok(())
    }

    unsafe fn on_dpi_changed(&mut self, dpi: u32) -> Result<()> {
        self.host.apply_dpi(dpi)?;
        self.renderer.resize(
            self.host.width_px(),
            self.host.height_px(),
            self.host.dpi() as f32,
        )?;
        self.render()
    }
}

fn with_app<R>(callback: impl FnOnce(&App) -> R) -> Option<R> {
    APP.with(|slot| slot.try_borrow().ok()?.as_ref().map(callback))
}

fn with_app_mut<R>(callback: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|slot| slot.try_borrow_mut().ok()?.as_mut().map(callback))
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => {
            let hit = with_app(|app| {
                hit_test::handle(hwnd, lparam, app.host.dpi(), &app.springs, app.auto_hidden)
            });
            hit.unwrap_or_else(|| DefWindowProcW(hwnd, message, wparam, lparam))
        }
        WM_MOUSEACTIVATE => {
            let wants_focus = with_app(|app| {
                let ctx = app.hit_context();
                matches!(
                    hit_test::target_at_cursor(app.host.hwnd(), app.host.dpi(), &app.springs, &ctx),
                    Some(HitTarget::ZekiChatInput | HitTarget::ZekiOpenChat)
                )
            })
            .unwrap_or(false);
            if wants_focus {
                LRESULT(MA_ACTIVATE as isize)
            } else {
                LRESULT(MA_NOACTIVATE as isize)
            }
        }
        WM_CHAR => {
            with_app_mut(|app| app.on_char(wparam.0 as u32));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            with_app_mut(|app| app.on_keydown(wparam.0 as u16));
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            with_app_mut(|app| app.on_kill_focus());
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 as u32 >> 16) & 0xFFFF) as u16 as i16;
            with_app_mut(|app| app.on_mouse_wheel(delta));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            with_app_mut(|app| app.on_lbutton_down(lparam));
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            with_app_mut(|app| app.on_lbutton_dblclk(lparam));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if with_app_mut(|app| app.on_mouse_move(lparam)).is_some() {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_MOUSELEAVE => {
            if with_app_mut(|app| app.on_mouse_leave()).is_some() {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_LBUTTONUP => {
            if with_app_mut(|app| app.on_click(lparam)).is_some() {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_MEDIA_UPDATED => {
            if with_app_mut(|app| app.on_media_updated()).is_some() {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_SYSTEM_UPDATED => {
            if with_app_mut(|app| app.on_system_updated()).is_some() {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_NOTIFICATION_UPDATED => {
            if with_app_mut(|app| app.on_notification_updated()).is_some() {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_ASSISTANT_UPDATED => {
            if with_app_mut(|app| app.on_assistant_updated()).is_some() {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_SETTINGS_SAVED => {
            with_app_mut(|app| {
                let cfg = AssistantConfig::load();
                app.auto_hide_enabled = cfg.auto_hide;
                app.ptt_hotkey = cfg.ptt_hotkey;
                app.hide_hotkey = cfg.hide_hotkey;
                ACTIVE_PTT_HOTKEY.store(cfg.ptt_hotkey.to_u8(), Ordering::SeqCst);
                ACTIVE_HIDE_HOTKEY.store(cfg.hide_hotkey.to_u8(), Ordering::SeqCst);
                sync_tray_state(
                    app.auto_hide_enabled,
                    app.manual_hidden,
                    app.hide_hotkey.to_u8(),
                );
                if !app.auto_hide_enabled {
                    app.reveal_from_auto_hide();
                } else {
                    app.last_visible_activity = Instant::now();
                }
                app.assistant.refresh_config_status();
                app.zeki.trigger_action_happy();
                app.update_timer();
                let _ = app.render();
            });
            LRESULT(0)
        }
        WM_FILE_ATTACHED => {
            with_app_mut(|app| app.on_file_attached());
            LRESULT(0)
        }
        WM_HOTKEY_PTT_DOWN => {
            with_app_mut(|app| app.on_hotkey_ptt_down());
            LRESULT(0)
        }
        WM_HOTKEY_PTT_UP => {
            with_app_mut(|app| app.on_hotkey_ptt_up());
            LRESULT(0)
        }
        WM_HOTKEY_CANCEL => {
            with_app_mut(|app| app.on_hotkey_cancel());
            LRESULT(0)
        }
        WM_HOTKEY_TOGGLE_HIDE => {
            with_app_mut(|app| app.toggle_manual_hide());
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE => {
            with_app_mut(|app| app.on_clipboard_update());
            LRESULT(0)
        }
        WM_INPUT => {
            handle_raw_input(lparam);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_POWERSETTINGCHANGE && lparam.0 != 0 {
                let setting = &*(lparam.0 as *const POWERBROADCAST_SETTING);
                if setting.PowerSetting == GUID_MONITOR_BRIGHTNESS
                    && setting.DataLength >= size_of::<u32>() as u32
                {
                    let percent = std::ptr::read_unaligned(setting.Data.as_ptr() as *const u32);
                    with_app(|app| app.system.on_power_brightness(percent));
                    return LRESULT(1);
                }
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_RBUTTONUP => {
            if with_app_mut(|app| {
                let _ = app.on_right_click();
            })
            .is_some()
            {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_TIMER if wparam.0 == TIMER_ANIMATION => {
            match with_app_mut(|app| app.on_timer().is_ok()) {
                Some(true) => LRESULT(0),
                Some(false) => {
                    let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                    LRESULT(0)
                }
                None => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
        WM_TIMER if wparam.0 == TIMER_HUD_HIDE => {
            with_app_mut(|app| app.on_hud_timer());
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_NOTIF_HIDE => {
            with_app_mut(|app| app.on_notif_timer());
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_FULLSCREEN_CHECK => {
            let _ = with_app_mut(|app| {
                let manual_hidden = app.manual_hidden;
                let _ = app.fullscreen_monitor.check_and_react(manual_hidden);
                app.update_timer();
                app.on_one_second_tick();
            });
            LRESULT(0)
        }
        WM_TRAY_COMMAND => {
            if with_app_mut(|app| {
                let command = wparam.0;
                match command {
                    IDM_TOGGLE_HIDE => {
                        app.toggle_manual_hide();
                        Some(true)
                    }
                    IDM_AUTO_HIDE => {
                        app.toggle_auto_hide();
                        Some(true)
                    }
                    IDM_EXIT => {
                        let _ = PostMessageW(app.host.hwnd(), WM_CLOSE, WPARAM(0), LPARAM(0));
                        Some(true)
                    }
                    IDM_STARTUP => {
                        let currently_enabled = crate::tray::is_startup_enabled();
                        let _ = crate::tray::set_startup_registry(!currently_enabled);
                        Some(true)
                    }
                    IDM_ASSISTANT_CONFIG => {
                        settings::open(app.host.hwnd());
                        Some(true)
                    }
                    IDM_NOTIF_SETTINGS => {
                        app.notifications.open_windows_settings();
                        Some(true)
                    }
                    IDM_RESET => {
                        let _ = app.host.recenter();
                        Some(true)
                    }
                    _ => None,
                }
            })
            .is_some()
            {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            match with_app_mut(|app| app.on_dpi_changed(dpi).is_ok()) {
                Some(true) => LRESULT(0),
                Some(false) => {
                    let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                    LRESULT(0)
                }
                None => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            with_app_mut(|app| app.stop_all_timers());
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let msg = wparam.0 as u32;
        let event = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = event.vkCode as u16;

        let ptt_mode = PttHotkey::from_u8(ACTIVE_PTT_HOTKEY.load(Ordering::Relaxed));
        let hide_mode = HideHotkey::from_u8(ACTIVE_HIDE_HOTKEY.load(Ordering::Relaxed));

        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            if vk == VK_LMENU.0 || vk == VK_RMENU.0 {
                ALT_DOWN.store(true, Ordering::SeqCst);
            } else if vk == VK_LCONTROL.0 || vk == VK_RCONTROL.0 {
                CTRL_DOWN.store(true, Ordering::SeqCst);
            } else if vk == VK_LSHIFT.0 || vk == VK_RSHIFT.0 {
                SHIFT_DOWN.store(true, Ordering::SeqCst);
            }

            let alt_held =
                ALT_DOWN.load(Ordering::SeqCst) || (event.flags.0 & LLKHF_ALTDOWN.0) != 0;
            let ctrl_held = CTRL_DOWN.load(Ordering::SeqCst);
            let shift_held = SHIFT_DOWN.load(Ordering::SeqCst);

            // Customizable Hide / Show Shortcut
            let hide_matched = match hide_mode {
                HideHotkey::AltH => vk == VK_H.0 && alt_held && !ctrl_held,
                HideHotkey::CtrlShiftH => vk == VK_H.0 && ctrl_held && shift_held,
                HideHotkey::AltX => vk == VK_X.0 && alt_held && !ctrl_held,
            };
            if hide_matched {
                if !HIDE_KEY_HELD.swap(true, Ordering::SeqCst) {
                    with_app(|app| {
                        let _ = PostMessageW(
                            app.host.hwnd(),
                            WM_HOTKEY_TOGGLE_HIDE,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    });
                }
                return LRESULT(1);
            }

            // Customizable Global Push-to-Talk Hotkey
            let ptt_matched = match ptt_mode {
                PttHotkey::AltSpace => vk == VK_SPACE.0 && alt_held && !ctrl_held,
                PttHotkey::CtrlSpace => vk == VK_SPACE.0 && ctrl_held && !shift_held && !alt_held,
                PttHotkey::AltZ => vk == VK_Z.0 && alt_held && !ctrl_held,
                PttHotkey::CtrlShiftSpace => vk == VK_SPACE.0 && ctrl_held && shift_held,
            };
            if ptt_matched {
                if !PTT_ACTIVE.swap(true, Ordering::SeqCst) {
                    with_app(|app| {
                        let _ = PostMessageW(
                            app.host.hwnd(),
                            WM_HOTKEY_PTT_DOWN,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    });
                }
                return LRESULT(1);
            }

            if vk == VK_ESCAPE.0 {
                let assistant_open_or_busy = with_app(|app| {
                    app.assistant_snapshot.phase.is_busy() || app.assistant_pinned_open
                })
                .unwrap_or(false);

                if assistant_open_or_busy {
                    PTT_ACTIVE.store(false, Ordering::SeqCst);
                    with_app(|app| {
                        let _ = PostMessageW(
                            app.host.hwnd(),
                            WM_HOTKEY_CANCEL,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    });
                    return LRESULT(1);
                }

                let over_pill = with_app(|app| {
                    hit_test::cursor_over_pill(app.host.hwnd(), app.host.dpi(), &app.springs)
                })
                .unwrap_or(false);

                if over_pill {
                    with_app(|app| {
                        let _ = PostMessageW(app.host.hwnd(), WM_CLOSE, WPARAM(0), LPARAM(0));
                    });
                    return LRESULT(1);
                }
            }

            if vk == VK_VOLUME_UP.0 || vk == VK_VOLUME_DOWN.0 || vk == VK_VOLUME_MUTE.0 {
                with_app(|app| app.system.notify_volume_key());
            }
        } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
            if vk == VK_H.0 || vk == VK_X.0 {
                HIDE_KEY_HELD.store(false, Ordering::SeqCst);
            }

            let mut stop_ptt = false;
            if vk == VK_LMENU.0 || vk == VK_RMENU.0 {
                ALT_DOWN.store(false, Ordering::SeqCst);
                if matches!(ptt_mode, PttHotkey::AltSpace | PttHotkey::AltZ) {
                    stop_ptt = PTT_ACTIVE.swap(false, Ordering::SeqCst);
                }
            } else if vk == VK_LCONTROL.0 || vk == VK_RCONTROL.0 {
                CTRL_DOWN.store(false, Ordering::SeqCst);
                if matches!(ptt_mode, PttHotkey::CtrlSpace | PttHotkey::CtrlShiftSpace) {
                    stop_ptt = PTT_ACTIVE.swap(false, Ordering::SeqCst);
                }
            } else if vk == VK_LSHIFT.0 || vk == VK_RSHIFT.0 {
                SHIFT_DOWN.store(false, Ordering::SeqCst);
                if ptt_mode == PttHotkey::CtrlShiftSpace {
                    stop_ptt = PTT_ACTIVE.swap(false, Ordering::SeqCst);
                }
            } else if (vk == VK_SPACE.0 && ptt_mode != PttHotkey::AltZ)
                || (vk == VK_Z.0 && ptt_mode == PttHotkey::AltZ)
            {
                if PTT_ACTIVE.swap(false, Ordering::SeqCst) {
                    with_app(|app| {
                        let _ =
                            PostMessageW(app.host.hwnd(), WM_HOTKEY_PTT_UP, WPARAM(0), LPARAM(0));
                    });
                    return LRESULT(1);
                }
            }

            if stop_ptt {
                with_app(|app| {
                    let _ = PostMessageW(app.host.hwnd(), WM_HOTKEY_PTT_UP, WPARAM(0), LPARAM(0));
                });
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct KeyboardHook(HHOOK);

impl KeyboardHook {
    unsafe fn install() -> Result<Self> {
        let module = GetModuleHandleW(None)?;
        let instance = HINSTANCE(module.0);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), instance, 0)?;
        Ok(Self(hook))
    }
}

impl Drop for KeyboardHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

unsafe fn message_loop() -> Result<()> {
    let mut message = MSG::default();

    loop {
        let result = GetMessageW(&mut message, None, 0, 0);
        if result.0 == -1 {
            return Err(Error::from_win32());
        }
        if result.0 == 0 {
            return Ok(());
        }

        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}

fn register_raw_input(hwnd: HWND) {
    let rid = RAWINPUTDEVICE {
        usUsagePage: 0x0C,
        usUsage: 0x01,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };
    unsafe {
        let _ = RegisterRawInputDevices(&[rid], size_of::<RAWINPUTDEVICE>() as u32);
    }
}

unsafe fn handle_raw_input(lparam: LPARAM) {
    let mut size: u32 = 0;
    let hraw = windows::Win32::UI::Input::HRAWINPUT(lparam.0 as *mut c_void);
    if GetRawInputData(
        hraw,
        RID_INPUT,
        None,
        &mut size,
        size_of::<RAWINPUTHEADER>() as u32,
    ) != 0
        || size == 0
    {
        return;
    }

    let mut buf = vec![0u8; size as usize];
    let read = GetRawInputData(
        hraw,
        RID_INPUT,
        Some(buf.as_mut_ptr() as *mut c_void),
        &mut size,
        size_of::<RAWINPUTHEADER>() as u32,
    );
    if read == u32::MAX || read == 0 {
        return;
    }

    let raw = &*(buf.as_ptr() as *const RAWINPUT);
    if raw.header.dwType != RIM_TYPEHID.0 {
        return;
    }

    let hid = &raw.data.hid;
    let report_len = hid.dwSizeHid as usize;
    let count = hid.dwCount as usize;
    if report_len == 0 || count == 0 {
        return;
    }

    let base = &hid.bRawData as *const u8;
    for i in 0..count {
        let report = std::slice::from_raw_parts(base.add(i * report_len), report_len);
        let mut saw_brightness = false;
        let mut saw_volume = false;
        for start in [0usize, 1] {
            if report.len() >= start + 2 {
                for w in report[start..].chunks_exact(2) {
                    let usage = u16::from_le_bytes([w[0], w[1]]);
                    match usage {
                        0x006F | 0x0070 => saw_brightness = true,
                        0x00E9 | 0x00EA | 0x00E2 => saw_volume = true,
                        _ => {}
                    }
                }
            }
        }
        if saw_brightness {
            with_app(|app| app.system.notify_brightness_key());
        } else if saw_volume {
            with_app(|app| app.system.notify_volume_key());
        }
    }
}

unsafe fn run() -> Result<()> {
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
    CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    let _apartment = ComApartment;

    let mutex_name = w!("Global\\ZekrimaSingleInstanceMutex");
    if let Ok(mutex) = windows::Win32::System::Threading::CreateMutexW(None, false, mutex_name) {
        if windows::Win32::Foundation::GetLastError()
            == windows::Win32::Foundation::ERROR_ALREADY_EXISTS
        {
            return Ok(());
        }
        let _ = mutex;
    }

    let app = App::new()?;
    APP.with(|slot| {
        *slot.borrow_mut() = Some(app);
    });

    let hook = KeyboardHook::install()?;
    let result = message_loop();
    drop(hook);

    APP.with(|slot| {
        slot.borrow_mut().take();
    });

    result
}

fn show_error(message: &str) {
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("Zekrima - Error"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn main() {
    unsafe {
        if let Err(error) = run() {
            show_error(&format!("Zekrima could not start.\n\n{error}"));
        }
    }
}
