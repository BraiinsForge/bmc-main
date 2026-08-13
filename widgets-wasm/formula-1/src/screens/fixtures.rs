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

use bmc_wasm_sdk::{CalendarDate, Length, LocalDateTime, Mass};

#[cfg(not(target_arch = "wasm32"))]
use crate::images::ImageKind;
use crate::model::{
    CarNumber, DriverStats, ImageUrl, NextRace, Session, SizeBucket, StandingsRow, team_color,
};
use crate::screens::driver::DriverViewData;
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

/// A fixture URL under the fake CDN, from whatever names the subject.
/// The `.test` TLD never resolves: fixtures are cache keys the stories
/// seed, not addresses.
fn image_url(kind: &str, subject: &str) -> ImageUrl {
    let mut url = String::from("https://cdn.example.test/");
    url.push_str(kind);
    url.push('/');
    url.push_str(&subject.to_lowercase().replace(' ', "-"));
    url.push_str(".png");
    ImageUrl::from(url)
}

fn flag_url(country: &str) -> ImageUrl {
    image_url("flag", country)
}

fn headshot_url(driver: &str) -> ImageUrl {
    image_url("headshot", driver)
}

fn logo_url(team: &str) -> ImageUrl {
    image_url("logo", team)
}

/// What the stories seed the image cache with: every headshot and flag
/// URL the fixtures carry, backed by one generic avatar and one
/// fictional flag, plus a generated circuit outline. Team logos are
/// left unseeded so the embedded marks keep drawing.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn image_seeds() -> Vec<(ImageKind, ImageUrl, &'static [u8])> {
    const HEADSHOT: &[u8] = include_bytes!("../../assets/fixtures/headshot-generic.png");
    const FLAG: &[u8] = include_bytes!("../../assets/fixtures/flag-generic.png");

    let mut seeds = Vec::new();
    for (driver, country, ..) in &GRID {
        seeds.push((ImageKind::Headshot, headshot_url(driver), HEADSHOT));
        seeds.push((ImageKind::Flag, flag_url(country), FLAG));
    }
    for driver in &DRIVERS {
        seeds.push((ImageKind::Headshot, headshot_url(driver.name), HEADSHOT));
        seeds.push((ImageKind::Flag, flag_url(driver.nationality), FLAG));
    }
    seeds.push((ImageKind::Flag, flag_url("Netherlands"), FLAG));
    seeds.push((
        ImageKind::Circuit,
        image_url("circuit", "Circuit Zandvoort"),
        include_bytes!("../../assets/fixtures/circuit.png"),
    ));
    seeds
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
                team_logo_url: logo_url(team),
                team_color: team_color(livery),
                country_code: (*country).to_owned(),
                country_flag_url: flag_url(country),
                points: *points,
                headshot_url: headshot_url(driver),
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
        country_flag_url: flag_url("Netherlands"),
        venue_timezone: Some("Europe/Brussels".to_owned()),
        date_start: Some(weekend_day(21)),
        date_end: Some(weekend_day(23)),
        circuit_name: "Circuit Zandvoort".to_owned(),
        circuit_image_url: image_url("circuit", "Circuit Zandvoort"),
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

/// One driver as the upstream describes them,
/// before the widget's own types wrap it.
///
/// Four have neither an engineer nor a debut season named upstream,
/// and Gasly's 177 kg is the upstream's own: his weight repeats
/// his height. The oddities stay because the screens have to seat them.
struct Driver {
    name: &'static str,
    number: u8,
    team: &'static str,
    livery: &'static str,
    ranking: u8,
    points: u16,
    nationality: &'static str,
    gp_wins: u8,
    world_titles: u8,
    age: u8,
    kilograms: f64,
    centimeters: f64,
    engineer: Option<&'static str>,
    debut: Option<u16>,
}

const DRIVERS: [Driver; 22] = [
    Driver {
        name: "Kimi Antonelli",
        number: 12,
        team: "Mercedes",
        livery: "00D7B6",
        ranking: 1,
        points: 219,
        nationality: "Italy",
        gp_wins: 6,
        world_titles: 0,
        age: 20,
        kilograms: 70.0,
        centimeters: 172.0,
        engineer: Some("Pete Bonnington"),
        debut: Some(2025),
    },
    Driver {
        name: "Lewis Hamilton",
        number: 44,
        team: "Ferrari",
        livery: "ED1131",
        ranking: 2,
        points: 169,
        nationality: "England",
        gp_wins: 106,
        world_titles: 7,
        age: 41,
        kilograms: 73.0,
        centimeters: 174.0,
        engineer: Some("Riccardo Adami"),
        debut: Some(2007),
    },
    Driver {
        name: "George Russell",
        number: 63,
        team: "Mercedes",
        livery: "00D7B6",
        ranking: 3,
        points: 160,
        nationality: "England",
        gp_wins: 7,
        world_titles: 0,
        age: 28,
        kilograms: 70.0,
        centimeters: 185.0,
        engineer: Some("Marcus Dudley"),
        debut: Some(2019),
    },
    Driver {
        name: "Charles Leclerc",
        number: 16,
        team: "Ferrari",
        livery: "ED1131",
        ranking: 4,
        points: 138,
        nationality: "Monaco",
        gp_wins: 9,
        world_titles: 0,
        age: 29,
        kilograms: 69.0,
        centimeters: 180.0,
        engineer: Some("Bryan Bozzi"),
        debut: Some(2018),
    },
    Driver {
        name: "Lando Norris",
        number: 4,
        team: "McLaren",
        livery: "F47600",
        ranking: 5,
        points: 128,
        nationality: "England",
        gp_wins: 12,
        world_titles: 1,
        age: 27,
        kilograms: 68.0,
        centimeters: 170.0,
        engineer: Some("Will Joseph"),
        debut: Some(2019),
    },
    Driver {
        name: "Max Verstappen",
        number: 1,
        team: "Red Bull Racing",
        livery: "0600EF",
        ranking: 6,
        points: 109,
        nationality: "Netherlands",
        gp_wins: 71,
        world_titles: 4,
        age: 29,
        kilograms: 72.0,
        centimeters: 181.0,
        engineer: Some("Gianpiero Lambiase"),
        debut: Some(2015),
    },
    Driver {
        name: "Oscar Piastri",
        number: 81,
        team: "McLaren",
        livery: "F47600",
        ranking: 7,
        points: 92,
        nationality: "Australia",
        gp_wins: 9,
        world_titles: 0,
        age: 25,
        kilograms: 68.0,
        centimeters: 178.0,
        engineer: Some("Tom Stallard"),
        debut: Some(2023),
    },
    Driver {
        name: "Isack Hadjar",
        number: 6,
        team: "Red Bull Racing",
        livery: "0600EF",
        ranking: 8,
        points: 68,
        nationality: "France",
        gp_wins: 0,
        world_titles: 0,
        age: 22,
        kilograms: 65.0,
        centimeters: 167.0,
        engineer: Some("Richard Wood"),
        debut: Some(2025),
    },
    Driver {
        name: "Liam Lawson",
        number: 30,
        team: "Racing Bulls",
        livery: "2B4562",
        ranking: 9,
        points: 43,
        nationality: "Australia",
        gp_wins: 0,
        world_titles: 0,
        age: 24,
        kilograms: 72.0,
        centimeters: 174.0,
        engineer: Some("Alexandre Iliopoulos"),
        debut: Some(2023),
    },
    Driver {
        name: "Pierre Gasly",
        number: 10,
        team: "Alpine",
        livery: "0090FF",
        ranking: 10,
        points: 42,
        nationality: "France",
        gp_wins: 1,
        world_titles: 0,
        age: 30,
        kilograms: 177.0,
        centimeters: 177.0,
        engineer: Some("Ciaron Pilbeam"),
        debut: Some(2017),
    },
    Driver {
        name: "Arvid Lindblad",
        number: 41,
        team: "Racing Bulls",
        livery: "2B4562",
        ranking: 11,
        points: 23,
        nationality: "England",
        gp_wins: 0,
        world_titles: 0,
        age: 19,
        kilograms: 68.0,
        centimeters: 173.0,
        engineer: None,
        debut: None,
    },
    Driver {
        name: "Franco Colapinto",
        number: 43,
        team: "Alpine",
        livery: "0090FF",
        ranking: 12,
        points: 19,
        nationality: "Argentina",
        gp_wins: 0,
        world_titles: 0,
        age: 23,
        kilograms: 71.0,
        centimeters: 174.0,
        engineer: None,
        debut: None,
    },
    Driver {
        name: "Oliver Bearman",
        number: 87,
        team: "Haas",
        livery: "B6BABD",
        ranking: 13,
        points: 18,
        nationality: "England",
        gp_wins: 0,
        world_titles: 0,
        age: 21,
        kilograms: 68.0,
        centimeters: 184.0,
        engineer: Some("Ronan O'Hare"),
        debut: Some(2024),
    },
    Driver {
        name: "Gabriel Bortoleto",
        number: 5,
        team: "Audi",
        livery: "900000",
        ranking: 14,
        points: 10,
        nationality: "Brazil",
        gp_wins: 0,
        world_titles: 0,
        age: 22,
        kilograms: 71.0,
        centimeters: 184.0,
        engineer: Some("Jose Manuel Lopez"),
        debut: Some(2025),
    },
    Driver {
        name: "Carlos Sainz",
        number: 55,
        team: "Williams",
        livery: "005AFF",
        ranking: 15,
        points: 6,
        nationality: "Spain",
        gp_wins: 4,
        world_titles: 0,
        age: 32,
        kilograms: 64.0,
        centimeters: 178.0,
        engineer: Some("Gaetan Jego"),
        debut: Some(2015),
    },
    Driver {
        name: "Alexander Albon",
        number: 23,
        team: "Williams",
        livery: "005AFF",
        ranking: 16,
        points: 5,
        nationality: "Thailand",
        gp_wins: 0,
        world_titles: 0,
        age: 30,
        kilograms: 74.0,
        centimeters: 186.0,
        engineer: Some("James Urwin"),
        debut: Some(2019),
    },
    Driver {
        name: "Esteban Ocon",
        number: 31,
        team: "Haas",
        livery: "B6BABD",
        ranking: 17,
        points: 3,
        nationality: "France",
        gp_wins: 1,
        world_titles: 0,
        age: 30,
        kilograms: 66.0,
        centimeters: 186.0,
        engineer: Some("Josh Peckett"),
        debut: Some(2016),
    },
    Driver {
        name: "Nico Hulkenberg",
        number: 27,
        team: "Audi",
        livery: "900000",
        ranking: 18,
        points: 2,
        nationality: "Germany",
        gp_wins: 0,
        world_titles: 0,
        age: 39,
        kilograms: 78.0,
        centimeters: 184.0,
        engineer: Some("Steven Petrik"),
        debut: Some(2010),
    },
    Driver {
        name: "Fernando Alonso",
        number: 14,
        team: "Aston Martin",
        livery: "006F62",
        ranking: 19,
        points: 1,
        nationality: "Spain",
        gp_wins: 32,
        world_titles: 2,
        age: 45,
        kilograms: 68.0,
        centimeters: 171.0,
        engineer: Some("Bob Sherlock"),
        debut: Some(2001),
    },
    Driver {
        name: "Lance Stroll",
        number: 18,
        team: "Aston Martin",
        livery: "006F62",
        ranking: 20,
        points: 0,
        nationality: "Canada",
        gp_wins: 0,
        world_titles: 0,
        age: 28,
        kilograms: 70.0,
        centimeters: 182.0,
        engineer: Some("Gary Gannon"),
        debut: Some(2017),
    },
    Driver {
        name: "Valtteri Bottas",
        number: 77,
        team: "Cadillac",
        livery: "909090",
        ranking: 21,
        points: 0,
        nationality: "Finland",
        gp_wins: 10,
        world_titles: 0,
        age: 37,
        kilograms: 69.0,
        centimeters: 173.0,
        engineer: None,
        debut: None,
    },
    Driver {
        name: "Sergio Perez",
        number: 11,
        team: "Cadillac",
        livery: "909090",
        ranking: 22,
        points: 0,
        nationality: "Mexico",
        gp_wins: 6,
        world_titles: 0,
        age: 36,
        kilograms: 63.0,
        centimeters: 173.0,
        engineer: None,
        debut: None,
    },
];

/// Every driver on the grid, in the order the standings put them.
#[must_use]
pub fn drivers() -> Vec<DriverStats> {
    DRIVERS
        .iter()
        .map(|driver| DriverStats {
            name: driver.name.to_owned(),
            number: CarNumber::new(driver.number),
            headshot_url: headshot_url(driver.name),
            team: driver.team.to_owned(),
            team_color: team_color(driver.livery),
            ranking: driver.ranking,
            points: driver.points,
            nationality: driver.nationality.to_owned(),
            nationality_flag_url: flag_url(driver.nationality),
            gp_wins: Some(driver.gp_wins),
            world_titles: Some(driver.world_titles),
            age: Some(driver.age),
            weight: Some(Mass::from_kilograms(driver.kilograms)),
            height: Some(Length::from_centimeters(driver.centimeters)),
            race_engineer: driver.engineer.map(str::to_owned),
            debut_year: driver.debut,
        })
        .collect()
}

/// The championship leader, whom the design's own frame shows.
#[must_use]
pub fn driver(bucket: SizeBucket) -> DriverViewData {
    DriverViewData {
        bucket,
        driver: drivers().into_iter().next(),
    }
}

/// A rookie: no engineer named yet, no debut season, nothing won.
#[must_use]
pub fn driver_sparse(bucket: SizeBucket) -> DriverViewData {
    DriverViewData {
        bucket,
        driver: drivers().into_iter().find(|it| it.race_engineer.is_none()),
    }
}

/// Between seasons, or before the first reply has landed.
#[must_use]
pub fn driver_unavailable(bucket: SizeBucket) -> DriverViewData {
    DriverViewData {
        bucket,
        driver: None,
    }
}
