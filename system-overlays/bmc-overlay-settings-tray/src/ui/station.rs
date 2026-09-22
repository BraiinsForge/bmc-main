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

use super::parts::{text_style, wifi_icon};
use super::{Content, Tier, WifiIcons, WifiView};
use bmc_render::tree::{PropsData, TextStyle, TreeNode, row, text};
use bmc_wasm_protocol::colors::{GREEN_50, WHITE};
use bmc_wasm_protocol::{CrossAlign, FontWeight, Justify, TextOverflow};

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

fn ssid_text(ssid: &str, size: u32) -> TreeNode {
    text(
        ssid,
        TextStyle {
            text_overflow: TextOverflow::Ellipsis,
            ..text_style(size, WHITE)
        },
    )
}

/// Icon and SSID. The address heads the panel instead,
/// so this line carries only who the device is connected to.
fn station_info(icons: WifiIcons, wifi_signal: Option<i32>, ssid: &str, tier: Tier) -> TreeNode {
    centered_line(vec![
        wifi_icon(icons, wifi_signal, tier.wifi_icon_size),
        ssid_text(ssid, tier.wifi_text_size),
    ])
}

/// Setup-mode section for the medium/small tiers: icon, badge and SSID,
/// occupying the same height the idle info line does, so the vertical budgets hold.
/// The Large tier shows setup mode inside [`wide_header`] instead.
fn setup_row(icons: WifiIcons, ap_ssid: &str, tier: Tier) -> TreeNode {
    let badge = text(
        "SETUP",
        TextStyle {
            size: tier.wifi_text_size,
            weight: FontWeight::BOLD,
            color: GREEN_50,
            ..TextStyle::default()
        },
    );
    centered_line(vec![
        wifi_icon(icons, None, tier.wifi_icon_size),
        badge,
        ssid_text(ap_ssid, tier.wifi_text_size),
    ])
}

/// The disc's bottom line: the station info, or the setup badge and AP SSID
/// while setup runs.
pub(super) fn station_line(content: Content<'_>, tier: Tier) -> TreeNode {
    match content.wifi_view {
        WifiView::Setup { ap_ssid } => setup_row(content.icons, ap_ssid, tier),
        WifiView::Idle => station_info(content.icons, content.wifi_signal, content.ssid, tier),
    }
}
