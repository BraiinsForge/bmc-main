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

//! The upcoming race weekend: where it is run, and when.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::images::ImageKind;
use crate::model::{NextRace, SizeBucket};
use crate::screens::parts::{self, LabelWeight, color, font, space};

/// Everything the screen draws.
#[derive(Clone, Debug)]
pub struct NextRaceViewData {
    pub bucket: SizeBucket,
    pub race: Option<NextRace>,
}

/// What the server had nothing for.
const UNKNOWN: &str = "N/A";

/// The design's gap between columns.
const COLUMN_GAP: f32 = 40.0;
/// The Grand Prix name, the largest type on the widget.
const GP_NAME: u32 = 32;
/// What the design floats the Grand Prix block off the header, past
/// the frame's own gap.
const GP_BLOCK_LEAD: f32 = 12.0;
/// What the design leaves between the Grand Prix block and the columns.
const GP_BLOCK_GAP: f32 = 48.0;
const TRACK_MAP_RADIUS: f32 = 8.0;

/// What the schedule column beside the circuit's stats carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Schedule {
    /// No column at all: the smallest frame keeps the stats alone.
    Absent,
    Sessions,
    /// The frames whose header has no room for the weekend's dates
    /// open the schedule with them instead.
    DatedSessions,
}

/// Which pieces a frame keeps, across the four ported layouts.
#[derive(Clone, Copy, Debug)]
struct Layout {
    stripe: bool,
    /// The Grand Prix name and country, above the columns.
    gp_block: bool,
    schedule: Schedule,
    /// The circuit outline, which only the widest frame has room for.
    track_map: bool,
    info_font: u32,
    labels: LabelWeight,
}

fn layout(bucket: SizeBucket) -> Layout {
    match bucket {
        SizeBucket::Full => Layout {
            stripe: true,
            gp_block: true,
            schedule: Schedule::Sessions,
            track_map: true,
            info_font: font::TITLE,
            labels: LabelWeight::Muted,
        },
        SizeBucket::Large => Layout {
            stripe: true,
            gp_block: true,
            schedule: Schedule::DatedSessions,
            track_map: false,
            // Smaller than the widest frame's, because this one carries the
            // schedule beside the stats in half the width: at the larger
            // size a sprint's session names cost more than the column has.
            info_font: font::ROW,
            labels: LabelWeight::Muted,
        },
        SizeBucket::Medium => Layout {
            stripe: false,
            gp_block: false,
            schedule: Schedule::DatedSessions,
            track_map: false,
            info_font: font::ROW,
            labels: LabelWeight::Muted,
        },
        SizeBucket::Small => Layout {
            stripe: false,
            gp_block: false,
            schedule: Schedule::Absent,
            track_map: false,
            info_font: font::ROW,
            labels: LabelWeight::Strong,
        },
    }
}

/// One of the three columns the widest frame divides into.
fn third(bucket: SizeBucket) -> f32 {
    let (width, _) = bucket.design_size();
    (width - space::padding(bucket) * 2.0 - COLUMN_GAP * 2.0) / 3.0
}

fn count(value: Option<u16>) -> String {
    value.map_or_else(|| UNKNOWN.to_owned(), |value| fmt!("{}", value))
}

/// A distance in the operator's own units — kilometres, or miles.
fn distance(value: Option<Length>, decimals: u32) -> String {
    value.map_or_else(|| UNKNOWN.to_owned(), |value| value.format(decimals))
}

fn session_time(at: Option<LocalDateTime>) -> String {
    at.map_or_else(
        || UNKNOWN.to_owned(),
        |at| fmt!("{} {}", at.weekday_short(), parts::clock(at)),
    )
}

fn info_row(label: &str, value: String, layout: Layout) -> Node {
    parts::stat_row(label, value, layout.info_font, layout.labels)
}

fn circuit_rows(race: &NextRace, layout: Layout) -> Vec<Node> {
    vec![
        info_row("Number of Laps", count(race.total_laps), layout),
        info_row("Circuit Length", distance(race.track_length, 3), layout),
        info_row("Race Distance", distance(race.race_distance, 1), layout),
        info_row("DRS Zones", count(race.drs_zones.map(u16::from)), layout),
        // Printed as it arrives. The order is the upstream's own;
        // softest-first belongs to the design rather than to the data.
        info_row(
            "S / M / H Tires",
            race.tire_compounds
                .clone()
                .unwrap_or_else(|| UNKNOWN.to_owned()),
            layout,
        ),
    ]
}

/// What a frame's schedule column seats as a session's name.
///
/// The column holds a label and a time, and the time is the fixed half —
/// so what is left over is the label's budget, and nothing in the tree
/// shrinks text to fit it. A sprint's `Sprint Qualifying` is the longest
/// name a weekend carries and the one these are cut for.
fn session_chars(bucket: SizeBucket) -> usize {
    match bucket {
        SizeBucket::Full => 20,
        SizeBucket::Large | SizeBucket::Medium => 18,
        SizeBucket::Small => 11,
    }
}

fn schedule_rows(race: &NextRace, layout: Layout, bucket: SizeBucket) -> Vec<Node> {
    let mut rows = Vec::new();
    if let (Schedule::DatedSessions, Some(start)) = (layout.schedule, race.date_start) {
        rows.push(info_row(
            "Date",
            parts::date_range(start, race.date_end),
            layout,
        ));
    }
    for session in &race.sessions {
        rows.push(info_row(
            &parts::truncate(&session.name, session_chars(bucket)),
            session_time(session.starts_at),
            layout,
        ));
    }
    rows
}

fn gp_block(race: &NextRace) -> Node {
    col(
        props!(gap: 4.0),
        [
            row(
                props!(gap: 10.0, cross_align: CrossAlign::Center),
                [
                    text(
                        race.gp_name.as_str(),
                        style!(size: GP_NAME, weight: FontWeight::SEMIBOLD, color: color::TEXT, line_height: 1.0),
                    ),
                    parts::flag(22.4, &race.country_flag_url),
                ],
            ),
            text(
                race.country_name.as_str(),
                style!(size: font::TITLE, color: color::TEXT_MUTED),
            ),
        ],
    )
}

/// The circuit outline, in a box that names the circuit until it lands.
///
/// The fixed width sits on this column rather than on the `center` it
/// wraps: a `center` given no flex is grown to fill instead, which would
/// take half the frame and squeeze the stats beside it.
fn track_map(race: &NextRace, bucket: SizeBucket) -> Node {
    let width = third(bucket) - space::padding(bucket) * 2.0;
    // The box the outline is given, shaped like the one its kind decodes
    // into. Taken from that constant rather than from the artwork, so a
    // circuit drawn to other proportions pads inside the panel instead of
    // resizing it.
    let (decode_width, decode_height) = ImageKind::Circuit.decode_size();
    #[expect(
        clippy::cast_precision_loss,
        reason = "a decode box is a small constant, exact in f32"
    )]
    let height = width * decode_height as f32 / decode_width as f32;
    let inner = parts::remote_image(
        ImageKind::Circuit,
        &race.circuit_image_url,
        width,
        height,
        text(
            race.circuit_name.as_str(),
            style!(size: font::ROW, color: color::TEXT_MUTED, align: TextAlign::Center),
        ),
    );
    col(
        props!(
            width: third(bucket),
            padding: space::padding(bucket),
            border_width: 1.0,
            border_color: color::DIVIDER,
            border_radius: TRACK_MAP_RADIUS
        ),
        [center(props!(flex: 1.0), [inner])],
    )
}

fn header(race: &NextRace, bucket: SizeBucket, layout: Layout) -> Node {
    let dates = race
        .date_start
        .map(|start| parts::date_range(start, race.date_end));
    let content = match bucket {
        SizeBucket::Full | SizeBucket::Large => vec![
            parts::title("Next Race"),
            parts::subtitle(&dates.unwrap_or_default(), bucket),
        ],
        SizeBucket::Medium => vec![
            parts::title("Next Race"),
            parts::subtitle(&race.gp_name, bucket),
            parts::flag(16.8, &race.country_flag_url),
        ],
        // The smallest frame has room for one name, and prefers the
        // country's: it fits where a Grand Prix's full title would not.
        SizeBucket::Small => {
            let name = if race.country_name.is_empty() {
                &race.gp_name
            } else {
                &race.country_name
            };
            vec![
                parts::subtitle(name, bucket),
                parts::flag(12.6, &race.country_flag_url),
            ]
        }
    };
    parts::header(content, layout.stripe, bucket)
}

/// The columns of stats, and the schedule where the frame keeps one.
fn columns(race: &NextRace, layout: Layout, bucket: SizeBucket) -> Node {
    let circuit = parts::stat_col(circuit_rows(race, layout));
    if layout.schedule == Schedule::Absent {
        return circuit;
    }
    row(
        props!(flex: 1.0, gap: COLUMN_GAP),
        [
            circuit,
            parts::stat_col(schedule_rows(race, layout, bucket)),
        ],
    )
}

/// The next race, or the empty state while nothing has arrived.
#[must_use]
pub fn next_race_view(view: &NextRaceViewData) -> Node {
    let layout = layout(view.bucket);
    let Some(race) = view.race.as_ref() else {
        return parts::frame(
            vec![
                parts::header(vec![parts::title("Next Race")], layout.stripe, view.bucket),
                center(
                    props!(flex: 1.0),
                    [text(
                        "Next race unavailable",
                        style!(size: font::ROW, color: color::TEXT_MUTED),
                    )],
                ),
            ],
            view.bucket,
        );
    };

    let mut body = if layout.gp_block {
        col(
            props!(flex: 1.0),
            [
                col(props!(height: GP_BLOCK_LEAD), []),
                gp_block(race),
                col(props!(height: GP_BLOCK_GAP), []),
                columns(race, layout, view.bucket),
            ],
        )
    } else {
        columns(race, layout, view.bucket)
    };
    if layout.track_map {
        body = row(
            props!(flex: 1.0, gap: COLUMN_GAP),
            [body, track_map(race, view.bucket)],
        );
    }
    parts::frame(vec![header(race, view.bucket, layout), body], view.bucket)
}

#[cfg(test)]
mod tests {
    use super::{Schedule, layout, session_chars};
    use crate::model::SizeBucket;
    use crate::screens::parts::truncate;

    /// The longest name a weekend carries, which a sprint introduced and
    /// which overran the large frame — carrying the times off the edge,
    /// since nothing in the tree shrinks text to fit.
    const LONGEST: &str = "Sprint Qualifying";

    /// Every frame wide enough to read a session name reads the longest
    /// one whole. Small is the exception: it seats a name in the width a
    /// phone-sized tile has, so it is the one frame that cuts.
    #[test]
    fn only_the_small_frame_cuts_the_longest_session_name() {
        for bucket in [SizeBucket::Full, SizeBucket::Large, SizeBucket::Medium] {
            assert_eq!(
                truncate(LONGEST, session_chars(bucket)),
                LONGEST,
                "{bucket:?} seats {} and should read `{LONGEST}` whole",
                session_chars(bucket),
            );
        }
        assert_eq!(
            truncate(LONGEST, session_chars(SizeBucket::Small)),
            "Sprint Qua…"
        );
    }

    /// The per-frame rules the port keeps.
    #[test]
    fn every_frame_matches_the_ported_layout() {
        let expected = [
            (SizeBucket::Full, true, true, Schedule::Sessions, true),
            (
                SizeBucket::Large,
                true,
                true,
                Schedule::DatedSessions,
                false,
            ),
            (
                SizeBucket::Medium,
                false,
                false,
                Schedule::DatedSessions,
                false,
            ),
            (SizeBucket::Small, false, false, Schedule::Absent, false),
        ];
        for (bucket, stripe, gp_block, schedule, track_map) in expected {
            let layout = layout(bucket);
            assert_eq!(layout.stripe, stripe, "{bucket:?} header stripe");
            assert_eq!(layout.gp_block, gp_block, "{bucket:?} Grand Prix block");
            assert_eq!(layout.schedule, schedule, "{bucket:?} schedule column");
            assert_eq!(layout.track_map, track_map, "{bucket:?} track map");
        }
    }
}
