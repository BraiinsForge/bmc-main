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
    AutoFit, DrawCommand, FontFamily, FontWeight, HostAnimationDef, PropsData, TextAlign,
    TextStyle, TreeNode, VerticalAlign,
};
use bmc_system_overlay::{DownloadProgress, UpgradeKind, UpgradePhase};
use bmc_wasm_protocol::{
    AnimProperty, Color, ColorSpace, Easing, Fill, LoopMode, SvgId, TRANSPARENT,
};

use crate::UpgradeView;
use crate::icons::UpgradeIcons;

const SAFETY_COPY: &str = "Keep the device plugged in and online during the update";
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

/// How the surface sits on the display. The two edge dividers key off this
/// rather than off the upgrade kind: they give a card an extent against the
/// black widgets it overlaps, and a fullscreen surface has nothing beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Fullscreen,
    Card,
}

/// The surface a tree is built for: its logical size and how it sits on the
/// display. Which upgrade it presents comes from the view.
#[derive(Debug, Clone, Copy)]
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub placement: Placement,
}

/// Per-surface sizing, selected by placement and width in [`tier_for`].
/// Type and icons do not scale linearly with the display, so each tier states
/// its own numbers rather than deriving them from a factor.
#[derive(Debug, Clone, Copy)]
struct Tier {
    icon: f32,
    /// Top edge of the icon, from the top of the surface. Fixed per tier rather
    /// than derived from what sits below it: the content under the icon changes
    /// with the phase, and centring the whole block against it made the icon
    /// jump between phases of one run.
    icon_top: f32,
    icon_bottom_pad: f32,
    title: u32,
    body: u32,
    gap: f32,
    /// Side padding, which the wrapped safety box is laid out inside.
    inset: f32,
    bar_height: f32,
    /// Progress-bar inset from both surface edges.
    bar_inset: f32,
    /// Rightward nudge of the determinate caption, which the stable Deck screen
    /// ships with. It predates this table and no reason for it is recorded, so
    /// the narrower tiers do not repeat it.
    caption_nudge: f32,
    /// Whether the determinate screen draws the transferred/total byte counts.
    /// The narrow card has no line to spare for them.
    transfer_line: bool,
    /// Whether the determinate caption drops its subject noun. It is the longest
    /// string either surface draws — a phase label, a percentage and an ellipsis
    /// — so the narrow card asks for [`UpgradePhase::short_label`] instead.
    short_percent_caption: bool,
    /// Lines the safety copy is allowed to take. Above one it is drawn as a
    /// wrapped box instead of a single line — see [`safety_draw`].
    safety_lines: f32,
}

/// The Deck's fullscreen firmware surface (1280x480).
const FULL_LARGE: Tier = Tier {
    icon: 80.0,
    icon_top: 176.5,
    icon_bottom_pad: 15.0,
    title: 24,
    body: 18,
    gap: 15.0,
    inset: 0.0,
    bar_height: 7.0,
    bar_inset: 128.0,
    caption_nudge: 10.0,
    transfer_line: true,
    short_percent_caption: false,
    safety_lines: 1.0,
};

/// The BMM101's fullscreen surface (480x320). The Deck's type and icon carry
/// over; only the bar, sized as a fraction of the display, comes in.
const FULL_MEDIUM: Tier = Tier {
    icon_top: 96.5,
    bar_inset: 48.0,
    caption_nudge: 0.0,
    ..FULL_LARGE
};

/// The Deck's package card (384x192).
const CARD_LARGE: Tier = Tier {
    icon: 40.0,
    icon_top: 53.5,
    icon_bottom_pad: 0.0,
    title: 20,
    body: 16,
    gap: 10.0,
    inset: 16.0,
    bar_height: 5.0,
    bar_inset: 16.0,
    caption_nudge: 0.0,
    transfer_line: true,
    short_percent_caption: false,
    safety_lines: 1.0,
};

/// The BMM101's package card (240x120). The Deck card's phase labels reach the
/// edges of this one, so the type steps down and the two longest strings give
/// way: the byte counts go, and the determinate caption loses its noun.
const CARD_SMALL: Tier = Tier {
    icon: 36.0,
    icon_top: 22.5,
    title: 18,
    body: 14,
    gap: 8.0,
    inset: 12.0,
    bar_inset: 12.0,
    transfer_line: false,
    short_percent_caption: true,
    ..CARD_LARGE
};

/// The BMM100's fullscreen surface (320x240), which both kinds use: a card small
/// enough to leave its widget visible would not hold the content. The Deck's
/// icon and type do not fit across 320 px, and the safety copy needs two lines
/// even after stepping down.
const FULL_SMALL: Tier = Tier {
    icon: 64.0,
    icon_top: 60.0,
    icon_bottom_pad: 8.0,
    title: 18,
    body: 14,
    gap: 8.0,
    inset: 12.0,
    bar_height: 5.0,
    bar_inset: 24.0,
    safety_lines: 2.0,
    ..FULL_MEDIUM
};

/// Thresholds sit in the gaps between the surfaces that exist, so no product
/// lands near an edge: cards are 240 and 384 wide, fullscreen surfaces 320, 480
/// and 1280.
fn tier_for(surface: Surface) -> Tier {
    match surface.placement {
        Placement::Fullscreen if surface.width >= 960 => FULL_LARGE,
        Placement::Fullscreen if surface.width >= 400 => FULL_MEDIUM,
        Placement::Fullscreen => FULL_SMALL,
        Placement::Card if surface.width >= 320 => CARD_LARGE,
        Placement::Card => CARD_SMALL,
    }
}

/// The safety copy, on one line or wrapped into a box.
///
/// It is the longest string the overlay draws, and a canvas text draw is always
/// a single unwrapped line, so a narrow display needs the paragraph path that
/// [`DrawCommand::AutofitText`] reaches. `min_size` equal to the style size
/// makes it wrap without shrinking.
///
/// The wide tiers stay on the single-line draw rather than taking the box for
/// consistency: the two paths anchor differently — glyph centre against line
/// box — so switching them over would move the stable copy a few pixels for no
/// gain.
#[expect(
    clippy::cast_precision_loss,
    reason = "type sizes are a couple of dozen pixels"
)]
fn safety_draw(tier: Tier, width: f32, center_y: f32, size: u32) -> DrawCommand {
    if tier.safety_lines <= 1.0 {
        return text_draw(
            width / 2.0,
            center_y,
            SAFETY_COPY,
            size,
            GRAY_50,
            FontWeight::REGULAR,
        );
    }
    // TextStyle's own default, spelled out so the box and the layout agree.
    let line_height = 1.4;
    let box_height = size as f32 * line_height * tier.safety_lines;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "body type sizes are a couple of dozen pixels"
    )]
    let floor = size as u16;
    DrawCommand::AutofitText {
        x: tier.inset,
        y: center_y - size as f32 / 2.0,
        box_width: width - tier.inset * 2.0,
        box_height,
        mode: AutoFit::Shrink,
        min_size: floor,
        max_size: floor,
        text: SAFETY_COPY.to_owned(),
        style: TextStyle {
            size,
            color: GRAY_50,
            weight: FontWeight::REGULAR,
            align: TextAlign::Center,
            line_height,
            family: FontFamily::Sans,
            ..TextStyle::default()
        },
    }
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
    reason = "a flat mapping keeps every phase's arrangement directly comparable"
)]
#[must_use]
pub fn build_upgrade_tree(view: &UpgradeView, surface: Surface, icons: UpgradeIcons) -> TreeNode {
    let (width, height) = (surface.width as f32, surface.height as f32);
    // Content follows the upgrade kind; sizing follows the surface.
    let packages = matches!(view.kind(), UpgradeKind::Packages);
    let tier = tier_for(surface);
    let Tier {
        icon: icon_size,
        icon_top,
        icon_bottom_pad,
        title: title_size,
        body: body_size,
        gap,
        ..
    } = tier;
    let mut draws = vec![DrawCommand::Rect {
        x: 0.0,
        y: 0.0,
        w: width,
        h: height,
        fill: Fill::Solid(BLACK),
    }];
    if surface.placement == Placement::Card {
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
    let bar_height = tier.bar_height;
    let (bar_x, bar_w) = (tier.bar_inset, width - tier.bar_inset * 2.0);
    let title_center_y = icon_top + icon_size + icon_bottom_pad + gap + title_size as f32 / 2.0;
    let below_title_y = title_center_y + title_size as f32 / 2.0 + gap;
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
                    // Determinate progress only ever runs under a download
                    // phase, so a phase is always in hand for the short form.
                    let label = match (tier.short_percent_caption, phase) {
                        (true, Some(phase)) => phase.short_label().to_owned(),
                        (true, None) | (false, _) => label,
                    };
                    draws.push(text_draw(
                        width / 2.0 + tier.caption_nudge,
                        title_center_y,
                        format!("{label} {percent:.0}%..."),
                        title_size,
                        WHITE,
                        FontWeight::BOLD,
                    ));
                    draws.push(DrawCommand::Rect {
                        x: bar_x,
                        y: below_title_y,
                        w: bar_w,
                        h: bar_height,
                        fill: Fill::Solid(GRAY_50),
                    });
                    draws.push(DrawCommand::Rect {
                        x: bar_x,
                        y: below_title_y,
                        w: bar_w * fraction,
                        h: bar_height,
                        fill: Fill::Solid(VIOLET_60),
                    });
                    if let Some(progress) = (*progress)
                        .filter(|_| tier.transfer_line)
                        .and_then(transfer_text)
                    {
                        draws.push(text_draw(
                            width / 2.0,
                            below_title_y + bar_height + gap + body_size as f32 / 2.0,
                            progress,
                            body_size,
                            GRAY_50,
                            FontWeight::REGULAR,
                        ));
                    }
                }
                ProgressMode::Indeterminate => {
                    draws.push(text_draw(
                        width / 2.0,
                        title_center_y,
                        label,
                        title_size,
                        WHITE,
                        FontWeight::BOLD,
                    ));
                    active_bar(&mut draws, bar_x, below_title_y, bar_w, bar_height);
                }
                ProgressMode::None => {
                    draws.push(text_draw(
                        width / 2.0,
                        title_center_y,
                        label,
                        title_size,
                        WHITE,
                        FontWeight::BOLD,
                    ));
                    if packages && phase.is_some() {
                        active_bar(&mut draws, bar_x, below_title_y, bar_w, bar_height);
                    } else if !packages {
                        draws.push(safety_draw(
                            tier,
                            width,
                            below_title_y + body_size as f32 / 2.0,
                            body_size,
                        ));
                    }
                }
            }
        }
        UpgradeView::Succeeded { .. } => draws.push(text_draw(
            width / 2.0,
            title_center_y,
            "Update Finished",
            title_size,
            WHITE,
            FontWeight::BOLD,
        )),
        UpgradeView::Failed { .. } => draws.push(text_draw(
            width / 2.0,
            title_center_y,
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

    /// Draws for a view at `size`, on the placement that kind ships with:
    /// firmware fullscreen, packages on the corner card. The cases that pair a
    /// kind with the other placement spell the surface out instead.
    fn tree_draws(view: &UpgradeView, size: (u32, u32)) -> Vec<DrawCommand> {
        let placement = match view.kind() {
            UpgradeKind::Packages => Placement::Card,
            UpgradeKind::Firmware => Placement::Fullscreen,
            kind => panic!("BUG: unsupported upgrade kind {kind:?}"),
        };
        surface_draws(view, size, placement)
    }

    fn surface_draws(
        view: &UpgradeView,
        size: (u32, u32),
        placement: Placement,
    ) -> Vec<DrawCommand> {
        let surface = Surface {
            width: size.0,
            height: size.1,
            placement,
        };
        let TreeNode::Canvas { draws, .. } = build_upgrade_tree(view, surface, test_icons()) else {
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

    /// Only the package card has both terminal screens: a firmware success
    /// is the device-info overlay's, so this surface never builds one.
    #[test]
    fn compact_terminal_states_use_semantic_icons_with_white_stable_titles() {
        for (view, expected_id, expected_title) in [
            (
                UpgradeView::Succeeded {
                    kind: UpgradeKind::Packages,
                },
                2,
                "Update Finished",
            ),
            (
                UpgradeView::Failed {
                    kind: UpgradeKind::Packages,
                },
                3,
                "Update Failed",
            ),
        ] {
            let draws = compact_content_draws(&view);
            assert!(matches!(
                &draws[0],
                DrawCommand::Svg { icon_id: Some(id), .. } if id.to_wire() == expected_id
            ));
            assert!(matches!(
                &draws[1],
                DrawCommand::Text { text, style, .. }
                    if text == expected_title && style.color == WHITE
            ));
        }
    }

    /// The icon holds its place for a whole run. What sits under it changes from
    /// phase to phase — a bar arrives, byte counts come and go, the terminal
    /// screen has neither — and an icon positioned against that content walks up
    /// and down the surface as the run proceeds.
    #[test]
    fn the_icon_does_not_move_between_the_phases_of_one_run() {
        let icon_y = |draws: &[DrawCommand]| {
            draws
                .iter()
                .find_map(|draw| {
                    if let DrawCommand::Svg { y, .. } = draw {
                        Some(*y)
                    } else {
                        None
                    }
                })
                .expect("BUG: every upgrade screen draws an icon")
        };

        for (kind, placement, size) in [
            (UpgradeKind::Firmware, Placement::Fullscreen, (1_280, 480)),
            (UpgradeKind::Firmware, Placement::Fullscreen, (480, 320)),
            (UpgradeKind::Firmware, Placement::Fullscreen, (320, 240)),
            (UpgradeKind::Packages, Placement::Card, (384, 192)),
            (UpgradeKind::Packages, Placement::Card, (240, 120)),
            (UpgradeKind::Packages, Placement::Fullscreen, (320, 240)),
        ] {
            let download = if kind == UpgradeKind::Firmware {
                UpgradePhase::FirmwareDownloading
            } else {
                UpgradePhase::PackageRealizing
            };
            let views = [
                running_view(kind, None, None),
                running_view(
                    kind,
                    Some(download),
                    Some(DownloadProgress {
                        downloaded_bytes: 82_000_000,
                        total_bytes: Some(151_000_000),
                    }),
                ),
                running_view(kind, Some(download), None),
                running_view(kind, Some(UpgradePhase::PackageBuilding), None),
                UpgradeView::Failed { kind },
            ];
            let tops: Vec<f32> = views
                .iter()
                .map(|view| icon_y(&surface_draws(view, size, placement)))
                .collect();

            assert!(
                tops.windows(2)
                    .all(|pair| (pair[0] - pair[1]).abs() < f32::EPSILON),
                "{kind:?} on {placement:?} {size:?} moves its icon across phases: {tops:?}"
            );
        }
    }

    /// The BMM101 card is the one surface where the determinate caption does not
    /// fit: the phase noun goes, and the byte counts with it.
    #[test]
    fn the_narrow_card_sheds_the_phase_noun_and_the_byte_counts() {
        let draws = surface_draws(
            &running_view(
                UpgradeKind::Packages,
                Some(UpgradePhase::PackageRealizing),
                Some(DownloadProgress {
                    downloaded_bytes: 82_000_000,
                    total_bytes: Some(151_000_000),
                }),
            ),
            crate::PACKAGE_SURFACE_SIZE_BMM101,
            Placement::Card,
        );

        assert!(matches!(
            &draws[4],
            DrawCommand::Text { text, style, .. }
                if text == "Downloading 54%..." && style.size == 18
        ));
        assert!(
            !draws.iter().any(|draw| matches!(
                draw,
                DrawCommand::Text { text, .. } if text.contains(" MB of ")
            )),
            "the byte counts have no line to sit on at 240x120"
        );
    }

    /// The Deck card keeps both, so shedding them stays a property of the narrow
    /// tier rather than of package upgrades.
    #[test]
    fn the_deck_card_keeps_the_phase_noun_and_the_byte_counts() {
        let draws = compact_content_draws(&running_view(
            UpgradeKind::Packages,
            Some(UpgradePhase::PackageRealizing),
            Some(DownloadProgress {
                downloaded_bytes: 82_000_000,
                total_bytes: Some(151_000_000),
            }),
        ));

        assert!(matches!(
            &draws[1],
            DrawCommand::Text { text, .. } if text == "Downloading packages 54%..."
        ));
        assert!(draws.iter().any(|draw| matches!(
            draw,
            DrawCommand::Text { text, .. } if text == "82 MB of 151 MB"
        )));
    }

    /// 320 px cannot hold the safety copy on one line at any readable size, and
    /// a canvas text draw never wraps — so that tier alone takes the box.
    #[test]
    fn the_small_fullscreen_surface_wraps_the_safety_copy_into_a_box() {
        let draws = tree_draws(
            &running_view(
                UpgradeKind::Firmware,
                Some(UpgradePhase::FirmwareVerifying),
                None,
            ),
            (320, 240),
        );

        assert!(matches!(
            &draws[3],
            DrawCommand::AutofitText {
                x: 12.0,
                box_width: 296.0,
                box_height,
                mode: AutoFit::Shrink,
                min_size: 14,
                max_size: 14,
                text,
                style,
                ..
            } if text == SAFETY_COPY
                && style.size == 14
                && (*box_height - 39.2).abs() < 0.01
        ));
    }

    /// The Deck keeps the single-line draw: the two paths anchor differently, so
    /// moving it onto the box would shift stable copy for nothing.
    #[test]
    fn the_wide_surfaces_keep_the_single_line_safety_draw() {
        for size in [(1_280, 480), (480, 320)] {
            let draws = tree_draws(
                &running_view(
                    UpgradeKind::Firmware,
                    Some(UpgradePhase::FirmwareVerifying),
                    None,
                ),
                size,
            );
            assert!(matches!(
                &draws[3],
                DrawCommand::Text { text, .. } if text == SAFETY_COPY
            ));
        }
    }

    /// The BMM101's fullscreen surface keeps the Deck's type and icon; only the
    /// bar, which is a fraction of the display, comes in with the display.
    #[test]
    fn the_medium_fullscreen_surface_keeps_deck_type_with_its_own_bar() {
        let draws = tree_draws(
            &running_view(
                UpgradeKind::Firmware,
                Some(UpgradePhase::FirmwareDownloading),
                Some(DownloadProgress {
                    downloaded_bytes: 82_000_000,
                    total_bytes: Some(151_000_000),
                }),
            ),
            (480, 320),
        );

        assert!(matches!(
            &draws[2],
            DrawCommand::Text { x: 240.0, text, style, .. }
                if text == "Downloading firmware 54%..." && style.size == 24
        ));
        assert!(matches!(
            &draws[3],
            DrawCommand::Rect {
                x: 48.0,
                w: 384.0,
                h: 7.0,
                ..
            }
        ));
    }

    #[test]
    fn the_fullscreen_failure_keeps_its_icon_and_white_title() {
        let draws = tree_draws(
            &UpgradeView::Failed {
                kind: UpgradeKind::Firmware,
            },
            (1_280, 480),
        );

        assert!(matches!(
            &draws[1],
            DrawCommand::Svg { icon_id: Some(id), .. } if id.to_wire() == 3
        ));
        assert!(matches!(
            &draws[2],
            DrawCommand::Text { text, style, .. }
                if text == "Update Failed" && style.color == WHITE
        ));
    }
}
