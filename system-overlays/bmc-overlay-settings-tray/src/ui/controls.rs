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

#[cfg(test)]
mod tests {
    use bmc_render::tree::DrawCommand;
    use bmc_wasm_protocol::colors::{GRAY_50, TRANSPARENT, WHITE};
    use bmc_wasm_protocol::{Color, Fill};

    use super::*;
    use crate::ui::test_support::*;
    use crate::ui::*;

    /// BMM101 is wide enough to caption its buttons. BMM100 is not, and the
    /// disc spends its room on the chord-safe band.
    #[test]
    fn only_the_labeled_tiers_caption_their_buttons() {
        for (panel, labeled) in [
            (wide_panel(), true),
            (narrow_panel(), true),
            (small_panel(), false),
            (round_panel(), false),
        ] {
            let mut texts = Vec::new();
            collect_texts(&build_with_controls(panel, all_controls()), &mut texts);
            assert_eq!(
                texts.iter().any(|t| t == "Restart"),
                labeled,
                "{panel:?}: the restart button's own label"
            );
            assert_eq!(
                texts.iter().any(|t| t == "Reconfigure Wi-Fi"),
                labeled,
                "{panel:?}: the WiFi button's own label"
            );
        }
    }

    /// Labels render verbatim, so the widest must fit its group.
    /// A wrapped label costs a line the 320px stack cannot spare,
    /// which is why BMM101 captions at 12pt and not at 14.
    ///
    /// The Deck goes unchecked: the same estimate says its 20pt label
    /// overruns 180px, and it ships that way, so either the glyphs are
    /// narrower than the estimate or the label wraps and nobody minds.
    #[test]
    fn the_widest_label_fits_a_bmm101_group() {
        let tier = tier_for(&narrow_panel());
        let width = line_width("Reconfigure Wi-Fi", tier.caption_size);
        assert!(
            width <= tier.single_w,
            "the label needs {width} of {}",
            tier.single_w
        );
    }

    #[test]
    fn gating_decides_which_buttons_exist() {
        let keys = |panel: Panel, view: WifiView<'_>, controls: Controls<'_>| {
            let mut out = Vec::new();
            canvas_keys(
                &build_tree(
                    Some("braiins-deck"),
                    Some("10.0.0.2"),
                    Some(-55),
                    Some("MyWifi"),
                    WifiIcons::default(),
                    panel,
                    view,
                    ControlIcons::default(),
                    controls,
                ),
                &mut out,
            );
            out
        };

        let minimal = keys(wide_panel(), WifiView::Idle, Controls::default());
        assert!(
            minimal.iter().any(|k| k == CLOSE_KEY),
            "{CLOSE_KEY} always renders"
        );
        for key in [
            BRIGHTNESS_DOWN_KEY,
            BRIGHTNESS_UP_KEY,
            VOLUME_DOWN_KEY,
            VOLUME_UP_KEY,
            NIGHT_MODE_KEY,
            RESTART_KEY,
        ] {
            assert!(!minimal.iter().any(|k| k == key), "{key} is gated off");
        }
        assert!(
            minimal.iter().any(|k| k == WIFI_RECONFIG_KEY),
            "wifi button renders when the panel supports it"
        );

        let full = keys(wide_panel(), WifiView::Idle, all_controls());
        for key in [
            BRIGHTNESS_DOWN_KEY,
            BRIGHTNESS_UP_KEY,
            VOLUME_DOWN_KEY,
            VOLUME_UP_KEY,
            NIGHT_MODE_KEY,
            RESTART_KEY,
            WIFI_RECONFIG_KEY,
            CLOSE_KEY,
        ] {
            assert!(full.iter().any(|k| k == key), "{key} renders when enabled");
        }

        let mut no_wifi_panel = wide_panel();
        no_wifi_panel.wifi_button = false;
        let no_wifi = keys(no_wifi_panel, WifiView::Idle, all_controls());
        assert!(!no_wifi.iter().any(|k| k == WIFI_RECONFIG_KEY));

        let setup = keys(
            wide_panel(),
            WifiView::Setup {
                ap_ssid: "Deck ABCD",
            },
            all_controls(),
        );
        assert!(!setup.iter().any(|k| k == WIFI_RECONFIG_KEY));
        assert!(setup.iter().any(|k| k == CLOSE_KEY));
    }

    /// The night button's circle fill and icon tint.
    fn night_colors(controls: Controls<'_>) -> (Color, Color) {
        let tree = build_with_controls(wide_panel(), controls);
        let draws = find_canvas(&tree, NIGHT_MODE_KEY).expect("BUG: night canvas must exist");
        let DrawCommand::Circle {
            fill: Fill::Solid(fill),
            ..
        } = draws[0]
        else {
            panic!("expected Circle")
        };
        let DrawCommand::Svg { color, .. } = draws[draws.len() - 1] else {
            panic!("expected Svg")
        };
        (fill, color)
    }

    #[test]
    fn night_mode_active_fills_blue_and_never_inverts() {
        let night = |active| Controls {
            night_mode: Some(NightMode {
                active,
                until: Some("06:30"),
            }),
            ..Controls::default()
        };
        assert_eq!(night_colors(night(true)), (NIGHT_ACTIVE, TRANSPARENT));
        assert_eq!(night_colors(night(false)), (CIRCLE_FILL, TRANSPARENT));

        let pressed_active = Controls {
            pressed: Some(NIGHT_MODE_KEY),
            ..night(true)
        };
        assert_eq!(
            night_colors(pressed_active),
            (NIGHT_ACTIVE, TRANSPARENT),
            "a pressed active night button must not invert"
        );
        let pressed_inactive = Controls {
            pressed: Some(NIGHT_MODE_KEY),
            ..night(false)
        };
        assert_eq!(
            night_colors(pressed_inactive),
            (CIRCLE_FILL, TRANSPARENT),
            "a pressed inactive night button must not invert either"
        );
    }

    #[test]
    fn pressed_step_button_inverts() {
        let controls = Controls {
            volume: Some(40),
            pressed: Some(VOLUME_UP_KEY),
            ..Controls::default()
        };
        let tree = build_with_controls(wide_panel(), controls);
        let draws = find_canvas(&tree, VOLUME_UP_KEY).expect("BUG: volume-up canvas must exist");
        let DrawCommand::Circle {
            fill: Fill::Solid(fill),
            ..
        } = draws[0]
        else {
            panic!("expected Circle")
        };
        assert_eq!(fill, CIRCLE_PRESSED);
        let DrawCommand::Svg { color, .. } = draws[draws.len() - 1] else {
            panic!("expected Svg")
        };
        assert_eq!(color, ICON_PRESSED_TINT);

        let unpressed =
            find_canvas(&tree, VOLUME_DOWN_KEY).expect("BUG: volume-down canvas must exist");
        let DrawCommand::Circle {
            fill: Fill::Solid(fill),
            ..
        } = unpressed[0]
        else {
            panic!("expected Circle")
        };
        assert_eq!(fill, CIRCLE_FILL, "only the pressed button inverts");
    }

    #[test]
    fn large_tier_hold_hint_stays_legible_over_the_progress_circle() {
        let controls = Controls {
            restart: Some(HoldControl {
                caption: None,
                progress: 0.15,
            }),
            ..Controls::default()
        };
        assert_eq!(
            text_color(
                &build_with_controls(wide_panel(), controls),
                "hold 5 seconds",
            ),
            Some(WHITE),
        );

        let resting = Controls {
            restart: Some(HoldControl::default()),
            ..Controls::default()
        };
        assert_eq!(
            text_color(
                &build_with_controls(wide_panel(), resting),
                "hold 5 seconds",
            ),
            Some(GRAY_50),
        );
    }

    /// The disc is the only layout left with a caption line. The wide one
    /// carries the same copy in its labeled groups, and the compact one
    /// renders bare buttons with nothing beneath them.
    #[test]
    fn caption_precedence_and_prefixes() {
        let caption_texts = |panel: Panel, controls: Controls<'_>| {
            let mut texts = Vec::new();
            collect_texts(&build_with_controls(panel, controls), &mut texts);
            texts
        };
        let holding = HoldControl {
            caption: Some("Keep holding…"),
            progress: 0.2,
        };

        let all = Controls {
            restart: Some(holding),
            wifi_reconfig: holding,
            ..Controls::default()
        };
        let all_texts = caption_texts(round_panel(), all);
        assert!(
            all_texts.iter().any(|t| t == "Restart: Keep holding…"),
            "restart beats the wifi caption"
        );
        assert!(
            !all_texts
                .iter()
                .any(|t| t.starts_with("Reconfigure Wi-Fi:")),
            "the losing caption must not render alongside the winner"
        );

        let wifi_only = Controls {
            wifi_reconfig: holding,
            ..Controls::default()
        };
        assert!(
            caption_texts(round_panel(), wifi_only)
                .iter()
                .any(|t| t == "Reconfigure Wi-Fi: Keep holding…"),
            "reconfigure surfaces its own caption when it is the only hold"
        );

        let night = Controls {
            night_mode: Some(NightMode {
                active: true,
                until: Some("22:00"),
            }),
            ..Controls::default()
        };
        assert!(
            caption_texts(round_panel(), night)
                .iter()
                .any(|t| t == "Night mode on until 22:00"),
            "the round layout surfaces the night end time on the caption line"
        );
        assert!(
            !caption_texts(wide_panel(), night)
                .iter()
                .any(|t| t.starts_with("Night mode on until")),
            "the Large tier shows the end time in the night group instead"
        );
        assert!(
            caption_texts(wide_panel(), night)
                .iter()
                .any(|t| t == "Until 22:00")
        );
    }

    fn assert_control_rows_fit(controls: Controls<'_>) {
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            let tree = build_tree(
                Some("braiins-deck"),
                Some("10.0.0.2"),
                Some(-55),
                Some("MyWifi"),
                WifiIcons::default(),
                panel,
                WifiView::Idle,
                ControlIcons::default(),
                controls,
            );
            #[expect(clippy::cast_precision_loss, reason = "panel sizes are small")]
            let panel_w = panel.width as f32;
            let kids = children(&tree).expect("BUG: root must be a container");

            let mut rows = 0;
            for kid in kids {
                let mut keys = Vec::new();
                canvas_keys(kid, &mut keys);
                if !keys.iter().any(|k| is_control_key(k)) {
                    continue;
                }
                rows += 1;
                let width = min_content_width(kid);
                assert!(
                    width <= panel_w,
                    "{panel:?}: control row of {width} overflows {panel_w} — \
                     the buttons run off the panel edge"
                );
            }
            assert!(rows > 0, "{panel:?}: control rows must render");
        }
    }

    #[test]
    fn control_rows_fit_the_panel_width() {
        assert_control_rows_fit(all_controls());
        let held = HoldControl {
            caption: Some("Keep holding…"),
            progress: 0.5,
        };
        assert_control_rows_fit(Controls {
            restart: Some(held),
            wifi_reconfig: held,
            ..all_controls()
        });
    }
}
