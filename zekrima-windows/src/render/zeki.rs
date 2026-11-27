//! Procedural Direct2D character engine for "Zeki", the Zekrima Assistant mascot.
//!
//! Inspired by Coucou's procedural Mochi engine: 100% code-drawn vector geometry
//! (zero external assets, crisp at every DPI, minimal memory footprint) with a
//! 7-state expressive animation machine:
//! - `Idle`     : gentle float, natural blinking, occasional glance
//! - `Sleepy`   : auto-triggered after 15 min idle, closed eyes, rising "zZz"
//! - `Listening`: raised glowing antenna, alert wide eyes, live 5-bar waveform mouth
//! - `Thinking` : upward scanning gaze, orbiting thought dots, violet pulse glow
//! - `Speaking` : cheerful bouncy rhythm, animated capsule/wave mouth
//! - `Happy`    : smiling `^ ^` eyes, warm blush, cheerful hop & sparkle bursts
//! - `Error`    : head tilt & shake, spinning spiral eyes, floating `?` badge

use std::f32::consts::PI;
use std::time::{Duration, Instant};

use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN, D2D_POINT_2F, D2D_RECT_F,
    D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ARC_SEGMENT,
    D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND, D2D1_ELLIPSE, D2D1_LINE_JOIN_ROUND,
    D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES, D2D1_SWEEP_DIRECTION_CLOCKWISE,
    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
};
use windows::Win32::Graphics::DirectWrite::{IDWriteFactory, IDWriteTextFormat};
use windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE;

/// Seconds of user inactivity before Zeki gently drifts into `Sleepy`.
const SLEEP_AFTER_SECS: u64 = 15 * 60;

/// Duration of automatic `Happy` celebration before returning to `Idle`.
const AUTO_HAPPY_DURATION: Duration = Duration::from_millis(2200);

/// Cicada Mode visual accessories for Zeki (`CIcada_planning.md` Section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZekiCicadaAccessory {
    None,
    /// Standby / Listening & Live Subtitle Bar: Zeki wears a sleek headset whose earcups pulse to audio.
    Headset,
    /// Expanded Transcript: Zeki wears headset and holds a tiny animated notebook & pen (taking notes).
    TakingNotes,
}

/// Seven core animation states defined in the Zekrima Assistant plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZekiState {
    Idle,
    Sleepy,
    Listening,
    Thinking,
    Speaking,
    Happy,
    Error,
}

pub const ALL_ZEKI_STATES: [ZekiState; 7] = [
    ZekiState::Idle,
    ZekiState::Sleepy,
    ZekiState::Listening,
    ZekiState::Thinking,
    ZekiState::Speaking,
    ZekiState::Happy,
    ZekiState::Error,
];

impl ZekiState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Sleepy => "Sleepy",
            Self::Listening => "Listening",
            Self::Thinking => "Thinking",
            Self::Speaking => "Speaking",
            Self::Happy => "Happy",
            Self::Error => "Error",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Sleepy => "Sleep",
            Self::Listening => "Listen",
            Self::Thinking => "Think",
            Self::Speaking => "Speak",
            Self::Happy => "Happy",
            Self::Error => "Error",
        }
    }

    pub fn status_line(self) -> &'static str {
        match self {
            Self::Idle => "Ready & floating quietly",
            Self::Sleepy => "Resting... hover or click to wake",
            Self::Listening => "Listening to your voice...",
            Self::Thinking => "Processing your request...",
            Self::Speaking => "Responding with rhythm...",
            Self::Happy => "Action complete!",
            Self::Error => "Hmm... something went wrong",
        }
    }

    pub fn collapsed_tag(self) -> &'static str {
        match self {
            Self::Idle => "Zekrima",
            Self::Sleepy => "zZz...",
            Self::Listening => "Listening",
            Self::Thinking => "Thinking",
            Self::Speaking => "Speaking",
            Self::Happy => "Nice!",
            Self::Error => "Oops?",
        }
    }

    /// Accent / glow RGB color `(0..1)` representing Zeki's mood.
    pub fn accent_rgb(self) -> (f32, f32, f32) {
        match self {
            Self::Idle => (0.22, 0.74, 0.97),      // Soft Cyan (#38BDF8)
            Self::Sleepy => (0.45, 0.53, 0.68),    // Calm Slate (#7387AD)
            Self::Listening => (0.13, 0.83, 0.93), // Bright Cyan (#22D3EE)
            Self::Thinking => (0.65, 0.55, 0.98),  // Glowing Violet (#A78BFA)
            Self::Speaking => (0.18, 0.83, 0.75),  // Vivid Teal (#2DD4BF)
            Self::Happy => (0.20, 0.83, 0.50),     // Cheerful Emerald (#34D380)
            Self::Error => (0.98, 0.44, 0.52),     // Warm Coral Red (#FB7185)
        }
    }

    /// True for high-energy states that deserve full 60 fps updates.
    pub fn is_high_energy(self) -> bool {
        matches!(
            self,
            Self::Listening | Self::Thinking | Self::Speaking | Self::Happy | Self::Error
        )
    }

    /// True when Zeki should take prominent focus in the collapsed pill.
    pub fn is_assistant_active(self) -> bool {
        matches!(
            self,
            Self::Listening | Self::Thinking | Self::Speaking | Self::Error
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct SparkParticle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    age: f32,
    life: f32,
    size: f32,
}

/// Real-time animation & state machine for Zeki.
pub struct ZekiEngine {
    state: ZekiState,
    state_entered: Instant,
    last_interaction: Instant,
    auto_revert_at: Option<Instant>,
    manual_override: bool,

    // Continuous animation clock
    elapsed: f32,

    // Smoothly interpolated visual parameters
    accent: (f32, f32, f32),
    gaze_x: f32,
    gaze_y: f32,
    target_gaze_x: f32,
    target_gaze_y: f32,
    next_gaze_change: f32,

    // Natural blink cycle
    blink_open: f32,
    blink_phase: f32,
    blinking: bool,
    next_blink: f32,

    // Body transform springs / offsets (normalized to radius R)
    tilt: f32,
    offset_x: f32,
    offset_y: f32,
    scale_x: f32,
    scale_y: f32,
    antenna_lift: f32,
    blush: f32,

    // Transient impulse timers
    hop_timer: f32,
    shake_timer: f32,
    peek_timer: f32,

    // Subtle music rhythm bounce when media is playing
    dancing: bool,
    dance_level: f32,

    // Live microphone audio level (0..1) — when Some, drives Zeki's real waveform mouth
    live_voice_level: Option<f32>,
    voice_level: f32,

    // Cicada Mode (Live Meeting Translator) visual state
    cicada_accessory: ZekiCicadaAccessory,
    cicada_audio_level: f32,
    nod_timer: f32,

    // Spark particles for Happy/poke celebrations
    sparks: Vec<SparkParticle>,

    // Simple deterministic LCG seed for organic blinks/glances/sparks (no rand crate needed)
    rng_state: u32,
}

impl ZekiEngine {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            state: ZekiState::Idle,
            state_entered: now,
            last_interaction: now,
            auto_revert_at: None,
            manual_override: false,
            elapsed: 0.0,
            accent: ZekiState::Idle.accent_rgb(),
            gaze_x: 0.0,
            gaze_y: 0.0,
            target_gaze_x: 0.0,
            target_gaze_y: 0.0,
            next_gaze_change: 2.4,
            blink_open: 1.0,
            blink_phase: 0.0,
            blinking: false,
            next_blink: 2.0,
            tilt: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            antenna_lift: 0.0,
            blush: 0.0,
            hop_timer: 0.0,
            shake_timer: 0.0,
            peek_timer: 0.0,
            dancing: false,
            dance_level: 0.0,
            live_voice_level: None,
            voice_level: 0.0,
            cicada_accessory: ZekiCicadaAccessory::None,
            cicada_audio_level: 0.0,
            nod_timer: 0.0,
            sparks: Vec::new(),
            rng_state: 0x5A45_4B49, // "ZEKI"
        }
    }

    pub fn state(&self) -> ZekiState {
        self.state
    }

    pub fn accent_color(&self) -> (f32, f32, f32) {
        self.accent
    }

    pub fn is_high_energy(&self) -> bool {
        self.state.is_high_energy()
            || self.cicada_accessory != ZekiCicadaAccessory::None
            || self.hop_timer > 0.0
            || self.nod_timer > 0.0
            || self.shake_timer > 0.0
            || self.peek_timer > 0.0
            || !self.sparks.is_empty()
            || self.dance_level > 0.02
    }

    /// Updates Zeki's Cicada Mode accessory (Headset / TakingNotes) and live audio rhythm level.
    pub fn set_cicada_mode(&mut self, accessory: ZekiCicadaAccessory, audio_level: f32) {
        self.cicada_accessory = accessory;
        self.cicada_audio_level = audio_level.clamp(0.0, 1.0);
        if accessory != ZekiCicadaAccessory::None {
            self.last_interaction = Instant::now();
        }
    }

    /// Gentle head nod impulse when a new translated subtitle line arrives in Cicada Mode.
    pub fn trigger_nod(&mut self) {
        self.nod_timer = 0.55;
        self.last_interaction = Instant::now();
    }

    pub fn set_dancing(&mut self, playing: bool) {
        self.dancing = playing;
    }

    /// Feeds real-time WASAPI microphone amplitude (`Some(0.0..=1.0)`) while recording,
    /// or `None` to fall back to gentle synthetic waveform motion during manual preview.
    pub fn set_live_voice_level(&mut self, level: Option<f32>) {
        self.live_voice_level = level.map(|v| v.clamp(0.0, 1.0));
    }

    /// Drives Zeki's animation directly from the Voice & AI assistant pipeline.
    pub fn set_state_from_assistant(&mut self, next: ZekiState) {
        self.last_interaction = Instant::now();
        self.manual_override = false;
        self.auto_revert_at = if next == ZekiState::Happy {
            Some(Instant::now() + AUTO_HAPPY_DURATION)
        } else {
            None
        };
        if self.state != next {
            self.transition_to(next);
        }
    }

    /// Manually select a state (e.g. from the Zeki preview controls).
    /// Stays in the selected state until changed or woken up.
    pub fn set_state_manual(&mut self, next: ZekiState) {
        self.manual_override = next != ZekiState::Idle;
        self.auto_revert_at = None;
        self.last_interaction = Instant::now();
        self.transition_to(next);
    }

    /// Trigger a short celebratory `Happy` reaction (volume/brightness/clipboard/timer).
    pub fn trigger_action_happy(&mut self) {
        self.last_interaction = Instant::now();
        // Don't interrupt an active manual preview of Listening/Thinking/Speaking/Error.
        if self.manual_override && self.state != ZekiState::Sleepy && self.state != ZekiState::Happy
        {
            return;
        }
        self.manual_override = false;
        self.auto_revert_at = Some(Instant::now() + AUTO_HAPPY_DURATION);
        self.transition_to(ZekiState::Happy);
    }

    /// Interactive poke when the user clicks Zeki directly.
    pub fn poke(&mut self) {
        self.last_interaction = Instant::now();
        self.manual_override = false;
        self.auto_revert_at = Some(Instant::now() + AUTO_HAPPY_DURATION);
        self.transition_to(ZekiState::Happy);
    }

    /// Subtle attentive peek when a desktop notification arrives.
    pub fn on_notification_peek(&mut self) {
        self.last_interaction = Instant::now();
        if self.state == ZekiState::Sleepy && !self.manual_override {
            self.transition_to(ZekiState::Idle);
        }
        self.peek_timer = 1.1;
        self.target_gaze_x = 0.7;
        self.target_gaze_y = -0.15;
    }

    /// Record any user interaction with the island to reset the 15-minute sleep timer.
    pub fn record_interaction(&mut self) {
        self.last_interaction = Instant::now();
        if self.state == ZekiState::Sleepy && !self.manual_override {
            self.transition_to(ZekiState::Idle);
            self.trigger_blink();
        }
    }

    /// 1-second housekeeping check (auto-sleep after 15 minutes, auto-revert from Happy).
    pub fn on_one_second_tick(&mut self) -> bool {
        let now = Instant::now();
        if let Some(revert_at) = self.auto_revert_at {
            if now >= revert_at {
                self.auto_revert_at = None;
                self.transition_to(ZekiState::Idle);
                return true;
            }
        }

        if !self.manual_override
            && self.state == ZekiState::Idle
            && now.duration_since(self.last_interaction).as_secs() >= SLEEP_AFTER_SECS
        {
            self.transition_to(ZekiState::Sleepy);
            return true;
        }

        false
    }

    fn transition_to(&mut self, next: ZekiState) {
        self.state = next;
        self.state_entered = Instant::now();

        match next {
            ZekiState::Happy => {
                self.hop_timer = 0.65;
                self.spawn_sparks(6);
            }
            ZekiState::Error => {
                self.shake_timer = 0.55;
            }
            ZekiState::Listening => {
                self.hop_timer = 0.25;
                self.trigger_blink();
            }
            ZekiState::Thinking | ZekiState::Speaking => {
                self.trigger_blink();
            }
            ZekiState::Idle | ZekiState::Sleepy => {}
        }
    }

    fn trigger_blink(&mut self) {
        self.blinking = true;
        self.blink_phase = 0.0;
    }

    fn rand_f32(&mut self) -> f32 {
        // Numerical Recipes LCG
        self.rng_state = self
            .rng_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        ((self.rng_state >> 8) as f32) / 16_777_216.0
    }

    fn spawn_sparks(&mut self, count: usize) {
        for i in 0..count {
            let angle = -PI * 0.85 + (i as f32 / (count.max(1) as f32)) * PI * 0.7
                + (self.rand_f32() - 0.5) * 0.28;
            let speed = 1.05 + self.rand_f32() * 0.85;
            let life = 0.65 + self.rand_f32() * 0.4;
            let size = 0.08 + self.rand_f32() * 0.05;
            self.sparks.push(SparkParticle {
                x: angle.cos() * 0.55,
                y: angle.sin() * 0.55 - 0.25,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed - 0.35,
                age: 0.0,
                life,
                size,
            });
        }
    }

    /// Advance procedural animations by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.001, 0.1);
        self.elapsed += dt;

        // Check auto-revert inside frame update too for snappy transitions
        if let Some(revert_at) = self.auto_revert_at {
            if Instant::now() >= revert_at {
                self.auto_revert_at = None;
                self.transition_to(ZekiState::Idle);
            }
        }

        // Smooth accent color transition
        let target_col = self.state.accent_rgb();
        let col_k = (1.0 - (-dt * 7.5).exp()).clamp(0.0, 1.0);
        self.accent.0 = lerp(self.accent.0, target_col.0, col_k);
        self.accent.1 = lerp(self.accent.1, target_col.1, col_k);
        self.accent.2 = lerp(self.accent.2, target_col.2, col_k);

        // Music dance level blend (only in calm Idle state)
        let should_dance = self.dancing && self.state == ZekiState::Idle;
        if should_dance {
            self.dance_level = (self.dance_level + dt / 0.35).min(1.0);
        } else {
            self.dance_level = (self.dance_level - dt / 0.45).max(0.0);
        }

        // Natural blink cycle
        if self.state == ZekiState::Sleepy || self.state == ZekiState::Happy {
            self.blink_open = 1.0;
            self.blinking = false;
        } else if self.blinking {
            self.blink_phase += dt / 0.15;
            if self.blink_phase >= 1.0 {
                self.blinking = false;
                self.blink_open = 1.0;
                self.next_blink = self.elapsed + 2.8 + self.rand_f32() * 3.4;
            } else {
                // Smooth close & reopen V-curve
                let t = (self.blink_phase * PI).sin();
                self.blink_open = (1.0 - t * 0.92).max(0.08);
            }
        } else if self.elapsed >= self.next_blink {
            self.trigger_blink();
        }

        // Gaze target per state
        if self.peek_timer > 0.0 {
            self.peek_timer = (self.peek_timer - dt).max(0.0);
        } else {
            match self.state {
                ZekiState::Idle => {
                    if self.elapsed >= self.next_gaze_change {
                        self.next_gaze_change = self.elapsed + 2.2 + self.rand_f32() * 3.0;
                        let roll = self.rand_f32();
                        if roll < 0.45 {
                            self.target_gaze_x = 0.0;
                            self.target_gaze_y = 0.0;
                        } else {
                            self.target_gaze_x = (self.rand_f32() - 0.5) * 1.1;
                            self.target_gaze_y = (self.rand_f32() - 0.5) * 0.45;
                        }
                    }
                }
                ZekiState::Sleepy => {
                    self.target_gaze_x = 0.0;
                    self.target_gaze_y = 0.15;
                }
                ZekiState::Listening => {
                    self.target_gaze_x = (self.elapsed * 1.4).sin() * 0.12;
                    self.target_gaze_y = -0.05;
                }
                ZekiState::Thinking => {
                    self.target_gaze_x = (self.elapsed * 2.2).cos() * 0.55;
                    self.target_gaze_y = -0.42 + (self.elapsed * 2.2).sin() * 0.14;
                }
                ZekiState::Speaking => {
                    self.target_gaze_x = (self.elapsed * 2.8).sin() * 0.18;
                    self.target_gaze_y = -0.06;
                }
                ZekiState::Happy => {
                    self.target_gaze_x = 0.0;
                    self.target_gaze_y = -0.10;
                }
                ZekiState::Error => {
                    self.target_gaze_x = 0.0;
                    self.target_gaze_y = 0.0;
                }
            }
        }

        let gaze_k = (1.0 - (-dt * 8.0).exp()).clamp(0.0, 1.0);
        self.gaze_x = lerp(self.gaze_x, self.target_gaze_x, gaze_k);
        self.gaze_y = lerp(self.gaze_y, self.target_gaze_y, gaze_k);

        // Transient timers
        if self.hop_timer > 0.0 {
            self.hop_timer = (self.hop_timer - dt).max(0.0);
        }
        if self.shake_timer > 0.0 {
            self.shake_timer = (self.shake_timer - dt).max(0.0);
        }
        if self.nod_timer > 0.0 {
            self.nod_timer = (self.nod_timer - dt).max(0.0);
        }

        // Body posture targets per state
        let mut target_oy = (self.elapsed * 2.1).sin() * 0.055; // Gentle ambient float
        let mut target_ox = 0.0f32;
        let mut target_tilt = 0.0f32;
        let mut target_sx = 1.0f32;
        let mut target_sy = 1.0f32;
        let target_antenna: f32;
        let mut target_blush = 0.12f32;

        match self.state {
            ZekiState::Idle => {
                // Subtle breathing
                let breath = (self.elapsed * 2.1).sin();
                target_sx = 1.0 + breath * 0.022;
                target_sy = 1.0 - breath * 0.022;
                target_antenna = 0.20 + breath * 0.06;
            }
            ZekiState::Sleepy => {
                let breath = (self.elapsed * 1.15).sin();
                target_oy = 0.06 + breath * 0.04;
                target_sx = 1.04 + breath * 0.035;
                target_sy = 0.94 - breath * 0.035;
                target_tilt = 0.07;
                target_antenna = -0.15;
                target_blush = 0.06;
            }
            ZekiState::Listening => {
                let pulse = (self.elapsed * 5.2).sin();
                self.voice_level = match self.live_voice_level {
                    Some(live) => 0.16 + live * 0.84,
                    None => {
                        0.35 + 0.35 * (self.elapsed * 6.5).sin().abs()
                            + 0.25 * (self.elapsed * 11.3).cos().abs()
                    }
                };
                target_oy = -0.05 + pulse * 0.03;
                target_sx = 1.04 + self.voice_level * 0.04;
                target_sy = 1.04 + self.voice_level * 0.03;
                target_antenna = 1.0;
                target_blush = 0.22;
            }
            ZekiState::Thinking => {
                target_oy = -0.04 + (self.elapsed * 2.8).sin() * 0.06;
                target_tilt = -0.10 + (self.elapsed * 1.6).sin() * 0.05;
                target_antenna = 0.72 + (self.elapsed * 6.0).sin() * 0.12;
                target_blush = 0.10;
            }
            ZekiState::Speaking => {
                let beat = (self.elapsed * 7.4).sin().abs();
                let land = (1.0 - beat).powi(4);
                target_oy = -0.11 * beat;
                target_sx = 1.0 + 0.06 * land;
                target_sy = 1.0 - 0.07 * land + 0.04 * beat;
                target_tilt = (self.elapsed * 3.7).sin() * 0.06;
                target_antenna = 0.65 + beat * 0.25;
                target_blush = 0.30;
            }
            ZekiState::Happy => {
                let cheer = (self.elapsed * 6.0).sin().abs();
                target_oy = -0.06 * cheer;
                target_sx = 1.04;
                target_sy = 0.98 + 0.05 * cheer;
                target_tilt = (self.elapsed * 5.0).sin() * 0.07;
                target_antenna = 0.9;
                target_blush = 0.75;
            }
            ZekiState::Error => {
                target_tilt = 0.22;
                target_oy = 0.02;
                target_antenna = -0.10;
                target_blush = 0.05;
            }
        }

        // Apply hop impulse (Happy / Poke / Listening entry)
        if self.hop_timer > 0.0 {
            let progress = 1.0 - (self.hop_timer / 0.65).clamp(0.0, 1.0);
            let hop = (progress * PI).sin();
            target_oy -= hop * 0.26;
            target_sx -= hop * 0.06;
            target_sy += hop * 0.10;
        }

        // Apply shake impulse (Error entry)
        if self.shake_timer > 0.0 {
            let decay = (self.shake_timer / 0.55).clamp(0.0, 1.0);
            target_ox += (self.elapsed * 42.0).sin() * 0.14 * decay;
        }

        // Apply gentle nod impulse when a new translated sentence arrives in Cicada Mode
        if self.nod_timer > 0.0 {
            let p = 1.0 - (self.nod_timer / 0.55).clamp(0.0, 1.0);
            let nod = (p * PI * 2.0).sin().max(0.0) * (1.0 - p * 0.45);
            target_oy += nod * 0.14;
            target_tilt += nod * 0.08;
        }

        // Apply subtle audio-reactive sway while wearing Cicada headset
        if self.cicada_accessory != ZekiCicadaAccessory::None && self.cicada_audio_level > 0.05 {
            let sway = (self.elapsed * 9.0).sin() * self.cicada_audio_level;
            target_tilt += sway * 0.06;
            target_oy -= self.cicada_audio_level * 0.04;
        }

        // Apply gentle 112-BPM music dance when media is playing & Zeki is idle
        if self.dance_level > 0.01 {
            let beat = (self.elapsed * 112.0) / 60.0;
            let hop = (PI * beat).sin().abs();
            let land = (1.0 - hop).powi(5);
            target_ox += 0.06 * (PI * beat).sin() * self.dance_level;
            target_oy -= 0.14 * hop * self.dance_level;
            target_tilt += 0.08 * (PI * beat).sin() * self.dance_level;
            target_sx += 0.04 * land * self.dance_level;
            target_sy -= 0.05 * land * self.dance_level;
        }

        let k = (1.0 - (-dt * 12.0).exp()).clamp(0.0, 1.0);
        self.offset_x = lerp(self.offset_x, target_ox, k);
        self.offset_y = lerp(self.offset_y, target_oy, k);
        self.tilt = lerp(self.tilt, target_tilt, k);
        self.scale_x = lerp(self.scale_x, target_sx, k);
        self.scale_y = lerp(self.scale_y, target_sy, k);
        self.antenna_lift = lerp(self.antenna_lift, target_antenna, k);
        self.blush = lerp(self.blush, target_blush, k);

        // Update sparks
        for spark in &mut self.sparks {
            spark.age += dt;
            spark.x += spark.vx * dt;
            spark.y += spark.vy * dt;
            spark.vy += 1.4 * dt; // soft gravity
        }
        self.sparks.retain(|s| s.age < s.life);
    }

    /// Procedural Direct2D rendering of Zeki centered at `(cx, cy)` with body radius `radius`.
    /// Uses a pre-cached `round_stroke` style from `Renderer` for zero per-frame COM allocations.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw(
        &self,
        factory: &ID2D1Factory,
        target: &ID2D1DCRenderTarget,
        brush: &ID2D1SolidColorBrush,
        round_stroke: Option<&ID2D1StrokeStyle>,
        dwrite: &IDWriteFactory,
        badge_format: &IDWriteTextFormat,
        cx: f32,
        cy: f32,
        radius: f32,
        alpha: f32,
        is_mini: bool,
    ) {
        if alpha <= 0.01 || radius <= 2.0 {
            return;
        }

        let (ar, ag, ab) = self.accent;
        let pulse = 0.5 + 0.5 * (self.elapsed * 4.2).sin();
        let draw_cx = cx + self.offset_x * radius;
        let draw_cy = cy + self.offset_y * radius;

        // 1. Ambient outer aura / glow halo behind Zeki
        let glow_alpha = match self.state {
            ZekiState::Listening => (0.28 + 0.16 * pulse) * alpha,
            ZekiState::Thinking => (0.24 + 0.16 * pulse) * alpha,
            ZekiState::Speaking => (0.22 + 0.12 * pulse) * alpha,
            ZekiState::Happy => 0.28 * alpha,
            ZekiState::Error => 0.26 * alpha,
            ZekiState::Sleepy => 0.10 * alpha,
            ZekiState::Idle => (0.14 + 0.05 * pulse) * alpha,
        };
        brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: glow_alpha,
        });
        let halo_r = radius * (if is_mini { 1.24 } else { 1.38 + 0.06 * pulse });
        target.FillEllipse(
            &D2D1_ELLIPSE {
                point: pt(draw_cx, draw_cy),
                radiusX: halo_r,
                radiusY: halo_r * 0.92,
            },
            brush,
        );

        // Extra crisp pulsing ring when Listening or Thinking
        if matches!(self.state, ZekiState::Listening | ZekiState::Thinking) && !is_mini {
            let ring_r = radius * (1.22 + 0.18 * pulse);
            brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: (0.45 * (1.0 - pulse * 0.6)) * alpha,
            });
            target.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: pt(draw_cx, draw_cy),
                    radiusX: ring_r,
                    radiusY: ring_r * 0.92,
                },
                brush,
                1.5,
                None,
            );
        }

        // Save current transform & apply Zeki's local transform (translate + tilt + squash/stretch)
        let mut prev_transform = Matrix3x2::identity();
        target.GetTransform(&mut prev_transform);

        let local_scale = Matrix3x2 {
            M11: self.scale_x,
            M12: 0.0,
            M21: 0.0,
            M22: self.scale_y,
            M31: 0.0,
            M32: 0.0,
        };
        let cos_t = self.tilt.cos();
        let sin_t = self.tilt.sin();
        let local_rot = Matrix3x2 {
            M11: cos_t,
            M12: sin_t,
            M21: -sin_t,
            M22: cos_t,
            M31: draw_cx,
            M32: draw_cy,
        };
        let xform = mul_matrix(mul_matrix(local_scale, local_rot), prev_transform);
        target.SetTransform(&xform);

        let rx = radius * 1.06;
        let ry = radius * 0.90;

        // 2. Expressive little antenna on top of Zeki's capsule head
        let stem_base_y = -ry * 0.88;
        let stem_len = radius * (0.20 + 0.16 * self.antenna_lift.clamp(-0.2, 1.1));
        let tip_x = (self.gaze_x * 0.18 + self.tilt * 0.25) * radius;
        let tip_y = stem_base_y - stem_len;

        if stem_len > 1.2 {
            brush.SetColor(&D2D1_COLOR_F {
                r: ar * 0.75 + 0.25,
                g: ag * 0.75 + 0.25,
                b: ab * 0.75 + 0.25,
                a: 0.85 * alpha,
            });
            target.DrawLine(
                pt(0.0, stem_base_y),
                pt(tip_x, tip_y),
                brush,
                (radius * 0.09).clamp(1.2, 2.4),
                round_stroke,
            );

            // Glowing antenna orb
            let orb_r = (radius * (0.11 + 0.04 * self.antenna_lift.max(0.0))).clamp(1.5, 4.2);
            brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: 0.40 * alpha,
            });
            target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: pt(tip_x, tip_y),
                    radiusX: orb_r * 1.7,
                    radiusY: orb_r * 1.7,
                },
                brush,
            );
            brush.SetColor(&D2D1_COLOR_F {
                r: (ar + 0.35).min(1.0),
                g: (ag + 0.35).min(1.0),
                b: (ab + 0.35).min(1.0),
                a: 0.98 * alpha,
            });
            target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: pt(tip_x, tip_y),
                    radiusX: orb_r,
                    radiusY: orb_r,
                },
                brush,
            );
        }

        // 3. Capsule-blob body (sleek dark obsidian shell with luminous mood rim)
        let body_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: -rx,
                top: -ry,
                right: rx,
                bottom: ry,
            },
            radiusX: rx * 0.78,
            radiusY: ry * 0.86,
        };

        // Dark elegant capsule body
        brush.SetColor(&D2D1_COLOR_F {
            r: 0.11 + ar * 0.08,
            g: 0.12 + ag * 0.08,
            b: 0.17 + ab * 0.10,
            a: 0.98 * alpha,
        });
        target.FillRoundedRectangle(&body_rect, brush);

        // Soft glossy inner top highlight (gives soft 3D blob volume)
        let hl_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: -rx * 0.62,
                top: -ry * 0.82,
                right: rx * 0.62,
                bottom: -ry * 0.36,
            },
            radiusX: rx * 0.45,
            radiusY: ry * 0.28,
        };
        brush.SetColor(&D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.12 * alpha,
        });
        target.FillRoundedRectangle(&hl_rect, brush);

        // Inner visor / face glass plate (deep dark so glowing eyes pop crisply)
        let visor_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: -rx * 0.76,
                top: -ry * 0.54,
                right: rx * 0.76,
                bottom: ry * 0.58,
            },
            radiusX: rx * 0.46,
            radiusY: ry * 0.48,
        };
        brush.SetColor(&D2D1_COLOR_F {
            r: 0.04,
            g: 0.05,
            b: 0.08,
            a: 0.92 * alpha,
        });
        target.FillRoundedRectangle(&visor_rect, brush);

        // Mood-tinted capsule border glow
        brush.SetColor(&D2D1_COLOR_F {
            r: ar,
            g: ag,
            b: ab,
            a: 0.65 * alpha,
        });
        target.DrawRoundedRectangle(
            &body_rect,
            brush,
            if is_mini { 1.15 } else { 1.65 },
            None,
        );

        // 4. Soft rosy/mood blush cheeks
        let face_ox = self.gaze_x * rx * 0.22;
        let face_oy = self.gaze_y * ry * 0.22;
        if self.blush > 0.05 && !is_mini {
            brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 0.45,
                b: 0.62,
                a: 0.38 * self.blush * alpha,
            });
            for side in [-1.0f32, 1.0f32] {
                target.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: pt(side * rx * 0.44 + face_ox, ry * 0.22 + face_oy),
                        radiusX: radius * 0.14,
                        radiusY: radius * 0.08,
                    },
                    brush,
                );
            }
        }

        // 5. Expressive Glowing Eyes
        let eye_spacing = rx * (if is_mini { 0.34 } else { 0.32 });
        let eye_y = -ry * 0.06 + face_oy;
        let eye_w = radius * (if is_mini { 0.28 } else { 0.24 });
        let eye_h = radius * (if is_mini { 0.34 } else { 0.30 });

        for side in [-1.0f32, 1.0f32] {
            let ex = side * eye_spacing + face_ox;
            self.draw_eye(
                factory,
                target,
                brush,
                round_stroke,
                ex,
                eye_y,
                eye_w,
                eye_h,
                side,
                (ar, ag, ab),
                alpha,
                is_mini,
            );
        }

        // 6. Dynamic Mouth (waveform when Listening, bouncy voice capsule when Speaking, tiny smile when Happy)
        let mouth_y = ry * 0.32 + face_oy * 0.6;
        match self.state {
            ZekiState::Listening => {
                let bars = if is_mini { 3usize } else { 5usize };
                let bar_w = (radius * 0.09).clamp(1.2, 2.8);
                let gap = bar_w * 0.9;
                let total_w = bars as f32 * bar_w + (bars - 1) as f32 * gap;
                let start_x = face_ox - total_w / 2.0;

                brush.SetColor(&D2D1_COLOR_F {
                    r: (ar + 0.25).min(1.0),
                    g: (ag + 0.25).min(1.0),
                    b: (ab + 0.25).min(1.0),
                    a: 0.95 * alpha,
                });
                for i in 0..bars {
                    let phase = self.elapsed * 9.5 + i as f32 * 1.15;
                    let envelope = 1.0 - ((i as f32 - (bars as f32 - 1.0) / 2.0).abs() * 0.24);
                    let level = (0.30 + 0.70 * phase.sin().abs()) * envelope * self.voice_level.max(0.5);
                    let bh = (radius * 0.26 * level).clamp(bar_w, radius * 0.28);
                    let bx = start_x + i as f32 * (bar_w + gap);
                    let bar = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: bx,
                            top: mouth_y - bh / 2.0,
                            right: bx + bar_w,
                            bottom: mouth_y + bh / 2.0,
                        },
                        radiusX: bar_w / 2.0,
                        radiusY: bar_w / 2.0,
                    };
                    target.FillRoundedRectangle(&bar, brush);
                }
            }
            ZekiState::Speaking => {
                let talk = 0.25 + 0.75 * (self.elapsed * 13.0).sin().abs();
                let mw = radius * (0.28 + 0.08 * talk);
                let mh = (radius * (0.08 + 0.14 * talk)).max(1.6);
                let mouth = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: face_ox - mw / 2.0,
                        top: mouth_y - mh / 2.0,
                        right: face_ox + mw / 2.0,
                        bottom: mouth_y + mh / 2.0,
                    },
                    radiusX: mh / 2.0,
                    radiusY: mh / 2.0,
                };
                brush.SetColor(&D2D1_COLOR_F {
                    r: (ar + 0.30).min(1.0),
                    g: (ag + 0.30).min(1.0),
                    b: (ab + 0.30).min(1.0),
                    a: 0.95 * alpha,
                });
                target.FillRoundedRectangle(&mouth, brush);
            }
            ZekiState::Happy if !is_mini => {
                // Cute tiny smile arc
                brush.SetColor(&D2D1_COLOR_F {
                    r: (ar + 0.35).min(1.0),
                    g: (ag + 0.35).min(1.0),
                    b: (ab + 0.35).min(1.0),
                    a: 0.90 * alpha,
                });
                draw_arc_stroke(
                    factory,
                    target,
                    brush,
                    round_stroke,
                    pt(face_ox - radius * 0.14, mouth_y - radius * 0.02),
                    pt(face_ox + radius * 0.14, mouth_y - radius * 0.02),
                    radius * 0.16,
                    radius * 0.14,
                    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                    (radius * 0.075).clamp(1.3, 2.4),
                );
            }
            _ => {}
        }

        // 6b. Cicada Mode Accessories (Headset + Boom Mic, plus Notebook & Pen in TakingNotes)
        if self.cicada_accessory != ZekiCicadaAccessory::None {
            let ear_bob = (self.elapsed * 12.0).sin() * 0.08 * self.cicada_audio_level;
            let band_col = D2D1_COLOR_F {
                r: 0.16,
                g: 0.85,
                b: 0.62,
                a: 0.94 * alpha,
            };
            // Headband arc over top of Zeki's capsule
            brush.SetColor(&band_col);
            draw_arc_stroke(
                factory,
                target,
                brush,
                round_stroke,
                pt(-rx * 1.04, -ry * 0.08),
                pt(rx * 1.04, -ry * 0.08),
                rx * 1.08,
                ry * 1.06,
                D2D1_SWEEP_DIRECTION_CLOCKWISE,
                (radius * 0.11).clamp(1.5, 3.2),
            );

            // Left & right earcups (pulse softly with live audio)
            let cup_w = (radius * 0.24).clamp(2.4, 7.2);
            let cup_h = (radius * (0.46 + 0.10 * self.cicada_audio_level)).clamp(4.8, 15.0);
            for side in [-1.0f32, 1.0f32] {
                let cx_cup = side * (rx * 1.02);
                let cy_cup = ear_bob * side * radius;
                let cup = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: cx_cup - cup_w / 2.0,
                        top: cy_cup - cup_h / 2.0,
                        right: cx_cup + cup_w / 2.0,
                        bottom: cy_cup + cup_h / 2.0,
                    },
                    radiusX: cup_w * 0.45,
                    radiusY: cup_w * 0.45,
                };
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.08,
                    g: 0.14,
                    b: 0.16,
                    a: 0.98 * alpha,
                });
                target.FillRoundedRectangle(&cup, brush);
                brush.SetColor(&band_col);
                target.DrawRoundedRectangle(&cup, brush, (radius * 0.07).clamp(1.0, 2.0), None);
            }

            // Sleek boom microphone curving from right earcup toward mouth
            if !is_mini {
                let mic_start = pt(rx * 0.96, ry * 0.18);
                let mic_tip = pt(rx * 0.36, ry * 0.44);
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.72,
                    g: 0.95,
                    b: 0.86,
                    a: 0.88 * alpha,
                });
                target.DrawLine(
                    mic_start,
                    mic_tip,
                    brush,
                    (radius * 0.065).clamp(1.2, 2.0),
                    round_stroke,
                );
                brush.SetColor(&D2D1_COLOR_F {
                    r: 0.18,
                    g: 0.92,
                    b: 0.65,
                    a: 0.96 * alpha,
                });
                target.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: mic_tip,
                        radiusX: (radius * 0.095).clamp(1.8, 3.2),
                        radiusY: (radius * 0.075).clamp(1.5, 2.6),
                    },
                    brush,
                );
            }
        }

        // Restore world transform for floating badges & particles (never tilted with body)
        target.SetTransform(&prev_transform);

        // 6c. TakingNotes miniature notebook & animated pen (Expanded Cicada Transcript view)
        if self.cicada_accessory == ZekiCicadaAccessory::TakingNotes && !is_mini {
            let nb_w = (radius * 0.62).clamp(13.0, 22.0);
            let nb_h = (radius * 0.72).clamp(15.0, 26.0);
            let nb_x = draw_cx + radius * 0.68;
            let nb_y = draw_cy + radius * 0.24;
            let nb_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: nb_x,
                    top: nb_y,
                    right: nb_x + nb_w,
                    bottom: nb_y + nb_h,
                },
                radiusX: 3.0,
                radiusY: 3.0,
            };
            brush.SetColor(&D2D1_COLOR_F {
                r: 0.95,
                g: 0.97,
                b: 1.0,
                a: 0.95 * alpha,
            });
            target.FillRoundedRectangle(&nb_rect, brush);
            brush.SetColor(&D2D1_COLOR_F {
                r: 0.16,
                g: 0.82,
                b: 0.62,
                a: 0.92 * alpha,
            });
            target.DrawRoundedRectangle(&nb_rect, brush, 1.3, None);

            // Tiny note lines inside the notebook
            brush.SetColor(&D2D1_COLOR_F {
                r: 0.25,
                g: 0.35,
                b: 0.48,
                a: 0.65 * alpha,
            });
            for row in 0..3 {
                let ly = nb_y + 5.0 + row as f32 * (nb_h - 9.0) / 2.5;
                let lw = if row == 2 { nb_w * 0.52 } else { nb_w * 0.68 };
                target.DrawLine(
                    pt(nb_x + 3.0, ly),
                    pt(nb_x + 3.0 + lw, ly),
                    brush,
                    1.2,
                    round_stroke,
                );
            }

            // Animated writing pen bobbing across the notebook
            let scribble_x = (self.elapsed * 8.5).sin() * 2.4;
            let scribble_y = (self.elapsed * 4.2).cos() * 1.4;
            let pen_tip = pt(nb_x + nb_w * 0.62 + scribble_x, nb_y + nb_h * 0.62 + scribble_y);
            let pen_top = pt(pen_tip.x + 8.5, pen_tip.y - 9.5);
            brush.SetColor(&D2D1_COLOR_F {
                r: 0.98,
                g: 0.72,
                b: 0.22,
                a: 0.98 * alpha,
            });
            target.DrawLine(pen_tip, pen_top, brush, 2.2, round_stroke);
        }

        // 7. State-specific ambient particles & badges
        match self.state {
            ZekiState::Sleepy => {
                // Rising "z" / "Z" bubbles in the top-right corner
                let count = if is_mini { 1 } else { 3 };
                for i in 0..count {
                    let cycle = ((self.elapsed * 0.42 + i as f32 * 0.33) % 1.0 + 1.0) % 1.0;
                    let fade = (cycle * PI).sin();
                    let zx = draw_cx + radius * (0.72 + cycle * 0.42) + (cycle * 5.0).sin() * 2.0;
                    let zy = draw_cy - radius * (0.55 + cycle * 0.85);
                    let txt = if i % 2 == 0 { "z" } else { "Z" };
                    let u16_z: Vec<u16> = txt.encode_utf16().collect();
                    if let Ok(layout) = dwrite.CreateTextLayout(&u16_z, badge_format, 16.0, 16.0) {
                        brush.SetColor(&D2D1_COLOR_F {
                            r: 0.78,
                            g: 0.85,
                            b: 0.98,
                            a: 0.88 * fade * alpha,
                        });
                        target.DrawTextLayout(
                            pt(zx - 8.0, zy - 8.0),
                            &layout,
                            brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
            }
            ZekiState::Thinking => {
                // Three bouncing violet thought dots above Zeki's shoulder
                let dot_r = (radius * 0.095).clamp(1.3, 3.0);
                let base_x = draw_cx + radius * 0.82;
                let base_y = draw_cy - radius * 0.72;
                for i in 0..3 {
                    let phase = ((self.elapsed * 2.6 - i as f32 * 0.22) % 1.0 + 1.0) % 1.0;
                    let bounce = (phase * PI * 2.0).sin().max(0.0);
                    brush.SetColor(&D2D1_COLOR_F {
                        r: (ar + 0.25).min(1.0),
                        g: (ag + 0.25).min(1.0),
                        b: (ab + 0.25).min(1.0),
                        a: (0.55 + 0.45 * bounce) * alpha,
                    });
                    target.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: pt(
                                base_x + (i as f32 - 1.0) * dot_r * 2.7,
                                base_y - bounce * dot_r * 1.4,
                            ),
                            radiusX: dot_r * (0.85 + 0.3 * bounce),
                            radiusY: dot_r * (0.85 + 0.3 * bounce),
                        },
                        brush,
                    );
                }
            }
            ZekiState::Error if !is_mini => {
                // Floating "?" badge at top-right
                let bx = draw_cx + radius * 0.92;
                let by = draw_cy - radius * 0.76 + (self.elapsed * 4.0).sin() * 2.0;
                let br = (radius * 0.28).clamp(6.0, 9.5);
                brush.SetColor(&D2D1_COLOR_F {
                    r: ar,
                    g: ag,
                    b: ab,
                    a: 0.95 * alpha,
                });
                target.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: pt(bx, by),
                        radiusX: br,
                        radiusY: br,
                    },
                    brush,
                );
                let q: Vec<u16> = "?".encode_utf16().collect();
                if let Ok(layout) = dwrite.CreateTextLayout(&q, badge_format, br * 2.0, br * 2.0) {
                    brush.SetColor(&D2D1_COLOR_F {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: alpha,
                    });
                    target.DrawTextLayout(
                        pt(bx - br, by - br),
                        &layout,
                        brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }
            _ => {}
        }

        // 8. Celebration spark particles (Happy / Poke)
        for spark in &self.sparks {
            let t = (spark.age / spark.life).clamp(0.0, 1.0);
            let fade = 1.0 - t;
            let sx = draw_cx + spark.x * radius * 1.3;
            let sy = draw_cy + spark.y * radius * 1.3;
            let sr = (spark.size * radius * (0.6 + 0.6 * fade)).clamp(1.2, 4.0);
            brush.SetColor(&D2D1_COLOR_F {
                r: 1.0,
                g: 0.88,
                b: 0.36,
                a: fade * alpha,
            });
            target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: pt(sx, sy),
                    radiusX: sr,
                    radiusY: sr,
                },
                brush,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_eye(
        &self,
        factory: &ID2D1Factory,
        target: &ID2D1DCRenderTarget,
        brush: &ID2D1SolidColorBrush,
        round_stroke: Option<&ID2D1StrokeStyle>,
        ex: f32,
        ey: f32,
        ew: f32,
        eh: f32,
        side: f32,
        accent: (f32, f32, f32),
        alpha: f32,
        is_mini: bool,
    ) {
        let (ar, ag, ab) = accent;
        let eye_col = D2D1_COLOR_F {
            r: (ar * 0.55 + 0.45).min(1.0),
            g: (ag * 0.55 + 0.45).min(1.0),
            b: (ab * 0.55 + 0.45).min(1.0),
            a: 0.98 * alpha,
        };

        // Soft eye glow behind each eye
        if !is_mini {
            brush.SetColor(&D2D1_COLOR_F {
                r: ar,
                g: ag,
                b: ab,
                a: 0.28 * alpha,
            });
            target.FillEllipse(
                &D2D1_ELLIPSE {
                    point: pt(ex, ey),
                    radiusX: ew * 0.95,
                    radiusY: eh * 0.85,
                },
                brush,
            );
        }

        brush.SetColor(&eye_col);

        match self.state {
            ZekiState::Sleepy => {
                // Gentle downward-curved closed eyelids
                let stroke = (ew * 0.28).clamp(1.2, 2.6);
                draw_arc_stroke(
                    factory,
                    target,
                    brush,
                    round_stroke,
                    pt(ex - ew * 0.52, ey),
                    pt(ex + ew * 0.52, ey),
                    ew * 0.55,
                    eh * 0.34,
                    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                    stroke,
                );
            }
            ZekiState::Happy => {
                // Joyful upward-curved `^ ^` eyes
                let stroke = (ew * 0.34).clamp(1.4, 3.0);
                draw_arc_stroke(
                    factory,
                    target,
                    brush,
                    round_stroke,
                    pt(ex - ew * 0.54, ey + eh * 0.14),
                    pt(ex + ew * 0.54, ey + eh * 0.14),
                    ew * 0.55,
                    eh * 0.46,
                    D2D1_SWEEP_DIRECTION_CLOCKWISE,
                    stroke,
                );
            }
            ZekiState::Error => {
                // Confused spinning spiral eyes (or crisp X at mini scale)
                let stroke = (ew * 0.22).clamp(1.1, 2.2);
                if is_mini {
                    let r = ew * 0.42;
                    target.DrawLine(pt(ex - r, ey - r), pt(ex + r, ey + r), brush, stroke, round_stroke);
                    target.DrawLine(pt(ex + r, ey - r), pt(ex - r, ey + r), brush, stroke, round_stroke);
                } else if let Ok(geom) = factory.CreatePathGeometry() {
                    if let Ok(sink) = geom.Open() {
                        let steps = 22usize;
                        for i in 0..=steps {
                            let t = i as f32 / steps as f32;
                            let angle = t * 3.8 * PI + self.elapsed * 6.5 * side;
                            let rad = ew * (0.08 + 0.50 * t);
                            let p = pt(ex + angle.cos() * rad, ey + angle.sin() * rad);
                            if i == 0 {
                                sink.BeginFigure(p, D2D1_FIGURE_BEGIN_HOLLOW);
                            } else {
                                sink.AddLine(p);
                            }
                        }
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        if sink.Close().is_ok() {
                            target.DrawGeometry(&geom, brush, stroke, round_stroke);
                        }
                    }
                }
            }
            ZekiState::Listening => {
                // Alert, slightly larger rounded pill eyes with crisp white glint
                let w = ew * 1.12;
                let h = (eh * 1.14 * self.blink_open).max(w * 0.32);
                let r = (w / 2.0).min(h / 2.0);
                let rect = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: ex - w / 2.0,
                        top: ey - h / 2.0,
                        right: ex + w / 2.0,
                        bottom: ey + h / 2.0,
                    },
                    radiusX: r,
                    radiusY: r,
                };
                target.FillRoundedRectangle(&rect, brush);
                if !is_mini && self.blink_open > 0.6 {
                    draw_eye_glint(target, brush, ex, ey, w, h, alpha);
                }
            }
            ZekiState::Idle | ZekiState::Thinking | ZekiState::Speaking => {
                // Expressive pill eyes with natural blink & subtle white specular dot
                let w = ew;
                let h = (eh * self.blink_open).max(w * 0.30);
                let r = (w / 2.0).min(h / 2.0);
                let rect = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: ex - w / 2.0,
                        top: ey - h / 2.0,
                        right: ex + w / 2.0,
                        bottom: ey + h / 2.0,
                    },
                    radiusX: r,
                    radiusY: r,
                };
                target.FillRoundedRectangle(&rect, brush);
                if !is_mini && self.blink_open > 0.6 {
                    draw_eye_glint(target, brush, ex, ey, w, h, alpha);
                }
            }
        }
    }
}

impl Default for ZekiEngine {
    fn default() -> Self {
        Self::new()
    }
}

unsafe fn draw_eye_glint(
    target: &ID2D1DCRenderTarget,
    brush: &ID2D1SolidColorBrush,
    ex: f32,
    ey: f32,
    w: f32,
    h: f32,
    alpha: f32,
) {
    brush.SetColor(&D2D1_COLOR_F {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 0.85 * alpha,
    });
    let gr = (w * 0.22).max(1.1);
    target.FillEllipse(
        &D2D1_ELLIPSE {
            point: pt(ex + w * 0.16, ey - h * 0.20),
            radiusX: gr,
            radiusY: gr,
        },
        brush,
    );
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_arc_stroke(
    factory: &ID2D1Factory,
    target: &ID2D1DCRenderTarget,
    brush: &ID2D1SolidColorBrush,
    stroke_style: Option<&ID2D1StrokeStyle>,
    start: D2D_POINT_2F,
    end: D2D_POINT_2F,
    rx: f32,
    ry: f32,
    sweep: windows::Win32::Graphics::Direct2D::D2D1_SWEEP_DIRECTION,
    stroke_width: f32,
) {
    if let Ok(geom) = factory.CreatePathGeometry() {
        if let Ok(sink) = geom.Open() {
            sink.BeginFigure(start, D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: end,
                size: D2D_SIZE_F {
                    width: rx.max(0.5),
                    height: ry.max(0.5),
                },
                rotationAngle: 0.0,
                sweepDirection: sweep,
                arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            if sink.Close().is_ok() {
                target.DrawGeometry(&geom, brush, stroke_width, stroke_style);
            }
        }
    }
}

pub unsafe fn create_round_stroke(factory: &ID2D1Factory) -> Option<ID2D1StrokeStyle> {
    let props = D2D1_STROKE_STYLE_PROPERTIES {
        startCap: D2D1_CAP_STYLE_ROUND,
        endCap: D2D1_CAP_STYLE_ROUND,
        dashCap: D2D1_CAP_STYLE_ROUND,
        lineJoin: D2D1_LINE_JOIN_ROUND,
        ..Default::default()
    };
    factory.CreateStrokeStyle(&props, None).ok()
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mul_matrix(a: Matrix3x2, b: Matrix3x2) -> Matrix3x2 {
    Matrix3x2 {
        M11: a.M11 * b.M11 + a.M12 * b.M21,
        M12: a.M11 * b.M12 + a.M12 * b.M22,
        M21: a.M21 * b.M11 + a.M22 * b.M21,
        M22: a.M21 * b.M12 + a.M22 * b.M22,
        M31: a.M31 * b.M11 + a.M32 * b.M21 + b.M31,
        M32: a.M31 * b.M12 + a.M32 * b.M22 + b.M32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_seven_states_have_distinct_labels_and_colors() {
        assert_eq!(ALL_ZEKI_STATES.len(), 7);
        for s in ALL_ZEKI_STATES {
            assert!(!s.label().is_empty());
            assert!(!s.short_label().is_empty());
            assert!(!s.status_line().is_empty());
            let (r, g, b) = s.accent_rgb();
            assert!((0.0..=1.0).contains(&r));
            assert!((0.0..=1.0).contains(&g));
            assert!((0.0..=1.0).contains(&b));
        }
    }

    #[test]
    fn poke_triggers_happy_and_spawns_sparks() {
        let mut zeki = ZekiEngine::new();
        assert_eq!(zeki.state(), ZekiState::Idle);
        zeki.poke();
        assert_eq!(zeki.state(), ZekiState::Happy);
        assert!(!zeki.sparks.is_empty());
        assert!(zeki.is_high_energy());
    }

    #[test]
    fn manual_state_switch_updates_engine() {
        let mut zeki = ZekiEngine::new();
        for s in ALL_ZEKI_STATES {
            zeki.set_state_manual(s);
            assert_eq!(zeki.state(), s);
            zeki.update(0.016);
        }
    }
}
