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
use crate::model::{DriverStats, SizeBucket};
use crate::screens::parts::{self, LabelWeight, color, font, space};

/// Everything the screen draws.
#[derive(Clone, Debug)]
pub struct DriverViewData {
    pub bucket: SizeBucket,
    pub driver: Option<DriverStats>,
}

/// What the server had nothing for.
const UNKNOWN: &str = "N/A";

/// The driver's name above their team, at the widest frame's size.
const NAME_FULL: u32 = 32;
/// The headshot, square on the widest frame and shorter below it.
const PHOTO_FULL: f32 = 327.0;
const PHOTO_LARGE_WIDTH: f32 = 280.0;
const PHOTO_LARGE_HEIGHT: f32 = 330.0;
const PHOTO_SMALL: f32 = 160.0;
const PHOTO_RADIUS: f32 = 4.0;
/// The constructor mark trailing the header.
const TEAM_MARK: f32 = 40.0;
/// The nationality flag, which keeps its size as the stat text shrinks.
const FLAG: f32 = 24.0;

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
}

fn layout(bucket: SizeBucket) -> Layout {
    match bucket {
        SizeBucket::Full => Layout {
            portrait: Portrait::Named,
            split_stats: true,
            stat_font: font::TITLE,
            labels: LabelWeight::Muted,
        },
        SizeBucket::Large => Layout {
            portrait: Portrait::Named,
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Muted,
        },
        SizeBucket::Medium => Layout {
            portrait: Portrait::Photo,
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Muted,
        },
        SizeBucket::Small => Layout {
            portrait: Portrait::Absent,
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Strong,
        },
    }
}

fn count(value: Option<u8>) -> String {
    value.map_or_else(|| UNKNOWN.to_owned(), |value| fmt!("{}", value))
}

fn year(value: Option<u16>) -> String {
    value.map_or_else(|| UNKNOWN.to_owned(), |value| fmt!("{}", value))
}

fn stat(label: &str, value: String, layout: Layout) -> Node {
    parts::stat_row(label, value, layout.stat_font, layout.labels)
}

/// Who the driver drives as — the name block already says this
/// wherever a frame draws one.
fn naming_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    vec![
        stat("Team", driver.team.clone(), layout),
        stat("Number", fmt!("#{}", driver.number.get()), layout),
    ]
}

/// Where the driver stands this season.
fn season_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    vec![
        stat("Ranking", fmt!("#{}", driver.ranking), layout),
        stat("Points", fmt!("{}", driver.points), layout),
    ]
}

/// What the driver has won.
fn record_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    vec![
        stat("Grand Prix Wins", count(driver.gp_wins), layout),
        stat("World Titles", count(driver.world_titles), layout),
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
        stat("Age", count(driver.age), layout),
        stat("Weight", weight, layout),
        stat("Height", height, layout),
        nationality_row(driver, layout),
        stat("Race Engineer", engineer, layout),
        stat("F1 Debut", year(driver.debut_year), layout),
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
                        driver.nationality.as_str(),
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
        Some(resolved) => canvas(
            frame,
            [Draw::bitmap_id(
                0.0,
                0.0,
                width,
                height,
                Some(resolved.bitmap),
            )],
        ),
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
                (PHOTO_LARGE_WIDTH, PHOTO_LARGE_HEIGHT, font::TITLE)
            };
            let mut named = vec![text(
                fmt!("{} #{}", driver.name, driver.number.get()),
                style!(size: name_size, weight: FontWeight::SEMIBOLD, color: color::TEXT),
            )];
            // The widest frame names the team in the header's mark
            // instead, so only the frame below it repeats the team here.
            if !layout.split_stats {
                named.push(text(
                    driver.team.as_str(),
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

fn header(driver: &DriverStats, bucket: SizeBucket) -> Node {
    // The driver payload names no team logo URL, so the mark draws the
    // embedded artwork or the livery.
    let mark = parts::team_mark(
        TEAM_MARK,
        &driver.team,
        &crate::model::ImageUrl::default(),
        driver.team_color,
    );
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
    parts::frame(vec![header(driver, view.bucket), body], view.bucket)
}

#[cfg(test)]
mod tests {
    use super::{Portrait, layout};
    use crate::model::SizeBucket;

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
