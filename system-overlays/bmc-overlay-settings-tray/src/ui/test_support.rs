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

//! Fixtures and tree walkers the layout tests share: the three panels,
//! a fully populated control set, and readers that pull text, canvases
//! and widths back out of a built tree.

use bmc_render::ui::{AnimProperty, AnimationDef, Color, Draw, Fill, Node, SvgId, TextStyle};
use bmc_system_overlay::DisplayShape;

use super::*;

pub(super) fn round_panel() -> Panel {
    Panel {
        shape: DisplayShape::Round,
        width: 480,
        height: 480,
        wifi_button: true,
    }
}

pub(super) fn wide_panel() -> Panel {
    Panel {
        shape: DisplayShape::Rectangular,
        width: 1280,
        height: 480,
        wifi_button: true,
    }
}

pub(super) fn narrow_panel() -> Panel {
    Panel {
        shape: DisplayShape::Rectangular,
        width: 480,
        height: 320,
        wifi_button: true,
    }
}

/// A distinct id per signal icon, so a test can tell which one was drawn.
pub(super) fn distinct_icons() -> WifiIcons {
    let id = |raw| SvgId::from_wire(raw).expect("BUG: test SvgId must be non-zero");
    WifiIcons {
        problem: Some(id(1)),
        low: Some(id(2)),
        fair: Some(id(3)),
        strong: Some(id(4)),
        cable: Some(id(5)),
    }
}

/// Every optional control present: the worst-case layout.
pub(super) fn all_controls() -> Controls<'static> {
    Controls {
        brightness: Some(100),
        volume: Some(100),
        night_mode: Some(NightMode {
            active: true,
            until: Some("06:30"),
        }),
        restart: true,
        pressed: None,
    }
}

/// `action` at `phase`, with no reason from bmc.
pub(super) fn status_at(action: Action, phase: Phase) -> Status {
    Status {
        action,
        phase,
        reason: None,
    }
}

/// A five-second hold `progress` of the way through.
pub(super) fn holding(progress: f32) -> Phase {
    let length = std::time::Duration::from_secs(5);
    Phase::Holding(Hold::new(length.mul_f32(progress), length))
}

/// Restart held halfway.
pub(super) fn held_status() -> Status {
    status_at(Action::Restart, holding(0.5))
}

pub(super) fn build(panel: Panel, view: WifiView<'_>) -> Node {
    build_tree(
        Some("braiins-deck"),
        Some("10.0.0.2"),
        Some(-55),
        Some("MyWifi"),
        WifiIcons::default(),
        panel,
        view,
        ControlIcons::default(),
        Controls::default(),
        None,
    )
}

pub(super) fn build_with_controls(
    panel: Panel,
    controls: Controls<'_>,
    status: Option<&Status>,
) -> Node {
    build_tree(
        Some("braiins-deck"),
        Some("10.0.0.2"),
        Some(-55),
        Some("MyWifi"),
        WifiIcons::default(),
        panel,
        WifiView::Idle,
        ControlIcons::default(),
        controls,
        status,
    )
}

/// Every SVG icon id the subtree draws, in tree order.
pub(super) fn svg_ids(node: &Node, out: &mut Vec<SvgId>) {
    if let Node::Canvas { draws, .. } = node {
        out.extend(draws.iter().filter_map(|draw| {
            if let Draw::Svg { icon_id, .. } = draw {
                *icon_id
            } else {
                None
            }
        }));
    }
    for kid in children(node).into_iter().flatten() {
        svg_ids(kid, out);
    }
}

/// Direct children of a container node, if any.
pub(super) fn children(node: &Node) -> Option<&[Node]> {
    match node {
        Node::Column(_, kids)
        | Node::Row(_, kids)
        | Node::Center(_, kids)
        | Node::Scroll { children: kids, .. } => Some(kids),
        Node::Tag { content, .. } | Node::Dimmed { child: content, .. } => {
            Some(std::slice::from_ref(&**content))
        }
        Node::Paragraph { .. }
        | Node::Button { .. }
        | Node::Spacer { .. }
        | Node::Canvas { .. }
        | Node::Notification { .. }
        | Node::RelTime { .. }
        | Node::Modal { .. }
        | Node::ProgressBar { .. }
        | Node::Switcher { .. }
        | Node::Skeleton(_) => None,
    }
}

pub(super) fn is_absolute(node: &Node) -> bool {
    match node {
        Node::Column(props, _)
        | Node::Row(props, _)
        | Node::Center(props, _)
        | Node::Paragraph { props, .. }
        | Node::Canvas { props, .. }
        | Node::Scroll { props, .. } => props.is_absolute(),
        Node::Dimmed { child, .. } => is_absolute(child),
        Node::Button { .. }
        | Node::Spacer { .. }
        | Node::Notification { .. }
        | Node::RelTime { .. }
        | Node::Tag { .. }
        | Node::Switcher { .. }
        | Node::Skeleton(_)
        | Node::Modal { .. }
        | Node::ProgressBar { .. } => false,
    }
}

/// Depth-first search for the first Canvas carrying `key`.
pub(super) fn find_canvas<'t>(node: &'t Node, key: &str) -> Option<&'t Vec<Draw>> {
    if let Node::Canvas {
        touch_key: Some(k),
        draws,
        ..
    } = node
        && k == key
    {
        return Some(draws);
    }
    children(node)?.iter().find_map(|k| find_canvas(k, key))
}

/// A hold circle's radius and colour as `pick` reads its animations:
/// `|a| a.from` for the first frame, `|a| a.to` for the last.
fn hold_circle_with(draw: &Draw, pick: impl Fn(&AnimationDef) -> f32) -> Option<(f32, Color)> {
    let Draw::Modified {
        animations, inner, ..
    } = draw
    else {
        return None;
    };
    let Draw::Circle {
        r,
        fill: Fill::Solid(color),
        ..
    } = inner.as_ref()
    else {
        return None;
    };
    let factor = |property| {
        animations
            .iter()
            .find(|a| a.property == property)
            .map_or(1.0, &pick)
    };
    Some((
        r * factor(AnimProperty::Scale),
        color.scale_alpha(factor(AnimProperty::Alpha)),
    ))
}

/// A hold circle as the first frame after a build draws it.
pub(super) fn hold_circle_at_start(draw: &Draw) -> Option<(f32, Color)> {
    hold_circle_with(draw, |a| a.from)
}

/// A hold circle as it ends up once its animations finish.
pub(super) fn hold_circle_at_end(draw: &Draw) -> Option<(f32, Color)> {
    hold_circle_with(draw, |a| a.to)
}

pub(super) fn find_hold_circle_for_key(node: &Node, key: &str) -> Option<(f32, Color)> {
    if let Node::Column(_, children) = node
        && let [
            Node::Canvas {
                touch_key: None,
                draws,
                ..
            },
            Node::Canvas {
                touch_key: Some(canvas_key),
                ..
            },
        ] = children.as_slice()
        && canvas_key == key
    {
        return draws.iter().find_map(hold_circle_at_start);
    }
    children(node)?
        .iter()
        .find_map(|child| find_hold_circle_for_key(child, key))
}

/// Recursively collect every keyed Canvas touch key in the tree.
pub(super) fn canvas_keys(node: &Node, out: &mut Vec<String>) {
    if let Node::Canvas {
        touch_key: Some(k), ..
    } = node
    {
        out.push(k.clone());
    }
    if let Some(kids) = children(node) {
        for k in kids {
            canvas_keys(k, out);
        }
    }
}

/// Every key in the tree that still takes touches, the slider's included.
pub(super) fn touch_keys(node: &Node) -> Vec<String> {
    let mut out = Vec::new();
    collect_touch_keys(node, &mut out);
    out.sort();
    out
}

fn collect_touch_keys(node: &Node, out: &mut Vec<String>) {
    if let Node::Canvas {
        touch_key: Some(k), ..
    }
    | Node::ProgressBar {
        touch_key: Some(k), ..
    } = node
    {
        out.push(k.clone());
    }
    for kid in children(node).into_iter().flatten() {
        collect_touch_keys(kid, out);
    }
}

/// Whether the subtree contains the unkeyed WiFi status-icon canvas.
pub(super) fn has_unkeyed_canvas(node: &Node) -> bool {
    if let Node::Canvas {
        touch_key: None, ..
    } = node
    {
        return true;
    }
    children(node).into_iter().flatten().any(has_unkeyed_canvas)
}

/// Recursively collect the payload of every QR draw in the tree.
pub(super) fn qr_texts(node: &Node, out: &mut Vec<String>) {
    if let Node::Canvas { draws, .. } = node {
        out.extend(draws.iter().filter_map(|draw| {
            if let Draw::Qr { text, .. } = draw {
                Some(text.clone())
            } else {
                None
            }
        }));
    }
    if let Some(kids) = children(node) {
        for k in kids {
            qr_texts(k, out);
        }
    }
}

/// Recursively collect every span text in the tree.
pub(super) fn collect_texts(node: &Node, out: &mut Vec<String>) {
    if let Node::Paragraph { spans, .. } = node {
        for span in spans {
            out.push(span.text.clone());
        }
    }
    if let Some(kids) = children(node) {
        for k in kids {
            collect_texts(k, out);
        }
    }
}

/// The style of the paragraph holding `needle`.
pub(super) fn style_of(node: &Node, needle: &str) -> Option<TextStyle> {
    if let Node::Paragraph {
        base_style, spans, ..
    } = node
        && spans.iter().any(|span| span.text == needle)
    {
        return Some(*base_style);
    }
    children(node)?
        .iter()
        .find_map(|child| style_of(child, needle))
}

pub(super) fn text_color(node: &Node, needle: &str) -> Option<Color> {
    style_of(node, needle).map(|style| style.color)
}

/// `node` with any [`Node::Dimmed`] wrappers peeled off:
/// what actually lays out.
pub(super) fn undimmed(mut node: &Node) -> &Node {
    while let Node::Dimmed { child, .. } = node {
        node = child;
    }
    node
}

/// The brightness the first node `hit` accepts draws at:
/// the product of the [`Node::Dimmed`] factors above it.
pub(super) fn brightness_where(node: &Node, hit: &dyn Fn(&Node) -> bool) -> Option<f32> {
    if hit(node) {
        return Some(1.0);
    }
    let own = if let Node::Dimmed { brightness, .. } = node {
        *brightness
    } else {
        1.0
    };
    children(node)?
        .iter()
        .find_map(|kid| brightness_where(kid, hit))
        .map(|below| own * below)
}

/// [`brightness_where`] for the paragraph showing `needle`.
pub(super) fn text_brightness(tree: &Node, needle: &str) -> Option<f32> {
    brightness_where(
        tree,
        &|node| matches!(node, Node::Paragraph { spans, .. } if spans.iter().any(|s| s.text == needle)),
    )
}

/// [`brightness_where`] for the canvas carrying `key`.
pub(super) fn canvas_brightness(tree: &Node, key: &str) -> Option<f32> {
    brightness_where(
        tree,
        &|node| matches!(node, Node::Canvas { touch_key: Some(k), .. } if k == key),
    )
}

/// Largest text size in the subtree — the line height driver of a text
/// band (hostname, caption).
pub(super) fn max_text_size(node: &Node) -> u32 {
    let own = if let Node::Paragraph { base_style, .. } = node {
        base_style.size
    } else {
        0
    };
    children(node)
        .into_iter()
        .flatten()
        .map(max_text_size)
        .fold(own, u32::max)
}

/// Abs-diff float assertion (`float_cmp` is denied by the workspace lints).
pub(super) fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-3,
        "{what}: expected ~{expected}, got {actual}"
    );
}

pub(super) fn assert_circle(panel: &Panel, expected: f32) {
    assert!((tier_for(panel).circle - expected).abs() < f32::EPSILON);
}

pub(super) const PAIR_KEYS: [&str; 4] = [
    VOLUME_DOWN_KEY,
    VOLUME_UP_KEY,
    BRIGHTNESS_DOWN_KEY,
    BRIGHTNESS_UP_KEY,
];

pub(super) const SINGLE_KEYS: [&str; 3] = [NIGHT_MODE_KEY, RESTART_KEY, WIFI_RECONFIG_KEY];

pub(super) fn is_control_key(k: &str) -> bool {
    PAIR_KEYS.contains(&k) || SINGLE_KEYS.contains(&k)
}

/// Per-glyph advance (px) used to estimate 24px single-line strings.
/// This is an average, not a bound: BraiinsSans-Bold's widest ASCII glyph
/// advances 23.4px, so a run of wide capitals measures wider.
/// Digits and dots stay far under it, which is what the IP header relies on.
pub(super) const HOSTNAME_CHAR_W: f32 = 16.0;

/// Width of a single-line string at [`HOSTNAME_CHAR_W`] a glyph —
/// an estimate, not a bound.
#[expect(clippy::cast_precision_loss, reason = "text sizes are small")]
pub(super) fn line_width(s: &str, size: u32) -> f32 {
    s.chars().count() as f32 * HOSTNAME_CHAR_W * (size as f32) / 24.0
}

/// Min-content width of a subtree: rows sum their kids plus their gaps,
/// anything else takes its widest child, and a declared width is a floor,
/// not a cap — content too wide for its box still overflows the panel.
/// Structurally worst-case, but only as accurate as [`HOSTNAME_CHAR_W`].
#[expect(clippy::cast_precision_loss, reason = "child counts are small")]
pub(super) fn min_content_width(node: &Node) -> f32 {
    if let Node::Canvas { props, .. } = node {
        return props.width;
    }
    if let Node::Paragraph {
        base_style, spans, ..
    } = node
    {
        return spans
            .iter()
            .map(|span| line_width(&span.text, base_style.size))
            .sum();
    }
    let kids: &[Node] = children(node).unwrap_or_default();
    let flow_kids = kids.iter().filter(|child| !is_absolute(child));
    let intrinsic = if let Node::Row(props, _) = node {
        let gaps = props.gap * flow_kids.clone().count().saturating_sub(1) as f32;
        flow_kids.map(min_content_width).sum::<f32>() + gaps
    } else {
        flow_kids.map(min_content_width).fold(0.0, f32::max)
    };
    if let Node::Row(props, _) | Node::Column(props, _) = node {
        return intrinsic.max(props.width);
    }
    intrinsic
}
