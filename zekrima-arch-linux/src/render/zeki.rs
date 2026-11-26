//! Procedural Cairo character engine for "Zeki", the Zekrima Assistant mascot on Linux.
//!
//! 100% vector-drawn geometry with zero external image assets and a 7-state
//! expressive animation machine:
//! - `Idle`, `Sleepy` (auto after 15 min idle), `Listening`, `Thinking`,
//!   `Speaking`, `Happy` (poke sparks), and `Error`.

use std::f32::consts::PI;
use std::f64::consts::{FRAC_PI_2, PI as PI64};
use std::time::{Duration, Instant};

use gtk4::cairo;
use gtk4::pango;

const SLEEP_AFTER_SECS: u64 = 15 * 60;
const AUTO_HAPPY_DURATION: Duration = Duration::from_millis(2200);

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

    pub fn accent_rgb(self) -> (f32, f32, f32) {
        match self {
            Self::Idle => (0.22, 0.74, 0.97),
            Self::Sleepy => (0.45, 0.53, 0.68),
            Self::Listening => (0.13, 0.83, 0.93),
            Self::Thinking => (0.65, 0.55, 0.98),
            Self::Speaking => (0.18, 0.83, 0.75),
            Self::Happy => (0.20, 0.83, 0.50),
            Self::Error => (0.98, 0.44, 0.52),
        }
    }

    pub fn is_high_energy(self) -> bool {
        matches!(
            self,
            Self::Listening | Self::Thinking | Self::Speaking | Self::Happy | Self::Error
        )
    }

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

pub struct ZekiEngine {
    state: ZekiState,
    last_interaction: Instant,
    auto_revert_at: Option<Instant>,
    manual_override: bool,

    elapsed: f32,
    accent: (f32, f32, f32),
    gaze_x: f32,
    gaze_y: f32,
    target_gaze_x: f32,
    target_gaze_y: f32,
    next_gaze_change: f32,

    blink_open: f32,
    blink_phase: f32,
    blinking: bool,
    next_blink: f32,

    tilt: f32,
    offset_x: f32,
    offset_y: f32,
    scale_x: f32,
    scale_y: f32,
    antenna_lift: f32,
    blush: f32,

    hop_timer: f32,
    shake_timer: f32,
    peek_timer: f32,

    dancing: bool,
    dance_level: f32,

    live_voice_level: Option<f32>,
    voice_level: f32,

    sparks: Vec<SparkParticle>,
    rng_state: u32,
}

impl ZekiEngine {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            state: ZekiState::Idle,
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
            sparks: Vec::new(),
            rng_state: 0x5A45_4B49,
        }
    }

    pub fn state(&self) -> ZekiState {
        self.state
    }

    pub fn accent_color(&self) -> (f64, f64, f64) {
        (
            self.accent.0 as f64,
            self.accent.1 as f64,
            self.accent.2 as f64,
        )
    }

    pub fn is_high_energy(&self) -> bool {
        self.state.is_high_energy()
            || self.hop_timer > 0.0
            || self.shake_timer > 0.0
            || self.peek_timer > 0.0
            || !self.sparks.is_empty()
            || self.dance_level > 0.02
    }

    pub fn set_dancing(&mut self, playing: bool) {
        self.dancing = playing;
    }

    pub fn set_live_voice_level(&mut self, level: Option<f32>) {
        self.live_voice_level = level.map(|v| v.clamp(0.0, 1.0));
    }

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

    pub fn set_state_manual(&mut self, next: ZekiState) {
        self.manual_override = next != ZekiState::Idle;
        self.auto_revert_at = None;
        self.last_interaction = Instant::now();
        self.transition_to(next);
    }

    pub fn trigger_action_happy(&mut self) {
        self.last_interaction = Instant::now();
        if self.manual_override
            && self.state != ZekiState::Sleepy
            && self.state != ZekiState::Happy
        {
            return;
        }
        self.manual_override = false;
        self.auto_revert_at = Some(Instant::now() + AUTO_HAPPY_DURATION);
        self.transition_to(ZekiState::Happy);
    }

    pub fn poke(&mut self) {
        self.last_interaction = Instant::now();
        self.manual_override = false;
        self.auto_revert_at = Some(Instant::now() + AUTO_HAPPY_DURATION);
        self.transition_to(ZekiState::Happy);
    }

    pub fn on_notification_peek(&mut self) {
        self.last_interaction = Instant::now();
        if self.state == ZekiState::Sleepy && !self.manual_override {
            self.transition_to(ZekiState::Idle);
        }
        self.peek_timer = 1.1;
        self.target_gaze_x = 0.7;
        self.target_gaze_y = -0.15;
    }

    pub fn record_interaction(&mut self) {
        self.last_interaction = Instant::now();
        if self.state == ZekiState::Sleepy && !self.manual_override {
            self.transition_to(ZekiState::Idle);
            self.trigger_blink();
        }
    }

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
        self.rng_state = self
            .rng_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        ((self.rng_state >> 8) as f32) / 16_777_216.0
    }

    fn spawn_sparks(&mut self, count: usize) {
        for i in 0..count {
            let angle = -PI * 0.85
                + (i as f32 / (count.max(1) as f32)) * PI * 0.7
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

    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.001, 0.1);
        self.elapsed += dt;

        if let Some(revert_at) = self.auto_revert_at {
            if Instant::now() >= revert_at {
                self.auto_revert_at = None;
                self.transition_to(ZekiState::Idle);
            }
        }

        let target_col = self.state.accent_rgb();
        let col_k = (1.0 - (-dt * 7.5).exp()).clamp(0.0, 1.0);
        self.accent.0 = lerp(self.accent.0, target_col.0, col_k);
        self.accent.1 = lerp(self.accent.1, target_col.1, col_k);
        self.accent.2 = lerp(self.accent.2, target_col.2, col_k);

        let should_dance = self.dancing && self.state == ZekiState::Idle;
        if should_dance {
            self.dance_level = (self.dance_level + dt / 0.35).min(1.0);
        } else {
            self.dance_level = (self.dance_level - dt / 0.45).max(0.0);
        }

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
                let t = (self.blink_phase * PI).sin();
                self.blink_open = (1.0 - t * 0.92).max(0.08);
            }
        } else if self.elapsed >= self.next_blink {
            self.trigger_blink();
        }

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

        if self.hop_timer > 0.0 {
            self.hop_timer = (self.hop_timer - dt).max(0.0);
        }
        if self.shake_timer > 0.0 {
            self.shake_timer = (self.shake_timer - dt).max(0.0);
        }

        let mut target_oy = (self.elapsed * 2.1).sin() * 0.055;
        let mut target_ox = 0.0f32;
        let mut target_tilt = 0.0f32;
        let mut target_sx = 1.0f32;
        let mut target_sy = 1.0f32;
        let target_antenna: f32;
        let mut target_blush = 0.12f32;

        match self.state {
            ZekiState::Idle => {
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

        if self.hop_timer > 0.0 {
            let progress = 1.0 - (self.hop_timer / 0.65).clamp(0.0, 1.0);
            let hop = (progress * PI).sin();
            target_oy -= hop * 0.26;
            target_sx -= hop * 0.06;
            target_sy += hop * 0.10;
        }

        if self.shake_timer > 0.0 {
            let decay = (self.shake_timer / 0.55).clamp(0.0, 1.0);
            target_ox += (self.elapsed * 42.0).sin() * 0.14 * decay;
        }

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

        for spark in &mut self.sparks {
            spark.age += dt;
            spark.x += spark.vx * dt;
            spark.y += spark.vy * dt;
            spark.vy += 1.4 * dt;
        }
        self.sparks.retain(|s| s.age < s.life);
    }

    /// Procedural Cairo vector rendering of Zeki centered at `(cx, cy)` with radius `radius`.
    pub fn draw(
        &self,
        cr: &cairo::Context,
        cx: f64,
        cy: f64,
        radius: f64,
        alpha: f64,
        is_mini: bool,
    ) {
        if alpha <= 0.01 || radius <= 2.0 {
            return;
        }

        let (ar, ag, ab) = (
            self.accent.0 as f64,
            self.accent.1 as f64,
            self.accent.2 as f64,
        );
        let pulse = 0.5 + 0.5 * (self.elapsed as f64 * 4.2).sin();
        let draw_cx = cx + self.offset_x as f64 * radius;
        let draw_cy = cy + self.offset_y as f64 * radius;

        // 1. Ambient outer aura
        let glow_alpha = match self.state {
            ZekiState::Listening => (0.28 + 0.16 * pulse) * alpha,
            ZekiState::Thinking => (0.24 + 0.16 * pulse) * alpha,
            ZekiState::Speaking => (0.22 + 0.12 * pulse) * alpha,
            ZekiState::Happy => 0.28 * alpha,
            ZekiState::Error => 0.26 * alpha,
            ZekiState::Sleepy => 0.10 * alpha,
            ZekiState::Idle => (0.14 + 0.05 * pulse) * alpha,
        };
        let halo_r = radius * if is_mini { 1.24 } else { 1.38 + 0.06 * pulse };
        fill_ellipse(
            cr,
            draw_cx,
            draw_cy,
            halo_r,
            halo_r * 0.92,
            (ar, ag, ab, glow_alpha),
        );

        if matches!(self.state, ZekiState::Listening | ZekiState::Thinking) && !is_mini {
            let ring_r = radius * (1.22 + 0.18 * pulse);
            let _ = cr.save();
            cr.translate(draw_cx, draw_cy);
            cr.scale(ring_r, ring_r * 0.92);
            cr.arc(0.0, 0.0, 1.0, 0.0, 2.0 * PI64);
            let _ = cr.restore();
            cr.set_source_rgba(ar, ag, ab, (0.45 * (1.0 - pulse * 0.6)) * alpha);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        }

        // Local body transform (translate + rotate + squash/stretch)
        let _ = cr.save();
        cr.translate(draw_cx, draw_cy);
        cr.rotate(self.tilt as f64);
        cr.scale(self.scale_x as f64, self.scale_y as f64);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);

        let rx = radius * 1.06;
        let ry = radius * 0.90;

        // 2. Expressive antenna
        let stem_base_y = -ry * 0.88;
        let stem_len = radius * (0.20 + 0.16 * self.antenna_lift.clamp(-0.2, 1.1) as f64);
        let tip_x = (self.gaze_x as f64 * 0.18 + self.tilt as f64 * 0.25) * radius;
        let tip_y = stem_base_y - stem_len;

        if stem_len > 1.2 {
            cr.set_source_rgba(ar * 0.75 + 0.25, ag * 0.75 + 0.25, ab * 0.75 + 0.25, 0.85 * alpha);
            cr.set_line_width((radius * 0.09).clamp(1.2, 2.4));
            cr.move_to(0.0, stem_base_y);
            cr.line_to(tip_x, tip_y);
            let _ = cr.stroke();

            let orb_r =
                (radius * (0.11 + 0.04 * self.antenna_lift.max(0.0) as f64)).clamp(1.5, 4.2);
            cr.arc(tip_x, tip_y, orb_r * 1.7, 0.0, 2.0 * PI64);
            cr.set_source_rgba(ar, ag, ab, 0.40 * alpha);
            let _ = cr.fill();

            cr.arc(tip_x, tip_y, orb_r, 0.0, 2.0 * PI64);
            cr.set_source_rgba((ar + 0.35).min(1.0), (ag + 0.35).min(1.0), (ab + 0.35).min(1.0), 0.98 * alpha);
            let _ = cr.fill();
        }

        // 3. Capsule-blob body
        rounded_rect(cr, -rx, -ry, rx * 2.0, ry * 2.0, (rx * 0.78).min(ry * 0.86));
        cr.set_source_rgba(0.11 + ar * 0.08, 0.12 + ag * 0.08, 0.17 + ab * 0.10, 0.98 * alpha);
        let _ = cr.fill();

        // Glossy top highlight
        rounded_rect(
            cr,
            -rx * 0.62,
            -ry * 0.82,
            rx * 1.24,
            ry * 0.46,
            (rx * 0.45).min(ry * 0.23),
        );
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.12 * alpha);
        let _ = cr.fill();

        // Inner dark visor plate
        rounded_rect(
            cr,
            -rx * 0.76,
            -ry * 0.54,
            rx * 1.52,
            ry * 1.12,
            (rx * 0.46).min(ry * 0.48),
        );
        cr.set_source_rgba(0.04, 0.05, 0.08, 0.92 * alpha);
        let _ = cr.fill();

        // Mood-tinted rim border
        rounded_rect(cr, -rx, -ry, rx * 2.0, ry * 2.0, (rx * 0.78).min(ry * 0.86));
        cr.set_source_rgba(ar, ag, ab, 0.65 * alpha);
        cr.set_line_width(if is_mini { 1.15 } else { 1.65 });
        let _ = cr.stroke();

        // 4. Rosy blush cheeks
        let face_ox = self.gaze_x as f64 * rx * 0.22;
        let face_oy = self.gaze_y as f64 * ry * 0.22;
        if self.blush > 0.05 && !is_mini {
            for side in [-1.0f64, 1.0f64] {
                fill_ellipse(
                    cr,
                    side * rx * 0.44 + face_ox,
                    ry * 0.22 + face_oy,
                    radius * 0.14,
                    radius * 0.08,
                    (1.0, 0.45, 0.62, 0.38 * self.blush as f64 * alpha),
                );
            }
        }

        // 5. Expressive glowing eyes
        let eye_spacing = rx * if is_mini { 0.34 } else { 0.32 };
        let eye_y = -ry * 0.06 + face_oy;
        let eye_w = radius * if is_mini { 0.28 } else { 0.24 };
        let eye_h = radius * if is_mini { 0.34 } else { 0.30 };

        for side in [-1.0f64, 1.0f64] {
            let ex = side * eye_spacing + face_ox;
            self.draw_eye(cr, ex, eye_y, eye_w, eye_h, side, (ar, ag, ab), alpha, is_mini);
        }

        // 6. Dynamic mouth
        let mouth_y = ry * 0.32 + face_oy * 0.6;
        match self.state {
            ZekiState::Listening => {
                let bars = if is_mini { 3usize } else { 5usize };
                let bar_w = (radius * 0.09).clamp(1.2, 2.8);
                let gap = bar_w * 0.9;
                let total_w = bars as f64 * bar_w + (bars - 1) as f64 * gap;
                let start_x = face_ox - total_w / 2.0;

                cr.set_source_rgba(
                    (ar + 0.25).min(1.0),
                    (ag + 0.25).min(1.0),
                    (ab + 0.25).min(1.0),
                    0.95 * alpha,
                );
                for i in 0..bars {
                    let phase = self.elapsed as f64 * 9.5 + i as f64 * 1.15;
                    let envelope =
                        1.0 - ((i as f64 - (bars as f64 - 1.0) / 2.0).abs() * 0.24);
                    let level = (0.30 + 0.70 * phase.sin().abs())
                        * envelope
                        * (self.voice_level.max(0.5) as f64);
                    let bh = (radius * 0.26 * level).clamp(bar_w, radius * 0.28);
                    let bx = start_x + i as f64 * (bar_w + gap);
                    rounded_rect(cr, bx, mouth_y - bh / 2.0, bar_w, bh, bar_w / 2.0);
                    let _ = cr.fill();
                }
            }
            ZekiState::Speaking => {
                let talk = 0.25 + 0.75 * (self.elapsed as f64 * 13.0).sin().abs();
                let mw = radius * (0.28 + 0.08 * talk);
                let mh = (radius * (0.08 + 0.14 * talk)).max(1.6);
                rounded_rect(cr, face_ox - mw / 2.0, mouth_y - mh / 2.0, mw, mh, mh / 2.0);
                cr.set_source_rgba(
                    (ar + 0.30).min(1.0),
                    (ag + 0.30).min(1.0),
                    (ab + 0.30).min(1.0),
                    0.95 * alpha,
                );
                let _ = cr.fill();
            }
            ZekiState::Happy if !is_mini => {
                cr.set_source_rgba(
                    (ar + 0.35).min(1.0),
                    (ag + 0.35).min(1.0),
                    (ab + 0.35).min(1.0),
                    0.90 * alpha,
                );
                cr.set_line_width((radius * 0.075).clamp(1.3, 2.4));
                cr.arc(
                    face_ox,
                    mouth_y - radius * 0.04,
                    radius * 0.14,
                    0.18 * PI64,
                    0.82 * PI64,
                );
                let _ = cr.stroke();
            }
            _ => {}
        }

        let _ = cr.restore();

        // 7. Ambient floating badges & particles (un-tilted world coordinates)
        match self.state {
            ZekiState::Sleepy => {
                let count = if is_mini { 1 } else { 3 };
                for i in 0..count {
                    let cycle = ((self.elapsed as f64 * 0.42 + i as f64 * 0.33) % 1.0 + 1.0) % 1.0;
                    let fade = (cycle * PI64).sin();
                    let zx =
                        draw_cx + radius * (0.72 + cycle * 0.42) + (cycle * 5.0).sin() * 2.0;
                    let zy = draw_cy - radius * (0.55 + cycle * 0.85);
                    let txt = if i % 2 == 0 { "z" } else { "Z" };
                    draw_small_badge_text(
                        cr,
                        txt,
                        zx - 6.0,
                        zy - 7.0,
                        10.5,
                        (0.78, 0.85, 0.98, 0.88 * fade * alpha),
                    );
                }
            }
            ZekiState::Thinking => {
                let dot_r = (radius * 0.095).clamp(1.3, 3.0);
                let base_x = draw_cx + radius * 0.82;
                let base_y = draw_cy - radius * 0.72;
                for i in 0..3 {
                    let phase = ((self.elapsed as f64 * 2.6 - i as f64 * 0.22) % 1.0 + 1.0) % 1.0;
                    let bounce = (phase * PI64 * 2.0).sin().max(0.0);
                    cr.arc(
                        base_x + (i as f64 - 1.0) * dot_r * 2.7,
                        base_y - bounce * dot_r * 1.4,
                        dot_r * (0.85 + 0.3 * bounce),
                        0.0,
                        2.0 * PI64,
                    );
                    cr.set_source_rgba(
                        (ar + 0.25).min(1.0),
                        (ag + 0.25).min(1.0),
                        (ab + 0.25).min(1.0),
                        (0.55 + 0.45 * bounce) * alpha,
                    );
                    let _ = cr.fill();
                }
            }
            ZekiState::Error if !is_mini => {
                let bx = draw_cx + radius * 0.92;
                let by = draw_cy - radius * 0.76 + (self.elapsed as f64 * 4.0).sin() * 2.0;
                let br = (radius * 0.28).clamp(6.0, 9.5);
                cr.arc(bx, by, br, 0.0, 2.0 * PI64);
                cr.set_source_rgba(ar, ag, ab, 0.95 * alpha);
                let _ = cr.fill();
                draw_small_badge_text(cr, "?", bx - 3.5, by - 7.0, 11.0, (1.0, 1.0, 1.0, alpha));
            }
            _ => {}
        }

        // 8. Celebration sparks
        for spark in &self.sparks {
            let t = (spark.age / spark.life).clamp(0.0, 1.0) as f64;
            let fade = 1.0 - t;
            let sx = draw_cx + spark.x as f64 * radius * 1.3;
            let sy = draw_cy + spark.y as f64 * radius * 1.3;
            let sr = (spark.size as f64 * radius * (0.6 + 0.6 * fade)).clamp(1.2, 4.0);
            cr.arc(sx, sy, sr, 0.0, 2.0 * PI64);
            cr.set_source_rgba(1.0, 0.88, 0.36, fade * alpha);
            let _ = cr.fill();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_eye(
        &self,
        cr: &cairo::Context,
        ex: f64,
        ey: f64,
        ew: f64,
        eh: f64,
        side: f64,
        accent: (f64, f64, f64),
        alpha: f64,
        is_mini: bool,
    ) {
        let (ar, ag, ab) = accent;
        let er = (ar * 0.55 + 0.45).min(1.0);
        let eg = (ag * 0.55 + 0.45).min(1.0);
        let eb = (ab * 0.55 + 0.45).min(1.0);

        if !is_mini {
            fill_ellipse(cr, ex, ey, ew * 0.95, eh * 0.85, (ar, ag, ab, 0.28 * alpha));
        }

        cr.set_source_rgba(er, eg, eb, 0.98 * alpha);

        match self.state {
            ZekiState::Sleepy => {
                cr.set_line_width((ew * 0.28).clamp(1.2, 2.6));
                cr.arc(ex, ey - eh * 0.08, ew * 0.50, 0.16 * PI64, 0.84 * PI64);
                let _ = cr.stroke();
            }
            ZekiState::Happy => {
                cr.set_line_width((ew * 0.34).clamp(1.4, 3.0));
                cr.arc(ex, ey + eh * 0.26, ew * 0.50, 1.16 * PI64, 1.84 * PI64);
                let _ = cr.stroke();
            }
            ZekiState::Error => {
                let stroke = (ew * 0.22).clamp(1.1, 2.2);
                cr.set_line_width(stroke);
                if is_mini {
                    let r = ew * 0.42;
                    cr.move_to(ex - r, ey - r);
                    cr.line_to(ex + r, ey + r);
                    cr.move_to(ex + r, ey - r);
                    cr.line_to(ex - r, ey + r);
                    let _ = cr.stroke();
                } else {
                    let steps = 22usize;
                    for i in 0..=steps {
                        let t = i as f64 / steps as f64;
                        let angle = t * 3.8 * PI64 + self.elapsed as f64 * 6.5 * side;
                        let rad = ew * (0.08 + 0.50 * t);
                        let px = ex + angle.cos() * rad;
                        let py = ey + angle.sin() * rad;
                        if i == 0 {
                            cr.move_to(px, py);
                        } else {
                            cr.line_to(px, py);
                        }
                    }
                    let _ = cr.stroke();
                }
            }
            ZekiState::Listening => {
                let w = ew * 1.12;
                let h = (eh * 1.14 * self.blink_open as f64).max(w * 0.32);
                let r = (w / 2.0).min(h / 2.0);
                rounded_rect(cr, ex - w / 2.0, ey - h / 2.0, w, h, r);
                let _ = cr.fill();
                if !is_mini && self.blink_open > 0.6 {
                    draw_eye_glint(cr, ex, ey, w, h, alpha);
                }
            }
            ZekiState::Idle | ZekiState::Thinking | ZekiState::Speaking => {
                let w = ew;
                let h = (eh * self.blink_open as f64).max(w * 0.30);
                let r = (w / 2.0).min(h / 2.0);
                rounded_rect(cr, ex - w / 2.0, ey - h / 2.0, w, h, r);
                let _ = cr.fill();
                if !is_mini && self.blink_open > 0.6 {
                    draw_eye_glint(cr, ex, ey, w, h, alpha);
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

fn draw_eye_glint(cr: &cairo::Context, ex: f64, ey: f64, w: f64, h: f64, alpha: f64) {
    let gr = (w * 0.22).max(1.1);
    cr.arc(ex + w * 0.16, ey - h * 0.20, gr, 0.0, 2.0 * PI64);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.85 * alpha);
    let _ = cr.fill();
}

fn fill_ellipse(
    cr: &cairo::Context,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    rgba: (f64, f64, f64, f64),
) {
    if rx <= 0.1 || ry <= 0.1 {
        return;
    }
    let _ = cr.save();
    cr.translate(cx, cy);
    cr.scale(rx, ry);
    cr.arc(0.0, 0.0, 1.0, 0.0, 2.0 * PI64);
    let _ = cr.restore();
    cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
    let _ = cr.fill();
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI64);
    cr.arc(x + r, y + r, r, PI64, 3.0 * FRAC_PI_2);
    cr.close_path();
}

fn draw_small_badge_text(
    cr: &cairo::Context,
    text: &str,
    x: f64,
    y: f64,
    size_px: f64,
    rgba: (f64, f64, f64, f64),
) {
    let layout = pangocairo::functions::create_layout(cr);
    let mut desc = pango::FontDescription::from_string("Sans Bold");
    desc.set_absolute_size(size_px * pango::SCALE as f64);
    layout.set_font_description(Some(&desc));
    layout.set_text(text);
    cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, &layout);
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
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
}
