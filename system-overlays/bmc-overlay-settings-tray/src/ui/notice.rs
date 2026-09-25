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

//! While a hold action runs, is pending or has just failed, a notice over the
//! section above the buttons says so, and everything but the action's own
//! button dims and stops taking touches.

use super::{Action, Controls, Phase, Status};
use bmc_render::tree::{PropsData, TextStyle, TreeNode, col, dimmed, text};
use bmc_wasm_protocol::colors::WHITE;
use bmc_wasm_protocol::{CrossAlign, FontWeight, Justify, TextAlign};

/// How much the notice dims everything but the action's button,
/// as the alpha of a black layer over it.
/// Colors are scaled instead of painting that layer:
/// over the black scrim both look the same, but a layer would also darken
/// the scene the scrim lets through, and only where the layer lies.
const DIM_ALPHA: f32 = 0.85;

/// What a layer of [`DIM_ALPHA`] would let through.
pub(super) const DIMMED: f32 = 1.0 - DIM_ALPHA;

/// Size of both notice lines where the layout names none of its own.
pub(super) const NOTICE_SIZE: u32 = 20;

const KEEP_HOLDING: &str = "Keep holding, or release to cancel";
const TRY_AGAIN: &str = "Try again";

/// The decline reasons bmc sends with `restart_declined`.
const DECLINED_FOR_UPGRADE: &str = "upgrade in progress";
const DECLINED_AS_FAILED: &str = "restart failed";

/// The notice's lines: what is happening, then, if anything,
/// what the user can do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NoticeText<'a> {
    pub(super) title: &'static str,
    pub(super) body: Option<&'a str>,
}

impl<'a> NoticeText<'a> {
    /// A decline reason bmc has no wording for here shows as it came.
    pub(super) fn for_status(status: Status<'a>) -> Self {
        let (title, body) = match (status.action, status.phase) {
            (Action::Restart, Phase::Holding { .. }) => ("Restart the device?", Some(KEEP_HOLDING)),
            (Action::Restart, Phase::Pending) => ("Restarting the device…", None),
            (Action::Restart, Phase::Failed) => match status.reason {
                Some(DECLINED_FOR_UPGRADE) => {
                    ("Can't restart now", Some("An upgrade is in progress"))
                }
                None | Some(DECLINED_AS_FAILED) => ("Restart failed", Some(TRY_AGAIN)),
                Some(reason) => ("Can't restart now", Some(reason)),
            },
            (Action::WifiReconfig, Phase::Holding { .. }) => {
                ("Reconfigure Wi-Fi?", Some(KEEP_HOLDING))
            }
            (Action::WifiReconfig, Phase::Pending) => ("Starting Wi-Fi setup…", None),
            (Action::WifiReconfig, Phase::Failed) => {
                ("Couldn't start Wi-Fi setup", Some(TRY_AGAIN))
            }
        };
        Self { title, body }
    }
}

/// Dims and disables everything but the action's button while the notice is up.
#[derive(Debug, Clone, Copy)]
pub(super) struct Notice<'a>(Option<Status<'a>>);

impl<'a> Notice<'a> {
    pub(super) fn for_controls(controls: &Controls<'a>) -> Self {
        Self(controls.status)
    }

    /// `node`, the button owning `key`: every button but the action's own
    /// dims and stops taking touches while the notice is up.
    pub(super) fn button(self, key: &str, node: TreeNode) -> TreeNode {
        match self.0 {
            Some(status) if status.action.key() != key => shade(node),
            Some(_) | None => node,
        }
    }

    /// `node`, anything that is not a button.
    pub(super) fn surroundings(self, node: TreeNode) -> TreeNode {
        match self.0 {
            Some(_) => shade(node),
            None => node,
        }
    }

    /// The close button dims with the rest but keeps taking touches,
    /// so the tray can always be left.
    pub(super) fn close(self, node: TreeNode) -> TreeNode {
        match self.0 {
            Some(_) => dimmed(DIMMED, node),
            None => node,
        }
    }

    /// The section above the buttons: a column of `children`,
    /// dimmed under a lit notice of `size` while one is up.
    pub(super) fn with_notice(
        self,
        props: PropsData,
        size: u32,
        children: Vec<TreeNode>,
    ) -> TreeNode {
        let Some(status) = self.0 else {
            return col(props, children);
        };
        let mut kids: Vec<_> = children
            .into_iter()
            .map(|kid| self.surroundings(kid))
            .collect();
        kids.push(notice_overlay(NoticeText::for_status(status), size));
        col(props, kids)
    }
}

/// `node`, dimmed and with every touch target in it removed.
fn shade(mut node: TreeNode) -> TreeNode {
    disable(&mut node);
    dimmed(DIMMED, node)
}

fn disable(node: &mut TreeNode) {
    match node {
        TreeNode::Canvas { touch_key, .. } | TreeNode::ProgressBar { touch_key, .. } => {
            *touch_key = None;
        }
        TreeNode::Button { disabled, .. } | TreeNode::Switcher { disabled, .. } => {
            *disabled = true;
        }
        TreeNode::Column(_, kids)
        | TreeNode::Row(_, kids)
        | TreeNode::Center(_, kids)
        | TreeNode::Scroll { children: kids, .. }
        | TreeNode::Modal { body: kids, .. } => kids.iter_mut().for_each(disable),
        TreeNode::Tag { content: kid, .. } | TreeNode::Dimmed { child: kid, .. } => disable(kid),
        TreeNode::Paragraph { .. }
        | TreeNode::Spacer { .. }
        | TreeNode::Notification { .. }
        | TreeNode::RelTime { .. }
        | TreeNode::Skeleton(_) => {}
    }
}

/// The notice, centered in an out-of-flow column. Zero insets on every side
/// stretch it over the section it ends, and painting follows child order,
/// so the lines land over the section's dimmed content.
fn notice_overlay(notice: NoticeText<'_>, size: u32) -> TreeNode {
    let line = |s: &str, weight: FontWeight| {
        text(
            s,
            TextStyle {
                size,
                weight,
                color: WHITE,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        )
    };
    let mut lines = vec![line(notice.title, FontWeight::BOLD)];
    lines.extend(notice.body.map(|body| line(body, FontWeight::REGULAR)));
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
        lines,
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

    /// Every status the notice can report, one per action and phase.
    fn every_status() -> Vec<Controls<'static>> {
        [Action::Restart, Action::WifiReconfig]
            .into_iter()
            .flat_map(|action| {
                [
                    Phase::Holding { progress: 0.5 },
                    Phase::Pending,
                    Phase::Failed,
                ]
                .map(|phase| controls_at(action, phase))
            })
            .collect()
    }

    fn status_of(controls: Controls<'static>) -> Status<'static> {
        controls.status.expect("BUG: the fixture carries a status")
    }

    #[test]
    fn nothing_dims_at_rest() {
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

    /// Pending and failed dim like a hold does: the notice stays up
    /// until the action has an outcome to show and has shown it.
    #[test]
    fn every_status_dims_everything_around_the_buttons() {
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            for controls in every_status() {
                let tree = build_with_controls(panel, controls);
                let what = format!("{panel:?} {:?}", controls.status);
                for needle in ["10.0.0.2", "MyWifi"] {
                    assert_eq!(
                        text_brightness(&tree, needle),
                        Some(DIMMED),
                        "{what}: {needle}"
                    );
                }
                let expected_qr = (layout_for(&panel) != Layout::Round).then_some(DIMMED);
                assert_eq!(qr_brightness(&tree), expected_qr, "{what}: the QR");
                assert_eq!(
                    canvas_brightness(&tree, CLOSE_KEY),
                    Some(DIMMED),
                    "{what}: close"
                );
                let title = NoticeText::for_status(status_of(controls)).title;
                assert_eq!(
                    text_brightness(&tree, title),
                    Some(1.0),
                    "{what}: the notice stays lit"
                );
            }
        }
    }

    #[test]
    fn each_status_reads_its_own_lines() {
        let text = |action, phase, reason| {
            let status = Status {
                action,
                phase,
                reason,
            };
            let text = NoticeText::for_status(status);
            (text.title, text.body)
        };
        let holding = Phase::Holding { progress: 0.5 };
        let restart = Action::Restart;
        let wifi = Action::WifiReconfig;
        assert_eq!(
            text(restart, holding, None),
            ("Restart the device?", Some(KEEP_HOLDING))
        );
        assert_eq!(
            text(restart, Phase::Pending, None),
            ("Restarting the device…", None)
        );
        assert_eq!(
            text(restart, Phase::Failed, Some(DECLINED_FOR_UPGRADE)),
            ("Can't restart now", Some("An upgrade is in progress"))
        );
        assert_eq!(
            text(restart, Phase::Failed, Some(DECLINED_AS_FAILED)),
            ("Restart failed", Some(TRY_AGAIN))
        );
        assert_eq!(
            text(restart, Phase::Failed, None),
            ("Restart failed", Some(TRY_AGAIN)),
            "a pending timeout carries no reason"
        );
        assert_eq!(
            text(restart, Phase::Failed, Some("battery low")),
            ("Can't restart now", Some("battery low")),
            "a reason with no wording here shows as it came"
        );
        assert_eq!(
            text(wifi, holding, None),
            ("Reconfigure Wi-Fi?", Some(KEEP_HOLDING))
        );
        assert_eq!(
            text(wifi, Phase::Pending, None),
            ("Starting Wi-Fi setup…", None)
        );
        assert_eq!(
            text(wifi, Phase::Failed, None),
            ("Couldn't start Wi-Fi setup", Some(TRY_AGAIN))
        );
    }

    #[test]
    fn a_notice_without_a_body_reads_one_line() {
        let tree =
            build_with_controls(narrow_panel(), controls_at(Action::Restart, Phase::Pending));
        let mut found = Vec::new();
        overlays(&tree, &mut found);
        let [TreeNode::Column(_, lines)] = found.as_slice() else {
            panic!("one notice column, got {}", found.len())
        };
        assert_eq!(lines.len(), 1, "the title alone");
    }

    /// The notice column's box comes from its insets alone,
    /// so it takes the size of the section it ends.
    #[test]
    fn the_notice_stretches_over_its_section() {
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            let tree = build_with_controls(panel, held_controls());
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
            let tree = build_with_controls(panel, held_controls());
            let mut found = Vec::new();
            overlays(&tree, &mut found);
            let [TreeNode::Column(_, lines)] = found.as_slice() else {
                panic!("{panel:?}: the notice reads once")
            };
            let size = if layout_for(&panel) == Layout::Wide {
                WIDE_NOTICE_SIZE
            } else {
                NOTICE_SIZE
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
                        "Restart the device?".to_owned(),
                        size,
                        FontWeight::BOLD,
                        WHITE
                    ),
                    (KEEP_HOLDING.to_owned(), size, FontWeight::REGULAR, WHITE),
                ],
                "{panel:?}: a bold title over a regular body, both {size}px white"
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
    fn only_the_actions_button_stays_lit_and_live() {
        let at = |node: &TreeNode| {
            if let TreeNode::Dimmed { brightness, .. } = node {
                *brightness
            } else {
                1.0
            }
        };
        let probe = |key: &str| TreeNode::Canvas {
            props: PropsData::default(),
            touch_key: Some(key.to_owned()),
            draws: Vec::new(),
        };
        let notice = Notice::for_controls(&held_controls());
        let held = notice.button(RESTART_KEY, probe(RESTART_KEY));
        assert_close(at(&held), 1.0, "the held button");
        assert_eq!(
            touch_keys(&held),
            [RESTART_KEY],
            "the held button takes touches"
        );
        for key in [WIFI_RECONFIG_KEY, NIGHT_MODE_KEY, VOLUME_UP_KEY] {
            let other = notice.button(key, probe(key));
            assert_close(at(&other), DIMMED, key);
            assert!(touch_keys(&other).is_empty(), "{key} stops taking touches");
        }
        let slider = notice.surroundings(probe(BRIGHTNESS_SLIDER_KEY));
        assert_close(at(&slider), DIMMED, "the surroundings");
        assert!(
            touch_keys(&slider).is_empty(),
            "the surroundings stop taking touches"
        );
        let close = notice.close(probe(CLOSE_KEY));
        assert_close(at(&close), DIMMED, "close dims");
        assert_eq!(
            touch_keys(&close),
            [CLOSE_KEY],
            "close keeps taking touches"
        );

        let resting = Notice::for_controls(&all_controls());
        let button = resting.button(WIFI_RECONFIG_KEY, probe(WIFI_RECONFIG_KEY));
        assert_close(at(&button), 1.0, "a resting button");
        assert_eq!(touch_keys(&button), [WIFI_RECONFIG_KEY]);
        assert_close(
            at(&resting.surroundings(fixed_height(1.0))),
            1.0,
            "resting surroundings",
        );
    }
}
