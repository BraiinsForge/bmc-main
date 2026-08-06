// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Node type and draw command constants for tree serialization.

use core::fmt;

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
pub const NODE_RELTIME: u8 = 0x0B;
pub const NODE_TAG: u8 = 0x0C;
pub const NODE_SWITCHER: u8 = 0x0D;
pub const NODE_SKELETON: u8 = 0x0E;

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

/// Tag severity kind (wire value after `NODE_TAG`). The host maps this to a
/// Carbon color theme + default icon; see `bmc-render`'s `tag_theme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TagKind {
    Info = 0,
    Warning = 1,
    Error = 2,
}

/// Invalid [`TagKind`] wire discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTagKind(pub u8);

impl fmt::Display for InvalidTagKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid TagKind wire discriminant: {}", self.0)
    }
}

impl std::error::Error for InvalidTagKind {}

impl TryFrom<u8> for TagKind {
    type Error = InvalidTagKind;

    // `Self::Error` would be ambiguous with the `Error` variant, so name the
    // concrete error type here and reach the variants through `TagKind`.
    fn try_from(value: u8) -> Result<Self, InvalidTagKind> {
        match value {
            0 => Ok(TagKind::Info),
            1 => Ok(TagKind::Warning),
            2 => Ok(TagKind::Error),
            other => Err(InvalidTagKind(other)),
        }
    }
}

/// Tag icon mode (wire byte after the kind). `Custom` is followed by an `SvgId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TagIconMode {
    Default = 0, // per-kind theme icon
    Hidden = 1,  // no icon
    Custom = 2,  // explicit SvgId follows
}

/// Invalid [`TagIconMode`] wire discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTagIconMode(pub u8);

impl fmt::Display for InvalidTagIconMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid TagIconMode wire discriminant: {}", self.0)
    }
}

impl std::error::Error for InvalidTagIconMode {}

impl TryFrom<u8> for TagIconMode {
    type Error = InvalidTagIconMode;

    fn try_from(value: u8) -> Result<Self, InvalidTagIconMode> {
        match value {
            0 => Ok(TagIconMode::Default),
            1 => Ok(TagIconMode::Hidden),
            2 => Ok(TagIconMode::Custom),
            other => Err(InvalidTagIconMode(other)),
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
pub const DRAW_CURVED_TEXT: u8 = 0x49;
pub const DRAW_ARC: u8 = 0x4A;
/// Autofit text: scale text to fit an explicit box. Shapes block (0x40–0x5F).
pub const DRAW_AUTOFIT_TEXT: u8 = 0x4B;
/// QR code: a module matrix the host rasterises. Shapes block (0x40–0x5F).
pub const DRAW_QR: u8 = 0x4C;

// Draw commands — transforms (0x60–0x7F)
pub const DRAW_CENTERED: u8 = 0x60;
pub const DRAW_ORBIT: u8 = 0x61;
pub const DRAW_ROTATED: u8 = 0x62;
pub const DRAW_SHADOW: u8 = 0x63;

/// Hard cap on Gaussian-blur sigma for a `DRAW_SHADOW`.
/// Clamped against by both the SDK (on encode)
/// and the host renderer (on decode).
pub const DROP_SHADOW_BLUR_MAX: f32 = 16.0;

// Draw commands — modifiers (0x80–0x9F)
pub const DRAW_MODIFIED: u8 = 0x80;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn draw_opcode_values_are_unique() {
        let opcodes = [
            DRAW_RECT,
            DRAW_CIRCLE,
            DRAW_ICON,
            DRAW_BITMAP,
            DRAW_PATH,
            DRAW_SPHERE,
            DRAW_TEXT,
            DRAW_MESH,
            DRAW_NINE_PATCH,
            DRAW_CURVED_TEXT,
            DRAW_ARC,
            DRAW_AUTOFIT_TEXT,
            DRAW_QR,
            DRAW_CENTERED,
            DRAW_ORBIT,
            DRAW_ROTATED,
            DRAW_SHADOW,
            DRAW_MODIFIED,
        ];
        let unique = opcodes.iter().copied().collect::<HashSet<_>>();
        assert_eq!(unique.len(), opcodes.len());
    }
}
