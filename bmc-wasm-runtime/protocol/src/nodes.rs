// Copyright (C) 2026  Braiins Systems s.r.o.

//! Node type and draw command constants for tree serialization.

// Node types (0x00–0x3F)
pub const NODE_COLUMN: u8 = 0x00;
pub const NODE_ROW: u8 = 0x01;
pub const NODE_CENTER: u8 = 0x02;
pub const NODE_PARAGRAPH: u8 = 0x03;
pub const NODE_BUTTON: u8 = 0x04;
pub const NODE_SPACER: u8 = 0x05;
pub const NODE_CANVAS: u8 = 0x06;
pub const NODE_MODAL: u8 = 0x07;
pub const NODE_NOTIFICATION: u8 = 0x08;
pub const NODE_SCROLL: u8 = 0x09;
pub const NODE_PROGRESS_BAR: u8 = 0x0A;

// Button size variants (wire value for NODE_BUTTON)
pub const BUTTON_SIZE_SMALL: u8 = 0;
pub const BUTTON_SIZE_NORMAL: u8 = 1;
pub const BUTTON_SIZE_LARGE: u8 = 2;

/// Button style variants — shared between SDK and host renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ButtonStyle {
    Primary = 0,
    Secondary = 1,
    Danger = 2,
    Tertiary = 3,
    /// Transparent background, no border. Pressed state shows a subtle rectangular fill.
    Ghost = 4,
}

impl From<u32> for ButtonStyle {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::Secondary,
            2 => Self::Danger,
            3 => Self::Tertiary,
            4 => Self::Ghost,
            _ => Self::Primary,
        }
    }
}

/// Button size variants — shared between SDK and host renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ButtonSize {
    Small = 0,
    Normal = 1,
    Large = 2,
}

impl From<u8> for ButtonSize {
    fn from(value: u8) -> Self {
        match value {
            BUTTON_SIZE_SMALL => Self::Small,
            BUTTON_SIZE_LARGE => Self::Large,
            _ => Self::Normal,
        }
    }
}

impl ButtonStyle {
    #[must_use]
    pub fn is_outline(self) -> bool {
        matches!(self, Self::Tertiary)
    }

    #[must_use]
    pub fn is_ghost(self) -> bool {
        matches!(self, Self::Ghost)
    }
}

impl ButtonSize {
    #[must_use]
    pub fn height(self) -> f32 {
        match self {
            Self::Small => 32.0,
            Self::Normal => 48.0,
            Self::Large => 56.0,
        }
    }

    #[must_use]
    pub fn font_size(self) -> f32 {
        match self {
            Self::Small => 13.0,
            Self::Normal => 16.0,
            Self::Large => 18.0,
        }
    }

    #[must_use]
    pub fn icon_size(self) -> f32 {
        match self {
            Self::Small => 14.0,
            Self::Normal => 16.0,
            Self::Large => 20.0,
        }
    }

    #[must_use]
    pub fn h_padding(self) -> f32 {
        match self {
            Self::Small => 12.0,
            Self::Normal => 16.0,
            Self::Large => 20.0,
        }
    }

    #[must_use]
    pub fn icon_text_gap(self) -> f32 {
        match self {
            Self::Small => 6.0,
            Self::Normal => 8.0,
            Self::Large => 10.0,
        }
    }
}

// Draw commands — shapes (0x40–0x5F)
pub const DRAW_RECT: u8 = 0x40;
pub const DRAW_CIRCLE: u8 = 0x41;
pub const DRAW_ICON: u8 = 0x42;
pub const DRAW_BITMAP: u8 = 0x43;
pub const DRAW_PATH: u8 = 0x44;
pub const DRAW_SPHERE: u8 = 0x45;
pub const DRAW_TEXT: u8 = 0x46;
pub const DRAW_MESH: u8 = 0x47;
pub const DRAW_NINE_PATCH: u8 = 0x48;

// Draw commands — transforms (0x60–0x7F)
pub const DRAW_CENTERED: u8 = 0x60;
pub const DRAW_ORBIT: u8 = 0x61;
pub const DRAW_ROTATED: u8 = 0x62;

// Draw commands — modifiers (0x80–0x9F)
pub const DRAW_MODIFIED: u8 = 0x80;
