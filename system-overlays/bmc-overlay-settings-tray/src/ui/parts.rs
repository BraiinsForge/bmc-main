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
use bmc_render::tree::{DrawCommand, PropsData, TextStyle, TreeNode, col, row, text};
use bmc_system_overlay::ViewportShape;
use bmc_wasm_protocol::colors::{BLACK, GREEN_50, TRANSPARENT, WHITE};
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

/// The SETUP badge before a setup AP's SSID: what tells the user this is
/// a network to join from a phone, not the one the device is on.
pub(super) fn setup_badge(size: u32) -> TreeNode {
    text(
        "SETUP",
        TextStyle {
            size,
            weight: FontWeight::BOLD,
            color: GREEN_50,
            ..TextStyle::default()
        },
    )
}

pub(super) fn wifi_icon(icons: WifiIcons, wifi_signal: Option<i32>, size: f32) -> TreeNode {
    svg_icon(icons.for_signal(wifi_signal), size, TRANSPARENT)
}

/// A square icon. A `TRANSPARENT` tint keeps the artwork's own colours.
pub(super) fn svg_icon(icon_id: Option<SvgId>, size: f32, tint: Color) -> TreeNode {
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
            color: tint,
            icon_id,
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

/// An icon inside a round button. `aspect` is width/height — 1.0
/// for the square control glyphs; nightmode.svg is 49×48 and keeps its ratio
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
/// An `icon_tint` of `TRANSPARENT` keeps the SVG's native fill (white for controls);
/// an opaque tint colorizes the whole icon while the button is not held.
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
    // Keep the touch key on the button-sized canvas;
    // putting it on the oversized visual canvas would expand the interactive area.
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
        ViewportShape::Rectangular => (w - tier.padding - CLOSE_TARGET, tier.padding),
        ViewportShape::Round => {
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
/// printed beside it. Callers omit it while the IP is unknown,
/// since a placeholder would scan as a dead link.
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

#[cfg(test)]
mod tests {
    use bmc_render::tree::{DrawCommand, TreeNode};
    use bmc_wasm_protocol::Fill;
    use bmc_wasm_protocol::colors::TRANSPARENT;

    use super::*;
    use crate::ui::test_support::*;
    use crate::ui::*;

    #[test]
    fn hold_circle_is_drawn_behind_the_button() {
        let btn = round_button(
            "k",
            ButtonIcon::square(None),
            112.0,
            48.0,
            CIRCLE_FILL,
            TRANSPARENT,
            Some(0.5),
        );
        let TreeNode::Column(_, children) = btn else {
            panic!("expected layered hold button")
        };
        let [
            TreeNode::Canvas { draws, .. },
            TreeNode::Canvas {
                draws: touch_draws,
                touch_key,
                ..
            },
        ] = children.as_slice()
        else {
            panic!("expected visual and touch canvases")
        };
        assert_eq!(touch_key.as_deref(), Some("k"));
        assert!(touch_draws.is_empty());
        let DrawCommand::Circle {
            fill: Fill::Solid(hold_fill),
            r: hold_radius,
            ..
        } = &draws[0]
        else {
            panic!("expected hold Circle")
        };
        let DrawCommand::Circle {
            fill: Fill::Solid(circle),
            r,
            ..
        } = &draws[1]
        else {
            panic!("expected button Circle")
        };
        assert_close(*hold_radius, 84.0, "half hold shrinks the circle halfway");
        assert_eq!(*hold_fill, HOLD_FILL, "half hold is fully opaque");
        assert_eq!(
            *circle,
            CIRCLE_FILL.with_alpha(1.0),
            "the held button masks the progress circle underneath",
        );
        assert_close(*r, 56.0, "a hold button keeps the full fill radius");
        let DrawCommand::Svg { color, .. } = draws[2] else {
            panic!("expected icon after both circles")
        };
        assert_eq!(
            color, ICON_PRESSED_TINT,
            "the held-button icon remains visible above both circles",
        );
    }

    #[test]
    fn hold_circle_shrinks_to_the_button_over_the_hold() {
        let radii = |progress| {
            let btn = round_button(
                "k",
                ButtonIcon::square(None),
                112.0,
                48.0,
                CIRCLE_FILL,
                TRANSPARENT,
                Some(progress),
            );
            let TreeNode::Column(_, children) = btn else {
                panic!("expected layered hold button")
            };
            let [TreeNode::Canvas { draws, .. }, TreeNode::Canvas { .. }] = children.as_slice()
            else {
                panic!("expected visual and touch canvases")
            };
            let DrawCommand::Circle { r: hold, .. } = &draws[0] else {
                panic!("expected hold Circle")
            };
            let DrawCommand::Circle { r: button, .. } = &draws[1] else {
                panic!("expected button Circle")
            };
            (*button, *hold)
        };

        let (button, started) = radii(f32::MIN_POSITIVE);
        let (_, half) = radii(0.5);
        let (_, full) = radii(1.0);
        assert_close(
            started,
            button * 2.0,
            "the circle starts at twice the button radius",
        );
        assert_close(
            half,
            button * 1.5,
            "the circle is halfway shrunk at half hold",
        );
        assert_close(full, button, "the circle meets the button at full hold");
        assert!(
            started > half && half > full,
            "the circle radius shrinks monotonically with hold progress"
        );
    }

    #[test]
    fn unheld_button_fills_the_whole_diameter() {
        let btn = round_button(
            "k",
            ButtonIcon::square(None),
            112.0,
            48.0,
            CIRCLE_FILL,
            TRANSPARENT,
            None,
        );
        let TreeNode::Canvas { draws, .. } = btn else {
            panic!("expected Canvas")
        };
        let DrawCommand::Circle { r, .. } = &draws[0] else {
            panic!("expected Circle")
        };
        assert_close(*r, 56.0, "the fill spans the tier diameter");
    }

    #[test]
    fn hold_circle_canvas_is_centered_behind_the_button() {
        let btn = round_button(
            "k",
            ButtonIcon::square(None),
            64.0,
            32.0,
            CIRCLE_FILL,
            TRANSPARENT,
            Some(0.5),
        );
        let TreeNode::Column(props, children) = btn else {
            panic!("expected a fixed-size layered hold button")
        };
        assert_close(props.width, 64.0, "hold button keeps its layout width");
        assert_close(props.height, 64.0, "hold button keeps its layout height");
        let [
            TreeNode::Canvas {
                props: visual_props,
                touch_key: None,
                draws,
            },
            TreeNode::Canvas {
                props: touch_props,
                touch_key: Some(touch_key),
                draws: touch_draws,
            },
        ] = children.as_slice()
        else {
            panic!("expected a visual canvas behind the touch target")
        };
        assert_close(visual_props.width, 128.0, "visual canvas width");
        assert_close(visual_props.height, 128.0, "visual canvas height");
        assert_close(visual_props.inset_top, -32.0, "visual canvas top inset");
        assert_close(visual_props.inset_left, -32.0, "visual canvas left inset");
        assert_eq!(touch_key, "k");
        assert_close(touch_props.width, 64.0, "touch target width");
        assert_close(touch_props.height, 64.0, "touch target height");
        assert!(touch_draws.is_empty());
        let DrawCommand::Circle { r, .. } = &draws[0] else {
            panic!("expected hold circle")
        };
        assert_close(*r, 48.0, "half-hold circle radius");
        let DrawCommand::Circle { r, .. } = &draws[1] else {
            panic!("expected full-size button fill")
        };
        assert_close(*r, 32.0, "hold button fill radius");
    }

    #[test]
    fn hold_circle_fades_in_over_the_start_of_the_hold() {
        let hold_circle_of = |progress| {
            let controls = Controls {
                restart: true,
                ..Controls::default()
            };
            let status = status_at(Action::Restart, Phase::Holding { progress });
            let tree = build_with_controls(wide_panel(), controls, Some(&status));
            assert!(
                find_canvas(&tree, RESTART_KEY).is_some(),
                "the restart button remains present at every hold progress",
            );
            find_hold_circle_for_key(&tree, RESTART_KEY)
        };
        assert_eq!(
            hold_circle_of(0.0),
            None,
            "an unheld button carries no hold circle"
        );
        assert_eq!(
            hold_circle_of(f32::MIN_POSITIVE).map(|(_, color)| color),
            Some(HOLD_FILL.scale_alpha(0.0)),
            "the circle starts transparent",
        );
        assert_eq!(
            hold_circle_of(HOLD_ALPHA_FULL_AT / 2.0).map(|(_, color)| color),
            Some(HOLD_FILL.scale_alpha(0.5)),
            "the circle reaches half opacity midway through its fade",
        );
        assert_eq!(
            hold_circle_of(HOLD_ALPHA_FULL_AT).map(|(_, color)| color),
            Some(HOLD_FILL),
            "the circle reaches full opacity partway in, while it is still shrinking",
        );
        assert_eq!(
            hold_circle_of(1.0).map(|(_, color)| color),
            Some(HOLD_FILL),
            "the circle stays opaque for the rest of the hold",
        );
    }
}
