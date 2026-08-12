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

//! Model values the storybook and the tests render.
//!
//! Typed fixtures rather than recorded payloads: the states worth
//! reviewing are the ones a live server rarely serves on demand.

use bmc_wasm_sdk::{CalendarDate, Length, LocalDateTime};

use crate::model::{ImageUrl, NextRace, Session, SizeBucket, StandingsRow, team_color};
use crate::screens::next_race::NextRaceViewData;
use crate::screens::standings::StandingsViewData;

/// The top ten as the server sends them, down to the spelling of a
/// constructor and the three letters it calls a country by — a fixture
/// of our own invention only agrees with itself.
const GRID: [(&str, &str, &str, &str, u16); 10] = [
    ("Kimi Antonelli", "ITA", "Mercedes", "00D7B6", 219),
    ("Lewis Hamilton", "ENG", "Ferrari", "ED1131", 169),
    ("George Russell", "ENG", "Mercedes", "00D7B6", 160),
    ("Charles Leclerc", "MON", "Ferrari", "ED1131", 138),
    ("Lando Norris", "ENG", "McLaren", "F47600", 128),
    ("Max Verstappen", "NED", "Red Bull Racing", "0600EF", 109),
    ("Oscar Piastri", "AUS", "McLaren", "F47600", 92),
    ("Isack Hadjar", "FRA", "Red Bull Racing", "0600EF", 68),
    ("Liam Lawson", "AUS", "Racing Bulls", "2B4562", 43),
    ("Pierre Gasly", "FRA", "Alpine", "0090FF", 42),
];

/// An image the widget has a URL for; a fixture only says "there is one".
fn some_image() -> ImageUrl {
    ImageUrl::from("https://cdn.example.test/f1.png".to_owned())
}

#[must_use]
pub fn standings_rows() -> Vec<StandingsRow> {
    GRID.iter()
        .enumerate()
        .map(
            |(index, (driver, country, team, livery, points))| StandingsRow {
                position: u8::try_from(index + 1).unwrap_or(u8::MAX),
                driver_name: (*driver).to_owned(),
                driver_code: driver
                    .split_whitespace()
                    .next_back()
                    .unwrap_or(driver)
                    .to_uppercase()
                    .chars()
                    .take(3)
                    .collect(),
                team_name: (*team).to_owned(),
                team_logo_url: some_image(),
                team_color: team_color(livery),
                country_code: (*country).to_owned(),
                country_flag_url: some_image(),
                points: *points,
                headshot_url: some_image(),
            },
        )
        .collect()
}

/// The standings as they read mid-season.
#[must_use]
pub fn standings(bucket: SizeBucket) -> StandingsViewData {
    StandingsViewData {
        bucket,
        rows: standings_rows(),
    }
}

/// Nothing stored yet: first reply outstanding, or a cold server 503ing.
#[must_use]
pub fn standings_empty(bucket: SizeBucket) -> StandingsViewData {
    StandingsViewData {
        bucket,
        rows: Vec::new(),
    }
}

/// A season's opening weekend: everyone level on nothing.
#[must_use]
pub fn standings_season_start(bucket: SizeBucket) -> StandingsViewData {
    let mut view = standings(bucket);
    for row in &mut view.rows {
        row.points = 0;
    }
    view
}

/// The longest names on the grid against three-digit scores. The server
/// names a constructor plainly — `Red Bull Racing`, never the sponsors
/// in front of it — so this is as wide as the columns are ever asked to
/// be. Rows differ so the eye can tell which column is losing.
const LONGEST: [(&str, &str); 10] = [
    ("Gabriel Bortoleto", "Red Bull Racing"),
    ("Franco Colapinto", "Aston Martin"),
    ("Max Verstappen", "Racing Bulls"),
    ("Alexander Albon", "Red Bull Racing"),
    ("Fernando Alonso", "Aston Martin"),
    ("Valtteri Bottas", "Racing Bulls"),
    ("Charles Leclerc", "Red Bull Racing"),
    ("Nico Hulkenberg", "Aston Martin"),
    ("Oliver Bearman", "Racing Bulls"),
    ("Arvid Lindblad", "Red Bull Racing"),
];

#[must_use]
pub fn standings_widest(bucket: SizeBucket) -> StandingsViewData {
    let mut view = standings(bucket);
    for (row, (driver, team)) in view.rows.iter_mut().zip(LONGEST) {
        driver.clone_into(&mut row.driver_name);
        team.clone_into(&mut row.team_name);
        row.points = 400 + u16::from(row.position);
    }
    view
}

/// The weekend below opens on Friday the 21st of August 2026 and runs to
/// Sunday the 23rd, so each day advances the weekday by one.
const OPENING_DAY: u8 = 21;
const FRIDAY: u8 = 4;

/// A day of that weekend, carrying the weekday the host would have
/// resolved for it.
#[must_use]
pub fn weekend_day(day: u8) -> CalendarDate {
    CalendarDate {
        year: 2026,
        month: 8,
        day,
        weekday: FRIDAY + (day - OPENING_DAY),
    }
}

/// A moment of one of those days.
#[must_use]
pub fn weekend_time(day: u8, hour: u8, minute: u8) -> LocalDateTime {
    let date = weekend_day(day);
    LocalDateTime {
        year: date.year,
        month: date.month,
        day: date.day,
        hour,
        minute,
        second: 0,
        weekday: date.weekday,
    }
}

fn session(name: &str, day: u8, hour: u8, minute: u8) -> Session {
    Session {
        name: name.to_owned(),
        starts_at: Some(weekend_time(day, hour, minute)),
    }
}

/// Zandvoort as the server sends it — a sprint weekend, so the schedule
/// is the longest the screens have to seat.
#[must_use]
pub fn next_race_weekend() -> NextRace {
    NextRace {
        gp_name: "Dutch GP".to_owned(),
        country_name: "Netherlands".to_owned(),
        country_flag_url: some_image(),
        venue_timezone: Some("Europe/Brussels".to_owned()),
        date_start: Some(weekend_day(21)),
        date_end: Some(weekend_day(23)),
        circuit_name: "Circuit Zandvoort".to_owned(),
        circuit_image_url: some_image(),
        track_length: Some(Length::from_kilometers(4.259)),
        total_laps: Some(72),
        race_distance: Some(Length::from_kilometers(306.648)),
        drs_zones: Some(2),
        tire_compounds: Some("C1, C2, C3".to_owned()),
        sessions: vec![
            session("P1", 21, 10, 30),
            session("Sprint Quali", 21, 14, 30),
            session("Sprint Race", 22, 10, 0),
            session("Quali", 22, 14, 0),
            session("Race", 23, 13, 0),
        ],
    }
}

#[must_use]
pub fn next_race(bucket: SizeBucket) -> NextRaceViewData {
    NextRaceViewData {
        bucket,
        race: Some(next_race_weekend()),
    }
}

/// Between seasons, or before the first reply has landed.
#[must_use]
pub fn next_race_unavailable(bucket: SizeBucket) -> NextRaceViewData {
    NextRaceViewData { bucket, race: None }
}

/// A weekend the upstream knows of but has no detail for yet — every
/// stat null, no session times published.
#[must_use]
pub fn next_race_sparse(bucket: SizeBucket) -> NextRaceViewData {
    let mut view = next_race(bucket);
    if let Some(race) = view.race.as_mut() {
        race.track_length = None;
        race.total_laps = None;
        race.race_distance = None;
        race.drs_zones = None;
        for session in &mut race.sessions {
            session.starts_at = None;
        }
    }
    view
}

/// The longest names on the calendar against the same sprint schedule.
#[must_use]
pub fn next_race_widest(bucket: SizeBucket) -> NextRaceViewData {
    let mut view = next_race(bucket);
    if let Some(race) = view.race.as_mut() {
        "Emilia-Romagna GP".clone_into(&mut race.gp_name);
        "United Arab Emirates".clone_into(&mut race.country_name);
        "Autodromo Enzo e Dino Ferrari".clone_into(&mut race.circuit_name);
    }
    view
}
