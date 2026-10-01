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

//! The disc's bottom line: what the device is online through,
//! or the setup AP while provisioning runs.

use super::parts::{ETHERNET, connection_icon, setup_badge, text_style};
use super::{Content, ROUND_WIFI_ICON_SIZE, ROUND_WIFI_TEXT_SIZE, WifiView};
use bmc_render::tree::{PropsData, TextStyle, TreeNode, row, text};
use bmc_wasm_protocol::colors::WHITE;
use bmc_wasm_protocol::{CrossAlign, Justify, TextOverflow};

/// One centered line of the disc's station info,
/// stretched across its column so only the SSID gives way when it runs out.
fn centered_line(children: Vec<TreeNode>) -> TreeNode {
    row(
        PropsData {
            cross_align: CrossAlign::Center,
            justify_content: Justify::Center,
            gap: 12.0,
            ..PropsData::default()
        },
        children,
    )
}

fn ssid_text(ssid: &str) -> TreeNode {
    text(
        ssid,
        TextStyle {
            text_overflow: TextOverflow::Ellipsis,
            ..text_style(ROUND_WIFI_TEXT_SIZE, WHITE)
        },
    )
}

/// The disc's bottom line. The address heads the panel instead,
/// so this line carries only what the device is online through.
/// Every variant is one line tall, so the vertical budget holds either way.
pub(super) fn station_line(content: Content<'_>) -> TreeNode {
    let icon = connection_icon(
        content.wifi_view,
        content.icons,
        content.wifi_signal,
        ROUND_WIFI_ICON_SIZE,
    );
    centered_line(match content.wifi_view {
        WifiView::Cable => vec![icon, ssid_text(ETHERNET)],
        WifiView::Idle => vec![icon, ssid_text(content.ssid)],
        WifiView::Setup { ap_ssid } => {
            vec![icon, setup_badge(ROUND_WIFI_TEXT_SIZE), ssid_text(ap_ssid)]
        }
    })
}
