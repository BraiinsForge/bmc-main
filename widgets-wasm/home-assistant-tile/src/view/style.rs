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

//! What the blocks share: their colours, their measures and the icon handle.

use bmc_wasm_sdk::*;

pub const CONTROL_HEIGHT: f32 = 64.0;
pub const CONTROL_RADIUS: f32 = 16.0;
pub const LABEL_COLOR: Color = WHITE;
/// A value Home Assistant has yet to confirm is drawn fainter.
pub const PENDING_LABEL_COLOR: Color = GRAY_40;
pub const TRACK_COLOR: Color = GRAY_90;
/// What a block with nothing to do is drawn at, as Home Assistant fades a disabled control:
/// a white label comes out as `GRAY_60`.
const DISABLED_BRIGHTNESS: f32 = 0.435;
pub const ICON_SIZE: f32 = 32.0;
pub const ROW_GAP: f32 = 12.0;
/// Below this width a row of blocks tightens: narrower buttons, smaller gaps.
const COMPACT_WIDTH: f32 = 360.0;
const COMPACT_BUTTON: f32 = 56.0;
const COMPACT_GAP: f32 = 8.0;

/// A colour of the Home Assistant frontend's default theme, which a state wears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    /// The frontend's `--state-icon-color`, for a domain it gives no state colour of its own.
    Neutral,
    Amber,
    DeepOrange,
    Orange,
    Blue,
    Cyan,
    Green,
    Purple,
    Red,
}

impl Accent {
    #[must_use]
    pub fn color(self) -> Color {
        match self {
            Self::Neutral => Color::from_hex(0x44_73_9E),
            Self::Amber => Color::from_hex(0xFF_C1_07),
            Self::DeepOrange => Color::from_hex(0xFF_6F_22),
            Self::Orange => Color::from_hex(0xFF_98_00),
            Self::Blue => Color::from_hex(0x21_96_F3),
            Self::Cyan => Color::from_hex(0x00_BC_D4),
            Self::Green => Color::from_hex(0x4C_AF_50),
            Self::Purple => Color::from_hex(0x92_6B_C7),
            Self::Red => Color::from_hex(0xF4_43_36),
        }
    }
}

/// An icon packaged with `include_svg!`.
#[derive(Debug, Clone, Copy)]
pub struct Icon(pub &'static Svg);

/// Two handles to the same asset are one icon, wherever the asset was included.
impl PartialEq for Icon {
    fn eq(&self, other: &Self) -> bool {
        self.0.name == other.0.name
    }
}

impl Eq for Icon {}

#[must_use]
pub fn dimmed_unless(enabled: bool, node: Node) -> Node {
    if enabled {
        node
    } else {
        dimmed(DISABLED_BRIGHTNESS, node)
    }
}

/// Props that stretch a node over its parent.
#[must_use]
pub fn fill_insets() -> PropsData {
    props!(inset_top: 0.0, inset_right: 0.0, inset_bottom: 0.0, inset_left: 0.0)
}

/// The width of a button beside others, and the gap between them, in a row `width` wide.
#[must_use]
pub fn row_spacing(width: f32) -> (f32, f32) {
    if width < COMPACT_WIDTH {
        (COMPACT_BUTTON, COMPACT_GAP)
    } else {
        (CONTROL_HEIGHT, ROW_GAP)
    }
}

/// `text` shrunk to fit a `width` by `height` box centred in a block `outer_height` high,
/// wrapping where the box has room for more than one line.
#[must_use]
pub fn fitted_text(
    text: &str,
    (width, height): (f32, f32),
    outer_height: f32,
    size: u32,
    color: Color,
) -> Node {
    center(
        props!(flex: 1.0),
        [canvas(
            props!(width: width, height: outer_height),
            [Draw::autofit_text(
                0.0,
                (outer_height - height) / 2.0,
                width,
                height,
                text.to_owned(),
                style!(
                    size: size,
                    weight: FontWeight::BOLD,
                    color: color,
                    align: TextAlign::Center,
                    valign: VerticalAlign::Center
                ),
            )],
        )],
    )
}
