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

//! The timing boards of a running session: race, qualifying, practice.
//!
//! One module rather than three: the boards differ only in which cells
//! a row carries and what the header says, so they share the table,
//! the cell vocabulary and the idle state, and each contributes only
//! its own per-frame column list.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::images::ImageKind;
use crate::model::{LiveBoard, Sector, SectorColor, SizeBucket, TimingRow, TireCompound};
use crate::screens::parts::{self, color, font, space};

/// Everything a board draws.
#[derive(Clone, Debug)]
pub struct LiveViewData {
    pub bucket: SizeBucket,
    pub board: LiveBoard,
}

/// Which session's board — its title, and which cells its rows carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    Race,
    Quali,
    Practice,
}

impl Board {
    fn title(self) -> &'static str {
        match self {
            Self::Race => "Race Live",
            Self::Quali => "Quali Live",
            Self::Practice => "Practice Live",
        }
    }

    /// What the board says when no session is running.
    fn idle(self) -> &'static str {
        match self {
            Self::Race => "No race running",
            Self::Quali => "No qualifying running",
            Self::Practice => "No practice running",
        }
    }
}

/// The live palette, read off the port's own CSS variables.
mod live_color {
    use bmc_wasm_sdk::Color;

    pub const SECTOR_WHITE: Color = Color::from_hex(0xF4_F4_F4);
    pub const SECTOR_GREEN: Color = Color::from_hex(0x78_F4_42);
    /// The session's best, and the colour a fastest-lap row is set in.
    pub const FASTEST: Color = Color::from_hex(0xAC_36_CE);
    pub const GAINED: Color = Color::from_hex(0x77_E9_46);
    pub const LOST: Color = Color::from_hex(0xC4_3C_1E);
    /// A driver who has neither gained nor lost a place.
    pub const HELD: Color = Color::from_hex(0x52_52_52);
    /// The unworn part of a tire's wear bar.
    pub const TRACK: Color = Color::from_hex(0x39_39_39);
    /// The age badge, which reads dark against its own fill.
    pub const AGE: Color = Color::from_hex(0x42_BE_65);
    pub const AGE_TEXT: Color = Color::from_hex(0x00_00_00);
}

/// A compound's own colour, or grey for one this build has not seen.
fn compound_color(compound: Option<TireCompound>) -> Color {
    match compound {
        Some(TireCompound::Soft) => Color::from_hex(0xDA_1E_28),
        Some(TireCompound::Medium) => Color::from_hex(0xF1_C2_1B),
        Some(TireCompound::Hard) => Color::from_hex(0xFF_FF_FF),
        Some(TireCompound::Intermediate) => Color::from_hex(0x42_BE_65),
        Some(TireCompound::Wet) => Color::from_hex(0x45_89_FF),
        None => Color::from_hex(0xA8_A8_A8),
    }
}

/// The team logo. The port draws 32–40 px but pulls 4 px back at each
/// end with `margin-block: -4px`, so the image never drives the row's
/// height; this tree has no negative margin, so the box is the 24 px
/// that leaves, which the rows have room for at every frame.
const LOGO: f32 = 24.0;
/// What the rows hold off one another, standing in for the port's
/// `padding: 2px 4px` on every cell.
const ROW_GAP: f32 = 4.0;
/// The identity columns' widths. A table aligns its columns because it
/// is a table; this tree is rows of flex boxes, so each column that
/// precedes another has to hold a width of its own or the row below
/// with a wider entry pushes everything after it out of line.
const POSITION_WIDTH: f32 = 40.0;
const CODE_WIDTH: f32 = 64.0;
const CHANGE_WIDTH: f32 = 56.0;
/// The compound and age badges, both circles of this diameter.
const BADGE: f32 = 22.0;
const BADGE_TEXT: u32 = 11;
/// The compound ring's stroke.
const RING_WIDTH: f32 = 2.0;
/// The place-change chevron, at the artwork's own 4:3.
const ARROW_WIDTH: f32 = 16.0;
const ARROW_HEIGHT: f32 = 12.0;
/// The wear bar between them. Taller than the port's 3 px hairline,
/// which reads as a scratch beside badges of this size.
const WEAR_WIDTH: f32 = 60.0;
const WEAR_HEIGHT: f32 = 6.0;
/// The stint length a full wear bar stands for.
const WEAR_FULL_LAPS: f32 = 30.0;
/// The height of the flag beside the Grand Prix in the header; its width
/// follows the artwork the deployment sends.
const FLAG: f32 = 16.8;

/// A cell of a timing row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Position,
    TeamLogo,
    Code,
    /// Places gained or lost since the previous lap.
    Change,
    /// The gap to the leader, which a car in the pits replaces.
    Gap,
    Interval,
    LastLap,
    BestLap,
    TotalTime,
    /// Compound, wear bar and age.
    Tire,
    /// The compound alone, as practice draws it.
    Compound,
    /// Practice's gap column where the frame also has a lap column to
    /// report an out lap in: the leader, or a gap.
    PracticeGap,
    /// Practice's gap column where the frame has no lap column, so it
    /// reports the out lap itself.
    PracticeGapOrOut,
    /// Practice's lap column, which an out lap takes over.
    PracticeLap,
    Sector(usize),
}

/// How much of the field a frame seats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seats {
    /// The leading rows, in one table.
    Top(usize),
    /// The whole field, halved into two tables side by side.
    Split,
}

#[derive(Clone, Debug)]
struct Columns {
    seats: Seats,
    cells: Vec<Cell>,
    /// The narrowest frames name the session in the header alone.
    titled: bool,
}

fn columns(board: Board, bucket: SizeBucket) -> Columns {
    use Cell::{
        BestLap, Change, Code, Compound, Gap, Interval, LastLap, Position, PracticeGap,
        PracticeGapOrOut, PracticeLap, TeamLogo, Tire, TotalTime,
    };

    let (seats, mut cells) = match (board, bucket) {
        (Board::Race, SizeBucket::Full) => (
            Seats::Top(10),
            vec![
                Position, TeamLogo, Code, Change, Gap, Interval, LastLap, Tire,
            ],
        ),
        (Board::Race, SizeBucket::Large) => (
            Seats::Top(10),
            vec![Position, TeamLogo, Code, Change, Gap, Tire],
        ),
        (Board::Race, SizeBucket::Medium) => (
            Seats::Top(5),
            vec![Position, TeamLogo, Code, Change, Gap, Tire],
        ),
        (Board::Race, SizeBucket::Small) => (Seats::Top(5), vec![Position, Code, Change, Gap]),
        (Board::Quali, SizeBucket::Full) => {
            (Seats::Split, vec![Position, TeamLogo, Code, TotalTime])
        }
        (Board::Quali, SizeBucket::Large) => {
            (Seats::Top(10), vec![Position, TeamLogo, Code, TotalTime])
        }
        (Board::Quali, SizeBucket::Medium | SizeBucket::Small) => {
            (Seats::Top(5), vec![Position, TeamLogo, Code, TotalTime])
        }
        (Board::Practice, SizeBucket::Full) => (
            Seats::Top(10),
            vec![
                Position,
                TeamLogo,
                Code,
                BestLap,
                Compound,
                PracticeGap,
                PracticeLap,
            ],
        ),
        (Board::Practice, SizeBucket::Large) => (
            Seats::Top(10),
            vec![Position, TeamLogo, Code, PracticeGapOrOut],
        ),
        (Board::Practice, SizeBucket::Medium) => (
            Seats::Top(5),
            vec![Position, TeamLogo, Code, PracticeGapOrOut],
        ),
        (Board::Practice, SizeBucket::Small) => {
            (Seats::Top(5), vec![Position, TeamLogo, Code, BestLap])
        }
    };
    // Sectors ride along wherever the frame has the width for them:
    // the race keeps them to its widest frame, the slimmer boards
    // carry them down to the medium one, and the smallest never does.
    let sectored = match board {
        Board::Race => bucket == SizeBucket::Full,
        Board::Quali | Board::Practice => bucket != SizeBucket::Small,
    };
    if sectored {
        cells.extend([Cell::Sector(0), Cell::Sector(1), Cell::Sector(2)]);
    }
    Columns {
        seats,
        cells,
        titled: bucket != SizeBucket::Small,
    }
}

fn row_font(bucket: SizeBucket) -> u32 {
    match bucket {
        SizeBucket::Full => font::TITLE,
        SizeBucket::Large | SizeBucket::Medium | SizeBucket::Small => font::ROW,
    }
}

/// A sector's time, to the thousandth.
///
/// A period, not the operator's decimal separator: the neighbouring
/// lap and gap columns arrive pre-formatted from the server and carry
/// one already, and a row mixing both separators reads as a mistake.
fn sector_time(seconds: f32) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a sector time is a small positive number of seconds"
    )]
    let total_ms = (f64::from(seconds) * 1000.0).round().max(0.0) as u64;
    let (whole, millis) = (total_ms.div_euclid(1000), total_ms.rem_euclid(1000));
    // `fmt!` takes no width specifier, so the leading zeros are our own.
    let lead = match millis {
        0..=9 => "00",
        10..=99 => "0",
        _ => "",
    };
    fmt!("{}.{}{}", whole, lead, millis)
}

fn plain(content: impl Into<String>, size: u32, tone: Color) -> Node {
    text(content, style!(size: size, color: tone, line_height: 1.0))
}

fn value(content: impl Into<String>, size: u32, tone: Color) -> Node {
    text(
        content,
        style!(size: size, color: tone, align: TextAlign::Right, line_height: 1.0),
    )
}

/// The chevron marking a place gained or lost, drawn rather than set:
/// the deck's font carries no triangle, so a glyph renders as tofu.
/// A driver holding station gets a bar instead, as the port draws one.
fn chevron(change: i8, tone: Color) -> Node {
    let (w, h) = (ARROW_WIDTH, ARROW_HEIGHT);
    let shape = match change {
        1.. => Draw::fill_path(vec![(0.0, h), (w, h), (w / 2.0, 0.0)], tone, false),
        ..=-1 => Draw::fill_path(vec![(0.0, 0.0), (w, 0.0), (w / 2.0, h)], tone, false),
        0 => Draw::rect(0.0, h / 2.0 - 1.0, w, 2.0, tone),
    };
    canvas(props!(width: w, height: h), [shape.with_anti_alias()])
}

/// The places a driver has gained or lost: a chevron in its own colour,
/// and the count in the row's. A driver holding station shows a bar
/// and a zero rather than an empty cell, as the port does.
fn change_cell(change: i8, size: u32) -> Node {
    let tone = match change {
        1.. => live_color::GAINED,
        ..=-1 => live_color::LOST,
        0 => live_color::HELD,
    };
    row(
        props!(gap: space::GAP, cross_align: CrossAlign::Center),
        [
            chevron(change, tone),
            plain(fmt!("{}", change.unsigned_abs()), size, color::TEXT),
        ],
    )
}

/// A badge circle with its label centred: the compound ringed in its
/// own colour, or the tire's age filled with one.
///
/// Circle and label share one canvas, which is what puts the letter in
/// the middle. Boxing them instead leaves the rim hard-edged and the
/// glyph off centre, since a text box carries the letter's own side
/// bearings and centring the box centres those along with it.
fn badge(label: String, ring: Option<Color>, fill: Option<Color>, tone: Color) -> Node {
    let middle = BADGE / 2.0;
    let mut shapes = Vec::new();
    // One radius for both kinds, a pixel off the canvas edge so the
    // rim has somewhere to fade. Sized apart, a row carrying a filled
    // badge and a ringed one reads lopsided.
    let outer = middle - 1.0;
    if let Some(fill) = fill {
        shapes.push(Draw::circle(middle, middle, outer, fill).with_anti_alias());
    }
    if let Some(ring) = ring {
        // A ring of two filled circles rather than a stroked arc: the
        // arc's stroke comes out stepped at this diameter, where a
        // filled circle's rim takes the anti-aliasing cleanly.
        shapes.push(Draw::circle(middle, middle, outer, ring).with_anti_alias());
        shapes.push(Draw::circle(middle, middle, outer - RING_WIDTH, color::BG).with_anti_alias());
    }
    // `align` and `valign` centre the glyph on the point given, so no
    // nudging here. It still reads a shade left: `draw_canvas_text`
    // centres the *advance* width, and ink sits at the glyph's
    // `offset_x` within it. Centring on ink is a `bmc-render` change
    // every widget's canvas text would feel, so it stands.
    shapes.push(Draw::text(
        middle,
        middle,
        label,
        style!(size: BADGE_TEXT, weight: FontWeight::BOLD, color: tone, align: TextAlign::Center, valign: VerticalAlign::Center),
    ));
    canvas(props!(width: BADGE, height: BADGE), shapes)
}

fn compound_badge(compound: Option<TireCompound>) -> Node {
    let tone = compound_color(compound);
    let label = compound.map_or_else(|| "?".to_owned(), |it| it.label().to_owned());
    badge(label, Some(tone), None, tone)
}

/// Compound, a wear bar filling with the stint, and the lap count.
fn tire_cell(row_data: &TimingRow) -> Node {
    let tone = compound_color(row_data.tire_compound);
    let worn = (f32::from(row_data.tire_age) / WEAR_FULL_LAPS).clamp(0.0, 1.0);
    let bar = canvas(
        props!(width: WEAR_WIDTH, height: WEAR_HEIGHT),
        [
            Draw::rect(0.0, 0.0, WEAR_WIDTH, WEAR_HEIGHT, live_color::TRACK),
            Draw::rect(0.0, 0.0, WEAR_WIDTH * worn, WEAR_HEIGHT, tone),
        ],
    );
    row(
        props!(gap: space::GAP, cross_align: CrossAlign::Center),
        [
            compound_badge(row_data.tire_compound),
            bar,
            badge(
                fmt!("{}", row_data.tire_age),
                None,
                Some(live_color::AGE),
                live_color::AGE_TEXT,
            ),
        ],
    )
}

fn sector_cell(sector: Option<Sector>, size: u32) -> Node {
    let Some(sector) = sector else {
        return value("\u{2014}", size, color::TEXT_MUTED);
    };
    let tone = match sector.color {
        SectorColor::Normal => live_color::SECTOR_WHITE,
        SectorColor::PersonalBest => live_color::SECTOR_GREEN,
        SectorColor::OverallBest => live_color::FASTEST,
    };
    value(sector_time(sector.seconds), size, tone)
}

/// Timing text the server sent, or a dash where it sent nothing.
fn timing(text_value: &crate::model::TimingText, size: u32, tone: Color) -> Node {
    if text_value.is_blank() {
        return value("-", size, tone);
    }
    value(text_value.as_str().to_owned(), size, tone)
}

fn cell(which: Cell, row_data: &TimingRow, bucket: SizeBucket) -> Node {
    let size = row_font(bucket);
    // A fastest lap colours the driver's own identity, nothing else.
    let identity = if row_data.fastest_lap {
        live_color::FASTEST
    } else {
        color::TEXT
    };
    match which {
        Cell::Position => plain(fmt!("{}", row_data.position), size, identity),
        Cell::TeamLogo => parts::remote_image(
            ImageKind::TeamLogo,
            &row_data.team_logo_url,
            LOGO,
            LOGO,
            parts::image_placeholder(LOGO, Some(row_data.team_color)),
        ),
        Cell::Code => text(
            row_data.driver_code.as_str(),
            style!(size: size, weight: FontWeight::SEMIBOLD, color: identity, line_height: 1.0),
        ),
        Cell::Change => change_cell(row_data.position_change, size),
        Cell::Gap => {
            if row_data.in_pit {
                value("PIT", size, live_color::LOST)
            } else {
                timing(&row_data.gap_to_leader, size, color::TEXT)
            }
        }
        Cell::Interval => timing(&row_data.interval, size, color::TEXT),
        Cell::LastLap => timing(&row_data.last_lap_time, size, color::TEXT),
        Cell::BestLap => timing(&row_data.best_lap_time, size, color::TEXT),
        Cell::TotalTime => timing(&row_data.total_time, size, color::TEXT),
        Cell::Tire => tire_cell(row_data),
        Cell::Compound => compound_badge(row_data.tire_compound),
        Cell::PracticeGap => {
            if row_data.position == 1 {
                value("LEADER", size, color::TEXT)
            } else {
                timing(&row_data.gap_to_leader, size, color::TEXT)
            }
        }
        Cell::PracticeGapOrOut => {
            if row_data.is_out_lap {
                // One word: this cell exists on the frames that fold the lap
                // into the gap, whose slot is cut for `+0.161` and wraps.
                value("OUT", size, color::TEXT_MUTED)
            } else {
                cell(Cell::PracticeGap, row_data, bucket)
            }
        }
        Cell::PracticeLap => {
            if row_data.is_out_lap {
                value("OUT LAP", size, color::TEXT_MUTED)
            } else {
                timing(&row_data.last_lap_time, size, color::TEXT)
            }
        }
        // Sectors keep the narrower frames' font wherever the row is
        // set larger, staying subordinate to the identity beside them.
        Cell::Sector(index) => {
            sector_cell(row_data.sectors.get(index).copied().flatten(), font::ROW)
        }
    }
}

/// A column of fixed width, so what follows it starts at one x.
fn fixed(width: f32, child: Node) -> Node {
    col(
        props!(width: width, cross_align: CrossAlign::Start, justify_content: Justify::Center),
        [child],
    )
}

/// A column taking an even share of the row's slack, its value pushed
/// to the right edge and held at the row's middle.
fn value_column(child: Node) -> Node {
    col(
        props!(flex: 1.0, cross_align: CrossAlign::End, justify_content: Justify::Center),
        [child],
    )
}

/// The rule a value column draws on its left. The port rules its table
/// vertically, between columns, and never between rows.
///
/// Carries no flex: a row grows its children along the main axis, so a
/// flexed rule eats width instead of standing as a line. Its height
/// comes from the row stretching what it holds.
fn column_rule() -> Node {
    col(props!(width: 1.0, background: color::DIVIDER), [])
}

/// One timing row: the identity cells pack left, the values take the
/// slack so their columns line up on the right.
fn timing_row(row_data: &TimingRow, cols: &Columns, bucket: SizeBucket) -> Node {
    let mut cells = Vec::new();
    for which in &cols.cells {
        let node = cell(*which, row_data, bucket);
        match which {
            // Who the row is: packed left, each holding its column's
            // width so the identity block lines up down the table.
            Cell::Position => cells.push(fixed(POSITION_WIDTH, node)),
            // Sized artwork, which centres itself against the row.
            Cell::TeamLogo | Cell::Compound => {
                cells.push(col(props!(justify_content: Justify::Center), [node]));
            }
            Cell::Code => cells.push(fixed(CODE_WIDTH, node)),
            // The port rules everything after the driver's code off
            // from what precedes it, the change column included.
            Cell::Change => {
                cells.push(column_rule());
                cells.push(fixed(CHANGE_WIDTH, node));
            }
            // What they did: each takes an even share of the slack, so
            // the columns line up down the table.
            Cell::Gap
            | Cell::Interval
            | Cell::LastLap
            | Cell::BestLap
            | Cell::TotalTime
            | Cell::Tire
            | Cell::PracticeGap
            | Cell::PracticeGapOrOut
            | Cell::PracticeLap
            | Cell::Sector(_) => {
                cells.push(column_rule());
                cells.push(value_column(node));
            }
        }
    }
    // Stretch, not centre: the rules take their height from the row,
    // and every cell holds its own content at the middle instead.
    row(
        props!(flex: 1.0, gap: space::GAP * 2.0, cross_align: CrossAlign::Stretch),
        cells,
    )
}

/// The rows a frame seats, sharing its height between them.
fn table(rows: &[TimingRow], cols: &Columns, bucket: SizeBucket) -> Node {
    let mut children = Vec::new();
    for row_data in rows {
        children.push(timing_row(row_data, cols, bucket));
    }
    col(props!(flex: 1.0, gap: ROW_GAP), children)
}

fn header(
    board: Board,
    data: &crate::model::TimingBoard,
    cols: &Columns,
    bucket: SizeBucket,
) -> Node {
    let info = match board {
        Board::Race => fmt!("LAP {}/{}", data.current_lap, data.total_laps),
        Board::Quali | Board::Practice => data.session_label.clone(),
    };
    let mut content = Vec::new();
    if cols.titled {
        content.push(parts::title(board.title()));
        content.push(parts::subtitle(&data.gp_name, bucket));
    }
    content.push(parts::flag(FLAG, &data.country_flag_url));
    content.push(spacer(1.0));
    content.push(parts::subtitle(&info, bucket));
    parts::header(content, false, bucket)
}

/// A board of the running session, or what it says while none runs.
fn board_view(board: Board, view: &LiveViewData) -> Node {
    let cols = columns(board, view.bucket);
    let Some(data) = view.board.board() else {
        return parts::frame(
            vec![
                parts::header(vec![parts::title(board.title())], false, view.bucket),
                center(
                    props!(flex: 1.0),
                    [text(
                        board.idle(),
                        style!(size: font::ROW, color: color::TEXT_MUTED),
                    )],
                ),
            ],
            view.bucket,
        );
    };
    let body = match cols.seats {
        // The widest qualifying frame runs the whole field, halved into
        // two tables so a full grid fits without shrinking its rows.
        Seats::Split => {
            let half = data.rows.len().div_ceil(2);
            let (left, right) = data.rows.split_at(half);
            row(
                props!(flex: 1.0, gap: space::GAP * 2.0),
                [
                    table(left, &cols, view.bucket),
                    col(props!(width: 2.0, background: color::TEXT_MUTED), []),
                    table(right, &cols, view.bucket),
                ],
            )
        }
        Seats::Top(count) => {
            let seated = data.rows.len().min(count);
            table(&data.rows[..seated], &cols, view.bucket)
        }
    };
    parts::frame(
        vec![header(board, data, &cols, view.bucket), body],
        view.bucket,
    )
}

#[must_use]
pub fn race_view(view: &LiveViewData) -> Node {
    board_view(Board::Race, view)
}

#[must_use]
pub fn quali_view(view: &LiveViewData) -> Node {
    board_view(Board::Quali, view)
}

#[must_use]
pub fn practice_view(view: &LiveViewData) -> Node {
    board_view(Board::Practice, view)
}

#[cfg(test)]
mod tests {
    use super::{Board, Cell, Seats, columns, sector_time};
    use crate::model::SizeBucket;

    const ALL: [SizeBucket; 4] = [
        SizeBucket::Full,
        SizeBucket::Large,
        SizeBucket::Medium,
        SizeBucket::Small,
    ];

    #[test]
    fn a_sector_time_keeps_its_thousandths() {
        assert_eq!(sector_time(23.456), "23.456");
        assert_eq!(sector_time(23.4), "23.400");
        assert_eq!(sector_time(23.04), "23.040");
        assert_eq!(sector_time(23.004), "23.004");
    }

    /// The per-frame rules the port keeps.
    #[test]
    fn every_frame_matches_the_ported_columns() {
        let expected = [
            (Board::Race, SizeBucket::Full, Seats::Top(10), 11),
            (Board::Race, SizeBucket::Large, Seats::Top(10), 6),
            (Board::Race, SizeBucket::Medium, Seats::Top(5), 6),
            (Board::Race, SizeBucket::Small, Seats::Top(5), 4),
            (Board::Quali, SizeBucket::Full, Seats::Split, 7),
            (Board::Quali, SizeBucket::Large, Seats::Top(10), 7),
            (Board::Quali, SizeBucket::Medium, Seats::Top(5), 7),
            (Board::Quali, SizeBucket::Small, Seats::Top(5), 4),
            (Board::Practice, SizeBucket::Full, Seats::Top(10), 10),
            (Board::Practice, SizeBucket::Large, Seats::Top(10), 7),
            (Board::Practice, SizeBucket::Medium, Seats::Top(5), 7),
            (Board::Practice, SizeBucket::Small, Seats::Top(5), 4),
        ];
        for (board, bucket, seats, cells) in expected {
            let cols = columns(board, bucket);
            assert_eq!(cols.seats, seats, "{board:?} {bucket:?} seating");
            assert_eq!(cols.cells.len(), cells, "{board:?} {bucket:?} cell count");
        }
    }

    #[test]
    fn only_the_widest_qualifying_frame_splits_its_field() {
        for board in [Board::Race, Board::Quali, Board::Practice] {
            for bucket in ALL {
                let splits = columns(board, bucket).seats == Seats::Split;
                let expected = board == Board::Quali && bucket == SizeBucket::Full;
                assert_eq!(splits, expected, "{board:?} {bucket:?} split");
            }
        }
    }

    #[test]
    fn the_smallest_frame_never_seats_a_sector() {
        for board in [Board::Race, Board::Quali, Board::Practice] {
            let cols = columns(board, SizeBucket::Small);
            assert!(
                !cols.cells.iter().any(|c| matches!(c, Cell::Sector(_))),
                "{board:?} must drop its sectors at the smallest frame"
            );
        }
    }
}
