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

//! The layouts as views over a [`ViewData`] and the system snapshot: a node tree out.
//!
//! The predicted date and time format against `system::current()`,
//! so whoever installs the snapshot — the host, the gallery, a test —
//! sets the formats and the timezone once for every layout.

#[expect(
    clippy::wildcard_imports,
    reason = "screen code uses the SDK's tree builders, macros, and tokens throughout"
)]
use bmc_wasm_sdk::*;
use units::availability::Availability;

use crate::manifest_params::Params;
use crate::model::{Countdown, Frame, Prediction, RATE_LIMIT_RETRY, SizeBucket, Status};
use crate::screens::parts::{self, Layout, Numerals, SizeParams, TileSizes};

// Round numerals are sized so the worst-case `DDDD:HH:MM` row
// (four-digit days) clears the circular cutout when centered.
const ROUND_TITLE_PX: u32 = 24;
const ROUND_NUMERAL_PX: u32 = 64;
const ROUND_LABEL_PX: u32 = 20;
const ROUND_GAP_PX: f32 = 12.0;

const FULL: SizeParams = SizeParams {
    layout: Layout::Tiled(TileSizes {
        label: 24,
        value: 40,
        sub: 24,
    }),
    title: 32,
    numeral: 120,
    label: 28,
    padding: 16.0,
    gap: 12.0,
};
const LARGE: SizeParams = SizeParams {
    layout: Layout::Tiled(TileSizes {
        label: 22,
        value: 36,
        sub: 22,
    }),
    title: 28,
    numeral: 96,
    label: 26,
    padding: 12.0,
    gap: 8.0,
};
const MEDIUM: SizeParams = SizeParams {
    layout: Layout::Compact,
    title: 26,
    numeral: 96,
    label: 24,
    padding: 8.0,
    gap: 20.0,
};
const SMALL: SizeParams = SizeParams {
    layout: Layout::Compact,
    title: 20,
    numeral: 48,
    label: 18,
    padding: 8.0,
    gap: 10.0,
};

fn size_params(variant: SizeVariant) -> &'static SizeParams {
    match variant {
        SizeVariant::Full => &FULL,
        SizeVariant::Large => &LARGE,
        SizeVariant::Medium => &MEDIUM,
        SizeVariant::Small => &SMALL,
    }
}

/// A prediction and the moment it is counted from, the viewport it is drawn into,
/// the operator's params and how far the prediction can be trusted.
#[derive(Clone, Debug)]
pub struct ViewData {
    pub viewport: WidgetViewport,
    pub params: Params,
    pub prediction: Availability<Prediction>,
    pub status: Status,
    pub now_secs: i64,
}

/// Everything a layout draws from a prediction, formatted once per render.
struct DisplayData {
    countdown: Countdown,
    predicted_date: String,
    predicted_time: String,
    blocks_remaining: u32,
    target_block: String,
}

impl DisplayData {
    fn blocks_left(&self) -> String {
        format_number!(f64::from(self.blocks_remaining), 0)
    }

    fn target_caption(&self) -> String {
        let mut caption = String::from("Target #");
        caption.push_str(&self.target_block);
        caption
    }
}

fn display_data(view: &ViewData) -> Option<DisplayData> {
    let prediction = view.prediction.as_option().copied()?;
    let (predicted_date, predicted_time) = predicted_moment(prediction.predicted_unix);
    Some(DisplayData {
        countdown: prediction.countdown(view.now_secs),
        predicted_date,
        predicted_time,
        blocks_remaining: prediction.blocks_remaining(),
        target_block: format_number!(f64::from(prediction.target_block), 0),
    })
}

/// The predicted instant as a date and a time captioned with its zone,
/// in the system timezone.
fn predicted_moment(predicted_unix: i64) -> (String, String) {
    let at = SystemTime {
        unix_secs: predicted_unix,
    };
    let tz = system::current().timezone().map(Tz::from_runtime);

    let date = format_date(
        at,
        FormatDateOpts {
            timezone: tz.clone(),
            ..FormatDateOpts::default()
        },
    );
    let mut time = format_time(
        at,
        FormatTimeOpts {
            timezone: tz.clone(),
            ..FormatTimeOpts::default()
        },
    );
    // Formatting returns empty for an out-of-range instant; fall back to `-`.
    if date.is_empty() || time.is_empty() {
        return (
            parts::NOT_AVAILABLE.to_owned(),
            parts::NOT_AVAILABLE.to_owned(),
        );
    }
    if let Some(meridiem) = format::meridiem(at, tz.as_ref()) {
        time.push(' ');
        time.push_str(&meridiem);
    }
    // Timezone caption (e.g. "Prague (+2)") — same convention as the clock widget.
    time.push(' ');
    format::push_tz_caption(&mut time, &format::resolve_tz_for_label(None, at.unix_secs));
    (date, time)
}

/// The layout `view`'s viewport draws, under the overlay its status asks for.
#[must_use]
pub fn halving_view(view: &ViewData) -> Node {
    let weight = parts::numeral_weight(view.params.numbers_font_style);
    let data = display_data(view);
    let root = match view.viewport.shape {
        // The rectangular layouts pin content to the edges, which the circular cutout clips;
        // the round face centers a scaled title · countdown · labels stack instead.
        ViewportShape::Round => view_round(view.viewport, weight, data.as_ref()),
        ViewportShape::Rectangular => view_rectangular(view.viewport, weight, data.as_ref()),
    };
    with_status(root, view.status, view.viewport.shape)
}

fn view_rectangular(
    viewport: WidgetViewport,
    weight: FontWeight,
    data: Option<&DisplayData>,
) -> Node {
    let frame = Frame::of(viewport);
    if frame.bucket == SizeBucket::Bmm101 {
        return bmm101(weight, data);
    }
    let size = size_params(frame.size.variant).scaled(frame.size.fit());
    match size.layout {
        Layout::Compact => view_compact(&size, weight, data),
        Layout::Tiled(tiles) => view_tiles(&size, tiles, weight, data),
    }
}

fn with_status(root: Node, status: Status, shape: ViewportShape) -> Node {
    match status {
        Status::Ready => root,
        Status::Stale(last_success) => status_overlay::with_stale_overlay(
            root,
            SystemTime {
                unix_secs: last_success,
            },
            shape,
        ),
        Status::Failed => {
            status_overlay::with_error_overlay(root, "Halving prediction unavailable", shape)
        }
        Status::RateLimited => status_overlay::with_overlay(
            root,
            tag(
                TagKind::Warning,
                TagIcon::Default,
                text(
                    fmt!(
                        "Rate limited — retrying in {} min",
                        RATE_LIMIT_RETRY.as_secs() / 60
                    ),
                    style!(size: 12, color: ORANGE_40),
                ),
            ),
            shape,
        ),
    }
}

fn numerals(data: Option<&DisplayData>) -> Numerals {
    Numerals::of(data.map(|d| d.countdown))
}

fn narrow_numerals(data: Option<&DisplayData>) -> Numerals {
    Numerals::narrow(data.map(|d| d.countdown))
}

/// Tiled layout (Large/Full): title and countdown on top,
/// the predicted-date and blocks-remaining tiles below.
fn view_tiles(
    size: &SizeParams,
    tiles: TileSizes,
    weight: FontWeight,
    data: Option<&DisplayData>,
) -> Node {
    let countdown = parts::countdown_columns(numerals(data), size, weight, size.gap * 2.0);

    let top = col(
        props!(background: parts::TILE_BG, flex: 3.0),
        [center(
            props!(flex: 1.0),
            [col(
                props!(gap: size.gap + 4.0, cross_align: CrossAlign::Center),
                [parts::title(size), countdown],
            )],
        )],
    );

    let bottom = row(
        props!(gap: size.gap, flex: 2.0),
        [
            parts::info_tile(
                "Predicted Date",
                data.map(|d| d.predicted_date.clone()),
                data.map(|d| d.predicted_time.clone()),
                tiles,
            ),
            parts::info_tile(
                "Blocks Remaining",
                data.map(DisplayData::blocks_left),
                data.map(DisplayData::target_caption),
                tiles,
            ),
        ],
    );

    col(
        props!(background: BLACK, padding: size.padding, gap: size.gap),
        [top, bottom],
    )
}

/// Compact layout (Small/Medium): one tile, the title on top
/// and the countdown numerals over their labels centered below.
fn view_compact(size: &SizeParams, weight: FontWeight, data: Option<&DisplayData>) -> Node {
    let countdown = parts::countdown_columns(narrow_numerals(data), size, weight, size.gap);

    let tile = col(
        props!(background: parts::TILE_BG, padding: 12.0, flex: 1.0),
        [parts::title(size), center(props!(flex: 1.0), [countdown])],
    );

    col(props!(background: BLACK, padding: size.padding), [tile])
}

/// Round layout: a vertically-centered title · countdown · labels stack,
/// scaled to sit within the circular cutout.
fn view_round(viewport: WidgetViewport, weight: FontWeight, data: Option<&DisplayData>) -> Node {
    let scale = WidgetSize::from_dimensions(viewport.width, viewport.height).round_scale();
    let size = SizeParams {
        layout: Layout::Compact,
        title: scale_font(ROUND_TITLE_PX, scale),
        numeral: scale_font(ROUND_NUMERAL_PX, scale),
        label: scale_font(ROUND_LABEL_PX, scale),
        padding: 0.0,
        gap: ROUND_GAP_PX * scale,
    };
    let countdown = parts::countdown_columns(narrow_numerals(data), &size, weight, size.gap);

    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [col(
                props!(gap: size.gap, cross_align: CrossAlign::Center),
                [parts::title(&size), countdown],
            )],
        )],
    )
}

// ── BMM101 ─────────────────────────────────────────────────────────────

const BMM101_EDGE: f32 = 16.0;
/// Figma's `normal` line box for Braiins Sans, which every slot below is measured in.
const BMM101_LINE_HEIGHT: f32 = 1.3;
const BMM101_TITLE_SIZE: u32 = 14;
const BMM101_TITLE_SLOT: f32 = 18.0;
const BMM101_NUMERAL_SIZE: u32 = 64;
/// The units start a pixel inside the numerals' 83 px line box.
const BMM101_NUMERAL_SLOT: f32 = 82.0;
const BMM101_UNIT_SIZE: u32 = 14;
const BMM101_UNIT_SLOT: f32 = 18.0;
const BMM101_COUNTDOWN_GAP: f32 = 10.0;
const BMM101_ROW_SIZE: u32 = 20;
const BMM101_ROW_SLOT: f32 = 26.0;
const BMM101_SUB_SIZE: u32 = 16;
const BMM101_SUB_SLOT: f32 = 21.0;
const BMM101_SUB_GAP: f32 = 4.0;

fn bmm101_text(value: impl Into<String>, size: u32, weight: FontWeight, color: Color) -> Node {
    text(
        value,
        style!(size: size, weight: weight, color: color, line_height: BMM101_LINE_HEIGHT),
    )
}

fn bmm101_slot(height: f32, align: CrossAlign, content: Node) -> Node {
    col(props!(height: height, cross_align: align), [content])
}

/// The numerals over their lowercase units, colons riding the numerals' line.
fn bmm101_countdown(numerals: Numerals, weight: FontWeight) -> Node {
    let numeral = |value: String| {
        bmm101_slot(
            BMM101_NUMERAL_SLOT,
            CrossAlign::Center,
            bmm101_text(value, BMM101_NUMERAL_SIZE, weight, WHITE),
        )
    };
    let column = |value: String, unit: &str| {
        col(
            props!(cross_align: CrossAlign::Center),
            [
                numeral(value),
                bmm101_slot(
                    BMM101_UNIT_SLOT,
                    CrossAlign::Center,
                    bmm101_text(unit, BMM101_UNIT_SIZE, FontWeight::REGULAR, GRAY_40),
                ),
            ],
        )
    };
    let mut columns = vec![
        column(numerals.days, "days"),
        numeral(":".to_owned()),
        column(numerals.hours, "hours"),
    ];
    if let Some(minutes) = numerals.minutes {
        columns.push(numeral(":".to_owned()));
        columns.push(column(minutes, "minutes"));
    }
    row(
        props!(
            gap: BMM101_COUNTDOWN_GAP,
            cross_align: CrossAlign::Start,
            justify_content: Justify::Center
        ),
        columns,
    )
}

/// A caption on the left; the value over its sub-line, right-aligned, on the right.
fn bmm101_row(caption: &str, value: Option<String>, sub: Option<String>) -> Node {
    let value = value.unwrap_or_else(|| parts::NOT_AVAILABLE.to_owned());
    let sub = sub.unwrap_or_default();
    row(
        props!(justify_content: Justify::SpaceBetween, cross_align: CrossAlign::Start),
        [
            bmm101_slot(
                BMM101_ROW_SLOT,
                CrossAlign::Start,
                bmm101_text(caption, BMM101_ROW_SIZE, FontWeight::SEMIBOLD, GRAY_10),
            ),
            col(
                props!(gap: BMM101_SUB_GAP, cross_align: CrossAlign::End),
                [
                    bmm101_slot(
                        BMM101_ROW_SLOT,
                        CrossAlign::End,
                        bmm101_text(value, BMM101_ROW_SIZE, FontWeight::SEMIBOLD, GRAY_10),
                    ),
                    bmm101_slot(
                        BMM101_SUB_SLOT,
                        CrossAlign::End,
                        bmm101_text(sub, BMM101_SUB_SIZE, FontWeight::REGULAR, GRAY_40),
                    ),
                ],
            ),
        ],
    )
}

fn bmm101_divider() -> Node {
    col(props!(height: 1.0, background: GRAY_90), [])
}

/// BMM101's 480×320 frame: the title, the countdown,
/// then the predicted moment and the blocks left as ruled rows.
fn bmm101(weight: FontWeight, data: Option<&DisplayData>) -> Node {
    col(
        props!(background: BLACK, padding: BMM101_EDGE, gap: BMM101_EDGE),
        [
            bmm101_slot(
                BMM101_TITLE_SLOT,
                CrossAlign::Start,
                bmm101_text(
                    "Halving Countdown",
                    BMM101_TITLE_SIZE,
                    FontWeight::SEMIBOLD,
                    GRAY_40,
                ),
            ),
            bmm101_countdown(numerals(data), weight),
            col(
                props!(flex: 1.0, justify_content: Justify::SpaceBetween),
                [
                    bmm101_divider(),
                    bmm101_row(
                        "Predicted Date",
                        data.map(|d| d.predicted_date.clone()),
                        data.map(|d| d.predicted_time.clone()),
                    ),
                    bmm101_divider(),
                    bmm101_row(
                        "Blocks Remaining",
                        data.map(DisplayData::blocks_left),
                        data.map(DisplayData::target_caption),
                    ),
                ],
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::system::{NumberFormat, SnapshotBuilder, TimeFormat};

    use super::*;
    use crate::model::SizeBucket;
    use crate::screens::fixtures;

    fn install_prague(time_format: TimeFormat) {
        assets::init_test_registrars();
        system::set_current(
            SnapshotBuilder::new()
                .timezone("Europe/Prague")
                .time_format(time_format)
                .number_format(NumberFormat::CommaGroupDotDecimal)
                .build(),
        );
    }

    /// Every string the tree would draw, in tree order.
    fn texts(node: &Node) -> Vec<String> {
        let mut out = Vec::new();
        collect_texts(node, &mut out);
        out
    }

    fn collect_texts(node: &Node, out: &mut Vec<String>) {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                for child in children {
                    collect_texts(child, out);
                }
            }
            Node::Paragraph { spans, .. } => {
                out.push(spans.iter().map(|span| span.text.as_str()).collect());
            }
            _ => {}
        }
    }

    fn healthy_at(bucket: SizeBucket) -> Vec<String> {
        let view = fixtures::healthy(fixtures::at_bucket(bucket), fixtures::default_params());
        texts(&halving_view(&view))
    }

    #[test]
    fn the_countdown_counts_from_the_render_moment() {
        install_prague(TimeFormat::Hour24);
        let texts = healthy_at(SizeBucket::Small);
        for numeral in ["631", "00", "50"] {
            assert!(texts.contains(&numeral.to_owned()), "{texts:?}");
        }
    }

    #[test]
    fn the_tiled_sizes_add_the_predicted_moment_and_the_blocks_left() {
        install_prague(TimeFormat::Hour24);
        for bucket in [SizeBucket::Full, SizeBucket::Large] {
            let texts = healthy_at(bucket);
            for expected in [
                "Predicted Date",
                "13.04.2028",
                "15:59 Prague (+2)",
                "Blocks Remaining",
                "90,869",
                "Target #1,050,000",
            ] {
                assert!(
                    texts.contains(&expected.to_owned()),
                    "{bucket:?}: {texts:?}"
                );
            }
        }
    }

    #[test]
    fn a_twelve_hour_clock_marks_the_predicted_time() {
        install_prague(TimeFormat::Hour12);
        assert!(
            healthy_at(SizeBucket::Full).contains(&"03:59 PM Prague (+2)".to_owned()),
            "the meridiem sits between the time and the zone"
        );
    }

    #[test]
    fn the_compact_sizes_draw_the_countdown_alone() {
        install_prague(TimeFormat::Hour24);
        for bucket in [SizeBucket::Medium, SizeBucket::Small] {
            let texts = healthy_at(bucket);
            assert!(texts.contains(&"Halving Countdown".to_owned()));
            assert!(!texts.contains(&"Predicted Date".to_owned()), "{bucket:?}");
        }
    }

    #[test]
    fn the_round_face_draws_the_countdown_alone() {
        install_prague(TimeFormat::Hour24);
        let view = fixtures::healthy(fixtures::round(480), fixtures::default_params());
        let texts = texts(&halving_view(&view));
        assert!(texts.contains(&"631".to_owned()), "{texts:?}");
        assert!(!texts.contains(&"Predicted Date".to_owned()));
    }

    #[test]
    fn only_the_narrow_layouts_drop_the_minutes_from_four_digit_days() {
        install_prague(TimeFormat::Hour24);
        let era_start = |viewport| {
            texts(&halving_view(&fixtures::era_start(
                viewport,
                fixtures::default_params(),
            )))
        };
        for viewport in [
            fixtures::at_bucket(SizeBucket::Medium),
            fixtures::at_bucket(SizeBucket::Small),
            fixtures::rectangular(320, 240),
            fixtures::round(480),
        ] {
            let texts = era_start(viewport);
            assert!(
                texts.contains(&"1458".to_owned()) && texts.contains(&"23".to_owned()),
                "{viewport:?}: {texts:?}"
            );
            assert!(
                !texts.contains(&"59".to_owned()) && !texts.contains(&"Min.".to_owned()),
                "{viewport:?}: {texts:?}"
            );
        }
        for bucket in [SizeBucket::Full, SizeBucket::Large, SizeBucket::Bmm101] {
            let texts = era_start(fixtures::at_bucket(bucket));
            assert!(texts.contains(&"59".to_owned()), "{bucket:?}: {texts:?}");
        }
    }

    #[test]
    fn the_bmm101_frame_reads_top_down_as_designed() {
        install_prague(TimeFormat::Hour24);
        assert_eq!(
            healthy_at(SizeBucket::Bmm101),
            [
                "Halving Countdown",
                "631",
                "days",
                ":",
                "00",
                "hours",
                ":",
                "50",
                "minutes",
                "Predicted Date",
                "13.04.2028",
                "15:59 Prague (+2)",
                "Blocks Remaining",
                "90,869",
                "Target #1,050,000",
            ]
        );
    }

    #[test]
    fn without_a_prediction_every_value_reads_as_a_dash() {
        install_prague(TimeFormat::Hour24);
        for bucket in [SizeBucket::Full, SizeBucket::Bmm101] {
            let view = fixtures::loading(fixtures::at_bucket(bucket), fixtures::default_params());
            let texts = texts(&halving_view(&view));
            let dashes = texts.iter().filter(|text| *text == "-").count();
            assert_eq!(
                dashes, 5,
                "three numerals and two values, {bucket:?}: {texts:?}"
            );
        }
    }

    #[test]
    fn each_status_but_ready_floats_its_tag_over_the_layout() {
        install_prague(TimeFormat::Hour24);
        let params = fixtures::default_params();
        let tagged = |view: &ViewData| {
            let Node::Column(_, children) = halving_view(view) else {
                panic!("BUG: every layout is a column");
            };
            children.len()
        };
        for bucket in [SizeBucket::Large, SizeBucket::Bmm101] {
            let viewport = fixtures::at_bucket(bucket);
            let bare = tagged(&fixtures::healthy(viewport, params.clone()));
            for view in [
                fixtures::failed(viewport, params.clone()),
                fixtures::stale(viewport, params.clone()),
                fixtures::rate_limited(viewport, params.clone()),
            ] {
                assert_eq!(tagged(&view), bare + 1, "{bucket:?}: {:?}", view.status);
            }
        }
    }
}
