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

use bmc_render::colors::{BLACK, GRAY_50, GRAY_70, VIOLET_60, WHITE};
use bmc_render::tree::{
    DrawCommand, FontFamily, FontWeight, HostAnimationDef, PropsData, TextAlign, TextStyle,
    TreeNode, VerticalAlign,
};
use bmc_system_overlay::{DownloadProgress, UpgradeKind, UpgradePhase};
use bmc_wasm_protocol::{
    AnimProperty, Color, ColorSpace, Easing, Fill, LoopMode, SvgId, TRANSPARENT,
};

use crate::UpgradeView;
use crate::icons::UpgradeIcons;

const SAFETY_COPY: &str = "Keep the device plugged in and online during update";
const ACTIVE_BAR_TRAVEL_MS: u32 = 800;
/// Divider along the compact card's top and left edges. Both the card and the
/// widgets behind it are black, so without it the card has no visible extent.
/// The remaining two edges sit against the screen border and need none.
const COMPACT_EDGE_WIDTH: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressMode {
    None,
    Indeterminate,
    Determinate(f32),
}

#[must_use]
pub fn progress_mode(
    phase: Option<UpgradePhase>,
    progress: Option<DownloadProgress>,
) -> ProgressMode {
    if !matches!(
        phase,
        Some(UpgradePhase::FirmwareDownloading | UpgradePhase::PackageRealizing)
    ) {
        return ProgressMode::None;
    }
    match progress.and_then(|progress| {
        progress
            .total_bytes
            .filter(|total| *total > 0)
            .map(|total| byte_fraction(progress.downloaded_bytes, total))
    }) {
        Some(fraction) => ProgressMode::Determinate(fraction.clamp(0.0, 1.0)),
        None => ProgressMode::Indeterminate,
    }
}

#[must_use]
pub fn has_active_bar(view: UpgradeView) -> bool {
    let UpgradeView::Running {
        kind,
        phase,
        progress,
    } = view
    else {
        return false;
    };
    match progress_mode(phase, progress) {
        ProgressMode::Indeterminate => true,
        ProgressMode::None => kind == UpgradeKind::Packages && phase.is_some(),
        ProgressMode::Determinate(_) => false,
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the visual fraction is clamped and only drives a bounded pixel width"
)]
fn byte_fraction(downloaded: u64, total: u64) -> f32 {
    downloaded as f32 / total as f32
}

#[must_use]
pub fn decimal_megabytes(bytes: u64) -> String {
    #[expect(
        clippy::cast_precision_loss,
        reason = "display formatting rounds byte counts to one decimal megabyte"
    )]
    let megabytes = bytes as f64 / 1_000_000.0;
    let rounded = (megabytes * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

#[must_use]
pub fn transfer_text(progress: DownloadProgress) -> Option<String> {
    progress.total_bytes.map(|total| {
        format!(
            "{} MB of {} MB",
            decimal_megabytes(progress.downloaded_bytes),
            decimal_megabytes(total)
        )
    })
}

fn text_draw(
    x: f32,
    y: f32,
    text: impl Into<String>,
    size: u32,
    color: Color,
    weight: FontWeight,
) -> DrawCommand {
    DrawCommand::Text {
        x,
        y,
        text: text.into(),
        style: TextStyle {
            size,
            color,
            weight,
            align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            family: FontFamily::Sans,
            ..TextStyle::default()
        },
    }
}

fn icon_draw(icon_id: Option<SvgId>, center_x: f32, top: f32, size: f32) -> DrawCommand {
    DrawCommand::Svg {
        x: center_x - size / 2.0,
        y: top,
        w: size,
        h: size,
        color: TRANSPARENT,
        icon_id,
        anti_alias: true,
        fills: Vec::new(),
    }
}

fn icon_for_view(view: &UpgradeView, icons: UpgradeIcons) -> Option<SvgId> {
    match view {
        UpgradeView::Running { .. } => icons.tools,
        UpgradeView::Succeeded { .. } => icons.checkmark,
        UpgradeView::Failed { .. } => icons.error,
    }
}

fn bar_height(compact: bool) -> f32 {
    if compact { 5.0 } else { 7.0 }
}

fn active_bar(draws: &mut Vec<DrawCommand>, x: f32, y: f32, width: f32, height: f32) {
    draws.push(DrawCommand::Rect {
        x,
        y,
        w: width,
        h: height,
        fill: Fill::Solid(GRAY_50),
    });
    // The renderer's stock indeterminate bar draws a large playhead and
    // squiggle, unlike the stable strip. Keep the active treatment within the
    // same bar bounds as determinate progress.
    draws.push(DrawCommand::Modified {
        animations: vec![HostAnimationDef {
            property: AnimProperty::TranslateX,
            from: 0.0,
            to: width * 0.7,
            duration_ms: ACTIVE_BAR_TRAVEL_MS,
            delay_ms: 0,
            easing: Easing::Linear,
            loop_mode: LoopMode::PingPong,
        }],
        transition: None,
        color_space: ColorSpace::Oklab,
        inner: Box::new(DrawCommand::Rect {
            x,
            y,
            w: width * 0.3,
            h: height,
            fill: Fill::Solid(VIOLET_60),
        }),
    });
}

#[expect(
    clippy::cast_precision_loss,
    reason = "overlay dimensions fit exactly in f32"
)]
#[expect(
    clippy::too_many_lines,
    reason = "flat mapping keeps the stable and compact arrangements directly comparable"
)]
#[must_use]
pub fn build_upgrade_tree(view: &UpgradeView, size: (u32, u32), icons: UpgradeIcons) -> TreeNode {
    let (width, height) = (size.0 as f32, size.1 as f32);
    let compact = matches!(view.kind(), UpgradeKind::Packages);
    let (icon_size, title_size, body_size, gap, inset, icon_top_pad, icon_bottom_pad) = if compact {
        (40.0, 20, 16, 10.0, 16.0, 0.0, 0.0)
    } else {
        (80.0, 24, 18, 15.0, 0.0, 40.0, 15.0)
    };
    let mut draws = vec![DrawCommand::Rect {
        x: 0.0,
        y: 0.0,
        w: width,
        h: height,
        fill: Fill::Solid(BLACK),
    }];
    if compact {
        draws.push(DrawCommand::Rect {
            x: 0.0,
            y: 0.0,
            w: width,
            h: COMPACT_EDGE_WIDTH,
            fill: Fill::Solid(GRAY_70),
        });
        draws.push(DrawCommand::Rect {
            x: 0.0,
            y: 0.0,
            w: COMPACT_EDGE_WIDTH,
            h: height,
            fill: Fill::Solid(GRAY_70),
        });
    }
    let bar_height = bar_height(compact);
    let content_height = match view {
        UpgradeView::Running {
            phase, progress, ..
        } => match progress_mode(*phase, *progress) {
            ProgressMode::Determinate(_) => {
                icon_top_pad
                    + icon_size
                    + icon_bottom_pad
                    + gap
                    + title_size as f32
                    + gap
                    + bar_height
                    + gap
                    + body_size as f32
            }
            ProgressMode::Indeterminate => {
                icon_top_pad
                    + icon_size
                    + icon_bottom_pad
                    + gap
                    + title_size as f32
                    + gap
                    + bar_height
            }
            ProgressMode::None if compact && phase.is_some() => {
                icon_size + gap + title_size as f32 + gap + bar_height
            }
            ProgressMode::None if compact => icon_size + gap + title_size as f32,
            ProgressMode::None => {
                icon_top_pad
                    + icon_size
                    + icon_bottom_pad
                    + gap
                    + title_size as f32
                    + gap
                    + body_size as f32
            }
        },
        UpgradeView::Succeeded { .. } | UpgradeView::Failed { .. } => {
            icon_top_pad + icon_size + icon_bottom_pad + gap + title_size as f32
        }
    };
    let top = ((height - content_height) / 2.0).max(inset);
    let icon_top = top + icon_top_pad;
    draws.push(icon_draw(
        icon_for_view(view, icons),
        width / 2.0,
        icon_top,
        icon_size,
    ));

    match view {
        UpgradeView::Running {
            phase, progress, ..
        } => {
            let label = match phase {
                Some(phase) => phase.to_string(),
                None => "Preparing update".to_owned(),
            };
            match progress_mode(*phase, *progress) {
                ProgressMode::Determinate(fraction) => {
                    let percent = (fraction * 100.0).round();
                    draws.push(text_draw(
                        width / 2.0 + if compact { 0.0 } else { 10.0 },
                        icon_top + icon_size + icon_bottom_pad + gap + title_size as f32 / 2.0,
                        format!("{label} {percent:.0}%..."),
                        title_size,
                        WHITE,
                        FontWeight::BOLD,
                    ));
                    let bar_y =
                        icon_top + icon_size + icon_bottom_pad + gap + title_size as f32 + gap;
                    let bar_w = if compact {
                        width - inset * 2.0
                    } else {
                        width * 0.8
                    };
                    let bar_x = (width - bar_w) / 2.0;
                    draws.push(DrawCommand::Rect {
                        x: bar_x,
                        y: bar_y,
                        w: bar_w,
                        h: bar_height,
                        fill: Fill::Solid(GRAY_50),
                    });
                    draws.push(DrawCommand::Rect {
                        x: bar_x,
                        y: bar_y,
                        w: bar_w * fraction,
                        h: bar_height,
                        fill: Fill::Solid(VIOLET_60),
                    });
                    if let Some(progress) = (*progress).and_then(transfer_text) {
                        draws.push(text_draw(
                            width / 2.0,
                            bar_y + bar_height + gap + body_size as f32 / 2.0,
                            progress,
                            body_size,
                            GRAY_50,
                            FontWeight::REGULAR,
                        ));
                    }
                }
                ProgressMode::Indeterminate => {
                    let label_size = title_size;
                    draws.push(text_draw(
                        width / 2.0,
                        icon_top + icon_size + icon_bottom_pad + gap + label_size as f32 / 2.0,
                        label,
                        label_size,
                        WHITE,
                        FontWeight::BOLD,
                    ));
                    let bar_y =
                        icon_top + icon_size + icon_bottom_pad + gap + label_size as f32 + gap;
                    let bar_w = if compact {
                        width - inset * 2.0
                    } else {
                        width * 0.8
                    };
                    active_bar(&mut draws, (width - bar_w) / 2.0, bar_y, bar_w, bar_height);
                }
                ProgressMode::None => {
                    draws.push(text_draw(
                        width / 2.0,
                        icon_top + icon_size + icon_bottom_pad + gap + title_size as f32 / 2.0,
                        label,
                        title_size,
                        WHITE,
                        FontWeight::BOLD,
                    ));
                    if compact && phase.is_some() {
                        let bar_y = icon_top + icon_size + gap + title_size as f32 + gap;
                        active_bar(&mut draws, inset, bar_y, width - inset * 2.0, bar_height);
                    } else if !compact {
                        draws.push(text_draw(
                            width / 2.0,
                            icon_top
                                + icon_size
                                + icon_bottom_pad
                                + gap
                                + title_size as f32
                                + gap
                                + body_size as f32 / 2.0,
                            SAFETY_COPY,
                            body_size,
                            GRAY_50,
                            FontWeight::REGULAR,
                        ));
                    }
                }
            }
        }
        UpgradeView::Succeeded { .. } => draws.push(text_draw(
            width / 2.0,
            icon_top + icon_size + icon_bottom_pad + gap + title_size as f32 / 2.0,
            "Update Finished",
            title_size,
            WHITE,
            FontWeight::BOLD,
        )),
        UpgradeView::Failed { .. } => draws.push(text_draw(
            width / 2.0,
            icon_top + icon_size + icon_bottom_pad + gap + title_size as f32 / 2.0,
            "Update Failed",
            title_size,
            WHITE,
            FontWeight::BOLD,
        )),
    }
    TreeNode::Canvas {
        props: PropsData {
            width,
            height,
            ..PropsData::default()
        },
        touch_key: None,
        draws,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_icons() -> UpgradeIcons {
        let id = |raw| SvgId::from_wire(raw).expect("BUG: test SVG id must be non-zero");
        UpgradeIcons {
            tools: Some(id(1)),
            checkmark: Some(id(2)),
            error: Some(id(3)),
        }
    }

    fn tree_draws(view: &UpgradeView, size: (u32, u32)) -> Vec<DrawCommand> {
        let TreeNode::Canvas { draws, .. } = build_upgrade_tree(view, size, test_icons()) else {
            panic!("BUG: upgrade presentation must remain a canvas");
        };
        draws
    }

    /// Compact-card draws with the background and edge dividers dropped,
    /// so content assertions index from the first content element
    /// and survive a change of chrome.
    fn compact_content_draws(view: &UpgradeView) -> Vec<DrawCommand> {
        let mut draws = tree_draws(view, crate::PACKAGE_SURFACE_SIZE);
        draws.drain(..3);
        draws
    }

    fn caption_style(view: &UpgradeView, size: (u32, u32)) -> (u32, FontWeight) {
        let draws = tree_draws(view, size);
        let DrawCommand::Text { style, .. } = &draws[2] else {
            panic!("BUG: the phase caption must be the third draw");
        };
        (style.size, style.weight)
    }

    fn running_view(
        kind: UpgradeKind,
        phase: Option<UpgradePhase>,
        progress: Option<DownloadProgress>,
    ) -> UpgradeView {
        UpgradeView::Running {
            kind,
            phase,
            progress,
        }
    }

    #[test]
    fn known_totals_are_determinate_without_fake_progress() {
        let known = DownloadProgress {
            downloaded_bytes: 500_000,
            total_bytes: Some(1_000_000),
        };
        assert_eq!(
            progress_mode(Some(UpgradePhase::FirmwareDownloading), Some(known)),
            ProgressMode::Determinate(0.5)
        );
        assert_eq!(
            progress_mode(Some(UpgradePhase::FirmwareDownloading), None),
            ProgressMode::Indeterminate
        );
        assert_eq!(
            progress_mode(Some(UpgradePhase::FirmwareVerifying), Some(known)),
            ProgressMode::None
        );
    }

    #[test]
    fn decimal_megabyte_copy_matches_stable_units() {
        assert_eq!(decimal_megabytes(82_000_000), "82");
        assert_eq!(decimal_megabytes(82_450_000), "82.5");
        assert_eq!(
            decimal_megabytes(82_960_000),
            "83",
            "a value that rounds to a whole megabyte must drop the decimal"
        );
        assert_eq!(
            transfer_text(DownloadProgress {
                downloaded_bytes: 82_000_000,
                total_bytes: Some(151_000_000)
            }),
            Some("82 MB of 151 MB".to_owned())
        );
    }

    #[test]
    fn fullscreen_stage_keeps_stable_icon_geometry_and_supporting_copy() {
        let draws = tree_draws(
            &running_view(
                UpgradeKind::Firmware,
                Some(UpgradePhase::FirmwareVerifying),
                None,
            ),
            (1_280, 480),
        );

        assert!(matches!(
            &draws[1],
            DrawCommand::Svg {
                x: 600.0,
                y: 176.5,
                w: 80.0,
                h: 80.0,
                icon_id: Some(id),
                ..
            } if id.to_wire() == 1
        ));
        assert!(matches!(
            &draws[2],
            DrawCommand::Text { text, style, .. }
                if text == "Verifying firmware"
                    && style.size == 24
                    && style.weight == FontWeight::BOLD
                    && style.color == WHITE
                    && style.family == FontFamily::Sans
        ));
        assert!(matches!(
            &draws[3],
            DrawCommand::Text { text, style, .. }
                if text == SAFETY_COPY
                    && style.size == 18
                    && style.weight == FontWeight::REGULAR
                    && style.color == GRAY_50
        ));
    }

    #[test]
    fn fullscreen_download_keeps_stable_bar_and_twenty_pixel_label_inset() {
        let draws = tree_draws(
            &running_view(
                UpgradeKind::Firmware,
                Some(UpgradePhase::FirmwareDownloading),
                Some(DownloadProgress {
                    downloaded_bytes: 82_000_000,
                    total_bytes: Some(151_000_000),
                }),
            ),
            (1_280, 480),
        );

        assert!(matches!(
            &draws[2],
            DrawCommand::Text { x: 650.0, text, style, .. }
                if text == "Downloading firmware 54%..."
                    && style.size == 24
                    && style.weight == FontWeight::BOLD
        ));
        assert!(matches!(
            &draws[3],
            DrawCommand::Rect {
                x: 128.0,
                w: 1024.0,
                h: 7.0,
                ..
            }
        ));
        assert!(matches!(
            &draws[5],
            DrawCommand::Text { text, style, .. }
                if text == "82 MB of 151 MB"
                    && style.size == 18
                    && style.color == GRAY_50
        ));
    }

    #[test]
    fn phase_caption_keeps_one_treatment_across_progress_modes() {
        let determinate = running_view(
            UpgradeKind::Firmware,
            Some(UpgradePhase::FirmwareDownloading),
            Some(DownloadProgress {
                downloaded_bytes: 82_000_000,
                total_bytes: Some(151_000_000),
            }),
        );
        let indeterminate = running_view(
            UpgradeKind::Firmware,
            Some(UpgradePhase::FirmwareDownloading),
            None,
        );
        let no_progress = running_view(
            UpgradeKind::Firmware,
            Some(UpgradePhase::FirmwareVerifying),
            None,
        );

        let expected = (24, FontWeight::BOLD);
        for view in [&determinate, &indeterminate, &no_progress] {
            assert_eq!(
                caption_style(view, (1_280, 480)),
                expected,
                "the phase caption must not change weight or size as progress arrives"
            );
        }
    }

    /// The card and the widgets it covers are both black, so the two edges
    /// facing widget content are all that gives the card an extent.
    /// The fullscreen firmware surface must not get them: its own tests
    /// expect content immediately after the background.
    #[test]
    fn compact_card_marks_the_edges_that_meet_widget_content() {
        let draws = tree_draws(
            &running_view(UpgradeKind::Packages, None, None),
            crate::PACKAGE_SURFACE_SIZE,
        );

        assert!(matches!(
            &draws[1],
            DrawCommand::Rect { x: 0.0, y: 0.0, w: 384.0, h: 2.0, fill: Fill::Solid(color) }
                if *color == GRAY_70
        ));
        assert!(matches!(
            &draws[2],
            DrawCommand::Rect { x: 0.0, y: 0.0, w: 2.0, h: 192.0, fill: Fill::Solid(color) }
                if *color == GRAY_70
        ));
    }

    #[test]
    fn compact_stage_scales_icon_type_and_bar_together() {
        let draws = compact_content_draws(&running_view(
            UpgradeKind::Packages,
            Some(UpgradePhase::PackageVerifying),
            None,
        ));

        assert!(matches!(
            &draws[0],
            DrawCommand::Svg {
                x: 172.0,
                y: 53.5,
                w: 40.0,
                h: 40.0,
                ..
            }
        ));
        assert!(matches!(
            &draws[1],
            DrawCommand::Text { text, style, .. }
                if text == "Verifying packages"
                    && style.size == 20
                    && style.weight == FontWeight::BOLD
        ));
        assert!(matches!(
            &draws[2],
            DrawCommand::Rect {
                x: 16.0,
                w: 352.0,
                h: 5.0,
                ..
            }
        ));
    }

    #[test]
    fn compact_preparing_state_does_not_invent_progress() {
        let draws = compact_content_draws(&running_view(UpgradeKind::Packages, None, None));

        assert_eq!(draws.len(), 2);
        assert!(matches!(
            &draws[1],
            DrawCommand::Text { text, .. } if text == "Preparing update"
        ));
    }

    #[test]
    fn active_bar_moves_its_segment_within_the_track() {
        let draws = compact_content_draws(&running_view(
            UpgradeKind::Packages,
            Some(UpgradePhase::PackageRealizing),
            None,
        ));

        let DrawCommand::Modified {
            animations, inner, ..
        } = &draws[3]
        else {
            panic!("BUG: active progress segment must carry its motion");
        };
        let [animation] = animations.as_slice() else {
            panic!("BUG: active progress segment must have exactly one animation");
        };
        assert_eq!(animation.property, AnimProperty::TranslateX);
        assert!(animation.from.abs() < f32::EPSILON);
        assert!((animation.to - 352.0 * 0.7).abs() < f32::EPSILON);
        assert_eq!(animation.duration_ms, ACTIVE_BAR_TRAVEL_MS);
        assert_eq!(animation.easing, Easing::Linear);
        assert_eq!(animation.loop_mode, LoopMode::PingPong);
        assert!(matches!(
            inner.as_ref(),
            DrawCommand::Rect {
                x: 16.0,
                h: 5.0,
                ..
            }
        ));
    }

    #[test]
    fn terminal_states_use_semantic_icons_with_white_stable_titles() {
        for (view, expected_id, expected_title) in [
            (
                UpgradeView::Succeeded {
                    kind: UpgradeKind::Firmware,
                },
                2,
                "Update Finished",
            ),
            (
                UpgradeView::Failed {
                    kind: UpgradeKind::Firmware,
                },
                3,
                "Update Failed",
            ),
        ] {
            let draws = tree_draws(&view, (1_280, 480));
            assert!(matches!(
                &draws[1],
                DrawCommand::Svg { icon_id: Some(id), .. } if id.to_wire() == expected_id
            ));
            assert!(matches!(
                &draws[2],
                DrawCommand::Text { text, style, .. }
                    if text == expected_title && style.color == WHITE
            ));
        }
    }
}
