//! Shared widget geometry, 5-tab navigation bar, Pillar notifications,
//! Pomodoro timer, Clipboard history, System Monitor, and hit-targets.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::render::spring::{
    COLLAPSED_HEIGHT, EXPANDED_HEIGHT, EXPANDED_WIDTH, NOTIFICATION_HEIGHT, NOTIFICATION_WIDTH,
};
use crate::render::zeki::{ZekiState, ALL_ZEKI_STATES};
use crate::services::media::MediaControl;
use crate::services::notifications::NotificationSnapshot;
use crate::services::pomodoro::TIMER_PRESETS_SECS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn center(&self) -> (f32, f32) {
        (
            (self.left + self.right) / 2.0,
            (self.top + self.bottom) / 2.0,
        )
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// Seven tabs in the expanded island header (`Zeki` + `Live` Cicada + 5 productivity tools).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpandedTab {
    Zeki,
    Cicada,
    Media,
    Timer,
    Clipboard,
    System,
    Notifications,
}

pub const ALL_TABS: [ExpandedTab; 7] = [
    ExpandedTab::Zeki,
    ExpandedTab::Cicada,
    ExpandedTab::Media,
    ExpandedTab::Timer,
    ExpandedTab::Clipboard,
    ExpandedTab::System,
    ExpandedTab::Notifications,
];

/// Interactive hit targets inside the pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    SelectTab(ExpandedTab),
    ZekiPoke,
    ZekiSelectState(ZekiState),
    ZekiOpenChat,
    ZekiMicToggle,
    ZekiTogglePrivacy,
    ZekiOpenConfig,
    ZekiAttachFile,
    ZekiRemoveFile,
    ZekiInsertReply,
    ZekiCopyReply,
    ZekiClearChat,
    ZekiChatCard,
    ZekiChatToggleSize,
    ZekiChatInput,
    ZekiChatSend,
    // Cicada Mode (Live Meeting Translator)
    CicadaToggleListen,
    CicadaToggleMic,
    CicadaQuickReply,
    CicadaToggleSubtitleBar,
    CicadaToggleClickThrough,
    CicadaCycleLang,
    CicadaToggleOriginal,
    CicadaCopyTranscript,
    CicadaSummarize,
    CicadaAskZeki,
    CicadaClear,
    CicadaCard,
    CicadaToggleSize,
    CicadaCopyQuickReply,
    CicadaExpandFromBar,
    MediaControl(MediaControl),
    ToastActivate(u32),
    ToastDismiss(u32),
    ClearAllNotifications,
    OpenNotificationSettings,
    NotifActivate(u32),
    NotifDismiss(u32),
    TimerToggle,
    TimerReset,
    TimerPreset(u32),
    ClipCopy(usize),
    ClipClear,
}

// -----------------------------------------------------------------------------
// Top 7-tab bar (y = 11..33, responsive across 440px..640px expanded pill)
// -----------------------------------------------------------------------------

pub fn tab_rect_for_width(tab: ExpandedTab, pill_w: f32) -> RectF {
    let idx = match tab {
        ExpandedTab::Zeki => 0.0,
        ExpandedTab::Cicada => 1.0,
        ExpandedTab::Media => 2.0,
        ExpandedTab::Timer => 3.0,
        ExpandedTab::Clipboard => 4.0,
        ExpandedTab::System => 5.0,
        ExpandedTab::Notifications => 6.0,
    };
    let pad = 14.0;
    let gap = 4.0;
    let w = pill_w.max(EXPANDED_WIDTH);
    let tab_w = (w - pad * 2.0 - gap * 6.0) / 7.0;
    let left = pad + idx * (tab_w + gap);
    RectF::new(left, 11.0, left + tab_w, 33.0)
}

// -----------------------------------------------------------------------------
// Cicada Mode Layouts (Live Subtitle Bar 620x94 & Expanded Transcript 560x340+)
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct CicadaBarLayout {
    pub zeki_center: (f32, f32),
    pub text_rect: RectF,
    pub reply_btn: RectF,
    pub pass_btn: RectF,
    pub expand_btn: RectF,
    pub stop_btn: RectF,
}

pub fn cicada_bar_layout(pill_w: f32, pill_h: f32) -> CicadaBarLayout {
    let w = pill_w.max(360.0);
    let h = pill_h.max(76.0);
    let right = w - 12.0;
    let stop_btn = RectF::new(right - 44.0, 10.0, right, 30.0);
    let expand_btn = RectF::new(stop_btn.left - 48.0, 10.0, stop_btn.left - 4.0, 30.0);
    let pass_btn = RectF::new(expand_btn.left - 48.0, 10.0, expand_btn.left - 4.0, 30.0);
    let reply_btn = RectF::new(pass_btn.left - 54.0, 10.0, pass_btn.left - 4.0, 30.0);
    let text_rect = RectF::new(66.0, 32.0, w - 14.0, h - 8.0);
    CicadaBarLayout {
        zeki_center: (34.0, h / 2.0 + 2.0),
        text_rect,
        reply_btn,
        pass_btn,
        expand_btn,
        stop_btn,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CicadaPanelLayout {
    pub zeki_stage: RectF,
    pub listen_btn: RectF,
    pub mic_btn: RectF,
    pub reply_btn: RectF,
    pub sub_bar_btn: RectF,
    pub lang_btn: RectF,
    pub orig_btn: RectF,
    pub size_btn: RectF,
    pub card: RectF,
    pub copy_btn: RectF,
    pub summary_btn: RectF,
    pub ask_btn: RectF,
    pub clear_btn: RectF,
}

pub fn cicada_panel_layout(pill_w: f32, pill_h: f32) -> CicadaPanelLayout {
    let w = pill_w.max(440.0);
    let h = pill_h.max(240.0);
    let right = w - 16.0;

    // Left compact Zeki taking-notes column
    let zeki_stage = RectF::new(16.0, 44.0, 106.0, 134.0);
    let listen_btn = RectF::new(16.0, 142.0, 106.0, 172.0);
    let mic_btn = RectF::new(16.0, 178.0, 106.0, 206.0);
    let reply_btn = RectF::new(16.0, 212.0, 106.0, 240.0);
    let sub_bar_btn = RectF::new(16.0, 246.0, 106.0, 274.0);

    // Top right utility chips
    let size_btn = RectF::new(right - 46.0, 44.0, right, 66.0);
    let orig_btn = RectF::new(size_btn.left - 56.0, 44.0, size_btn.left - 5.0, 66.0);
    let lang_btn = RectF::new(orig_btn.left - 62.0, 44.0, orig_btn.left - 5.0, 66.0);

    // Bottom action bar inside right column
    let bot_bottom = h - 12.0;
    let bot_top = bot_bottom - 28.0;
    let col_left = 118.0;
    let col_w = (right - col_left).max(200.0);
    let gap = 6.0;
    let clear_w = 54.0;
    let rem_w = (col_w - clear_w - gap * 3.0) / 3.0;
    let copy_btn = RectF::new(col_left, bot_top, col_left + rem_w, bot_bottom);
    let summary_btn = RectF::new(copy_btn.right + gap, bot_top, copy_btn.right + gap + rem_w, bot_bottom);
    let ask_btn = RectF::new(summary_btn.right + gap, bot_top, summary_btn.right + gap + rem_w, bot_bottom);
    let clear_btn = RectF::new(ask_btn.right + gap, bot_top, right, bot_bottom);

    let card = RectF::new(col_left, 72.0, right, (bot_top - 8.0).max(130.0));

    CicadaPanelLayout {
        zeki_stage,
        listen_btn,
        mic_btn,
        reply_btn,
        sub_bar_btn,
        lang_btn,
        orig_btn,
        size_btn,
        card,
        copy_btn,
        summary_btn,
        ask_btn,
        clear_btn,
    }
}

// -----------------------------------------------------------------------------
// Zeki Assistant tab layout (preview 440x198, Chat 520x310, Large Chat 640x470)
// -----------------------------------------------------------------------------

pub const ZEKI_STAGE_RECT: RectF = RectF::new(18.0, 46.0, 118.0, 146.0);

// Buttons in default (preview) Zeki view (440 x 198)
pub const ZEKI_CHAT_BTN_PREVIEW: RectF = RectF::new(214.0, 45.0, 280.0, 67.0);
pub const ZEKI_MIC_BTN_PREVIEW: RectF = RectF::new(286.0, 45.0, 352.0, 67.0);
pub const ZEKI_CFG_BTN_PREVIEW: RectF = RectF::new(358.0, 45.0, 422.0, 67.0);
pub const ZEKI_PRIVACY_BTN_PREVIEW: RectF = RectF::new(18.0, 154.0, 118.0, 182.0);

// Left-column buttons in Chat mode
pub const ZEKI_MIC_BTN_CHAT: RectF = RectF::new(18.0, 154.0, 118.0, 186.0);
pub const ZEKI_CFG_BTN_CHAT: RectF = RectF::new(18.0, 192.0, 118.0, 224.0);
pub const ZEKI_PRIVACY_BTN_CHAT: RectF = RectF::new(18.0, 230.0, 118.0, 258.0);

/// Responsive right-column layout for Chat mode (adapts smoothly to 520x310 & 640x470).
#[derive(Clone, Copy, Debug)]
pub struct ChatLayout {
    pub expand_btn: RectF,
    pub insert_btn: RectF,
    pub copy_btn: RectF,
    pub clear_btn: RectF,
    pub card: RectF,
    pub file_chip: RectF,
    pub attach_btn: RectF,
    pub input: RectF,
    pub send_btn: RectF,
}

pub fn chat_layout(pill_w: f32, pill_h: f32, has_file: bool) -> ChatLayout {
    let right = (pill_w - 18.0).max(360.0);
    let clear_btn = RectF::new(right - 46.0, 45.0, right, 66.0);
    let copy_btn = RectF::new(clear_btn.left - 50.0, 45.0, clear_btn.left - 4.0, 66.0);
    let insert_btn = RectF::new(copy_btn.left - 54.0, 45.0, copy_btn.left - 4.0, 66.0);
    let expand_btn = RectF::new(insert_btn.left - 56.0, 45.0, insert_btn.left - 4.0, 66.0);

    let input_bottom = (pill_h - 12.0).max(188.0);
    let input_top = input_bottom - 32.0;
    let attach_btn = RectF::new(132.0, input_top, 182.0, input_bottom);
    let send_btn = RectF::new(right - 56.0, input_top, right, input_bottom);
    let input = RectF::new(attach_btn.right + 6.0, input_top, send_btn.left - 6.0, input_bottom);

    let file_chip = RectF::new(132.0, input_top - 24.0, right, input_top - 4.0);
    let card_bottom = if has_file {
        (file_chip.top - 6.0).max(116.0)
    } else {
        (input_top - 8.0).max(124.0)
    };
    let card = RectF::new(132.0, 72.0, right, card_bottom);

    ChatLayout {
        expand_btn,
        insert_btn,
        copy_btn,
        clear_btn,
        card,
        file_chip,
        attach_btn,
        input,
        send_btn,
    }
}

// Persistent Zeki Companion Dock shown at the bottom of all non-Zeki sub-menus
pub const ZEKI_DOCK_RECT: RectF = RectF::new(16.0, 156.0, 424.0, 190.0);
pub const ZEKI_DOCK_POKE_RECT: RectF = RectF::new(16.0, 156.0, 56.0, 190.0);
pub const ZEKI_DOCK_PROMPT_RECT: RectF = RectF::new(56.0, 156.0, 214.0, 190.0);
pub const ZEKI_DOCK_PRIVACY_BTN: RectF = RectF::new(218.0, 161.0, 274.0, 185.0);
pub const ZEKI_DOCK_CHAT_BTN: RectF = RectF::new(280.0, 161.0, 348.0, 185.0);
pub const ZEKI_DOCK_MIC_BTN: RectF = RectF::new(354.0, 161.0, 420.0, 185.0);

pub fn zeki_state_btn_rect(index: usize) -> RectF {
    let start_x = 132.0;
    let total_w = EXPANDED_WIDTH - start_x - 18.0; // 290.0
    let gap = 6.0;
    if index < 4 {
        let w = (total_w - gap * 3.0) / 4.0;
        let left = start_x + index as f32 * (w + gap);
        RectF::new(left, 102.0, left + w, 128.0)
    } else {
        let col = index - 4;
        let w = (total_w - gap * 2.0) / 3.0;
        let left = start_x + col as f32 * (w + gap);
        RectF::new(left, 134.0, left + w, 160.0)
    }
}

// -----------------------------------------------------------------------------
// Media tab layout (logo on top-left directly above the clock, controls on right)
// -----------------------------------------------------------------------------

pub const COLLAPSED_ART: RectF = RectF::new(10.0, 7.0, 30.0, 27.0);
pub const EXPANDED_ART: RectF = RectF::new(22.0, 44.0, 94.0, 116.0);
pub const MEDIA_CLOCK_BOX: RectF = RectF::new(18.0, 120.0, 98.0, 146.0);

pub const TITLE_BOX: RectF = RectF::new(110.0, 46.0, 422.0, 74.0);
pub const ARTIST_BOX: RectF = RectF::new(110.0, 74.0, 422.0, 96.0);

pub const CONTROL_SIZE: f32 = 30.0;
pub const CONTROL_GAP: f32 = 8.0;
pub const CONTROLS_TOP: f32 = 106.0;

pub const EQUALIZER_BAR_COUNT: usize = 5;
pub const EQUALIZER_BAR_WIDTH: f32 = 3.0;
pub const EQUALIZER_BAR_GAP: f32 = 2.5;
pub const EQUALIZER_RIGHT: f32 = 164.0;

// -----------------------------------------------------------------------------
// Incoming notification toast layout (380 x 88)
// -----------------------------------------------------------------------------

pub const TOAST_AVATAR: RectF = RectF::new(16.0, 22.0, 52.0, 58.0);
pub const TOAST_DISMISS_BTN: RectF =
    RectF::new(NOTIFICATION_WIDTH - 38.0, 12.0, NOTIFICATION_WIDTH - 12.0, 38.0);
pub const TOAST_PROGRESS_RECT: RectF = RectF::new(
    22.0,
    NOTIFICATION_HEIGHT - 6.0,
    NOTIFICATION_WIDTH - 22.0,
    NOTIFICATION_HEIGHT - 3.5,
);

// -----------------------------------------------------------------------------
// Notifications tab layout (below top tab bar: y = 40..154)
// -----------------------------------------------------------------------------

pub const NOTIF_CARD_MAX_VISIBLE: usize = 2;
pub const NOTIF_CLEAR_BTN: RectF = RectF::new(EXPANDED_WIDTH - 80.0, 40.0, EXPANDED_WIDTH - 16.0, 58.0);
pub const NOTIF_SETTINGS_BTN: RectF = RectF::new(124.0, 114.0, 316.0, 144.0);

pub fn notif_card_rect(index: usize) -> RectF {
    let top = 64.0 + index as f32 * 45.0;
    RectF::new(16.0, top, EXPANDED_WIDTH - 16.0, top + 41.0)
}

pub fn notif_card_dismiss_rect(index: usize) -> RectF {
    let card = notif_card_rect(index);
    RectF::new(
        card.right - 28.0,
        card.top + 8.0,
        card.right - 6.0,
        card.top + 30.0,
    )
}

// -----------------------------------------------------------------------------
// Pomodoro Timer tab layout (y = 42..154)
// -----------------------------------------------------------------------------

pub const TIMER_TOGGLE_BTN: RectF = RectF::new(248.0, 52.0, 334.0, 82.0);
pub const TIMER_RESET_BTN: RectF = RectF::new(342.0, 52.0, 422.0, 82.0);
pub const TIMER_PROGRESS_BAR: RectF = RectF::new(20.0, 144.0, 420.0, 150.0);

pub fn timer_preset_rect(index: usize) -> RectF {
    let pad = 20.0;
    let gap = 10.0;
    let w = (EXPANDED_WIDTH - pad * 2.0 - gap * 3.0) / 4.0; // 92.5
    let left = pad + index as f32 * (w + gap);
    RectF::new(left, 110.0, left + w, 134.0)
}

// -----------------------------------------------------------------------------
// Clipboard tab layout (y = 40..154)
// -----------------------------------------------------------------------------

pub const CLIP_MAX_VISIBLE: usize = 3;
pub const CLIP_CLEAR_BTN: RectF = RectF::new(EXPANDED_WIDTH - 72.0, 40.0, EXPANDED_WIDTH - 16.0, 58.0);

pub fn clip_row_rect(index: usize) -> RectF {
    let top = 64.0 + index as f32 * 30.0;
    RectF::new(16.0, top, EXPANDED_WIDTH - 16.0, top + 26.0)
}

// -----------------------------------------------------------------------------
// Media control helpers
// -----------------------------------------------------------------------------

const CONTROLS_ACTIVE_HEIGHT: f32 = EXPANDED_HEIGHT - 12.0;

pub fn control_rect(control: MediaControl) -> RectF {
    let index = match control {
        MediaControl::Previous => 0.0,
        MediaControl::PlayPause => 1.0,
        MediaControl::Next => 2.0,
    };
    let left = TITLE_BOX.left + index * (CONTROL_SIZE + CONTROL_GAP);
    RectF::new(
        left,
        CONTROLS_TOP,
        left + CONTROL_SIZE,
        CONTROLS_TOP + CONTROL_SIZE,
    )
}

pub fn control_at(x: f32, y: f32) -> Option<MediaControl> {
    [
        MediaControl::Previous,
        MediaControl::PlayPause,
        MediaControl::Next,
    ]
    .into_iter()
    .find(|control| control_rect(*control).contains(x, y))
}

pub fn controls_active(height: f32) -> bool {
    height >= CONTROLS_ACTIVE_HEIGHT
}

pub fn expansion_progress(height: f32) -> f32 {
    ((height - COLLAPSED_HEIGHT) / (EXPANDED_HEIGHT - COLLAPSED_HEIGHT)).clamp(0.0, 1.0)
}

pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub struct HitContext<'a> {
    pub pill_width: f32,
    pub pill_height: f32,
    pub expanded: bool,
    pub expanded_tab: ExpandedTab,
    pub has_media: bool,
    pub hud_visible: bool,
    pub toast_visible: bool,
    pub notifications: &'a NotificationSnapshot,
    pub clip_count: usize,
    pub assistant_chat_open: bool,
    pub assistant_has_reply: bool,
    pub assistant_has_error: bool,
    pub assistant_has_file: bool,
    pub cicada_subtitle_bar: bool,
    pub cicada_has_entries: bool,
    pub cicada_has_quick_reply: bool,
}

pub fn hit_target_at(x: f32, y: f32, ctx: &HitContext) -> Option<HitTarget> {
    if ctx.hud_visible {
        return None;
    }

    if ctx.toast_visible && !ctx.expanded {
        if let Some(toast) = &ctx.notifications.toast {
            if TOAST_DISMISS_BTN.contains(x, y) {
                return Some(HitTarget::ToastDismiss(toast.id));
            }
            return Some(HitTarget::ToastActivate(toast.id));
        }
        return None;
    }

    // Live Subtitle Bar controls when active and not expanded
    if ctx.cicada_subtitle_bar && !ctx.expanded {
        let bl = cicada_bar_layout(ctx.pill_width, ctx.pill_height);
        if bl.stop_btn.contains(x, y) {
            return Some(HitTarget::CicadaToggleListen);
        }
        if bl.expand_btn.contains(x, y) {
            return Some(HitTarget::CicadaExpandFromBar);
        }
        if bl.pass_btn.contains(x, y) {
            return Some(HitTarget::CicadaToggleClickThrough);
        }
        if bl.reply_btn.contains(x, y) {
            return Some(HitTarget::CicadaQuickReply);
        }
        return Some(HitTarget::CicadaExpandFromBar);
    }

    if !ctx.expanded || !controls_active(ctx.pill_height) {
        return None;
    }

    // Top 7-tab bar works from any expanded tab.
    for tab in ALL_TABS {
        if tab_rect_for_width(tab, ctx.pill_width).contains(x, y) {
            return Some(HitTarget::SelectTab(tab));
        }
    }

    // Persistent Zeki Companion Dock works across compact non-Zeki/non-Cicada sub-menus
    if !matches!(ctx.expanded_tab, ExpandedTab::Zeki | ExpandedTab::Cicada) {
        if ZEKI_DOCK_POKE_RECT.contains(x, y) {
            return Some(HitTarget::ZekiPoke);
        }
        if ZEKI_DOCK_PRIVACY_BTN.contains(x, y) {
            return Some(HitTarget::ZekiTogglePrivacy);
        }
        if ZEKI_DOCK_CHAT_BTN.contains(x, y) || ZEKI_DOCK_PROMPT_RECT.contains(x, y) {
            return Some(HitTarget::ZekiOpenChat);
        }
        if ZEKI_DOCK_MIC_BTN.contains(x, y) {
            return Some(HitTarget::ZekiMicToggle);
        }
    }

    match ctx.expanded_tab {
        ExpandedTab::Cicada => {
            let cl = cicada_panel_layout(ctx.pill_width, ctx.pill_height);
            if cl.zeki_stage.contains(x, y) {
                return Some(HitTarget::ZekiPoke);
            }
            if cl.listen_btn.contains(x, y) {
                return Some(HitTarget::CicadaToggleListen);
            }
            if cl.mic_btn.contains(x, y) {
                return Some(HitTarget::CicadaToggleMic);
            }
            if cl.reply_btn.contains(x, y) {
                return Some(HitTarget::CicadaQuickReply);
            }
            if cl.sub_bar_btn.contains(x, y) {
                return Some(HitTarget::CicadaToggleSubtitleBar);
            }
            if cl.lang_btn.contains(x, y) {
                return Some(HitTarget::CicadaCycleLang);
            }
            if cl.orig_btn.contains(x, y) {
                return Some(HitTarget::CicadaToggleOriginal);
            }
            if cl.size_btn.contains(x, y) {
                return Some(HitTarget::CicadaToggleSize);
            }
            if cl.copy_btn.contains(x, y) && ctx.cicada_has_entries {
                return Some(HitTarget::CicadaCopyTranscript);
            }
            if cl.summary_btn.contains(x, y) && ctx.cicada_has_entries {
                return Some(HitTarget::CicadaSummarize);
            }
            if cl.ask_btn.contains(x, y) {
                return Some(HitTarget::CicadaAskZeki);
            }
            if cl.clear_btn.contains(x, y) && ctx.cicada_has_entries {
                return Some(HitTarget::CicadaClear);
            }
            if cl.card.contains(x, y) {
                if ctx.cicada_has_quick_reply && y <= cl.card.top + 30.0 {
                    return Some(HitTarget::CicadaCopyQuickReply);
                }
                return Some(HitTarget::CicadaCard);
            }
            None
        }
        ExpandedTab::Zeki => {
            if ZEKI_STAGE_RECT.contains(x, y) {
                return Some(HitTarget::ZekiPoke);
            }
            if ctx.assistant_chat_open {
                let cl = chat_layout(ctx.pill_width, ctx.pill_height, ctx.assistant_has_file);
                if ZEKI_MIC_BTN_CHAT.contains(x, y) {
                    return Some(HitTarget::ZekiMicToggle);
                }
                if ZEKI_CFG_BTN_CHAT.contains(x, y) {
                    return Some(HitTarget::ZekiOpenConfig);
                }
                if ZEKI_PRIVACY_BTN_CHAT.contains(x, y) {
                    return Some(HitTarget::ZekiTogglePrivacy);
                }
                if cl.attach_btn.contains(x, y) {
                    return Some(HitTarget::ZekiAttachFile);
                }
                if ctx.assistant_has_file && cl.file_chip.contains(x, y) {
                    return Some(HitTarget::ZekiRemoveFile);
                }
                if cl.input.contains(x, y) {
                    return Some(HitTarget::ZekiChatInput);
                }
                if cl.send_btn.contains(x, y) {
                    return Some(HitTarget::ZekiChatSend);
                }
                if cl.expand_btn.contains(x, y) {
                    return Some(HitTarget::ZekiChatToggleSize);
                }
                if cl.insert_btn.contains(x, y) && ctx.assistant_has_reply {
                    return Some(HitTarget::ZekiInsertReply);
                }
                if cl.clear_btn.contains(x, y) {
                    return Some(HitTarget::ZekiClearChat);
                }
                if cl.copy_btn.contains(x, y) {
                    if ctx.assistant_has_reply {
                        return Some(HitTarget::ZekiCopyReply);
                    } else if ctx.assistant_has_error {
                        return Some(HitTarget::ZekiOpenConfig);
                    }
                }
                if cl.card.contains(x, y) {
                    return Some(HitTarget::ZekiChatCard);
                }
                return None;
            }
            if ZEKI_CHAT_BTN_PREVIEW.contains(x, y) {
                return Some(HitTarget::ZekiOpenChat);
            }
            if ZEKI_MIC_BTN_PREVIEW.contains(x, y) {
                return Some(HitTarget::ZekiMicToggle);
            }
            if ZEKI_CFG_BTN_PREVIEW.contains(x, y) {
                return Some(HitTarget::ZekiOpenConfig);
            }
            if ZEKI_PRIVACY_BTN_PREVIEW.contains(x, y) {
                return Some(HitTarget::ZekiTogglePrivacy);
            }
            for (i, &state) in ALL_ZEKI_STATES.iter().enumerate() {
                if zeki_state_btn_rect(i).contains(x, y) {
                    return Some(HitTarget::ZekiSelectState(state));
                }
            }
            None
        }
        ExpandedTab::Media => {
            if ctx.has_media {
                if let Some(ctrl) = control_at(x, y) {
                    return Some(HitTarget::MediaControl(ctrl));
                }
            }
            None
        }
        ExpandedTab::Timer => {
            if TIMER_TOGGLE_BTN.contains(x, y) {
                return Some(HitTarget::TimerToggle);
            }
            if TIMER_RESET_BTN.contains(x, y) {
                return Some(HitTarget::TimerReset);
            }
            for (i, &secs) in TIMER_PRESETS_SECS.iter().enumerate() {
                if timer_preset_rect(i).contains(x, y) {
                    return Some(HitTarget::TimerPreset(secs));
                }
            }
            None
        }
        ExpandedTab::Clipboard => {
            if ctx.clip_count > 0 && CLIP_CLEAR_BTN.contains(x, y) {
                return Some(HitTarget::ClipClear);
            }
            for i in 0..ctx.clip_count.min(CLIP_MAX_VISIBLE) {
                if clip_row_rect(i).contains(x, y) {
                    return Some(HitTarget::ClipCopy(i));
                }
            }
            None
        }
        ExpandedTab::System => None,
        ExpandedTab::Notifications => {
            if !ctx.notifications.items.is_empty() && NOTIF_CLEAR_BTN.contains(x, y) {
                return Some(HitTarget::ClearAllNotifications);
            }
            if !ctx.notifications.access_granted && ctx.notifications.items.is_empty() {
                if NOTIF_SETTINGS_BTN.contains(x, y) {
                    return Some(HitTarget::OpenNotificationSettings);
                }
                return None;
            }
            for (index, item) in ctx
                .notifications
                .items
                .iter()
                .take(NOTIF_CARD_MAX_VISIBLE)
                .enumerate()
            {
                if notif_card_dismiss_rect(index).contains(x, y) {
                    return Some(HitTarget::NotifDismiss(item.id));
                }
                if notif_card_rect(index).contains(x, y) {
                    return Some(HitTarget::NotifActivate(item.id));
                }
            }
            None
        }
    }
}

pub fn app_accent_rgb(app_name: &str) -> (f32, f32, f32) {
    const PALETTE: [(f32, f32, f32); 6] = [
        (59.0 / 255.0, 130.0 / 255.0, 246.0 / 255.0),
        (34.0 / 255.0, 197.0 / 255.0, 94.0 / 255.0),
        (168.0 / 255.0, 85.0 / 255.0, 247.0 / 255.0),
        (236.0 / 255.0, 72.0 / 255.0, 153.0 / 255.0),
        (249.0 / 255.0, 115.0 / 255.0, 22.0 / 255.0),
        (6.0 / 255.0, 182.0 / 255.0, 212.0 / 255.0),
    ];

    let mut hash: i32 = 0;
    for unit in app_name.encode_utf16() {
        hash = (unit as i32).wrapping_add(hash.wrapping_shl(5).wrapping_sub(hash));
    }
    let idx = (hash.unsigned_abs() as usize) % PALETTE.len();
    PALETTE[idx]
}

pub fn app_initial(app_name: &str) -> String {
    app_name
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "N".to_string())
}

pub fn format_relative_time(timestamp_ms: u64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let diff = now_ms.saturating_sub(timestamp_ms);
    if diff < 60_000 {
        "just now".to_string()
    } else if diff < 3_600_000 {
        format!("{}m ago", diff / 60_000)
    } else if diff < 86_400_000 {
        format!("{}h ago", diff / 3_600_000)
    } else {
        format!("{}d ago", diff / 86_400_000)
    }
}
