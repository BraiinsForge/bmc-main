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

//! While a hold button is held, everything but that button dims and a notice
//! over the section above the buttons says what the hold does.

use super::{Controls, RESTART_KEY, WIFI_RECONFIG_KEY};
use bmc_render::tree::{PropsData, TextStyle, TreeNode, col, dimmed, text};
use bmc_wasm_protocol::colors::WHITE;
use bmc_wasm_protocol::{CrossAlign, FontWeight, Justify, TextAlign};

/// How much a hold dims everything but the held button,
/// as the alpha of a black layer over it.
/// Colors are scaled instead of painting that layer:
/// over the black scrim both look the same, but a layer would also darken
/// the scene the scrim lets through, and only where the layer lies.
const DIM_ALPHA: f32 = 0.85;

/// Size of both notice lines.
pub(super) const NOTICE_SIZE: u32 = 20;

/// The two lines over the dimmed section above the buttons: what the hold
/// does, then how it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HoldNotice {
    pub(super) title: &'static str,
    pub(super) body: &'static str,
}

const RESTART_NOTICE: HoldNotice = HoldNotice {
    title: "Restarting the device...",
    body: "Keep holding, or release to cancel",
};

const WIFI_RECONFIG_NOTICE: HoldNotice = HoldNotice {
    title: "Starting Wi-Fi reconfiguration...",
    body: "Keep holding, or release to cancel",
};

/// The button whose hold is in progress. A byte, so `Content` stays small
/// enough to keep passing by value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Restart,
    WifiReconfig,
}

impl Held {
    fn key(self) -> &'static str {
        match self {
            Held::Restart => RESTART_KEY,
            Held::WifiReconfig => WIFI_RECONFIG_KEY,
        }
    }

    fn notice(self) -> HoldNotice {
        match self {
            Held::Restart => RESTART_NOTICE,
            Held::WifiReconfig => WIFI_RECONFIG_NOTICE,
        }
    }
}

/// What a layer of [`DIM_ALPHA`] would let through.
pub(super) const DIMMED: f32 = 1.0 - DIM_ALPHA;

/// Dims everything but the held button while a hold is in progress.
#[derive(Debug, Clone, Copy)]
pub(super) struct HoldDim(Option<Held>);

impl HoldDim {
    /// Held means the hold has progress, the same test that puts
    /// the progress circle behind the button, so both appear on the same frame.
    /// Restart wins a tie, like the caption line.
    pub(super) fn for_controls(controls: &Controls<'_>) -> Self {
        Self(if controls.restart.is_some_and(|r| r.progress > 0.0) {
            Some(Held::Restart)
        } else if controls.wifi_reconfig.progress > 0.0 {
            Some(Held::WifiReconfig)
        } else {
            None
        })
    }

    /// `node`, the button owning `key`: every button dims while a hold
    /// is in progress, except the one being held.
    pub(super) fn button(self, key: &str, node: TreeNode) -> TreeNode {
        match self.0 {
            Some(held) if held.key() != key => dimmed(DIMMED, node),
            Some(_) | None => node,
        }
    }

    /// `node`, anything that is not a button.
    pub(super) fn surroundings(self, node: TreeNode) -> TreeNode {
        match self.0 {
            Some(_) => dimmed(DIMMED, node),
            None => node,
        }
    }

    /// The section above the buttons: a column of `children`,
    /// dimmed under a lit notice while a hold is in progress.
    pub(super) fn with_notice(self, props: PropsData, children: Vec<TreeNode>) -> TreeNode {
        let Some(held) = self.0 else {
            return col(props, children);
        };
        let mut kids: Vec<_> = children
            .into_iter()
            .map(|kid| self.surroundings(kid))
            .collect();
        kids.push(notice_overlay(held.notice()));
        col(props, kids)
    }
}

/// The notice, centered in an out-of-flow column. Zero insets on every side
/// stretch it over the section it ends, and painting follows child order,
/// so the lines land over the section's dimmed content.
fn notice_overlay(notice: HoldNotice) -> TreeNode {
    let line = |s: &'static str, weight: FontWeight| {
        text(
            s,
            TextStyle {
                size: NOTICE_SIZE,
                weight,
                color: WHITE,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        )
    };
    col(
        PropsData {
            inset_top: 0.0,
            inset_right: 0.0,
            inset_bottom: 0.0,
            inset_left: 0.0,
            justify_content: Justify::Center,
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![
            line(notice.title, FontWeight::BOLD),
            line(notice.body, FontWeight::REGULAR),
        ],
    )
}

#[cfg(test)]
mod tests {
    use bmc_render::tree::{DrawCommand, TreeNode, fixed_height};
    use bmc_wasm_protocol::FontWeight;
    use bmc_wasm_protocol::colors::TRANSPARENT;

    use super::*;
    use crate::ui::test_support::*;
    use crate::ui::*;

    fn overlays<'t>(node: &'t TreeNode, out: &mut Vec<&'t TreeNode>) {
        if is_absolute(node) && matches!(node, TreeNode::Column(..)) {
            out.push(node);
        }
        for kid in children(node).into_iter().flatten() {
            overlays(kid, out);
        }
    }

    /// The brightness the QR code draws at, if the layout prints one.
    fn qr_brightness(tree: &TreeNode) -> Option<f32> {
        brightness_where(tree, &|node| {
            matches!(node, TreeNode::Canvas { draws, .. }
                if draws.iter().any(|draw| matches!(draw, DrawCommand::Qr { .. })))
        })
    }

    fn restart_held() -> Controls<'static> {
        Controls {
            wifi_reconfig: HoldControl::default(),
            ..held_controls()
        }
    }

    fn wifi_held() -> Controls<'static> {
        Controls {
            restart: Some(HoldControl::default()),
            ..held_controls()
        }
    }

    #[test]
    fn nothing_dims_until_a_hold_has_progress() {
        let pressed_only = Controls {
            pressed: Some(RESTART_KEY),
            ..all_controls()
        };
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            for controls in [all_controls(), pressed_only] {
                let tree = build_with_controls(panel, controls);
                let mut found = Vec::new();
                overlays(&tree, &mut found);
                assert!(
                    found.is_empty(),
                    "{panel:?}: a resting or merely pressed tray carries no notice"
                );
                assert_eq!(
                    text_brightness(&tree, "10.0.0.2"),
                    Some(1.0),
                    "{panel:?}: the address stays lit"
                );
            }
        }
    }

    /// The address, the network, the QR code, the caption line and the close glyph
    /// all paint dimmed, on every layout, whichever hold is in progress.
    #[test]
    fn a_hold_dims_everything_around_the_buttons() {
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            for controls in [restart_held(), wifi_held()] {
                let tree = build_with_controls(panel, controls);
                for needle in ["10.0.0.2", "MyWifi"] {
                    assert_eq!(
                        text_brightness(&tree, needle),
                        Some(DIMMED),
                        "{panel:?}: {needle} dims"
                    );
                }
                let expected_qr = (layout_for(&panel) != Layout::Round).then_some(DIMMED);
                assert_eq!(
                    qr_brightness(&tree),
                    expected_qr,
                    "{panel:?}: the rectangular layouts print a QR, and it dims"
                );
                if layout_for(&panel) != Layout::Compact {
                    let caption = if controls.restart.is_some_and(|r| r.progress > 0.0) {
                        "Restart: Keep holding…"
                    } else {
                        "Reconfigure Wi-Fi: Keep holding…"
                    };
                    assert_eq!(
                        text_brightness(&tree, caption),
                        Some(DIMMED),
                        "{panel:?}: the caption line under the buttons dims"
                    );
                }
                assert_eq!(
                    canvas_brightness(&tree, CLOSE_KEY),
                    Some(DIMMED),
                    "{panel:?}: the close glyph dims"
                );
                assert_eq!(
                    text_brightness(&tree, controls_notice(controls).title),
                    Some(1.0),
                    "{panel:?}: the notice over the dimmed section stays lit"
                );
            }
        }
    }

    /// The notice column's box comes from its insets alone,
    /// so it takes the size of the section it ends.
    #[test]
    fn the_notice_stretches_over_its_section() {
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            let tree = build_with_controls(panel, restart_held());
            let mut found = Vec::new();
            overlays(&tree, &mut found);
            let [TreeNode::Column(props, _)] = found.as_slice() else {
                panic!("{panel:?}: one notice column, got {}", found.len())
            };
            for inset in [
                props.inset_top,
                props.inset_right,
                props.inset_bottom,
                props.inset_left,
            ] {
                assert_close(inset, 0.0, "the notice hugs every edge of its section");
            }
            assert_close(props.width, 0.0, "no fixed width, the insets size it");
            assert_close(props.height, 0.0, "no fixed height, the insets size it");
            assert_eq!(
                props.background, TRANSPARENT,
                "no layer: the renderer dims the colors beneath"
            );
        }
    }

    /// The notice reads once, over the section that precedes the first button row.
    #[test]
    fn the_notice_sits_above_the_buttons() {
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            let tree = build_with_controls(panel, restart_held());
            let mut found = Vec::new();
            overlays(&tree, &mut found);
            let [TreeNode::Column(_, lines)] = found.as_slice() else {
                panic!("{panel:?}: the notice reads once")
            };
            let styles: Vec<_> = lines
                .iter()
                .map(|line| {
                    let TreeNode::Paragraph {
                        base_style, spans, ..
                    } = line
                    else {
                        panic!("a notice line is a paragraph")
                    };
                    (
                        spans[0].text.clone(),
                        base_style.size,
                        base_style.weight,
                        base_style.color,
                    )
                })
                .collect();
            assert_eq!(
                styles,
                vec![
                    (
                        RESTART_NOTICE.title.to_owned(),
                        NOTICE_SIZE,
                        FontWeight::BOLD,
                        WHITE
                    ),
                    (
                        RESTART_NOTICE.body.to_owned(),
                        NOTICE_SIZE,
                        FontWeight::REGULAR,
                        WHITE
                    ),
                ],
                "{panel:?}: a bold title over a regular body, both 20px white"
            );

            let root_kids = children(&tree).expect("BUG: root must be a container");
            let notice_index = root_kids
                .iter()
                .position(|kid| {
                    let mut own = Vec::new();
                    overlays(kid, &mut own);
                    !own.is_empty()
                })
                .expect("BUG: the notice must hang off a root child");
            let first_row = root_kids
                .iter()
                .position(|kid| {
                    let mut keys = Vec::new();
                    canvas_keys(kid, &mut keys);
                    keys.iter().any(|k| is_control_key(k))
                })
                .expect("BUG: control rows must render");
            assert!(
                notice_index < first_row,
                "{panel:?}: the notice precedes the buttons in flow"
            );
            if layout_for(&panel) == Layout::Wide {
                let TreeNode::Column(_, top_kids) = &root_kids[0] else {
                    panic!("the top half is a column")
                };
                assert!(
                    top_kids.last().is_some_and(is_absolute),
                    "the notice ends the top half itself, so it spans the half \
                     and not just the header's height"
                );
            }
        }
    }

    #[test]
    fn restart_wins_the_notice_when_both_holds_have_progress() {
        let notice = |controls: Controls<'_>| HoldDim::for_controls(&controls).0.map(Held::notice);
        assert_eq!(notice(held_controls()), Some(RESTART_NOTICE));
        assert_eq!(notice(wifi_held()), Some(WIFI_RECONFIG_NOTICE));
        assert_eq!(notice(all_controls()), None);
    }

    fn controls_notice(controls: Controls<'_>) -> HoldNotice {
        HoldDim::for_controls(&controls)
            .0
            .map(Held::notice)
            .expect("BUG: a hold is in progress")
    }

    #[test]
    fn only_the_held_button_stays_lit() {
        let at = |node: TreeNode| {
            if let TreeNode::Dimmed { brightness, .. } = node {
                brightness
            } else {
                1.0
            }
        };
        let probe = || fixed_height(1.0);
        let dim = HoldDim::for_controls(&held_controls());
        assert_close(at(dim.button(RESTART_KEY, probe())), 1.0, "the held button");
        for key in [WIFI_RECONFIG_KEY, NIGHT_MODE_KEY, VOLUME_UP_KEY, CLOSE_KEY] {
            assert_close(at(dim.button(key, probe())), DIMMED, key);
        }
        assert_close(at(dim.surroundings(probe())), DIMMED, "the surroundings");
        let resting = HoldDim::for_controls(&all_controls());
        assert_close(
            at(resting.button(WIFI_RECONFIG_KEY, probe())),
            1.0,
            "a resting button",
        );
        assert_close(
            at(resting.surroundings(probe())),
            1.0,
            "resting surroundings",
        );
    }
}
