//! Zekrima Personal Assistant v0.3.0 — Native Dynamic Island overlay for Arch Linux / CachyOS.
//!
//! Pure Rust + GTK4 + `gtk4-layer-shell` + Cairo/PangoCairo with full feature parity
//! with Windows v0.3.0:
//! - Interactive procedural mascot "Zeki" (7 states, auto-sleep, poke sparks, music dance)
//! - Persistent Zeki Companion Dock across all 6 tabs (`Zeki`, `Media`, `Timer`, `Clip`, `Sys`, `Notifs`)
//! - Native Mode Chat (scrollable multi-turn history, double-click expand `520x310` <-> `640x470`,
//!   file attachments via XDG Portal `FileDialog`, `Insert`/`Copy`/`Clear`)
//! - Multi-provider AI (BYOK): Gemini 3.8 Flash (+ native audio & TTS), OpenRouter (Free),
//!   OpenCode Zen (Free), NVIDIA NIM (Free), Groq, and OpenAI/Custom
//! - Zero-Retention Screen Vision (Standard 1024px / HD 1600px) + Incognito toggle
//! - System Control tools (`wpctl`, `brightnessctl`, Linux app launcher)
//! - Auto-Hide when Idle (4s fade + top-center reveal strip) & Wayland IPC hotkey flags
//!   (`zekrima --toggle-hide`, `zekrima --voice`) + Sleepy low-fps battery-saver mode

mod layout;
mod render;
mod services;
mod settings;

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, Timelike};
use gtk4::cairo;
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib::{self, ControlFlow};
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, CssProvider, DrawingArea, PopoverMenu};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::layout::{CANVAS_HEIGHT_LOGICAL, CANVAS_WIDTH_LOGICAL, PILL_TOP_LOGICAL};
use crate::render::cairo_ui::{CairoRenderer, Frame};
use crate::render::spring::{PillMode, PillSprings};
use crate::render::widgets::{self, ExpandedTab, HitContext, HitTarget};
use crate::render::zeki::{ZekiCicadaAccessory, ZekiEngine, ZekiState};
use crate::services::assistant::{AssistantPhase, AssistantService, AssistantSnapshot, RequestContext};
use crate::services::audio_viz::AudioVisualizer;
use crate::services::cicada::{CicadaService, CicadaSnapshot};
use crate::services::config::{AssistantConfig, HideHotkey, PttHotkey};
use crate::services::files::{self, FileAttachment};
use crate::services::media::{MediaInfo, MediaService};
use crate::services::notifications::{NotificationService, NotificationSnapshot};
use crate::services::pomodoro::PomodoroTimer;
use crate::services::sysmon::SystemMonitor;
use crate::services::system::{SystemService, SystemSnapshot};
use crate::services::tools;
use crate::services::voice::CaptureMode;

const FRAME_MS: u64 = 16;
const FRAME_SLEEPY_MS: u64 = 120;
const AUTO_HIDE_IDLE_SECS: u64 = 10;
const MAX_CLIP_ITEMS: usize = 6;
const MAX_CLIP_CHARS: usize = 2000;
const MAX_CHAT_INPUT_CHARS: usize = 1200;

struct AppState {
    app: Application,
    window: ApplicationWindow,
    area: DrawingArea,
    renderer: CairoRenderer,
    springs: PillSprings,
    zeki: ZekiEngine,
    assistant: AssistantService,
    assistant_snapshot: AssistantSnapshot,
    cicada: CicadaService,
    cicada_snapshot: CicadaSnapshot,
    cicada_subtitle_bar: bool,
    cicada_click_through: bool,
    cicada_large: bool,
    cicada_scroll_offset: f32,
    cicada_auto_scroll: bool,
    cicada_copied_flash_until: Option<Instant>,
    last_cicada_arrival_seq: u64,
    assistant_pinned_open: bool,
    chat_large: bool,
    chat_input: String,
    chat_focused: bool,
    chat_scroll_offset: f32,
    chat_auto_scroll: bool,
    chat_copied_flash_until: Option<Instant>,
    chat_attachment: Option<FileAttachment>,
    media: MediaService,
    media_info: MediaInfo,
    audio_viz: AudioVisualizer,
    system: SystemService,
    system_snapshot: SystemSnapshot,
    notifications: NotificationService,
    notification_snapshot: NotificationSnapshot,
    timer: PomodoroTimer,
    clip_items: Vec<String>,
    clip_copied_flash_until: Option<Instant>,
    ignore_next_clip: bool,
    sysmon: SystemMonitor,
    auto_hide_enabled: bool,
    auto_hidden: bool,
    manual_hidden: bool,
    last_visible_activity: Instant,
    ptt_hotkey: PttHotkey,
    hide_hotkey: HideHotkey,
    last_clock_minute: (u32, u32),
    last_input_rect: Option<(i32, i32, i32, i32)>,
    expanded: bool,
    expanded_tab: ExpandedTab,
    hovered: bool,
    hovered_target: Option<HitTarget>,
    anim_timer_id: Option<glib::SourceId>,
    anim_interval_ms: u64,
    last_anim_tick: Instant,
    hud_timer: Option<glib::SourceId>,
    notif_timer: Option<glib::SourceId>,
}

impl AppState {
    fn assistant_chat_open(&self) -> bool {
        self.assistant_pinned_open || self.assistant_snapshot.has_conversation_view()
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
            clip_count: self.clip_items.len(),
            assistant_chat_open: self.assistant_chat_open(),
            assistant_has_reply: !self.assistant_snapshot.reply.is_empty(),
            assistant_has_error: self.assistant_snapshot.error.is_some(),
            assistant_has_file: self.chat_attachment.is_some(),
            cicada_subtitle_bar: self.cicada_subtitle_bar,
            cicada_has_entries: self.cicada_snapshot.has_entries(),
            cicada_has_quick_reply: self.cicada_snapshot.last_quick_reply.is_some(),
        }
    }

    fn cicada_flash_active(&self) -> bool {
        self.cicada_copied_flash_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }

    fn hud_visible(&self) -> bool {
        self.system_snapshot.hud.is_visible(Instant::now())
    }

    fn toast_visible(&self) -> bool {
        self.notification_snapshot.is_toast_visible(Instant::now())
    }

    fn clip_flash_active(&self) -> bool {
        self.clip_copied_flash_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }

    fn chat_flash_active(&self) -> bool {
        self.chat_copied_flash_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }

    fn equalizer_active(&self) -> bool {
        !self.manual_hidden
            && !self.auto_hidden
            && self.media_info.has_session
            && self.media_info.is_playing
            && !self.hud_visible()
            && !self.toast_visible()
            && (!self.expanded || self.expanded_tab == ExpandedTab::Media)
    }

    fn toast_progress_active(&self) -> bool {
        !self.manual_hidden && self.toast_visible() && !self.expanded && !self.hud_visible()
    }

    fn zeki_needs_animation(&self) -> bool {
        if self.manual_hidden || self.auto_hidden {
            return false;
        }
        self.expanded
            || self.cicada_subtitle_bar
            || self.cicada_snapshot.active
            || self.cicada_snapshot.quick_reply_recording
            || self.zeki.is_high_energy()
            || !self.media_info.has_session
            || self.zeki.state().is_assistant_active()
    }

    fn desired_anim_interval_ms(&self) -> Option<u64> {
        if self.manual_hidden {
            return None;
        }
        if self.auto_hidden {
            return if !self.springs.is_settled() {
                Some(FRAME_MS)
            } else {
                None
            };
        }
        let needs_fast = !self.springs.is_settled()
            || self.equalizer_active()
            || self.toast_progress_active()
            || self.cicada_snapshot.active
            || self.cicada_snapshot.quick_reply_recording
            || self.zeki.is_high_energy()
            || self.expanded;
        if needs_fast {
            return Some(FRAME_MS);
        }
        if self.zeki_needs_animation() {
            if self.zeki.state() == ZekiState::Sleepy && !self.expanded {
                return Some(FRAME_SLEEPY_MS);
            }
            return Some(33);
        }
        None
    }

    fn pill_origin(&self) -> (f32, f32, f32, f32) {
        let (w, h, _) = self.springs.geometry();
        let left = (CANVAS_WIDTH_LOGICAL - w) / 2.0;
        (left, PILL_TOP_LOGICAL, w, h)
    }

    fn update_input_region(&mut self) {
        let rect_tuple = if self.manual_hidden {
            (0, 0, 1, 1)
        } else if self.auto_hidden {
            // Generous 12px top-center reveal strip so hovering top-center wakes the island
            let strip_w = 240;
            let sx = ((CANVAS_WIDTH_LOGICAL as i32) - strip_w) / 2;
            (sx, 0, strip_w, 12)
        } else if !self.expanded && self.cicada_subtitle_bar && self.cicada_click_through {
            // Selective Click-Through Subtitle Bar: only top-right controls capture clicks!
            let (left, top, w, _) = self.pill_origin();
            let ctrl_w = 156;
            let ctrl_x = (left + w).ceil() as i32 - ctrl_w;
            (ctrl_x, top.floor() as i32, ctrl_w, 34)
        } else {
            let (left, top, w, h) = self.pill_origin();
            (
                left.floor() as i32,
                0,
                w.ceil() as i32,
                (top + h).ceil() as i32,
            )
        };

        if self.last_input_rect == Some(rect_tuple) {
            return;
        }
        if let Some(surface) = self.window.surface() {
            let rect = cairo::RectangleInt::new(
                rect_tuple.0,
                rect_tuple.1,
                rect_tuple.2.max(1),
                rect_tuple.3.max(1),
            );
            let region = cairo::Region::create_rectangle(&rect);
            surface.set_input_region(Some(&region));
            self.last_input_rect = Some(rect_tuple);
        }
    }

    fn apply_pill_mode(&mut self) {
        let mode = if self.auto_hidden && !self.manual_hidden {
            PillMode::AutoHidden
        } else if self.hud_visible() {
            PillMode::Hud
        } else if self.expanded {
            if self.expanded_tab == ExpandedTab::Zeki && self.assistant_chat_open() {
                if self.chat_large {
                    PillMode::ExpandedChatLarge
                } else {
                    PillMode::ExpandedChat
                }
            } else if self.expanded_tab == ExpandedTab::Cicada {
                if self.cicada_large {
                    PillMode::ExpandedChatLarge
                } else {
                    PillMode::ExpandedCicada
                }
            } else {
                PillMode::Expanded
            }
        } else if self.toast_visible() {
            PillMode::Notification
        } else if self.cicada_subtitle_bar {
            PillMode::CicadaSubtitle
        } else if self.cicada_snapshot.active
            || self.cicada_snapshot.quick_reply_recording
            || self.zeki.state().is_assistant_active()
        {
            PillMode::Assistant
        } else if self.notification_snapshot.unread_count() > 0 {
            PillMode::CollapsedWithBadge
        } else {
            PillMode::Collapsed
        };
        self.springs.set_mode(mode);
    }

    fn set_chat_focus(&mut self, focused: bool) {
        if self.chat_focused == focused {
            return;
        }
        self.chat_focused = focused;
        if gtk4_layer_shell::is_supported() {
            self.window.set_keyboard_mode(if focused {
                KeyboardMode::Exclusive
            } else {
                KeyboardMode::None
            });
        }
        if focused {
            self.window.present();
            self.area.grab_focus();
        }
    }

    fn set_expanded(&mut self, expanded: bool) {
        if self.expanded == expanded {
            return;
        }
        self.expanded = expanded;
        if expanded {
            self.last_visible_activity = Instant::now();
            self.auto_hidden = false;
            self.zeki.record_interaction();
            if self.expanded_tab == ExpandedTab::System {
                self.sysmon.sample();
            }
        } else {
            self.hovered_target = None;
            self.set_chat_focus(false);
        }
        self.apply_pill_mode();
        self.update_input_region();
    }

    fn set_expanded_tab(&mut self, tab: ExpandedTab) {
        if self.expanded_tab == tab {
            return;
        }
        self.expanded_tab = tab;
        self.hovered_target = None;
        if tab != ExpandedTab::Zeki {
            self.set_chat_focus(false);
        }
        if tab == ExpandedTab::System {
            self.sysmon.sample();
        }
        self.apply_pill_mode();
        self.area.queue_draw();
    }

    fn open_chat_panel(&mut self, focus_input: bool) {
        self.reveal_from_auto_hide();
        self.expanded_tab = ExpandedTab::Zeki;
        self.assistant_pinned_open = true;
        self.set_expanded(true);
        self.apply_pill_mode();
        if focus_input {
            self.set_chat_focus(true);
        }
        self.area.queue_draw();
    }

    fn build_request_context(&mut self) -> RequestContext {
        let clip = self.clip_items.first().cloned();
        let attachment = self.chat_attachment.take();
        RequestContext {
            notifications: self.notification_snapshot.clone(),
            clipboard_preview: clip,
            attachment,
            meeting_transcript: self.cicada.build_qa_context(),
        }
    }

    fn sync_cicada_snapshot(&mut self) {
        let snap = self.cicada.snapshot();
        if snap.arrival_seq != self.last_cicada_arrival_seq {
            self.last_cicada_arrival_seq = snap.arrival_seq;
            self.zeki.trigger_nod();
            if self.cicada_auto_scroll {
                self.cicada_scroll_offset = f32::MAX;
            }
            if let Some(qr) = &snap.last_quick_reply {
                if qr.id == snap.entries.last().map(|e| e.id).unwrap_or(0) {
                    if let Some(display) = gdk::Display::default() {
                        self.ignore_next_clip = true;
                        display.clipboard().set_text(&qr.translation);
                    }
                }
            }
        }
        self.cicada_snapshot = snap;
        if self.cicada_snapshot.active || self.cicada_snapshot.quick_reply_recording {
            self.reveal_from_auto_hide();
        }
        self.apply_pill_mode();
        self.update_input_region();
        self.area.queue_draw();
    }

    fn submit_chat_input(&mut self) {
        let text = self.chat_input.trim().to_string();
        if text.is_empty() && self.chat_attachment.is_none() {
            return;
        }
        let prompt = if text.is_empty() {
            "Tolong baca dan jelaskan isi file yang saya lampirkan.".to_string()
        } else {
            text
        };
        self.chat_input.clear();
        self.chat_auto_scroll = true;
        let ctx = self.build_request_context();
        self.assistant.send_text(prompt, ctx);
        self.area.queue_draw();
    }

    fn toggle_voice_capture(&mut self, mode: CaptureMode) {
        self.reveal_from_auto_hide();
        if self.assistant.is_listening() {
            self.assistant.stop_listening();
        } else {
            self.open_chat_panel(false);
            let ctx = self.build_request_context();
            self.assistant.start_listening(mode, ctx);
        }
        self.area.queue_draw();
    }

    fn sync_assistant_to_zeki(&mut self) {
        let snap = self.assistant.snapshot();
        let phase = snap.phase;
        let prev_len = self.assistant_snapshot.reply.len();
        let prev_msgs = self.assistant_snapshot.messages.len();
        self.assistant_snapshot = snap;

        if self.chat_auto_scroll
            && (self.assistant_snapshot.reply.len() != prev_len
                || self.assistant_snapshot.messages.len() != prev_msgs)
        {
            self.chat_scroll_offset = f32::MAX;
        }

        let target_state = match phase {
            AssistantPhase::Idle => {
                self.zeki.set_live_voice_level(None);
                ZekiState::Idle
            }
            AssistantPhase::Listening => {
                self.zeki
                    .set_live_voice_level(Some(self.assistant.live_voice_level()));
                ZekiState::Listening
            }
            AssistantPhase::Transcribing | AssistantPhase::Thinking => {
                self.zeki.set_live_voice_level(None);
                ZekiState::Thinking
            }
            AssistantPhase::Speaking => {
                self.zeki.set_live_voice_level(None);
                ZekiState::Speaking
            }
            AssistantPhase::Done => {
                self.zeki.set_live_voice_level(None);
                ZekiState::Happy
            }
            AssistantPhase::Error => {
                self.zeki.set_live_voice_level(None);
                ZekiState::Error
            }
        };
        self.zeki.set_state_from_assistant(target_state);
        if phase.is_busy() {
            self.reveal_from_auto_hide();
        }
        self.apply_pill_mode();
        self.area.queue_draw();
    }

    fn set_manual_hidden(&mut self, hidden: bool) {
        if self.manual_hidden == hidden {
            return;
        }
        self.manual_hidden = hidden;
        if hidden {
            self.set_chat_focus(false);
            self.assistant_pinned_open = false;
            self.hovered = false;
            self.set_expanded(false);
            self.window.set_visible(false);
        } else {
            self.auto_hidden = false;
            self.last_visible_activity = Instant::now();
            self.apply_pill_mode();
            self.window.set_visible(true);
            self.update_input_region();
            self.area.queue_draw();
        }
    }

    fn toggle_manual_hide(&mut self) {
        if self.auto_hidden && !self.manual_hidden {
            self.reveal_from_auto_hide();
            return;
        }
        let next = !self.manual_hidden;
        self.set_manual_hidden(next);
    }

    fn toggle_auto_hide(&mut self) {
        self.auto_hide_enabled = !self.auto_hide_enabled;
        let mut cfg = AssistantConfig::load();
        cfg.auto_hide = self.auto_hide_enabled;
        let _ = cfg.save();
        if !self.auto_hide_enabled {
            self.reveal_from_auto_hide();
        } else {
            self.last_visible_activity = Instant::now();
        }
    }

    fn reveal_from_auto_hide(&mut self) {
        self.last_visible_activity = Instant::now();
        if self.manual_hidden {
            self.set_manual_hidden(false);
            return;
        }
        if self.auto_hidden {
            self.auto_hidden = false;
            self.apply_pill_mode();
            self.update_input_region();
            self.area.queue_draw();
        }
    }

    fn can_auto_hide_now(&self) -> bool {
        self.auto_hide_enabled
            && !self.manual_hidden
            && !self.expanded
            && !self.hovered
            && !self.chat_focused
            && !self.hud_visible()
            && !self.toast_visible()
            && !self.assistant_snapshot.phase.is_busy()
            && !self.timer.running
            && !self.cicada_snapshot.active
            && !self.cicada_subtitle_bar
    }

    fn push_clip_text(&mut self, raw: &str) -> bool {
        let normalized: String = raw.trim().chars().take(MAX_CLIP_CHARS).collect();
        if normalized.is_empty() {
            return false;
        }
        if self.clip_items.first().map(|s| s.as_str()) == Some(normalized.as_str()) {
            return false;
        }
        self.clip_items.retain(|existing| existing != &normalized);
        self.clip_items.insert(0, normalized);
        self.clip_items.truncate(MAX_CLIP_ITEMS);
        true
    }
}

fn ensure_anim_timer(state_rc: &Rc<RefCell<AppState>>) {
    let desired = {
        let mut st = state_rc.borrow_mut();
        let target = st.desired_anim_interval_ms();
        match (target, st.anim_timer_id.is_some()) {
            (None, true) => {
                if let Some(id) = st.anim_timer_id.take() {
                    id.remove();
                }
                st.anim_interval_ms = 0;
                return;
            }
            (None, false) => return,
            (Some(ms), true) if st.anim_interval_ms == ms => return,
            (Some(ms), true) => {
                if let Some(id) = st.anim_timer_id.take() {
                    id.remove();
                }
                st.anim_interval_ms = ms;
                st.last_anim_tick = Instant::now();
                ms
            }
            (Some(ms), false) => {
                st.anim_interval_ms = ms;
                st.last_anim_tick = Instant::now();
                ms
            }
        }
    };

    let weak = Rc::downgrade(state_rc);
    let id = glib::timeout_add_local(Duration::from_millis(desired), move || {
        let Some(rc) = weak.upgrade() else {
            return ControlFlow::Break;
        };
        let next_interval = {
            let mut st = rc.borrow_mut();
            let now = Instant::now();
            let dt = now
                .duration_since(st.last_anim_tick)
                .as_secs_f32()
                .clamp(0.004, 0.12);
            st.last_anim_tick = now;

            let _moving = st.springs.update(dt);
            if st.assistant.is_listening() {
                let level = st.assistant.live_voice_level();
                st.zeki.set_live_voice_level(Some(level));
            }
            let cicada_acc = if st.expanded && st.expanded_tab == ExpandedTab::Cicada {
                ZekiCicadaAccessory::TakingNotes
            } else if st.cicada_snapshot.active
                || st.cicada_snapshot.quick_reply_recording
                || st.cicada_subtitle_bar
            {
                ZekiCicadaAccessory::Headset
            } else {
                ZekiCicadaAccessory::None
            };
            let cicada_lvl = st.cicada.live_audio_level();
            st.zeki.set_cicada_mode(cicada_acc, cicada_lvl);
            st.zeki.update(dt);
            st.update_input_region();
            st.area.queue_draw();

            let desired_now = st.desired_anim_interval_ms();
            if desired_now != Some(st.anim_interval_ms) {
                st.anim_timer_id = None;
                st.anim_interval_ms = 0;
                Some(desired_now)
            } else {
                None
            }
        };

        if let Some(maybe_ms) = next_interval {
            if maybe_ms.is_some() {
                ensure_anim_timer(&rc);
            }
            ControlFlow::Break
        } else {
            ControlFlow::Continue
        }
    });
    state_rc.borrow_mut().anim_timer_id = Some(id);
}

fn schedule_hud_hide(state_rc: &Rc<RefCell<AppState>>) {
    let wait_ms = {
        let mut st = state_rc.borrow_mut();
        if let Some(id) = st.hud_timer.take() {
            id.remove();
        }
        let now = Instant::now();
        st.system_snapshot
            .hud
            .visible_until
            .and_then(|u| u.checked_duration_since(now))
            .map(|d| d.as_millis() as u64 + 35)
    };

    if let Some(ms) = wait_ms {
        let weak = Rc::downgrade(state_rc);
        let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            let Some(rc) = weak.upgrade() else { return };
            let still_visible = {
                let mut st = rc.borrow_mut();
                st.hud_timer = None;
                st.system_snapshot = st.system.snapshot();
                if st.hud_visible() {
                    true
                } else {
                    st.apply_pill_mode();
                    st.area.queue_draw();
                    false
                }
            };
            if still_visible {
                schedule_hud_hide(&rc);
            } else {
                ensure_anim_timer(&rc);
            }
        });
        state_rc.borrow_mut().hud_timer = Some(id);
    }
}

fn schedule_notif_hide(state_rc: &Rc<RefCell<AppState>>) {
    let wait_ms = {
        let mut st = state_rc.borrow_mut();
        if let Some(id) = st.notif_timer.take() {
            id.remove();
        }
        let now = Instant::now();
        st.notification_snapshot
            .toast_until
            .and_then(|u| u.checked_duration_since(now))
            .map(|d| d.as_millis() as u64 + 35)
    };

    if let Some(ms) = wait_ms {
        let weak = Rc::downgrade(state_rc);
        let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            let Some(rc) = weak.upgrade() else { return };
            let still_visible = {
                let mut st = rc.borrow_mut();
                st.notif_timer = None;
                st.notification_snapshot = st.notifications.snapshot();
                if st.toast_visible() {
                    true
                } else {
                    st.apply_pill_mode();
                    st.area.queue_draw();
                    false
                }
            };
            if still_visible {
                schedule_notif_hide(&rc);
            } else {
                ensure_anim_timer(&rc);
            }
        });
        state_rc.borrow_mut().notif_timer = Some(id);
    }
}

fn autostart_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/autostart/zekrima.desktop"))
}

fn is_autostart_enabled() -> bool {
    autostart_path().map(|p| p.exists()).unwrap_or(false)
}

fn toggle_autostart() {
    let Some(path) = autostart_path() else { return };
    if path.exists() {
        let _ = fs::remove_file(path);
    } else {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "zekrima".to_string());
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Zekrima\nComment=Native Dynamic Island & AI Personal Assistant\nExec={exe}\nTerminal=false\nCategories=Utility;\nX-GNOME-Autostart-enabled=true\n"
        );
        let _ = fs::write(path, entry);
    }
}

fn build_ui(app: &Application) {
    let already_built = APP_HANDLE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|w| w.upgrade())
            .is_some()
    });
    if already_built {
        return;
    }

    let provider = CssProvider::new();
    provider.load_from_data(
        "window.zekrima-window, window.zekrima-window > * { background-color: transparent; background-image: none; box-shadow: none; }",
    );
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Zekrima")
        .default_width(CANVAS_WIDTH_LOGICAL as i32)
        .default_height(CANVAS_HEIGHT_LOGICAL as i32)
        .decorated(false)
        .resizable(false)
        .build();
    window.add_css_class("zekrima-window");

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_namespace(Some("zekrima"));
        window.set_layer(Layer::Overlay);
        window.set_anchor(Edge::Top, true);
        window.set_exclusive_zone(0);
        window.set_keyboard_mode(KeyboardMode::None);
    }

    let area = DrawingArea::new();
    area.set_content_width(CANVAS_WIDTH_LOGICAL as i32);
    area.set_content_height(CANVAS_HEIGHT_LOGICAL as i32);
    area.set_focusable(true);
    window.set_child(Some(&area));

    let media_wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {
        glib::MainContext::default().invoke(|| {
            APP_HANDLE.with(|cell| {
                if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                    {
                        let mut st = rc.borrow_mut();
                        st.media_info = st.media.snapshot();
                        let active = st.media_info.has_session && st.media_info.is_playing;
                        st.audio_viz.set_active(active);
                        st.zeki.set_dancing(active);
                        st.apply_pill_mode();
                        st.area.queue_draw();
                    }
                    ensure_anim_timer(&rc);
                }
            });
        });
    });

    let system_wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {
        glib::MainContext::default().invoke(|| {
            APP_HANDLE.with(|cell| {
                if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                    {
                        let mut st = rc.borrow_mut();
                        st.system_snapshot = st.system.snapshot();
                        st.reveal_from_auto_hide();
                        st.apply_pill_mode();
                        st.area.queue_draw();
                    }
                    schedule_hud_hide(&rc);
                    ensure_anim_timer(&rc);
                }
            });
        });
    });

    let notif_wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {
        glib::MainContext::default().invoke(|| {
            APP_HANDLE.with(|cell| {
                if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                    {
                        let mut st = rc.borrow_mut();
                        st.notification_snapshot = st.notifications.snapshot();
                        st.zeki.on_notification_peek();
                        st.reveal_from_auto_hide();
                        st.apply_pill_mode();
                        st.area.queue_draw();
                    }
                    schedule_notif_hide(&rc);
                    ensure_anim_timer(&rc);
                }
            });
        });
    });

    let assistant_wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {
        glib::MainContext::default().invoke(|| {
            APP_HANDLE.with(|cell| {
                if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                    rc.borrow_mut().sync_assistant_to_zeki();
                    ensure_anim_timer(&rc);
                }
            });
        });
    });

    let cicada_wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {
        glib::MainContext::default().invoke(|| {
            APP_HANDLE.with(|cell| {
                if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                    rc.borrow_mut().sync_cicada_snapshot();
                    ensure_anim_timer(&rc);
                }
            });
        });
    });

    let cfg = AssistantConfig::load();
    let media = MediaService::start(media_wake);
    let media_info = media.snapshot();
    let audio_viz = AudioVisualizer::start();
    let playing_now = media_info.has_session && media_info.is_playing;
    audio_viz.set_active(playing_now);
    let system = SystemService::start(system_wake);
    let system_snapshot = system.snapshot();
    let notifications = NotificationService::start(notif_wake);
    let notification_snapshot = notifications.snapshot();
    let assistant = AssistantService::start(assistant_wake);
    let assistant_snapshot = assistant.snapshot();
    let cicada = CicadaService::start(cicada_wake);
    let cicada_snapshot = cicada.snapshot();
    let mut zeki = ZekiEngine::new();
    zeki.set_dancing(playing_now);
    let mut sysmon = SystemMonitor::new();
    sysmon.sample();

    let now_local = Local::now();
    let state = Rc::new(RefCell::new(AppState {
        app: app.clone(),
        window: window.clone(),
        area: area.clone(),
        renderer: CairoRenderer::new(),
        springs: PillSprings::new(),
        zeki,
        assistant,
        assistant_snapshot,
        cicada,
        cicada_snapshot,
        cicada_subtitle_bar: false,
        cicada_click_through: false,
        cicada_large: false,
        cicada_scroll_offset: 0.0,
        cicada_auto_scroll: true,
        cicada_copied_flash_until: None,
        last_cicada_arrival_seq: 0,
        assistant_pinned_open: false,
        chat_large: false,
        chat_input: String::new(),
        chat_focused: false,
        chat_scroll_offset: 0.0,
        chat_auto_scroll: true,
        chat_copied_flash_until: None,
        chat_attachment: None,
        media,
        media_info,
        audio_viz,
        system,
        system_snapshot,
        notifications,
        notification_snapshot,
        timer: PomodoroTimer::default(),
        clip_items: Vec::with_capacity(MAX_CLIP_ITEMS),
        clip_copied_flash_until: None,
        ignore_next_clip: false,
        sysmon,
        auto_hide_enabled: cfg.auto_hide,
        auto_hidden: false,
        manual_hidden: false,
        last_visible_activity: Instant::now(),
        ptt_hotkey: cfg.ptt_hotkey,
        hide_hotkey: cfg.hide_hotkey,
        last_clock_minute: (now_local.hour(), now_local.minute()),
        last_input_rect: None,
        expanded: false,
        expanded_tab: ExpandedTab::Zeki,
        hovered: false,
        hovered_target: None,
        anim_timer_id: None,
        anim_interval_ms: 0,
        last_anim_tick: Instant::now(),
        hud_timer: None,
        notif_timer: None,
    }));

    APP_HANDLE.with(|cell| {
        *cell.borrow_mut() = Some(Rc::downgrade(&state));
    });

    // Event-driven GTK4 Clipboard watcher (zero polling)
    if let Some(display) = gdk::Display::default() {
        let clipboard = display.clipboard();
        let weak = Rc::downgrade(&state);
        clipboard.connect_changed(move |cb| {
            let weak_inner = weak.clone();
            cb.read_text_async(None::<&gio::Cancellable>, move |res| {
                let Ok(Some(text)) = res else { return };
                let Some(rc) = weak_inner.upgrade() else {
                    return;
                };
                let mut st = rc.borrow_mut();
                if st.ignore_next_clip {
                    st.ignore_next_clip = false;
                    return;
                }
                if st.push_clip_text(&text)
                    && st.expanded
                    && st.expanded_tab == ExpandedTab::Clipboard
                    && st.anim_timer_id.is_none()
                {
                    st.area.queue_draw();
                }
            });
        });
    }

    // Draw function
    {
        let st_rc = Rc::clone(&state);
        area.set_draw_func(move |_area, cr, _w, _h| {
            let mut st = st_rc.borrow_mut();
            let (width, height, radius) = st.springs.geometry();
            let expanded = st.expanded;
            let expanded_tab = st.expanded_tab;
            let hovered_target = st.hovered_target;
            let eq_levels = st.audio_viz.levels();
            let media = st.media_info.clone();
            let system = st.system_snapshot.clone();
            let notification = st.notification_snapshot.clone();
            let timer = st.timer.clone();
            let clip_items = st.clip_items.clone();
            let clip_copied_flash = st.clip_flash_active();
            let sys_stats = st.sysmon.stats.clone();
            let assistant = st.assistant_snapshot.clone();
            let assistant_chat_open = st.assistant_chat_open();
            let chat_large = st.chat_large;
            let chat_input = st.chat_input.clone();
            let chat_focused = st.chat_focused;
            let chat_copied_flash = st.chat_flash_active();
            let chat_attached_label = st.chat_attachment.as_ref().map(|a| a.short_label());
            let ptt_hotkey_short = st.ptt_hotkey.short_label();
            let auto_hidden = st.auto_hidden;
            let cicada = st.cicada_snapshot.clone();
            let cicada_audio_level = st.cicada.live_audio_level();
            let cicada_subtitle_bar = st.cicada_subtitle_bar;
            let cicada_click_through = st.cicada_click_through;
            let cicada_copied_flash = st.cicada_flash_active();
            let cicada_scroll_in = if st.cicada_auto_scroll {
                f32::MAX
            } else {
                st.cicada_scroll_offset
            };

            let scroll_in = if st.chat_auto_scroll {
                f32::MAX
            } else {
                st.chat_scroll_offset
            };

            let app_ref = &mut *st;
            let frame = Frame {
                width,
                height,
                radius,
                expanded,
                expanded_tab,
                media: &media,
                hovered_target,
                eq_levels,
                system: &system,
                notification: &notification,
                timer: &timer,
                clip_items: &clip_items,
                clip_copied_flash,
                sys_stats: &sys_stats,
                zeki: &app_ref.zeki,
                assistant: &assistant,
                assistant_chat_open,
                chat_large,
                chat_input: &chat_input,
                chat_focused,
                chat_scroll_offset: scroll_in,
                chat_copied_flash,
                chat_attached_label: chat_attached_label.as_deref(),
                ptt_hotkey_short,
                auto_hidden,
                cicada: &cicada,
                cicada_audio_level,
                cicada_subtitle_bar,
                cicada_click_through,
                cicada_scroll_offset: cicada_scroll_in,
                cicada_copied_flash,
            };
            app_ref.renderer.draw(cr, &frame);
            let max_s = app_ref.renderer.last_chat_max_scroll();
            if app_ref.chat_auto_scroll {
                app_ref.chat_scroll_offset = max_s;
            } else {
                app_ref.chat_scroll_offset = app_ref.chat_scroll_offset.clamp(0.0, max_s);
            }
            let max_c = app_ref.renderer.last_cicada_max_scroll();
            if app_ref.cicada_auto_scroll {
                app_ref.cicada_scroll_offset = max_c;
            } else {
                app_ref.cicada_scroll_offset = app_ref.cicada_scroll_offset.clamp(0.0, max_c);
            }
        });
    }

    {
        let st_rc = Rc::clone(&state);
        window.connect_realize(move |_| {
            st_rc.borrow_mut().update_input_region();
        });
    }

    // Hover & motion tracking
    let motion = gtk4::EventControllerMotion::new();
    {
        let st_rc = Rc::clone(&state);
        motion.connect_enter(move |_, x, y| {
            {
                let mut st = st_rc.borrow_mut();
                st.reveal_from_auto_hide();
                if !st.hovered {
                    st.hovered = true;
                    if !st.toast_visible() && !st.cicada_subtitle_bar {
                        st.set_expanded(true);
                    }
                }
                update_hover_target(&mut st, x as f32, y as f32);
            }
            ensure_anim_timer(&st_rc);
        });
    }
    {
        let st_rc = Rc::clone(&state);
        motion.connect_motion(move |_, x, y| {
            let mut st = st_rc.borrow_mut();
            st.last_visible_activity = Instant::now();
            if update_hover_target(&mut st, x as f32, y as f32) && st.anim_timer_id.is_none() {
                st.area.queue_draw();
            }
        });
    }
    {
        let st_rc = Rc::clone(&state);
        motion.connect_leave(move |_| {
            {
                let mut st = st_rc.borrow_mut();
                st.hovered = false;
                st.hovered_target = None;
                st.last_visible_activity = Instant::now();
                // Keep open only while actively typing a draft or while an AI request is busy.
                // Otherwise always auto-minimize smoothly when cursor leaves the island!
                if !(st.chat_focused && !st.chat_input.is_empty())
                    && !st.assistant_snapshot.phase.is_busy()
                {
                    st.set_chat_focus(false);
                    st.assistant_pinned_open = false;
                    st.set_expanded(false);
                }
            }
            ensure_anim_timer(&st_rc);
        });
    }
    area.add_controller(motion);

    // Mouse wheel / touchpad scrolling inside the Chat Box
    let scroll_ctrl = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    {
        let st_rc = Rc::clone(&state);
        scroll_ctrl.connect_scroll(move |_, _dx, dy| {
            let mut st = st_rc.borrow_mut();
            if st.expanded && st.expanded_tab == ExpandedTab::Cicada {
                let max_c = st.renderer.last_cicada_max_scroll();
                let next = (st.cicada_scroll_offset + (dy as f32) * 32.0).clamp(0.0, max_c);
                st.cicada_scroll_offset = next;
                st.cicada_auto_scroll = (max_c - next) <= 2.0;
                st.area.queue_draw();
                return glib::Propagation::Stop;
            }
            if st.expanded && st.expanded_tab == ExpandedTab::Zeki && st.assistant_chat_open() {
                let max_s = st.renderer.last_chat_max_scroll();
                let next = (st.chat_scroll_offset + (dy as f32) * 32.0).clamp(0.0, max_s);
                st.chat_scroll_offset = next;
                st.chat_auto_scroll = (max_s - next) <= 2.0;
                st.area.queue_draw();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    area.add_controller(scroll_ctrl);

    // Keyboard input for Mode Chat (active only when chat input is focused)
    let key_ctrl = gtk4::EventControllerKey::new();
    {
        let st_rc = Rc::clone(&state);
        key_ctrl.connect_key_pressed(move |_, keyval, _keycode, mods| {
            let mut st = st_rc.borrow_mut();
            if !st.chat_focused {
                return glib::Propagation::Proceed;
            }
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);

            match keyval {
                gdk::Key::Escape => {
                    st.set_chat_focus(false);
                    if st.assistant_snapshot.phase.is_busy() {
                        st.assistant.cancel();
                    }
                    st.area.queue_draw();
                    return glib::Propagation::Stop;
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    st.submit_chat_input();
                    drop(st);
                    ensure_anim_timer(&st_rc);
                    return glib::Propagation::Stop;
                }
                gdk::Key::BackSpace => {
                    st.chat_input.pop();
                    st.area.queue_draw();
                    return glib::Propagation::Stop;
                }
                gdk::Key::Page_Up | gdk::Key::Up => {
                    let step = if keyval == gdk::Key::Page_Up { 120.0 } else { 28.0 };
                    st.chat_scroll_offset = (st.chat_scroll_offset - step).max(0.0);
                    st.chat_auto_scroll = false;
                    st.area.queue_draw();
                    return glib::Propagation::Stop;
                }
                gdk::Key::Page_Down | gdk::Key::Down => {
                    let step = if keyval == gdk::Key::Page_Down { 120.0 } else { 28.0 };
                    let max_s = st.renderer.last_chat_max_scroll();
                    st.chat_scroll_offset = (st.chat_scroll_offset + step).min(max_s);
                    st.chat_auto_scroll = (max_s - st.chat_scroll_offset) <= 2.0;
                    st.area.queue_draw();
                    return glib::Propagation::Stop;
                }
                gdk::Key::v | gdk::Key::V if ctrl => {
                    if let Some(display) = gdk::Display::default() {
                        let weak = Rc::downgrade(&st_rc);
                        display.clipboard().read_text_async(
                            None::<&gio::Cancellable>,
                            move |res| {
                                if let Ok(Some(txt)) = res {
                                    if let Some(rc) = weak.upgrade() {
                                        let mut s = rc.borrow_mut();
                                        let clean = txt.replace(['\r', '\n'], " ");
                                        let rem =
                                            MAX_CHAT_INPUT_CHARS.saturating_sub(s.chat_input.chars().count());
                                        s.chat_input.extend(clean.chars().take(rem));
                                        s.area.queue_draw();
                                    }
                                }
                            },
                        );
                    }
                    return glib::Propagation::Stop;
                }
                _ => {
                    if !ctrl {
                        if let Some(ch) = keyval.to_unicode() {
                            if !ch.is_control()
                                && st.chat_input.chars().count() < MAX_CHAT_INPUT_CHARS
                            {
                                st.chat_input.push(ch);
                                st.area.queue_draw();
                                return glib::Propagation::Stop;
                            }
                        }
                    }
                }
            }
            glib::Propagation::Proceed
        });
    }
    area.add_controller(key_ctrl);

    // Left-click & double-click handler
    let left_click = gtk4::GestureClick::builder().button(1).build();
    {
        let st_rc = Rc::clone(&state);
        left_click.connect_released(move |_, n_press, x, y| {
            let mut open_settings = false;
            let mut open_file_dialog = false;
            {
                let mut st = st_rc.borrow_mut();
                st.reveal_from_auto_hide();
                let (left, top, _, _) = st.pill_origin();
                let lx = x as f32 - left;
                let ly = y as f32 - top;
                let target = {
                    let ctx = st.hit_context();
                    widgets::hit_target_at(lx, ly, &ctx)
                };

                // Double-click on the chat box toggles Large (640x470) <-> Normal (520x310)
                if n_press == 2
                    && matches!(target, Some(HitTarget::ZekiChatCard))
                {
                    st.chat_large = !st.chat_large;
                    st.apply_pill_mode();
                    st.area.queue_draw();
                } else if n_press == 2
                    && matches!(target, Some(HitTarget::CicadaCard))
                {
                    st.cicada_large = !st.cicada_large;
                    st.apply_pill_mode();
                    st.area.queue_draw();
                } else if let Some(target) = target {
                    match target {
                        HitTarget::SelectTab(tab) => st.set_expanded_tab(tab),
                        HitTarget::ZekiPoke => st.zeki.poke(),
                        HitTarget::ZekiSelectState(s) => st.zeki.set_state_manual(s),
                        HitTarget::ZekiOpenChat => st.open_chat_panel(true),
                        HitTarget::ZekiMicToggle => {
                            st.toggle_voice_capture(CaptureMode::HandsFreeVad);
                        }
                        HitTarget::ZekiTogglePrivacy => {
                            let _ = st.assistant.toggle_screen_access();
                            st.assistant_snapshot = st.assistant.snapshot();
                        }
                        HitTarget::ZekiOpenConfig => {
                            st.set_chat_focus(false);
                            open_settings = true;
                        }
                        HitTarget::ZekiAttachFile => {
                            st.set_chat_focus(false);
                            open_file_dialog = true;
                        }
                        HitTarget::ZekiRemoveFile => {
                            st.chat_attachment = None;
                        }
                        HitTarget::ZekiChatToggleSize => {
                            st.chat_large = !st.chat_large;
                            st.apply_pill_mode();
                        }
                        HitTarget::ZekiChatInput => {
                            st.open_chat_panel(true);
                        }
                        HitTarget::ZekiChatSend => {
                            st.submit_chat_input();
                        }
                        HitTarget::ZekiChatCard => {
                            // Single click inside card keeps it open without stealing focus
                        }
                        HitTarget::ZekiCopyReply => {
                            let reply = st.assistant_snapshot.reply.clone();
                            if !reply.is_empty() {
                                if let Some(display) = gdk::Display::default() {
                                    st.ignore_next_clip = true;
                                    display.clipboard().set_text(&reply);
                                    st.chat_copied_flash_until =
                                        Some(Instant::now() + Duration::from_millis(1400));
                                }
                            }
                        }
                        HitTarget::ZekiInsertReply => {
                            let reply = st.assistant_snapshot.reply.clone();
                            if !reply.is_empty() {
                                if let Some(display) = gdk::Display::default() {
                                    st.ignore_next_clip = true;
                                    display.clipboard().set_text(&reply);
                                    st.chat_copied_flash_until =
                                        Some(Instant::now() + Duration::from_millis(1400));
                                }
                                st.set_chat_focus(false);
                                st.set_expanded(false);
                                tools::trigger_paste_shortcut();
                            }
                        }
                        HitTarget::ZekiClearChat => {
                            st.assistant.clear_conversation();
                            st.assistant_snapshot = st.assistant.snapshot();
                            st.assistant_pinned_open = false;
                            st.chat_large = false;
                            st.chat_input.clear();
                            st.chat_attachment = None;
                            st.chat_scroll_offset = 0.0;
                            st.chat_auto_scroll = true;
                            st.set_chat_focus(false);
                            st.apply_pill_mode();
                        }
                        HitTarget::CicadaToggleListen => {
                            let was_active = st.cicada_snapshot.active;
                            st.cicada.toggle_listening();
                            st.cicada_snapshot = st.cicada.snapshot();
                            if was_active && st.cicada_snapshot.has_entries() {
                                let _ = st.cicada.save_markdown_file();
                            }
                            st.apply_pill_mode();
                            st.update_input_region();
                        }
                        HitTarget::CicadaToggleMic => {
                            st.cicada.toggle_mic();
                            st.cicada_snapshot = st.cicada.snapshot();
                        }
                        HitTarget::CicadaQuickReply => {
                            st.cicada.toggle_quick_reply();
                            st.cicada_snapshot = st.cicada.snapshot();
                            st.apply_pill_mode();
                        }
                        HitTarget::CicadaToggleSubtitleBar => {
                            st.cicada_subtitle_bar = !st.cicada_subtitle_bar;
                            if st.cicada_subtitle_bar {
                                st.expanded = false;
                                st.hovered_target = None;
                            } else {
                                st.cicada_click_through = false;
                            }
                            st.apply_pill_mode();
                            st.update_input_region();
                        }
                        HitTarget::CicadaToggleClickThrough => {
                            st.cicada_click_through = !st.cicada_click_through;
                            st.update_input_region();
                        }
                        HitTarget::CicadaExpandFromBar => {
                            st.cicada_click_through = false;
                            st.cicada_subtitle_bar = false;
                            st.expanded_tab = ExpandedTab::Cicada;
                            st.set_expanded(true);
                        }
                        HitTarget::CicadaCycleLang => {
                            st.cicada.cycle_target_language();
                            st.cicada_snapshot = st.cicada.snapshot();
                        }
                        HitTarget::CicadaToggleOriginal => {
                            st.cicada.toggle_show_original();
                            st.cicada_snapshot = st.cicada.snapshot();
                        }
                        HitTarget::CicadaToggleSize => {
                            st.cicada_large = !st.cicada_large;
                            st.apply_pill_mode();
                        }
                        HitTarget::CicadaCopyTranscript => {
                            let md = st.cicada.format_markdown();
                            if !md.is_empty() {
                                if let Some(display) = gdk::Display::default() {
                                    st.ignore_next_clip = true;
                                    display.clipboard().set_text(&md);
                                }
                                let _ = st.cicada.save_markdown_file();
                                st.cicada_copied_flash_until =
                                    Some(Instant::now() + Duration::from_millis(1600));
                                st.zeki.trigger_action_happy();
                            }
                        }
                        HitTarget::CicadaCopyQuickReply => {
                            if let Some(qr) = st.cicada_snapshot.last_quick_reply.clone() {
                                if let Some(display) = gdk::Display::default() {
                                    st.ignore_next_clip = true;
                                    display.clipboard().set_text(&qr.translation);
                                }
                                st.cicada_copied_flash_until =
                                    Some(Instant::now() + Duration::from_millis(1400));
                                st.zeki.trigger_action_happy();
                            }
                        }
                        HitTarget::CicadaSummarize => {
                            if let Some(prompt) = st.cicada.build_summary_prompt() {
                                let _ = st.cicada.save_markdown_file();
                                st.cicada_subtitle_bar = false;
                                st.cicada_click_through = false;
                                st.open_chat_panel(false);
                                st.chat_auto_scroll = true;
                                let ctx = st.build_request_context();
                                st.assistant.send_text(prompt, ctx);
                            }
                        }
                        HitTarget::CicadaAskZeki => {
                            st.cicada_subtitle_bar = false;
                            st.cicada_click_through = false;
                            st.open_chat_panel(true);
                        }
                        HitTarget::CicadaClear => {
                            st.cicada.clear_transcript();
                            st.cicada_snapshot = st.cicada.snapshot();
                            st.cicada_scroll_offset = 0.0;
                            st.cicada_auto_scroll = true;
                        }
                        HitTarget::CicadaCard => {}
                        HitTarget::MediaControl(c) => st.media.control(c),
                        HitTarget::ToastDismiss(id) => st.notifications.dismiss(id),
                        HitTarget::ToastClear(_) => st.notifications.clear_toast(),
                        HitTarget::ClearAllNotifications => st.notifications.clear_all(),
                        HitTarget::NotifDismiss(id) => st.notifications.dismiss(id),
                        HitTarget::TimerToggle => st.timer.toggle_start_pause(),
                        HitTarget::TimerReset => st.timer.reset(),
                        HitTarget::TimerPreset(secs) => st.timer.select_preset(secs),
                        HitTarget::ClipCopy(idx) => {
                            if let Some(text) = st.clip_items.get(idx).cloned() {
                                if let Some(display) = gdk::Display::default() {
                                    st.ignore_next_clip = true;
                                    display.clipboard().set_text(&text);
                                    st.clip_items.remove(idx);
                                    st.clip_items.insert(0, text);
                                    st.clip_copied_flash_until =
                                        Some(Instant::now() + Duration::from_millis(1400));
                                }
                            }
                        }
                        HitTarget::ClipClear => {
                            st.clip_items.clear();
                            st.clip_copied_flash_until = None;
                        }
                    }
                } else if st.toast_visible() && !st.expanded {
                    st.notifications.clear_toast();
                } else {
                    let next = !st.expanded;
                    st.set_expanded(next);
                }
                st.area.queue_draw();
            }

            if open_settings {
                open_settings_for_state(&st_rc);
            }
            if open_file_dialog {
                let win_ref = st_rc.borrow().window.clone();
                let weak = Rc::downgrade(&st_rc);
                files::open_file_picker(Some(win_ref.upcast_ref::<gtk4::Window>()), move |res| {
                    if let Some(rc) = weak.upgrade() {
                        {
                            let mut st = rc.borrow_mut();
                            match res {
                                Ok(att) => {
                                    st.chat_attachment = Some(att);
                                    st.open_chat_panel(true);
                                }
                                Err(err) => {
                                    st.assistant_snapshot.error = Some(err);
                                }
                            }
                            st.area.queue_draw();
                        }
                        ensure_anim_timer(&rc);
                    }
                });
            }

            ensure_anim_timer(&st_rc);
        });
    }
    area.add_controller(left_click);

    // Right-click menu
    let action_group = gio::SimpleActionGroup::new();
    {
        let st_rc = Rc::clone(&state);
        let act = gio::SimpleAction::new("toggle-hide", None);
        act.connect_activate(move |_, _| {
            st_rc.borrow_mut().toggle_manual_hide();
            ensure_anim_timer(&st_rc);
        });
        action_group.add_action(&act);
    }
    {
        let st_rc = Rc::clone(&state);
        let act = gio::SimpleAction::new("toggle-auto-hide", None);
        act.connect_activate(move |_, _| {
            st_rc.borrow_mut().toggle_auto_hide();
            ensure_anim_timer(&st_rc);
        });
        action_group.add_action(&act);
    }
    {
        let st_rc = Rc::clone(&state);
        let act = gio::SimpleAction::new("open-settings", None);
        act.connect_activate(move |_, _| open_settings_for_state(&st_rc));
        action_group.add_action(&act);
    }
    {
        let act = gio::SimpleAction::new("toggle-autostart", None);
        act.connect_activate(|_, _| toggle_autostart());
        action_group.add_action(&act);
    }
    {
        let st_rc = Rc::clone(&state);
        let act = gio::SimpleAction::new("test-notif", None);
        act.connect_activate(move |_, _| {
            st_rc.borrow().notifications.push_manual(
                "Zekrima",
                "Zekrima Personal Assistant v0.3.0",
                "Notifications & Zeki Assistant are ready on Arch/CachyOS!",
            );
        });
        action_group.add_action(&act);
    }
    {
        let app_clone = app.clone();
        let act = gio::SimpleAction::new("quit", None);
        act.connect_activate(move |_, _| app_clone.quit());
        action_group.add_action(&act);
    }
    window.insert_action_group("island", Some(&action_group));

    let right_click = gtk4::GestureClick::builder().button(3).build();
    {
        let area_clone = area.clone();
        let st_rc = Rc::clone(&state);
        right_click.connect_released(move |_, _, x, y| {
            let (auto_hide, hide_lbl) = {
                let st = st_rc.borrow();
                (st.auto_hide_enabled, st.hide_hotkey.label().to_string())
            };
            let menu = gio::Menu::new();
            menu.append(
                Some(&format!("Hide Island ({hide_lbl})")),
                Some("island.toggle-hide"),
            );
            let auto_lbl = if auto_hide {
                "✓ Auto-Hide when Idle"
            } else {
                "Auto-Hide when Idle"
            };
            menu.append(Some(auto_lbl), Some("island.toggle-auto-hide"));
            menu.append(
                Some("Assistant Settings (API Keys & Hotkeys)"),
                Some("island.open-settings"),
            );
            let autostart_label = if is_autostart_enabled() {
                "✓ Run at Login"
            } else {
                "Run at Login"
            };
            menu.append(Some(autostart_label), Some("island.toggle-autostart"));
            menu.append(Some("Test Notification"), Some("island.test-notif"));
            menu.append(Some("Quit Zekrima"), Some("island.quit"));

            let popover = PopoverMenu::from_model(Some(&menu));
            popover.set_parent(&area_clone);
            popover.set_has_arrow(false);
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.connect_closed(|p| p.unparent());
            popover.popup();
        });
    }
    area.add_controller(right_click);

    // 1-second housekeeping: minute rollover, Pomodoro tick, on-demand SysMon,
    // Zeki 15-min auto-sleep, and Auto-Hide idle timeout
    {
        let weak = Rc::downgrade(&state);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(rc) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            let mut recheck_anim = false;
            {
                let mut st = rc.borrow_mut();
                let mut needs_draw = false;

                let now = Local::now();
                let cur = (now.hour(), now.minute());
                if cur != st.last_clock_minute {
                    st.last_clock_minute = cur;
                    needs_draw = true;
                }

                if st.zeki.on_one_second_tick() {
                    needs_draw = true;
                    recheck_anim = true;
                }

                if st.timer.running {
                    let finished = st.timer.tick();
                    needs_draw = true;
                    if finished {
                        st.zeki.trigger_action_happy();
                        st.apply_pill_mode();
                        st.notifications.push_manual(
                            "Pomodoro",
                            "Focus Session Complete!",
                            "Time's up — take a short break.",
                        );
                    }
                }

                if st.expanded && st.expanded_tab == ExpandedTab::System {
                    st.sysmon.sample();
                    needs_draw = true;
                }

                if st.expanded
                    && st.expanded_tab == ExpandedTab::Clipboard
                    && st.clip_copied_flash_until.is_some()
                    && !st.clip_flash_active()
                {
                    st.clip_copied_flash_until = None;
                    needs_draw = true;
                }

                if st.chat_copied_flash_until.is_some() && !st.chat_flash_active() {
                    st.chat_copied_flash_until = None;
                    needs_draw = true;
                }
                if st.cicada_copied_flash_until.is_some() && !st.cicada_flash_active() {
                    st.cicada_copied_flash_until = None;
                    needs_draw = true;
                }

                if !st.auto_hidden && st.auto_hide_enabled && !st.manual_hidden {
                    if st.can_auto_hide_now() {
                        if st.last_visible_activity.elapsed().as_secs() >= AUTO_HIDE_IDLE_SECS {
                            st.auto_hidden = true;
                            st.hovered_target = None;
                            st.apply_pill_mode();
                            st.update_input_region();
                            needs_draw = true;
                            recheck_anim = true;
                        }
                    } else {
                        st.last_visible_activity = Instant::now();
                    }
                }

                if needs_draw && st.anim_timer_id.is_none() && !st.auto_hidden && !st.manual_hidden {
                    st.area.queue_draw();
                }
            }
            if recheck_anim {
                ensure_anim_timer(&rc);
            }
            ControlFlow::Continue
        });
    }

    ensure_anim_timer(&state);
    window.present();
}

fn open_settings_for_state(state_rc: &Rc<RefCell<AppState>>) {
    let app = state_rc.borrow().app.clone();
    let weak = Rc::downgrade(state_rc);
    settings::open_settings_window(&app, move || {
        if let Some(rc) = weak.upgrade() {
            {
                let mut st = rc.borrow_mut();
                let cfg = AssistantConfig::load();
                st.auto_hide_enabled = cfg.auto_hide;
                st.ptt_hotkey = cfg.ptt_hotkey;
                st.hide_hotkey = cfg.hide_hotkey;
                if !st.auto_hide_enabled {
                    st.reveal_from_auto_hide();
                } else {
                    st.last_visible_activity = Instant::now();
                }
                st.assistant.refresh_config_status();
                st.assistant_snapshot = st.assistant.snapshot();
                st.cicada.refresh_from_config();
                st.cicada_snapshot = st.cicada.snapshot();
                st.zeki.trigger_action_happy();
                st.area.queue_draw();
            }
            ensure_anim_timer(&rc);
        }
    });
}

fn update_hover_target(st: &mut AppState, canvas_x: f32, canvas_y: f32) -> bool {
    let (left, top, _, _) = st.pill_origin();
    let lx = canvas_x - left;
    let ly = canvas_y - top;
    let next = {
        let ctx = st.hit_context();
        widgets::hit_target_at(lx, ly, &ctx)
    };
    if next != st.hovered_target {
        st.hovered_target = next;
        true
    } else {
        false
    }
}

thread_local! {
    static APP_HANDLE: RefCell<Option<std::rc::Weak<RefCell<AppState>>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    let app = Application::builder()
        .application_id("io.zekrima.DynamicIsland")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    app.connect_command_line(|app, cmdline| {
        build_ui(app);

        let args = cmdline.arguments();
        for arg in args.iter().skip(1) {
            match arg.to_string_lossy().as_ref() {
                "--toggle-hide" => {
                    APP_HANDLE.with(|cell| {
                        if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                            rc.borrow_mut().toggle_manual_hide();
                            ensure_anim_timer(&rc);
                        }
                    });
                }
                "--voice" | "--ptt" => {
                    APP_HANDLE.with(|cell| {
                        if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                            rc.borrow_mut()
                                .toggle_voice_capture(CaptureMode::HandsFreeVad);
                            ensure_anim_timer(&rc);
                        }
                    });
                }
                "--chat" => {
                    APP_HANDLE.with(|cell| {
                        if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                            rc.borrow_mut().open_chat_panel(true);
                            ensure_anim_timer(&rc);
                        }
                    });
                }
                "--settings" | "--config" => {
                    APP_HANDLE.with(|cell| {
                        if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                            open_settings_for_state(&rc);
                        }
                    });
                }
                "--cicada" | "--live" => {
                    APP_HANDLE.with(|cell| {
                        if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                            {
                                let mut st = rc.borrow_mut();
                                st.reveal_from_auto_hide();
                                st.cicada.toggle_listening();
                                st.cicada_snapshot = st.cicada.snapshot();
                                if st.cicada_snapshot.active {
                                    st.cicada_subtitle_bar = true;
                                    st.expanded = false;
                                }
                                st.apply_pill_mode();
                                st.update_input_region();
                                st.area.queue_draw();
                            }
                            ensure_anim_timer(&rc);
                        }
                    });
                }
                "--reply" => {
                    APP_HANDLE.with(|cell| {
                        if let Some(rc) = cell.borrow().as_ref().and_then(|w| w.upgrade()) {
                            {
                                let mut st = rc.borrow_mut();
                                st.reveal_from_auto_hide();
                                st.cicada.toggle_quick_reply();
                                st.cicada_snapshot = st.cicada.snapshot();
                                st.apply_pill_mode();
                                st.area.queue_draw();
                            }
                            ensure_anim_timer(&rc);
                        }
                    });
                }
                _ => {}
            }
        }

        glib::ExitCode::SUCCESS
    });

    app.run()
}
