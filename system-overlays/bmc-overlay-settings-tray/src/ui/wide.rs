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

use super::notice::Notice;
use super::parts::{
    capped_text, fixed_width, ip_qr, pad_horizontal, setup_badge, text_style, wifi_icon,
};
use super::{
    Content, INFO_HEADER_GAP, INFO_HEADER_SIZE, NO_DATA_PLACEHOLDER, Tier, WIDE_HOSTNAME_WIDTH,
    WIDE_INFO_GAP, WIDE_INFO_LEFT_PAD, WIDE_INFO_RIGHT_PAD, WIDE_INFO_STACK_GAP,
    WIDE_INFO_VALUE_SIZE, WIDE_NOTICE_SIZE, WIDE_QR_SIZE, WIDE_ROW_GAP, WIDE_SETUP_BADGE_SIZE,
    WIDE_SSID_WIDTH, WIDE_TOP_PAD, WIDE_WIFI_GAP, WIDE_WIFI_ICON_SIZE, WifiIcons, WifiView,
    control_row_nodes,
};
use bmc_render::tree::{PropsData, TreeNode, col, fixed_height, row, spacer, text};
use bmc_wasm_protocol::CrossAlign;
use bmc_wasm_protocol::colors::{GRAY_50, WHITE};

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
    wifi_view: WifiView<'_>,
) -> TreeNode {
    let value_size = WIDE_INFO_VALUE_SIZE;
    let wifi_block = match wifi_view {
        WifiView::Idle | WifiView::Cable => info_block(
            "Wi-Fi Connection",
            row(
                PropsData {
                    cross_align: CrossAlign::Center,
                    gap: WIDE_WIFI_GAP,
                    ..PropsData::default()
                },
                vec![
                    wifi_icon(icons, wifi_signal, WIDE_WIFI_ICON_SIZE),
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
                            wifi_icon(icons, None, WIDE_WIFI_ICON_SIZE),
                            setup_badge(WIDE_SETUP_BADGE_SIZE),
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
/// top half holds the info header and, while one is up, the notice over it;
/// the bottom half the control rows.
fn wide_halves(
    header: TreeNode,
    rows: Vec<TreeNode>,
    h_pad: f32,
    notice: Notice<'_>,
) -> [TreeNode; 2] {
    let half = PropsData {
        flex: 1.0,
        ..PropsData::default()
    };
    let mut bottom_half: Vec<TreeNode> = Vec::new();
    for row_node in rows {
        bottom_half.push(pad_horizontal(row_node, h_pad));
        bottom_half.push(fixed_height(WIDE_ROW_GAP));
    }
    [
        notice.with_notice(
            half,
            WIDE_NOTICE_SIZE,
            vec![fixed_height(WIDE_TOP_PAD), header],
        ),
        col(half, bottom_half),
    ]
}

/// The Deck's flow children: the info header in the top half,
/// the labeled control row in the bottom half.
pub(super) fn wide_children(content: Content<'_>, tier: Tier) -> Vec<TreeNode> {
    let header = wide_header(
        content.hostname.unwrap_or("N/A"),
        content.ip,
        content.icons,
        content.wifi_signal,
        content.ssid,
        content.wifi_view,
    );
    let rows = control_row_nodes(content, tier);
    wide_halves(header, rows, tier.padding, content.notice).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::*;

    /// Min-content width of [`wide_header`] at its caps. The structure
    /// is worst-case; the glyph width is only [`HOSTNAME_CHAR_W`]'s estimate.
    /// Including the SETUP badge bounds both WiFi views, leaving idle mode —
    /// which has no badge — some slack. The setup hint is left out: it wraps,
    /// so a single-line glyph budget does not describe it.
    #[expect(clippy::cast_precision_loss, reason = "the caps are a few hundred px")]
    fn wide_info_width() -> f32 {
        let addresses =
            line_width("255.255.255.255", WIDE_INFO_VALUE_SIZE).max(WIDE_HOSTNAME_WIDTH as f32);
        let wifi = WIDE_WIFI_ICON_SIZE
            + WIDE_WIFI_GAP
            + line_width("SETUP", WIDE_SETUP_BADGE_SIZE)
            + WIDE_WIFI_GAP
            + WIDE_SSID_WIDTH as f32;
        WIDE_INFO_LEFT_PAD + WIDE_QR_SIZE + WIDE_INFO_GAP + addresses + wifi + WIDE_INFO_RIGHT_PAD
    }

    #[test]
    fn wide_header_fits_the_panel_width() {
        let panel = wide_panel();
        #[expect(clippy::cast_precision_loss, reason = "panel sizes are small")]
        let panel_w = panel.width as f32;
        let width = wide_info_width();
        assert!(
            width <= panel_w,
            "header content {width} overflows {panel_w} — the flex spacer \
             collapses and the WiFi block runs off the right edge"
        );
    }
}
