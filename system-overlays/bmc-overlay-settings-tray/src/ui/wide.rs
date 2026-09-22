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

//! The Deck's arrangement: an info header over one labeled control row.

use super::parts::{capped_text, fixed_width, ip_qr, pad_horizontal, text_style, wifi_icon};
use super::{
    Content, INFO_HEADER_GAP, INFO_HEADER_SIZE, NO_DATA_PLACEHOLDER, Tier, WIDE_HOSTNAME_WIDTH,
    WIDE_INFO_GAP, WIDE_INFO_LEFT_PAD, WIDE_INFO_RIGHT_PAD, WIDE_INFO_STACK_GAP, WIDE_QR_SIZE,
    WIDE_SETUP_BADGE_SIZE, WIDE_SSID_WIDTH, WIDE_TOP_PAD, WIDE_WIFI_GAP, WifiIcons, WifiView,
    caption_slot, control_row_nodes,
};
use bmc_render::tree::{PropsData, TextStyle, TreeNode, col, fixed_height, row, spacer, text};
use bmc_wasm_protocol::colors::{GRAY_50, GREEN_50, WHITE};
use bmc_wasm_protocol::{CrossAlign, FontWeight};

/// One Large-tier info block: a small gray header over a white value node.
fn info_block(header: &'static str, value: TreeNode) -> TreeNode {
    col(
        PropsData {
            gap: INFO_HEADER_GAP,
            ..PropsData::default()
        },
        vec![text(header, text_style(INFO_HEADER_SIZE, GRAY_50)), value],
    )
}

/// The Large tier's top info section: the QR code in the left corner,
/// the IP address and hostname stacked beside it,
/// the WiFi connection block in the right corner, nothing in between —
/// each text pair a gray header over a 24px value.
/// In setup mode the WiFi block carries the SETUP badge, the AP SSID,
/// and the join hint instead of the station info.
pub(super) fn wide_header(
    hostname: &str,
    ip: Option<&str>,
    icons: WifiIcons,
    wifi_signal: Option<i32>,
    ssid: &str,
    tier: Tier,
    wifi_view: WifiView<'_>,
) -> TreeNode {
    let value_size = tier.hostname_size;
    let wifi_block = match wifi_view {
        WifiView::Idle => info_block(
            "Wi-Fi Connection",
            row(
                PropsData {
                    cross_align: CrossAlign::Center,
                    gap: WIDE_WIFI_GAP,
                    ..PropsData::default()
                },
                vec![
                    wifi_icon(icons, wifi_signal, tier.wifi_icon_size),
                    capped_text(ssid, value_size, WIDE_SSID_WIDTH),
                ],
            ),
        ),
        WifiView::Setup { ap_ssid } => info_block(
            "Wi-Fi Connection",
            col(
                PropsData {
                    gap: 6.0,
                    ..PropsData::default()
                },
                vec![
                    row(
                        PropsData {
                            cross_align: CrossAlign::Center,
                            gap: 12.0,
                            ..PropsData::default()
                        },
                        vec![
                            wifi_icon(icons, None, tier.wifi_icon_size),
                            text(
                                "SETUP",
                                TextStyle {
                                    size: WIDE_SETUP_BADGE_SIZE,
                                    weight: FontWeight::BOLD,
                                    color: GREEN_50,
                                    ..TextStyle::default()
                                },
                            ),
                            capped_text(ap_ssid, value_size, WIDE_SSID_WIDTH),
                        ],
                    ),
                    text(
                        "Join this network from your phone to reconfigure Wi-Fi.",
                        text_style(INFO_HEADER_SIZE, GRAY_50),
                    ),
                ],
            ),
        ),
    };

    let addresses = col(
        PropsData {
            gap: WIDE_INFO_STACK_GAP,
            ..PropsData::default()
        },
        vec![
            info_block(
                "IP Address",
                text(
                    ip.unwrap_or(NO_DATA_PLACEHOLDER),
                    text_style(value_size, WHITE),
                ),
            ),
            info_block(
                "Hostname",
                capped_text(hostname, value_size, WIDE_HOSTNAME_WIDTH),
            ),
        ],
    );
    let left_info = match ip {
        Some(ip) => vec![ip_qr(ip, WIDE_QR_SIZE), addresses],
        None => vec![addresses],
    };

    row(
        PropsData::default(),
        vec![
            fixed_width(WIDE_INFO_LEFT_PAD),
            row(
                PropsData {
                    gap: WIDE_INFO_GAP,
                    ..PropsData::default()
                },
                left_info,
            ),
            spacer(1.0),
            wifi_block,
            fixed_width(WIDE_INFO_RIGHT_PAD),
        ],
    )
}

/// The Large tier's flow children: two equal flex halves pin the control
/// block's top edge to the vertical middle, matching the stable design. The
/// top half holds the info header, the bottom half the control rows and the
/// shared caption.
fn wide_halves(
    header: TreeNode,
    rows: Vec<TreeNode>,
    caption_node: TreeNode,
    tier: Tier,
    h_pad: f32,
) -> [TreeNode; 2] {
    let half = PropsData {
        flex: 1.0,
        ..PropsData::default()
    };
    let mut bottom_half: Vec<TreeNode> = Vec::new();
    for row_node in rows {
        bottom_half.push(pad_horizontal(row_node, h_pad));
        bottom_half.push(fixed_height(tier.row_gap));
    }
    bottom_half.push(pad_horizontal(caption_node, h_pad));
    [
        col(half, vec![fixed_height(WIDE_TOP_PAD), header]),
        col(half, bottom_half),
    ]
}

/// The Deck's flow children: the info header in the top half,
/// the labeled control row and the caption in the bottom half.
pub(super) fn wide_children(content: Content<'_>, tier: Tier) -> Vec<TreeNode> {
    let header = wide_header(
        content.hostname.unwrap_or("N/A"),
        content.ip,
        content.icons,
        content.wifi_signal,
        content.ssid,
        tier,
        content.wifi_view,
    );
    let rows = control_row_nodes(content, tier);
    let caption = caption_slot(content, tier);
    wide_halves(header, rows, caption, tier, tier.padding).into()
}
