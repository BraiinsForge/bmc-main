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
use crate::screens::parts::{self, LabelWeight, Yields, color, font, space};

/// Everything the screen draws.
#[derive(Clone, Debug)]
pub struct DriverViewData {
    pub bucket: SizeBucket,
    pub driver: Option<DriverStats>,
    /// From the teams table, which is the only resource naming one.
    pub team_logo_url: ImageUrl,
}

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
/// The largest square on the 8 px grid whose caption still leaves the stats inside the frame.
const PHOTO_BMM101: f32 = 184.0;
const PHOTO_RADIUS: f32 = 4.0;
/// The constructor mark trailing the header.
const TEAM_MARK: f32 = 40.0;
/// BMM101's mark, which its 14 px header line has room for.
const TEAM_MARK_BMM101: f32 = 18.0;
/// The nationality flag's height across the Deck frames,
/// which holds as their stat text shrinks from 24 to 20;
/// its width follows the artwork the deployment sends.
const FLAG: f32 = 16.8;

/// Which pieces a frame keeps, and at what size.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Portrait {
    /// No room for a headshot; the stats have the frame to themselves.
    Absent,
    /// The headshot alone, this wide; the header carries the driver's name.
    Photo(f32),
    /// The headshot over the driver's name, and their team under it.
    Named {
        photo: f32,
        name: u32,
        team: Option<u32>,
    },
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    portrait: Portrait,
    /// The widest frame splits its stats in two; the rest run one column.
    split_stats: bool,
    stat_font: u32,
    labels: LabelWeight,
    team_mark: f32,
    flag: f32,
}

fn layout(bucket: SizeBucket) -> Layout {
    match bucket {
        SizeBucket::Full => Layout {
            portrait: Portrait::Named {
                photo: PHOTO_FULL,
                name: NAME_FULL,
                // The header's mark names the team.
                team: None,
            },
            split_stats: true,
            stat_font: font::TITLE,
            labels: LabelWeight::Muted,
            team_mark: TEAM_MARK,
            flag: FLAG,
        },
        SizeBucket::Large => Layout {
            portrait: Portrait::Named {
                photo: PHOTO_LARGE,
                name: font::TITLE,
                team: Some(font::ROW),
            },
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Muted,
            team_mark: TEAM_MARK,
            flag: FLAG,
        },
        SizeBucket::Medium => Layout {
            portrait: Portrait::Photo(PHOTO_SMALL),
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Muted,
            team_mark: TEAM_MARK,
            flag: FLAG,
        },
        SizeBucket::Small => Layout {
            portrait: Portrait::Absent,
            split_stats: false,
            stat_font: font::ROW,
            labels: LabelWeight::Strong,
            team_mark: TEAM_MARK,
            flag: FLAG,
        },
        SizeBucket::Bmm101 => Layout {
            portrait: Portrait::Named {
                photo: PHOTO_BMM101,
                name: font::ROW,
                team: Some(font::ROW_BMM101),
            },
            split_stats: false,
            stat_font: font::ROW_BMM101,
            labels: LabelWeight::Muted,
            team_mark: TEAM_MARK_BMM101,
            flag: parts::flag_height(font::ROW_BMM101),
        },
    }
}

fn count(value: Option<u8>) -> Option<String> {
    value.map(|value| fmt!("{}", value))
}

fn year(value: Option<u16>) -> Option<String> {
    value.map(|value| fmt!("{}", value))
}

/// The driver's name, carrying their number where the payload named one.
/// The number is part of this line rather than a value of its own,
/// so an absent one leaves the name standing alone, not a placeholder.
fn name_line(driver: &DriverStats) -> String {
    driver.number.map_or_else(
        || driver.name.clone(),
        |number| fmt!("{} #{}", driver.name, number.get()),
    )
}

fn stat(label: &str, value: Option<&str>, layout: Layout) -> Node {
    parts::stat_row(label, value, layout.stat_font, layout.labels, Yields::Value)
}

/// Who the driver drives as — the name block already says this
/// wherever a frame draws one.
fn naming_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    let number = driver.number.map(|number| fmt!("#{}", number.get()));
    vec![
        stat("Team", Some(&driver.team), layout),
        stat("Number", number.as_deref(), layout),
    ]
}

/// Where the driver stands this season.
fn season_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    let ranking = driver.ranking.map(|rank| fmt!("#{}", rank));
    let points = driver.points.map(parts::points);
    vec![
        stat("Ranking", ranking.as_deref(), layout),
        stat("Points", points.as_deref(), layout),
    ]
}

/// What the driver has won.
fn record_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    vec![
        stat("Grand Prix Wins", count(driver.gp_wins).as_deref(), layout),
        stat(
            "World Titles",
            count(driver.world_titles).as_deref(),
            layout,
        ),
    ]
}

/// Who the driver is, in the operator's own units.
fn person_rows(driver: &DriverStats, layout: Layout) -> Vec<Node> {
    let weight = driver.weight.map(|it| it.format(0));
    let height = driver.height.map(|it| it.format_short(0));
    vec![
        stat("Age", count(driver.age).as_deref(), layout),
        stat("Weight", weight.as_deref(), layout),
        stat("Height", height.as_deref(), layout),
        nationality_row(driver, layout),
        stat("Race Engineer", driver.race_engineer.as_deref(), layout),
        stat("F1 Debut", year(driver.debut_year).as_deref(), layout),
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
                        &driver.nationality,
                        style!(size: layout.stat_font, weight: FontWeight::SEMIBOLD, color: color::TEXT, line_height: 1.0, text_overflow: TextOverflow::Ellipsis),
                    ),
                    parts::flag(layout.flag, &driver.nationality_flag_url),
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
        Portrait::Photo(side) => Some(photo(driver, side, side)),
        Portrait::Named {
            photo: side,
            name,
            team,
        } => {
            let mut named = vec![text(
                name_line(driver),
                style!(size: name, weight: FontWeight::SEMIBOLD, color: color::TEXT, text_overflow: TextOverflow::Ellipsis),
            )];
            if let Some(team) = team {
                named.push(text(
                    &driver.team,
                    style!(size: team, color: color::TEXT_MUTED, text_overflow: TextOverflow::Ellipsis),
                ));
            }
            // As wide as the photo, so a long name is cut under it
            // rather than widening the column into the stats.
            Some(col(
                props!(width: side, gap: space::GAP * 2.0),
                [
                    photo(driver, side, side),
                    col(props!(gap: space::GAP / 2.0), named),
                ],
            ))
        }
    }
}

fn header(driver: &DriverStats, logo: &ImageUrl, bucket: SizeBucket, layout: Layout) -> Node {
    let mark = parts::team_mark(layout.team_mark, logo, driver.team_color);
    let content = match bucket {
        SizeBucket::Full | SizeBucket::Large | SizeBucket::Bmm101 => {
            vec![parts::title("Driver stats", bucket), spacer(1.0), mark]
        }
        SizeBucket::Medium => vec![
            parts::yielding(vec![parts::subtitle(&name_line(driver), bucket)]),
            mark,
        ],
        SizeBucket::Small => vec![parts::subtitle(&name_line(driver), bucket)],
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
    if matches!(layout.portrait, Portrait::Named { .. }) {
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
                parts::header(
                    vec![parts::title("Driver stats", view.bucket)],
                    false,
                    view.bucket,
                ),
                center(
                    props!(flex: 1.0),
                    [text(
                        "Driver unavailable",
                        style!(size: font::notice(view.bucket), color: color::TEXT_MUTED),
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
        vec![
            header(driver, &view.team_logo_url, view.bucket, layout),
            body,
        ],
        view.bucket,
    )
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::tree::TextStyle;

    use super::{Portrait, layout, name_line};
    use crate::model::SizeBucket;
    use crate::screens::fixtures;
    use crate::screens::parts::{font, space};

    /// The height a line of `size` takes at the renderer's default line height.
    fn line(size: u32) -> f64 {
        f64::from(size) * f64::from(TextStyle::default().line_height)
    }

    /// There is no zeroth car, so a driver the payload gave no number
    /// keeps their name to themselves rather than trailing a bare `#`.
    #[test]
    fn a_driver_without_a_number_is_named_without_one() {
        let mut driver = fixtures::drivers()
            .into_iter()
            .next()
            .expect("BUG: the fixtures name a driver");
        assert!(
            name_line(&driver).contains('#'),
            "a driver with a number is named with it"
        );
        driver.number = None;
        assert_eq!(name_line(&driver), driver.name);
    }

    /// The stats column stretches to the portrait beside it,
    /// so a portrait past the frame puts the last stat outside it —
    /// which is how this frame lost its bottom row once.
    #[test]
    fn the_portrait_leaves_the_stats_inside_the_frame() {
        for bucket in SizeBucket::ALL {
            let layout = layout(bucket);
            let Portrait::Named { photo, name, team } = layout.portrait else {
                continue;
            };
            // Split stats keep their own height rather than stretching to the portrait.
            if layout.split_stats {
                continue;
            }
            let (_, frame) = bucket.design_size();
            let header = f64::from(layout.team_mark).max(line(font::header(bucket)));
            let body = f64::from(
                frame - space::padding(bucket) * 2.0 - space::below_header(bucket) - space::GAP,
            ) - header;
            // The photo, `GAP * 2`, the name, then `GAP / 2` and the team where it is drawn.
            let caption = team.map_or(0.0, |team| f64::from(space::GAP / 2.0) + line(team));
            let portrait = f64::from(photo + space::GAP * 2.0) + line(name) + caption;
            assert!(
                portrait <= body,
                "{bucket:?}: the portrait takes {portrait} of the {body} under the header",
            );
        }
    }

    /// The per-frame rules the port keeps.
    #[test]
    fn every_frame_matches_the_ported_layout() {
        let named = |portrait| matches!(portrait, Portrait::Named { .. });
        let photo = |portrait| matches!(portrait, Portrait::Photo(_));
        let absent = |portrait| matches!(portrait, Portrait::Absent);
        let expected: [(SizeBucket, fn(Portrait) -> bool, bool); 5] = [
            (SizeBucket::Full, named, true),
            (SizeBucket::Large, named, false),
            (SizeBucket::Medium, photo, false),
            (SizeBucket::Small, absent, false),
            (SizeBucket::Bmm101, named, false),
        ];
        for (bucket, portrait, split) in expected {
            let layout = layout(bucket);
            assert!(portrait(layout.portrait), "{bucket:?} portrait");
            assert_eq!(layout.split_stats, split, "{bucket:?} split stats");
        }
    }
}
