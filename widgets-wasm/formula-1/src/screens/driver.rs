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

//! One driver's season and their measurements.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::images::{self, ImageKind};
use crate::model::{DriverStats, ImageUrl, SizeBucket};
use crate::screens::parts::{self, LabelWeight, color, font, space};

/// Everything the screen draws.
#[derive(Clone, Debug)]
pub struct DriverViewData {
    pub bucket: SizeBucket,
    pub driver: Option<DriverStats>,
    /// From the teams table, which is the only resource naming one.
    pub team_logo_url: ImageUrl,
}

/// What the server had nothing for.
const UNKNOWN: &str = "N/A";

/// The driver's name above their team, at the widest frame's size.
const NAME_FULL: u32 = 32;
/// The headshot, square on the widest frame and shorter below it.
const PHOTO_FULL: f32 = 327.0;
/// Square, like the portraits a provider serves — a box of other
/// proportions bars the livery down both edges of every face, which reads
/// as a fault rather than as a frame.
///
/// Bounded by the frame as well, not only by the design: the portrait is
/// the body's tallest child and the stats column stretches to it, so one
/// taller than 300 takes the last stat out of view with it.
const PHOTO_LARGE: f32 = 280.0;
const PHOTO_SMALL: f32 = 160.0;
const PHOTO_RADIUS: f32 = 4.0;
/// The constructor mark trailing the header.
const TEAM_MARK: f32 = 40.0;
/// The nationality flag's height, which holds as the stat text shrinks;
/// its width follows the artwork the deployment sends.
const FLAG: f32 = 16.8;

/// Which pieces a frame keeps, across the four ported layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Portrait {
    /// No room for a headshot; the stats have the frame to themselves.
    Absent,
    /// The headshot alone; the header carries the driver's name.
    Photo,
    /// The headshot over the driver's name, and their team under it.
    Named,
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    portrait: Portrait,
    /// The widest frame splits its stats in two; the rest run one column.
    split_stats: bool,
    stat_font: u32,
    labels: LabelWeight,
    /// What a stat's value seats beside its label.
    /// The team and the race engineer are the server's
    /// own strings, and the longest of them barely fits.
    value_chars: usize,
}

fn layout(bucket: SizeBucket) -> Layout {
    match bucket {
        SizeBucket::Full => Layout {
            portrait: Portrait::Named,
            split_stats: true,
            stat_font: font::TITLE,
            labels: LabelWeight::Muted,
            value_chars: 20,
        },
        SizeBucket::Large => Layout {
            portrait: Portrait::Named,
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Muted,
            value_chars: 22,
        },
        SizeBucket::Medium => Layout {
            portrait: Portrait::Photo,
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Muted,
            value_chars: 24,
        },
        SizeBucket::Small => Layout {
            portrait: Portrait::Absent,
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Strong,
            value_chars: 20,
        },
    }
}

fn count(value: Option<u8>) -> String {
    value.map_or_else(|| UNKNOWN.to_owned(), |value| fmt!("{}", value))
}

fn year(value: Option<u16>) -> String {
    value.map_or_else(|| UNKNOWN.to_owned(), |value| fmt!("{}", value))
}

fn stat(label: &str, value: &str, layout: Layout) -> Node {
    parts::stat_row(
        label,
        value,
        layout.value_chars,
        layout.stat_font,
        layout.labels,
    )
}

/// Who the driver drives as — the name block already says this
/// wherever a frame draws one.
fn naming_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    vec![
        stat("Team", &driver.team, layout),
        stat("Number", &fmt!("#{}", driver.number.get()), layout),
    ]
}

/// Where the driver stands this season.
fn season_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    vec![
        stat("Ranking", &fmt!("#{}", driver.ranking), layout),
        stat("Points", &fmt!("{}", driver.points), layout),
    ]
}

/// What the driver has won.
fn record_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    vec![
        stat("Grand Prix Wins", &count(driver.gp_wins), layout),
        stat("World Titles", &count(driver.world_titles), layout),
    ]
}

/// Who the driver is, in the operator's own units.
fn person_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    let weight = driver
        .weight
        .map_or_else(|| UNKNOWN.to_owned(), |it| it.format(0));
    let height = driver
        .height
        .map_or_else(|| UNKNOWN.to_owned(), |it| it.format_short(0));
    let engineer = driver
        .race_engineer
        .clone()
        .unwrap_or_else(|| UNKNOWN.to_owned());
    vec![
        stat("Age", &count(driver.age), layout),
        stat("Weight", &weight, layout),
        stat("Height", &height, layout),
        nationality_row(driver, layout),
        stat("Race Engineer", &engineer, layout),
        stat("F1 Debut", &year(driver.debut_year), layout),
    ]
}

/// The nationality, with room held for the flag beside it.
fn nationality_row(driver: &DriverStats, layout: Layout) -> Node {
    row(
        props!(
            flex: 1.0,
            gap: space::GAP * 2.0,
            cross_align: CrossAlign::Center,
            justify_content: Justify::SpaceBetween
        ),
        [
            text(
                "Nationality",
                style!(size: layout.stat_font, color: color::TEXT_MUTED),
            ),
            row(
                props!(gap: space::GAP, cross_align: CrossAlign::Center),
                [
                    text(
                        parts::truncate(&driver.nationality, layout.value_chars),
                        style!(size: layout.stat_font, weight: FontWeight::SEMIBOLD, color: color::TEXT, line_height: 1.0),
                    ),
                    parts::flag(FLAG, &driver.nationality_flag_url),
                ],
            ),
        ],
    )
}

/// The headshot, in a box the driver's livery holds until it arrives.
fn photo(driver: &DriverStats, width: f32, height: f32) -> Node {
    let frame = props!(
        width: width,
        height: height,
        background: driver.team_color,
        border_radius: PHOTO_RADIUS
    );
    match images::resolve(ImageKind::Headshot, &driver.headshot_url) {
        Some(resolved) => {
            let (x, y, drawn_width, drawn_height) =
                parts::contained((resolved.width, resolved.height), width, height);
            canvas(
                frame,
                [Draw::bitmap_id(
                    x,
                    y,
                    drawn_width,
                    drawn_height,
                    Some(resolved.bitmap),
                )],
            )
        }
        None => col(frame, []),
    }
}

/// The headshot where the frame has room for one, with the driver named
/// under it where there is room for that too.
fn portrait(driver: &DriverStats, layout: Layout) -> Option<Node> {
    match layout.portrait {
        Portrait::Absent => None,
        Portrait::Photo => Some(photo(driver, PHOTO_SMALL, PHOTO_SMALL)),
        Portrait::Named => {
            let (width, height, name_size) = if layout.split_stats {
                (PHOTO_FULL, PHOTO_FULL, NAME_FULL)
            } else {
                (PHOTO_LARGE, PHOTO_LARGE, font::TITLE)
            };
            let mut named = vec![text(
                parts::truncate(
                    &fmt!("{} #{}", driver.name, driver.number.get()),
                    layout.value_chars,
                ),
                style!(size: name_size, weight: FontWeight::SEMIBOLD, color: color::TEXT),
            )];
            // The widest frame names the team in the header's mark
            // instead, so only the frame below it repeats the team here.
            if !layout.split_stats {
                named.push(text(
                    parts::truncate(&driver.team, layout.value_chars),
                    style!(size: font::ROW, color: color::TEXT_MUTED),
                ));
            }
            Some(col(
                props!(gap: space::GAP * 2.0),
                [
                    photo(driver, width, height),
                    col(props!(gap: space::GAP / 2.0), named),
                ],
            ))
        }
    }
}

fn header(driver: &DriverStats, logo: &ImageUrl, bucket: SizeBucket) -> Node {
    let mark = parts::team_mark(TEAM_MARK, logo, driver.team_color);
    let content = match bucket {
        SizeBucket::Full | SizeBucket::Large => {
            vec![parts::title("Driver stats"), spacer(1.0), mark]
        }
        SizeBucket::Medium => vec![
            parts::subtitle(&fmt!("{} #{}", driver.name, driver.number.get()), bucket),
            spacer(1.0),
            mark,
        ],
        SizeBucket::Small => vec![parts::subtitle(
            &fmt!("{} #{}", driver.name, driver.number.get()),
            bucket,
        )],
    };
    parts::header(content, false, bucket)
}

/// The stats, split into two columns where the frame is wide enough.
fn stats(driver: &DriverStats, layout: Layout) -> Node {
    if layout.split_stats {
        let mut left = naming_rows(driver, layout);
        left.extend(season_rows(driver, layout));
        left.extend(record_rows(driver, layout));
        return row(
            props!(flex: 1.0, gap: space::GAP * 6.0),
            [
                parts::stat_col(left),
                parts::stat_col(person_rows(driver, layout)),
            ],
        );
    }
    // The narrower frames drop what will not fit, keeping the season
    // before the person: a rank and a score outrank a shirt size.
    let mut rows = Vec::new();
    if layout.portrait == Portrait::Named {
        rows.extend(season_rows(driver, layout));
        rows.extend(record_rows(driver, layout));
        rows.extend(person_rows(driver, layout));
    } else {
        rows.extend(naming_rows(driver, layout));
        rows.extend(season_rows(driver, layout));
        rows.extend(record_rows(driver, layout).into_iter().take(1));
    }
    parts::stat_col(rows)
}

/// One driver, or the empty state while nothing has arrived.
#[must_use]
pub fn driver_view(view: &DriverViewData) -> Node {
    let layout = layout(view.bucket);
    let Some(driver) = view.driver.as_ref() else {
        return parts::frame(
            vec![
                parts::header(vec![parts::title("Driver stats")], false, view.bucket),
                center(
                    props!(flex: 1.0),
                    [text(
                        "Driver unavailable",
                        style!(size: font::ROW, color: color::TEXT_MUTED),
                    )],
                ),
            ],
            view.bucket,
        );
    };

    let body = match portrait(driver, layout) {
        None => stats(driver, layout),
        Some(portrait) => {
            // The single-column frames give the seam's spare width to
            // the stats: their longest row barely seats its engineer.
            let gap = if layout.split_stats {
                space::GAP * 6.0
            } else {
                space::GAP * 2.0
            };
            row(
                props!(flex: 1.0, gap: gap),
                [portrait, stats(driver, layout)],
            )
        }
    };
    parts::frame(
        vec![header(driver, &view.team_logo_url, view.bucket), body],
        view.bucket,
    )
}

#[cfg(test)]
mod tests {
    use super::{PHOTO_LARGE, Portrait, TEAM_MARK, layout, parts, parts::space};
    use crate::model::SizeBucket;
    use crate::screens::fixtures;

    /// What sits under the photo on the frames that name the team there:
    /// `GAP * 2`, [`super::font::TITLE`], `GAP / 2`, then
    /// [`super::font::ROW`].
    const PHOTO_CAPTION: f32 = 64.0;

    /// Nothing in the tree shrinks text, so the budget is the only thing
    /// keeping a long value from pushing the column off the frame. Each
    /// was read off the gallery's `Driver Widest` ruler at the narrowest
    /// frame of its band.
    #[test]
    fn every_frame_seats_the_longest_value_it_draws_and_cuts_what_overruns() {
        let drivers = fixtures::drivers();
        let longest_team = drivers
            .iter()
            .map(|driver| driver.team.as_str())
            .max_by_key(|team| team.chars().count())
            .expect("BUG: the fixtures name at least one team");
        let longest_engineer = drivers
            .iter()
            .filter_map(|driver| driver.race_engineer.as_deref())
            .max_by_key(|name| name.chars().count())
            .expect("BUG: the fixtures name at least one engineer");

        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
        ] {
            let chars = layout(bucket).value_chars;
            assert_eq!(
                parts::truncate(longest_team, chars),
                longest_team,
                "{bucket:?} seats {chars} and draws the team on every frame",
            );
            // The narrow frames drop the engineer's row rather than cut it.
            if matches!(bucket, SizeBucket::Full | SizeBucket::Large) {
                assert_eq!(
                    parts::truncate(longest_engineer, chars),
                    longest_engineer,
                    "{bucket:?} seats {chars} and should read `{longest_engineer}` whole",
                );
            }
            let overrun = "x".repeat(chars + 1);
            assert!(
                parts::truncate(&overrun, chars).ends_with('\u{2026}'),
                "{bucket:?} must still cut what overruns its {chars}",
            );
        }
    }

    /// The stats column stretches to the portrait beside it,
    /// so a portrait past the frame puts the last stat outside it —
    /// which is how this frame lost its bottom row once.
    #[test]
    fn the_portrait_leaves_the_stats_inside_the_frame() {
        let bucket = SizeBucket::Large;
        let (_, frame) = bucket.design_size();
        let body = frame
            - space::padding(bucket) * 2.0
            - TEAM_MARK
            - space::below_header(bucket)
            - space::GAP;
        let portrait = PHOTO_LARGE + PHOTO_CAPTION;
        assert!(
            portrait <= body,
            "the portrait takes {portrait} of the {body} under the header",
        );
    }

    /// The per-frame rules the port keeps.
    #[test]
    fn every_frame_matches_the_ported_layout() {
        let expected = [
            (SizeBucket::Full, Portrait::Named, true),
            (SizeBucket::Large, Portrait::Named, false),
            (SizeBucket::Medium, Portrait::Photo, false),
            (SizeBucket::Small, Portrait::Absent, false),
        ];
        for (bucket, portrait, split) in expected {
            let layout = layout(bucket);
            assert_eq!(layout.portrait, portrait, "{bucket:?} portrait");
            assert_eq!(layout.split_stats, split, "{bucket:?} split stats");
        }
    }
}
