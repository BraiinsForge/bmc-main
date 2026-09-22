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

//! Fragments every arrangement draws from: text and spacers,
//! the round buttons, the QR code and the close target.

use super::{
    CIRCLE_FILL, CIRCLE_PRESSED, CLOSE_GLYPH, CLOSE_KEY, CLOSE_TARGET, HOLD_ALPHA_FULL_AT,
    HOLD_FILL, ICON_PRESSED_TINT, Panel, Tier, WIDE_QR_QUIET_ZONE, WifiIcons,
};
use bmc_platform::DisplayShape;
use bmc_render::tree::{DrawCommand, PropsData, TextStyle, TreeNode, col, row, text};
use bmc_wasm_protocol::colors::{BLACK, TRANSPARENT, WHITE};
use bmc_wasm_protocol::{Color, CrossAlign, Fill, FontWeight, SvgId, TextAlign, TextOverflow};

pub(super) fn text_style(size: u32, color: Color) -> TextStyle {
    TextStyle {
        size,
        color,
        ..TextStyle::default()
    }
}

/// A runtime string on one line, ending in "…" past `max_width` px.
pub(super) fn capped_text(s: &str, size: u32, max_width: u32) -> TreeNode {
    text(
        s,
        TextStyle {
            max_width,
            text_overflow: TextOverflow::Ellipsis,
            ..text_style(size, WHITE)
        },
    )
}

pub(super) fn fixed_width(width: f32) -> TreeNode {
    row(
        PropsData {
            width,
            ..PropsData::default()
        },
        Vec::new(),
    )
}

/// Wrap `node` with equal left/right padding by flanking it with fixed-width
/// spacers; the content takes the remaining width.
pub(super) fn pad_horizontal(node: TreeNode, padding: f32) -> TreeNode {
    row(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![
            fixed_width(padding),
            col(
                PropsData {
                    flex: 1.0,
                    ..PropsData::default()
                },
                vec![node],
            ),
            fixed_width(padding),
        ],
    )
}

/// Centered header row, on one line that ends in "…" if its column is too narrow.
/// The centering column is what actually centers the text node; a bare paragraph
/// with `align: Center` does not center under a stretching parent.
pub(super) fn header_row(header: &str, size: u32) -> TreeNode {
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![text(
            header,
            TextStyle {
                size,
                weight: FontWeight::BOLD,
                color: WHITE,
                align: TextAlign::Center,
                text_overflow: TextOverflow::Ellipsis,
                ..TextStyle::default()
            },
        )],
    )
}

pub(super) fn wifi_icon(icons: WifiIcons, wifi_signal: Option<i32>, size: f32) -> TreeNode {
    TreeNode::Canvas {
        props: PropsData {
            width: size,
            height: size,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Svg {
            x: 0.0,
            y: 0.0,
            w: size,
            h: size,
            color: TRANSPARENT,
            icon_id: icons.for_signal(wifi_signal),
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

/// An icon inside a round button. `aspect` is width/height — 1.0 for the
/// square control glyphs; nightmode.svg is 49×48 and keeps its ratio
/// instead of stretching.
#[derive(Debug, Clone, Copy)]
pub(super) struct ButtonIcon {
    pub(super) id: Option<SvgId>,
    pub(super) aspect: f32,
}

impl ButtonIcon {
    pub(super) fn square(id: Option<SvgId>) -> Self {
        Self { id, aspect: 1.0 }
    }
}

/// One round icon button with a centered icon. While held, a larger circle
/// fades in behind it and shrinks to the button radius; the button becomes
/// opaque with a black icon so the progress circle cannot show through it.
/// `icon_tint`
/// `TRANSPARENT` keeps the SVG's native fill (white for controls); an opaque
/// tint colorizes the whole icon while the button is not held.
pub(super) fn round_button(
    key: &str,
    icon: ButtonIcon,
    diameter: f32,
    icon_size: f32,
    fill: Color,
    icon_tint: Color,
    hold_progress: Option<f32>,
) -> TreeNode {
    let c = diameter / 2.0;
    debug_assert!(
        hold_progress.is_none_or(|p| (0.0..=1.0).contains(&p)),
        "BUG: hold progress must be a 0..=1 fraction, got {hold_progress:?}",
    );
    let hold_progress = hold_progress.filter(|p| *p > 0.0);
    let held = hold_progress.is_some();
    let canvas_size = if held { diameter * 2.0 } else { diameter };
    let center = canvas_size / 2.0;
    let mut draws = Vec::with_capacity(if held { 3 } else { 2 });
    if let Some(p) = hold_progress {
        let hold_radius = c * (2.0 - p);
        let hold_alpha = (p / HOLD_ALPHA_FULL_AT).min(1.0);
        draws.push(DrawCommand::Circle {
            cx: center,
            cy: center,
            r: hold_radius,
            fill: Fill::Solid(HOLD_FILL.scale_alpha(hold_alpha)),
        });
    }
    draws.push(DrawCommand::Circle {
        cx: center,
        cy: center,
        r: c,
        fill: Fill::Solid(if held { fill.with_alpha(1.0) } else { fill }),
    });
    let (icon_w, icon_h) = (icon_size * icon.aspect, icon_size);
    draws.push(DrawCommand::Svg {
        x: center - icon_w / 2.0,
        y: center - icon_h / 2.0,
        w: icon_w,
        h: icon_h,
        color: if held { ICON_PRESSED_TINT } else { icon_tint },
        icon_id: icon.id,
        anti_alias: true,
        fills: Vec::new(),
    });
    let visual_props = PropsData {
        width: canvas_size,
        height: canvas_size,
        ..PropsData::default()
    };
    if !held {
        return TreeNode::Canvas {
            props: visual_props,
            touch_key: Some(key.to_owned()),
            draws,
        };
    }

    let visual = TreeNode::Canvas {
        props: PropsData {
            inset_top: -c,
            inset_left: -c,
            ..visual_props
        },
        touch_key: None,
        draws,
    };
    // Keep the touch key on the button-sized canvas; putting it on the
    // oversized visual canvas would expand the interactive area.
    let touch_target = TreeNode::Canvas {
        props: PropsData {
            width: diameter,
            height: diameter,
            ..PropsData::default()
        },
        touch_key: Some(key.to_owned()),
        draws: Vec::new(),
    };
    col(
        PropsData {
            width: diameter,
            height: diameter,
            ..PropsData::default()
        },
        vec![visual, touch_target],
    )
}

pub(super) fn press_fill(pressed: bool) -> Color {
    if pressed { CIRCLE_PRESSED } else { CIRCLE_FILL }
}

pub(super) fn press_tint(pressed: bool) -> Color {
    if pressed {
        ICON_PRESSED_TINT
    } else {
        TRANSPARENT
    }
}

/// Top-left corner of the absolutely positioned close target, in panel
/// coordinates. Shared by `close_button` and the hit-disjointness test:
/// top-right padded corner on rectangular panels; on round panels centered
/// on the 45° point of the disc inset by 56px, which is chord-safe (farthest
/// corner ≈218px < R = 240).
pub(super) fn close_origin(panel: &Panel, tier: Tier) -> (f32, f32) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "panel sizes are far below f32 mantissa precision"
    )]
    let w = panel.width as f32;
    match panel.shape {
        DisplayShape::Rectangular => (w - tier.padding - CLOSE_TARGET, tier.padding),
        DisplayShape::Round => {
            let r = w / 2.0;
            let d = (r - 56.0) * std::f32::consts::FRAC_1_SQRT_2;
            (r + d - CLOSE_TARGET / 2.0, r - d - CLOSE_TARGET / 2.0)
        }
    }
}

/// The 48×48 close target with its 24×24 gray glyph, absolutely positioned
/// via `PropsData` insets (finite inset = absolute positioning).
pub(super) fn close_button(panel: &Panel, tier: Tier, icon: Option<SvgId>) -> TreeNode {
    let (left, top) = close_origin(panel, tier);
    let glyph_inset = (CLOSE_TARGET - CLOSE_GLYPH) / 2.0;
    TreeNode::Canvas {
        props: PropsData {
            width: CLOSE_TARGET,
            height: CLOSE_TARGET,
            inset_top: top,
            inset_left: left,
            ..PropsData::default()
        },
        touch_key: Some(CLOSE_KEY.to_owned()),
        draws: vec![DrawCommand::Svg {
            x: glyph_inset,
            y: glyph_inset,
            w: CLOSE_GLYPH,
            h: CLOSE_GLYPH,
            color: TRANSPARENT,
            icon_id: icon,
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

/// The QR code for the device's web UI, so scanning it opens the address
/// printed beside it. Callers omit it while the IP is unknown, since a
/// placeholder would scan as a dead link.
pub(super) fn ip_qr(ip: &str, size: f32) -> TreeNode {
    TreeNode::Canvas {
        props: PropsData {
            width: size,
            height: size,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Qr {
            x: 0.0,
            y: 0.0,
            size,
            dark: BLACK,
            light: WHITE,
            quiet_zone: WIDE_QR_QUIET_ZONE,
            text: format!("http://{ip}"),
        }],
    }
}
