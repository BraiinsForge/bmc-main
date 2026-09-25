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

//! The BMM100 and BMM101 arrangement: the address block,
//! then the brightness slider, then bare buttons.

use super::controls::{control_groups, control_rows};
use super::notice::NOTICE_SIZE;
use super::parts::{close_origin, fixed_width, ip_qr, pad_horizontal, text_style};
use super::{
    BRIGHTNESS_ICON_SIZE, BRIGHTNESS_SLIDER_KEY, COMPACT_CLOSE_MARGIN, COMPACT_GAP,
    COMPACT_INFO_SIZE, COMPACT_QR_SIZE, Content, ControlIcons, NO_DATA_PLACEHOLDER, Panel,
    SLIDER_TRACK_H, Tier, brightness_fraction,
};
use bmc_render::tree::{DrawCommand, PropsData, TextStyle, TreeNode, col, fixed_height, row, text};
use bmc_wasm_protocol::colors::{GRAY_40, TRANSPARENT, WHITE};
use bmc_wasm_protocol::{CrossAlign, ProgressKind, TextAlign, TextOverflow};

/// The icon naming the brightness slider; the slider owns the touch key.
fn brightness_icon(icons: ControlIcons) -> TreeNode {
    TreeNode::Canvas {
        props: PropsData {
            width: BRIGHTNESS_ICON_SIZE,
            height: BRIGHTNESS_ICON_SIZE,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Svg {
            x: 0.0,
            y: 0.0,
            w: BRIGHTNESS_ICON_SIZE,
            h: BRIGHTNESS_ICON_SIZE,
            color: TRANSPARENT,
            icon_id: icons.brightness_high,
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

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

/// One column of the compact address table, evenly spaced.
fn compact_info_column(props: PropsData, lines: [&str; 3], style: TextStyle) -> TreeNode {
    let mut kids = Vec::new();
    for line in lines {
        if !kids.is_empty() {
            kids.push(fixed_height(COMPACT_GAP));
        }
        kids.push(text(line, style));
    }
    col(props, kids)
}

/// The compact address block. Both columns come from one list, so a row
/// cannot appear on one side alone and slide the values out of line.
/// Capped short of the close target floating over its top-right corner.
fn compact_info_row(content: Content<'_>, panel: Panel, tier: Tier) -> TreeNode {
    let rows = [
        ("Hostname", content.hostname.unwrap_or(NO_DATA_PLACEHOLDER)),
        ("IP Address", content.ip.unwrap_or(NO_DATA_PLACEHOLDER)),
        ("WiFi SSID", content.ssid),
    ];
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
    kids.extend([
        compact_info_column(
            PropsData::default(),
            rows.map(|(label, _)| label),
            label_style,
        ),
        fixed_width(COMPACT_GAP),
        // Grown from zero rather than shrunk from its content, so an overlong value
        // gives way inside its own column and never squeezes a label into wrapping.
        compact_info_column(
            PropsData {
                flex: 1.0,
                ..PropsData::default()
            },
            rows.map(|(_, value)| value),
            value_style,
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

fn compact_brightness_row(content: Content<'_>) -> TreeNode {
    let fraction = content.controls.brightness.map_or(0.0, brightness_fraction);
    row(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        [
            slider(BRIGHTNESS_SLIDER_KEY, fraction),
            fixed_width(COMPACT_GAP),
            brightness_icon(content.control_icons),
        ],
    )
}

/// The compact control buttons, singles only. Brightness moved to the slider
/// above, and no product on this layout has sound, so the ± pairs are empty.
fn compact_control_rows(content: Content<'_>, tier: Tier) -> Vec<TreeNode> {
    let (_, singles) = control_groups(
        tier,
        &content.controls,
        content.control_icons,
        content.icons,
        content.wifi_button,
    );
    control_rows(tier, Vec::new(), singles)
}

/// The BMM100 and BMM101 flow children: address block, brightness slider,
/// then whichever buttons the product still has.
pub(super) fn compact_children(content: Content<'_>, panel: Panel, tier: Tier) -> Vec<TreeNode> {
    let mut children = vec![content.notice.with_notice(
        PropsData::default(),
        NOTICE_SIZE,
        vec![
            // Top padding is an explicit spacer, not container padding,
            // so the close button's absolute insets resolve against the panel box.
            fixed_height(COMPACT_GAP),
            compact_info_row(content, panel, tier),
            fixed_height(COMPACT_GAP),
            pad_horizontal(compact_brightness_row(content), COMPACT_GAP),
            fixed_height(COMPACT_GAP),
        ],
    )];
    children.extend(compact_control_rows(content, tier));
    children
}
