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

//! Drivers' championship standings.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::model::{SizeBucket, StandingsRow};
use crate::screens::parts::{self, color, font};

/// Everything the screen draws.
#[derive(Clone, Debug)]
pub struct StandingsViewData {
    pub bucket: SizeBucket,
    pub rows: Vec<StandingsRow>,
}

/// Team logo box, the legacy widget's 32 px at every frame.
const LOGO: f32 = 32.0;
/// Position column, its 28 px plus the 8 px it holds off the name.
const POSITION: f32 = 36.0;

/// The table's geometry for one frame.
///
/// The legacy widget used flex ratios — name 4, country 2, team 4 —
/// with cells shrinking and ellipsizing. This tree has no
/// `min-width: 0`, so text never shrinks below its content; these are
/// those ratios resolved per frame, with names cut to what they seat.
#[derive(Clone, Copy, Debug)]
struct Columns {
    rows: usize,
    stripe: bool,
    name: f32,
    name_chars: usize,
    /// `None` drops the column, as the narrow frames do.
    country: Option<f32>,
    team: Option<f32>,
    team_chars: usize,
    points: f32,
    /// The legacy widget quietens the score as the frame narrows: bold,
    /// then semibold, then the same weight as everything else.
    points_weight: FontWeight,
}

fn columns(bucket: SizeBucket) -> Columns {
    match bucket {
        SizeBucket::Full => Columns {
            rows: 10,
            stripe: true,
            name: 449.0,
            name_chars: 41,
            country: Some(224.0),
            team: Some(449.0),
            team_chars: 34,
            points: 56.0,
            points_weight: FontWeight::BOLD,
        },
        SizeBucket::Large => Columns {
            rows: 10,
            stripe: false,
            name: 245.0,
            name_chars: 22,
            country: None,
            team: Some(245.0),
            team_chars: 16,
            points: 56.0,
            points_weight: FontWeight::BOLD,
        },
        SizeBucket::Medium => Columns {
            rows: 5,
            stripe: true,
            name: 245.0,
            name_chars: 22,
            country: None,
            team: Some(245.0),
            team_chars: 16,
            points: 56.0,
            points_weight: FontWeight::SEMIBOLD,
        },
        SizeBucket::Small => Columns {
            rows: 5,
            stripe: false,
            name: 189.0,
            name_chars: 17,
            country: None,
            team: None,
            team_chars: 0,
            points: 44.0,
            points_weight: FontWeight::REGULAR,
        },
    }
}

/// Cut a name to what its column seats: one that overflows pushes
/// every column after it out of line.
fn truncate(label: &str, max_chars: usize) -> String {
    if label.chars().count() <= max_chars {
        return label.to_owned();
    }
    let mut out: String = label.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// The one run the row sets in semibold.
fn name(content: impl Into<String>) -> Node {
    text(
        content,
        style!(size: font::ROW, weight: FontWeight::SEMIBOLD, color: color::TEXT, line_height: 1.0),
    )
}

/// Everything else, which the legacy widget sets in the same white.
fn plain(content: impl Into<String>) -> Node {
    text(
        content,
        style!(size: font::ROW, color: color::TEXT, line_height: 1.0),
    )
}

/// A fixed-width column, so the cell below it starts at the same x.
fn cell(width: f32, child: Node) -> Node {
    col(
        props!(width: width, cross_align: CrossAlign::Start),
        [child],
    )
}

fn standings_row(entry: &StandingsRow, cols: Columns) -> Node {
    let mut cells = vec![
        cell(POSITION, plain(fmt!("{}.", entry.position))),
        cell(
            cols.name,
            name(truncate(&entry.driver_name, cols.name_chars)),
        ),
    ];
    if let Some(width) = cols.country {
        cells.push(cell(
            width,
            row(
                props!(gap: parts::space::GAP * 3.0, cross_align: CrossAlign::Center),
                [
                    parts::image_placeholder(LOGO, None),
                    plain(entry.country_code.clone()),
                ],
            ),
        ));
    }
    if let Some(width) = cols.team {
        cells.push(cell(
            width,
            row(
                props!(gap: parts::space::GAP * 3.0, cross_align: CrossAlign::Center),
                [
                    parts::team_mark(LOGO, &entry.team_name, entry.team_color),
                    plain(truncate(&entry.team_name, cols.team_chars)),
                ],
            ),
        ));
    }

    // The points take the slack, pinning them to the right edge.
    cells.push(col(
        props!(flex: 1.0, width: cols.points),
        [text(
            fmt!("{}", entry.points),
            style!(size: font::ROW, weight: cols.points_weight, color: color::TEXT, align: TextAlign::Right, line_height: 1.0),
        )],
    ));

    // Rows share the frame's spare height rather than claiming a fixed
    // one, so a table cannot outgrow its frame.
    row(
        props!(flex: 1.0, gap: parts::space::GAP, cross_align: CrossAlign::Center),
        cells,
    )
}

/// The standings table, or the empty state while nothing has arrived.
#[must_use]
pub fn standings_view(view: &StandingsViewData) -> Node {
    let cols = columns(view.bucket);
    let mut children = vec![parts::header("Drivers Standing", cols.stripe)];
    if view.rows.is_empty() {
        children.push(center(
            props!(flex: 1.0),
            [text(
                "Standings unavailable",
                style!(size: font::ROW, color: color::TEXT_MUTED),
            )],
        ));
        return parts::frame(children);
    }

    let mut table = Vec::new();
    for (index, entry) in view.rows.iter().take(cols.rows).enumerate() {
        if index > 0 {
            table.push(parts::divider());
        }
        table.push(standings_row(entry, cols));
    }
    // No gap: the dividers already separate the rows, and a gap
    // between all of them costs more height than the frame has.
    children.push(col(props!(flex: 1.0), table));
    parts::frame(children)
}

#[cfg(test)]
mod tests {
    use super::{FontWeight, POSITION, columns, truncate};
    use crate::model::SizeBucket;
    use crate::screens::fixtures;
    use crate::screens::parts::space;

    /// Every bucket, so a new frame cannot skip these checks.
    const ALL: [SizeBucket; 4] = [
        SizeBucket::Full,
        SizeBucket::Large,
        SizeBucket::Medium,
        SizeBucket::Small,
    ];

    #[test]
    fn the_columns_fit_inside_every_frame() {
        // Guards the regression that shipped a table wider than its frame.
        for bucket in ALL {
            let cols = columns(bucket);
            let (width, _) = bucket.design_size();
            let content = width - space::PADDING * 2.0;
            let taken = POSITION
                + cols.name
                + cols.country.unwrap_or(0.0)
                + cols.team.unwrap_or(0.0)
                + cols.points;
            let columns_shown =
                2.0 + f32::from(u8::from(cols.country.is_some()) + u8::from(cols.team.is_some()));
            assert!(
                taken + space::GAP * columns_shown <= content,
                "{bucket:?}: columns take {taken} of {content}",
            );
        }
    }

    /// The legacy widget's per-frame rules, which this screen replicates.
    #[test]
    fn every_frame_matches_the_legacy_widgets_layout() {
        let expected = [
            (SizeBucket::Full, 10, true, true, true, FontWeight::BOLD),
            (SizeBucket::Large, 10, false, true, false, FontWeight::BOLD),
            (
                SizeBucket::Medium,
                5,
                false,
                true,
                true,
                FontWeight::SEMIBOLD,
            ),
            (
                SizeBucket::Small,
                5,
                false,
                false,
                false,
                FontWeight::REGULAR,
            ),
        ];
        for (bucket, rows, country, team, stripe, points_weight) in expected {
            let cols = columns(bucket);
            assert_eq!(cols.rows, rows, "{bucket:?} rows");
            assert_eq!(cols.country.is_some(), country, "{bucket:?} country column");
            assert_eq!(cols.team.is_some(), team, "{bucket:?} team column");
            assert_eq!(cols.stripe, stripe, "{bucket:?} header stripe");
            assert_eq!(
                cols.points_weight, points_weight,
                "{bucket:?} points weight"
            );
        }
    }

    #[test]
    fn a_name_longer_than_its_column_is_cut_to_it() {
        assert_eq!(truncate("Lando Norris", 34), "Lando Norris");
        assert_eq!(truncate("Andrea Kimi Antonelli", 10), "Andrea Ki\u{2026}");
    }

    /// Cutting is the guard against an upstream that renames a driver,
    /// not something the grid as it stands should ever provoke: the
    /// narrow frames drop whole columns to make the room instead.
    #[test]
    fn every_frame_seats_the_grid_whole() {
        for row in fixtures::standings_widest(SizeBucket::Full).rows {
            for bucket in ALL {
                let cols = columns(bucket);
                assert_eq!(
                    truncate(&row.driver_name, cols.name_chars),
                    row.driver_name,
                    "{bucket:?} cuts `{}`",
                    row.driver_name,
                );
                if cols.team.is_some() {
                    assert_eq!(
                        truncate(&row.team_name, cols.team_chars),
                        row.team_name,
                        "{bucket:?} cuts `{}`",
                        row.team_name,
                    );
                }
            }
        }
    }
}
