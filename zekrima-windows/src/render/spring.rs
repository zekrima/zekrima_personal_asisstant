//! Damped harmonic spring physics used by the island geometry.

/// Collapsed geometry (logical pixels).
pub const COLLAPSED_WIDTH: f32 = 178.0;
pub const COLLAPSED_BADGE_WIDTH: f32 = 232.0;
pub const COLLAPSED_HEIGHT: f32 = 34.0;
pub const COLLAPSED_RADIUS: f32 = 17.0;

/// Active assistant pill geometry (collapsed Listening / Thinking / Speaking / Error).
pub const ASSISTANT_WIDTH: f32 = 224.0;
pub const ASSISTANT_HEIGHT: f32 = 38.0;
pub const ASSISTANT_RADIUS: f32 = 19.0;

/// HUD geometry (logical pixels) - shown for volume/brightness overlay.
pub const HUD_WIDTH: f32 = 300.0;
pub const HUD_HEIGHT: f32 = 52.0;
pub const HUD_RADIUS: f32 = 26.0;

/// Incoming notification toast geometry (logical pixels) - Pillar-style card.
pub const NOTIFICATION_WIDTH: f32 = 380.0;
pub const NOTIFICATION_HEIGHT: f32 = 88.0;
pub const NOTIFICATION_RADIUS: f32 = 26.0;

/// Unified expanded geometry across all 6 tabs (includes persistent Zeki Companion Dock).
pub const EXPANDED_WIDTH: f32 = 440.0;
pub const EXPANDED_HEIGHT: f32 = 198.0;
pub const EXPANDED_RADIUS: f32 = 28.0;

/// Expanded chat geometry (normal comfortable chat box).
pub const EXPANDED_CHAT_WIDTH: f32 = 520.0;
pub const EXPANDED_CHAT_HEIGHT: f32 = 310.0;

/// Large expanded chat geometry (toggled by double-clicking the chat box or clicking Expand).
pub const EXPANDED_CHAT_LARGE_WIDTH: f32 = 640.0;
pub const EXPANDED_CHAT_LARGE_HEIGHT: f32 = 470.0;

const REST_DISTANCE: f32 = 0.05;
const REST_VELOCITY: f32 = 0.05;

#[derive(Debug, Clone, Copy)]
pub struct Spring {
    current: f32,
    target: f32,
    velocity: f32,
    stiffness: f32,
    damping: f32,
}

impl Spring {
    pub fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            velocity: 0.0,
            stiffness: 280.0,
            damping: 26.0,
        }
    }

    pub fn set_target(&mut self, target: f32) -> bool {
        if self.target == target {
            false
        } else {
            self.target = target;
            true
        }
    }

    pub fn update(&mut self, dt: f32) -> bool {
        let force = (self.target - self.current) * self.stiffness;
        let damping_force = self.velocity * self.damping;

        self.velocity += (force - damping_force) * dt;
        self.current += self.velocity * dt;

        if (self.target - self.current).abs() < REST_DISTANCE
            && self.velocity.abs() < REST_VELOCITY
        {
            self.current = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }

    pub fn current(&self) -> f32 {
        self.current
    }

    pub fn is_settled(&self) -> bool {
        self.current == self.target && self.velocity == 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillMode {
    Collapsed,
    CollapsedWithBadge,
    Assistant,
    Hud,
    Notification,
    Expanded,
    ExpandedChat,
    ExpandedChatLarge,
}

#[derive(Debug, Clone, Copy)]
pub struct PillSprings {
    pub width: Spring,
    pub height: Spring,
    pub radius: Spring,
}

impl PillSprings {
    pub fn new() -> Self {
        Self {
            width: Spring::new(COLLAPSED_WIDTH),
            height: Spring::new(COLLAPSED_HEIGHT),
            radius: Spring::new(COLLAPSED_RADIUS),
        }
    }

    pub fn set_mode(&mut self, mode: PillMode) -> bool {
        let (w, h, r) = match mode {
            PillMode::Collapsed => (COLLAPSED_WIDTH, COLLAPSED_HEIGHT, COLLAPSED_RADIUS),
            PillMode::CollapsedWithBadge => {
                (COLLAPSED_BADGE_WIDTH, COLLAPSED_HEIGHT, COLLAPSED_RADIUS)
            }
            PillMode::Assistant => (ASSISTANT_WIDTH, ASSISTANT_HEIGHT, ASSISTANT_RADIUS),
            PillMode::Hud => (HUD_WIDTH, HUD_HEIGHT, HUD_RADIUS),
            PillMode::Notification => (NOTIFICATION_WIDTH, NOTIFICATION_HEIGHT, NOTIFICATION_RADIUS),
            PillMode::Expanded => (EXPANDED_WIDTH, EXPANDED_HEIGHT, EXPANDED_RADIUS),
            PillMode::ExpandedChat => {
                (EXPANDED_CHAT_WIDTH, EXPANDED_CHAT_HEIGHT, EXPANDED_RADIUS)
            }
            PillMode::ExpandedChatLarge => (
                EXPANDED_CHAT_LARGE_WIDTH,
                EXPANDED_CHAT_LARGE_HEIGHT,
                EXPANDED_RADIUS,
            ),
        };
        let width_changed = self.width.set_target(w);
        let height_changed = self.height.set_target(h);
        let radius_changed = self.radius.set_target(r);
        width_changed || height_changed || radius_changed
    }

    pub fn update(&mut self, dt: f32) -> bool {
        let width_moving = self.width.update(dt);
        let height_moving = self.height.update(dt);
        let radius_moving = self.radius.update(dt);
        width_moving || height_moving || radius_moving
    }

    pub fn geometry(&self) -> (f32, f32, f32) {
        (
            self.width.current(),
            self.height.current(),
            self.radius.current(),
        )
    }

    pub fn is_settled(&self) -> bool {
        self.width.is_settled() && self.height.is_settled() && self.radius.is_settled()
    }
}

impl Default for PillSprings {
    fn default() -> Self {
        Self::new()
    }
}
