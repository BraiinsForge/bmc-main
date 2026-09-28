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

//! The disc's bottom line: who the device is connected to,
//! or the setup AP while provisioning runs.

use super::parts::{setup_badge, text_style, wifi_icon};
use super::{Content, ROUND_WIFI_ICON_SIZE, ROUND_WIFI_TEXT_SIZE, WifiIcons, WifiView};
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

/// Icon and SSID. The address heads the panel instead,
/// so this line carries only who the device is connected to.
fn station_info(icons: WifiIcons, wifi_signal: Option<i32>, ssid: &str) -> TreeNode {
    centered_line(vec![
        wifi_icon(icons, wifi_signal, ROUND_WIFI_ICON_SIZE),
        ssid_text(ssid),
    ])
}

/// One centered line of icon, badge and SSID, as tall as the idle
/// line it replaces so the vertical budget holds either way.
fn setup_row(icons: WifiIcons, ap_ssid: &str) -> TreeNode {
    centered_line(vec![
        wifi_icon(icons, None, ROUND_WIFI_ICON_SIZE),
        setup_badge(ROUND_WIFI_TEXT_SIZE),
        ssid_text(ap_ssid),
    ])
}

/// The disc's bottom line: the station info, or the setup badge and AP SSID
/// while setup runs.
pub(super) fn station_line(content: Content<'_>) -> TreeNode {
    match content.wifi_view {
        WifiView::Setup { ap_ssid } => setup_row(content.icons, ap_ssid),
        WifiView::Idle | WifiView::Cable => {
            station_info(content.icons, content.wifi_signal, content.ssid)
        }
    }
}
