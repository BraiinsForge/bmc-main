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

//! The control buttons: the plus/minus pairs and the single hold buttons.

use super::notice::Notice;
use super::parts::{ButtonIcon, press_fill, press_tint, round_button};
use super::{
    Action, BRIGHTNESS_DOWN_KEY, BRIGHTNESS_UP_KEY, CIRCLE_FILL, ControlIcons, Controls,
    NIGHT_ACTIVE, NIGHT_MODE_KEY, RESTART_KEY, Tier, VOLUME_DOWN_KEY, VOLUME_UP_KEY,
    WIFI_RECONFIG_KEY, WifiIcons,
};
use bmc_render::tree::{PropsData, TextStyle, TreeNode, col, fixed_height, row, text};
use bmc_wasm_protocol::colors::{GRAY_50, TRANSPARENT, WHITE};
use bmc_wasm_protocol::{Color, CrossAlign, FontWeight, SvgId, TextAlign};

/// A ±step pair (volume / brightness) with its value text below.
/// With captions the block is a fixed-width column with bold value + gray name.
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
    let mut kids = vec![
        buttons,
        fixed_height(if tier.captions.is_some() { 8.0 } else { 2.0 }),
    ];
    if let Some(captions) = tier.captions {
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
                size: captions.size,
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
            width: tier.captions.map_or(0.0, |c| c.pair_w),
            ..PropsData::default()
        },
        kids,
    )
}

/// A single-button group. With captions it adds the fixed-width
/// label/sublabel block; without, it is the bare button.
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
    let Some(captions) = tier.captions else {
        return btn;
    };
    // The label/sublabel copy is fixed at compile time ("Night Mode: Off",
    // "hold 5 seconds", …) and sized to its column, so it is never cut.
    let mut kids = vec![btn, fixed_height(8.0)];
    kids.push(text(
        label,
        TextStyle {
            size: captions.size,
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
                size: captions.size,
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
            width: captions.single_w,
            ..PropsData::default()
        },
        kids,
    )
}

/// All control groups in spec order, split into the ± pair groups
/// (volume/brightness) and the single-button groups. On the Large tier
/// the two halves concatenate into one row; medium/small render them as two rows.
/// `wifi` is true only when the WiFi button applies (caps gate, not
/// in setup mode). While the notice is up, every group but the action's own
/// is dimmed and disabled.
pub(super) fn control_groups(
    tier: Tier,
    controls: &Controls<'_>,
    notice: Notice<'_>,
    icons: ControlIcons,
    wifi_icons: WifiIcons,
    wifi: bool,
) -> (Vec<TreeNode>, Vec<TreeNode>) {
    let mut pairs = Vec::new();
    if let Some(v) = controls.volume {
        pairs.push(notice.button(
            VOLUME_UP_KEY,
            pair_group(
                tier,
                VOLUME_DOWN_KEY,
                VOLUME_UP_KEY,
                icons.sound_low,
                icons.sound_high,
                v,
                "Volume",
                controls.pressed,
            ),
        ));
    }
    if let Some(b) = controls.brightness {
        pairs.push(notice.button(
            BRIGHTNESS_UP_KEY,
            pair_group(
                tier,
                BRIGHTNESS_DOWN_KEY,
                BRIGHTNESS_UP_KEY,
                icons.brightness_low,
                icons.brightness_high,
                b,
                "Brightness",
                controls.pressed,
            ),
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
        singles.push(notice.button(
            NIGHT_MODE_KEY,
            single_group(
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
            ),
        ));
    }
    if controls.restart {
        let p = controls.pressed == Some(RESTART_KEY);
        singles.push(notice.button(
            RESTART_KEY,
            single_group(
                tier,
                RESTART_KEY,
                ButtonIcon::square(icons.restart),
                press_fill(p),
                press_tint(p),
                Some(notice.hold_progress(Action::Restart)),
                "Restart",
                "hold 5 seconds",
            ),
        ));
    }
    if wifi {
        let p = controls.pressed == Some(WIFI_RECONFIG_KEY);
        singles.push(notice.button(
            WIFI_RECONFIG_KEY,
            single_group(
                tier,
                WIFI_RECONFIG_KEY,
                ButtonIcon::square(wifi_icons.problem),
                press_fill(p),
                press_tint(p),
                Some(notice.hold_progress(Action::WifiReconfig)),
                "Reconfigure Wi-Fi",
                "hold 5 seconds",
            ),
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
    if tier.captions.is_some() {
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

#[cfg(test)]
mod tests {
    use bmc_render::tree::DrawCommand;
    use bmc_wasm_protocol::colors::{GRAY_50, TRANSPARENT, WHITE};
    use bmc_wasm_protocol::{Color, Fill};

    use super::*;
    use crate::ui::notice::DIMMED;
    use crate::ui::test_support::*;
    use crate::ui::*;

    /// BMM101 is wide enough to caption its buttons.
    /// The disc spends its room on the chord-safe band.
    #[test]
    fn only_the_labeled_tiers_caption_their_buttons() {
        for (panel, labeled) in [
            (wide_panel(), true),
            (narrow_panel(), true),
            (round_panel(), false),
        ] {
            let mut texts = Vec::new();
            collect_texts(
                &build_with_controls(panel, all_controls(), None),
                &mut texts,
            );
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
        let captions = tier_for(&narrow_panel())
            .captions
            .expect("BUG: BMM101 captions its groups");
        let width = line_width("Reconfigure Wi-Fi", captions.size);
        assert!(
            width <= captions.single_w,
            "the label needs {width} of {}",
            captions.single_w
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
                    None,
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
        let tree = build_with_controls(wide_panel(), controls, None);
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

    /// While restart is held, every other button dims and stops taking
    /// touches; the held button stays lit and live, and so does close.
    #[test]
    fn the_notice_dims_and_disables_every_other_button() {
        for panel in [wide_panel(), narrow_panel(), round_panel()] {
            let tree = build_with_controls(panel, all_controls(), Some(&held_status()));
            let held = canvas_brightness(&tree, RESTART_KEY).expect("BUG: the held button renders");
            assert_close(held, 1.0, "the held button stays lit");
            assert_eq!(
                touch_keys(&tree),
                [CLOSE_KEY, RESTART_KEY],
                "{panel:?}: only the held button and close take touches"
            );
            if tier_for(&panel).captions.is_some() {
                let label = |s| text_brightness(&tree, s).expect("BUG: the label renders");
                assert_close(label("Restart"), 1.0, "the held button's label stays lit");
                assert_close(label("Reconfigure Wi-Fi"), DIMMED, "another's label dims");
                assert_close(label("Night Mode: On"), DIMMED, "another's label dims");
            }
        }
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
        let tree = build_with_controls(wide_panel(), controls, None);
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
            restart: true,
            ..Controls::default()
        };
        let held = status_at(Action::Restart, Phase::Holding { progress: 0.15 });
        assert_eq!(
            text_color(
                &build_with_controls(wide_panel(), controls, Some(&held)),
                "hold 5 seconds",
            ),
            Some(WHITE),
        );

        let resting = Controls {
            restart: true,
            ..Controls::default()
        };
        assert_eq!(
            text_color(
                &build_with_controls(wide_panel(), resting, None),
                "hold 5 seconds",
            ),
            Some(GRAY_50),
        );
    }

    /// Every status reads on the notice alone:
    /// no layout keeps a line of its own for it, nor for the night end time.
    #[test]
    fn no_layout_keeps_a_caption_line() {
        let failed = status_at(Action::Restart, Phase::Failed);
        for panel in [wide_panel(), narrow_panel(), round_panel()] {
            let mut texts = Vec::new();
            collect_texts(
                &build_with_controls(panel, all_controls(), Some(&failed)),
                &mut texts,
            );
            assert!(
                !texts.iter().any(|t| t.starts_with("Restart:")),
                "{panel:?}: no prefixed caption"
            );
            assert!(
                !texts.iter().any(|t| t.starts_with("Night mode on until")),
                "{panel:?}: no night mode caption"
            );
        }
    }

    fn assert_control_rows_fit(status: Option<&Status>) {
        for panel in [wide_panel(), narrow_panel(), round_panel()] {
            let tree = build_tree(
                Some("braiins-deck"),
                Some("10.0.0.2"),
                Some(-55),
                Some("MyWifi"),
                WifiIcons::default(),
                panel,
                WifiView::Idle,
                ControlIcons::default(),
                all_controls(),
                status,
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
        assert_control_rows_fit(None);
        assert_control_rows_fit(Some(&held_status()));
    }
}
