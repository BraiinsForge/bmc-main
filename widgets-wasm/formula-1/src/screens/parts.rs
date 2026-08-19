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

use crate::images::{self, ImageKind};
use crate::model::{ImageUrl, SizeBucket};
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

/// How far below its own box's middle a glyph renders, as a share of
/// the font's size, so artwork set beside text drops by this much to
/// meet the letters.
///
/// Measured by eye against the testbed rather than derived: the
/// renderer's line box is not symmetric about the letters, and nothing
/// in the tree reports where a baseline landed. Text drawn onto a
/// canvas needs none of this — `Draw::text` takes a `valign` the
/// renderer resolves to the font's own baseline.
pub const GLYPH_DROP: f32 = 0.04;

pub mod space {
    use crate::model::SizeBucket;

    pub const GAP: f32 = 8.0;

    /// The frame's padding, which the half-height frames tighten.
    #[must_use]
    pub fn padding(bucket: SizeBucket) -> f32 {
        match bucket {
            SizeBucket::Small | SizeBucket::Medium => 16.0,
            SizeBucket::Large | SizeBucket::Full => 24.0,
        }
    }

    /// What the header holds off the content under it,
    /// beyond the frame's gap.
    #[must_use]
    pub fn below_header(bucket: SizeBucket) -> f32 {
        match bucket {
            SizeBucket::Medium => 16.0 - GAP,
            SizeBucket::Small => 24.0 - GAP,
            SizeBucket::Large | SizeBucket::Full => 28.0 - GAP,
        }
    }
}

/// The swept bars, at the artwork's own 10:1 aspect — fitting them to a
/// square collapses them to a dash.
const STRIPE_HEIGHT: f32 = 24.0;
const STRIPE_WIDTH: f32 = STRIPE_HEIGHT * 10.0;

/// The frame every screen sits in: the design's black field, padded.
#[must_use]
pub fn frame(children: Vec<Node>, bucket: SizeBucket) -> Node {
    col(
        props!(
            background: color::BG,
            padding: space::padding(bucket),
            gap: space::GAP,
            flex: 1.0
        ),
        children,
    )
}

/// The `F1` mark, whatever the screen names itself, and optionally the
/// swept bars. Which frames carry the bars is the screen's call.
#[must_use]
pub fn header(content: Vec<Node>, stripe: bool, bucket: SizeBucket) -> Node {
    let mut children = vec![text(
        "F1",
        style!(size: font::TITLE, weight: FontWeight::BOLD, color: color::TEXT),
    )];
    children.extend(content);
    if stripe {
        // Absolutely positioned, as the design bleeds the bars through
        // the frame's padding to the widget's right edge.
        children.push(canvas(
            props!(
                width: STRIPE_WIDTH,
                height: STRIPE_HEIGHT,
                inset_top: 0.0,
                inset_right: -space::padding(bucket)
            ),
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
    col(
        props!(),
        [
            row(
                props!(gap: space::GAP * 2.0, cross_align: CrossAlign::Center),
                children,
            ),
            col(props!(height: space::below_header(bucket)), []),
        ],
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
        style!(size: size, weight: FontWeight::SEMIBOLD, color: color::TEXT, line_height: 1.0),
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
    // Both halves stay on one line.
    // A flex item will not shrink below its own content, so either half
    // wrapping would floor the row at two lines,
    // and the column it sits in would overrun its frame.
    let label = match weight {
        LabelWeight::Strong => text(
            label,
            style!(size: size, weight: FontWeight::SEMIBOLD, color: color::TEXT, line_height: 1.0, text_overflow: TextOverflow::Ellipsis),
        ),
        LabelWeight::Muted => text(
            label,
            style!(size: size, color: color::TEXT_MUTED, line_height: 1.0, text_overflow: TextOverflow::Ellipsis),
        ),
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
                style!(size: size, weight: FontWeight::SEMIBOLD, color: color::TEXT, align: TextAlign::Right, line_height: 1.0, text_overflow: TextOverflow::Ellipsis),
            ),
        ],
    )
}

/// Cut a label to what its column seats.
///
/// A text node keeps its content's width whatever box surrounds it.
/// Nothing shrinks, so an overlong label pushes every column after it
/// off the frame, and any column holding a string the server chose
/// needs cutting here first.
#[must_use]
pub fn truncate(label: &str, max_chars: usize) -> String {
    if label.chars().count() <= max_chars {
        return label.to_owned();
    }
    let mut out: String = label.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
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

/// A race weekend's span, which drops to one day when it is only one.
/// The opening day carries no month unless the weekend crosses one.
#[must_use]
pub fn date_range(start: CalendarDate, end: Option<CalendarDate>) -> String {
    // The sport writes a round day-first; only the US setting reverses that,
    // and a year-first ordering needs a year this label does not carry.
    let month_first = matches!(
        system::current().date_format().unwrap_or_default(),
        DateFormat::MDYyyySlash
    );
    let Some(end) = end.filter(|end| *end != start) else {
        return day_and_month(start, month_first);
    };
    let opening = if month_first || start.month != end.month {
        day_and_month(start, month_first)
    } else {
        fmt!("{}", start.day)
    };
    fmt!("{} \u{2013} {}", opening, day_and_month(end, month_first))
}

/// The largest box of `bitmap`'s own proportions that fits inside
/// `width`×`height`, and the offset that centres it there.
#[must_use]
pub fn contained(bitmap: (u32, u32), width: f32, height: f32) -> (f32, f32, f32, f32) {
    let (bitmap_width, bitmap_height) = bitmap;
    if bitmap_width == 0 || bitmap_height == 0 {
        return (0.0, 0.0, width, height);
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "bitmap dimensions are bounded by the kind's decode box, far inside f32"
    )]
    let (drawn_width, drawn_height) = (bitmap_width as f32, bitmap_height as f32);
    let scale = (width / drawn_width).min(height / drawn_height);
    let (fitted_width, fitted_height) = (drawn_width * scale, drawn_height * scale);
    (
        (width - fitted_width) / 2.0,
        (height - fitted_height) / 2.0,
        fitted_width,
        fitted_height,
    )
}

/// A cached remote image centred in a fixed box, as large as its own
/// proportions let it sit there, or `fallback` while nothing has arrived.
///
/// The box is what the layout reserves, so nothing the deployment sends
/// can move a row: a mark wider than it is tall pads above and below
/// rather than pushing its neighbours, and the placeholder holding the
/// space is the same size the image will occupy.
#[must_use]
pub fn remote_image(
    kind: ImageKind,
    url: &ImageUrl,
    width: f32,
    height: f32,
    fallback: Node,
) -> Node {
    let Some(resolved) = images::resolve(kind, url) else {
        return fallback;
    };
    let (x, y, drawn_width, drawn_height) =
        contained((resolved.width, resolved.height), width, height);
    canvas(
        props!(width: width, height: height),
        [Draw::bitmap_id(
            x,
            y,
            drawn_width,
            drawn_height,
            Some(resolved.bitmap),
        )],
    )
}

/// `url` drawn `height` tall and as wide as its own proportions ask,
/// or `fallback` while nothing has arrived.
///
/// The decode fits the source inside its kind's box without distorting it,
/// so the bitmap's own dimensions are the ratio to honour. Drawing into a
/// box we picked instead is what stretches it.
#[must_use]
pub fn image_at_height(kind: ImageKind, url: &ImageUrl, height: f32, fallback: Node) -> Node {
    let Some(resolved) = images::resolve(kind, url) else {
        return fallback;
    };
    #[expect(
        clippy::cast_precision_loss,
        reason = "bitmap dimensions are bounded by the kind's decode box, far inside f32"
    )]
    let width = if resolved.height == 0 {
        height
    } else {
        height * (resolved.width as f32) / (resolved.height as f32)
    };
    canvas(
        props!(width: width, height: height),
        [Draw::bitmap_id(
            0.0,
            0.0,
            width,
            height,
            Some(resolved.bitmap),
        )],
    )
}

/// A team's mark: the server's logo once cached, the livery colour until
/// then. No constructor's artwork ships with the widget, so a mark whose
/// fetch has not answered is a coloured disc.
#[must_use]
pub fn team_mark(size: f32, url: &ImageUrl, livery: Color) -> Node {
    remote_image(
        ImageKind::TeamLogo,
        url,
        size,
        size,
        image_placeholder(size, Some(livery)),
    )
}

/// A country's flag at `height`, as wide as its own proportions make it.
///
/// The one image allowed to size itself. Everything else draws into a box
/// the layout fixed, so no payload can shift a row; a flag is let off that
/// because one provider serves one flag set, and a set is internally
/// consistent. Should that ever fail, the cost is a row of flags whose
/// widths differ — visible at a glance, not a silent wrong.
///
/// Two things put a flag level with that text. The text needs
/// `line_height: 1.0`, or its box stands a fifth taller than its
/// glyphs and the flag centres against the slack; and the flag drops
/// by [`GLYPH_DROP`] to meet letters that sit low in their own box.
#[must_use]
pub fn flag(height: f32, url: &ImageUrl) -> Node {
    // Square while nothing has arrived: the shape our own fixtures take,
    // and no worse a guess than any other for artwork not yet seen.
    let placeholder = col(
        props!(width: height, height: height, background: color::PLACEHOLDER),
        [],
    );
    col(
        props!(),
        [
            // Twice the drop wanted: the box is centred as a whole, so
            // padding over the flag lowers it by half the padding.
            col(props!(height: height * GLYPH_DROP * 2.0), []),
            image_at_height(ImageKind::Flag, url, height, placeholder),
        ],
    )
}

/// Holds an image's box before it arrives, so the row does not reflow
/// when it lands. A livery fills a team's square; anything else is neutral.
#[must_use]
pub fn image_placeholder(size: f32, livery: Option<Color>) -> Node {
    // A livery stands in for a constructor's mark, and a saturated square
    // of it reads as something the screen meant to draw. Rounding says the
    // slot is waiting; the neutral fill is quiet enough to leave square.
    let radius = if livery.is_some() { size / 2.0 } else { 0.0 };
    col(
        props!(
            width: size,
            height: size,
            border_radius: radius,
            background: livery.unwrap_or(color::PLACEHOLDER)
        ),
        [],
    )
}

#[cfg(test)]
mod tests {
    use super::{CalendarDate, DateFormat, clock, contained, date_range, system};
    use crate::screens::fixtures::{weekend_day, weekend_time};

    /// Every image but a flag draws through this, so a box that stretched
    /// its contents would distort the whole widget at once — which is how
    /// the flags, the marks and the headshots each went out squashed.
    #[test]
    fn a_box_pads_around_its_image_rather_than_stretching_it() {
        // Wider than its box: bars above and below, full width.
        assert_eq!(contained((100, 50), 40.0, 40.0), (0.0, 10.0, 40.0, 20.0));
        // Taller than its box: bars either side, full height.
        assert_eq!(contained((50, 100), 40.0, 40.0), (10.0, 0.0, 20.0, 40.0));
        // A square portrait in the frame that used to squash it.
        assert_eq!(
            contained((330, 330), 280.0, 300.0),
            (0.0, 10.0, 280.0, 280.0)
        );
        // Nothing decoded yet: the box stands rather than dividing by zero.
        assert_eq!(contained((0, 0), 40.0, 30.0), (0.0, 0.0, 40.0, 30.0));
    }

    /// Put the operator on `format`, since the snapshot is held per thread
    /// and `cargo test` hands one thread to several tests in turn.
    fn dates(format: DateFormat) {
        system::set_current(system::SnapshotBuilder::new().date_format(format).build());
    }

    /// A day in a month the weekend fixture does not run in.
    fn day_of(month: u8, day: u8) -> CalendarDate {
        CalendarDate {
            year: 2026,
            month,
            day,
            weekday: 0,
        }
    }

    #[test]
    fn a_weekend_spanning_days_names_its_month_once() {
        dates(DateFormat::DdMmYyyyDot);
        let range = date_range(weekend_day(21), Some(weekend_day(23)));
        assert_eq!(range, "21 \u{2013} 23 Aug");
    }

    #[test]
    fn a_weekend_of_one_day_reads_as_that_day() {
        dates(DateFormat::DdMmYyyyDot);
        assert_eq!(date_range(weekend_day(23), None), "23 Aug");
        assert_eq!(date_range(weekend_day(23), Some(weekend_day(23))), "23 Aug",);
    }

    #[test]
    fn a_weekend_crossing_a_month_names_both_of_them() {
        let across = (day_of(10, 30), Some(day_of(11, 1)));

        dates(DateFormat::DdMmYyyyDot);
        assert_eq!(
            date_range(across.0, across.1),
            "30 Oct \u{2013} 1 Nov",
            "dropping the opening month reads as two days of November",
        );

        dates(DateFormat::MDYyyySlash);
        assert_eq!(date_range(across.0, across.1), "Oct 30 \u{2013} Nov 1");
        assert_eq!(
            date_range(weekend_day(21), Some(weekend_day(23))),
            "Aug 21 \u{2013} Aug 23",
            "month-first names the month on both ends within one month too",
        );
    }

    #[test]
    fn the_clock_keeps_the_leading_zero_of_a_minute() {
        assert_eq!(clock(weekend_time(23, 13, 0)), "13:00");
        assert_eq!(clock(weekend_time(21, 9, 5)), "9:05");
    }
}
