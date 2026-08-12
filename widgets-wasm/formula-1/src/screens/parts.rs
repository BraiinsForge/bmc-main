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

//! Fragments every screen shares, and the design tokens behind them.
//! Each takes its geometry from parameters; the screen decides which.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use bmc_wasm_sdk::system::{self, DateFormat, TimeFormat};

use crate::model::SizeBucket;
use crate::screens::icons;

// The Figma variables these screens draw from:
// white text, Gray/40 muted, Gray/90 rules.
pub mod color {
    use bmc_wasm_sdk::{Color, GRAY_40, GRAY_90, WHITE};

    pub const BG: Color = Color::from_hex(0x00_00_00);
    pub const TEXT: Color = WHITE;
    pub const TEXT_MUTED: Color = GRAY_40;
    pub const DIVIDER: Color = GRAY_90;
    pub const BRAND: Color = Color::from_hex(0xE1_06_00);
    pub const PLACEHOLDER: Color = GRAY_90;
}

pub mod font {
    pub const TITLE: u32 = 24;
    /// Every table row, at every frame.
    pub const ROW: u32 = 20;
    /// The subtitle, which the narrow frames set smaller.
    pub const SUBTITLE_MEDIUM: u32 = 20;
    pub const SUBTITLE_SMALL: u32 = 18;
}

pub mod space {
    pub const PADDING: f32 = 16.0;
    pub const GAP: f32 = 8.0;
}

/// The swept bars, at the artwork's own 10:1 aspect — fitting them to a
/// square collapses them to a dash.
const STRIPE_HEIGHT: f32 = 24.0;
const STRIPE_WIDTH: f32 = STRIPE_HEIGHT * 10.0;

/// The frame every screen sits in: the design's black field, padded.
#[must_use]
pub fn frame(children: Vec<Node>) -> Node {
    col(
        props!(
            background: color::BG,
            padding: space::PADDING,
            gap: space::GAP,
            flex: 1.0
        ),
        children,
    )
}

/// The `F1` mark, whatever the screen names itself, and optionally the
/// swept bars. Which frames carry the bars is the screen's call.
#[must_use]
pub fn header(content: Vec<Node>, stripe: bool) -> Node {
    let mut children = vec![text(
        "F1",
        style!(size: font::TITLE, weight: FontWeight::BOLD, color: color::TEXT),
    )];
    children.extend(content);
    if stripe {
        children.push(spacer(1.0));
        children.push(canvas(
            props!(width: STRIPE_WIDTH, height: STRIPE_HEIGHT),
            [Draw::svg(
                0.0,
                0.0,
                STRIPE_WIDTH,
                STRIPE_HEIGHT,
                &icons::BRAND_STRIPE,
                color::BRAND,
            )
            .with_anti_alias()],
        ));
    }
    row(
        props!(gap: space::GAP * 2.0, cross_align: CrossAlign::Center),
        children,
    )
}

/// What the screen is, in the design's quieter weight.
#[must_use]
pub fn title(label: &str) -> Node {
    text(label, style!(size: font::TITLE, color: color::TEXT_MUTED))
}

/// What the screen is showing — a date range, a Grand Prix, a country.
#[must_use]
pub fn subtitle(label: &str, bucket: SizeBucket) -> Node {
    let size = match bucket {
        SizeBucket::Full | SizeBucket::Large => font::TITLE,
        SizeBucket::Medium => font::SUBTITLE_MEDIUM,
        SizeBucket::Small => font::SUBTITLE_SMALL,
    };
    text(
        label,
        style!(size: size, weight: FontWeight::SEMIBOLD, color: color::TEXT),
    )
}

#[must_use]
pub fn divider() -> Node {
    col(props!(height: 1.0, background: color::DIVIDER), [])
}

/// How a stat's label is set against its value.
#[derive(Clone, Copy, Debug)]
pub enum LabelWeight {
    /// The design's quieter label beside a bold value.
    Muted,
    /// The smallest frames set both alike, so the pair reads as one line.
    Strong,
}

/// A label and its value, pushed to opposite edges of the row.
#[must_use]
pub fn stat_row(label: &str, value: String, size: u32, weight: LabelWeight) -> Node {
    let label = match weight {
        LabelWeight::Strong => text(
            label,
            style!(size: size, weight: FontWeight::SEMIBOLD, color: color::TEXT),
        ),
        LabelWeight::Muted => text(label, style!(size: size, color: color::TEXT_MUTED)),
    };
    row(
        props!(
            flex: 1.0,
            gap: space::GAP * 2.0,
            cross_align: CrossAlign::Center,
            justify_content: Justify::SpaceBetween
        ),
        [
            label,
            text(
                value,
                style!(size: size, weight: FontWeight::SEMIBOLD, color: color::TEXT, align: TextAlign::Right),
            ),
        ],
    )
}

/// Rows sharing the column's height, ruled off from one another.
#[must_use]
pub fn stat_col(rows: Vec<Node>) -> Node {
    let mut children = Vec::new();
    for (index, stat) in rows.into_iter().enumerate() {
        if index > 0 {
            children.push(divider());
        }
        children.push(stat);
    }
    col(props!(flex: 1.0), children)
}

/// A wall clock in the operator's 12- or 24-hour preference.
#[must_use]
pub fn clock(at: LocalDateTime) -> String {
    let minute = at.minute;
    // `fmt!` takes no width specifier, so the leading zero is our own.
    let lead = if minute < 10 { "0" } else { "" };
    match system::current().time_format().unwrap_or_default() {
        TimeFormat::Hour24 => fmt!("{}:{}{}", at.hour, lead, minute),
        TimeFormat::Hour12 => {
            let (hour, half) = match at.hour {
                0 => (12, "AM"),
                hour @ 1..=11 => (hour, "AM"),
                12 => (12, "PM"),
                hour => (hour - 12, "PM"),
            };
            fmt!("{}:{}{} {}", hour, lead, minute, half)
        }
    }
}

/// A day as `14 Jul`, or `Jul 14` where the operator puts the month first.
fn day_and_month(at: CalendarDate, month_first: bool) -> String {
    let (day, month) = (at.day, at.month_short());
    if month_first {
        fmt!("{} {}", month, day)
    } else {
        fmt!("{} {}", day, month)
    }
}

/// A race weekend's span, which drops to one day when it is only one:
/// the opening day carries no month, the closing day does.
#[must_use]
pub fn date_range(start: CalendarDate, end: Option<CalendarDate>) -> String {
    let month_first = matches!(
        system::current().date_format().unwrap_or_default(),
        DateFormat::MDYyyySlash
    );
    let Some(end) = end.filter(|end| *end != start) else {
        return day_and_month(start, month_first);
    };
    let opening = if month_first {
        day_and_month(start, month_first)
    } else {
        fmt!("{}", start.day)
    };
    fmt!("{} \u{2013} {}", opening, day_and_month(end, month_first))
}

/// A team's mark, or its livery colour where this build carries no
/// artwork for the team — a new constructor, or one renamed since.
#[must_use]
pub fn team_mark(size: f32, team_name: &str, livery: Color) -> Node {
    let Some(mark) = icons::team_mark(team_name) else {
        return image_placeholder(size, Some(livery));
    };
    canvas(
        props!(width: size, height: size),
        [Draw::bitmap(0.0, 0.0, size, size, mark)],
    )
}

/// A country's flag, at the 10:7 box a flag is drawn in.
#[must_use]
pub fn flag(width: f32) -> Node {
    col(
        props!(width: width, height: width * 0.7, background: color::PLACEHOLDER),
        [],
    )
}

/// Holds an image's box before it arrives, so the row does not reflow
/// when it lands. A livery fills a team's square; anything else is neutral.
#[must_use]
pub fn image_placeholder(size: f32, livery: Option<Color>) -> Node {
    col(
        props!(
            width: size,
            height: size,
            background: livery.unwrap_or(color::PLACEHOLDER)
        ),
        [],
    )
}

#[cfg(test)]
mod tests {
    use super::{clock, date_range};
    use crate::screens::fixtures::{weekend_day, weekend_time};

    #[test]
    fn a_weekend_spanning_days_names_its_month_once() {
        let range = date_range(weekend_day(21), Some(weekend_day(23)));
        assert_eq!(range, "21 \u{2013} 23 Aug");
    }

    #[test]
    fn a_weekend_of_one_day_reads_as_that_day() {
        assert_eq!(date_range(weekend_day(23), None), "23 Aug");
        assert_eq!(date_range(weekend_day(23), Some(weekend_day(23))), "23 Aug",);
    }

    #[test]
    fn the_clock_keeps_the_leading_zero_of_a_minute() {
        assert_eq!(clock(weekend_time(23, 13, 0)), "13:00");
        assert_eq!(clock(weekend_time(21, 9, 5)), "9:05");
    }
}
