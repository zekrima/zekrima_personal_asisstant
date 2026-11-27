//! Direct2D renderer for the layered island window.
//!
//! Renders:
//! - Collapsed pill (bold clock, Zekrima brand / album art, 5-bar real-time FFT
//!   music equalizer, live Pomodoro timer pill, Pillar notification badge)
//! - Transient Volume / Brightness HUD & Pillar incoming notification toast
//! - Unified 5-tab Expanded Island (`Media`, `Timer`, `Clip`, `Sys`, `Notifs`)

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use windows::core::{w, Error, PCWSTR, Result};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{BOOL, COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory,
    ID2D1PathGeometry, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteInlineObject,
    IDWriteTextFormat, IDWriteTextLayout, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_REGULAR,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

use crate::layout::{CANVAS_HEIGHT_LOGICAL, CANVAS_WIDTH_LOGICAL, PILL_TOP_LOGICAL};
use crate::render::spring::{AUTO_HIDDEN_HEIGHT, COLLAPSED_HEIGHT};
use crate::render::widgets::{
    app_accent_rgb, app_initial, chat_layout, cicada_bar_layout, cicada_panel_layout,
    clip_row_rect, control_rect, expansion_progress, format_relative_time,
    notif_card_dismiss_rect, notif_card_rect, smoothstep, tab_rect_for_width, timer_preset_rect,
    zeki_state_btn_rect, ExpandedTab, HitTarget, RectF, ALL_TABS, ARTIST_BOX, CLIP_CLEAR_BTN,
    CLIP_MAX_VISIBLE, COLLAPSED_ART, CONTROLS_TOP, EQUALIZER_BAR_COUNT, EQUALIZER_BAR_GAP,
    EQUALIZER_BAR_WIDTH, EQUALIZER_RIGHT, EXPANDED_ART, MEDIA_CLOCK_BOX, NOTIF_CARD_MAX_VISIBLE,
    NOTIF_CLEAR_BTN, NOTIF_SETTINGS_BTN, TIMER_PROGRESS_BAR, TIMER_RESET_BTN, TIMER_TOGGLE_BTN,
    TITLE_BOX, TOAST_AVATAR, TOAST_DISMISS_BTN, TOAST_PROGRESS_RECT, ZEKI_CFG_BTN_CHAT,
    ZEKI_CFG_BTN_PREVIEW, ZEKI_CHAT_BTN_PREVIEW, ZEKI_DOCK_CHAT_BTN, ZEKI_DOCK_MIC_BTN,
    ZEKI_DOCK_PRIVACY_BTN, ZEKI_DOCK_RECT, ZEKI_MIC_BTN_CHAT, ZEKI_MIC_BTN_PREVIEW,
    ZEKI_PRIVACY_BTN_CHAT, ZEKI_PRIVACY_BTN_PREVIEW, ZEKI_STAGE_RECT,
};
use crate::render::zeki::{create_round_stroke, ZekiEngine, ALL_ZEKI_STATES};
use crate::services::assistant::{AssistantPhase, AssistantSnapshot};
use crate::services::cicada::{CicadaSnapshot, SpeakerChannel};
use crate::services::config::cicada_lang_short_badge;
use crate::services::media::{AlbumArt, MediaControl, MediaInfo};
use crate::services::notifications::{NotificationItem, NotificationSnapshot};
use crate::services::pomodoro::{PomodoroTimer, TIMER_PRESETS_SECS};
use crate::services::sysmon::SystemStats;
use crate::services::system::{HudKind, HudState, SystemSnapshot};

const PILL_FILL: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 10.0 / 255.0,
    g: 10.0 / 255.0,
    b: 13.0 / 255.0,
    a: 0.96,
};
const PILL_BORDER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.11,
};
const PILL_ACCENT: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 235.0 / 255.0,
    g: 0.0,
    b: 40.0 / 255.0,
    a: 1.0,
};
const BADGE_RED: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 239.0 / 255.0,
    g: 68.0 / 255.0,
    b: 68.0 / 255.0,
    a: 1.0,
};
const TITLE_COLOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
const ARTIST_COLOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.65,
};
const ICON_COLOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.92,
};
const HOVER_COLOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
const EQUALIZER_ACCENT: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.88,
};
const ART_PLACEHOLDER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.07,
};

const TITLE_FONT_SIZE: f32 = 19.0;
const ARTIST_FONT_SIZE: f32 = 15.0;
const COLLAPSED_ART_RADIUS: f32 = 6.0;
const EXPANDED_ART_RADIUS: f32 = 14.0;
const HOVER_RADIUS: f32 = 16.0;
const PAUSED_EQUALIZER: [f32; EQUALIZER_BAR_COUNT] = [0.22, 0.45, 0.30, 0.50, 0.25];

pub struct Frame<'a> {
    pub manual_hidden: bool,
    pub auto_hidden: bool,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub expanded: bool,
    pub expanded_tab: ExpandedTab,
    pub zeki: &'a ZekiEngine,
    pub assistant: &'a AssistantSnapshot,
    pub voice_level: f32,
    pub chat_mode: bool,
    pub chat_large: bool,
    pub chat_input: &'a str,
    pub chat_focused: bool,
    pub chat_scroll_y: f32,
    pub chat_auto_scroll: bool,
    pub attached_file_label: Option<&'a str>,
    pub ptt_hotkey_short: &'a str,
    pub media: &'a MediaInfo,
    pub hovered_target: Option<HitTarget>,
    pub eq_levels: [f32; EQUALIZER_BAR_COUNT],
    pub system: &'a SystemSnapshot,
    pub notification: &'a NotificationSnapshot,
    pub timer: &'a PomodoroTimer,
    pub clip_items: &'a [String],
    pub clip_copied_flash: bool,
    pub sys_stats: &'a SystemStats,
    pub cicada: &'a CicadaSnapshot,
    pub cicada_audio_level: f32,
    pub cicada_subtitle_bar: bool,
    pub cicada_click_through: bool,
    pub cicada_scroll_y: f32,
    pub cicada_auto_scroll: bool,
    pub cicada_copied_flash: bool,
}

pub struct Renderer {
    factory: ID2D1Factory,
    round_stroke: Option<ID2D1StrokeStyle>,
    target: ID2D1DCRenderTarget,
    fill_brush: ID2D1SolidColorBrush,
    border_brush: ID2D1SolidColorBrush,
    accent_brush: ID2D1SolidColorBrush,
    badge_red_brush: ID2D1SolidColorBrush,
    dynamic_brush: ID2D1SolidColorBrush,
    title_brush: ID2D1SolidColorBrush,
    artist_brush: ID2D1SolidColorBrush,
    icon_brush: ID2D1SolidColorBrush,
    hover_brush: ID2D1SolidColorBrush,
    equalizer_accent_brush: ID2D1SolidColorBrush,
    placeholder_brush: ID2D1SolidColorBrush,
    hud_track_brush: ID2D1SolidColorBrush,
    hud_fill_brush: ID2D1SolidColorBrush,
    hud_text_brush: ID2D1SolidColorBrush,
    notif_title_brush: ID2D1SolidColorBrush,
    notif_body_brush: ID2D1SolidColorBrush,

    dwrite: IDWriteFactory,
    title_format: IDWriteTextFormat,
    artist_format: IDWriteTextFormat,
    hud_format: IDWriteTextFormat,
    hud_value_format: IDWriteTextFormat,
    clock_collapsed_format: IDWriteTextFormat,
    clock_media_format: IDWriteTextFormat,
    clock_hero_format: IDWriteTextFormat,
    notif_app_format: IDWriteTextFormat,
    notif_title_format: IDWriteTextFormat,
    notif_body_format: IDWriteTextFormat,
    badge_format: IDWriteTextFormat,
    pill_btn_format: IDWriteTextFormat,
    center_title_format: IDWriteTextFormat,
    center_sub_format: IDWriteTextFormat,
    title_trim: IDWriteInlineObject,
    artist_trim: IDWriteInlineObject,
    notif_title_trim: IDWriteInlineObject,
    notif_body_trim: IDWriteInlineObject,
    cached_title: String,
    cached_artist: String,
    title_layout: Option<IDWriteTextLayout>,
    artist_layout: Option<IDWriteTextLayout>,

    previous_icon: ID2D1PathGeometry,
    next_icon: ID2D1PathGeometry,
    play_icon: ID2D1PathGeometry,

    art_revision: u64,
    art_bitmap: Option<ID2D1Bitmap>,

    mem_dc: HDC,
    dib: HBITMAP,
    previous_bitmap: HGDIOBJ,
    width: u32,
    height: u32,
    dpi: f32,
    last_chat_max_scroll: f32,
    last_cicada_max_scroll: f32,
}

impl Renderer {
    pub unsafe fn new(width: u32, height: u32, dpi: f32) -> Result<Self> {
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;

        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: dpi,
            dpiY: dpi,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };

        let target = factory.CreateDCRenderTarget(&properties)?;
        let fill_brush = target.CreateSolidColorBrush(&PILL_FILL, None)?;
        let border_brush = target.CreateSolidColorBrush(&PILL_BORDER, None)?;
        let accent_brush = target.CreateSolidColorBrush(&PILL_ACCENT, None)?;
        let badge_red_brush = target.CreateSolidColorBrush(&BADGE_RED, None)?;
        let dynamic_brush = target.CreateSolidColorBrush(&PILL_ACCENT, None)?;
        let title_brush = target.CreateSolidColorBrush(&TITLE_COLOR, None)?;
        let artist_brush = target.CreateSolidColorBrush(&ARTIST_COLOR, None)?;
        let icon_brush = target.CreateSolidColorBrush(&ICON_COLOR, None)?;
        let hover_brush = target.CreateSolidColorBrush(&HOVER_COLOR, None)?;
        let equalizer_accent_brush = target.CreateSolidColorBrush(&EQUALIZER_ACCENT, None)?;
        let placeholder_brush = target.CreateSolidColorBrush(&ART_PLACEHOLDER, None)?;
        let hud_track_brush = target.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.16,
            },
            None,
        )?;
        let hud_fill_brush = target.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.92,
            },
            None,
        )?;
        let hud_text_brush = target.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.88,
            },
            None,
        )?;
        let notif_title_brush = target.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.96,
            },
            None,
        )?;
        let notif_body_brush = target.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.68,
            },
            None,
        )?;

        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let family = font_family(&dwrite);

        let title_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            TITLE_FONT_SIZE,
            w!("en-us"),
        )?;
        let artist_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_REGULAR,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            ARTIST_FONT_SIZE,
            w!("en-us"),
        )?;

        let hud_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            12.0,
            w!("en-us"),
        )?;
        hud_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
        hud_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;

        let hud_value_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            11.5,
            w!("en-us"),
        )?;
        hud_value_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
        hud_value_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;

        let clock_collapsed_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            16.5,
            w!("en-us"),
        )?;
        clock_collapsed_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        clock_collapsed_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

        let clock_media_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            18.5,
            w!("en-us"),
        )?;
        clock_media_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        clock_media_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

        let clock_hero_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            34.0,
            w!("en-us"),
        )?;
        clock_hero_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
        clock_hero_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;

        let notif_app_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            11.0,
            w!("en-us"),
        )?;
        notif_app_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;

        let notif_title_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            12.5,
            w!("en-us"),
        )?;
        notif_title_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;

        let notif_body_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_REGULAR,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            11.5,
            w!("en-us"),
        )?;
        notif_body_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;

        let badge_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            11.0,
            w!("en-us"),
        )?;
        badge_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        badge_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

        let pill_btn_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            11.0,
            w!("en-us"),
        )?;
        pill_btn_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        pill_btn_format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;

        let center_title_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            13.5,
            w!("en-us"),
        )?;
        center_title_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;

        let center_sub_format = dwrite.CreateTextFormat(
            family,
            None,
            DWRITE_FONT_WEIGHT_REGULAR,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            11.5,
            w!("en-us"),
        )?;
        center_sub_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;

        let title_trim = dwrite.CreateEllipsisTrimmingSign(&title_format)?;
        let artist_trim = dwrite.CreateEllipsisTrimmingSign(&artist_format)?;
        let notif_title_trim = dwrite.CreateEllipsisTrimmingSign(&notif_title_format)?;
        let notif_body_trim = dwrite.CreateEllipsisTrimmingSign(&notif_body_format)?;

        let previous_icon = build_control_icon(&factory, MediaControl::Previous)?;
        let next_icon = build_control_icon(&factory, MediaControl::Next)?;
        let play_icon = build_control_icon(&factory, MediaControl::PlayPause)?;
        let round_stroke = create_round_stroke(&factory);

        let mut renderer = Self {
            factory,
            round_stroke,
            target,
            fill_brush,
            border_brush,
            accent_brush,
            badge_red_brush,
            dynamic_brush,
            title_brush,
            artist_brush,
            icon_brush,
            hover_brush,
            equalizer_accent_brush,
            placeholder_brush,
            hud_track_brush,
            hud_fill_brush,
            hud_text_brush,
            notif_title_brush,
            notif_body_brush,
            dwrite,
            title_format,
            artist_format,
            hud_format,
            hud_value_format,
            clock_collapsed_format,
            clock_media_format,
            clock_hero_format,
            notif_app_format,
            notif_title_format,
            notif_body_format,
            badge_format,
            pill_btn_format,
            center_title_format,
            center_sub_format,
            title_trim,
            artist_trim,
            notif_title_trim,
            notif_body_trim,
            cached_title: String::new(),
            cached_artist: String::new(),
            title_layout: None,
            artist_layout: None,
            previous_icon,
            next_icon,
            play_icon,
            art_revision: 0,
            art_bitmap: None,
            mem_dc: HDC::default(),
            dib: HBITMAP::default(),
            previous_bitmap: HGDIOBJ::default(),
            width: 0,
            height: 0,
            dpi: 0.0,
            last_chat_max_scroll: 0.0,
            last_cicada_max_scroll: 0.0,
        };
        renderer.resize(width, height, dpi)?;
        Ok(renderer)
    }

    pub fn chat_max_scroll(&self) -> f32 {
        self.last_chat_max_scroll
    }

    pub fn cicada_max_scroll(&self) -> f32 {
        self.last_cicada_max_scroll
    }

    pub unsafe fn resize(&mut self, width: u32, height: u32, dpi: f32) -> Result<()> {
        if width == self.width && height == self.height && dpi == self.dpi {
            return Ok(());
        }

        self.release_surface();
        self.create_surface(width, height)?;
        self.target.SetDpi(dpi, dpi);
        self.width = width;
        self.height = height;
        self.dpi = dpi;
        Ok(())
    }

    pub unsafe fn render(&mut self, hwnd: HWND, frame: &Frame) -> Result<()> {
        self.prepare_art(frame.media);
        self.prepare_text(frame.media);

        let bounds = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: self.width as i32,
            bottom: self.height as i32,
        };
        self.target.BindDC(self.mem_dc, &bounds)?;

        self.target.BeginDraw();

        let transparent = D2D1_COLOR_F {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
        self.target.Clear(Some(&transparent));

        if frame.manual_hidden {
            self.target.EndDraw(None, None)?;
            return self.present(hwnd);
        }

        let width = frame.width.clamp(1.0, CANVAS_WIDTH_LOGICAL);
        let height = frame.height.clamp(1.0, CANVAS_HEIGHT_LOGICAL);
        let radius = frame.radius.clamp(0.0, width.min(height) / 2.0);

        let left = (CANVAS_WIDTH_LOGICAL - width) / 2.0;
        let tuck = ((COLLAPSED_HEIGHT - height) / (COLLAPSED_HEIGHT - AUTO_HIDDEN_HEIGHT))
            .clamp(0.0, 1.0);
        let top = PILL_TOP_LOGICAL * (1.0 - tuck);

        // Windows UpdateLayeredWindow ignores mouse input on alpha == 0 pixels.
        // Paint a visually-invisible (alpha = 2/255) hover-catch strip so moving the cursor
        // to the top edge always wakes the tucked island and prevents top-bezel bounce.
        if frame.auto_hidden {
            let sensor_w = 240.0_f32;
            let sensor_left = (CANVAS_WIDTH_LOGICAL - sensor_w) / 2.0;
            let sensor_rect = D2D_RECT_F {
                left: sensor_left,
                top: 0.0,
                right: sensor_left + sensor_w,
                bottom: 12.0,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 2.0 / 255.0,
            });
            self.target.FillRectangle(&sensor_rect, &self.dynamic_brush);
        } else if !(frame.cicada_subtitle_bar && frame.cicada_click_through) && top > 0.5 {
            let bridge_rect = D2D_RECT_F {
                left,
                top: 0.0,
                right: left + width,
                bottom: top + 1.0,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 2.0 / 255.0,
            });
            self.target.FillRectangle(&bridge_rect, &self.dynamic_brush);
        }

        // Sleek tucked top-edge handle while auto-hidden or mid-tuck animation
        if height < 20.0 {
            let handle_h = height.max(4.5);
            let handle_r = (handle_h / 2.0).min(radius.max(2.25));
            let handle_pill = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left,
                    top: (top - handle_r * 0.35).max(0.0),
                    right: left + width,
                    bottom: top + handle_h,
                },
                radiusX: handle_r,
                radiusY: handle_r,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 12.0 / 255.0,
                g: 12.0 / 255.0,
                b: 16.0 / 255.0,
                a: 0.86,
            });
            self.target
                .FillRoundedRectangle(&handle_pill, &self.dynamic_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.16,
            });
            self.target
                .DrawRoundedRectangle(&handle_pill, &self.dynamic_brush, 0.8, None);

            let grip_w = (width * 0.34).clamp(28.0, 48.0);
            let grip_left = left + (width - grip_w) / 2.0;
            let grip_y = top + (handle_h - 1.5) * 0.55;
            let grip = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: grip_left,
                    top: grip_y,
                    right: grip_left + grip_w,
                    bottom: grip_y + 1.5,
                },
                radiusX: 0.75,
                radiusY: 0.75,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.40,
            });
            self.target
                .FillRoundedRectangle(&grip, &self.dynamic_brush);

            self.target.EndDraw(None, None)?;
            return self.present(hwnd);
        }

        let pill = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left,
                top,
                right: left + width,
                bottom: top + height,
            },
            radiusX: radius,
            radiusY: radius,
        };

        if frame.cicada_subtitle_bar && !frame.expanded {
            // Semi-transparent sleek subtitle bar so meeting slides/video remain subtly visible
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 10.0 / 255.0,
                g: 12.0 / 255.0,
                b: 16.0 / 255.0,
                a: if frame.cicada_click_through { 0.80 } else { 0.88 },
            });
            self.target.FillRoundedRectangle(&pill, &self.dynamic_brush);
        } else {
            self.target.FillRoundedRectangle(&pill, &self.fill_brush);
        }

        let now = std::time::Instant::now();
        let hud_visible = frame.system.hud.is_visible(now);
        let toast_visible = frame.notification.is_toast_visible(now);

        // Top-edge accent highlight
        let inset = radius.max(14.0);
        if width > inset * 2.0 + 4.0 {
            let accent_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: left + inset,
                    top: top + 1.0,
                    right: left + width - inset,
                    bottom: top + 2.5,
                },
                radiusX: 0.75,
                radiusY: 0.75,
            };
            if toast_visible && !hud_visible && !frame.expanded {
                if let Some(toast) = &frame.notification.toast {
                    let (r, g, b) = app_accent_rgb(&toast.app);
                    self.dynamic_brush
                        .SetColor(&D2D1_COLOR_F { r, g, b, a: 0.65 });
                    self.target
                        .FillRoundedRectangle(&accent_rect, &self.dynamic_brush);
                }
            } else if frame.cicada.active && (!frame.expanded || frame.expanded_tab == ExpandedTab::Cicada) {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 0.16,
                    g: 0.86,
                    b: 0.62,
                    a: 0.85,
                });
                self.target
                    .FillRoundedRectangle(&accent_rect, &self.dynamic_brush);
            } else {
                let (zr, zg, zb) = frame.zeki.accent_color();
                let accent_alpha = if frame.zeki.state().is_assistant_active() {
                    0.85
                } else {
                    0.55
                };
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: zr,
                    g: zg,
                    b: zb,
                    a: accent_alpha,
                });
                self.target
                    .FillRoundedRectangle(&accent_rect, &self.dynamic_brush);
            }
        }

        // Clip all interior content to pill bounds
        let clip = D2D_RECT_F {
            left: left + 1.0,
            top: top + 1.0,
            right: (left + width - 1.0).max(left + 1.0),
            bottom: (top + height - 1.0).max(top + 1.0),
        };
        self.target
            .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

        let expanded_progress = expansion_progress(height);

        if hud_visible {
            self.draw_hud(left, top, width, height, &frame.system.hud);
        } else if toast_visible && !frame.expanded {
            if let Some(toast) = &frame.notification.toast {
                let progress = frame.notification.toast_progress(now);
                self.draw_notification_toast(left, top, toast, progress, frame.hovered_target);
            }
        } else if frame.cicada_subtitle_bar && !frame.expanded {
            self.draw_cicada_subtitle_bar(left, top, width, height, frame);
        } else if frame.expanded || expanded_progress >= 0.28 {
            let alpha = smoothstep(((expanded_progress - 0.15) / 0.85).clamp(0.0, 1.0));
            if alpha > 0.01 {
                self.draw_tab_bar(left, top, width, frame, alpha);
                match frame.expanded_tab {
                    ExpandedTab::Zeki => {
                        self.draw_zeki_panel(left, top, width, height, frame, alpha);
                    }
                    ExpandedTab::Cicada => {
                        self.draw_cicada_panel(left, top, width, height, frame, alpha);
                    }
                    ExpandedTab::Media => {
                        if frame.media.has_session {
                            self.draw_expanded_media(left, top, frame, alpha);
                        } else {
                            self.draw_idle_expanded(left, top, width, height, frame.system, frame.notification, alpha);
                        }
                    }
                    ExpandedTab::Timer => {
                        self.draw_timer_panel(left, top, frame.timer, frame.hovered_target, alpha);
                    }
                    ExpandedTab::Clipboard => {
                        self.draw_clipboard_panel(
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
                        self.draw_sysmon_panel(left, top, width, frame.sys_stats, alpha);
                    }
                    ExpandedTab::Notifications => {
                        self.draw_notifications_panel(
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
                    self.draw_zeki_dock(left, top, frame, alpha);
                }
            }
        } else {
            let collapsed_alpha = (1.0 - expanded_progress * 3.5).clamp(0.0, 1.0);
            if frame.cicada.active || frame.cicada.quick_reply_recording {
                self.draw_collapsed_cicada(left, top, width, height, frame, collapsed_alpha);
            } else if frame.zeki.state().is_assistant_active() {
                self.draw_collapsed_assistant(left, top, width, height, frame, collapsed_alpha);
            } else if frame.media.has_session {
                self.draw_collapsed_media(left, top, width, height, frame, collapsed_alpha);
            } else {
                self.draw_collapsed_idle(left, top, width, height, frame, collapsed_alpha);
            }
        }

        self.target.PopAxisAlignedClip();

        self.target
            .DrawRoundedRectangle(&pill, &self.border_brush, 1.0, None);

        self.target.EndDraw(None, None)?;
        self.present(hwnd)
    }

    /// Top 6-tab pill switcher shared by all expanded views (scales with pill width).
    unsafe fn draw_tab_bar(
        &self,
        origin_x: f32,
        origin_y: f32,
        pill_width: f32,
        frame: &Frame,
        alpha: f32,
    ) {
        let unread = frame.notification.unread_count();
        for tab in ALL_TABS {
            let rect = tab_rect_for_width(tab, pill_width);
            let active = frame.expanded_tab == tab;
            let hovered = frame.hovered_target == Some(HitTarget::SelectTab(tab));

            let pill = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + rect.left,
                    top: origin_y + rect.top,
                    right: origin_x + rect.right,
                    bottom: origin_y + rect.bottom,
                },
                radiusX: rect.height() / 2.0,
                radiusY: rect.height() / 2.0,
            };

            if active {
                if tab == ExpandedTab::Zeki {
                    let (zr, zg, zb) = frame.zeki.accent_color();
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: zr,
                        g: zg,
                        b: zb,
                        a: 0.24 * alpha,
                    });
                    self.target.FillRoundedRectangle(&pill, &self.dynamic_brush);
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: zr,
                        g: zg,
                        b: zb,
                        a: 0.72 * alpha,
                    });
                    self.target
                        .DrawRoundedRectangle(&pill, &self.dynamic_brush, 1.0, None);
                } else if tab == ExpandedTab::Cicada {
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: 0.16,
                        g: 0.85,
                        b: 0.62,
                        a: 0.25 * alpha,
                    });
                    self.target.FillRoundedRectangle(&pill, &self.dynamic_brush);
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: 0.16,
                        g: 0.85,
                        b: 0.62,
                        a: 0.78 * alpha,
                    });
                    self.target
                        .DrawRoundedRectangle(&pill, &self.dynamic_brush, 1.0, None);
                } else {
                    self.accent_brush.SetOpacity(0.28 * alpha);
                    self.target.FillRoundedRectangle(&pill, &self.accent_brush);
                    self.accent_brush.SetOpacity(0.65 * alpha);
                    self.target
                        .DrawRoundedRectangle(&pill, &self.accent_brush, 1.0, None);
                    self.accent_brush.SetOpacity(1.0);
                }
            } else {
                self.hover_brush
                    .SetOpacity(if hovered { 0.14 } else { 0.055 } * alpha);
                self.target.FillRoundedRectangle(&pill, &self.hover_brush);
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
                        "Notif 9+".to_string()
                    } else if unread > 0 {
                        format!("Notif {unread}")
                    } else {
                        "Notifs".to_string()
                    }
                }
            };

            let u16_lbl: Vec<u16> = label.encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &u16_lbl,
                &self.pill_btn_format,
                rect.width(),
                rect.height(),
            ) {
                let text_alpha = if active {
                    1.0
                } else if hovered {
                    0.92
                } else {
                    0.68
                };
                self.title_brush.SetOpacity(text_alpha * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + rect.left,
                        y: origin_y + rect.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    }

    unsafe fn draw_collapsed_media(
        &self,
        origin_x: f32,
        origin_y: f32,
        pill_width: f32,
        pill_height: f32,
        frame: &Frame,
        alpha: f32,
    ) {
        self.draw_art(origin_x, origin_y, &COLLAPSED_ART, COLLAPSED_ART_RADIUS);

        let unread = frame.notification.unread_count();
        let center_text = if frame.timer.running {
            frame.timer.formatted()
        } else {
            current_clock_string()
        };
        let center_u16: Vec<u16> = center_text.encode_utf16().collect();

        if unread > 0 {
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &center_u16,
                &self.clock_collapsed_format,
                66.0,
                pill_height,
            ) {
                if frame.timer.running {
                    self.accent_brush.SetOpacity(alpha);
                    self.target.DrawTextLayout(
                        D2D_POINT_2F { x: origin_x + 36.0, y: origin_y },
                        &layout,
                        &self.accent_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                    self.accent_brush.SetOpacity(1.0);
                } else {
                    self.title_brush.SetOpacity(0.96 * alpha);
                    self.target.DrawTextLayout(
                        D2D_POINT_2F { x: origin_x + 36.0, y: origin_y },
                        &layout,
                        &self.title_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }

            self.draw_equalizer(
                origin_x,
                origin_y,
                pill_height / 2.0,
                &frame.eq_levels,
                frame.media.is_playing,
                alpha,
                pill_width - 62.0,
                EQUALIZER_BAR_WIDTH,
                EQUALIZER_BAR_GAP,
                3.5,
                14.5,
            );
            if let Some(app_name) = frame.notification.latest_app_name() {
                self.draw_collapsed_notification_badge(
                    origin_x,
                    origin_y,
                    pill_width,
                    pill_height,
                    app_name,
                    unread,
                    alpha,
                );
            }
        } else {
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &center_u16,
                &self.clock_collapsed_format,
                76.0,
                pill_height,
            ) {
                self.title_brush.SetOpacity(0.97 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + 42.0, y: origin_y },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            self.draw_equalizer(
                origin_x,
                origin_y,
                pill_height / 2.0,
                &frame.eq_levels,
                frame.media.is_playing,
                alpha,
                EQUALIZER_RIGHT,
                EQUALIZER_BAR_WIDTH,
                EQUALIZER_BAR_GAP,
                3.5,
                14.5,
            );
        }
    }

    unsafe fn draw_collapsed_assistant(
        &self,
        pill_left: f32,
        pill_top: f32,
        pill_width: f32,
        pill_height: f32,
        frame: &Frame,
        alpha: f32,
    ) {
        // Mini Zeki living on the left edge of the active assistant capsule
        frame.zeki.draw(
            &self.factory,
            &self.target,
            &self.dynamic_brush,
            self.round_stroke.as_ref(),
            &self.dwrite,
            &self.badge_format,
            pill_left + 23.0,
            pill_top + pill_height / 2.0 + 1.0,
            11.5,
            alpha,
            true,
        );

        let (zr, zg, zb) = frame.zeki.accent_color();
        let state_label = match frame.assistant.phase {
            AssistantPhase::Listening => "Listening...",
            AssistantPhase::Transcribing => "Transcribing...",
            AssistantPhase::Thinking if frame.assistant.screen_capturing_now => "Seeing screen...",
            AssistantPhase::Thinking => "Thinking...",
            AssistantPhase::Speaking => "Speaking...",
            _ => frame.zeki.state().collapsed_tag(),
        };
        let lbl_u16: Vec<u16> = state_label.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&lbl_u16, &self.notif_title_format, 108.0, 20.0)
        {
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: (zr * 0.45 + 0.55).min(1.0),
                g: (zg * 0.45 + 0.55).min(1.0),
                b: (zb * 0.45 + 0.55).min(1.0),
                a: 0.98 * alpha,
            });
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: pill_left + 44.0,
                    y: pill_top + (pill_height - 17.0) / 2.0,
                },
                &layout,
                &self.dynamic_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // While actively recording mic, draw live 5-bar voice waveform on the right!
        if frame.assistant.phase == AssistantPhase::Listening {
            let level = frame.voice_level.clamp(0.05, 1.0);
            let bar_w = 3.0f32;
            let gap = 3.0f32;
            let right_edge = pill_left + pill_width - 18.0;
            let cy = pill_top + pill_height / 2.0;
            let weights = [0.55f32, 0.85, 1.0, 0.80, 0.50];
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: zr,
                g: zg,
                b: zb,
                a: 0.95 * alpha,
            });
            for (i, &w_mul) in weights.iter().enumerate() {
                let bh = (4.0 + 15.0 * level * w_mul).clamp(4.0, 19.0);
                let bx = right_edge - (4 - i) as f32 * (bar_w + gap) - bar_w;
                let bar = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: bx,
                        top: cy - bh / 2.0,
                        right: bx + bar_w,
                        bottom: cy + bh / 2.0,
                    },
                    radiusX: bar_w / 2.0,
                    radiusY: bar_w / 2.0,
                };
                self.target.FillRoundedRectangle(&bar, &self.dynamic_brush);
            }
        } else {
            // Bold clock on the right so time remains readable even while Zeki is active
            let clock = current_clock_string();
            let clock_u16: Vec<u16> = clock.encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &clock_u16,
                &self.clock_collapsed_format,
                66.0,
                pill_height,
            ) {
                self.title_brush.SetOpacity(0.95 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: pill_left + pill_width - 74.0,
                        y: pill_top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    }

    // =========================================================================
    // Cicada Mode (Live Meeting Translator): 3 States
    // 1. Standby / Listening (Collapsed Pill)
    // 2. Live Subtitle Bar (620x94 semi-transparent horizontal bar)
    // 3. Expanded Transcript (560x340 / 640x470 scrollable bilingual panel)
    // =========================================================================

    unsafe fn draw_collapsed_cicada(
        &self,
        pill_left: f32,
        pill_top: f32,
        pill_width: f32,
        pill_height: f32,
        frame: &Frame,
        alpha: f32,
    ) {
        // Mini Zeki wearing headset on left
        frame.zeki.draw(
            &self.factory,
            &self.target,
            &self.dynamic_brush,
            self.round_stroke.as_ref(),
            &self.dwrite,
            &self.badge_format,
            pill_left + 23.0,
            pill_top + pill_height / 2.0 + 1.0,
            11.0,
            alpha,
            true,
        );

        // Blinking live dot (amber when Quick-Reply recording, emerald when listening)
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
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: dr,
            g: dg,
            b: db,
            a: (if blink { 0.98 } else { 0.40 }) * alpha,
        });
        self.target.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: pill_left + 44.0,
                    y: pill_top + pill_height / 2.0,
                },
                radiusX: 3.6,
                radiusY: 3.6,
            },
            &self.dynamic_brush,
        );

        let tag = if frame.cicada.quick_reply_recording {
            format!("Reply→{}", cicada_lang_short_badge(&frame.cicada.reply_lang))
        } else if frame.cicada.pending_segments > 0 {
            format!("{} …", frame.cicada.lang_pair_badge())
        } else {
            frame.cicada.lang_pair_badge()
        };
        let tag_u16: Vec<u16> = tag.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&tag_u16, &self.notif_title_format, 96.0, 20.0)
        {
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 0.72,
                g: 0.98,
                b: 0.88,
                a: 0.96 * alpha,
            });
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: pill_left + 54.0,
                    y: pill_top + (pill_height - 17.0) / 2.0,
                },
                &layout,
                &self.dynamic_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Live 5-bar waveform on the right
        let level = frame.cicada_audio_level.clamp(0.08, 1.0);
        let bar_w = 3.0f32;
        let gap = 3.0f32;
        let right_edge = pill_left + pill_width - 16.0;
        let cy = pill_top + pill_height / 2.0;
        let weights = [0.50f32, 0.82, 1.0, 0.78, 0.46];
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: dr,
            g: dg,
            b: db,
            a: 0.92 * alpha,
        });
        for (i, &w_mul) in weights.iter().enumerate() {
            let bh = (4.0 + 14.0 * level * w_mul).clamp(4.0, 18.0);
            let bx = right_edge - (4 - i) as f32 * (bar_w + gap) - bar_w;
            let bar = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: bx,
                    top: cy - bh / 2.0,
                    right: bx + bar_w,
                    bottom: cy + bh / 2.0,
                },
                radiusX: bar_w / 2.0,
                radiusY: bar_w / 2.0,
            };
            self.target.FillRoundedRectangle(&bar, &self.dynamic_brush);
        }
    }

    unsafe fn draw_cicada_subtitle_bar(
        &self,
        pill_left: f32,
        pill_top: f32,
        pill_width: f32,
        pill_height: f32,
        frame: &Frame,
    ) {
        let bl = cicada_bar_layout(pill_width, pill_height);
        let hovered = frame.hovered_target;

        // Zeki wearing headset on left end of bar (nods on each new line!)
        frame.zeki.draw(
            &self.factory,
            &self.target,
            &self.dynamic_brush,
            self.round_stroke.as_ref(),
            &self.dwrite,
            &self.badge_format,
            pill_left + bl.zeki_center.0,
            pill_top + bl.zeki_center.1,
            19.0,
            1.0,
            false,
        );

        // Compact status header on top-left next to Zeki
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
        let st_u16: Vec<u16> = status_txt.encode_utf16().collect();
        let header_w = (bl.reply_btn.left - 70.0).max(80.0);
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&st_u16, &self.notif_app_format, header_w, 18.0)
        {
            let _ = layout.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                },
                &self.notif_body_trim,
            );
            let (sr, sg, sb) = if frame.cicada.error.is_some() {
                (0.98, 0.44, 0.52)
            } else if frame.cicada.quick_reply_recording {
                (0.98, 0.76, 0.25)
            } else {
                (0.24, 0.90, 0.68)
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: sr,
                g: sg,
                b: sb,
                a: 0.95,
            });
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: pill_left + 66.0,
                    y: pill_top + 11.0,
                },
                &layout,
                &self.dynamic_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Top-right micro controls: Reply | Pass/Lock | Full | Stop
        self.draw_cicada_chip_btn(
            pill_left,
            pill_top,
            bl.reply_btn,
            if frame.cicada.quick_reply_recording { "● Stop" } else { "Reply" },
            frame.cicada.quick_reply_recording,
            hovered == Some(HitTarget::CicadaQuickReply),
            (0.98, 0.72, 0.22),
            1.0,
        );
        self.draw_cicada_chip_btn(
            pill_left,
            pill_top,
            bl.pass_btn,
            if frame.cicada_click_through { "Unlock" } else { "Pass" },
            frame.cicada_click_through,
            hovered == Some(HitTarget::CicadaToggleClickThrough),
            (0.38, 0.76, 0.98),
            1.0,
        );
        self.draw_cicada_chip_btn(
            pill_left,
            pill_top,
            bl.expand_btn,
            "Full",
            false,
            hovered == Some(HitTarget::CicadaExpandFromBar),
            (0.68, 0.74, 0.85),
            1.0,
        );
        self.draw_cicada_chip_btn(
            pill_left,
            pill_top,
            bl.stop_btn,
            if frame.cicada.active { "Stop" } else { "Start" },
            frame.cicada.active,
            hovered == Some(HitTarget::CicadaToggleListen),
            (0.94, 0.30, 0.38),
            1.0,
        );

        // Live 2-3 line bilingual subtitle area
        let tx = pill_left + bl.text_rect.left;
        let ty = pill_top + bl.text_rect.top;
        let tw = bl.text_rect.width().max(120.0);
        let th = bl.text_rect.height().max(40.0);

        if let Some(latest) = frame.cicada.entries.last() {
            let mut y_cur = ty;
            // Optional dim original line on top (1 line)
            if frame.cicada.show_original && !latest.original.eq_ignore_ascii_case(&latest.translation) {
                let orig_line = format!("[{}] {}", latest.speaker.tag(), latest.original);
                let u16_orig: Vec<u16> = orig_line.encode_utf16().collect();
                if let Ok(layout) =
                    self.dwrite
                        .CreateTextLayout(&u16_orig, &self.notif_app_format, tw, 16.0)
                {
                    let _ = layout.SetTrimming(
                        &DWRITE_TRIMMING {
                            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                            delimiter: 0,
                            delimiterCount: 0,
                        },
                        &self.notif_body_trim,
                    );
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: 0.76,
                        g: 0.82,
                        b: 0.90,
                        a: 0.72,
                    });
                    self.target.DrawTextLayout(
                        D2D_POINT_2F { x: tx, y: y_cur },
                        &layout,
                        &self.dynamic_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
                y_cur += 16.0;
            }

            // Crisp bright translated subtitle (up to 2 lines)
            let trans_text = if latest.speaker == SpeakerChannel::QuickReply {
                format!("💬 {}", latest.translation)
            } else {
                latest.translation.clone()
            };
            let u16_tr: Vec<u16> = trans_text.encode_utf16().collect();
            let avail_h = (ty + th - y_cur).max(20.0);
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&u16_tr, &self.notif_title_format, tw, avail_h)
            {
                let _ = layout.SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    },
                    &self.notif_title_trim,
                );
                let (tr, tg, tb) = match latest.speaker {
                    SpeakerChannel::Online => (1.0, 1.0, 1.0),
                    SpeakerChannel::You => (0.62, 0.98, 0.80),
                    SpeakerChannel::QuickReply => (1.0, 0.88, 0.55),
                };
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: tr,
                    g: tg,
                    b: tb,
                    a: 0.99,
                });
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: tx, y: y_cur },
                    &layout,
                    &self.dynamic_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        } else {
            let waiting = if frame.cicada.active {
                "Mendengarkan audio sistem (Zoom / Meet / Teams / YouTube)... subtitle akan muncul otomatis."
            } else {
                "Cicada Subtitle Bar siap. Klik Start untuk mulai menerjemahkan audio rapat."
            };
            let u16_w: Vec<u16> = waiting.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&u16_w, &self.notif_body_format, tw, th)
            {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 0.78,
                    g: 0.84,
                    b: 0.92,
                    a: 0.72,
                });
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: tx, y: ty + 8.0 },
                    &layout,
                    &self.dynamic_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    }

    unsafe fn draw_cicada_panel(
        &mut self,
        origin_x: f32,
        origin_y: f32,
        pill_width: f32,
        pill_height: f32,
        frame: &Frame,
        alpha: f32,
    ) {
        let cl = cicada_panel_layout(pill_width, pill_height);
        let hovered = frame.hovered_target;

        // 1. Left Zeki Taking-Notes Stage
        let stage = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + cl.zeki_stage.left,
                top: origin_y + cl.zeki_stage.top,
                right: origin_x + cl.zeki_stage.right,
                bottom: origin_y + cl.zeki_stage.bottom,
            },
            radiusX: 16.0,
            radiusY: 16.0,
        };
        let (er, eg, eb) = if frame.cicada.active {
            (0.16, 0.86, 0.62)
        } else {
            (0.24, 0.74, 0.96)
        };
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: er,
            g: eg,
            b: eb,
            a: 0.10 * alpha,
        });
        self.target.FillRoundedRectangle(&stage, &self.dynamic_brush);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: er,
            g: eg,
            b: eb,
            a: 0.36 * alpha,
        });
        self.target
            .DrawRoundedRectangle(&stage, &self.dynamic_brush, 1.0, None);

        let (scx, scy) = cl.zeki_stage.center();
        frame.zeki.draw(
            &self.factory,
            &self.target,
            &self.dynamic_brush,
            self.round_stroke.as_ref(),
            &self.dwrite,
            &self.badge_format,
            origin_x + scx - 4.0,
            origin_y + scy + 2.0,
            23.0,
            alpha,
            false,
        );

        // Live audio badge inside top-left of Zeki stage
        if frame.cicada.active {
            let live_u16: Vec<u16> = "● LIVE".encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&live_u16, &self.badge_format, 46.0, 14.0)
            {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 0.18,
                    g: 0.92,
                    b: 0.65,
                    a: 0.95 * alpha,
                });
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + cl.zeki_stage.left + 4.0,
                        y: origin_y + cl.zeki_stage.top + 4.0,
                    },
                    &layout,
                    &self.dynamic_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        // 2. Left Column Control Buttons
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.listen_btn,
            if frame.cicada.active { "■ Stop Live" } else { "▶ Start Live" },
            frame.cicada.active,
            hovered == Some(HitTarget::CicadaToggleListen),
            if frame.cicada.active { (0.94, 0.30, 0.38) } else { (0.16, 0.86, 0.62) },
            alpha,
        );
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.mic_btn,
            if frame.cicada.mic_enabled { "Mic: ON" } else { "Mic: OFF" },
            frame.cicada.mic_enabled,
            hovered == Some(HitTarget::CicadaToggleMic),
            (0.22, 0.82, 0.94),
            alpha,
        );
        let reply_lbl = if frame.cicada.quick_reply_recording {
            "● Stop Reply".to_string()
        } else if frame.cicada.quick_reply_busy {
            "Translating...".to_string()
        } else {
            format!("Reply →{}", cicada_lang_short_badge(&frame.cicada.reply_lang))
        };
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.reply_btn,
            &reply_lbl,
            frame.cicada.quick_reply_recording,
            hovered == Some(HitTarget::CicadaQuickReply),
            (0.98, 0.72, 0.22),
            alpha,
        );
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.sub_bar_btn,
            "Subtitle Bar",
            frame.cicada_subtitle_bar,
            hovered == Some(HitTarget::CicadaToggleSubtitleBar),
            (0.65, 0.56, 0.98),
            alpha,
        );

        // 3. Top-Right Title & Language / Original / Expand Chips
        let hdr = if frame.cicada.glossary_count > 0 {
            format!(
                "Cicada Live · {} · {} terms",
                frame.cicada.lang_pair_badge(),
                frame.cicada.glossary_count
            )
        } else {
            format!("Cicada Live · {}", frame.cicada.lang_pair_badge())
        };
        let hdr_u16: Vec<u16> = hdr.encode_utf16().collect();
        let hdr_w = (cl.lang_btn.left - cl.card.left - 6.0).max(80.0);
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&hdr_u16, &self.notif_title_format, hdr_w, 20.0)
        {
            self.title_brush.SetOpacity(0.95 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + cl.card.left + 2.0,
                    y: origin_y + 47.0,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let lang_lbl = format!("→ {}", cicada_lang_short_badge(&frame.cicada.target_lang));
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.lang_btn,
            &lang_lbl,
            true,
            hovered == Some(HitTarget::CicadaCycleLang),
            (0.16, 0.86, 0.62),
            alpha,
        );
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.orig_btn,
            if frame.cicada.show_original { "Orig ON" } else { "Orig OFF" },
            frame.cicada.show_original,
            hovered == Some(HitTarget::CicadaToggleOriginal),
            (0.38, 0.74, 0.98),
            alpha,
        );
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.size_btn,
            if pill_width > 600.0 { "Small" } else { "Wide" },
            false,
            hovered == Some(HitTarget::CicadaToggleSize),
            (0.68, 0.74, 0.85),
            alpha,
        );

        // 4. Scrollable Bilingual Transcript Card
        let card_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + cl.card.left,
                top: origin_y + cl.card.top,
                right: origin_x + cl.card.right,
                bottom: origin_y + cl.card.bottom,
            },
            radiusX: 14.0,
            radiusY: 14.0,
        };
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.045 * alpha,
        });
        self.target
            .FillRoundedRectangle(&card_rect, &self.dynamic_brush);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.11 * alpha,
        });
        self.target
            .DrawRoundedRectangle(&card_rect, &self.dynamic_brush, 1.0, None);

        let card_clip = D2D_RECT_F {
            left: origin_x + cl.card.left + 8.0,
            top: origin_y + cl.card.top + 6.0,
            right: origin_x + cl.card.right - 8.0,
            bottom: origin_y + cl.card.bottom - 6.0,
        };
        self.target
            .PushAxisAlignedClip(&card_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

        let inner_w = (cl.card.width() - 22.0).max(160.0);
        let view_h = (cl.card.height() - 12.0).max(40.0);

        if let Some(err) = &frame.cicada.error {
            let msg = format!("Cicada Notice:\n{err}");
            let u16_e: Vec<u16> = msg.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&u16_e, &self.notif_body_format, inner_w, view_h)
            {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 0.98,
                    g: 0.54,
                    b: 0.58,
                    a: 0.95 * alpha,
                });
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + cl.card.left + 10.0,
                        y: origin_y + cl.card.top + 10.0,
                    },
                    &layout,
                    &self.dynamic_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            self.last_cicada_max_scroll = 0.0;
        } else if frame.cicada.entries.is_empty() {
            let empty_msg = if frame.cicada.active {
                "Menunggu suara rapat dari speaker (WASAPI Loopback)...\n\nTips:\n• Klik 'Subtitle Bar' untuk tampilan bar tipis click-through di atas Zoom/Meet.\n• Nyalakan 'Mic: ON' untuk transkrip 2 arah (Online + Anda).\n• Klik 'Reply' untuk bicara bahasa Indonesia dan langsung diterjemahkan."
            } else {
                "Cicada Mode — Live Meeting Translator\n\nKlik '▶ Start Live' untuk mulai menangkap audio speaker (Zoom, Google Meet, Teams, YouTube) tanpa bot rapat.\n\nAtur bahasa target & Custom Glossary lewat tombol Config."
            };
            let u16_m: Vec<u16> = empty_msg.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&u16_m, &self.notif_body_format, inner_w, view_h)
            {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 0.80,
                    g: 0.85,
                    b: 0.92,
                    a: 0.72 * alpha,
                });
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + cl.card.left + 10.0,
                        y: origin_y + cl.card.top + 10.0,
                    },
                    &layout,
                    &self.dynamic_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            self.last_cicada_max_scroll = 0.0;
        } else {
            // Measure all entries so scrolling & auto-scroll work smoothly
            struct MeasuredEntry {
                header: String,
                orig_layout: Option<IDWriteTextLayout>,
                orig_h: f32,
                trans_layout: Option<IDWriteTextLayout>,
                speaker: SpeakerChannel,
                total_h: f32,
            }

            let mut items: Vec<MeasuredEntry> = Vec::with_capacity(frame.cicada.entries.len());
            let mut total_content_h = 0.0f32;
            for entry in &frame.cicada.entries {
                let header = format!(
                    "{}  {}  {}→{}",
                    entry.timestamp_label(),
                    entry.speaker.tag(),
                    cicada_lang_short_badge(&entry.source_lang),
                    cicada_lang_short_badge(&entry.target_lang),
                );
                let (orig_layout, orig_h) = if frame.cicada.show_original
                    && !entry.original.eq_ignore_ascii_case(&entry.translation)
                {
                    let u16_o: Vec<u16> = entry.original.encode_utf16().collect();
                    if let Ok(l) =
                        self.dwrite
                            .CreateTextLayout(&u16_o, &self.notif_app_format, inner_w - 6.0, 200.0)
                    {
                        let mut m = DWRITE_TEXT_METRICS::default();
                        let h = if l.GetMetrics(&mut m).is_ok() {
                            m.height.ceil()
                        } else {
                            15.0
                        };
                        (Some(l), h + 2.0)
                    } else {
                        (None, 0.0)
                    }
                } else {
                    (None, 0.0)
                };

                let u16_t: Vec<u16> = entry.translation.encode_utf16().collect();
                let (trans_layout, trans_h) = if let Ok(l) =
                    self.dwrite
                        .CreateTextLayout(&u16_t, &self.notif_title_format, inner_w - 6.0, 260.0)
                {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    let h = if l.GetMetrics(&mut m).is_ok() {
                        m.height.ceil()
                    } else {
                        17.0
                    };
                    (Some(l), h)
                } else {
                    (None, 17.0)
                };

                let block_h = 16.0 + orig_h + trans_h + 10.0;
                total_content_h += block_h;
                items.push(MeasuredEntry {
                    header,
                    orig_layout,
                    orig_h,
                    trans_layout,
                    speaker: entry.speaker,
                    total_h: block_h,
                });
            }

            let max_scroll = (total_content_h - view_h).max(0.0);
            self.last_cicada_max_scroll = max_scroll;
            let scroll = if frame.cicada_auto_scroll {
                max_scroll
            } else {
                frame.cicada_scroll_y.clamp(0.0, max_scroll)
            };

            let mut cur_y = origin_y + cl.card.top + 8.0 - scroll;
            let clip_top = origin_y + cl.card.top - 40.0;
            let clip_bot = origin_y + cl.card.bottom + 40.0;

            for item in &items {
                if cur_y + item.total_h >= clip_top && cur_y <= clip_bot {
                    let (sr, sg, sb) = match item.speaker {
                        SpeakerChannel::Online => (0.24, 0.80, 0.98),
                        SpeakerChannel::You => (0.22, 0.88, 0.62),
                        SpeakerChannel::QuickReply => (0.98, 0.76, 0.25),
                    };

                    // Left color accent bar for speaker distinction
                    let bar_rect = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: origin_x + cl.card.left + 10.0,
                            top: cur_y + 2.0,
                            right: origin_x + cl.card.left + 12.5,
                            bottom: cur_y + item.total_h - 8.0,
                        },
                        radiusX: 1.2,
                        radiusY: 1.2,
                    };
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: sr,
                        g: sg,
                        b: sb,
                        a: 0.85 * alpha,
                    });
                    self.target
                        .FillRoundedRectangle(&bar_rect, &self.dynamic_brush);

                    // Header line (timestamp + speaker + lang pair)
                    let u16_h: Vec<u16> = item.header.encode_utf16().collect();
                    if let Ok(hl) =
                        self.dwrite
                            .CreateTextLayout(&u16_h, &self.notif_app_format, inner_w - 8.0, 15.0)
                    {
                        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                            r: sr,
                            g: sg,
                            b: sb,
                            a: 0.92 * alpha,
                        });
                        self.target.DrawTextLayout(
                            D2D_POINT_2F {
                                x: origin_x + cl.card.left + 18.0,
                                y: cur_y,
                            },
                            &hl,
                            &self.dynamic_brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }

                    let mut text_y = cur_y + 15.0;
                    if let Some(ol) = &item.orig_layout {
                        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                            r: 0.75,
                            g: 0.80,
                            b: 0.88,
                            a: 0.68 * alpha,
                        });
                        self.target.DrawTextLayout(
                            D2D_POINT_2F {
                                x: origin_x + cl.card.left + 18.0,
                                y: text_y,
                            },
                            ol,
                            &self.dynamic_brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                        text_y += item.orig_h;
                    }
                    if let Some(tl) = &item.trans_layout {
                        self.title_brush.SetOpacity(0.97 * alpha);
                        self.target.DrawTextLayout(
                            D2D_POINT_2F {
                                x: origin_x + cl.card.left + 18.0,
                                y: text_y,
                            },
                            tl,
                            &self.title_brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
                cur_y += item.total_h;
            }

            // Subtle scrollbar thumb when scrollable
            if max_scroll > 1.0 {
                let track_h = view_h - 4.0;
                let thumb_h = (track_h * (view_h / total_content_h)).clamp(18.0, track_h);
                let thumb_y =
                    origin_y + cl.card.top + 8.0 + (scroll / max_scroll) * (track_h - thumb_h);
                let thumb = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: origin_x + cl.card.right - 6.0,
                        top: thumb_y,
                        right: origin_x + cl.card.right - 3.0,
                        bottom: thumb_y + thumb_h,
                    },
                    radiusX: 1.5,
                    radiusY: 1.5,
                };
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.28 * alpha,
                });
                self.target
                    .FillRoundedRectangle(&thumb, &self.dynamic_brush);
            }
        }

        self.target.PopAxisAlignedClip();

        // 5. Bottom Action Bar (Copy MD | Summary | Ask Zeki | Clear)
        let has_entries = frame.cicada.has_entries();
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.copy_btn,
            if frame.cicada_copied_flash { "Copied!" } else { "Copy MD" },
            frame.cicada_copied_flash,
            has_entries && hovered == Some(HitTarget::CicadaCopyTranscript),
            (0.20, 0.85, 0.56),
            if has_entries { alpha } else { alpha * 0.42 },
        );
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.summary_btn,
            "Auto-Summary",
            false,
            has_entries && hovered == Some(HitTarget::CicadaSummarize),
            (0.65, 0.55, 0.98),
            if has_entries { alpha } else { alpha * 0.42 },
        );
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.ask_btn,
            "Ask Zeki",
            true,
            hovered == Some(HitTarget::CicadaAskZeki),
            (0.22, 0.74, 0.97),
            alpha,
        );
        self.draw_cicada_chip_btn(
            origin_x,
            origin_y,
            cl.clear_btn,
            "Clear",
            false,
            has_entries && hovered == Some(HitTarget::CicadaClear),
            (0.94, 0.34, 0.42),
            if has_entries { alpha } else { alpha * 0.42 },
        );
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_cicada_chip_btn(
        &self,
        origin_x: f32,
        origin_y: f32,
        rect: RectF,
        label: &str,
        active: bool,
        hovered: bool,
        rgb: (f32, f32, f32),
        alpha: f32,
    ) {
        let rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + rect.left,
                top: origin_y + rect.top,
                right: origin_x + rect.right,
                bottom: origin_y + rect.bottom,
            },
            radiusX: (rect.height() * 0.42).min(10.0),
            radiusY: (rect.height() * 0.42).min(10.0),
        };
        let (r, g, b) = rgb;
        if active {
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r,
                g,
                b,
                a: (if hovered { 0.34 } else { 0.24 }) * alpha,
            });
            self.target.FillRoundedRectangle(&rr, &self.dynamic_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r,
                g,
                b,
                a: 0.80 * alpha,
            });
            self.target
                .DrawRoundedRectangle(&rr, &self.dynamic_brush, 1.0, None);
        } else {
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: (if hovered { 0.16 } else { 0.08 }) * alpha,
            });
            self.target.FillRoundedRectangle(&rr, &self.dynamic_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: (if hovered { 0.28 } else { 0.14 }) * alpha,
            });
            self.target
                .DrawRoundedRectangle(&rr, &self.dynamic_brush, 0.9, None);
        }

        let u16_l: Vec<u16> = label.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&u16_l, &self.pill_btn_format, rect.width(), rect.height())
        {
            self.title_brush
                .SetOpacity((if active || hovered { 0.98 } else { 0.84 }) * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + rect.left,
                    y: origin_y + rect.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    unsafe fn draw_collapsed_idle(
        &self,
        pill_left: f32,
        pill_top: f32,
        pill_width: f32,
        pill_height: f32,
        frame: &Frame,
        alpha: f32,
    ) {
        // Living Mini Zeki mascot on the left of the collapsed pill
        frame.zeki.draw(
            &self.factory,
            &self.target,
            &self.dynamic_brush,
            self.round_stroke.as_ref(),
            &self.dwrite,
            &self.badge_format,
            pill_left + 19.5,
            pill_top + pill_height / 2.0 + 0.5,
            9.8,
            alpha,
            true,
        );

        let clock = current_clock_string();
        let clock_u16: Vec<u16> = clock.encode_utf16().collect();
        let unread = frame.notification.unread_count();

        if frame.timer.running {
            // [Zeki] 24:59   10:22  (or [Zeki] 24:59   [Avatar][Badge])
            let t_str = frame.timer.formatted();
            let t_u16: Vec<u16> = t_str.encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &t_u16,
                &self.clock_collapsed_format,
                64.0,
                pill_height,
            ) {
                self.accent_brush.SetOpacity(alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: pill_left + 34.0, y: pill_top },
                    &layout,
                    &self.accent_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
                self.accent_brush.SetOpacity(1.0);
            }

            if unread > 0 {
                if let Some(app_name) = frame.notification.latest_app_name() {
                    self.draw_collapsed_notification_badge(
                        pill_left, pill_top, pill_width, pill_height, app_name, unread, alpha,
                    );
                }
            } else if let Ok(layout) = self.dwrite.CreateTextLayout(
                &clock_u16,
                &self.clock_collapsed_format,
                66.0,
                pill_height,
            ) {
                self.title_brush.SetOpacity(0.95 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: pill_left + pill_width - 74.0, y: pill_top },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            return;
        }

        if unread > 0 {
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &clock_u16,
                &self.clock_collapsed_format,
                68.0,
                pill_height,
            ) {
                self.title_brush.SetOpacity(0.98 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: pill_left + 35.0, y: pill_top },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            if let Some(app_name) = frame.notification.latest_app_name() {
                self.draw_collapsed_notification_badge(
                    pill_left, pill_top, pill_width, pill_height, app_name, unread, alpha,
                );
            }
        } else {
            let tag = frame.zeki.state().collapsed_tag();
            let label: Vec<u16> = tag.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&label, &self.hud_format, 68.0, pill_height)
            {
                self.hud_text_brush.SetOpacity(0.84 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: pill_left + 36.0,
                        y: pill_top + (pill_height - 15.0) / 2.0,
                    },
                    &layout,
                    &self.hud_text_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &clock_u16,
                &self.clock_collapsed_format,
                66.0,
                pill_height,
            ) {
                self.title_brush.SetOpacity(0.98 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: pill_left + pill_width - 74.0, y: pill_top },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    }

    /// Expanded "Zeki" tab: interactive hero mascot + scrollable multi-turn Chat OR 7-state preview.
    unsafe fn draw_zeki_panel(
        &mut self,
        origin_x: f32,
        origin_y: f32,
        pill_width: f32,
        pill_height: f32,
        frame: &Frame,
        alpha: f32,
    ) {
        let zeki = frame.zeki;
        let assistant = frame.assistant;
        let hovered = frame.hovered_target;
        let (ar, ag, ab) = zeki.accent_color();
        let stage_hovered = hovered == Some(HitTarget::ZekiPoke);

        // Left interactive hero stage
        let stage = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + ZEKI_STAGE_RECT.left,
                top: origin_y + ZEKI_STAGE_RECT.top,
                right: origin_x + ZEKI_STAGE_RECT.right,
                bottom: origin_y + ZEKI_STAGE_RECT.bottom,
            },
            radiusX: 18.0,
            radiusY: 18.0,
        };
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: if stage_hovered { 0.14 } else { 0.08 } * alpha,
        });
        self.target.FillRoundedRectangle(&stage, &self.dynamic_brush);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: if stage_hovered { 0.58 } else { 0.28 } * alpha,
        });
        self.target
            .DrawRoundedRectangle(&stage, &self.dynamic_brush, 1.0, None);

        let (scx, scy) = ZEKI_STAGE_RECT.center();
        zeki.draw(
            &self.factory,
            &self.target,
            &self.dynamic_brush,
            self.round_stroke.as_ref(),
            &self.dwrite,
            &self.badge_format,
            origin_x + scx,
            origin_y + scy + 3.0,
            27.0,
            alpha,
            false,
        );

        // Clear live Privacy Indicator badge inside top-left of Zeki's stage when Mic or Screen Vision is active
        if assistant.phase == AssistantPhase::Listening || assistant.screen_capturing_now {
            let (ir, ig, ib, ind_text) = if assistant.phase == AssistantPhase::Listening {
                (0.96, 0.30, 0.38, "● MIC")
            } else {
                (0.14, 0.84, 0.94, "● SCREEN")
            };
            let badge_w = if assistant.phase == AssistantPhase::Listening { 44.0 } else { 58.0 };
            let ind_pill = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + ZEKI_STAGE_RECT.left + 6.0,
                    top: origin_y + ZEKI_STAGE_RECT.top + 6.0,
                    right: origin_x + ZEKI_STAGE_RECT.left + 6.0 + badge_w,
                    bottom: origin_y + ZEKI_STAGE_RECT.top + 20.0,
                },
                radiusX: 7.0,
                radiusY: 7.0,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ir,
                g: ig,
                b: ib,
                a: 0.28 * alpha,
            });
            self.target
                .FillRoundedRectangle(&ind_pill, &self.dynamic_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ir,
                g: ig,
                b: ib,
                a: 0.90 * alpha,
            });
            self.target
                .DrawRoundedRectangle(&ind_pill, &self.dynamic_brush, 0.9, None);
            let ind_u16: Vec<u16> = ind_text.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&ind_u16, &self.badge_format, badge_w, 14.0)
            {
                self.title_brush.SetOpacity(0.98 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + ZEKI_STAGE_RECT.left + 6.0,
                        y: origin_y + ZEKI_STAGE_RECT.top + 6.0,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        // Subtle "Poke!" prompt when hovering Zeki's stage
        if stage_hovered {
            let hint: Vec<u16> = "Poke!".encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&hint, &self.badge_format, ZEKI_STAGE_RECT.width(), 14.0)
            {
                self.title_brush.SetOpacity(0.70 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + ZEKI_STAGE_RECT.left,
                        y: origin_y + ZEKI_STAGE_RECT.bottom - 16.0,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        // Branch 1: Active Chat / Voice conversation view (520x310 normal, 640x470 large on 2x click)
        if frame.chat_mode || assistant.has_conversation_view() {
            let has_file = frame.attached_file_label.is_some();
            let cl = chat_layout(pill_width, pill_height, has_file);

            // Mic / Stop pill button beneath Zeki's stage
            let is_listening = assistant.phase == AssistantPhase::Listening;
            let mic_hov = hovered == Some(HitTarget::ZekiMicToggle);
            let mic_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + ZEKI_MIC_BTN_CHAT.left,
                    top: origin_y + ZEKI_MIC_BTN_CHAT.top,
                    right: origin_x + ZEKI_MIC_BTN_CHAT.right,
                    bottom: origin_y + ZEKI_MIC_BTN_CHAT.bottom,
                },
                radiusX: 11.0,
                radiusY: 11.0,
            };
            let (mr, mg, mb) = if is_listening {
                (0.94, 0.27, 0.35)
            } else {
                (ar, ag, ab)
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: mr,
                g: mg,
                b: mb,
                a: if is_listening {
                    0.34
                } else if mic_hov {
                    0.26
                } else {
                    0.15
                } * alpha,
            });
            self.target
                .FillRoundedRectangle(&mic_rect, &self.dynamic_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: mr,
                g: mg,
                b: mb,
                a: if is_listening || mic_hov { 0.85 } else { 0.45 } * alpha,
            });
            self.target
                .DrawRoundedRectangle(&mic_rect, &self.dynamic_brush, 1.0, None);

            let mic_label = if is_listening {
                "Stop Mic".to_string()
            } else if assistant.phase.is_busy() {
                "Cancel".to_string()
            } else {
                format!("Mic ({})", frame.ptt_hotkey_short)
            };
            let mic_u16: Vec<u16> = mic_label.encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &mic_u16,
                &self.pill_btn_format,
                ZEKI_MIC_BTN_CHAT.width(),
                ZEKI_MIC_BTN_CHAT.height(),
            ) {
                self.title_brush.SetOpacity(0.98 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + ZEKI_MIC_BTN_CHAT.left,
                        y: origin_y + ZEKI_MIC_BTN_CHAT.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // Quick Config button beneath Mic button
            let cfg_hov = hovered == Some(HitTarget::ZekiOpenConfig);
            let cfg_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + ZEKI_CFG_BTN_CHAT.left,
                    top: origin_y + ZEKI_CFG_BTN_CHAT.top,
                    right: origin_x + ZEKI_CFG_BTN_CHAT.right,
                    bottom: origin_y + ZEKI_CFG_BTN_CHAT.bottom,
                },
                radiusX: 11.0,
                radiusY: 11.0,
            };
            self.hover_brush
                .SetOpacity(if cfg_hov { 0.16 } else { 0.065 } * alpha);
            self.target.FillRoundedRectangle(&cfg_rect, &self.hover_brush);
            let cfg_u16: Vec<u16> = "Config".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &cfg_u16,
                &self.pill_btn_format,
                ZEKI_CFG_BTN_CHAT.width(),
                ZEKI_CFG_BTN_CHAT.height(),
            ) {
                self.title_brush
                    .SetOpacity(if cfg_hov { 0.98 } else { 0.78 } * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + ZEKI_CFG_BTN_CHAT.left,
                        y: origin_y + ZEKI_CFG_BTN_CHAT.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // Privacy Toggle button (Screen Vision ON <-> Incognito Mode)
            self.draw_privacy_pill(
                origin_x,
                origin_y,
                &ZEKI_PRIVACY_BTN_CHAT,
                assistant.screen_access_enabled,
                hovered == Some(HitTarget::ZekiTogglePrivacy),
                alpha,
            );

            // Subtle scroll & double-click tip below Privacy button
            if pill_height >= 300.0 {
                let tip = if frame.chat_large {
                    "Scroll / drag chat\n2x click to shrink"
                } else {
                    "Scroll / drag chat\n2x click to expand"
                };
                let tip_u16: Vec<u16> = tip.encode_utf16().collect();
                if let Ok(layout) =
                    self.dwrite
                        .CreateTextLayout(&tip_u16, &self.center_sub_format, 100.0, 36.0)
                {
                    self.notif_body_brush.SetOpacity(0.50 * alpha);
                    self.target.DrawTextLayout(
                        D2D_POINT_2F {
                            x: origin_x + 18.0,
                            y: origin_y + 266.0,
                        },
                        &layout,
                        &self.notif_body_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }

            // Right header: phase title
            let header_text = match assistant.phase {
                AssistantPhase::Listening => "Listening...".to_string(),
                AssistantPhase::Transcribing => "Transcribing...".to_string(),
                AssistantPhase::Thinking => format!("Thinking • {}", assistant.provider_label),
                AssistantPhase::Speaking => format!("Zeki • {}", assistant.provider_label),
                AssistantPhase::Done => format!("Zeki • {}", assistant.provider_label),
                AssistantPhase::Error => "Assistant Notice".to_string(),
                AssistantPhase::Idle => format!("Zeki • {}", assistant.provider_label),
            };
            let head_w = (cl.expand_btn.left - 138.0).max(90.0);
            let head_u16: Vec<u16> = header_text.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&head_u16, &self.notif_title_format, head_w, 22.0)
            {
                self.title_brush.SetOpacity(0.98 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + 132.0,
                        y: origin_y + 47.0,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // Action button 0: Expand / Shrink toggle (same as double-clicking the chat box)
            let exp_hov = hovered == Some(HitTarget::ZekiChatToggleSize);
            let exp_btn = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + cl.expand_btn.left,
                    top: origin_y + cl.expand_btn.top,
                    right: origin_x + cl.expand_btn.right,
                    bottom: origin_y + cl.expand_btn.bottom,
                },
                radiusX: 10.5,
                radiusY: 10.5,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: if frame.chat_large {
                    0.26
                } else if exp_hov {
                    0.22
                } else {
                    0.10
                } * alpha,
            });
            self.target
                .FillRoundedRectangle(&exp_btn, &self.dynamic_brush);
            let exp_lbl = if frame.chat_large { "Shrink" } else { "Expand" };
            let exp_u16: Vec<u16> = exp_lbl.encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &exp_u16,
                &self.pill_btn_format,
                cl.expand_btn.width(),
                cl.expand_btn.height(),
            ) {
                self.title_brush
                    .SetOpacity(if exp_hov || frame.chat_large { 1.0 } else { 0.85 } * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + cl.expand_btn.left,
                        y: origin_y + cl.expand_btn.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // Action button 0b: Explicit Insert (pastes Zeki's reply into user's active app cursor)
            if !assistant.reply.is_empty() {
                let ins_hov = hovered == Some(HitTarget::ZekiInsertReply);
                let ins_btn = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: origin_x + cl.insert_btn.left,
                        top: origin_y + cl.insert_btn.top,
                        right: origin_x + cl.insert_btn.right,
                        bottom: origin_y + cl.insert_btn.bottom,
                    },
                    radiusX: 10.5,
                    radiusY: 10.5,
                };
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 0.14,
                    g: 0.83,
                    b: 0.62,
                    a: if ins_hov { 0.36 } else { 0.20 } * alpha,
                });
                self.target
                    .FillRoundedRectangle(&ins_btn, &self.dynamic_brush);
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 0.14,
                    g: 0.83,
                    b: 0.62,
                    a: if ins_hov { 0.92 } else { 0.52 } * alpha,
                });
                self.target
                    .DrawRoundedRectangle(&ins_btn, &self.dynamic_brush, 1.0, None);
                let ins_u16: Vec<u16> = "Insert".encode_utf16().collect();
                if let Ok(layout) = self.dwrite.CreateTextLayout(
                    &ins_u16,
                    &self.pill_btn_format,
                    cl.insert_btn.width(),
                    cl.insert_btn.height(),
                ) {
                    self.title_brush.SetOpacity(0.98 * alpha);
                    self.target.DrawTextLayout(
                        D2D_POINT_2F {
                            x: origin_x + cl.insert_btn.left,
                            y: origin_y + cl.insert_btn.top,
                        },
                        &layout,
                        &self.title_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }

            // Action button 1: Copy (when reply exists) OR Config (when error exists)
            let show_copy = !assistant.reply.is_empty();
            let show_cfg = !show_copy && assistant.error.is_some();
            if show_copy || show_cfg {
                let target_id = if show_copy {
                    HitTarget::ZekiCopyReply
                } else {
                    HitTarget::ZekiOpenConfig
                };
                let hov = hovered == Some(target_id);
                let btn = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: origin_x + cl.copy_btn.left,
                        top: origin_y + cl.copy_btn.top,
                        right: origin_x + cl.copy_btn.right,
                        bottom: origin_y + cl.copy_btn.bottom,
                    },
                    radiusX: 10.5,
                    radiusY: 10.5,
                };
                self.hover_brush
                    .SetOpacity(if hov { 0.18 } else { 0.08 } * alpha);
                self.target.FillRoundedRectangle(&btn, &self.hover_brush);
                let txt = if show_copy {
                    if frame.clip_copied_flash { "Copied!" } else { "Copy" }
                } else {
                    "Config"
                };
                let u16_t: Vec<u16> = txt.encode_utf16().collect();
                if let Ok(layout) = self.dwrite.CreateTextLayout(
                    &u16_t,
                    &self.pill_btn_format,
                    cl.copy_btn.width(),
                    cl.copy_btn.height(),
                ) {
                    self.title_brush.SetOpacity(if hov { 1.0 } else { 0.85 } * alpha);
                    self.target.DrawTextLayout(
                        D2D_POINT_2F {
                            x: origin_x + cl.copy_btn.left,
                            y: origin_y + cl.copy_btn.top,
                        },
                        &layout,
                        &self.title_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }

            // Action button 2: Clear / Back
            let clear_hov = hovered == Some(HitTarget::ZekiClearChat);
            let clear_btn = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + cl.clear_btn.left,
                    top: origin_y + cl.clear_btn.top,
                    right: origin_x + cl.clear_btn.right,
                    bottom: origin_y + cl.clear_btn.bottom,
                },
                radiusX: 10.5,
                radiusY: 10.5,
            };
            self.hover_brush
                .SetOpacity(if clear_hov { 0.18 } else { 0.08 } * alpha);
            self.target
                .FillRoundedRectangle(&clear_btn, &self.hover_brush);
            let clr_u16: Vec<u16> = "Clear".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &clr_u16,
                &self.pill_btn_format,
                cl.clear_btn.width(),
                cl.clear_btn.height(),
            ) {
                self.title_brush
                    .SetOpacity(if clear_hov { 1.0 } else { 0.82 } * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + cl.clear_btn.left,
                        y: origin_y + cl.clear_btn.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // Scrollable Chat Card surface (double-click to enlarge, wheel or drag to scroll!)
            let card_hov = hovered == Some(HitTarget::ZekiChatCard);
            let card = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + cl.card.left,
                    top: origin_y + cl.card.top,
                    right: origin_x + cl.card.right,
                    bottom: origin_y + cl.card.bottom,
                },
                radiusX: 14.0,
                radiusY: 14.0,
            };
            self.hover_brush
                .SetOpacity(if card_hov { 0.07 } else { 0.055 } * alpha);
            self.target.FillRoundedRectangle(&card, &self.hover_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: if card_hov { 0.36 } else { 0.22 } * alpha,
            });
            self.target
                .DrawRoundedRectangle(&card, &self.dynamic_brush, 1.0, None);

            // Build complete multi-turn conversation text so the user can scroll through everything
            let (full_chat_text, is_err, is_placeholder_body) = build_chat_transcript(assistant);

            let pad_l = 11.0f32;
            let pad_r = 14.0f32;
            let pad_v = 9.0f32;
            let text_w = (cl.card.width() - pad_l - pad_r).max(60.0);
            let view_h = (cl.card.height() - pad_v * 2.0).max(24.0);

            let chat_u16: Vec<u16> = full_chat_text.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&chat_u16, &self.notif_body_format, text_w, 12_000.0)
            {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                let content_h = if layout.GetMetrics(&mut metrics).is_ok() {
                    metrics.height
                } else {
                    view_h
                };
                let max_scroll = (content_h - view_h).max(0.0);
                self.last_chat_max_scroll = max_scroll;

                let scroll_y = if frame.chat_auto_scroll {
                    max_scroll
                } else {
                    frame.chat_scroll_y.clamp(0.0, max_scroll)
                };

                // Clip strictly to the inside of the chat card so scrolled text never bleeds out
                let card_clip = D2D_RECT_F {
                    left: origin_x + cl.card.left + 6.0,
                    top: origin_y + cl.card.top + 6.0,
                    right: origin_x + cl.card.right - 9.0,
                    bottom: origin_y + cl.card.bottom - 6.0,
                };
                self.target
                    .PushAxisAlignedClip(&card_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

                let draw_pt = D2D_POINT_2F {
                    x: origin_x + cl.card.left + pad_l,
                    y: origin_y + cl.card.top + pad_v - scroll_y,
                };

                if is_err {
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: 0.99,
                        g: 0.62,
                        b: 0.66,
                        a: 0.96 * alpha,
                    });
                    self.target.DrawTextLayout(
                        draw_pt,
                        &layout,
                        &self.dynamic_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                } else {
                    self.notif_title_brush
                        .SetOpacity(if is_placeholder_body { 0.66 } else { 0.95 } * alpha);
                    self.target.DrawTextLayout(
                        draw_pt,
                        &layout,
                        &self.notif_title_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }

                self.target.PopAxisAlignedClip();

                // Sleek vertical scrollbar thumb when content exceeds viewport
                if max_scroll > 1.0 {
                    let track_top = origin_y + cl.card.top + 10.0;
                    let track_h = (cl.card.height() - 20.0).max(20.0);
                    let thumb_h = ((view_h / content_h.max(1.0)) * track_h).clamp(22.0, track_h);
                    let ratio = (scroll_y / max_scroll).clamp(0.0, 1.0);
                    let thumb_top = track_top + ratio * (track_h - thumb_h);
                    let sb_x = origin_x + cl.card.right - 7.0;

                    let track_rect = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: sb_x,
                            top: track_top,
                            right: sb_x + 3.5,
                            bottom: track_top + track_h,
                        },
                        radiusX: 1.75,
                        radiusY: 1.75,
                    };
                    self.hover_brush.SetOpacity(0.10 * alpha);
                    self.target
                        .FillRoundedRectangle(&track_rect, &self.hover_brush);

                    let thumb_rect = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: sb_x,
                            top: thumb_top,
                            right: sb_x + 3.5,
                            bottom: thumb_top + thumb_h,
                        },
                        radiusX: 1.75,
                        radiusY: 1.75,
                    };
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: ar,
                        g: ag,
                        b: ab,
                        a: 0.72 * alpha,
                    });
                    self.target
                        .FillRoundedRectangle(&thumb_rect, &self.dynamic_brush);
                }
            } else {
                self.last_chat_max_scroll = 0.0;
            }

            // Optional Attached File chip above input bar (click to detach)
            if let Some(file_lbl) = frame.attached_file_label {
                let chip_hov = hovered == Some(HitTarget::ZekiRemoveFile);
                let chip = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: origin_x + cl.file_chip.left,
                        top: origin_y + cl.file_chip.top,
                        right: origin_x + cl.file_chip.right,
                        bottom: origin_y + cl.file_chip.bottom,
                    },
                    radiusX: 9.0,
                    radiusY: 9.0,
                };
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: ar,
                    g: ag,
                    b: ab,
                    a: if chip_hov { 0.28 } else { 0.16 } * alpha,
                });
                self.target
                    .FillRoundedRectangle(&chip, &self.dynamic_brush);
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: ar,
                    g: ag,
                    b: ab,
                    a: 0.55 * alpha,
                });
                self.target
                    .DrawRoundedRectangle(&chip, &self.dynamic_brush, 0.9, None);

                let chip_text = format!("File: {file_lbl}   (click to remove ×)");
                if let Ok(layout) = create_layout(
                    &self.dwrite,
                    &self.notif_app_format,
                    &self.notif_body_trim,
                    &chip_text,
                    cl.file_chip.width() - 16.0,
                    16.0,
                ) {
                    self.title_brush.SetOpacity(0.94 * alpha);
                    self.target.DrawTextLayout(
                        D2D_POINT_2F {
                            x: origin_x + cl.file_chip.left + 8.0,
                            y: origin_y + cl.file_chip.top + 2.5,
                        },
                        &layout,
                        &self.title_brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }

            // +File attach button on the left of the input bar
            let att_hov = hovered == Some(HitTarget::ZekiAttachFile);
            let att_btn = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + cl.attach_btn.left,
                    top: origin_y + cl.attach_btn.top,
                    right: origin_x + cl.attach_btn.right,
                    bottom: origin_y + cl.attach_btn.bottom,
                },
                radiusX: 11.0,
                radiusY: 11.0,
            };
            self.hover_brush
                .SetOpacity(if att_hov { 0.18 } else { 0.08 } * alpha);
            self.target.FillRoundedRectangle(&att_btn, &self.hover_brush);
            if has_file || att_hov {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: ar,
                    g: ag,
                    b: ab,
                    a: 0.65 * alpha,
                });
                self.target
                    .DrawRoundedRectangle(&att_btn, &self.dynamic_brush, 1.0, None);
            }
            let att_u16: Vec<u16> = "+File".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &att_u16,
                &self.pill_btn_format,
                cl.attach_btn.width(),
                cl.attach_btn.height(),
            ) {
                self.title_brush
                    .SetOpacity(if has_file || att_hov { 0.98 } else { 0.82 } * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + cl.attach_btn.left,
                        y: origin_y + cl.attach_btn.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // Bottom chat input box + Send button
            let input_hov = hovered == Some(HitTarget::ZekiChatInput);
            let input_box = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + cl.input.left,
                    top: origin_y + cl.input.top,
                    right: origin_x + cl.input.right,
                    bottom: origin_y + cl.input.bottom,
                },
                radiusX: 11.0,
                radiusY: 11.0,
            };
            self.hover_brush.SetOpacity(
                if frame.chat_focused {
                    0.11
                } else if input_hov {
                    0.09
                } else {
                    0.06
                } * alpha,
            );
            self.target
                .FillRoundedRectangle(&input_box, &self.hover_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: if frame.chat_focused {
                    0.82
                } else if input_hov {
                    0.45
                } else {
                    0.22
                } * alpha,
            });
            self.target.DrawRoundedRectangle(
                &input_box,
                &self.dynamic_brush,
                if frame.chat_focused { 1.3 } else { 1.0 },
                None,
            );

            // Blinking caret every ~530ms when focused
            let caret_on = frame.chat_focused
                && ((std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() / 530)
                    .unwrap_or(0)
                    % 2)
                    == 0);

            let max_visible_chars = ((cl.input.width() - 22.0) / 6.5).max(24.0) as usize;
            let (input_display, is_placeholder) = if frame.chat_input.is_empty() {
                if frame.chat_focused {
                    (if caret_on { "|".to_string() } else { String::new() }, false)
                } else {
                    ("Ketik pesan ke Zeki...".to_string(), true)
                }
            } else {
                let chars: Vec<char> = frame.chat_input.chars().collect();
                let mut s: String = if chars.len() > max_visible_chars {
                    format!(
                        "…{}",
                        chars[chars.len() - (max_visible_chars - 1)..]
                            .iter()
                            .collect::<String>()
                    )
                } else {
                    frame.chat_input.to_string()
                };
                if caret_on {
                    s.push('|');
                }
                (s, false)
            };

            if !input_display.is_empty() {
                let u16_in: Vec<u16> = input_display.encode_utf16().collect();
                if let Ok(layout) = self.dwrite.CreateTextLayout(
                    &u16_in,
                    &self.notif_body_format,
                    cl.input.width() - 18.0,
                    20.0,
                ) {
                    if is_placeholder {
                        self.notif_body_brush.SetOpacity(0.55 * alpha);
                        self.target.DrawTextLayout(
                            D2D_POINT_2F {
                                x: origin_x + cl.input.left + 10.0,
                                y: origin_y + cl.input.top + 8.0,
                            },
                            &layout,
                            &self.notif_body_brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    } else {
                        self.title_brush.SetOpacity(0.96 * alpha);
                        self.target.DrawTextLayout(
                            D2D_POINT_2F {
                                x: origin_x + cl.input.left + 10.0,
                                y: origin_y + cl.input.top + 8.0,
                            },
                            &layout,
                            &self.title_brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
            }

            // Send button
            let has_text = !frame.chat_input.trim().is_empty();
            let send_hov = hovered == Some(HitTarget::ZekiChatSend);
            let send_btn = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + cl.send_btn.left,
                    top: origin_y + cl.send_btn.top,
                    right: origin_x + cl.send_btn.right,
                    bottom: origin_y + cl.send_btn.bottom,
                },
                radiusX: 11.0,
                radiusY: 11.0,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: if has_text {
                    if send_hov { 0.44 } else { 0.30 }
                } else {
                    0.10
                } * alpha,
            });
            self.target
                .FillRoundedRectangle(&send_btn, &self.dynamic_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: if has_text { 0.85 } else { 0.28 } * alpha,
            });
            self.target
                .DrawRoundedRectangle(&send_btn, &self.dynamic_brush, 1.0, None);
            let send_u16: Vec<u16> = "Send".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &send_u16,
                &self.pill_btn_format,
                cl.send_btn.width(),
                cl.send_btn.height(),
            ) {
                self.title_brush
                    .SetOpacity(if has_text { 0.98 } else { 0.50 } * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + cl.send_btn.left,
                        y: origin_y + cl.send_btn.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            return;
        }
        self.last_chat_max_scroll = 0.0;

        // Branch 2: Default Zeki tab — Chat + Mic + Config + Privacy Toggle + 7 State Preview Buttons
        self.draw_privacy_pill(
            origin_x,
            origin_y,
            &ZEKI_PRIVACY_BTN_PREVIEW,
            assistant.screen_access_enabled,
            hovered == Some(HitTarget::ZekiTogglePrivacy),
            alpha,
        );

        let heading: Vec<u16> = "Zeki".encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&heading, &self.title_format, 76.0, 26.0)
        {
            self.title_brush.SetOpacity(0.98 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + 132.0,
                    y: origin_y + 43.0,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Quick Chat button
        let chat_hov = hovered == Some(HitTarget::ZekiOpenChat);
        let chat_btn = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + ZEKI_CHAT_BTN_PREVIEW.left,
                top: origin_y + ZEKI_CHAT_BTN_PREVIEW.top,
                right: origin_x + ZEKI_CHAT_BTN_PREVIEW.right,
                bottom: origin_y + ZEKI_CHAT_BTN_PREVIEW.bottom,
            },
            radiusX: 11.0,
            radiusY: 11.0,
        };
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: if chat_hov { 0.34 } else { 0.22 } * alpha,
        });
        self.target
            .FillRoundedRectangle(&chat_btn, &self.dynamic_brush);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: if chat_hov { 0.95 } else { 0.62 } * alpha,
        });
        self.target
            .DrawRoundedRectangle(&chat_btn, &self.dynamic_brush, 1.0, None);
        let chat_lbl: Vec<u16> = "Chat".encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &chat_lbl,
            &self.pill_btn_format,
            ZEKI_CHAT_BTN_PREVIEW.width(),
            ZEKI_CHAT_BTN_PREVIEW.height(),
        ) {
            self.title_brush.SetOpacity(0.98 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + ZEKI_CHAT_BTN_PREVIEW.left,
                    y: origin_y + ZEKI_CHAT_BTN_PREVIEW.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Quick Mic button (Hands-free VAD click-to-talk)
        let mic_hov = hovered == Some(HitTarget::ZekiMicToggle);
        let mic_btn = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + ZEKI_MIC_BTN_PREVIEW.left,
                top: origin_y + ZEKI_MIC_BTN_PREVIEW.top,
                right: origin_x + ZEKI_MIC_BTN_PREVIEW.right,
                bottom: origin_y + ZEKI_MIC_BTN_PREVIEW.bottom,
            },
            radiusX: 11.0,
            radiusY: 11.0,
        };
        self.hover_brush
            .SetOpacity(if mic_hov { 0.18 } else { 0.085 } * alpha);
        self.target.FillRoundedRectangle(&mic_btn, &self.hover_brush);
        let mic_lbl: Vec<u16> = "Mic".encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &mic_lbl,
            &self.pill_btn_format,
            ZEKI_MIC_BTN_PREVIEW.width(),
            ZEKI_MIC_BTN_PREVIEW.height(),
        ) {
            self.title_brush.SetOpacity(0.98 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + ZEKI_MIC_BTN_PREVIEW.left,
                    y: origin_y + ZEKI_MIC_BTN_PREVIEW.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Config button (opens assistant.ini)
        let cfg_hov = hovered == Some(HitTarget::ZekiOpenConfig);
        let cfg_btn = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + ZEKI_CFG_BTN_PREVIEW.left,
                top: origin_y + ZEKI_CFG_BTN_PREVIEW.top,
                right: origin_x + ZEKI_CFG_BTN_PREVIEW.right,
                bottom: origin_y + ZEKI_CFG_BTN_PREVIEW.bottom,
            },
            radiusX: 11.0,
            radiusY: 11.0,
        };
        self.hover_brush
            .SetOpacity(if cfg_hov { 0.18 } else { 0.075 } * alpha);
        self.target.FillRoundedRectangle(&cfg_btn, &self.hover_brush);
        let cfg_lbl: Vec<u16> = "Config".encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &cfg_lbl,
            &self.pill_btn_format,
            ZEKI_CFG_BTN_PREVIEW.width(),
            ZEKI_CFG_BTN_PREVIEW.height(),
        ) {
            self.title_brush
                .SetOpacity(if cfg_hov { 1.0 } else { 0.80 } * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + ZEKI_CFG_BTN_PREVIEW.left,
                    y: origin_y + ZEKI_CFG_BTN_PREVIEW.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Status subtitle line
        let status_str = if zeki.state() != crate::render::zeki::ZekiState::Idle {
            format!("{} • {}", zeki.state().label(), zeki.state().status_line())
        } else if assistant.has_config_key {
            format!("Hold {} to talk • {}", frame.ptt_hotkey_short, assistant.provider_label)
        } else {
            "Click Config -> OpenRouter / OpenCode / NVIDIA / Gemini".to_string()
        };
        let status: Vec<u16> = status_str.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&status, &self.notif_body_format, 290.0, 18.0)
        {
            self.notif_body_brush.SetOpacity(0.82 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + 132.0,
                    y: origin_y + 70.0,
                },
                &layout,
                &self.notif_body_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // 7 interactive state preview buttons (2 balanced rows)
        for (i, &state) in ALL_ZEKI_STATES.iter().enumerate() {
            let rect = zeki_state_btn_rect(i);
            let is_active = zeki.state() == state;
            let is_hov = hovered == Some(HitTarget::ZekiSelectState(state));
            let (sr, sg, sb) = state.accent_rgb();

            let btn = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + rect.left,
                    top: origin_y + rect.top,
                    right: origin_x + rect.right,
                    bottom: origin_y + rect.bottom,
                },
                radiusX: 11.5,
                radiusY: 11.5,
            };

            if is_active {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: sr,
                    g: sg,
                    b: sb,
                    a: 0.28 * alpha,
                });
                self.target.FillRoundedRectangle(&btn, &self.dynamic_brush);
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: sr,
                    g: sg,
                    b: sb,
                    a: 0.85 * alpha,
                });
                self.target
                    .DrawRoundedRectangle(&btn, &self.dynamic_brush, 1.0, None);
            } else {
                self.hover_brush
                    .SetOpacity(if is_hov { 0.16 } else { 0.065 } * alpha);
                self.target.FillRoundedRectangle(&btn, &self.hover_brush);
                if is_hov {
                    self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                        r: sr,
                        g: sg,
                        b: sb,
                        a: 0.45 * alpha,
                    });
                    self.target
                        .DrawRoundedRectangle(&btn, &self.dynamic_brush, 1.0, None);
                }
            }

            // Tiny mood color dot inside each button
            let dot_x = origin_x + rect.left + 11.0;
            let dot_y = origin_y + rect.top + rect.height() / 2.0;
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: sr,
                g: sg,
                b: sb,
                a: if is_active { 1.0 } else { 0.72 } * alpha,
            });
            self.target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: dot_x, y: dot_y },
                    radiusX: 3.0,
                    radiusY: 3.0,
                },
                &self.dynamic_brush,
            );

            let lbl: Vec<u16> = state.short_label().encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &lbl,
                &self.pill_btn_format,
                rect.width() - 12.0,
                rect.height(),
            ) {
                self.title_brush
                    .SetOpacity(if is_active { 1.0 } else if is_hov { 0.94 } else { 0.74 } * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + rect.left + 10.0,
                        y: origin_y + rect.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    }

    /// Persistent Zeki Companion Dock shown at the bottom of every non-Zeki sub-menu
    /// (`Media`, `Timer`, `Clip`, `Sys`, `Notifs`) with live Zeki + instant Chat & Listen buttons.
    unsafe fn draw_zeki_dock(&self, origin_x: f32, origin_y: f32, frame: &Frame, alpha: f32) {
        let (ar, ag, ab) = frame.zeki.accent_color();
        let hovered = frame.hovered_target;

        let dock = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + ZEKI_DOCK_RECT.left,
                top: origin_y + ZEKI_DOCK_RECT.top,
                right: origin_x + ZEKI_DOCK_RECT.right,
                bottom: origin_y + ZEKI_DOCK_RECT.bottom,
            },
            radiusX: 14.0,
            radiusY: 14.0,
        };
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: 0.075 * alpha,
        });
        self.target.FillRoundedRectangle(&dock, &self.dynamic_brush);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: 0.24 * alpha,
        });
        self.target
            .DrawRoundedRectangle(&dock, &self.dynamic_brush, 1.0, None);

        // Living Mini Zeki on the left of the dock
        let dock_cy = origin_y + ZEKI_DOCK_RECT.top + ZEKI_DOCK_RECT.height() / 2.0;
        frame.zeki.draw(
            &self.factory,
            &self.target,
            &self.dynamic_brush,
            self.round_stroke.as_ref(),
            &self.dwrite,
            &self.badge_format,
            origin_x + 35.0,
            dock_cy + 1.0,
            10.5,
            alpha,
            true,
        );

        // Middle interactive status / quick-chat prompt (or live mic waveform when Listening)
        if frame.assistant.phase == AssistantPhase::Listening {
            let lbl: Vec<u16> = "Listening... speak now".encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&lbl, &self.notif_title_format, 146.0, 18.0)
            {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: (ar * 0.45 + 0.55).min(1.0),
                    g: (ag * 0.45 + 0.55).min(1.0),
                    b: (ab * 0.45 + 0.55).min(1.0),
                    a: 0.98 * alpha,
                });
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + 56.0,
                        y: dock_cy - 8.5,
                    },
                    &layout,
                    &self.dynamic_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            // Live 5-bar voice level indicator inside dock
            let level = frame.voice_level.clamp(0.08, 1.0);
            let weights = [0.55f32, 0.85, 1.0, 0.80, 0.50];
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: 0.95 * alpha,
            });
            for (i, &mul) in weights.iter().enumerate() {
                let bh = (4.0 + 13.0 * level * mul).clamp(4.0, 17.0);
                let bx = origin_x + 216.0 + i as f32 * 6.0;
                let bar = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: bx,
                        top: dock_cy - bh / 2.0,
                        right: bx + 3.0,
                        bottom: dock_cy + bh / 2.0,
                    },
                    radiusX: 1.5,
                    radiusY: 1.5,
                };
                self.target.FillRoundedRectangle(&bar, &self.dynamic_brush);
            }
        } else {
            let prompt_text = match frame.assistant.phase {
                AssistantPhase::Transcribing => "Transcribing voice...".to_string(),
                AssistantPhase::Thinking if frame.assistant.screen_capturing_now => {
                    "Seeing screen...".to_string()
                }
                AssistantPhase::Thinking => format!("Thinking ({})...", frame.assistant.provider_label),
                AssistantPhase::Speaking => "Zeki replying...".to_string(),
                _ => format!("Ask Zeki ({})", frame.assistant.provider_label),
            };
            if let Ok(layout) = create_layout(
                &self.dwrite,
                &self.notif_body_format,
                &self.notif_body_trim,
                &prompt_text,
                156.0,
                18.0,
            ) {
                self.notif_body_brush.SetOpacity(0.84 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + 56.0,
                        y: dock_cy - 8.0,
                    },
                    &layout,
                    &self.notif_body_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        // Quick Privacy Toggle inside Companion Dock (Screen ON / Incognito)
        self.draw_privacy_pill(
            origin_x,
            origin_y,
            &ZEKI_DOCK_PRIVACY_BTN,
            frame.assistant.screen_access_enabled,
            hovered == Some(HitTarget::ZekiTogglePrivacy),
            alpha,
        );

        // Quick Chat button
        let chat_hov = hovered == Some(HitTarget::ZekiOpenChat);
        let chat_btn = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + ZEKI_DOCK_CHAT_BTN.left,
                top: origin_y + ZEKI_DOCK_CHAT_BTN.top,
                right: origin_x + ZEKI_DOCK_CHAT_BTN.right,
                bottom: origin_y + ZEKI_DOCK_CHAT_BTN.bottom,
            },
            radiusX: 12.0,
            radiusY: 12.0,
        };
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: if chat_hov { 0.34 } else { 0.20 } * alpha,
        });
        self.target
            .FillRoundedRectangle(&chat_btn, &self.dynamic_brush);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: if chat_hov { 0.92 } else { 0.56 } * alpha,
        });
        self.target
            .DrawRoundedRectangle(&chat_btn, &self.dynamic_brush, 1.0, None);
        let chat_u16: Vec<u16> = "Chat".encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &chat_u16,
            &self.pill_btn_format,
            ZEKI_DOCK_CHAT_BTN.width(),
            ZEKI_DOCK_CHAT_BTN.height(),
        ) {
            self.title_brush.SetOpacity(0.98 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + ZEKI_DOCK_CHAT_BTN.left,
                    y: origin_y + ZEKI_DOCK_CHAT_BTN.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Quick Listen / Stop button
        let is_listening = frame.assistant.phase == AssistantPhase::Listening;
        let mic_hov = hovered == Some(HitTarget::ZekiMicToggle);
        let mic_btn = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + ZEKI_DOCK_MIC_BTN.left,
                top: origin_y + ZEKI_DOCK_MIC_BTN.top,
                right: origin_x + ZEKI_DOCK_MIC_BTN.right,
                bottom: origin_y + ZEKI_DOCK_MIC_BTN.bottom,
            },
            radiusX: 12.0,
            radiusY: 12.0,
        };
        if is_listening {
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 0.94,
                g: 0.27,
                b: 0.35,
                a: 0.38 * alpha,
            });
            self.target
                .FillRoundedRectangle(&mic_btn, &self.dynamic_brush);
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: 0.98,
                g: 0.38,
                b: 0.45,
                a: 0.92 * alpha,
            });
            self.target
                .DrawRoundedRectangle(&mic_btn, &self.dynamic_brush, 1.0, None);
        } else {
            self.hover_brush
                .SetOpacity(if mic_hov { 0.20 } else { 0.09 } * alpha);
            self.target
                .FillRoundedRectangle(&mic_btn, &self.hover_brush);
            if mic_hov {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: ar,
                    g: ag,
                    b: ab,
                    a: 0.65 * alpha,
                });
                self.target
                    .DrawRoundedRectangle(&mic_btn, &self.dynamic_brush, 1.0, None);
            }
        }
        let mic_lbl = if is_listening {
            "Stop"
        } else if frame.assistant.phase.is_busy() {
            "Cancel"
        } else {
            "Listen"
        };
        let mic_u16: Vec<u16> = mic_lbl.encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &mic_u16,
            &self.pill_btn_format,
            ZEKI_DOCK_MIC_BTN.width(),
            ZEKI_DOCK_MIC_BTN.height(),
        ) {
            self.title_brush
                .SetOpacity(if is_listening || mic_hov { 1.0 } else { 0.88 } * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + ZEKI_DOCK_MIC_BTN.left,
                    y: origin_y + ZEKI_DOCK_MIC_BTN.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    /// Draws a compact Privacy Toggle pill (`Screen: ON` / `Incognito`, or compact `Eye` / `Priv` in dock).
    unsafe fn draw_privacy_pill(
        &self,
        origin_x: f32,
        origin_y: f32,
        rect: &RectF,
        screen_on: bool,
        hovered: bool,
        alpha: f32,
    ) {
        let pill = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + rect.left,
                top: origin_y + rect.top,
                right: origin_x + rect.right,
                bottom: origin_y + rect.bottom,
            },
            radiusX: rect.height() / 2.0,
            radiusY: rect.height() / 2.0,
        };

        let (r, g, b) = if screen_on {
            (0.14, 0.83, 0.72) // Emerald-Cyan: Screen Vision active
        } else {
            (0.96, 0.68, 0.22) // Warm Amber: Incognito / Private
        };

        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r,
            g,
            b,
            a: if hovered { 0.28 } else { 0.14 } * alpha,
        });
        self.target.FillRoundedRectangle(&pill, &self.dynamic_brush);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r,
            g,
            b,
            a: if hovered { 0.88 } else { 0.48 } * alpha,
        });
        self.target
            .DrawRoundedRectangle(&pill, &self.dynamic_brush, 1.0, None);

        // Status dot inside button
        let dot_x = origin_x + rect.left + 10.0;
        let dot_y = origin_y + rect.top + rect.height() / 2.0;
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r,
            g,
            b,
            a: 0.98 * alpha,
        });
        self.target.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: dot_x, y: dot_y },
                radiusX: 3.0,
                radiusY: 3.0,
            },
            &self.dynamic_brush,
        );

        let compact = rect.width() < 75.0;
        let label = match (screen_on, compact) {
            (true, false) => "Screen: ON",
            (false, false) => "Incognito",
            (true, true) => "Eye",
            (false, true) => "Priv",
        };
        let u16_lbl: Vec<u16> = label.encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &u16_lbl,
            &self.pill_btn_format,
            rect.width() - 12.0,
            rect.height(),
        ) {
            self.title_brush
                .SetOpacity(if hovered { 1.0 } else { 0.90 } * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + rect.left + 9.0,
                    y: origin_y + rect.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    unsafe fn draw_expanded_media(&self, origin_x: f32, origin_y: f32, frame: &Frame, alpha: f32) {
        // Media logo / album art on the top-left directly above the clock
        self.draw_art(origin_x, origin_y, &EXPANDED_ART, EXPANDED_ART_RADIUS);

        // Bold clock centered directly beneath the media logo on the left
        let clock = current_clock_string();
        let clock_u16: Vec<u16> = clock.encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &clock_u16,
            &self.clock_media_format,
            MEDIA_CLOCK_BOX.width(),
            MEDIA_CLOCK_BOX.height(),
        ) {
            self.title_brush.SetOpacity(0.96 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + MEDIA_CLOCK_BOX.left,
                    y: origin_y + MEDIA_CLOCK_BOX.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Right column: title, artist, playback controls, and live 5-bar FFT equalizer
        self.draw_text(origin_x, origin_y, alpha);

        let hovered_ctrl = match frame.hovered_target {
            Some(HitTarget::MediaControl(c)) => Some(c),
            _ => None,
        };
        self.draw_controls(origin_x, origin_y, alpha, frame.media.is_playing, hovered_ctrl);

        self.draw_equalizer(
            origin_x,
            origin_y,
            CONTROLS_TOP + 15.0,
            &frame.eq_levels,
            frame.media.is_playing,
            alpha,
            416.0,
            4.5,
            4.0,
            4.0,
            22.0,
        );
    }

    unsafe fn draw_idle_expanded(
        &self,
        pill_left: f32,
        pill_top: f32,
        pill_width: f32,
        _pill_height: f32,
        system: &SystemSnapshot,
        notif: &NotificationSnapshot,
        alpha: f32,
    ) {
        // Media logo badge on the top-left directly above the clock
        let logo_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: pill_left + EXPANDED_ART.left,
                top: pill_top + EXPANDED_ART.top,
                right: pill_left + EXPANDED_ART.right,
                bottom: pill_top + EXPANDED_ART.bottom,
            },
            radiusX: EXPANDED_ART_RADIUS,
            radiusY: EXPANDED_ART_RADIUS,
        };
        self.placeholder_brush.SetOpacity(alpha);
        self.target
            .FillRoundedRectangle(&logo_rect, &self.placeholder_brush);
        self.hover_brush.SetOpacity(0.10 * alpha);
        self.target
            .DrawRoundedRectangle(&logo_rect, &self.hover_brush, 1.0, None);

        let note: Vec<u16> = "♪".encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &note,
            &self.clock_hero_format,
            EXPANDED_ART.width(),
            EXPANDED_ART.height(),
        ) {
            self.title_brush.SetOpacity(0.72 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: pill_left + EXPANDED_ART.left + 25.0,
                    y: pill_top + EXPANDED_ART.top + 14.0,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Bold clock centered directly beneath the media logo on the left
        let st = GetLocalTime();
        let clock = format!("{:02}:{:02}", st.wHour, st.wMinute);
        let clock_u16: Vec<u16> = clock.encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &clock_u16,
            &self.clock_media_format,
            MEDIA_CLOCK_BOX.width(),
            MEDIA_CLOCK_BOX.height(),
        ) {
            self.title_brush.SetOpacity(0.98 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: pill_left + MEDIA_CLOCK_BOX.left,
                    y: pill_top + MEDIA_CLOCK_BOX.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Right column: Date + Media status + live Volume slider
        let right_x = pill_left + 112.0;
        let right_w = (pill_width - 132.0).max(80.0);

        let heading_u16: Vec<u16> = "No media playing".encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&heading_u16, &self.title_format, right_w, 26.0)
        {
            self.title_brush.SetOpacity(0.94 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: right_x,
                    y: pill_top + 46.0,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let date_str = format_short_date(&st);
        let sub_text = if notif.unread_count() > 0 {
            format!("{date_str}  •  {} unread", notif.unread_count())
        } else {
            date_str
        };
        let sub: Vec<u16> = sub_text.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&sub, &self.artist_format, right_w, 22.0)
        {
            self.artist_brush.SetOpacity(0.75 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: right_x,
                    y: pill_top + 73.0,
                },
                &layout,
                &self.artist_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Live system volume bar (sits cleanly above Zeki Companion Dock)
        let bar_y = pill_top + 128.0;
        let bar_h = 6.0f32;
        let vol_label = if system.muted {
            "Volume  •  Muted".to_string()
        } else {
            format!("Volume  •  {}%", (system.volume * 100.0).round() as u32)
        };
        let vol_u16: Vec<u16> = vol_label.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&vol_u16, &self.hud_format, right_w, 16.0)
        {
            self.hud_text_brush.SetOpacity(0.68 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: right_x,
                    y: bar_y - 19.0,
                },
                &layout,
                &self.hud_text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let track = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: right_x,
                top: bar_y,
                right: right_x + right_w,
                bottom: bar_y + bar_h,
            },
            radiusX: bar_h / 2.0,
            radiusY: bar_h / 2.0,
        };
        self.hud_track_brush.SetOpacity(alpha);
        self.target
            .FillRoundedRectangle(&track, &self.hud_track_brush);

        let fill_w = (right_w * system.volume.clamp(0.0, 1.0))
            .max(if system.volume > 0.001 { bar_h } else { 0.0 });
        if fill_w > 0.0 {
            let fill = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: right_x,
                    top: bar_y,
                    right: right_x + fill_w,
                    bottom: bar_y + bar_h,
                },
                radiusX: bar_h / 2.0,
                radiusY: bar_h / 2.0,
            };
            self.hud_fill_brush
                .SetOpacity(if system.muted { 0.30 } else { 0.88 } * alpha);
            self.target
                .FillRoundedRectangle(&fill, &self.hud_fill_brush);
        }
    }

    /// Focus / Pomodoro Timer panel.
    unsafe fn draw_timer_panel(
        &self,
        origin_x: f32,
        origin_y: f32,
        timer: &PomodoroTimer,
        hovered: Option<HitTarget>,
        alpha: f32,
    ) {
        let time_str = timer.formatted();
        let time_u16: Vec<u16> = time_str.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&time_u16, &self.clock_hero_format, 180.0, 44.0)
        {
            self.title_brush.SetOpacity(alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + 22.0,
                    y: origin_y + 44.0,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let status = if timer.running {
            "Focus • Running"
        } else if timer.completed {
            "Focus • Complete!"
        } else if timer.remaining_secs < timer.total_secs {
            "Focus • Paused"
        } else {
            "Focus • Ready"
        };
        let status_u16: Vec<u16> = status.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&status_u16, &self.notif_app_format, 180.0, 18.0)
        {
            if timer.running {
                self.accent_brush.SetOpacity(0.95 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + 22.0, y: origin_y + 88.0 },
                    &layout,
                    &self.accent_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
                self.accent_brush.SetOpacity(1.0);
            } else {
                self.artist_brush.SetOpacity(0.75 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + 22.0, y: origin_y + 88.0 },
                    &layout,
                    &self.artist_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        // Start / Pause accent button
        let toggle_lbl = if timer.running { "Pause" } else { "Start" };
        let toggle_hovered = hovered == Some(HitTarget::TimerToggle);
        let toggle_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + TIMER_TOGGLE_BTN.left,
                top: origin_y + TIMER_TOGGLE_BTN.top,
                right: origin_x + TIMER_TOGGLE_BTN.right,
                bottom: origin_y + TIMER_TOGGLE_BTN.bottom,
            },
            radiusX: 15.0,
            radiusY: 15.0,
        };
        self.accent_brush
            .SetOpacity(if toggle_hovered { 0.96 } else { 0.80 } * alpha);
        self.target
            .FillRoundedRectangle(&toggle_rect, &self.accent_brush);
        self.accent_brush.SetOpacity(1.0);

        let t_u16: Vec<u16> = toggle_lbl.encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &t_u16,
            &self.pill_btn_format,
            TIMER_TOGGLE_BTN.width(),
            TIMER_TOGGLE_BTN.height(),
        ) {
            self.title_brush.SetOpacity(alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + TIMER_TOGGLE_BTN.left,
                    y: origin_y + TIMER_TOGGLE_BTN.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        // Reset button
        self.draw_action_pill(
            origin_x,
            origin_y,
            TIMER_RESET_BTN,
            "Reset",
            hovered == Some(HitTarget::TimerReset),
            alpha,
        );

        // 4 preset buttons (5m, 15m, 25m, 45m)
        for (i, &secs) in TIMER_PRESETS_SECS.iter().enumerate() {
            let rect = timer_preset_rect(i);
            let selected = timer.total_secs == secs;
            let is_hov = hovered == Some(HitTarget::TimerPreset(secs));
            let lbl = format!("{}m", secs / 60);

            let d2d_r = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + rect.left,
                    top: origin_y + rect.top,
                    right: origin_x + rect.right,
                    bottom: origin_y + rect.bottom,
                },
                radiusX: 12.0,
                radiusY: 12.0,
            };
            self.hover_brush.SetOpacity(
                if selected {
                    0.20
                } else if is_hov {
                    0.15
                } else {
                    0.07
                } * alpha,
            );
            self.target.FillRoundedRectangle(&d2d_r, &self.hover_brush);

            let lbl_u16: Vec<u16> = lbl.encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &lbl_u16,
                &self.pill_btn_format,
                rect.width(),
                rect.height(),
            ) {
                self.title_brush
                    .SetOpacity(if selected || is_hov { 1.0 } else { 0.72 } * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + rect.left,
                        y: origin_y + rect.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        // Bottom progress bar
        let bar_h = TIMER_PROGRESS_BAR.height();
        let track = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + TIMER_PROGRESS_BAR.left,
                top: origin_y + TIMER_PROGRESS_BAR.top,
                right: origin_x + TIMER_PROGRESS_BAR.right,
                bottom: origin_y + TIMER_PROGRESS_BAR.bottom,
            },
            radiusX: bar_h / 2.0,
            radiusY: bar_h / 2.0,
        };
        self.hud_track_brush.SetOpacity(alpha);
        self.target
            .FillRoundedRectangle(&track, &self.hud_track_brush);

        let fill_w = (TIMER_PROGRESS_BAR.width() * timer.progress()).max(0.0);
        if fill_w > 1.0 {
            let fill = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + TIMER_PROGRESS_BAR.left,
                    top: origin_y + TIMER_PROGRESS_BAR.top,
                    right: origin_x + TIMER_PROGRESS_BAR.left + fill_w,
                    bottom: origin_y + TIMER_PROGRESS_BAR.bottom,
                },
                radiusX: bar_h / 2.0,
                radiusY: bar_h / 2.0,
            };
            self.accent_brush.SetOpacity(0.92 * alpha);
            self.target.FillRoundedRectangle(&fill, &self.accent_brush);
            self.accent_brush.SetOpacity(1.0);
        }
    }

    /// Quick Clipboard History panel (click any row to copy back).
    unsafe fn draw_clipboard_panel(
        &self,
        origin_x: f32,
        origin_y: f32,
        pill_width: f32,
        items: &[String],
        copied_flash: bool,
        hovered: Option<HitTarget>,
        alpha: f32,
    ) {
        let header = if copied_flash {
            "Copied to clipboard!".to_string()
        } else {
            format!("Clipboard History ({})", items.len())
        };
        let header_u16: Vec<u16> = header.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&header_u16, &self.hud_format, 260.0, 18.0)
        {
            if copied_flash {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: 34.0 / 255.0,
                    g: 197.0 / 255.0,
                    b: 94.0 / 255.0,
                    a: alpha,
                });
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + 20.0, y: origin_y + 41.0 },
                    &layout,
                    &self.dynamic_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            } else {
                self.hud_text_brush.SetOpacity(0.82 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + 20.0, y: origin_y + 41.0 },
                    &layout,
                    &self.hud_text_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        if !items.is_empty() {
            self.draw_action_pill(
                origin_x,
                origin_y,
                CLIP_CLEAR_BTN,
                "Clear",
                hovered == Some(HitTarget::ClipClear),
                alpha,
            );
        }

        if items.is_empty() {
            let m1: Vec<u16> = "Clipboard history is empty".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &m1,
                &self.center_title_format,
                pill_width - 32.0,
                22.0,
            ) {
                self.title_brush.SetOpacity(0.88 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + 16.0, y: origin_y + 82.0 },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            let m2: Vec<u16> = "Copy any text (Ctrl+C) to pin it here".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &m2,
                &self.center_sub_format,
                pill_width - 32.0,
                20.0,
            ) {
                self.artist_brush.SetOpacity(0.65 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + 16.0, y: origin_y + 106.0 },
                    &layout,
                    &self.artist_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            return;
        }

        for (i, text) in items.iter().take(CLIP_MAX_VISIBLE).enumerate() {
            let rect = clip_row_rect(i);
            let is_hov = hovered == Some(HitTarget::ClipCopy(i));

            let row = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + rect.left,
                    top: origin_y + rect.top,
                    right: origin_x + rect.right,
                    bottom: origin_y + rect.bottom,
                },
                radiusX: 8.0,
                radiusY: 8.0,
            };
            self.hover_brush
                .SetOpacity(if is_hov { 0.14 } else { 0.06 } * alpha);
            self.target.FillRoundedRectangle(&row, &self.hover_brush);

            // Index badge (`#1`, `#2`, `#3`)
            let idx_str = format!("#{}", i + 1);
            let idx_u16: Vec<u16> = idx_str.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&idx_u16, &self.badge_format, 24.0, rect.height())
            {
                self.accent_brush.SetOpacity(0.90 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + rect.left + 6.0,
                        y: origin_y + rect.top,
                    },
                    &layout,
                    &self.accent_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
                self.accent_brush.SetOpacity(1.0);
            }

            let single_line = text.replace(['\r', '\n', '\t'], " ");
            if let Ok(layout) = create_layout(
                &self.dwrite,
                &self.notif_body_format,
                &self.notif_body_trim,
                &single_line,
                rect.width() - 42.0,
                18.0,
            ) {
                self.title_brush
                    .SetOpacity(if is_hov { 1.0 } else { 0.86 } * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: origin_x + rect.left + 34.0,
                        y: origin_y + rect.top + 5.0,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    }

    /// Compact System Monitor panel (CPU, RAM, Battery/Power).
    unsafe fn draw_sysmon_panel(
        &self,
        origin_x: f32,
        origin_y: f32,
        pill_width: f32,
        stats: &SystemStats,
        alpha: f32,
    ) {
        let pad_x = 22.0f32;
        let bar_w = (pill_width - pad_x * 2.0).max(80.0);

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

        let rows: [(&str, String, f32, bool); 3] = [
            ("CPU", format!("{:.0}%", stats.cpu_percent), stats.cpu_percent / 100.0, false),
            ("Memory", ram_val, stats.ram_percent / 100.0, false),
            (power_lbl, power_val, stats.battery_percent / 100.0, true),
        ];

        for (i, (label, value, frac, is_battery)) in rows.iter().enumerate() {
            let y = origin_y + 46.0 + i as f32 * 36.0;

            let l_u16: Vec<u16> = label.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&l_u16, &self.hud_format, bar_w, 16.0)
            {
                self.hud_text_brush.SetOpacity(0.90 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + pad_x, y },
                    &layout,
                    &self.hud_text_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            let v_u16: Vec<u16> = value.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&v_u16, &self.hud_value_format, bar_w, 16.0)
            {
                self.hud_text_brush.SetOpacity(0.75 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x + pad_x, y },
                    &layout,
                    &self.hud_text_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            let bar_y = y + 19.0;
            let bar_h = 7.0f32;
            let track = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + pad_x,
                    top: bar_y,
                    right: origin_x + pad_x + bar_w,
                    bottom: bar_y + bar_h,
                },
                radiusX: bar_h / 2.0,
                radiusY: bar_h / 2.0,
            };
            self.hud_track_brush.SetOpacity(alpha);
            self.target
                .FillRoundedRectangle(&track, &self.hud_track_brush);

            let clamped = frac.clamp(0.0, 1.0);
            let fill_w = (bar_w * clamped).max(if clamped > 0.005 { bar_h } else { 0.0 });
            if fill_w > 0.0 {
                let (r, g, b) = metric_bar_color(clamped, *is_battery);
                self.dynamic_brush
                    .SetColor(&D2D1_COLOR_F { r, g, b, a: 0.92 * alpha });
                let fill = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: origin_x + pad_x,
                        top: bar_y,
                        right: origin_x + pad_x + fill_w,
                        bottom: bar_y + bar_h,
                    },
                    radiusX: bar_h / 2.0,
                    radiusY: bar_h / 2.0,
                };
                self.target.FillRoundedRectangle(&fill, &self.dynamic_brush);
            }
        }
    }

    unsafe fn draw_art(&self, origin_x: f32, origin_y: f32, rect: &RectF, radius: f32) {
        let destination = D2D_RECT_F {
            left: origin_x + rect.left,
            top: origin_y + rect.top,
            right: origin_x + rect.right,
            bottom: origin_y + rect.bottom,
        };

        if let Some(bitmap) = &self.art_bitmap {
            self.target.DrawBitmap(
                bitmap,
                Some(&destination),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
            return;
        }

        let placeholder = D2D1_ROUNDED_RECT {
            rect: destination,
            radiusX: radius,
            radiusY: radius,
        };
        self.target
            .FillRoundedRectangle(&placeholder, &self.placeholder_brush);
    }

    unsafe fn draw_text(&self, origin_x: f32, origin_y: f32, alpha: f32) {
        if let Some(layout) = &self.title_layout {
            self.title_brush.SetOpacity(alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + TITLE_BOX.left,
                    y: origin_y + TITLE_BOX.top,
                },
                layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        if let Some(layout) = &self.artist_layout {
            self.artist_brush.SetOpacity(alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + ARTIST_BOX.left,
                    y: origin_y + ARTIST_BOX.top,
                },
                layout,
                &self.artist_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    unsafe fn draw_controls(
        &self,
        origin_x: f32,
        origin_y: f32,
        alpha: f32,
        is_playing: bool,
        hovered: Option<MediaControl>,
    ) {
        let transform = Matrix3x2 {
            M11: 1.0,
            M12: 0.0,
            M21: 0.0,
            M22: 1.0,
            M31: origin_x,
            M32: origin_y,
        };
        let identity = Matrix3x2 {
            M11: 1.0,
            M12: 0.0,
            M21: 0.0,
            M22: 1.0,
            M31: 0.0,
            M32: 0.0,
        };
        self.target.SetTransform(&transform);

        if let Some(control) = hovered {
            let rect = control_rect(control);
            let (center_x, center_y) = rect.center();
            self.hover_brush.SetOpacity(0.12 * alpha);
            let ellipse = D2D1_ELLIPSE {
                point: D2D_POINT_2F {
                    x: center_x,
                    y: center_y,
                },
                radiusX: HOVER_RADIUS,
                radiusY: HOVER_RADIUS,
            };
            self.target.FillEllipse(&ellipse, &self.hover_brush);
        }

        self.icon_brush.SetOpacity(0.92 * alpha);

        self.target.FillGeometry(
            &self.previous_icon,
            &self.icon_brush,
            None::<&ID2D1Brush>,
        );

        if is_playing {
            let left = control_rect(MediaControl::PlayPause).left;
            for offset in [10.5f32, 16.5] {
                let bar = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: left + offset,
                        top: CONTROLS_TOP + 9.0,
                        right: left + offset + 3.0,
                        bottom: CONTROLS_TOP + 21.0,
                    },
                    radiusX: 1.5,
                    radiusY: 1.5,
                };
                self.target.FillRoundedRectangle(&bar, &self.icon_brush);
            }
        } else {
            self.target
                .FillGeometry(&self.play_icon, &self.icon_brush, None::<&ID2D1Brush>);
        }

        self.target
            .FillGeometry(&self.next_icon, &self.icon_brush, None::<&ID2D1Brush>);

        self.target.SetTransform(&identity);
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_equalizer(
        &self,
        origin_x: f32,
        origin_y: f32,
        center_y: f32,
        levels: &[f32; EQUALIZER_BAR_COUNT],
        is_playing: bool,
        alpha: f32,
        right_edge: f32,
        bar_width: f32,
        bar_gap: f32,
        min_height: f32,
        max_delta: f32,
    ) {
        let total_width = EQUALIZER_BAR_COUNT as f32 * bar_width
            + (EQUALIZER_BAR_COUNT as f32 - 1.0) * bar_gap;
        let start_x = right_edge - total_width;
        let bar_alpha = if is_playing { alpha } else { alpha * 0.45 };

        for index in 0..EQUALIZER_BAR_COUNT {
            let strength = if is_playing {
                levels[index].clamp(0.0, 1.0)
            } else {
                PAUSED_EQUALIZER[index]
            };
            let bar_height = min_height + strength * max_delta;
            let half_h = bar_height / 2.0;
            let x = start_x + index as f32 * (bar_width + bar_gap);
            let bar = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: origin_x + x,
                    top: origin_y + center_y - half_h,
                    right: origin_x + x + bar_width,
                    bottom: origin_y + center_y + half_h,
                },
                radiusX: bar_width / 2.0,
                radiusY: bar_width / 2.0,
            };

            if index % 2 == 0 {
                self.accent_brush.SetOpacity(bar_alpha);
                self.target.FillRoundedRectangle(&bar, &self.accent_brush);
            } else {
                self.equalizer_accent_brush.SetOpacity(bar_alpha * 0.92);
                self.target
                    .FillRoundedRectangle(&bar, &self.equalizer_accent_brush);
            }
        }
        self.accent_brush.SetOpacity(1.0);
    }

    unsafe fn draw_collapsed_notification_badge(
        &self,
        pill_left: f32,
        pill_top: f32,
        pill_width: f32,
        pill_height: f32,
        app_name: &str,
        count: usize,
        alpha: f32,
    ) {
        let cy = pill_top + pill_height / 2.0;

        let count_text = if count > 9 {
            "9+".to_string()
        } else {
            count.to_string()
        };
        let count_w = if count > 9 { 24.0 } else { 20.0 };
        let count_right = pill_left + pill_width - 9.0;
        let count_left = count_right - count_w;
        let count_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: count_left,
                top: cy - 10.0,
                right: count_right,
                bottom: cy + 10.0,
            },
            radiusX: 10.0,
            radiusY: 10.0,
        };
        self.badge_red_brush.SetOpacity(alpha);
        self.target
            .FillRoundedRectangle(&count_rect, &self.badge_red_brush);

        let count_u16: Vec<u16> = count_text.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&count_u16, &self.badge_format, count_w, 20.0)
        {
            self.title_brush.SetOpacity(alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: count_left,
                    y: cy - 10.0,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let avatar_cx = count_left - 14.0;
        let (r, g, b) = app_accent_rgb(app_name);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F { r, g, b, a: alpha });
        let avatar = D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: avatar_cx, y: cy },
            radiusX: 10.0,
            radiusY: 10.0,
        };
        self.target.FillEllipse(&avatar, &self.dynamic_brush);

        let initial = app_initial(app_name);
        let init_u16: Vec<u16> = initial.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&init_u16, &self.badge_format, 20.0, 20.0)
        {
            self.title_brush.SetOpacity(alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: avatar_cx - 10.0,
                    y: cy - 10.0,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    unsafe fn draw_notification_toast(
        &self,
        pill_left: f32,
        pill_top: f32,
        item: &NotificationItem,
        progress: f32,
        hovered: Option<HitTarget>,
    ) {
        let (ar, ag, ab) = app_accent_rgb(&item.app);

        let avatar_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: pill_left + TOAST_AVATAR.left,
                top: pill_top + TOAST_AVATAR.top,
                right: pill_left + TOAST_AVATAR.right,
                bottom: pill_top + TOAST_AVATAR.bottom,
            },
            radiusX: 10.0,
            radiusY: 10.0,
        };
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: 0.36,
        });
        self.target
            .FillRoundedRectangle(&avatar_rect, &self.dynamic_brush);
        self.dynamic_brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: 0.55,
        });
        self.target
            .DrawRoundedRectangle(&avatar_rect, &self.dynamic_brush, 1.0, None);

        let init_u16: Vec<u16> = app_initial(&item.app).encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &init_u16,
            &self.badge_format,
            TOAST_AVATAR.width(),
            TOAST_AVATAR.height(),
        ) {
            self.title_brush.SetOpacity(1.0);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: pill_left + TOAST_AVATAR.left,
                    y: pill_top + TOAST_AVATAR.top,
                },
                &layout,
                &self.title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let text_left = pill_left + 64.0;
        let text_w = (TOAST_DISMISS_BTN.left - 68.0).max(60.0);
        let header = format!("{}  •  just now", item.app);
        let header_u16: Vec<u16> = header.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&header_u16, &self.notif_app_format, text_w, 16.0)
        {
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: 1.0,
            });
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: text_left,
                    y: pill_top + 13.0,
                },
                &layout,
                &self.dynamic_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let title_str = if item.title.is_empty() {
            "Notification"
        } else {
            item.title.as_str()
        };
        if let Ok(layout) = create_layout(
            &self.dwrite,
            &self.notif_title_format,
            &self.notif_title_trim,
            title_str,
            text_w,
            19.0,
        ) {
            self.notif_title_brush.SetOpacity(1.0);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: text_left,
                    y: pill_top + 30.0,
                },
                &layout,
                &self.notif_title_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        if !item.body.is_empty() {
            if let Ok(layout) = create_layout(
                &self.dwrite,
                &self.notif_body_format,
                &self.notif_body_trim,
                &item.body,
                text_w + 18.0,
                30.0,
            ) {
                self.notif_body_brush.SetOpacity(1.0);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: text_left,
                        y: pill_top + 49.0,
                    },
                    &layout,
                    &self.notif_body_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        self.draw_dismiss_x(
            pill_left,
            pill_top,
            TOAST_DISMISS_BTN,
            hovered == Some(HitTarget::ToastDismiss(item.id)),
            1.0,
        );

        let bar_w = TOAST_PROGRESS_RECT.width() * progress.clamp(0.0, 1.0);
        if bar_w > 1.0 {
            let bar = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: pill_left + TOAST_PROGRESS_RECT.left,
                    top: pill_top + TOAST_PROGRESS_RECT.top,
                    right: pill_left + TOAST_PROGRESS_RECT.left + bar_w,
                    bottom: pill_top + TOAST_PROGRESS_RECT.bottom,
                },
                radiusX: 1.25,
                radiusY: 1.25,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: 0.72,
            });
            self.target.FillRoundedRectangle(&bar, &self.dynamic_brush);
        }
    }

    unsafe fn draw_notifications_panel(
        &self,
        pill_left: f32,
        pill_top: f32,
        pill_width: f32,
        notif: &NotificationSnapshot,
        hovered: Option<HitTarget>,
        alpha: f32,
    ) {
        let unread = notif.unread_count();
        let header_text = if unread > NOTIF_CARD_MAX_VISIBLE {
            format!(
                "Notifications ({unread})  •  +{} more",
                unread - NOTIF_CARD_MAX_VISIBLE
            )
        } else if unread > 0 {
            format!("Notifications ({unread})")
        } else {
            "Notifications".to_string()
        };
        let header_u16: Vec<u16> = header_text.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&header_u16, &self.hud_format, 260.0, 18.0)
        {
            self.hud_text_brush.SetOpacity(0.84 * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: pill_left + 20.0,
                    y: pill_top + 41.0,
                },
                &layout,
                &self.hud_text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        if !notif.items.is_empty() {
            self.draw_action_pill(
                pill_left,
                pill_top,
                NOTIF_CLEAR_BTN,
                "Clear all",
                hovered == Some(HitTarget::ClearAllNotifications),
                alpha,
            );
        }

        if !notif.access_granted && notif.items.is_empty() {
            let msg1: Vec<u16> = "Notification access required".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &msg1,
                &self.center_title_format,
                pill_width - 32.0,
                22.0,
            ) {
                self.title_brush.SetOpacity(0.92 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: pill_left + 16.0,
                        y: pill_top + 66.0,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            let msg2: Vec<u16> =
                "Enable desktop app access in Windows Privacy Settings"
                    .encode_utf16()
                    .collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &msg2,
                &self.center_sub_format,
                pill_width - 32.0,
                20.0,
            ) {
                self.artist_brush.SetOpacity(0.75 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: pill_left + 16.0,
                        y: pill_top + 88.0,
                    },
                    &layout,
                    &self.artist_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            let btn_hovered = hovered == Some(HitTarget::OpenNotificationSettings);
            let btn_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: pill_left + NOTIF_SETTINGS_BTN.left,
                    top: pill_top + NOTIF_SETTINGS_BTN.top,
                    right: pill_left + NOTIF_SETTINGS_BTN.right,
                    bottom: pill_top + NOTIF_SETTINGS_BTN.bottom,
                },
                radiusX: 15.0,
                radiusY: 15.0,
            };
            self.accent_brush
                .SetOpacity(if btn_hovered { 0.95 } else { 0.78 } * alpha);
            self.target
                .FillRoundedRectangle(&btn_rect, &self.accent_brush);
            self.accent_brush.SetOpacity(1.0);

            let btn_lbl: Vec<u16> = "Open Windows Settings".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &btn_lbl,
                &self.pill_btn_format,
                NOTIF_SETTINGS_BTN.width(),
                NOTIF_SETTINGS_BTN.height(),
            ) {
                self.title_brush.SetOpacity(alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: pill_left + NOTIF_SETTINGS_BTN.left,
                        y: pill_top + NOTIF_SETTINGS_BTN.top,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            return;
        }

        if notif.items.is_empty() {
            let msg1: Vec<u16> = "No notifications".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &msg1,
                &self.center_title_format,
                pill_width - 32.0,
                22.0,
            ) {
                self.title_brush.SetOpacity(0.90 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: pill_left + 16.0,
                        y: pill_top + 82.0,
                    },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            let msg2: Vec<u16> = "All caught up!".encode_utf16().collect();
            if let Ok(layout) = self.dwrite.CreateTextLayout(
                &msg2,
                &self.center_sub_format,
                pill_width - 32.0,
                20.0,
            ) {
                self.artist_brush.SetOpacity(0.65 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: pill_left + 16.0,
                        y: pill_top + 106.0,
                    },
                    &layout,
                    &self.artist_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
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
            let card_hovered = hovered == Some(HitTarget::NotifActivate(item.id));
            let dismiss_hovered = hovered == Some(HitTarget::NotifDismiss(item.id));

            let card_d2d = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: pill_left + card.left,
                    top: pill_top + card.top,
                    right: pill_left + card.right,
                    bottom: pill_top + card.bottom,
                },
                radiusX: 10.0,
                radiusY: 10.0,
            };
            self.hover_brush
                .SetOpacity(if card_hovered { 0.11 } else { 0.055 } * alpha);
            self.target
                .FillRoundedRectangle(&card_d2d, &self.hover_brush);

            let (ar, ag, ab) = app_accent_rgb(&item.app);
            let av_left = pill_left + card.left + 8.0;
            let av_top = pill_top + card.top + 8.0;
            let av_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: av_left,
                    top: av_top,
                    right: av_left + 25.0,
                    bottom: av_top + 25.0,
                },
                radiusX: 6.0,
                radiusY: 6.0,
            };
            self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: 0.90 * alpha,
            });
            self.target
                .FillRoundedRectangle(&av_rect, &self.dynamic_brush);

            let init_u16: Vec<u16> = app_initial(&item.app).encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&init_u16, &self.badge_format, 25.0, 25.0)
            {
                self.title_brush.SetOpacity(alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F { x: av_left, y: av_top },
                    &layout,
                    &self.title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            let tx = pill_left + card.left + 40.0;
            let tw = (dismiss_rect.left - (card.left + 44.0)).max(60.0);
            let meta = format!("{}  •  {}", item.app, format_relative_time(item.timestamp_ms));
            let meta_u16: Vec<u16> = meta.encode_utf16().collect();
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&meta_u16, &self.notif_app_format, tw, 14.0)
            {
                self.dynamic_brush.SetColor(&D2D1_COLOR_F {
                    r: ar,
                    g: ag,
                    b: ab,
                    a: 0.95 * alpha,
                });
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: tx,
                        y: pill_top + card.top + 4.0,
                    },
                    &layout,
                    &self.dynamic_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            let line2 = if item.body.is_empty() {
                item.title.clone()
            } else if item.title.is_empty() {
                item.body.clone()
            } else {
                format!("{} — {}", item.title, item.body)
            };
            if let Ok(layout) = create_layout(
                &self.dwrite,
                &self.notif_title_format,
                &self.notif_title_trim,
                &line2,
                tw,
                18.0,
            ) {
                self.notif_title_brush.SetOpacity(0.94 * alpha);
                self.target.DrawTextLayout(
                    D2D_POINT_2F {
                        x: tx,
                        y: pill_top + card.top + 19.0,
                    },
                    &layout,
                    &self.notif_title_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }

            self.draw_dismiss_x(pill_left, pill_top, dismiss_rect, dismiss_hovered, alpha);
        }
    }

    unsafe fn draw_action_pill(
        &self,
        origin_x: f32,
        origin_y: f32,
        rect: RectF,
        label: &str,
        hovered: bool,
        alpha: f32,
    ) {
        let d2d_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: origin_x + rect.left,
                top: origin_y + rect.top,
                right: origin_x + rect.right,
                bottom: origin_y + rect.bottom,
            },
            radiusX: rect.height() / 2.0,
            radiusY: rect.height() / 2.0,
        };
        self.hover_brush
            .SetOpacity(if hovered { 0.20 } else { 0.10 } * alpha);
        self.target
            .FillRoundedRectangle(&d2d_rect, &self.hover_brush);

        let u16_lbl: Vec<u16> = label.encode_utf16().collect();
        if let Ok(layout) = self.dwrite.CreateTextLayout(
            &u16_lbl,
            &self.pill_btn_format,
            rect.width(),
            rect.height(),
        ) {
            self.hud_text_brush
                .SetOpacity(if hovered { 1.0 } else { 0.82 } * alpha);
            self.target.DrawTextLayout(
                D2D_POINT_2F {
                    x: origin_x + rect.left,
                    y: origin_y + rect.top,
                },
                &layout,
                &self.hud_text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    unsafe fn draw_dismiss_x(
        &self,
        origin_x: f32,
        origin_y: f32,
        rect: RectF,
        hovered: bool,
        alpha: f32,
    ) {
        let (cx, cy) = rect.center();
        let x = origin_x + cx;
        let y = origin_y + cy;

        if hovered {
            let circle = D2D1_ELLIPSE {
                point: D2D_POINT_2F { x, y },
                radiusX: 10.5,
                radiusY: 10.5,
            };
            self.hover_brush.SetOpacity(0.16 * alpha);
            self.target.FillEllipse(&circle, &self.hover_brush);
        }

        let r = 3.8;
        self.icon_brush
            .SetOpacity(if hovered { 0.92 } else { 0.45 } * alpha);
        self.target.DrawLine(
            D2D_POINT_2F { x: x - r, y: y - r },
            D2D_POINT_2F { x: x + r, y: y + r },
            &self.icon_brush,
            1.6,
            None,
        );
        self.target.DrawLine(
            D2D_POINT_2F { x: x + r, y: y - r },
            D2D_POINT_2F { x: x - r, y: y + r },
            &self.icon_brush,
            1.6,
            None,
        );
    }

    unsafe fn draw_hud(
        &self,
        pill_left: f32,
        pill_top: f32,
        pill_width: f32,
        pill_height: f32,
        hud: &HudState,
    ) {
        let pad_x = 20.0f32;
        let bar_h = 6.0f32;
        let bar_w = (pill_width - pad_x * 2.0).max(40.0);
        let bar_x = pill_left + (pill_width - bar_w) / 2.0;
        let bar_y = pill_top + (pill_height * 0.56).max(24.0);

        let label = match hud.kind {
            HudKind::Volume => "Volume",
            HudKind::Brightness => "Brightness",
        };
        let value_text = if hud.kind == HudKind::Volume && hud.muted {
            "Muted".to_string()
        } else {
            format!("{}%", (hud.value.clamp(0.0, 1.0) * 100.0).round() as u32)
        };

        let label_y = (bar_y - 20.0).max(pill_top + 8.0);
        let label_u16: Vec<u16> = label.encode_utf16().collect();
        if let Ok(layout) = self
            .dwrite
            .CreateTextLayout(&label_u16, &self.hud_format, bar_w, 18.0)
        {
            self.hud_text_brush.SetOpacity(0.92);
            self.target.DrawTextLayout(
                D2D_POINT_2F { x: bar_x, y: label_y },
                &layout,
                &self.hud_text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let value_u16: Vec<u16> = value_text.encode_utf16().collect();
        if let Ok(layout) =
            self.dwrite
                .CreateTextLayout(&value_u16, &self.hud_value_format, bar_w, 18.0)
        {
            self.hud_text_brush.SetOpacity(0.75);
            self.target.DrawTextLayout(
                D2D_POINT_2F { x: bar_x, y: label_y },
                &layout,
                &self.hud_text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }

        let track = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: bar_x,
                top: bar_y,
                right: bar_x + bar_w,
                bottom: bar_y + bar_h,
            },
            radiusX: bar_h / 2.0,
            radiusY: bar_h / 2.0,
        };
        self.hud_track_brush.SetOpacity(1.0);
        self.target
            .FillRoundedRectangle(&track, &self.hud_track_brush);

        let clamped = hud.value.clamp(0.0, 1.0);
        let fill_w = (bar_w * clamped).max(if clamped > 0.001 { bar_h } else { 0.0 });
        if fill_w > 0.0 {
            let fill = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: bar_x,
                    top: bar_y,
                    right: bar_x + fill_w,
                    bottom: bar_y + bar_h,
                },
                radiusX: bar_h / 2.0,
                radiusY: bar_h / 2.0,
            };
            let alpha = if hud.muted { 0.35 } else { 0.95 };
            self.hud_fill_brush.SetOpacity(alpha);
            self.target
                .FillRoundedRectangle(&fill, &self.hud_fill_brush);
        }
    }

    unsafe fn prepare_art(&mut self, media: &MediaInfo) {
        if media.art_revision == self.art_revision {
            return;
        }
        self.art_revision = media.art_revision;
        self.art_bitmap = None;
        if let Some(art) = &media.album_art {
            if let Ok(bitmap) = self.create_art_bitmap(art) {
                self.art_bitmap = Some(bitmap);
            }
        }
    }

    unsafe fn create_art_bitmap(&self, art: &AlbumArt) -> Result<ID2D1Bitmap> {
        let properties = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        self.target.CreateBitmap(
            D2D_SIZE_U {
                width: art.width,
                height: art.height,
            },
            Some(art.pixels.as_ptr() as *const c_void),
            art.width * 4,
            &properties,
        )
    }

    unsafe fn prepare_text(&mut self, media: &MediaInfo) {
        let title = if media.title.trim().is_empty() {
            "Unknown title"
        } else {
            media.title.as_str()
        };
        let artist = media.artist.as_str();

        if title == self.cached_title && artist == self.cached_artist {
            return;
        }
        self.cached_title = title.to_owned();
        self.cached_artist = artist.to_owned();

        self.title_layout = create_layout(
            &self.dwrite,
            &self.title_format,
            &self.title_trim,
            title,
            TITLE_BOX.width(),
            TITLE_BOX.height(),
        )
        .ok();
        self.artist_layout = create_layout(
            &self.dwrite,
            &self.artist_format,
            &self.artist_trim,
            artist,
            ARTIST_BOX.width(),
            ARTIST_BOX.height(),
        )
        .ok();
    }

    unsafe fn create_surface(&mut self, width: u32, height: u32) -> Result<()> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = null_mut();
        let dib = CreateDIBSection(HDC::default(), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;

        let mem_dc = CreateCompatibleDC(None);
        if mem_dc.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(dib.0));
            return Err(Error::from_win32());
        }

        let previous_bitmap = SelectObject(mem_dc, HGDIOBJ(dib.0));
        if previous_bitmap.is_invalid() {
            let _ = DeleteDC(mem_dc);
            let _ = DeleteObject(HGDIOBJ(dib.0));
            return Err(Error::from_win32());
        }

        self.mem_dc = mem_dc;
        self.dib = dib;
        self.previous_bitmap = previous_bitmap;
        Ok(())
    }

    unsafe fn present(&self, hwnd: HWND) -> Result<()> {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err(Error::from_win32());
        }

        let size = SIZE {
            cx: self.width as i32,
            cy: self.height as i32,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        let result = UpdateLayeredWindow(
            hwnd,
            screen_dc,
            None,
            Some(&size),
            self.mem_dc,
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        let _ = ReleaseDC(None, screen_dc);
        result
    }

    unsafe fn release_surface(&mut self) {
        if !self.mem_dc.is_invalid() {
            if !self.previous_bitmap.is_invalid() {
                SelectObject(self.mem_dc, self.previous_bitmap);
            }
            let _ = DeleteDC(self.mem_dc);
            self.mem_dc = HDC::default();
            self.previous_bitmap = HGDIOBJ::default();
        }

        if !self.dib.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(self.dib.0));
            self.dib = HBITMAP::default();
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.release_surface();
        }
    }
}

fn metric_bar_color(frac: f32, is_battery: bool) -> (f32, f32, f32) {
    if is_battery {
        if frac < 0.20 {
            (239.0 / 255.0, 68.0 / 255.0, 68.0 / 255.0) // red
        } else if frac < 0.40 {
            (245.0 / 255.0, 158.0 / 255.0, 11.0 / 255.0) // amber
        } else {
            (34.0 / 255.0, 197.0 / 255.0, 94.0 / 255.0) // green
        }
    } else if frac > 0.85 {
        (235.0 / 255.0, 0.0, 40.0 / 255.0) // crimson
    } else if frac > 0.60 {
        (245.0 / 255.0, 158.0 / 255.0, 11.0 / 255.0) // amber
    } else {
        (34.0 / 255.0, 197.0 / 255.0, 94.0 / 255.0) // green
    }
}

unsafe fn font_family(factory: &IDWriteFactory) -> PCWSTR {
    let mut collection: Option<IDWriteFontCollection> = None;
    if factory
        .GetSystemFontCollection(&mut collection, BOOL(0))
        .is_ok()
    {
        if let Some(collection) = collection {
            let mut index = 0u32;
            let mut exists = BOOL(0);
            if collection
                .FindFamilyName(w!("Segoe UI Variable Text"), &mut index, &mut exists)
                .is_ok()
                && exists.as_bool()
            {
                return w!("Segoe UI Variable Text");
            }
        }
    }

    w!("Segoe UI")
}

unsafe fn create_layout(
    factory: &IDWriteFactory,
    format: &IDWriteTextFormat,
    sign: &IDWriteInlineObject,
    text: &str,
    width: f32,
    height: f32,
) -> Result<IDWriteTextLayout> {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let layout = factory.CreateTextLayout(&utf16, format, width, height)?;

    let trimming = DWRITE_TRIMMING {
        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
        delimiter: 0,
        delimiterCount: 0,
    };
    layout.SetTrimming(&trimming, sign)?;

    Ok(layout)
}

unsafe fn build_control_icon(
    factory: &ID2D1Factory,
    control: MediaControl,
) -> Result<ID2D1PathGeometry> {
    let rect = control_rect(control);
    let left = rect.left;
    let top = rect.top;

    let mut polygons: Vec<Vec<D2D_POINT_2F>> = Vec::new();

    match control {
        MediaControl::Previous => {
            polygons.push(vec![
                point(left + 3.0, top + 8.5),
                point(left + 5.0, top + 8.5),
                point(left + 5.0, top + 21.5),
                point(left + 3.0, top + 21.5),
            ]);
            polygons.push(vec![
                point(left + 21.0, top + 8.5),
                point(left + 21.0, top + 21.5),
                point(left + 8.0, top + 15.0),
            ]);
        }
        MediaControl::Next => {
            polygons.push(vec![
                point(left + 9.0, top + 8.5),
                point(left + 9.0, top + 21.5),
                point(left + 22.0, top + 15.0),
            ]);
            polygons.push(vec![
                point(left + 25.0, top + 8.5),
                point(left + 27.0, top + 8.5),
                point(left + 27.0, top + 21.5),
                point(left + 25.0, top + 21.5),
            ]);
        }
        MediaControl::PlayPause => {
            polygons.push(vec![
                point(left + 11.0, top + 8.0),
                point(left + 11.0, top + 22.0),
                point(left + 22.0, top + 15.0),
            ]);
        }
    }

    let geometry = factory.CreatePathGeometry()?;
    let sink = geometry.Open()?;

    for polygon in &polygons {
        sink.BeginFigure(polygon[0], D2D1_FIGURE_BEGIN_FILLED);
        for vertex in &polygon[1..] {
            sink.AddLine(*vertex);
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }

    sink.Close()?;
    Ok(geometry)
}

fn build_chat_transcript(assistant: &AssistantSnapshot) -> (String, bool, bool) {
    if let Some(err) = &assistant.error {
        return (err.clone(), true, false);
    }

    let mut sections: Vec<String> = Vec::new();
    for msg in &assistant.messages {
        let speaker = if msg.is_user { "You" } else { "Zeki" };
        sections.push(format!("{speaker}: {}", msg.text.trim()));
    }

    // Show the active in-flight turn (prompt + streaming reply) when not yet committed to history
    let last_committed_is_current = assistant
        .messages
        .last()
        .map(|m| !m.is_user && !assistant.reply.is_empty() && m.text.trim() == assistant.reply.trim())
        .unwrap_or(false);

    if !last_committed_is_current {
        if !assistant.transcript.is_empty() {
            sections.push(format!("You: {}", assistant.transcript.trim()));
        }
        if !assistant.reply.is_empty() {
            let mut reply_line = format!("Zeki: {}", assistant.reply.trim());
            if assistant.phase == AssistantPhase::Speaking {
                reply_line.push('▍');
            }
            sections.push(reply_line);
        } else if assistant.phase == AssistantPhase::Thinking {
            sections.push("Zeki: Sedang memikirkan jawaban...".to_string());
        } else if assistant.phase == AssistantPhase::Transcribing {
            sections.push("Zeki: Mengubah suara menjadi teks...".to_string());
        } else if assistant.phase == AssistantPhase::Listening {
            sections.push("Zeki: Silakan bicara... (lepas Alt+Space atau tunggu hening)".to_string());
        }
    }

    if !sections.is_empty() {
        return (sections.join("\n\n"), false, false);
    }

    let placeholder = if assistant.has_config_key {
        "Halo! Ketik pesan di bawah lalu tekan Enter, atau klik Mic untuk bicara.\n\nTips:\n• Scroll roda mouse atau klik-geser kotak ini untuk membaca chat panjang.\n• Klik 2x (double-click) kotak chat untuk memperbesar/memperkecil."
            .to_string()
    } else {
        "Klik Config di kiri bawah untuk isi API key gratis (Gemini 3.8 Flash / OpenRouter / OpenCode / NVIDIA)."
            .to_string()
    };
    (placeholder, false, true)
}

fn point(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

unsafe fn current_clock_string() -> String {
    let st = GetLocalTime();
    format!("{:02}:{:02}", st.wHour, st.wMinute)
}

fn format_short_date(st: &windows::Win32::Foundation::SYSTEMTIME) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let day = DAYS.get(st.wDayOfWeek as usize).copied().unwrap_or("");
    let month = st
        .wMonth
        .checked_sub(1)
        .and_then(|i| MONTHS.get(i as usize))
        .copied()
        .unwrap_or("");
    format!("{day}, {} {month}", st.wDay)
}
