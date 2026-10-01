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

//! The BMM101 arrangement: the address block,
//! then the brightness slider, then labeled buttons.

use super::controls::{control_groups, control_rows};
use super::notice::NOTICE_SIZE;
use super::parts::{
    ETHERNET, close_origin, connection_icon, fixed_width, ip_qr, pad_horizontal, setup_badge,
    svg_icon, text_style,
};
use super::{
    BRIGHTNESS_ICON_SIZE, BRIGHTNESS_SLIDER_KEY, COMPACT_CLOSE_MARGIN, COMPACT_CONNECTION_GAP,
    COMPACT_CONNECTION_ICON_SIZE, COMPACT_GAP, COMPACT_INFO_SIZE, COMPACT_QR_SIZE, Content,
    ControlIcons, NO_DATA_PLACEHOLDER, Panel, SLIDER_TRACK_H, Tier, WifiView, brightness_fraction,
};
use bmc_render::tree::{PropsData, TextStyle, TreeNode, col, fixed_height, row, spacer, text};
use bmc_wasm_protocol::colors::{GRAY_40, TRANSPARENT, WHITE};
use bmc_wasm_protocol::{CrossAlign, ProgressKind, TextAlign, TextOverflow};

/// A draggable slider holding no value of its own. The drag position comes
/// back on `key`; the caller feeds the new fraction in on the next frame.
pub(super) fn slider(key: &'static str, fraction: f32) -> TreeNode {
    TreeNode::ProgressBar {
        touch_key: Some(key.to_owned()),
        track_h: SLIDER_TRACK_H,
        mode: ProgressKind::Slider,
        fraction: fraction.clamp(0.0, 1.0),
        active: false,
        fill_color: WHITE,
        track_color: GRAY_40,
        bg_color: TRANSPARENT,
        skin: None,
    }
}

/// What the device is online through: the cable while it carries the uplink,
/// else the station SSID, or the SETUP badge while setup runs.
fn compact_connection_value(content: Content<'_>, style: TextStyle) -> TreeNode {
    let icon = connection_icon(
        content.wifi_view,
        content.icons,
        content.wifi_signal,
        COMPACT_CONNECTION_ICON_SIZE,
    );
    let name = match content.wifi_view {
        WifiView::Cable => text(ETHERNET, style),
        WifiView::Idle => text(content.ssid, style),
        // The AP SSID stays off, unlike on the other layouts: this cell has
        // sixteen characters, and the device-info screen names it in full.
        WifiView::Setup { .. } => setup_badge(style.size),
    };
    row(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![spacer(1.0), icon, fixed_width(COMPACT_CONNECTION_GAP), name],
    )
}

/// The compact address block. Both columns come from one list, so a row
/// cannot appear on one side alone and slide the values out of line.
/// Capped short of the close target floating over its top-right corner.
fn compact_info_row(content: Content<'_>, panel: Panel, tier: Tier) -> TreeNode {
    let label_style = TextStyle {
        align: TextAlign::Left,
        ..text_style(COMPACT_INFO_SIZE, GRAY_40)
    };
    let value_style = TextStyle {
        align: TextAlign::Right,
        text_overflow: TextOverflow::Ellipsis,
        ..text_style(COMPACT_INFO_SIZE, WHITE)
    };

    let mut kids = vec![fixed_width(COMPACT_GAP)];
    if let Some(ip) = content.ip {
        kids.push(ip_qr(ip, COMPACT_QR_SIZE));
        kids.push(fixed_width(COMPACT_GAP));
    }

    let rows = [
        (
            "Hostname",
            text(content.hostname.unwrap_or(NO_DATA_PLACEHOLDER), value_style),
        ),
        (
            "IP Address",
            text(content.ip.unwrap_or(NO_DATA_PLACEHOLDER), value_style),
        ),
        ("Connection", compact_connection_value(content, value_style)),
    ];
    let (label_nodes, value_nodes): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .map(|(label, value)| (text(label, label_style), value))
        .unzip();
    kids.extend([
        col(
            PropsData {
                gap: COMPACT_GAP,
                ..PropsData::default()
            },
            label_nodes,
        ),
        fixed_width(COMPACT_GAP),
        // Grown from zero rather than shrunk from its content, so an overlong value
        // gives way inside its own column and never squeezes a label into wrapping.
        col(
            PropsData {
                flex: 1.0,
                gap: COMPACT_GAP,
                ..PropsData::default()
            },
            value_nodes,
        ),
    ]);

    row(
        PropsData {
            max_width: close_origin(&panel, tier).0 - COMPACT_CLOSE_MARGIN,
            ..PropsData::default()
        },
        kids,
    )
}

fn compact_brightness_row(icons: ControlIcons, brightness: u8) -> TreeNode {
    row(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        [
            slider(BRIGHTNESS_SLIDER_KEY, brightness_fraction(brightness)),
            fixed_width(COMPACT_GAP),
            svg_icon(icons.brightness_high, BRIGHTNESS_ICON_SIZE, TRANSPARENT),
        ],
    )
}

/// The compact control buttons, singles only. Brightness moved to the slider
/// above, and no product on this layout has sound, so the ± pairs are empty.
fn compact_control_rows(content: Content<'_>, tier: Tier) -> Vec<TreeNode> {
    let (_, singles) = control_groups(
        tier,
        &content.controls,
        content.notice,
        content.control_icons,
        content.icons,
        content.wifi_button,
    );
    control_rows(tier, Vec::new(), singles)
}

/// The BMM101 flow children: address block, the brightness slider where
/// the capability allows one, then whichever buttons the product still has.
pub(super) fn compact_children(content: Content<'_>, panel: Panel, tier: Tier) -> Vec<TreeNode> {
    let mut section = vec![
        // Top padding is an explicit spacer, not container padding,
        // so the close button's absolute insets resolve against the panel box.
        fixed_height(COMPACT_GAP),
        compact_info_row(content, panel, tier),
        fixed_height(COMPACT_GAP),
    ];
    if let Some(brightness) = content.controls.brightness {
        section.push(pad_horizontal(
            compact_brightness_row(content.control_icons, brightness),
            COMPACT_GAP,
        ));
        section.push(fixed_height(COMPACT_GAP));
    }
    let mut children = vec![
        content
            .notice
            .with_notice(PropsData::default(), NOTICE_SIZE, section),
    ];
    children.extend(compact_control_rows(content, tier));
    children
}

#[cfg(test)]
mod tests {
    use bmc_render::tree::TreeNode;
    use bmc_wasm_protocol::colors::GREEN_50;
    use bmc_wasm_protocol::{FontWeight, SvgId};

    use super::*;
    use crate::ui::test_support::*;
    use crate::ui::{ControlIcons, Controls, build_tree};

    /// BMM101 as it ships: no reconfigure button,
    /// whose glyph would add a second WiFi icon to the tree.
    fn bmm101_panel() -> Panel {
        Panel {
            wifi_button: false,
            ..narrow_panel()
        }
    }

    fn signal_icons_drawn(tree: &TreeNode) -> Vec<SvgId> {
        let icons = distinct_icons();
        let signal = [icons.problem, icons.low, icons.fair, icons.strong];
        let mut ids = Vec::new();
        svg_ids(tree, &mut ids);
        ids.retain(|id| signal.contains(&Some(*id)));
        ids
    }

    fn tray(wifi_signal: Option<i32>, view: WifiView<'_>) -> TreeNode {
        build_tree(
            Some("braiins-mini"),
            Some("10.0.0.42"),
            wifi_signal,
            Some("Workshop-WiFi"),
            distinct_icons(),
            bmm101_panel(),
            view,
            ControlIcons::default(),
            all_controls(),
            None,
        )
    }

    /// Like the pairs on the other layouts, the slider follows the capability;
    /// without it there is nothing for the slider to set.
    #[test]
    fn the_slider_goes_with_the_brightness_capability() {
        let has_slider = |controls: Controls<'_>| {
            touch_keys(&build_with_controls(bmm101_panel(), controls, None))
                .iter()
                .any(|key| key == BRIGHTNESS_SLIDER_KEY)
        };
        assert!(has_slider(all_controls()));
        assert!(!has_slider(Controls {
            brightness: None,
            ..all_controls()
        }));
    }

    #[test]
    fn the_ssid_carries_the_icon_of_its_signal_band() {
        let icons = distinct_icons();
        for (dbm, expected) in [
            (Some(-52), icons.strong),
            (Some(-70), icons.fair),
            (Some(-80), icons.low),
            (None, icons.problem),
        ] {
            let drawn = signal_icons_drawn(&tray(dbm, WifiView::Idle));
            assert_eq!(
                drawn,
                vec![expected.expect("BUG: distinct icons are all set")],
                "{dbm:?}"
            );
        }
    }

    #[test]
    fn the_cable_takes_the_row_from_the_ssid() {
        let tree = tray(Some(-52), WifiView::Cable);
        let mut ids = Vec::new();
        svg_ids(&tree, &mut ids);
        let cable = distinct_icons()
            .cable
            .expect("BUG: distinct icons are all set");
        assert!(ids.contains(&cable), "the cable glyph must draw: {ids:?}");
        assert!(signal_icons_drawn(&tree).is_empty(), "{ids:?}");
        let mut texts = Vec::new();
        collect_texts(&tree, &mut texts);
        assert!(texts.iter().any(|t| t == "Connection"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Ethernet"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "Workshop-WiFi"), "{texts:?}");
    }

    #[test]
    /// The badge is what says setup mode, as on the Deck and the disc;
    /// the problem icon alone reads as a fault. The SSID stays off the row:
    /// the device-info screen names it in full.
    fn setup_mode_shows_the_problem_icon_and_the_badge_without_the_ssid() {
        let tree = tray(
            Some(-52),
            WifiView::Setup {
                ap_ssid: "Mini-Setup",
            },
        );
        assert_eq!(
            signal_icons_drawn(&tree),
            vec![
                distinct_icons()
                    .problem
                    .expect("BUG: distinct icons are all set")
            ],
            "the station's reading does not describe the setup AP"
        );
        let mut texts = Vec::new();
        collect_texts(&tree, &mut texts);
        assert!(texts.iter().any(|t| t == "Connection"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "SETUP"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "Mini-Setup"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "Workshop-WiFi"), "{texts:?}");
        let badge = style_of(&tree, "SETUP").expect("BUG: the badge is a paragraph");
        assert_eq!(
            (badge.size, badge.weight, badge.color),
            (COMPACT_INFO_SIZE, FontWeight::BOLD, GREEN_50),
            "bold green at the row's own size, so the row stays one line tall"
        );
    }
}
