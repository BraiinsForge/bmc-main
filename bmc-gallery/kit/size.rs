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

//! The Deck's preview frame sizes — the device form factors a widget is staged in.

/// The Deck's display.
pub const DEVICE_WIDTH: usize = 1_280;
pub const DEVICE_HEIGHT: usize = 480;

/// Ceiling for a content-driven frame's target, so a runaway layout can't ask
/// for an unbounded texture.
pub const AUTO_HEIGHT_MAX: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivHeight {
    Px(usize),
    /// Content-driven: the frame is as tall as the tree lays out to.
    Auto,
}

impl From<usize> for DivHeight {
    fn from(v: usize) -> Self {
        Self::Px(v)
    }
}

/// Preset frame sizes for [`node_stage`](crate::kit::DeckSceneCtx::node_stage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckSize {
    /// Device display dimensions (1280x480).
    Full,
    /// Device width, height driven by the content — a page of widgets, which
    /// is what most scenes stage.
    Page,
    /// 480x320.
    Large,
    /// 320x240.
    Medium,
    /// 160x120.
    Small,
    /// Fully content-driven (both width and height from layout).
    Auto,
    /// Circular face inscribed in a `diameter × diameter` frame.
    Round(usize),
    /// Arbitrary dimensions.
    Custom(usize, DivHeight),
}

impl DeckSize {
    /// Frame width in pixels; a content-driven one is staged at the device's.
    #[must_use]
    pub fn width(self) -> usize {
        match self {
            Self::Large => 480,
            Self::Medium => 320,
            Self::Small => 160,
            Self::Full | Self::Page | Self::Auto => DEVICE_WIDTH,
            Self::Round(d) => d,
            Self::Custom(w, _) => w,
        }
    }

    #[must_use]
    pub fn div_height(self) -> DivHeight {
        match self {
            Self::Full => DivHeight::Px(DEVICE_HEIGHT),
            Self::Large => DivHeight::Px(320),
            Self::Medium => DivHeight::Px(240),
            Self::Small => DivHeight::Px(120),
            Self::Page | Self::Auto => DivHeight::Auto,
            Self::Round(d) => DivHeight::Px(d),
            Self::Custom(_, h) => h,
        }
    }

    #[must_use]
    pub fn is_auto_width(self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Whether the frame is a circular face — the render is masked to the
    /// inscribed circle.
    #[must_use]
    pub fn is_round(self) -> bool {
        matches!(self, Self::Round(_))
    }

    /// Layout width passed to `process_tree` (Auto → 0 for content-driven).
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a device width is a few hundred pixels"
    )]
    pub fn layout_width(self) -> f32 {
        if self.is_auto_width() {
            0.0
        } else {
            self.width() as f32
        }
    }

    /// Layout height passed to `process_tree` (Auto → 0 for content-driven).
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a device height is a few hundred pixels"
    )]
    pub fn layout_height(self) -> f32 {
        match self.div_height() {
            DivHeight::Px(h) => h as f32,
            DivHeight::Auto => 0.0,
        }
    }
}

impl From<(u32, u32)> for DeckSize {
    fn from((w, h): (u32, u32)) -> Self {
        Self::Custom(w as usize, DivHeight::Px(h as usize))
    }
}

impl From<(f32, f32)> for DeckSize {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a design frame is a whole positive number of pixels"
    )]
    fn from((w, h): (f32, f32)) -> Self {
        Self::Custom(w as usize, DivHeight::Px(h as usize))
    }
}

impl From<(usize, DivHeight)> for DeckSize {
    fn from((w, h): (usize, DivHeight)) -> Self {
        Self::Custom(w, h)
    }
}
