//! Native Cairo + PangoCairo renderer for Zekrima Personal Assistant v0.3.0 on Arch Linux / CachyOS.
//!
//! Full parity with Windows v0.3.0:
//! - Collapsed pill with animated mini Zeki, bold clock, PipeWire FFT music equalizer,
//!   Pomodoro countdown, active Assistant state pill, and Pillar notification badge
//! - Unified 6-tab Expanded Island (`Zeki`, `Media`, `Timer`, `Clip`, `Sys`, `Notifs`)
//! - Persistent Zeki Companion Dock across all sub-menus (`Chat`, `Voice`, `Screen/Incog`)
//! - Media logo positioned top-left above the bold clock
//! - Native scrollable & double-click-expandable multi-turn AI Chat box (`520x310` <-> `640x470`)
//!   with file attachment chip, live `● MIC` / `● SCREEN` indicators, and `Insert`/`Copy`/`Clear`
//! - Incoming Pillar notification toast & Volume/Brightness HUD

use std::f64::consts::{FRAC_PI_2, PI};

use chrono::Local;
use gtk4::cairo;
use gtk4::gdk_pixbuf::{self, Pixbuf};
use gtk4::pango;
use gtk4::prelude::*;

use crate::layout::{CANVAS_HEIGHT_LOGICAL, CANVAS_WIDTH_LOGICAL, PILL_TOP_LOGICAL};
use crate::render::spring::{AUTO_HIDDEN_HEIGHT, COLLAPSED_HEIGHT};
use crate::render::widgets::{
    app_accent_rgb, app_initial, chat_layout, cicada_bar_layout, cicada_panel_layout,
    clip_row_rect, control_rect, expansion_progress, format_relative_time,
    notif_card_dismiss_rect, notif_card_rect, smoothstep, tab_rect_for_width, timer_preset_rect,
    zeki_state_btn_rect, ExpandedTab, HitTarget, RectF, ALL_TABS, ARTIST_BOX, CLIP_CLEAR_BTN,
    CLIP_MAX_VISIBLE, COLLAPSED_ART, CONTROLS_TOP, EQUALIZER_BAR_COUNT, EQUALIZER_BAR_GAP,
    EQUALIZER_BAR_WIDTH, EQUALIZER_RIGHT, EXPANDED_ART, MEDIA_CLOCK_BOX, NOTIF_CARD_MAX_VISIBLE,
    NOTIF_CLEAR_BTN, TIMER_PROGRESS_BAR, TIMER_RESET_BTN, TIMER_TOGGLE_BTN, TITLE_BOX,
    TOAST_AVATAR, TOAST_DISMISS_BTN, TOAST_PROGRESS_RECT, ZEKI_CFG_BTN_CHAT, ZEKI_CFG_BTN_PREVIEW,
    ZEKI_CHAT_BTN_PREVIEW, ZEKI_DOCK_CHAT_BTN, ZEKI_DOCK_MIC_BTN, ZEKI_DOCK_PRIVACY_BTN,
    ZEKI_DOCK_RECT, ZEKI_MIC_BTN_CHAT, ZEKI_MIC_BTN_PREVIEW, ZEKI_PRIVACY_BTN_CHAT,
    ZEKI_PRIVACY_BTN_PREVIEW, ZEKI_STAGE_RECT,
};
use crate::render::zeki::{ZekiEngine, ALL_ZEKI_STATES};
use crate::services::assistant::{AssistantPhase, AssistantSnapshot};
use crate::services::cicada::{CicadaSnapshot, SpeakerChannel};
use crate::services::config::cicada_lang_short_badge;
use crate::services::media::{MediaControl, MediaInfo};
use crate::services::notifications::{NotificationItem, NotificationSnapshot};
use crate::services::pomodoro::{PomodoroTimer, TIMER_PRESETS_SECS};
use crate::services::sysmon::SystemStats;
use crate::services::system::{HudKind, HudState, SystemSnapshot};

const PAUSED_EQUALIZER: [f32; EQUALIZER_BAR_COUNT] = [0.22, 0.45, 0.30, 0.50, 0.25];
const ACCENT_RED: (f64, f64, f64) = (235.0 / 255.0, 0.0, 40.0 / 255.0);
const BADGE_RED: (f64, f64, f64) = (239.0 / 255.0, 68.0 / 255.0, 68.0 / 255.0);

pub struct Frame<'a> {
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub expanded: bool,
    pub expanded_tab: ExpandedTab,
    pub media: &'a MediaInfo,
    pub hovered_target: Option<HitTarget>,
    pub eq_levels: [f32; EQUALIZER_BAR_COUNT],
    pub system: &'a SystemSnapshot,
    pub notification: &'a NotificationSnapshot,
    pub timer: &'a PomodoroTimer,
    pub clip_items: &'a [String],
    pub clip_copied_flash: bool,
    pub sys_stats: &'a SystemStats,
    pub zeki: &'a ZekiEngine,
    pub assistant: &'a AssistantSnapshot,
    pub assistant_chat_open: bool,
    pub chat_large: bool,
    pub chat_input: &'a str,
    pub chat_focused: bool,
    pub chat_scroll_offset: f32,
    pub chat_copied_flash: bool,
    pub chat_attached_label: Option<&'a str>,
    pub ptt_hotkey_short: &'a str,
    pub auto_hidden: bool,
    pub cicada: &'a CicadaSnapshot,
    pub cicada_audio_level: f32,
    pub cicada_subtitle_bar: bool,
    pub cicada_click_through: bool,
    pub cicada_scroll_offset: f32,
    pub cicada_copied_flash: bool,
}

pub struct CairoRenderer {
    art_revision: u64,
    art_pixbuf: Option<Pixbuf>,
    last_chat_max_scroll: f32,
    last_cicada_max_scroll: f32,
}

impl CairoRenderer {
    pub fn new() -> Self {
        Self {
            art_revision: 0,
            art_pixbuf: None,
            last_chat_max_scroll: 0.0,
            last_cicada_max_scroll: 0.0,
        }
    }

    pub fn last_chat_max_scroll(&self) -> f32 {
        self.last_chat_max_scroll
    }

    pub fn last_cicada_max_scroll(&self) -> f32 {
        self.last_cicada_max_scroll
    }

    pub fn prepare_art(&mut self, media: &MediaInfo) {
        if media.art_revision == self.art_revision {
            return;
        }
        self.art_revision = media.art_revision;
        self.art_pixbuf = None;

        if let Some(bytes) = &media.art_bytes {
            let loader = gdk_pixbuf::PixbufLoader::new();
            if loader.write(bytes).is_ok() && loader.close().is_ok() {
                self.art_pixbuf = loader.pixbuf();
            }
        }
    }

    pub fn draw(&mut self, cr: &cairo::Context, frame: &Frame) {
        self.prepare_art(frame.media);

        cr.set_operator(cairo::Operator::Clear);
        let _ = cr.paint();
        cr.set_operator(cairo::Operator::Over);

        let width = frame.width.clamp(1.0, CANVAS_WIDTH_LOGICAL) as f64;
        let height = frame.height.clamp(1.0, CANVAS_HEIGHT_LOGICAL) as f64;
        let radius = (frame.radius as f64).clamp(0.0, width.min(height) / 2.0);
        let left = (CANVAS_WIDTH_LOGICAL as f64 - width) / 2.0;
        let tuck = ((COLLAPSED_HEIGHT - frame.height)
            / (COLLAPSED_HEIGHT - AUTO_HIDDEN_HEIGHT))
            .clamp(0.0, 1.0) as f64;
        let top = (PILL_TOP_LOGICAL as f64) * (1.0 - tuck);

        if height < 20.0 || frame.auto_hidden && height < 22.0 {
            let handle_h = height.max(4.5);
            let handle_r = (handle_h / 2.0).min(radius.max(2.25));
            let handle_top = (top - handle_r * 0.35).max(0.0);
            rounded_rect_path(cr, left, handle_top, width, handle_h, handle_r);
            cr.set_source_rgba(12.0 / 255.0, 12.0 / 255.0, 16.0 / 255.0, 0.86);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.16);
            cr.set_line_width(0.8);
            let _ = cr.stroke();

            let grip_w = (width * 0.34).clamp(28.0, 48.0);
            let grip_left = left + (width - grip_w) / 2.0;
            let grip_y = top + (handle_h - 1.5) * 0.55;
            rounded_rect_path(cr, grip_left, grip_y, grip_w, 1.5, 0.75);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.40);
            let _ = cr.fill();
            return;
        }

        rounded_rect_path(cr, left, top, width, height, radius);
        if frame.cicada_subtitle_bar && !frame.expanded {
            cr.set_source_rgba(
                10.0 / 255.0,
                12.0 / 255.0,
                16.0 / 255.0,
                if frame.cicada_click_through { 0.80 } else { 0.88 },
            );
        } else {
            cr.set_source_rgba(10.0 / 255.0, 10.0 / 255.0, 13.0 / 255.0, 0.96);
        }
        let _ = cr.fill();

        let now = std::time::Instant::now();
        let hud_visible = frame.system.hud.is_visible(now);
        let toast_visible = frame.notification.is_toast_visible(now);
        let (zr, zg, zb) = frame.zeki.accent_color();

        let inset = radius.max(14.0);
        if width > inset * 2.0 + 4.0 {
            rounded_rect_path(cr, left + inset, top + 1.0, width - inset * 2.0, 1.5, 0.75);
            if toast_visible && !hud_visible && !frame.expanded {
                if let Some(toast) = &frame.notification.toast {
                    let (r, g, b) = app_accent_rgb(&toast.app);
                    cr.set_source_rgba(r, g, b, 0.65);
                    let _ = cr.fill();
                }
            } else if frame.cicada.active && (!frame.expanded || frame.expanded_tab == ExpandedTab::Cicada) {
                cr.set_source_rgba(0.16, 0.86, 0.62, 0.85);
                let _ = cr.fill();
            } else if frame.expanded && frame.expanded_tab == ExpandedTab::Zeki
                || frame.zeki.state().is_assistant_active()
            {
                cr.set_source_rgba(zr, zg, zb, 0.68);
                let _ = cr.fill();
            } else {
                cr.set_source_rgba(ACCENT_RED.0, ACCENT_RED.1, ACCENT_RED.2, 0.42);
                let _ = cr.fill();
            }
        }

        let _ = cr.save();
        rounded_rect_path(
            cr,
            left + 1.0,
            top + 1.0,
            (width - 2.0).max(1.0),
            (height - 2.0).max(1.0),
            (radius - 1.0).max(0.0),
        );
        cr.clip();

        let expanded_progress = expansion_progress(frame.height);

        if hud_visible {
            self.draw_hud(cr, left, top, width, height, &frame.system.hud);
        } else if toast_visible && !frame.expanded {
            if let Some(toast) = &frame.notification.toast {
                let progress = frame.notification.toast_progress(now);
                self.draw_notification_toast(cr, left, top, toast, progress, frame.hovered_target);
            }
        } else if frame.cicada_subtitle_bar && !frame.expanded {
            self.draw_cicada_subtitle_bar(cr, left, top, width as f32, height as f32, frame);
        } else if frame.expanded || expanded_progress >= 0.28 {
            let alpha = smoothstep(((expanded_progress - 0.15) / 0.85).clamp(0.0, 1.0)) as f64;
            if alpha > 0.01 {
                draw_tab_bar(cr, left, top, width as f32, frame, alpha);
                match frame.expanded_tab {
                    ExpandedTab::Zeki => {
                        self.draw_zeki_panel(cr, left, top, width as f32, height as f32, frame, alpha);
                    }
                    ExpandedTab::Cicada => {
                        self.draw_cicada_panel(cr, left, top, width as f32, height as f32, frame, alpha);
                    }
                    ExpandedTab::Media => {
                        if frame.media.has_session {
                            self.draw_expanded_media(cr, left, top, frame, alpha);
                        } else {
                            self.draw_idle_expanded(
                                cr,
                                left,
                                top,
                                width,
                                frame.system,
                                frame.notification,
                                alpha,
                            );
                        }
                    }
                    ExpandedTab::Timer => {
                        draw_timer_panel(cr, left, top, frame.timer, frame.hovered_target, alpha);
                    }
                    ExpandedTab::Clipboard => {
                        draw_clipboard_panel(
                            cr,
                            left,
                            top,
                            width,
                            frame.clip_items,
                            frame.clip_copied_flash,
                            frame.hovered_target,
                            alpha,
                        );
                    }
                    ExpandedTab::System => {
                        draw_sysmon_panel(cr, left, top, width, frame.sys_stats, alpha);
                    }
                    ExpandedTab::Notifications => {
                        self.draw_notifications_panel(
                            cr,
                            left,
                            top,
                            width,
                            frame.notification,
                            frame.hovered_target,
                            alpha,
                        );
                    }
                }
                if !matches!(frame.expanded_tab, ExpandedTab::Zeki | ExpandedTab::Cicada) {
                    draw_zeki_companion_dock(cr, left, top, frame, alpha);
                }
            }
        } else {
            let collapsed_alpha = (1.0 - expanded_progress * 3.5).clamp(0.0, 1.0) as f64;
            if frame.cicada.active || frame.cicada.quick_reply_recording {
                self.draw_collapsed_cicada(cr, left, top, width, height, frame, collapsed_alpha);
            } else if frame.zeki.state().is_assistant_active() {
                self.draw_collapsed_assistant(cr, left, top, width, height, frame, collapsed_alpha);
            } else if frame.media.has_session {
                self.draw_collapsed_media(cr, left, top, width, height, frame, collapsed_alpha);
            } else {
                self.draw_collapsed_idle(cr, left, top, width, height, frame, collapsed_alpha);
            }
        }

        let _ = cr.restore();

        rounded_rect_path(cr, left + 0.5, top + 0.5, width - 1.0, height - 1.0, radius);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }

    fn draw_collapsed_cicada(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f64,
        pill_h: f64,
        frame: &Frame,
        alpha: f64,
    ) {
        frame.zeki.draw(cr, ox + 22.0, oy + pill_h / 2.0 + 1.0, 11.0, alpha, true);

        let blink = ((std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
            / 500)
            % 2)
            == 0;
        let (dr, dg, db) = if frame.cicada.quick_reply_recording {
            (0.98, 0.72, 0.22)
        } else {
            (0.16, 0.88, 0.62)
        };
        cr.arc(ox + 42.0, oy + pill_h / 2.0, 3.6, 0.0, 2.0 * PI);
        cr.set_source_rgba(dr, dg, db, if blink { 0.98 } else { 0.40 } * alpha);
        let _ = cr.fill();

        let tag = if frame.cicada.quick_reply_recording {
            format!("Reply→{}", cicada_lang_short_badge(&frame.cicada.reply_lang))
        } else if frame.cicada.pending_segments > 0 {
            format!("{} …", frame.cicada.lang_pair_badge())
        } else {
            frame.cicada.lang_pair_badge()
        };
        draw_text_centered_v(
            cr,
            &tag,
            ox + 52.0,
            oy,
            96.0,
            pill_h,
            12.0,
            true,
            pango::Alignment::Left,
            (0.72, 0.98, 0.88, 0.96 * alpha),
        );

        let level = frame.cicada_audio_level.clamp(0.08, 1.0) as f64;
        let bar_w = 3.0;
        let gap = 3.0;
        let right_edge = ox + pill_w - 16.0;
        let cy = oy + pill_h / 2.0;
        let weights = [0.50f64, 0.82, 1.0, 0.78, 0.46];
        cr.set_source_rgba(dr, dg, db, 0.92 * alpha);
        for (i, &w_mul) in weights.iter().enumerate() {
            let bh = (4.0 + 14.0 * level * w_mul).clamp(4.0, 18.0);
            let bx = right_edge - (4 - i) as f64 * (bar_w + gap) - bar_w;
            rounded_rect_path(cr, bx, cy - bh / 2.0, bar_w, bh, bar_w / 2.0);
            let _ = cr.fill();
        }
    }

    fn draw_cicada_subtitle_bar(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f32,
        pill_h: f32,
        frame: &Frame,
    ) {
        let bl = cicada_bar_layout(pill_w, pill_h);
        let hovered = frame.hovered_target;

        frame.zeki.draw(
            cr,
            ox + bl.zeki_center.0 as f64,
            oy + bl.zeki_center.1 as f64,
            19.0,
            1.0,
            false,
        );

        let status_txt = if frame.cicada.quick_reply_recording {
            format!(
                "● Quick-Reply (→{}) — Bicara sekarang...",
                cicada_lang_short_badge(&frame.cicada.reply_lang)
            )
        } else if frame.cicada.quick_reply_busy {
            "● Menerjemahkan Quick-Reply...".to_string()
        } else if let Some(err) = &frame.cicada.error {
            format!("! {err}")
        } else {
            let mic_tag = if frame.cicada.mic_enabled { " + Mic" } else { "" };
            let pend = if frame.cicada.pending_segments > 0 { " · …" } else { "" };
            format!("● LIVE {} {mic_tag}{pend}", frame.cicada.lang_pair_badge())
        };
        let st_col = if frame.cicada.error.is_some() {
            (0.98, 0.44, 0.52, 0.95)
        } else if frame.cicada.quick_reply_recording {
            (0.98, 0.76, 0.25, 0.95)
        } else {
            (0.24, 0.90, 0.68, 0.95)
        };
        draw_text(
            cr,
            &status_txt,
            ox + 66.0,
            oy + 11.0,
            (bl.reply_btn.left - 70.0).max(80.0) as f64,
            10.5,
            true,
            pango::Alignment::Left,
            st_col,
        );

        draw_tinted_pill(
            cr,
            ox,
            oy,
            bl.reply_btn,
            if frame.cicada.quick_reply_recording { "● Stop" } else { "Reply" },
            (0.98, 0.72, 0.22),
            hovered == Some(HitTarget::CicadaQuickReply),
            frame.cicada.quick_reply_recording,
            1.0,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            bl.pass_btn,
            if frame.cicada_click_through { "Unlock" } else { "Pass" },
            (0.38, 0.76, 0.98),
            hovered == Some(HitTarget::CicadaToggleClickThrough),
            frame.cicada_click_through,
            1.0,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            bl.expand_btn,
            "Full",
            (0.68, 0.74, 0.85),
            hovered == Some(HitTarget::CicadaExpandFromBar),
            false,
            1.0,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            bl.stop_btn,
            if frame.cicada.active { "Stop" } else { "Start" },
            (0.94, 0.30, 0.38),
            hovered == Some(HitTarget::CicadaToggleListen),
            frame.cicada.active,
            1.0,
        );

        let tx = ox + bl.text_rect.left as f64;
        let ty = oy + bl.text_rect.top as f64;
        let tw = bl.text_rect.width().max(120.0) as f64;

        if let Some(latest) = frame.cicada.entries.last() {
            let mut y_cur = ty;
            if frame.cicada.show_original && !latest.original.eq_ignore_ascii_case(&latest.translation) {
                let orig_line = format!("[{}] {}", latest.speaker.tag(), latest.original);
                draw_text(
                    cr,
                    &orig_line,
                    tx,
                    y_cur,
                    tw,
                    10.5,
                    false,
                    pango::Alignment::Left,
                    (0.76, 0.82, 0.90, 0.72),
                );
                y_cur += 16.0;
            }
            let trans = if latest.speaker == SpeakerChannel::QuickReply {
                format!("💬 {}", latest.translation)
            } else {
                latest.translation.clone()
            };
            let tr_col = match latest.speaker {
                SpeakerChannel::Online => (1.0, 1.0, 1.0, 0.99),
                SpeakerChannel::You => (0.62, 0.98, 0.80, 0.99),
                SpeakerChannel::QuickReply => (1.0, 0.88, 0.55, 0.99),
            };
            let layout = pangocairo::functions::create_layout(cr);
            let mut desc = pango::FontDescription::from_string("Sans Bold");
            desc.set_absolute_size(12.2 * pango::SCALE as f64);
            layout.set_font_description(Some(&desc));
            layout.set_width((tw * pango::SCALE as f64) as i32);
            layout.set_wrap(pango::WrapMode::WordChar);
            layout.set_text(&trans);
            cr.set_source_rgba(tr_col.0, tr_col.1, tr_col.2, tr_col.3);
            cr.move_to(tx, y_cur);
            pangocairo::functions::show_layout(cr, &layout);
        } else {
            let waiting = if frame.cicada.active {
                "Mendengarkan audio sistem PipeWire (@DEFAULT_MONITOR@)... subtitle akan muncul otomatis."
            } else {
                "Cicada Subtitle Bar siap. Klik Start untuk mulai menerjemahkan audio rapat."
            };
            draw_text(
                cr,
                waiting,
                tx,
                ty + 10.0,
                tw,
                11.0,
                false,
                pango::Alignment::Left,
                (0.78, 0.84, 0.92, 0.72),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_cicada_panel(
        &mut self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f32,
        pill_h: f32,
        frame: &Frame,
        alpha: f64,
    ) {
        let cl = cicada_panel_layout(pill_w, pill_h);
        let hovered = frame.hovered_target;

        // Left Zeki Taking-Notes Stage
        let (er, eg, eb) = if frame.cicada.active {
            (0.16, 0.86, 0.62)
        } else {
            (0.24, 0.74, 0.96)
        };
        rounded_rect_path(
            cr,
            ox + cl.zeki_stage.left as f64,
            oy + cl.zeki_stage.top as f64,
            cl.zeki_stage.width() as f64,
            cl.zeki_stage.height() as f64,
            16.0,
        );
        cr.set_source_rgba(er, eg, eb, 0.10 * alpha);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(er, eg, eb, 0.36 * alpha);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        let (scx, scy) = cl.zeki_stage.center();
        frame.zeki.draw(
            cr,
            ox + scx as f64 - 4.0,
            oy + scy as f64 + 2.0,
            23.0,
            alpha,
            false,
        );

        if frame.cicada.active {
            draw_text(
                cr,
                "● LIVE",
                ox + cl.zeki_stage.left as f64 + 6.0,
                oy + cl.zeki_stage.top as f64 + 5.0,
                54.0,
                9.5,
                true,
                pango::Alignment::Left,
                (0.18, 0.92, 0.65, 0.95 * alpha),
            );
        }

        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.listen_btn,
            if frame.cicada.active { "■ Stop Live" } else { "▶ Start Live" },
            if frame.cicada.active { (0.94, 0.30, 0.38) } else { (0.16, 0.86, 0.62) },
            hovered == Some(HitTarget::CicadaToggleListen),
            frame.cicada.active,
            alpha,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.mic_btn,
            if frame.cicada.mic_enabled { "Mic: ON" } else { "Mic: OFF" },
            (0.22, 0.82, 0.94),
            hovered == Some(HitTarget::CicadaToggleMic),
            frame.cicada.mic_enabled,
            alpha,
        );
        let reply_lbl = if frame.cicada.quick_reply_recording {
            "● Stop Reply".to_string()
        } else if frame.cicada.quick_reply_busy {
            "Translating...".to_string()
        } else {
            format!("Reply →{}", cicada_lang_short_badge(&frame.cicada.reply_lang))
        };
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.reply_btn,
            &reply_lbl,
            (0.98, 0.72, 0.22),
            hovered == Some(HitTarget::CicadaQuickReply),
            frame.cicada.quick_reply_recording,
            alpha,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.sub_bar_btn,
            "Subtitle Bar",
            (0.65, 0.56, 0.98),
            hovered == Some(HitTarget::CicadaToggleSubtitleBar),
            frame.cicada_subtitle_bar,
            alpha,
        );

        let hdr = if frame.cicada.glossary_count > 0 {
            format!(
                "Cicada Live · {} · {} terms",
                frame.cicada.lang_pair_badge(),
                frame.cicada.glossary_count
            )
        } else {
            format!("Cicada Live · {}", frame.cicada.lang_pair_badge())
        };
        draw_text(
            cr,
            &hdr,
            ox + cl.card.left as f64 + 2.0,
            oy + 47.0,
            (cl.lang_btn.left - cl.card.left - 6.0).max(80.0) as f64,
            12.0,
            true,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, 0.95 * alpha),
        );

        let lang_lbl = format!("→ {}", cicada_lang_short_badge(&frame.cicada.target_lang));
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.lang_btn,
            &lang_lbl,
            (0.16, 0.86, 0.62),
            hovered == Some(HitTarget::CicadaCycleLang),
            true,
            alpha,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.orig_btn,
            if frame.cicada.show_original { "Orig ON" } else { "Orig OFF" },
            (0.38, 0.74, 0.98),
            hovered == Some(HitTarget::CicadaToggleOriginal),
            frame.cicada.show_original,
            alpha,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.size_btn,
            if pill_w > 600.0 { "Small" } else { "Wide" },
            (0.68, 0.74, 0.85),
            hovered == Some(HitTarget::CicadaToggleSize),
            false,
            alpha,
        );

        // Scrollable Transcript Card
        let cx = ox + cl.card.left as f64;
        let cy = oy + cl.card.top as f64;
        let cw = cl.card.width() as f64;
        let ch = cl.card.height() as f64;

        rounded_rect_path(cr, cx, cy, cw, ch, 14.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.045 * alpha);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.11 * alpha);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        let _ = cr.save();
        cr.rectangle(cx + 6.0, cy + 6.0, (cw - 12.0).max(10.0), (ch - 12.0).max(10.0));
        cr.clip();

        let inner_w = (cw - 24.0).max(120.0);
        let view_h = (ch - 12.0).max(40.0);

        if let Some(err) = &frame.cicada.error {
            let layout = pangocairo::functions::create_layout(cr);
            let mut desc = pango::FontDescription::from_string("Sans");
            desc.set_absolute_size(11.2 * pango::SCALE as f64);
            layout.set_font_description(Some(&desc));
            layout.set_width((inner_w * pango::SCALE as f64) as i32);
            layout.set_wrap(pango::WrapMode::WordChar);
            layout.set_text(&format!("Cicada Notice:\n{err}"));
            cr.set_source_rgba(0.98, 0.54, 0.58, 0.95 * alpha);
            cr.move_to(cx + 10.0, cy + 10.0);
            pangocairo::functions::show_layout(cr, &layout);
            self.last_cicada_max_scroll = 0.0;
        } else if frame.cicada.entries.is_empty() {
            let msg = if frame.cicada.active {
                "Menunggu suara rapat dari PipeWire (@DEFAULT_MONITOR@)...\n\n• Klik 'Subtitle Bar' untuk tampilan bar tipis click-through.\n• Nyalakan 'Mic: ON' untuk transkrip 2 arah.\n• Klik 'Reply' untuk bicara dan langsung diterjemahkan."
            } else {
                "Cicada Mode — Live Meeting Translator\n\nKlik '▶ Start Live' untuk menangkap audio sistem tanpa bot rapat.\nAtur bahasa target & Custom Glossary lewat Config."
            };
            let layout = pangocairo::functions::create_layout(cr);
            let mut desc = pango::FontDescription::from_string("Sans");
            desc.set_absolute_size(11.0 * pango::SCALE as f64);
            layout.set_font_description(Some(&desc));
            layout.set_width((inner_w * pango::SCALE as f64) as i32);
            layout.set_wrap(pango::WrapMode::WordChar);
            layout.set_text(msg);
            cr.set_source_rgba(0.80, 0.85, 0.92, 0.72 * alpha);
            cr.move_to(cx + 10.0, cy + 10.0);
            pangocairo::functions::show_layout(cr, &layout);
            self.last_cicada_max_scroll = 0.0;
        } else {
            struct Measured {
                header: String,
                orig: Option<pango::Layout>,
                orig_h: f64,
                trans: pango::Layout,
                speaker: SpeakerChannel,
                total_h: f64,
            }

            let mut orig_desc = pango::FontDescription::from_string("Sans");
            orig_desc.set_absolute_size(10.5 * pango::SCALE as f64);
            let mut tr_desc = pango::FontDescription::from_string("Sans Bold");
            tr_desc.set_absolute_size(12.0 * pango::SCALE as f64);

            let mut items = Vec::with_capacity(frame.cicada.entries.len());
            let mut total_h = 0.0f64;
            for entry in &frame.cicada.entries {
                let header = format!(
                    "{}  {}  {}→{}",
                    entry.timestamp_label(),
                    entry.speaker.tag(),
                    cicada_lang_short_badge(&entry.source_lang),
                    cicada_lang_short_badge(&entry.target_lang),
                );
                let (orig, orig_h) = if frame.cicada.show_original
                    && !entry.original.eq_ignore_ascii_case(&entry.translation)
                {
                    let l = pangocairo::functions::create_layout(cr);
                    l.set_font_description(Some(&orig_desc));
                    l.set_width(((inner_w - 8.0) * pango::SCALE as f64) as i32);
                    l.set_wrap(pango::WrapMode::WordChar);
                    l.set_text(&entry.original);
                    let (_, h) = l.pixel_size();
                    (Some(l), h as f64 + 2.0)
                } else {
                    (None, 0.0)
                };

                let trans = pangocairo::functions::create_layout(cr);
                trans.set_font_description(Some(&tr_desc));
                trans.set_width(((inner_w - 8.0) * pango::SCALE as f64) as i32);
                trans.set_wrap(pango::WrapMode::WordChar);
                trans.set_text(&entry.translation);
                let (_, th) = trans.pixel_size();

                let bh = 16.0 + orig_h + th as f64 + 10.0;
                total_h += bh;
                items.push(Measured {
                    header,
                    orig,
                    orig_h,
                    trans,
                    speaker: entry.speaker,
                    total_h: bh,
                });
            }

            let max_s = (total_h - view_h).max(0.0);
            self.last_cicada_max_scroll = max_s as f32;
            let scroll = (frame.cicada_scroll_offset as f64).clamp(0.0, max_s);

            let mut cur_y = cy + 8.0 - scroll;
            for item in &items {
                if cur_y + item.total_h >= cy - 40.0 && cur_y <= cy + ch + 40.0 {
                    let (sr, sg, sb) = match item.speaker {
                        SpeakerChannel::Online => (0.24, 0.80, 0.98),
                        SpeakerChannel::You => (0.22, 0.88, 0.62),
                        SpeakerChannel::QuickReply => (0.98, 0.76, 0.25),
                    };
                    rounded_rect_path(cr, cx + 10.0, cur_y + 2.0, 2.5, item.total_h - 8.0, 1.2);
                    cr.set_source_rgba(sr, sg, sb, 0.85 * alpha);
                    let _ = cr.fill();

                    draw_text(
                        cr,
                        &item.header,
                        cx + 18.0,
                        cur_y,
                        inner_w - 8.0,
                        10.2,
                        true,
                        pango::Alignment::Left,
                        (sr, sg, sb, 0.92 * alpha),
                    );
                    let mut ty = cur_y + 15.0;
                    if let Some(ol) = &item.orig {
                        cr.set_source_rgba(0.75, 0.80, 0.88, 0.68 * alpha);
                        cr.move_to(cx + 18.0, ty);
                        pangocairo::functions::show_layout(cr, ol);
                        ty += item.orig_h;
                    }
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.97 * alpha);
                    cr.move_to(cx + 18.0, ty);
                    pangocairo::functions::show_layout(cr, &item.trans);
                }
                cur_y += item.total_h;
            }

            if max_s > 1.0 {
                let track_h = view_h - 4.0;
                let thumb_h = (track_h * (view_h / total_h)).clamp(18.0, track_h);
                let thumb_y = cy + 8.0 + (scroll / max_s) * (track_h - thumb_h);
                rounded_rect_path(cr, cx + cw - 6.0, thumb_y, 3.0, thumb_h, 1.5);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.28 * alpha);
                let _ = cr.fill();
            }
        }
        let _ = cr.restore();

        let has_entries = frame.cicada.has_entries();
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.copy_btn,
            if frame.cicada_copied_flash { "Copied!" } else { "Copy MD" },
            (0.20, 0.85, 0.56),
            has_entries && hovered == Some(HitTarget::CicadaCopyTranscript),
            frame.cicada_copied_flash,
            if has_entries { alpha } else { alpha * 0.42 },
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.summary_btn,
            "Auto-Summary",
            (0.65, 0.55, 0.98),
            has_entries && hovered == Some(HitTarget::CicadaSummarize),
            false,
            if has_entries { alpha } else { alpha * 0.42 },
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.ask_btn,
            "Ask Zeki",
            (0.22, 0.74, 0.97),
            hovered == Some(HitTarget::CicadaAskZeki),
            true,
            alpha,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.clear_btn,
            "Clear",
            (0.94, 0.34, 0.42),
            has_entries && hovered == Some(HitTarget::CicadaClear),
            false,
            if has_entries { alpha } else { alpha * 0.42 },
        );
    }

    fn draw_collapsed_assistant(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f64,
        pill_h: f64,
        frame: &Frame,
        alpha: f64,
    ) {
        frame.zeki.draw(cr, ox + 22.0, oy + pill_h / 2.0, 11.5, alpha, true);
        let (zr, zg, zb) = frame.zeki.accent_color();
        let label = match frame.assistant.phase {
            AssistantPhase::Listening => "Listening...",
            AssistantPhase::Transcribing => "Transcribing...",
            AssistantPhase::Thinking => "Thinking...",
            AssistantPhase::Speaking => "Speaking...",
            AssistantPhase::Error => "Assistant Error",
            _ => frame.zeki.state().collapsed_tag(),
        };
        draw_text_centered_v(
            cr,
            label,
            ox + 40.0,
            oy,
            pill_w - 92.0,
            pill_h,
            12.5,
            true,
            pango::Alignment::Left,
            (zr * 0.45 + 0.55, zg * 0.45 + 0.55, zb * 0.45 + 0.55, alpha),
        );

        let badge = if frame.assistant.phase.is_listening() {
            "● MIC"
        } else if frame.assistant.screen_capturing_now {
            "● SCR"
        } else {
            &frame.assistant.provider_label
        };
        let badge_col = if frame.assistant.phase.is_listening() {
            (0.13, 0.85, 0.94, alpha)
        } else if frame.assistant.screen_capturing_now {
            (0.98, 0.65, 0.25, alpha)
        } else {
            (1.0, 1.0, 1.0, 0.65 * alpha)
        };
        draw_text_centered_v(
            cr,
            badge,
            ox + pill_w - 58.0,
            oy,
            50.0,
            pill_h,
            10.5,
            true,
            pango::Alignment::Right,
            badge_col,
        );
    }

    fn draw_collapsed_media(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f64,
        pill_h: f64,
        frame: &Frame,
        alpha: f64,
    ) {
        self.draw_art(cr, ox, oy, &COLLAPSED_ART, 6.0);

        let unread = frame.notification.unread_count();
        let center_text = if frame.timer.running {
            frame.timer.formatted()
        } else {
            current_clock_string()
        };
        let center_color = if frame.timer.running {
            (ACCENT_RED.0, ACCENT_RED.1, ACCENT_RED.2, alpha)
        } else {
            (1.0, 1.0, 1.0, 0.96 * alpha)
        };

        if unread > 0 {
            draw_text_centered_v(
                cr,
                &center_text,
                ox + 36.0,
                oy,
                66.0,
                pill_h,
                16.0,
                true,
                pango::Alignment::Center,
                center_color,
            );
            draw_equalizer(
                cr,
                ox,
                oy,
                pill_h / 2.0,
                &frame.eq_levels,
                frame.media.is_playing,
                alpha,
                pill_w - 62.0,
                EQUALIZER_BAR_WIDTH as f64,
                EQUALIZER_BAR_GAP as f64,
                3.5,
                14.5,
            );
            if let Some(app_name) = frame.notification.latest_app_name() {
                draw_collapsed_notification_badge(
                    cr, ox, oy, pill_w, pill_h, app_name, unread, alpha,
                );
            }
        } else {
            draw_text_centered_v(
                cr,
                &center_text,
                ox + 42.0,
                oy,
                76.0,
                pill_h,
                16.5,
                true,
                pango::Alignment::Center,
                center_color,
            );
            draw_equalizer(
                cr,
                ox,
                oy,
                pill_h / 2.0,
                &frame.eq_levels,
                frame.media.is_playing,
                alpha,
                EQUALIZER_RIGHT as f64,
                EQUALIZER_BAR_WIDTH as f64,
                EQUALIZER_BAR_GAP as f64,
                3.5,
                14.5,
            );
        }
    }

    fn draw_collapsed_idle(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f64,
        pill_h: f64,
        frame: &Frame,
        alpha: f64,
    ) {
        // Animated mini Zeki mascot right in the idle collapsed pill
        frame.zeki.draw(cr, ox + 19.0, oy + pill_h / 2.0, 10.2, alpha, true);

        let clock = current_clock_string();
        let unread = frame.notification.unread_count();

        if frame.timer.running {
            draw_text_centered_v(
                cr,
                &frame.timer.formatted(),
                ox + 34.0,
                oy,
                62.0,
                pill_h,
                15.5,
                true,
                pango::Alignment::Center,
                (ACCENT_RED.0, ACCENT_RED.1, ACCENT_RED.2, alpha),
            );
            if unread > 0 {
                if let Some(app_name) = frame.notification.latest_app_name() {
                    draw_collapsed_notification_badge(
                        cr, ox, oy, pill_w, pill_h, app_name, unread, alpha,
                    );
                }
            } else {
                draw_text_centered_v(
                    cr,
                    &clock,
                    ox + pill_w - 72.0,
                    oy,
                    64.0,
                    pill_h,
                    16.0,
                    true,
                    pango::Alignment::Center,
                    (1.0, 1.0, 1.0, 0.95 * alpha),
                );
            }
            return;
        }

        if unread > 0 {
            draw_text_centered_v(
                cr,
                &clock,
                ox + 36.0,
                oy,
                68.0,
                pill_h,
                16.5,
                true,
                pango::Alignment::Center,
                (1.0, 1.0, 1.0, 0.98 * alpha),
            );
            if let Some(app_name) = frame.notification.latest_app_name() {
                draw_collapsed_notification_badge(
                    cr, ox, oy, pill_w, pill_h, app_name, unread, alpha,
                );
            }
        } else {
            draw_text_centered_v(
                cr,
                frame.zeki.state().collapsed_tag(),
                ox + 36.0,
                oy,
                66.0,
                pill_h,
                12.0,
                true,
                pango::Alignment::Left,
                (1.0, 1.0, 1.0, 0.84 * alpha),
            );
            draw_text_centered_v(
                cr,
                &clock,
                ox + pill_w - 72.0,
                oy,
                64.0,
                pill_h,
                16.5,
                true,
                pango::Alignment::Center,
                (1.0, 1.0, 1.0, 0.98 * alpha),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_zeki_panel(
        &mut self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f32,
        pill_h: f32,
        frame: &Frame,
        alpha: f64,
    ) {
        let (zr, zg, zb) = frame.zeki.accent_color();
        let stage = ZEKI_STAGE_RECT;
        let stage_hovered = frame.hovered_target == Some(HitTarget::ZekiPoke);

        rounded_rect_path(
            cr,
            ox + stage.left as f64,
            oy + stage.top as f64,
            stage.width() as f64,
            stage.height() as f64,
            18.0,
        );
        cr.set_source_rgba(zr, zg, zb, if stage_hovered { 0.16 } else { 0.08 } * alpha);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(zr, zg, zb, 0.35 * alpha);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        let (scx, scy) = stage.center();
        frame.zeki.draw(
            cr,
            ox + scx as f64,
            oy + scy as f64 + 3.0,
            26.0,
            alpha,
            false,
        );

        // Live hardware indicator badge inside stage top-right
        if frame.assistant.phase.is_listening() || frame.assistant.screen_capturing_now {
            let (ind_txt, ind_col) = if frame.assistant.phase.is_listening() {
                ("● MIC", (0.13, 0.85, 0.94, alpha))
            } else {
                ("● SCR", (0.98, 0.65, 0.25, alpha))
            };
            draw_text(
                cr,
                ind_txt,
                ox + stage.left as f64 + 6.0,
                oy + stage.top as f64 + 5.0,
                stage.width() as f64 - 12.0,
                9.5,
                true,
                pango::Alignment::Right,
                ind_col,
            );
        }

        let listening = frame.assistant.phase.is_listening();
        let screen_on = frame.assistant.screen_access_enabled;

        if !frame.assistant_chat_open {
            // Preview mode (440x198)
            draw_text(
                cr,
                "Zeki Assistant",
                ox + 132.0,
                oy + 45.0,
                84.0,
                13.0,
                true,
                pango::Alignment::Left,
                (1.0, 1.0, 1.0, 0.96 * alpha),
            );
            let sub = format!(
                "{}: {} • {}",
                frame.zeki.state().label(),
                frame.zeki.state().status_line(),
                frame.ptt_hotkey_short
            );
            draw_text(
                cr,
                &sub,
                ox + 132.0,
                oy + 74.0,
                290.0,
                11.0,
                false,
                pango::Alignment::Left,
                (1.0, 1.0, 1.0, 0.68 * alpha),
            );

            draw_tinted_pill(
                cr,
                ox,
                oy,
                ZEKI_CHAT_BTN_PREVIEW,
                "Chat",
                (zr, zg, zb),
                frame.hovered_target == Some(HitTarget::ZekiOpenChat),
                true,
                alpha,
            );
            draw_tinted_pill(
                cr,
                ox,
                oy,
                ZEKI_MIC_BTN_PREVIEW,
                if listening { "Stop" } else { "Voice" },
                if listening { ACCENT_RED } else { (0.13, 0.83, 0.93) },
                frame.hovered_target == Some(HitTarget::ZekiMicToggle),
                listening,
                alpha,
            );
            draw_action_pill(
                cr,
                ox,
                oy,
                ZEKI_CFG_BTN_PREVIEW,
                "Config",
                frame.hovered_target == Some(HitTarget::ZekiOpenConfig),
                alpha,
            );
            draw_tinted_pill(
                cr,
                ox,
                oy,
                ZEKI_PRIVACY_BTN_PREVIEW,
                if screen_on { "Screen: ON" } else { "Incognito" },
                if screen_on {
                    (0.18, 0.80, 0.52)
                } else {
                    (0.95, 0.58, 0.18)
                },
                frame.hovered_target == Some(HitTarget::ZekiTogglePrivacy),
                !screen_on,
                alpha,
            );

            for (i, &state) in ALL_ZEKI_STATES.iter().enumerate() {
                let rect = zeki_state_btn_rect(i);
                let active = frame.zeki.state() == state;
                let hov = frame.hovered_target == Some(HitTarget::ZekiSelectState(state));
                let (sr, sg, sb) = state.accent_rgb();
                draw_tinted_pill(
                    cr,
                    ox,
                    oy,
                    rect,
                    state.short_label(),
                    (sr as f64, sg as f64, sb as f64),
                    hov,
                    active,
                    alpha,
                );
            }
            return;
        }

        // Chat Mode (520x310 normal or 640x470 large on double-click)
        let has_file = frame.chat_attached_label.is_some();
        let cl = chat_layout(pill_w, pill_h, has_file);

        // Left column controls
        draw_tinted_pill(
            cr,
            ox,
            oy,
            ZEKI_MIC_BTN_CHAT,
            if listening { "Stop Mic" } else { "Voice" },
            if listening { ACCENT_RED } else { (0.13, 0.83, 0.93) },
            frame.hovered_target == Some(HitTarget::ZekiMicToggle),
            listening,
            alpha,
        );
        draw_action_pill(
            cr,
            ox,
            oy,
            ZEKI_CFG_BTN_CHAT,
            "Config",
            frame.hovered_target == Some(HitTarget::ZekiOpenConfig),
            alpha,
        );
        draw_tinted_pill(
            cr,
            ox,
            oy,
            ZEKI_PRIVACY_BTN_CHAT,
            if screen_on { "Screen: ON" } else { "Incognito" },
            if screen_on {
                (0.18, 0.80, 0.52)
            } else {
                (0.95, 0.58, 0.18)
            },
            frame.hovered_target == Some(HitTarget::ZekiTogglePrivacy),
            !screen_on,
            alpha,
        );

        // Right column header
        let hdr_text = if frame.chat_copied_flash {
            "Copied!".to_string()
        } else {
            format!("{} • {}", frame.assistant.provider_label, frame.ptt_hotkey_short)
        };
        let hdr_col = if frame.chat_copied_flash {
            (0.20, 0.86, 0.50, alpha)
        } else {
            (1.0, 1.0, 1.0, 0.80 * alpha)
        };
        draw_text_centered_v(
            cr,
            &hdr_text,
            ox + 134.0,
            oy + 45.0,
            (cl.expand_btn.left - 138.0).max(50.0) as f64,
            21.0,
            11.0,
            true,
            pango::Alignment::Left,
            hdr_col,
        );

        draw_action_pill(
            cr,
            ox,
            oy,
            cl.expand_btn,
            if frame.chat_large { "Small" } else { "Large" },
            frame.hovered_target == Some(HitTarget::ZekiChatToggleSize),
            alpha,
        );
        let has_reply = !frame.assistant.reply.is_empty();
        if has_reply {
            draw_tinted_pill(
                cr,
                ox,
                oy,
                cl.insert_btn,
                "Insert",
                (0.20, 0.80, 0.50),
                frame.hovered_target == Some(HitTarget::ZekiInsertReply),
                false,
                alpha,
            );
            draw_action_pill(
                cr,
                ox,
                oy,
                cl.copy_btn,
                "Copy",
                frame.hovered_target == Some(HitTarget::ZekiCopyReply),
                alpha,
            );
        } else if frame.assistant.error.is_some() {
            draw_action_pill(
                cr,
                ox,
                oy,
                cl.copy_btn,
                "Setup",
                frame.hovered_target == Some(HitTarget::ZekiOpenConfig),
                alpha,
            );
        }
        draw_action_pill(
            cr,
            ox,
            oy,
            cl.clear_btn,
            "Clear",
            frame.hovered_target == Some(HitTarget::ZekiClearChat),
            alpha,
        );

        // Scrollable multi-turn chat card
        let cx = ox + cl.card.left as f64;
        let cy = oy + cl.card.top as f64;
        let cw = cl.card.width() as f64;
        let ch = cl.card.height() as f64;

        rounded_rect_path(cr, cx, cy, cw, ch, 12.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.055 * alpha);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.12 * alpha);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        // Build formatted multi-turn conversation transcript
        let body_text = build_chat_display_text(frame.assistant);
        let pad = 10.0;
        let inner_w = (cw - pad * 2.0 - 8.0).max(40.0);
        let inner_h = (ch - pad * 2.0).max(20.0);

        let layout = pangocairo::functions::create_layout(cr);
        let mut desc = pango::FontDescription::from_string("Sans");
        desc.set_absolute_size(12.0 * pango::SCALE as f64);
        layout.set_font_description(Some(&desc));
        layout.set_width((inner_w * pango::SCALE as f64) as i32);
        layout.set_wrap(pango::WrapMode::WordChar);
        layout.set_text(&body_text);

        let (_, text_h_px) = layout.pixel_size();
        let max_scroll = ((text_h_px as f64 - inner_h).max(0.0)) as f32;
        self.last_chat_max_scroll = max_scroll;
        let clamped_scroll = frame.chat_scroll_offset.clamp(0.0, max_scroll) as f64;

        let _ = cr.save();
        rounded_rect_path(cr, cx + 2.0, cy + 2.0, cw - 4.0, ch - 4.0, 10.0);
        cr.clip();

        let text_col = if frame.assistant.error.is_some()
            && frame.assistant.messages.is_empty()
            && frame.assistant.reply.is_empty()
        {
            (1.0, 0.55, 0.58, 0.96 * alpha)
        } else {
            (1.0, 1.0, 1.0, 0.92 * alpha)
        };
        cr.set_source_rgba(text_col.0, text_col.1, text_col.2, text_col.3);
        cr.move_to(cx + pad, cy + pad - clamped_scroll);
        pangocairo::functions::show_layout(cr, &layout);
        let _ = cr.restore();

        // Scrollbar thumb when content overflows
        if max_scroll > 1.0 {
            let track_x = cx + cw - 7.0;
            let track_y = cy + 8.0;
            let track_h = (ch - 16.0).max(16.0);
            rounded_rect_path(cr, track_x, track_y, 3.5, track_h, 1.75);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.12 * alpha);
            let _ = cr.fill();

            let frac_vis = (inner_h / text_h_px.max(1) as f64).clamp(0.16, 1.0);
            let thumb_h = (track_h * frac_vis).max(18.0);
            let scroll_ratio = (clamped_scroll / max_scroll as f64).clamp(0.0, 1.0);
            let thumb_y = track_y + (track_h - thumb_h) * scroll_ratio;
            rounded_rect_path(cr, track_x, thumb_y, 3.5, thumb_h, 1.75);
            cr.set_source_rgba(zr, zg, zb, 0.78 * alpha);
            let _ = cr.fill();
        }

        // Attached file chip
        if let Some(file_lbl) = frame.chat_attached_label {
            let chip = cl.file_chip;
            let hov = frame.hovered_target == Some(HitTarget::ZekiRemoveFile);
            rounded_rect_path(
                cr,
                ox + chip.left as f64,
                oy + chip.top as f64,
                chip.width() as f64,
                chip.height() as f64,
                8.0,
            );
            cr.set_source_rgba(zr, zg, zb, if hov { 0.28 } else { 0.16 } * alpha);
            let _ = cr.fill();
            let chip_txt = format!("File: {file_lbl}  (klik untuk hapus)");
            draw_text_centered_v(
                cr,
                &chip_txt,
                ox + chip.left as f64 + 8.0,
                oy + chip.top as f64,
                chip.width() as f64 - 16.0,
                chip.height() as f64,
                10.5,
                true,
                pango::Alignment::Left,
                (1.0, 1.0, 1.0, 0.92 * alpha),
            );
        }

        // Attach file button
        draw_action_pill(
            cr,
            ox,
            oy,
            cl.attach_btn,
            "+ File",
            frame.hovered_target == Some(HitTarget::ZekiAttachFile),
            alpha,
        );

        // Chat text input field
        let inp = cl.input;
        rounded_rect_path(
            cr,
            ox + inp.left as f64,
            oy + inp.top as f64,
            inp.width() as f64,
            inp.height() as f64,
            10.0,
        );
        cr.set_source_rgba(
            1.0,
            1.0,
            1.0,
            if frame.chat_focused { 0.12 } else { 0.07 } * alpha,
        );
        let _ = cr.fill_preserve();
        if frame.chat_focused {
            cr.set_source_rgba(zr, zg, zb, 0.80 * alpha);
            cr.set_line_width(1.3);
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * alpha);
            cr.set_line_width(1.0);
        }
        let _ = cr.stroke();

        let (display_inp, inp_col) = if frame.chat_input.is_empty() {
            (
                if frame.chat_focused {
                    "Ketik pesan lalu Enter...|".to_string()
                } else {
                    "Klik di sini untuk chat (2x klik card = perbesar)...".to_string()
                },
                (1.0, 1.0, 1.0, 0.45 * alpha),
            )
        } else if frame.chat_focused {
            (format!("{}|", frame.chat_input), (1.0, 1.0, 1.0, 0.96 * alpha))
        } else {
            (frame.chat_input.to_string(), (1.0, 1.0, 1.0, 0.92 * alpha))
        };
        draw_text_centered_v(
            cr,
            &display_inp,
            ox + inp.left as f64 + 10.0,
            oy + inp.top as f64,
            (inp.width() as f64 - 20.0).max(20.0),
            inp.height() as f64,
            12.0,
            false,
            pango::Alignment::Left,
            inp_col,
        );

        // Send button
        draw_tinted_pill(
            cr,
            ox,
            oy,
            cl.send_btn,
            "Send",
            (zr, zg, zb),
            frame.hovered_target == Some(HitTarget::ZekiChatSend),
            !frame.chat_input.trim().is_empty(),
            alpha,
        );
    }

    fn draw_expanded_media(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        frame: &Frame,
        alpha: f64,
    ) {
        // Media logo top-left directly above the bold clock
        self.draw_art(cr, ox, oy, &EXPANDED_ART, 12.0);
        draw_text_centered_v(
            cr,
            &current_clock_string(),
            ox + MEDIA_CLOCK_BOX.left as f64,
            oy + MEDIA_CLOCK_BOX.top as f64,
            MEDIA_CLOCK_BOX.width() as f64,
            MEDIA_CLOCK_BOX.height() as f64,
            18.5,
            true,
            pango::Alignment::Center,
            (1.0, 1.0, 1.0, 0.96 * alpha),
        );

        let title = if frame.media.title.trim().is_empty() {
            "Unknown title"
        } else {
            frame.media.title.as_str()
        };
        draw_text(
            cr,
            title,
            ox + TITLE_BOX.left as f64,
            oy + TITLE_BOX.top as f64,
            TITLE_BOX.width() as f64,
            17.5,
            true,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, alpha),
        );
        draw_text(
            cr,
            &frame.media.artist,
            ox + ARTIST_BOX.left as f64,
            oy + ARTIST_BOX.top as f64,
            ARTIST_BOX.width() as f64,
            13.5,
            false,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, 0.65 * alpha),
        );

        let hovered_ctrl = match frame.hovered_target {
            Some(HitTarget::MediaControl(c)) => Some(c),
            _ => None,
        };
        draw_controls(cr, ox, oy, alpha, frame.media.is_playing, hovered_ctrl);

        draw_equalizer(
            cr,
            ox,
            oy,
            CONTROLS_TOP as f64 + 15.0,
            &frame.eq_levels,
            frame.media.is_playing,
            alpha,
            418.0,
            4.0,
            3.5,
            4.0,
            20.0,
        );
    }

    fn draw_art(&self, cr: &cairo::Context, ox: f64, oy: f64, rect: &RectF, radius: f64) {
        let x = ox + rect.left as f64;
        let y = oy + rect.top as f64;
        let w = rect.width() as f64;
        let h = rect.height() as f64;

        let _ = cr.save();
        rounded_rect_path(cr, x, y, w, h, radius);
        cr.clip();

        if let Some(ref pixbuf) = self.art_pixbuf {
            let pw = pixbuf.width().max(1) as f64;
            let ph = pixbuf.height().max(1) as f64;
            let scale = (w / pw).max(h / ph);
            let draw_w = pw * scale;
            let draw_h = ph * scale;
            let dx = x + (w - draw_w) / 2.0;
            let dy = y + (h - draw_h) / 2.0;

            cr.translate(dx, dy);
            cr.scale(scale, scale);
            cr.set_source_pixbuf(pixbuf, 0.0, 0.0);
            let _ = cr.paint();
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
            let _ = cr.paint();
        }

        let _ = cr.restore();
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_idle_expanded(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f64,
        system: &SystemSnapshot,
        notif: &NotificationSnapshot,
        alpha: f64,
    ) {
        let pad_x = 22.0;
        let content_w = (pill_w - pad_x * 2.0).max(40.0);

        let now_local = Local::now();
        let clock = now_local.format("%H:%M").to_string();
        draw_text(
            cr,
            &clock,
            ox + pad_x,
            oy + 46.0,
            125.0,
            34.0,
            true,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, alpha),
        );

        let date_str = now_local.format("%a, %d %b").to_string();
        let sub = if notif.unread_count() > 0 {
            format!("{date_str}  •  {} unread", notif.unread_count())
        } else {
            format!("{date_str}  •  No media playing")
        };
        draw_text(
            cr,
            &sub,
            ox + pad_x + 118.0,
            oy + 60.0,
            content_w - 122.0,
            14.0,
            false,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, 0.76 * alpha),
        );

        let bar_y = oy + 136.0;
        let bar_h = 6.0;
        let vol_lbl = if system.muted {
            "Volume  •  Muted".to_string()
        } else {
            format!("Volume  •  {}%", (system.volume * 100.0).round() as u32)
        };
        draw_text(
            cr,
            &vol_lbl,
            ox + pad_x,
            bar_y - 19.0,
            content_w,
            12.0,
            true,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, 0.66 * alpha),
        );

        rounded_rect_path(cr, ox + pad_x, bar_y, content_w, bar_h, bar_h / 2.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * alpha);
        let _ = cr.fill();

        let clamped = system.volume.clamp(0.0, 1.0) as f64;
        let fill_w = (content_w * clamped).max(if clamped > 0.001 { bar_h } else { 0.0 });
        if fill_w > 0.0 {
            rounded_rect_path(cr, ox + pad_x, bar_y, fill_w, bar_h, bar_h / 2.0);
            cr.set_source_rgba(
                1.0,
                1.0,
                1.0,
                if system.muted { 0.30 } else { 0.88 } * alpha,
            );
            let _ = cr.fill();
        }
    }

    fn draw_notification_toast(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        item: &NotificationItem,
        progress: f32,
        hovered: Option<HitTarget>,
    ) {
        let (ar, ag, ab) = app_accent_rgb(&item.app);

        let ax = ox + TOAST_AVATAR.left as f64;
        let ay = oy + TOAST_AVATAR.top as f64;
        let aw = TOAST_AVATAR.width() as f64;
        let ah = TOAST_AVATAR.height() as f64;
        rounded_rect_path(cr, ax, ay, aw, ah, 10.0);
        cr.set_source_rgba(ar, ag, ab, 0.36);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(ar, ag, ab, 0.58);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        draw_text_centered_v(
            cr,
            &app_initial(&item.app),
            ax,
            ay,
            aw,
            ah,
            14.0,
            true,
            pango::Alignment::Center,
            (1.0, 1.0, 1.0, 1.0),
        );

        let tx = ox + 64.0;
        let tw = (TOAST_DISMISS_BTN.left - 68.0).max(60.0) as f64;
        let header = format!("{}  •  just now", item.app);
        draw_text(
            cr,
            &header,
            tx,
            oy + 13.0,
            tw,
            11.5,
            true,
            pango::Alignment::Left,
            (ar, ag, ab, 1.0),
        );

        let title = if item.title.is_empty() {
            "Notification"
        } else {
            item.title.as_str()
        };
        draw_text(
            cr,
            title,
            tx,
            oy + 30.0,
            tw,
            13.0,
            true,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, 0.96),
        );

        if !item.body.is_empty() {
            draw_text(
                cr,
                &item.body,
                tx,
                oy + 49.0,
                tw + 18.0,
                11.5,
                false,
                pango::Alignment::Left,
                (1.0, 1.0, 1.0, 0.68),
            );
        }

        draw_dismiss_x(
            cr,
            ox,
            oy,
            TOAST_DISMISS_BTN,
            hovered == Some(HitTarget::ToastDismiss(item.id)),
            1.0,
        );

        let bar_w = TOAST_PROGRESS_RECT.width() as f64 * progress.clamp(0.0, 1.0) as f64;
        if bar_w > 1.0 {
            rounded_rect_path(
                cr,
                ox + TOAST_PROGRESS_RECT.left as f64,
                oy + TOAST_PROGRESS_RECT.top as f64,
                bar_w,
                TOAST_PROGRESS_RECT.height() as f64,
                1.25,
            );
            cr.set_source_rgba(ar, ag, ab, 0.72);
            let _ = cr.fill();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_notifications_panel(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f64,
        notif: &NotificationSnapshot,
        hovered: Option<HitTarget>,
        alpha: f64,
    ) {
        let unread = notif.unread_count();
        let header = if unread > NOTIF_CARD_MAX_VISIBLE {
            format!(
                "Notifications ({unread})  •  +{} more",
                unread - NOTIF_CARD_MAX_VISIBLE
            )
        } else if unread > 0 {
            format!("Notifications ({unread})")
        } else {
            "Notifications".to_string()
        };
        draw_text(
            cr,
            &header,
            ox + 20.0,
            oy + 41.0,
            260.0,
            12.0,
            true,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, 0.84 * alpha),
        );

        if !notif.items.is_empty() {
            draw_action_pill(
                cr,
                ox,
                oy,
                NOTIF_CLEAR_BTN,
                "Clear all",
                hovered == Some(HitTarget::ClearAllNotifications),
                alpha,
            );
        }

        if notif.items.is_empty() {
            draw_text(
                cr,
                "No notifications",
                ox + 16.0,
                oy + 82.0,
                pill_w - 32.0,
                13.5,
                true,
                pango::Alignment::Center,
                (1.0, 1.0, 1.0, 0.90 * alpha),
            );
            draw_text(
                cr,
                "All caught up!",
                ox + 16.0,
                oy + 106.0,
                pill_w - 32.0,
                11.5,
                false,
                pango::Alignment::Center,
                (1.0, 1.0, 1.0, 0.65 * alpha),
            );
            return;
        }

        for (index, item) in notif
            .items
            .iter()
            .take(NOTIF_CARD_MAX_VISIBLE)
            .enumerate()
        {
            let card = notif_card_rect(index);
            let dismiss_rect = notif_card_dismiss_rect(index);
            let card_hovered = hovered == Some(HitTarget::NotifDismiss(item.id));

            rounded_rect_path(
                cr,
                ox + card.left as f64,
                oy + card.top as f64,
                card.width() as f64,
                card.height() as f64,
                10.0,
            );
            cr.set_source_rgba(
                1.0,
                1.0,
                1.0,
                if card_hovered { 0.11 } else { 0.055 } * alpha,
            );
            let _ = cr.fill();

            let (ar, ag, ab) = app_accent_rgb(&item.app);
            let av_x = ox + card.left as f64 + 8.0;
            let av_y = oy + card.top as f64 + 8.0;
            rounded_rect_path(cr, av_x, av_y, 25.0, 25.0, 6.0);
            cr.set_source_rgba(ar, ag, ab, 0.90 * alpha);
            let _ = cr.fill();

            draw_text_centered_v(
                cr,
                &app_initial(&item.app),
                av_x,
                av_y,
                25.0,
                25.0,
                11.5,
                true,
                pango::Alignment::Center,
                (1.0, 1.0, 1.0, alpha),
            );

            let tx = ox + card.left as f64 + 40.0;
            let tw = (dismiss_rect.left - (card.left + 44.0)).max(60.0) as f64;
            let meta = format!("{}  •  {}", item.app, format_relative_time(item.timestamp_ms));
            draw_text(
                cr,
                &meta,
                tx,
                oy + card.top as f64 + 4.0,
                tw,
                10.5,
                true,
                pango::Alignment::Left,
                (ar, ag, ab, 0.95 * alpha),
            );

            let line2 = if item.body.is_empty() {
                item.title.clone()
            } else if item.title.is_empty() {
                item.body.clone()
            } else {
                format!("{} — {}", item.title, item.body)
            };
            draw_text(
                cr,
                &line2,
                tx,
                oy + card.top as f64 + 19.0,
                tw,
                12.0,
                true,
                pango::Alignment::Left,
                (1.0, 1.0, 1.0, 0.94 * alpha),
            );

            draw_dismiss_x(cr, ox, oy, dismiss_rect, card_hovered, alpha);
        }
    }

    fn draw_hud(
        &self,
        cr: &cairo::Context,
        ox: f64,
        oy: f64,
        pill_w: f64,
        pill_h: f64,
        hud: &HudState,
    ) {
        let pad_x = 20.0;
        let bar_h = 6.0;
        let bar_w = (pill_w - pad_x * 2.0).max(40.0);
        let bar_x = ox + (pill_w - bar_w) / 2.0;
        let bar_y = oy + (pill_h * 0.58).max(24.0);
        let label_y = (bar_y - 20.0).max(oy + 8.0);

        let label = match hud.kind {
            HudKind::Volume => "Volume",
            HudKind::Brightness => "Brightness",
        };
        let value_text = if hud.kind == HudKind::Volume && hud.muted {
            "Muted".to_string()
        } else {
            format!("{}%", (hud.value.clamp(0.0, 1.0) * 100.0).round() as u32)
        };

        draw_text(
            cr,
            label,
            bar_x,
            label_y,
            bar_w,
            12.5,
            true,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, 0.92),
        );
        draw_text(
            cr,
            &value_text,
            bar_x,
            label_y,
            bar_w,
            12.0,
            true,
            pango::Alignment::Right,
            (1.0, 1.0, 1.0, 0.78),
        );

        rounded_rect_path(cr, bar_x, bar_y, bar_w, bar_h, bar_h / 2.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
        let _ = cr.fill();

        let clamped = hud.value.clamp(0.0, 1.0) as f64;
        let fill_w = (bar_w * clamped).max(if clamped > 0.001 { bar_h } else { 0.0 });
        if fill_w > 0.0 {
            rounded_rect_path(cr, bar_x, bar_y, fill_w, bar_h, bar_h / 2.0);
            cr.set_source_rgba(1.0, 1.0, 1.0, if hud.muted { 0.35 } else { 0.95 });
            let _ = cr.fill();
        }
    }
}

fn build_chat_display_text(assistant: &AssistantSnapshot) -> String {
    let mut parts: Vec<String> = Vec::new();
    for msg in &assistant.messages {
        let prefix = if msg.is_user { "You: " } else { "Zeki: " };
        parts.push(format!("{prefix}{}", msg.text));
    }

    match assistant.phase {
        AssistantPhase::Listening => {
            parts.push("Zeki: Mendengarkan suara Anda...".to_string());
        }
        AssistantPhase::Transcribing => {
            parts.push("Zeki: Mentranskripsi suara...".to_string());
        }
        AssistantPhase::Thinking => {
            if !assistant.transcript.is_empty()
                && assistant
                    .messages
                    .last()
                    .map(|m| !m.is_user || m.text != assistant.transcript)
                    .unwrap_or(true)
            {
                parts.push(format!("You: {}", assistant.transcript));
            }
            parts.push("Zeki: Sedang berpikir...".to_string());
        }
        AssistantPhase::Speaking => {
            if !assistant.reply.is_empty()
                && assistant
                    .messages
                    .last()
                    .map(|m| m.is_user || m.text != assistant.reply)
                    .unwrap_or(true)
            {
                parts.push(format!("Zeki: {}", assistant.reply));
            }
        }
        AssistantPhase::Error => {
            if let Some(err) = &assistant.error {
                parts.push(format!("Error: {err}"));
            }
        }
        _ => {}
    }

    if parts.is_empty() {
        return "Halo! Ketik pertanyaan di bawah, klik + File untuk melampirkan dokumen/gambar/PDF, atau klik Voice untuk berbicara dengan Zeki.\n(Klik 2x area ini untuk memperbesar kotak chat)".to_string();
    }

    parts.join("\n\n")
}

fn draw_zeki_companion_dock(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    frame: &Frame,
    alpha: f64,
) {
    let dock = ZEKI_DOCK_RECT;
    let (zr, zg, zb) = frame.zeki.accent_color();

    rounded_rect_path(
        cr,
        ox + dock.left as f64,
        oy + dock.top as f64,
        dock.width() as f64,
        dock.height() as f64,
        14.0,
    );
    cr.set_source_rgba(zr, zg, zb, 0.08 * alpha);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(zr, zg, zb, 0.28 * alpha);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    frame.zeki.draw(
        cr,
        ox + dock.left as f64 + 19.0,
        oy + dock.top as f64 + dock.height() as f64 / 2.0,
        10.5,
        alpha,
        true,
    );

    let prompt_lbl = match frame.assistant.phase {
        AssistantPhase::Listening => "Listening...",
        AssistantPhase::Thinking => "Thinking...",
        AssistantPhase::Speaking => "Speaking...",
        _ => "Ask Zeki anything...",
    };
    draw_text_centered_v(
        cr,
        prompt_lbl,
        ox + dock.left as f64 + 38.0,
        oy + dock.top as f64,
        158.0,
        dock.height() as f64,
        11.0,
        true,
        pango::Alignment::Left,
        (1.0, 1.0, 1.0, 0.80 * alpha),
    );

    let screen_on = frame.assistant.screen_access_enabled;
    draw_tinted_pill(
        cr,
        ox,
        oy,
        ZEKI_DOCK_PRIVACY_BTN,
        if screen_on { "Screen" } else { "Incog" },
        if screen_on {
            (0.18, 0.80, 0.52)
        } else {
            (0.95, 0.58, 0.18)
        },
        frame.hovered_target == Some(HitTarget::ZekiTogglePrivacy),
        !screen_on,
        alpha,
    );
    draw_tinted_pill(
        cr,
        ox,
        oy,
        ZEKI_DOCK_CHAT_BTN,
        "Chat",
        (zr, zg, zb),
        frame.hovered_target == Some(HitTarget::ZekiOpenChat),
        false,
        alpha,
    );
    let listening = frame.assistant.phase.is_listening();
    draw_tinted_pill(
        cr,
        ox,
        oy,
        ZEKI_DOCK_MIC_BTN,
        if listening { "Stop" } else { "Voice" },
        if listening { ACCENT_RED } else { (0.13, 0.83, 0.93) },
        frame.hovered_target == Some(HitTarget::ZekiMicToggle),
        listening,
        alpha,
    );
}

fn draw_tab_bar(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    pill_w: f32,
    frame: &Frame,
    alpha: f64,
) {
    let unread = frame.notification.unread_count();
    let (zr, zg, zb) = frame.zeki.accent_color();
    for tab in ALL_TABS {
        let rect = tab_rect_for_width(tab, pill_w);
        let active = frame.expanded_tab == tab;
        let hovered = frame.hovered_target == Some(HitTarget::SelectTab(tab));

        let x = ox + rect.left as f64;
        let y = oy + rect.top as f64;
        let w = rect.width() as f64;
        let h = rect.height() as f64;

        let accent = if tab == ExpandedTab::Zeki {
            (zr, zg, zb)
        } else if tab == ExpandedTab::Cicada {
            (0.16, 0.86, 0.62)
        } else {
            ACCENT_RED
        };

        rounded_rect_path(cr, x, y, w, h, h / 2.0);
        if active {
            cr.set_source_rgba(accent.0, accent.1, accent.2, 0.28 * alpha);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(accent.0, accent.1, accent.2, 0.68 * alpha);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, if hovered { 0.14 } else { 0.055 } * alpha);
            let _ = cr.fill();
        }

        let label = match tab {
            ExpandedTab::Zeki => "Zeki".to_string(),
            ExpandedTab::Cicada => {
                if frame.cicada.active {
                    "● Live".to_string()
                } else {
                    "Live".to_string()
                }
            }
            ExpandedTab::Media => "Media".to_string(),
            ExpandedTab::Timer => {
                if frame.timer.running {
                    frame.timer.formatted()
                } else {
                    "Timer".to_string()
                }
            }
            ExpandedTab::Clipboard => "Clip".to_string(),
            ExpandedTab::System => "Sys".to_string(),
            ExpandedTab::Notifications => {
                if unread > 9 {
                    "Notifs 9+".to_string()
                } else if unread > 0 {
                    format!("Notifs {unread}")
                } else {
                    "Notifs".to_string()
                }
            }
        };

        let text_a = if active {
            1.0
        } else if hovered {
            0.92
        } else {
            0.68
        };
        draw_text_centered_v(
            cr,
            &label,
            x,
            y,
            w,
            h,
            10.8,
            true,
            pango::Alignment::Center,
            (1.0, 1.0, 1.0, text_a * alpha),
        );
    }
}

fn draw_timer_panel(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    timer: &PomodoroTimer,
    hovered: Option<HitTarget>,
    alpha: f64,
) {
    draw_text(
        cr,
        &timer.formatted(),
        ox + 22.0,
        oy + 44.0,
        180.0,
        34.0,
        true,
        pango::Alignment::Left,
        (1.0, 1.0, 1.0, alpha),
    );

    let (status, color) = if timer.running {
        (
            "Focus • Running",
            (ACCENT_RED.0, ACCENT_RED.1, ACCENT_RED.2, 0.95 * alpha),
        )
    } else if timer.completed {
        ("Focus • Complete!", (34.0 / 255.0, 197.0 / 255.0, 94.0 / 255.0, alpha))
    } else if timer.remaining_secs < timer.total_secs {
        ("Focus • Paused", (1.0, 1.0, 1.0, 0.72 * alpha))
    } else {
        ("Focus • Ready", (1.0, 1.0, 1.0, 0.68 * alpha))
    };
    draw_text(
        cr,
        status,
        ox + 22.0,
        oy + 88.0,
        180.0,
        11.5,
        true,
        pango::Alignment::Left,
        color,
    );

    let toggle_lbl = if timer.running { "Pause" } else { "Start" };
    let toggle_hov = hovered == Some(HitTarget::TimerToggle);
    rounded_rect_path(
        cr,
        ox + TIMER_TOGGLE_BTN.left as f64,
        oy + TIMER_TOGGLE_BTN.top as f64,
        TIMER_TOGGLE_BTN.width() as f64,
        TIMER_TOGGLE_BTN.height() as f64,
        15.0,
    );
    cr.set_source_rgba(
        ACCENT_RED.0,
        ACCENT_RED.1,
        ACCENT_RED.2,
        if toggle_hov { 0.96 } else { 0.80 } * alpha,
    );
    let _ = cr.fill();
    draw_text_centered_v(
        cr,
        toggle_lbl,
        ox + TIMER_TOGGLE_BTN.left as f64,
        oy + TIMER_TOGGLE_BTN.top as f64,
        TIMER_TOGGLE_BTN.width() as f64,
        TIMER_TOGGLE_BTN.height() as f64,
        12.0,
        true,
        pango::Alignment::Center,
        (1.0, 1.0, 1.0, alpha),
    );

    draw_action_pill(
        cr,
        ox,
        oy,
        TIMER_RESET_BTN,
        "Reset",
        hovered == Some(HitTarget::TimerReset),
        alpha,
    );

    for (i, &secs) in TIMER_PRESETS_SECS.iter().enumerate() {
        let rect = timer_preset_rect(i);
        let selected = timer.total_secs == secs;
        let is_hov = hovered == Some(HitTarget::TimerPreset(secs));
        let lbl = format!("{}m", secs / 60);

        rounded_rect_path(
            cr,
            ox + rect.left as f64,
            oy + rect.top as f64,
            rect.width() as f64,
            rect.height() as f64,
            12.0,
        );
        cr.set_source_rgba(
            1.0,
            1.0,
            1.0,
            if selected {
                0.20
            } else if is_hov {
                0.15
            } else {
                0.07
            } * alpha,
        );
        let _ = cr.fill();

        draw_text_centered_v(
            cr,
            &lbl,
            ox + rect.left as f64,
            oy + rect.top as f64,
            rect.width() as f64,
            rect.height() as f64,
            11.5,
            true,
            pango::Alignment::Center,
            (1.0, 1.0, 1.0, if selected || is_hov { 1.0 } else { 0.72 } * alpha),
        );
    }

    let bar_h = TIMER_PROGRESS_BAR.height() as f64;
    rounded_rect_path(
        cr,
        ox + TIMER_PROGRESS_BAR.left as f64,
        oy + TIMER_PROGRESS_BAR.top as f64,
        TIMER_PROGRESS_BAR.width() as f64,
        bar_h,
        bar_h / 2.0,
    );
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * alpha);
    let _ = cr.fill();

    let fill_w = TIMER_PROGRESS_BAR.width() as f64 * timer.progress() as f64;
    if fill_w > 1.0 {
        rounded_rect_path(
            cr,
            ox + TIMER_PROGRESS_BAR.left as f64,
            oy + TIMER_PROGRESS_BAR.top as f64,
            fill_w,
            bar_h,
            bar_h / 2.0,
        );
        cr.set_source_rgba(ACCENT_RED.0, ACCENT_RED.1, ACCENT_RED.2, 0.92 * alpha);
        let _ = cr.fill();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_clipboard_panel(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    pill_w: f64,
    items: &[String],
    copied_flash: bool,
    hovered: Option<HitTarget>,
    alpha: f64,
) {
    let (header, color) = if copied_flash {
        (
            "Copied to clipboard!".to_string(),
            (34.0 / 255.0, 197.0 / 255.0, 94.0 / 255.0, alpha),
        )
    } else {
        (
            format!("Clipboard History ({})", items.len()),
            (1.0, 1.0, 1.0, 0.82 * alpha),
        )
    };
    draw_text(
        cr,
        &header,
        ox + 20.0,
        oy + 41.0,
        260.0,
        12.0,
        true,
        pango::Alignment::Left,
        color,
    );

    if !items.is_empty() {
        draw_action_pill(
            cr,
            ox,
            oy,
            CLIP_CLEAR_BTN,
            "Clear",
            hovered == Some(HitTarget::ClipClear),
            alpha,
        );
    }

    if items.is_empty() {
        draw_text(
            cr,
            "Clipboard history is empty",
            ox + 16.0,
            oy + 82.0,
            pill_w - 32.0,
            13.5,
            true,
            pango::Alignment::Center,
            (1.0, 1.0, 1.0, 0.88 * alpha),
        );
        draw_text(
            cr,
            "Copy any text (Ctrl+C) to pin it here",
            ox + 16.0,
            oy + 106.0,
            pill_w - 32.0,
            11.5,
            false,
            pango::Alignment::Center,
            (1.0, 1.0, 1.0, 0.65 * alpha),
        );
        return;
    }

    for (i, text) in items.iter().take(CLIP_MAX_VISIBLE).enumerate() {
        let rect = clip_row_rect(i);
        let is_hov = hovered == Some(HitTarget::ClipCopy(i));

        rounded_rect_path(
            cr,
            ox + rect.left as f64,
            oy + rect.top as f64,
            rect.width() as f64,
            rect.height() as f64,
            8.0,
        );
        cr.set_source_rgba(1.0, 1.0, 1.0, if is_hov { 0.14 } else { 0.06 } * alpha);
        let _ = cr.fill();

        let idx_str = format!("#{}", i + 1);
        draw_text_centered_v(
            cr,
            &idx_str,
            ox + rect.left as f64 + 6.0,
            oy + rect.top as f64,
            24.0,
            rect.height() as f64,
            11.0,
            true,
            pango::Alignment::Center,
            (ACCENT_RED.0, ACCENT_RED.1, ACCENT_RED.2, 0.92 * alpha),
        );

        let single_line = text.replace(['\r', '\n', '\t'], " ");
        draw_text_centered_v(
            cr,
            &single_line,
            ox + rect.left as f64 + 34.0,
            oy + rect.top as f64,
            rect.width() as f64 - 42.0,
            rect.height() as f64,
            11.5,
            false,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, if is_hov { 1.0 } else { 0.86 } * alpha),
        );
    }
}

fn draw_sysmon_panel(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    pill_w: f64,
    stats: &SystemStats,
    alpha: f64,
) {
    let pad_x = 22.0;
    let bar_w = (pill_w - pad_x * 2.0).max(80.0);

    let ram_val = if stats.ram_total_gb > 0.1 {
        format!(
            "{:.1} / {:.1} GB  ({:.0}%)",
            stats.ram_used_gb, stats.ram_total_gb, stats.ram_percent
        )
    } else {
        format!("{:.0}%", stats.ram_percent)
    };
    let power_lbl = if stats.has_battery { "Battery" } else { "Power" };
    let power_val = if stats.has_battery {
        if stats.is_charging {
            format!("{:.0}%  •  Charging", stats.battery_percent)
        } else {
            format!("{:.0}%", stats.battery_percent)
        }
    } else {
        "AC Power".to_string()
    };

    let rows: [(&str, String, f64, bool); 3] = [
        (
            "CPU",
            format!("{:.0}%", stats.cpu_percent),
            (stats.cpu_percent / 100.0) as f64,
            false,
        ),
        ("Memory", ram_val, (stats.ram_percent / 100.0) as f64, false),
        (
            power_lbl,
            power_val,
            (stats.battery_percent / 100.0) as f64,
            true,
        ),
    ];

    for (i, (label, value, frac, is_battery)) in rows.iter().enumerate() {
        let y = oy + 46.0 + i as f64 * 36.0;
        draw_text(
            cr,
            label,
            ox + pad_x,
            y,
            bar_w,
            12.0,
            true,
            pango::Alignment::Left,
            (1.0, 1.0, 1.0, 0.90 * alpha),
        );
        draw_text(
            cr,
            value,
            ox + pad_x,
            y,
            bar_w,
            11.5,
            true,
            pango::Alignment::Right,
            (1.0, 1.0, 1.0, 0.75 * alpha),
        );

        let bar_y = y + 19.0;
        let bar_h = 7.0;
        rounded_rect_path(cr, ox + pad_x, bar_y, bar_w, bar_h, bar_h / 2.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * alpha);
        let _ = cr.fill();

        let clamped = frac.clamp(0.0, 1.0);
        let fill_w = (bar_w * clamped).max(if clamped > 0.005 { bar_h } else { 0.0 });
        if fill_w > 0.0 {
            let (r, g, b) = metric_bar_color(clamped, *is_battery);
            rounded_rect_path(cr, ox + pad_x, bar_y, fill_w, bar_h, bar_h / 2.0);
            cr.set_source_rgba(r, g, b, 0.92 * alpha);
            let _ = cr.fill();
        }
    }
}

fn metric_bar_color(frac: f64, is_battery: bool) -> (f64, f64, f64) {
    if is_battery {
        if frac < 0.20 {
            (239.0 / 255.0, 68.0 / 255.0, 68.0 / 255.0)
        } else if frac < 0.40 {
            (245.0 / 255.0, 158.0 / 255.0, 11.0 / 255.0)
        } else {
            (34.0 / 255.0, 197.0 / 255.0, 94.0 / 255.0)
        }
    } else if frac > 0.85 {
        ACCENT_RED
    } else if frac > 0.60 {
        (245.0 / 255.0, 158.0 / 255.0, 11.0 / 255.0)
    } else {
        (34.0 / 255.0, 197.0 / 255.0, 94.0 / 255.0)
    }
}

fn current_clock_string() -> String {
    Local::now().format("%H:%M").to_string()
}

#[allow(clippy::too_many_arguments)]
fn draw_equalizer(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    center_y: f64,
    levels: &[f32; EQUALIZER_BAR_COUNT],
    is_playing: bool,
    alpha: f64,
    right_edge: f64,
    bar_width: f64,
    bar_gap: f64,
    min_height: f64,
    max_delta: f64,
) {
    let total_w = EQUALIZER_BAR_COUNT as f64 * bar_width
        + (EQUALIZER_BAR_COUNT as f64 - 1.0) * bar_gap;
    let start_x = right_edge - total_w;
    let bar_alpha = if is_playing { alpha } else { alpha * 0.45 };

    for index in 0..EQUALIZER_BAR_COUNT {
        let strength = if is_playing {
            levels[index].clamp(0.0, 1.0) as f64
        } else {
            PAUSED_EQUALIZER[index] as f64
        };
        let h = min_height + strength * max_delta;
        let x = ox + start_x + index as f64 * (bar_width + bar_gap);
        let y = oy + center_y - h / 2.0;

        rounded_rect_path(cr, x, y, bar_width, h, bar_width / 2.0);
        if index % 2 == 0 {
            cr.set_source_rgba(ACCENT_RED.0, ACCENT_RED.1, ACCENT_RED.2, bar_alpha);
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.90 * bar_alpha);
        }
        let _ = cr.fill();
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_collapsed_notification_badge(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    pill_w: f64,
    pill_h: f64,
    app_name: &str,
    count: usize,
    alpha: f64,
) {
    let cy = oy + pill_h / 2.0;
    let count_text = if count > 9 {
        "9+".to_string()
    } else {
        count.to_string()
    };
    let count_w = if count > 9 { 24.0 } else { 20.0 };
    let count_right = ox + pill_w - 9.0;
    let count_left = count_right - count_w;

    rounded_rect_path(cr, count_left, cy - 10.0, count_w, 20.0, 10.0);
    cr.set_source_rgba(BADGE_RED.0, BADGE_RED.1, BADGE_RED.2, alpha);
    let _ = cr.fill();

    draw_text_centered_v(
        cr,
        &count_text,
        count_left,
        cy - 10.0,
        count_w,
        20.0,
        11.0,
        true,
        pango::Alignment::Center,
        (1.0, 1.0, 1.0, alpha),
    );

    let avatar_cx = count_left - 14.0;
    let (r, g, b) = app_accent_rgb(app_name);
    cr.arc(avatar_cx, cy, 10.0, 0.0, 2.0 * PI);
    cr.set_source_rgba(r, g, b, alpha);
    let _ = cr.fill();

    draw_text_centered_v(
        cr,
        &app_initial(app_name),
        avatar_cx - 10.0,
        cy - 10.0,
        20.0,
        20.0,
        11.0,
        true,
        pango::Alignment::Center,
        (1.0, 1.0, 1.0, alpha),
    );
}

fn draw_controls(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    alpha: f64,
    is_playing: bool,
    hovered: Option<MediaControl>,
) {
    if let Some(ctrl) = hovered {
        let r = control_rect(ctrl);
        let (cx, cy) = r.center();
        cr.arc(ox + cx as f64, oy + cy as f64, 15.5, 0.0, 2.0 * PI);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.14 * alpha);
        let _ = cr.fill();
    }

    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92 * alpha);

    let prev = control_rect(MediaControl::Previous);
    let pl = ox + prev.left as f64;
    let pt = oy + prev.top as f64;
    cr.rectangle(pl + 4.0, pt + 8.5, 2.2, 13.0);
    let _ = cr.fill();
    cr.move_to(pl + 21.0, pt + 8.5);
    cr.line_to(pl + 21.0, pt + 21.5);
    cr.line_to(pl + 8.0, pt + 15.0);
    cr.close_path();
    let _ = cr.fill();

    let pp = control_rect(MediaControl::PlayPause);
    let cl = ox + pp.left as f64;
    let ct = oy + pp.top as f64;
    if is_playing {
        rounded_rect_path(cr, cl + 10.5, ct + 9.0, 3.0, 12.0, 1.5);
        let _ = cr.fill();
        rounded_rect_path(cr, cl + 16.5, ct + 9.0, 3.0, 12.0, 1.5);
        let _ = cr.fill();
    } else {
        cr.move_to(cl + 11.0, ct + 8.0);
        cr.line_to(cl + 11.0, ct + 22.0);
        cr.line_to(cl + 22.5, ct + 15.0);
        cr.close_path();
        let _ = cr.fill();
    }

    let next = control_rect(MediaControl::Next);
    let nl = ox + next.left as f64;
    let nt = oy + next.top as f64;
    cr.move_to(nl + 9.0, nt + 8.5);
    cr.line_to(nl + 9.0, nt + 21.5);
    cr.line_to(nl + 22.0, nt + 15.0);
    cr.close_path();
    let _ = cr.fill();
    cr.rectangle(nl + 24.5, nt + 8.5, 2.2, 13.0);
    let _ = cr.fill();
}

#[allow(clippy::too_many_arguments)]
fn draw_tinted_pill(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    rect: RectF,
    label: &str,
    rgb: (f64, f64, f64),
    hovered: bool,
    active: bool,
    alpha: f64,
) {
    let x = ox + rect.left as f64;
    let y = oy + rect.top as f64;
    let w = rect.width() as f64;
    let h = rect.height() as f64;

    rounded_rect_path(cr, x, y, w, h, h / 2.0);
    let fill_a = if active {
        if hovered { 0.42 } else { 0.30 }
    } else if hovered {
        0.22
    } else {
        0.13
    };
    cr.set_source_rgba(rgb.0, rgb.1, rgb.2, fill_a * alpha);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(rgb.0, rgb.1, rgb.2, if active || hovered { 0.78 } else { 0.40 } * alpha);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    draw_text_centered_v(
        cr,
        label,
        x,
        y,
        w,
        h,
        10.5,
        true,
        pango::Alignment::Center,
        (1.0, 1.0, 1.0, if active || hovered { 1.0 } else { 0.88 } * alpha),
    );
}

fn draw_action_pill(
    cr: &cairo::Context,
    ox: f64,
    oy: f64,
    rect: RectF,
    label: &str,
    hovered: bool,
    alpha: f64,
) {
    let x = ox + rect.left as f64;
    let y = oy + rect.top as f64;
    let w = rect.width() as f64;
    let h = rect.height() as f64;

    rounded_rect_path(cr, x, y, w, h, h / 2.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, if hovered { 0.20 } else { 0.10 } * alpha);
    let _ = cr.fill();

    draw_text_centered_v(
        cr,
        label,
        x,
        y,
        w,
        h,
        10.5,
        true,
        pango::Alignment::Center,
        (1.0, 1.0, 1.0, if hovered { 1.0 } else { 0.82 } * alpha),
    );
}

fn draw_dismiss_x(cr: &cairo::Context, ox: f64, oy: f64, rect: RectF, hovered: bool, alpha: f64) {
    let (cx, cy) = rect.center();
    let x = ox + cx as f64;
    let y = oy + cy as f64;

    if hovered {
        cr.arc(x, y, 10.5, 0.0, 2.0 * PI);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.16 * alpha);
        let _ = cr.fill();
    }

    let r = 3.8;
    cr.set_source_rgba(1.0, 1.0, 1.0, if hovered { 0.92 } else { 0.45 } * alpha);
    cr.set_line_width(1.6);
    cr.move_to(x - r, y - r);
    cr.line_to(x + r, y + r);
    cr.move_to(x + r, y - r);
    cr.line_to(x - r, y + r);
    let _ = cr.stroke();
}

fn rounded_rect_path(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    cr: &cairo::Context,
    text: &str,
    x: f64,
    y: f64,
    max_w: f64,
    size_px: f64,
    bold: bool,
    align: pango::Alignment,
    rgba: (f64, f64, f64, f64),
) {
    let layout = pangocairo::functions::create_layout(cr);
    let mut desc = pango::FontDescription::from_string(if bold { "Sans Bold" } else { "Sans" });
    desc.set_absolute_size(size_px * pango::SCALE as f64);
    layout.set_font_description(Some(&desc));
    layout.set_width((max_w.max(10.0) * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_alignment(align);
    layout.set_text(text);

    cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, &layout);
}

#[allow(clippy::too_many_arguments)]
fn draw_text_centered_v(
    cr: &cairo::Context,
    text: &str,
    x: f64,
    y: f64,
    max_w: f64,
    box_h: f64,
    size_px: f64,
    bold: bool,
    align: pango::Alignment,
    rgba: (f64, f64, f64, f64),
) {
    let layout = pangocairo::functions::create_layout(cr);
    let mut desc = pango::FontDescription::from_string(if bold { "Sans Bold" } else { "Sans" });
    desc.set_absolute_size(size_px * pango::SCALE as f64);
    layout.set_font_description(Some(&desc));
    layout.set_width((max_w.max(10.0) * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_alignment(align);
    layout.set_text(text);

    let (_, text_h) = layout.pixel_size();
    let dy = y + (box_h - text_h as f64) / 2.0;

    cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
    cr.move_to(x, dy);
    pangocairo::functions::show_layout(cr, &layout);
}
