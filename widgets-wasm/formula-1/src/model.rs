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

//! What the widget holds after a reply is read: plain data the screens
//! render, plus the rules that turn wire values into it.
//!
//! Everything here is free of host calls so it can be exercised
//! natively — by tests and by the gallery's fixtures.

use bmc_wasm_sdk::{CalendarDate, Color, Length, LocalDateTime, Mass};

/// The design's four frames. The widget renders the same screens in each,
/// dropping columns and rows as the box shrinks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Small,
    Medium,
    Large,
    Full,
}

impl SizeBucket {
    /// Design reference sizes: S 317×238, M 638×238, L 638×480, Full 1280×480.
    #[must_use]
    pub fn design_size(self) -> (f32, f32) {
        match self {
            Self::Small => (317.0, 238.0),
            Self::Medium => (638.0, 238.0),
            Self::Large => (638.0, 480.0),
            Self::Full => (1280.0, 480.0),
        }
    }
}

/// Which frame a viewport falls in.
///
/// A box narrower than the medium design takes the small frame rather than
/// the nearest band: the wider frames seat a second column, and one drawn
/// for 638 px does not fit a Mini Miner's 480.
#[must_use]
pub fn size_bucket(width: u32, height: u32) -> SizeBucket {
    if width >= 900 {
        SizeBucket::Full
    } else if width < 638 {
        SizeBucket::Small
    } else if height <= 330 {
        SizeBucket::Medium
    } else {
        SizeBucket::Large
    }
}

/// Livery colour shown for a team whose own colour is missing
/// or unreadable, so the row still renders in team-neutral grey.
pub const FALLBACK_TEAM_COLOR: Color = Color::from_hex(0x52_52_52);

/// Read a wire livery colour: six hex digits,
/// with or without a leading `#`.
/// Anything else is [`FALLBACK_TEAM_COLOR`],
/// so a bad value costs the row its colour but not its place.
#[must_use]
pub fn team_color(hex: &str) -> Color {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    if digits.len() != 6 {
        return FALLBACK_TEAM_COLOR;
    }
    u32::from_str_radix(digits, 16).map_or(FALLBACK_TEAM_COLOR, Color::from_hex)
}

/// A car's racing number, which the screens print and nothing else reads.
///
/// Distinct from a championship or grid position, which are small
/// integers too: the two resources join on `jolpica_id`, so a number
/// mistaken for a place would print as one without ever being caught.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarNumber(u8);

impl CarNumber {
    #[must_use]
    pub fn new(number: u8) -> Self {
        Self(number)
    }

    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

/// URL of a remote image — a headshot, team logo, flag, or circuit
/// map. Empty when the server has none; the screens draw their own
/// placeholder rather than a broken image.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageUrl(String);

impl ImageUrl {
    /// Whether there is an image to fetch at all.
    ///
    /// Nexus sends the flag emoji in `country_flag_url` where it holds
    /// no image, so emptiness alone does not decide this.
    #[must_use]
    pub fn is_present(&self) -> bool {
        self.0.starts_with("https://") || self.0.starts_with("http://")
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ImageUrl {
    fn from(url: String) -> Self {
        Self(url)
    }
}

/// Timing text the server has already formatted — a gap, an interval,
/// or a lap time, including the sentinels it uses in place of one
/// (`LEADER`, `PIT`, `-`). Printed as received; deriving anything from
/// it is the server's job, not ours.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimingText(String);

impl TimingText {
    /// Whether there is nothing to print —
    /// an empty value, or the server's `-` placeholder.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.0.is_empty() || self.0 == "-"
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for TimingText {
    fn from(text: String) -> Self {
        Self(text)
    }
}

/// Tyre fitted to a car. The wire spells these as single letters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TireCompound {
    Soft,
    Medium,
    Hard,
    Intermediate,
    Wet,
}

impl TireCompound {
    #[must_use]
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "S" => Some(Self::Soft),
            "M" => Some(Self::Medium),
            "H" => Some(Self::Hard),
            "I" => Some(Self::Intermediate),
            "W" => Some(Self::Wet),
            _ => None,
        }
    }

    /// Single-letter label, as the timing screens print it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Soft => "S",
            Self::Medium => "M",
            Self::Hard => "H",
            Self::Intermediate => "I",
            Self::Wet => "W",
        }
    }
}

/// How a sector time compares with the rest of the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectorColor {
    /// Slower than this driver's own best.
    Normal,
    /// This driver's best.
    PersonalBest,
    /// The session's best.
    OverallBest,
}

impl SectorColor {
    #[must_use]
    pub fn from_wire(value: &str) -> Self {
        match value {
            "green" => Self::PersonalBest,
            "purple" => Self::OverallBest,
            _ => Self::Normal,
        }
    }
}

/// Where a driver stands in a running session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverStatus {
    Running,
    Retired,
    Disqualified,
    DidNotFinish,
    DidNotStart,
    Finished,
}

impl DriverStatus {
    /// Unknown values read as absent rather than failing the row:
    /// a status this build has not seen
    /// must not cost us the whole timing board.
    #[must_use]
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "RUN" => Some(Self::Running),
            "RET" => Some(Self::Retired),
            "DSQ" => Some(Self::Disqualified),
            "DNF" => Some(Self::DidNotFinish),
            "DNS" => Some(Self::DidNotStart),
            "FIN" => Some(Self::Finished),
            _ => None,
        }
    }

    /// Whether the driver is still circulating.
    #[must_use]
    pub fn is_out(self) -> bool {
        matches!(
            self,
            Self::Retired | Self::Disqualified | Self::DidNotFinish | Self::DidNotStart
        )
    }
}

/// One sector time and how it ranked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sector {
    pub seconds: f32,
    pub color: SectorColor,
}

/// A championship standings row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StandingsRow {
    /// Absent where the payload named none.
    pub position: Option<u8>,
    pub driver_name: String,
    pub driver_code: String,
    pub team_name: String,
    pub team_logo_url: ImageUrl,
    pub team_color: Color,
    pub country_code: String,
    pub country_flag_url: ImageUrl,
    pub points: u16,
    pub headshot_url: ImageUrl,
}

/// A driver's season and career figures, as the statistics screens
/// show them. The optional fields are absent for a driver the career
/// source has no history for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DriverStats {
    /// The key every driver-facing resource carries,
    /// and what the `driver` param holds — so one joins to another.
    pub jolpica_id: String,
    pub name: String,
    /// Absent where the payload named none: there is no zeroth car.
    pub number: Option<CarNumber>,
    pub headshot_url: ImageUrl,
    pub team: String,
    pub team_color: Color,
    /// Absent where the payload named none.
    pub ranking: Option<u8>,
    pub points: u16,
    pub nationality: String,
    pub nationality_flag_url: ImageUrl,
    pub gp_wins: Option<u8>,
    pub world_titles: Option<u8>,
    pub age: Option<u8>,
    pub weight: Option<Mass>,
    pub height: Option<Length>,
    pub race_engineer: Option<String>,
    pub debut_year: Option<u16>,
}

/// One session of a race weekend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub name: String,
    /// Wall clock at the circuit, or in the viewer's own zone where they
    /// asked for that — the host resolves the instant and converts it.
    pub starts_at: Option<LocalDateTime>,
}

/// The upcoming race weekend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NextRace {
    pub gp_name: String,
    pub country_name: String,
    pub country_flag_url: ImageUrl,
    /// IANA zone the circuit keeps, e.g. `Europe/Monaco`. The one
    /// thing that turns a session's wall clock into an instant.
    pub venue_timezone: Option<String>,
    pub date_start: Option<CalendarDate>,
    pub date_end: Option<CalendarDate>,
    pub circuit_name: String,
    pub circuit_image_url: ImageUrl,
    pub track_length: Option<Length>,
    pub total_laps: Option<u16>,
    pub race_distance: Option<Length>,
    pub drs_zones: Option<u8>,
    pub tire_compounds: Option<String>,
    pub sessions: Vec<Session>,
}

/// A row of a running session's timing board. Not every column
/// applies to every session type; qualifying has no gap to the leader,
/// practice no interval.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimingRow {
    /// Absent where the payload named none.
    pub position: Option<u8>,
    pub driver_code: String,
    pub driver_name: String,
    pub team_logo_url: ImageUrl,
    pub team_color: Color,
    pub position_change: i8,
    pub gap_to_leader: TimingText,
    pub interval: TimingText,
    pub last_lap_time: TimingText,
    pub best_lap_time: TimingText,
    pub total_time: TimingText,
    pub tire_compound: Option<TireCompound>,
    pub tire_age: u8,
    pub sectors: [Option<Sector>; 3],
    pub in_pit: bool,
    pub is_out_lap: bool,
    pub fastest_lap: bool,
    pub status: Option<DriverStatus>,
}

/// A session's timing board.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimingBoard {
    /// The server's own header, e.g. `LAP 15/57` or `Q1 / 14:03`.
    pub session_label: String,
    pub gp_name: String,
    pub country_flag_url: ImageUrl,
    /// Absent outside a race, and before one starts.
    pub current_lap: Option<u16>,
    pub total_laps: Option<u16>,
    pub rows: Vec<TimingRow>,
}

/// What a live resource last said.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum LiveBoard {
    /// Nothing fetched yet.
    #[default]
    Unknown,
    /// The server answered, and no session is running.
    Idle,
    /// A session is running.
    Running(Box<TimingBoard>),
}

impl LiveBoard {
    /// A board with no rows is not a session in progress:
    /// the server empties the entry list around a session's edges,
    /// so having entries is the test for one being live.
    #[must_use]
    pub fn from_board(board: TimingBoard) -> Self {
        if board.rows.is_empty() {
            Self::Idle
        } else {
            Self::Running(Box::new(board))
        }
    }

    #[must_use]
    pub fn board(&self) -> Option<&TimingBoard> {
        match self {
            Self::Running(board) => Some(board),
            Self::Unknown | Self::Idle => None,
        }
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running(_))
    }
}

/// A constructor, as the teams table names it.
///
/// The mark lives here rather than beside a driver: the per-driver
/// resources name no logo, and a season has a couple of dozen drivers
/// to eleven teams.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Team {
    pub id: u64,
    pub name: String,
    pub logo_url: ImageUrl,
    pub color: Color,
}

/// Which constructor a driver races for. The driver payloads name a team
/// without identifying it, so the index is the only tie to a mark.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriverTeam {
    pub jolpica_id: String,
    pub team_id: u64,
}

/// Everything fetched so far.
/// A resource that has never answered — or answered
/// while the server was still warming its caches — stays empty,
/// and the screens show their own missing-data state for it.
#[derive(Clone, Debug, Default)]
pub struct Data {
    pub standings: Vec<StandingsRow>,
    pub driver_stats: Vec<DriverStats>,
    pub driver: Option<DriverStats>,
    pub teams: Vec<Team>,
    pub driver_teams: Vec<DriverTeam>,
    pub next_race: Option<NextRace>,
    pub live_race: LiveBoard,
    pub live_quali: LiveBoard,
    pub live_practice: LiveBoard,
}

impl Data {
    /// The running session, preferring the race, then qualifying,
    /// then practice.
    #[must_use]
    pub fn running_board(&self) -> Option<&TimingBoard> {
        self.live_race
            .board()
            .or_else(|| self.live_quali.board())
            .or_else(|| self.live_practice.board())
    }

    /// Whether any live resource reports a session in progress.
    #[must_use]
    pub fn any_session_running(&self) -> bool {
        self.live_race.is_running()
            || self.live_quali.is_running()
            || self.live_practice.is_running()
    }

    /// The mark for the constructor a driver races for.
    ///
    /// The index names the team by id and the snapshot carries the mark
    /// under it; a driver either has yet to answer or simply has no mark,
    /// which draws as the livery colour.
    #[must_use]
    pub fn team_logo(&self, jolpica_id: &str) -> ImageUrl {
        self.team_of(jolpica_id)
            .map(|team| team.logo_url.clone())
            .unwrap_or_default()
    }

    /// The constructor `jolpica_id` races for,
    /// by the same two hops the mark takes.
    #[must_use]
    pub fn team_of(&self, jolpica_id: &str) -> Option<&Team> {
        self.driver_teams
            .iter()
            .find(|row| row.jolpica_id == jolpica_id)
            .and_then(|row| self.teams.iter().find(|team| team.id == row.team_id))
    }

    /// The statistics row for the selected driver.
    ///
    /// The per-driver resource carries the fuller card,
    /// the table the season's figures; both key by `jolpica_id`,
    /// which joins them without leaning on a car number being unique.
    /// The card stands alone for a driver the table has no row for.
    #[must_use]
    pub fn selected_driver_stats(&self) -> Option<&DriverStats> {
        let id = self.driver.as_ref()?.jolpica_id.as_str();
        self.driver_stats
            .iter()
            .find(|row| row.jolpica_id == id)
            .or(self.driver.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CarNumber, Color, Data, DriverStats, DriverStatus, DriverTeam, FALLBACK_TEAM_COLOR,
        ImageUrl, LiveBoard, SectorColor, Team, TimingBoard, TimingText, TireCompound, team_color,
    };

    fn board(gp: &str) -> LiveBoard {
        LiveBoard::Running(Box::new(TimingBoard {
            gp_name: gp.to_owned(),
            ..TimingBoard::default()
        }))
    }

    fn driver(slug: &str, number: u8, name: &str) -> DriverStats {
        DriverStats {
            jolpica_id: slug.to_owned(),
            number: Some(CarNumber::new(number)),
            name: name.to_owned(),
            ..DriverStats::default()
        }
    }

    #[test]
    fn a_livery_colour_reads_with_or_without_its_hash() {
        let expected = Color::from_hex(0x00_D7_B6);
        assert_eq!(team_color("00D7B6"), expected);
        assert_eq!(team_color("#00D7B6"), expected);
    }

    #[test]
    fn an_unusable_livery_colour_falls_back_rather_than_dropping_the_row() {
        for hex in ["", "00D7B", "00D7B6AA", "ZZZZZZ", "#"] {
            assert_eq!(
                team_color(hex),
                FALLBACK_TEAM_COLOR,
                "`{hex}` must not yield a colour",
            );
        }
    }

    #[test]
    fn an_absent_image_is_distinguishable_from_one_we_have() {
        assert!(!ImageUrl::default().is_present());
        assert!(ImageUrl::from("https://example.test/a.png".to_owned()).is_present());
    }

    /// A country the upstream holds no flag image of arrives as the emoji.
    #[test]
    fn a_flag_emoji_is_not_something_to_fetch() {
        assert!(!ImageUrl::from("🇮🇹".to_owned()).is_present());
    }

    /// A driver's own payload names a team without identifying it, so
    /// the mark takes two hops — and matching on the name instead went
    /// unnoticed here for as long as both sides spelled it alike.
    #[test]
    fn a_drivers_mark_comes_from_the_team_its_index_row_names() {
        let data = Data {
            teams: vec![Team {
                id: 276_189,
                name: "Ferrari".to_owned(),
                logo_url: ImageUrl::from("https://cdn.test/ferrari.png".to_owned()),
                color: FALLBACK_TEAM_COLOR,
            }],
            driver_teams: vec![DriverTeam {
                jolpica_id: "leclerc".to_owned(),
                team_id: 276_189,
            }],
            ..Data::default()
        };
        assert_eq!(
            data.team_logo("leclerc").as_str(),
            "https://cdn.test/ferrari.png"
        );
        assert!(
            !data.team_logo("hamilton").is_present(),
            "a driver the index has no row for races for no known team"
        );
    }

    #[test]
    fn the_servers_dash_placeholder_counts_as_nothing_to_print() {
        assert!(TimingText::default().is_blank());
        assert!(TimingText::from("-".to_owned()).is_blank());
        assert!(!TimingText::from("LEADER".to_owned()).is_blank());
        assert!(!TimingText::from("+1.204".to_owned()).is_blank());
    }

    #[test]
    fn tire_compounds_round_trip_their_wire_letters() {
        for compound in [
            TireCompound::Soft,
            TireCompound::Medium,
            TireCompound::Hard,
            TireCompound::Intermediate,
            TireCompound::Wet,
        ] {
            assert_eq!(TireCompound::from_wire(compound.label()), Some(compound));
        }
        assert_eq!(TireCompound::from_wire("X"), None);
    }

    #[test]
    fn an_unrecognised_sector_colour_reads_as_normal() {
        assert_eq!(SectorColor::from_wire("purple"), SectorColor::OverallBest);
        assert_eq!(SectorColor::from_wire("green"), SectorColor::PersonalBest);
        assert_eq!(SectorColor::from_wire("white"), SectorColor::Normal);
        assert_eq!(SectorColor::from_wire("chartreuse"), SectorColor::Normal);
    }

    #[test]
    fn an_unknown_driver_status_is_absent_rather_than_wrong() {
        assert_eq!(DriverStatus::from_wire("RET"), Some(DriverStatus::Retired));
        assert_eq!(DriverStatus::from_wire("BOX"), None);
    }

    #[test]
    fn only_the_non_finishing_statuses_read_as_out() {
        assert!(DriverStatus::Retired.is_out());
        assert!(DriverStatus::Disqualified.is_out());
        assert!(!DriverStatus::Running.is_out());
        // A finisher is not "out" — the flag marks a car that stopped early.
        assert!(!DriverStatus::Finished.is_out());
    }

    #[test]
    fn the_race_outranks_qualifying_which_outranks_practice() {
        let mut data = Data {
            live_practice: board("practice"),
            ..Data::default()
        };
        assert_eq!(
            data.running_board().map(|b| b.gp_name.as_str()),
            Some("practice")
        );
        data.live_quali = board("quali");
        assert_eq!(
            data.running_board().map(|b| b.gp_name.as_str()),
            Some("quali")
        );
        data.live_race = board("race");
        assert_eq!(
            data.running_board().map(|b| b.gp_name.as_str()),
            Some("race")
        );
    }

    #[test]
    fn a_board_without_rows_is_not_a_session_in_progress() {
        // The server empties the entry list around a session's edges;
        // an empty board must not outrank the next-race screen.
        assert_eq!(
            LiveBoard::from_board(TimingBoard::default()),
            LiveBoard::Idle,
        );
        assert!(
            LiveBoard::from_board(TimingBoard {
                rows: vec![super::TimingRow::default()],
                ..TimingBoard::default()
            })
            .is_running(),
        );
    }

    #[test]
    fn an_idle_board_is_not_a_running_session() {
        let data = Data {
            live_race: LiveBoard::Idle,
            live_quali: LiveBoard::Unknown,
            ..Data::default()
        };
        assert!(!data.any_session_running());
        assert!(data.running_board().is_none());
    }

    #[test]
    fn the_selected_driver_joins_the_stats_table_by_its_key() {
        let data = Data {
            driver: Some(driver("hamilton", 44, "Lewis Hamilton")),
            driver_stats: vec![driver("max_verstappen", 1, "Max Verstappen"), {
                let mut row = driver("hamilton", 44, "Lewis Hamilton");
                row.gp_wins = Some(105);
                row
            }],
            ..Data::default()
        };
        let selected = data
            .selected_driver_stats()
            .expect("BUG: the joined row must be found");
        assert_eq!(selected.gp_wins, Some(105), "the full figures must win");
    }

    /// Car numbers are unique within a season but not across the sources
    /// the two resources derive from, which is why the join left them.
    #[test]
    fn a_number_shared_with_another_driver_joins_neither_to_the_other() {
        let data = Data {
            driver: Some(driver("hamilton", 44, "Lewis Hamilton")),
            driver_stats: vec![{
                let mut row = driver("hulkenberg", 44, "Nico Hülkenberg");
                row.gp_wins = Some(0);
                row
            }],
            ..Data::default()
        };
        assert_eq!(
            data.selected_driver_stats().map(|row| row.name.as_str()),
            Some("Lewis Hamilton"),
            "the card stands rather than taking another driver's figures"
        );
    }

    #[test]
    fn a_driver_missing_from_the_stats_table_still_renders_what_we_have() {
        // The tables come from different upstreams, so a driver can
        // appear in one before the other. The thinner row beats nothing.
        let data = Data {
            driver: Some(driver("bearman_reserve", 87, "Reserve Driver")),
            driver_stats: vec![driver("max_verstappen", 1, "Max Verstappen")],
            ..Data::default()
        };
        assert_eq!(
            data.selected_driver_stats().map(|row| row.name.as_str()),
            Some("Reserve Driver"),
        );
    }

    #[test]
    fn nothing_is_selected_before_the_driver_resource_answers() {
        let data = Data {
            driver_stats: vec![driver("max_verstappen", 1, "Max Verstappen")],
            ..Data::default()
        };
        assert!(data.selected_driver_stats().is_none());
    }
}
