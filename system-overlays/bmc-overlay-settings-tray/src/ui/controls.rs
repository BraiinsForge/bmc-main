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

//! The control buttons: the plus/minus pairs, the single hold buttons,
//! and the caption line they share.

use super::parts::{ButtonIcon, press_fill, press_tint, round_button};
use super::{
    BRIGHTNESS_DOWN_KEY, BRIGHTNESS_UP_KEY, CIRCLE_FILL, ControlIcons, Controls, NIGHT_ACTIVE,
    NIGHT_MODE_KEY, RESTART_KEY, Tier, VOLUME_DOWN_KEY, VOLUME_UP_KEY, WIFI_RECONFIG_KEY,
    WifiIcons,
};
use bmc_render::tree::{PropsData, TextStyle, TreeNode, col, fixed_height, row, text};
use bmc_wasm_protocol::colors::{GRAY_50, TRANSPARENT, WHITE};
use bmc_wasm_protocol::{Color, CrossAlign, FontWeight, SvgId, TextAlign, TextOverflow};

/// A ±step pair (volume / brightness) with its value text below. On the Large
/// tier the block is a fixed-width column with bold value + gray name.
#[expect(
    clippy::too_many_arguments,
    reason = "flat display fields, same as build_tree"
)]
fn pair_group(
    tier: Tier,
    down_key: &'static str,
    up_key: &'static str,
    low_icon: Option<SvgId>,
    high_icon: Option<SvgId>,
    value: u8,
    name: &str,
    pressed: Option<&str>,
) -> TreeNode {
    let btn = |key: &'static str, icon: Option<SvgId>| {
        let p = pressed == Some(key);
        round_button(
            key,
            ButtonIcon::square(icon),
            tier.circle,
            tier.icon,
            press_fill(p),
            press_tint(p),
            None,
        )
    };
    let buttons = row(
        PropsData {
            gap: tier.pair_gap,
            ..PropsData::default()
        },
        vec![btn(down_key, low_icon), btn(up_key, high_icon)],
    );
    let mut kids = vec![buttons, fixed_height(if tier.labeled { 8.0 } else { 2.0 })];
    if tier.labeled {
        kids.push(text(
            format!("{value}"),
            TextStyle {
                size: tier.value_size,
                weight: FontWeight::BOLD,
                color: WHITE,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        ));
        kids.push(text(
            name,
            TextStyle {
                size: tier.caption_size,
                color: GRAY_50,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        ));
    } else {
        kids.push(text(
            format!("{value}"),
            TextStyle {
                size: tier.value_size,
                color: WHITE,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        ));
    }
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            width: if tier.labeled { tier.pair_w } else { 0.0 },
            ..PropsData::default()
        },
        kids,
    )
}

/// A single-button group. Large tier adds the fixed-width label/sublabel
/// block; other tiers render the bare button.
#[expect(
    clippy::too_many_arguments,
    reason = "flat display fields, same as build_tree"
)]
fn single_group(
    tier: Tier,
    key: &'static str,
    icon: ButtonIcon,
    fill: Color,
    tint: Color,
    hold_progress: Option<f32>,
    label: &str,
    sublabel: &str,
) -> TreeNode {
    let btn = round_button(key, icon, tier.circle, tier.icon, fill, tint, hold_progress);
    if !tier.labeled {
        return btn;
    }
    // The label/sublabel copy is fixed at compile time ("Night Mode: Off",
    // "hold 5 seconds", …) and sized to its column, so it is never cut.
    let mut kids = vec![btn, fixed_height(8.0)];
    kids.push(text(
        label,
        TextStyle {
            size: tier.caption_size,
            weight: FontWeight::BOLD,
            color: WHITE,
            align: TextAlign::Center,
            ..TextStyle::default()
        },
    ));
    if !sublabel.is_empty() {
        kids.push(text(
            sublabel,
            TextStyle {
                size: tier.caption_size,
                color: if hold_progress.is_some_and(|p| p > 0.0) {
                    WHITE
                } else {
                    GRAY_50
                },
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        ));
    }
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            width: tier.single_w,
            ..PropsData::default()
        },
        kids,
    )
}

/// All control groups in spec order, split into the ± pair groups
/// (volume/brightness) and the single-button groups. On the Large tier the
/// two halves concatenate into one row; medium/small render them as two rows.
/// `wifi` is true only when the WiFi button applies (product/caps gate, not
/// in setup mode).
pub(super) fn control_groups(
    tier: Tier,
    controls: &Controls<'_>,
    icons: ControlIcons,
    wifi_icons: WifiIcons,
    wifi: bool,
) -> (Vec<TreeNode>, Vec<TreeNode>) {
    let mut pairs = Vec::new();
    if let Some(v) = controls.volume {
        pairs.push(pair_group(
            tier,
            VOLUME_DOWN_KEY,
            VOLUME_UP_KEY,
            icons.sound_low,
            icons.sound_high,
            v,
            "Volume",
            controls.pressed,
        ));
    }
    if let Some(b) = controls.brightness {
        pairs.push(pair_group(
            tier,
            BRIGHTNESS_DOWN_KEY,
            BRIGHTNESS_UP_KEY,
            icons.brightness_low,
            icons.brightness_high,
            b,
            "Brightness",
            controls.pressed,
        ));
    }

    let mut singles = Vec::new();
    if let Some(night) = controls.night_mode {
        // The boundary reads as "current state lasts until HH:MM" in both
        // states: the end of the night window while active, its next start
        // while inactive (absent when the schedule is disabled).
        let sublabel = night
            .until
            .map_or_else(String::new, |until| format!("Until {until}"));
        singles.push(single_group(
            tier,
            NIGHT_MODE_KEY,
            ButtonIcon {
                id: icons.night_mode,
                aspect: icons.night_mode_aspect,
            },
            if night.active {
                NIGHT_ACTIVE
            } else {
                CIRCLE_FILL
            },
            TRANSPARENT,
            None,
            if night.active {
                "Night Mode: On"
            } else {
                "Night Mode: Off"
            },
            &sublabel,
        ));
    }
    if let Some(restart) = controls.restart {
        let p = controls.pressed == Some(RESTART_KEY);
        singles.push(single_group(
            tier,
            RESTART_KEY,
            ButtonIcon::square(icons.restart),
            press_fill(p),
            press_tint(p),
            Some(restart.progress),
            "Restart",
            "hold 5 seconds",
        ));
    }
    if wifi {
        let p = controls.pressed == Some(WIFI_RECONFIG_KEY);
        singles.push(single_group(
            tier,
            WIFI_RECONFIG_KEY,
            ButtonIcon::square(wifi_icons.problem),
            press_fill(p),
            press_tint(p),
            Some(controls.wifi_reconfig.progress),
            "Reconfigure Wi-Fi",
            "hold 5 seconds",
        ));
    }
    (pairs, singles)
}

/// The control row nodes: one row (pairs + singles) on the Large tier, two
/// rows ([pairs], [singles]) on medium/small, each row centered. Empty rows
/// are dropped entirely.
pub(super) fn control_rows(
    tier: Tier,
    pairs: Vec<TreeNode>,
    singles: Vec<TreeNode>,
) -> Vec<TreeNode> {
    let centered = |groups: Vec<TreeNode>| {
        col(
            PropsData {
                cross_align: CrossAlign::Center,
                ..PropsData::default()
            },
            vec![row(
                PropsData {
                    gap: tier.group_gap,
                    cross_align: CrossAlign::Start,
                    ..PropsData::default()
                },
                groups,
            )],
        )
    };
    if tier.labeled {
        let mut groups = pairs;
        groups.extend(singles);
        if groups.is_empty() {
            return Vec::new();
        }
        return vec![centered(groups)];
    }
    [pairs, singles]
        .into_iter()
        .filter(|groups| !groups.is_empty())
        .map(centered)
        .collect()
}

/// Dynamic status line under the control rows. Precedence: restart >
/// reconfigure > night-mode-until (medium/small only) > none.
/// Prefixed with the control name so unlabeled small-tier buttons stay
/// attributable.
pub(super) fn shared_caption(tier: Tier, controls: &Controls<'_>) -> Option<TreeNode> {
    let raw = if let Some(c) = controls.restart.and_then(|r| r.caption) {
        Some(format!("Restart: {c}"))
    } else if let Some(c) = controls.wifi_reconfig.caption {
        Some(format!("Reconfigure Wi-Fi: {c}"))
    } else if !tier.labeled
        && let Some(n) = controls.night_mode
        && let Some(until) = n.until
    {
        Some(if n.active {
            format!("Night mode on until {until}")
        } else {
            format!("Night mode off until {until}")
        })
    } else {
        None
    };
    raw.map(|s| {
        col(
            PropsData {
                cross_align: CrossAlign::Center,
                ..PropsData::default()
            },
            vec![text(
                &s,
                TextStyle {
                    size: tier.caption_size,
                    color: GRAY_50,
                    align: TextAlign::Center,
                    text_overflow: TextOverflow::Ellipsis,
                    ..TextStyle::default()
                },
            )],
        )
    })
}
