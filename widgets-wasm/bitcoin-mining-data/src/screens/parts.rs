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

#[expect(
    clippy::wildcard_imports,
    reason = "screen fragments use the SDK's tree builders, macros, and tokens throughout"
)]
use bmc_wasm_sdk::*;

use bmc_wasm_sdk::typography::unbroken;

use crate::chart;
use crate::model::Series;

pub mod color {
    use bmc_wasm_sdk::{Color, GRAY_40, GRAY_60, GRAY_80, GRAY_90, WHITE};

    pub const BACKGROUND: Color = Color::from_hex(0x00_00_00);
    pub const CARD: Color = Color::from_rgba(0x16, 0x16, 0x16, 179);
    pub const BORDER: Color = GRAY_90;
    pub const GRID: Color = GRAY_80;
    pub const LABEL: Color = GRAY_40;
    pub const ABSENT: Color = GRAY_60;
    pub const VALUE: Color = WHITE;
    pub const UP: Color = Color::from_hex(0x34_C0_6A);
    pub const DOWN: Color = Color::from_hex(0xF9_53_55);
}

pub const GAP: f32 = 8.0;
pub(super) const LOADING: &str = "--";
pub(super) const NOT_AVAILABLE: &str = "N/A";
const BADGE_PADDING: f32 = 4.0;
const SECS_PER_DAY: u64 = 86_400;
const HALF_DAY_SECS: u64 = SECS_PER_DAY / 2;

#[must_use]
pub fn font_height(size: u32) -> f32 {
    f32::from(u16::try_from(size).expect("BUG: authored font sizes fit in u16"))
}

#[must_use]
pub fn bordered(children: impl IntoIterator<Item = Node>) -> Node {
    col(
        props!(
            background: color::CARD,
            border_radius: 8.0,
            border_width: 1.0,
            border_color: color::BORDER,
            flex: 1.0
        ),
        children,
    )
}

#[derive(Clone, Copy, Debug)]
pub struct TitleSize {
    pub icon: f32,
    pub label: u32,
}

#[must_use]
pub fn title(
    icon: &Svg,
    icon_color: Color,
    label: &str,
    trailing: Option<Node>,
    size: TitleSize,
) -> Node {
    let mut children = vec![
        canvas(
            props!(width: size.icon, height: size.icon),
            [Draw::svg_contain(icon, size.icon, icon_color).with_anti_alias()],
        ),
        text(
            unbroken(label),
            style!(size: size.label, weight: FontWeight::SEMIBOLD, color: color::LABEL, line_height: 1.0),
        ),
    ];
    children.push(spacer(1.0));
    if let Some(trailing) = trailing {
        children.push(trailing);
    }
    row(props!(cross_align: CrossAlign::Center, gap: 8.0), children)
}

#[must_use]
pub fn primary(value: &str, size: u32, value_color: Color) -> Node {
    text(
        unbroken(value),
        style!(size: size, weight: FontWeight::SEMIBOLD, color: value_color, line_height: 1.0),
    )
}

#[derive(Clone, Copy, Debug)]
pub struct ValueSizes {
    pub number: u32,
    pub unit: u32,
}

/// A number in the value voice trailed by its unit — "56.20 USD/PH/Day".
/// The unit takes the label's size and colour but keeps the number's weight:
/// a span inherits its paragraph's, and a regular one inside semibold cannot be asked for.
/// One paragraph, so the unit sits on the number's baseline;
/// two nodes could not, `CrossAlign` having no baseline option.
#[must_use]
pub fn quantity(number: &str, unit: &str, sizes: ValueSizes, value_color: Color) -> Node {
    paragraph(
        style!(size: sizes.number, weight: FontWeight::SEMIBOLD, color: value_color, line_height: 1.0),
        [
            span(unbroken(number), ()),
            span(
                unbroken(&fmt!(" {unit}")),
                style!(size: sizes.unit, color: color::LABEL),
            ),
        ],
    )
}

#[must_use]
pub fn unavailable(size: u32) -> Node {
    text(
        NOT_AVAILABLE,
        style!(size: size, color: color::ABSENT, line_height: 1.0),
    )
}

#[must_use]
pub fn muted(value: &str, size: u32) -> Node {
    text(
        unbroken(value),
        style!(size: size, color: color::LABEL, line_height: 1.0),
    )
}

#[must_use]
pub fn divider() -> Node {
    col(props!(height: 1.0, background: color::BORDER), [])
}

fn percent_decimals(value: f64) -> u32 {
    if value.abs() >= 99.0 {
        return 0;
    }
    let rounded_hundredths = (value * 100.0).round();
    if rounded_hundredths.rem_euclid(10.0) <= f64::EPSILON {
        1
    } else {
        2
    }
}

fn percent_label(value: f64) -> String {
    let sign = if value > 0.0 { "+" } else { "" };
    let number = format_number!(value, percent_decimals(value));
    fmt!("{}{}%", sign, number)
}

#[must_use]
pub fn percent_badge(value: Option<f64>, font_size: u32) -> Node {
    let Some(value) = value else {
        let size = font_height(font_size) + BADGE_PADDING * 2.0;
        return col(props!(width: size, height: size), []);
    };
    let (background, foreground) = if value >= 0.0 {
        (Color::from_hex(0x0E_3F_25), color::UP)
    } else {
        (Color::from_hex(0x51_0B_27), Color::from_hex(0xFF_83_A0))
    };
    row(
        props!(background: background, border_radius: 4.0, padding: BADGE_PADDING),
        [
            spacer(4.0),
            text(
                percent_label(value),
                style!(size: font_size, weight: FontWeight::SEMIBOLD, color: foreground, line_height: 1.0),
            ),
            spacer(4.0),
        ],
    )
}

#[must_use]
pub fn trend(value: Option<f64>, show_24h: bool, font_size: u32) -> Node {
    let mut children = Vec::new();
    if show_24h {
        children.push(muted("24h", font_size));
    }
    children.push(percent_badge(value, font_size));
    row(props!(gap: GAP, cross_align: CrossAlign::Center), children)
}

#[must_use]
pub fn stat_row(label: &str, value: &str, value_color: Color) -> Node {
    row(
        props!(cross_align: CrossAlign::Center, gap: GAP),
        [
            text(
                unbroken(label),
                style!(size: 24, color: color::LABEL, flex: 1.0, line_height: 1.0),
            ),
            text(
                unbroken(value),
                style!(size: 24, weight: FontWeight::SEMIBOLD, color: value_color, align: TextAlign::Right, line_height: 1.0),
            ),
        ],
    )
}

#[must_use]
pub fn adjustment_row(
    label: &str,
    when: &str,
    percent: Option<f64>,
    label_size: u32,
    time_size: u32,
    badge_size: u32,
    when_color: Color,
) -> Node {
    // The time and the percentage are the row's data, so they hold their width;
    // the label gives way if the row runs out.
    row(
        props!(cross_align: CrossAlign::Center, gap: GAP),
        [
            text(
                label,
                style!(size: label_size, color: color::LABEL, line_height: 1.0, text_overflow: TextOverflow::Ellipsis),
            ),
            spacer(1.0),
            text(
                unbroken(when),
                style!(size: time_size, color: when_color, line_height: 1.0),
            ),
            percent_badge(percent, badge_size),
        ],
    )
}

#[derive(Clone, Copy, Debug)]
pub struct ChartBox {
    pub width: f32,
    pub height: f32,
    pub gridlines: usize,
    pub corner_label: Option<&'static str>,
}

const CORNER_LABEL_SIZE: u32 = 14;
const CORNER_LABEL_INSET_TOP: f32 = 5.0;
const CORNER_LABEL_INSET_RIGHT: f32 = 4.0;
/// The label sits on the line where the series peaks, so it carries a dark outline.
/// Only canvas text has one: the host paints a paragraph's colour and nothing around it.
const CORNER_LABEL_OUTLINE_ALPHA: f32 = 0.75;
const CORNER_LABEL_OUTLINE_WIDTH: f32 = 2.0;

#[must_use]
pub fn sparkline(series: &Series, chart: ChartBox, force_color: Option<Color>) -> Node {
    let ChartBox {
        width,
        height,
        gridlines,
        corner_label,
    } = chart;
    let line = chart::series_points(&series.values, width, height, 2.0);
    if line.len() < 2 {
        return col(props!(width: width, height: height), []);
    }
    let color = force_color.unwrap_or_else(|| {
        if chart::is_rising(&series.values) {
            color::UP
        } else {
            color::DOWN
        }
    });
    let mut area = line.clone();
    area.push((width, height));
    area.push((0.0, height));
    let mut draws: Vec<Draw> = (1..=gridlines)
        .map(|index| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a handful of gridlines is exact in f32"
            )]
            let y = height * index as f32 / (gridlines + 1) as f32;
            path!(vec![(0.0, y), (width, y)], stroke: 1.0, color: color::GRID, dashed: (3.0, 3.0))
        })
        .collect();
    draws.push(fill!(area, linear: (color.with_alpha(0.22), color.with_alpha(0.0)), smooth));
    draws.push(path!(line, stroke: 2.0, color: color, smooth));
    if let Some(label) = corner_label {
        draws.push(Draw::text(
            width - CORNER_LABEL_INSET_RIGHT,
            CORNER_LABEL_INSET_TOP,
            label,
            style!(
                size: CORNER_LABEL_SIZE,
                color: color::LABEL,
                align: TextAlign::Right,
                outline_color: color::BACKGROUND.with_alpha(CORNER_LABEL_OUTLINE_ALPHA),
                outline_width: CORNER_LABEL_OUTLINE_WIDTH
            ),
        ));
    }
    canvas(props!(width: width, height: height), draws)
}

#[must_use]
pub fn unavailable_chart(width: f32, height: f32) -> Node {
    center(
        props!(width: width, height: height),
        [text(
            "No history",
            style!(size: 16, color: color::ABSENT, line_height: 1.0),
        )],
    )
}

#[must_use]
pub fn series_change_percent(series: &Series) -> Option<f64> {
    if series.values.len() < 2 {
        return None;
    }
    let (Some(first), Some(last)) = (series.values.first(), series.values.last()) else {
        return None;
    };
    if first.abs() <= f64::EPSILON {
        return None;
    }
    let percent = (last / first - 1.0) * 100.0;
    percent.is_finite().then_some(percent)
}

const COMPACT_UNITS: [(f64, &str); 4] = [
    (1.0, ""),
    (1_000.0, "K"),
    (1_000_000.0, "M"),
    (1_000_000_000.0, "B"),
];

/// Difficulty is read in the tera band today; the next two keep it readable as it climbs.
const DIFFICULTY_UNITS: [(f64, &str); 3] = [(1e12, "T"), (1e15, "P"), (1e18, "E")];

/// `value` under the largest unit it reaches, promoting once more
/// when rounding to `decimals` would show a thousand of the lower one.
fn scaled(value: f64, decimals: u32, units: &[(f64, &'static str)]) -> (String, &'static str) {
    let magnitude = value.abs();
    let mut unit_index = 0;
    for (index, (threshold, _)) in units.iter().enumerate().skip(1) {
        if magnitude < *threshold {
            break;
        }
        unit_index = index;
    }

    let mut scaled = value / units[unit_index].0;
    let decimal_scale = 10_f64
        .powi(i32::try_from(decimals).expect("BUG: scaled-number decimal precision fits i32"));
    let rounded = (scaled * decimal_scale).round() / decimal_scale;
    if rounded.abs() >= 1_000.0 && unit_index + 1 < units.len() {
        unit_index += 1;
        scaled = value / units[unit_index].0;
    }

    (format_number!(scaled, decimals), units[unit_index].1)
}

#[must_use]
pub fn compact_number(value: f64, decimals: u32) -> String {
    let (number, unit) = scaled(value, decimals, &COMPACT_UNITS);
    fmt!("{number}{unit}")
}

#[must_use]
pub fn difficulty_parts(difficulty: f64) -> (String, &'static str) {
    scaled(difficulty, 1, &DIFFICULTY_UNITS)
}

#[must_use]
pub fn compact_revenue(value: f64) -> String {
    let decimals = if value.abs() >= 1_000_000_000.0 { 0 } else { 2 };
    fmt!("{} USD", compact_number(value, decimals))
}

fn whole_days(seconds: u64) -> u64 {
    seconds.saturating_add(HALF_DAY_SECS) / SECS_PER_DAY
}

#[must_use]
pub fn relative_days(at: i64, now: i64) -> String {
    let seconds = at - now;
    let days = whole_days(seconds.unsigned_abs());
    if seconds >= 0 {
        fmt!("~ in {} days", days)
    } else {
        fmt!("{} days ago", days)
    }
}

#[must_use]
pub fn previous_adjustment_days(block: u64, block_time_secs: u64) -> String {
    let days = whole_days(block.saturating_mul(block_time_secs));
    fmt!("{} days ago", days)
}

#[must_use]
pub fn duration_minutes(seconds: u64) -> String {
    let minutes = seconds / 60;
    let remaining = seconds % 60;
    fmt!(
        "{}{}:{}{}",
        if minutes < 10 { "0" } else { "" },
        minutes,
        if remaining < 10 { "0" } else { "" },
        remaining
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The draw kind and colour of each canvas entry, in paint order.
    fn paint_order(draws: &[Draw]) -> Vec<(&'static str, Color)> {
        draws
            .iter()
            .map(|draw| match draw {
                Draw::Path {
                    paint: PathPaint::Stroke { color, dash, .. },
                    ..
                } => (if dash.is_some() { "dashed" } else { "solid" }, *color),
                Draw::Path {
                    paint: PathPaint::Fill(fill),
                    ..
                } => ("fill", fill.primary_color()),
                Draw::Text { style, .. } => ("text", style.color),
                other => panic!("BUG: a sparkline draws paths and text only, not {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_sparkline_draws_its_gridlines_under_the_area_and_line() {
        let series = Series {
            values: vec![1.0, 2.0, 3.0],
        };
        let chart = ChartBox {
            width: 100.0,
            height: 50.0,
            gridlines: 3,
            corner_label: None,
        };

        let Node::Canvas { draws, .. } = sparkline(&series, chart, None) else {
            panic!("BUG: a sparkline is drawn on a canvas");
        };
        assert_eq!(
            paint_order(&draws),
            vec![
                ("dashed", color::GRID),
                ("dashed", color::GRID),
                ("dashed", color::GRID),
                ("fill", color::UP.with_alpha(0.22)),
                ("solid", color::UP),
            ]
        );
    }

    #[test]
    fn a_corner_label_is_the_last_thing_a_sparkline_draws() {
        let series = Series {
            values: vec![3.0, 2.0, 1.0],
        };
        let chart = ChartBox {
            width: 100.0,
            height: 50.0,
            gridlines: 1,
            corner_label: Some("1 year"),
        };

        let Node::Canvas { draws, .. } = sparkline(&series, chart, None) else {
            panic!("BUG: a sparkline is drawn on a canvas");
        };
        assert_eq!(
            paint_order(&draws),
            vec![
                ("dashed", color::GRID),
                ("fill", color::DOWN.with_alpha(0.22)),
                ("solid", color::DOWN),
                ("text", color::LABEL),
            ]
        );
    }

    #[test]
    fn series_change_uses_first_and_last_samples() {
        let series = Series {
            values: vec![100.0, 110.0, 105.0],
        };
        assert!((series_change_percent(&series).expect("BUG: change exists") - 5.0).abs() < 1e-9);
    }

    #[test]
    fn one_sample_has_no_trend() {
        assert_eq!(
            series_change_percent(&Series {
                values: vec![100.0]
            }),
            None
        );
    }

    #[test]
    fn overflowing_change_has_no_trend() {
        assert_eq!(
            series_change_percent(&Series {
                values: vec![1.0, f64::MAX]
            }),
            None
        );
    }

    #[test]
    fn missing_percent_reserves_an_empty_badge_slot() {
        let font_size = 24;
        let expected_size = font_height(font_size) + BADGE_PADDING * 2.0;
        assert!(
            matches!(
                percent_badge(None, font_size),
                Node::Column(props, children)
                    if props.width == expected_size
                        && props.height == expected_size
                        && children.is_empty()
            ),
            "missing percentages must reserve the badge box without rendering text"
        );
    }

    #[test]
    fn previous_adjustment_uses_elapsed_epoch_time() {
        assert_eq!(previous_adjustment_days(1_293, 559), "8 days ago");
    }

    #[test]
    fn both_adjustment_rows_round_to_the_nearest_day() {
        let half_day = i64::try_from(HALF_DAY_SECS).expect("BUG: half a day fits i64");
        assert_eq!(relative_days(half_day, 0), "~ in 1 days");
        assert_eq!(relative_days(half_day - 1, 0), "~ in 0 days");
        assert_eq!(previous_adjustment_days(1, HALF_DAY_SECS), "1 days ago");
        assert_eq!(previous_adjustment_days(1, HALF_DAY_SECS - 1), "0 days ago");
    }

    #[test]
    fn block_time_pads_both_components() {
        assert_eq!(duration_minutes(9 * 60 + 9), "09:09");
    }

    #[test]
    fn difficulty_climbs_through_tera_peta_and_exa() {
        assert_eq!(difficulty_parts(129.7e12).1, "T");
        assert_eq!(difficulty_parts(999.96e12).1, "P");
        assert_eq!(difficulty_parts(2.5e18).1, "E");
    }

    #[test]
    fn large_numbers_use_compact_units() {
        assert!(compact_number(49_350_000.0, 2).ends_with('M'));
        assert!(compact_number(9_999_999_999.99, 2).ends_with('B'));
    }

    #[test]
    fn rounded_compact_numbers_promote_to_the_next_unit() {
        assert_eq!(
            [
                compact_number(999.999, 2),
                compact_number(999_999.99, 2),
                compact_number(999_999_999.99, 2),
                compact_number(-999_999.99, 2),
            ],
            [
                fmt!("{}K", format_number!(1.0, 2)),
                fmt!("{}M", format_number!(1.0, 2)),
                fmt!("{}B", format_number!(1.0, 2)),
                fmt!("{}M", format_number!(-1.0, 2)),
            ]
        );
    }

    #[test]
    fn compact_numbers_below_the_rounding_boundary_keep_their_unit() {
        assert_eq!(compact_number(999.994, 2), format_number!(999.994, 2));
        assert_eq!(
            compact_number(999_994.0, 2),
            fmt!("{}K", format_number!(999.994, 2))
        );
    }

    #[test]
    fn extreme_percentages_drop_fractional_precision() {
        assert_eq!(percent_label(-99.99), "-100%");
        let positive: String = percent_label(999.99)
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        assert_eq!(positive, "+1000%");
    }

    #[test]
    fn percentage_precision_follows_the_numeric_hundredths() {
        assert_eq!(
            [
                percent_decimals(5.0),
                percent_decimals(5.31),
                percent_decimals(-2.8),
            ],
            [1, 2, 1]
        );
    }
}
