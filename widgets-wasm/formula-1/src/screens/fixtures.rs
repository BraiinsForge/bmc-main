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

//! Model values the gallery and the tests render.
//!
//! Typed fixtures rather than recorded payloads: the states worth
//! reviewing are the ones a live server rarely serves on demand.

use bmc_wasm_sdk::{CalendarDate, Length, LocalDateTime, Mass};

use crate::images::ImageKind;
use crate::model::{
    CarNumber, DriverStats, DriverStatus, ImageUrl, LiveBoard, NextRace, Sector, SectorColor,
    Session, SizeBucket, StandingsRow, TimingBoard, TimingRow, TimingText, TireCompound,
    team_color,
};
use crate::screens::driver::DriverViewData;
use crate::screens::live::LiveViewData;
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
    ("Liam Lawson", "NZL", "Racing Bulls", "2B4562", 43),
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

/// Names a scene's artwork without carrying any: [`CIRCUIT`], a flag from
/// [`flag_asset`], a face from [`headshot_asset`], or a constructor's key
/// from [`team_logo_key`].
pub type AssetName = &'static str;

pub const CIRCUIT: AssetName = "circuit";

/// The flag a country draws, under either spelling the fixtures name it
/// by: a board carries the trigram a payload publishes, a driver profile
/// the country's name.
///
/// `None` for a country no scene carries, which then draws as a deck
/// draws it before a fetch lands.
#[must_use]
pub fn flag_asset(country: &str) -> Option<AssetName> {
    Some(match country {
        "ARG" | "Argentina" => "flag-arg",
        "AUS" | "Australia" => "flag-aus",
        "BRA" | "Brazil" => "flag-bra",
        "CAN" | "Canada" => "flag-can",
        "ENG" | "England" => "flag-gbr",
        "ESP" | "Spain" => "flag-esp",
        "FIN" | "Finland" => "flag-fin",
        "FRA" | "France" => "flag-fra",
        "GER" | "Germany" => "flag-ger",
        "ITA" | "Italy" => "flag-ita",
        "MEX" | "Mexico" => "flag-mex",
        "MON" | "Monaco" => "flag-mon",
        "NED" | "Netherlands" => "flag-ned",
        "NZL" | "New Zealand" => "flag-nzl",
        "THA" | "Thailand" => "flag-tha",
        _ => return None,
    })
}

/// The invented faces, which stand in for portraits we may not publish.
/// One per seat on the grid — see [`headshot_asset`].
const HEADSHOTS: [AssetName; 22] = [
    "headshot-01",
    "headshot-02",
    "headshot-03",
    "headshot-04",
    "headshot-05",
    "headshot-06",
    "headshot-07",
    "headshot-08",
    "headshot-09",
    "headshot-10",
    "headshot-11",
    "headshot-12",
    "headshot-13",
    "headshot-14",
    "headshot-15",
    "headshot-16",
    "headshot-17",
    "headshot-18",
    "headshot-19",
    "headshot-20",
    "headshot-21",
    "headshot-22",
];

const _: () = assert!(
    HEADSHOTS.len() == DRIVERS.len(),
    "a face per driver, so no two share one",
);

/// A driver's face, chosen by name rather than by position in a list.
///
/// The same driver sits in more than one fixture list and every list
/// derives their headshot URL from the name, so the face has to follow
/// the name too — otherwise one list seeds a portrait the other wrote a
/// different one under. A driver no list names still gets a face, since
/// a scene may invent one.
#[must_use]
pub fn headshot_asset(driver_name: &str) -> AssetName {
    DRIVERS
        .iter()
        .position(|driver| driver.name == driver_name)
        .map_or_else(
            || {
                let spread = driver_name.bytes().fold(0_usize, |acc, byte| {
                    acc.wrapping_mul(31).wrapping_add(usize::from(byte))
                });
                HEADSHOTS[spread % HEADSHOTS.len()]
            },
            |seat| HEADSHOTS[seat],
        )
}

/// A mark per constructor, keyed by a word inside whatever a payload
/// calls the team. The marks are invented and numbered — like the
/// headshots, they depict nothing — so which number a team gets is
/// arbitrary. Order breaks ties: a Haas is often named for its Ferrari
/// power unit, and Racing Bulls would otherwise answer to Red Bull.
const LOGO_KEYS: &[(&str, AssetName)] = &[
    ("haas", "logo-01"),
    ("racing bulls", "logo-02"),
    ("red bull", "logo-03"),
    ("ferrari", "logo-04"),
    ("mclaren", "logo-05"),
    ("mercedes", "logo-06"),
    ("williams", "logo-07"),
    ("aston martin", "logo-08"),
    ("alpine", "logo-09"),
    ("audi", "logo-10"),
    ("cadillac", "logo-11"),
];

/// Which constructor's artwork a team name asks for.
/// `None` for a team no scene carries, which then draws as a deck draws
/// it before a fetch lands.
#[must_use]
pub fn team_logo_key(team_name: &str) -> Option<AssetName> {
    let name = team_name.to_lowercase();
    LOGO_KEYS
        .iter()
        .find_map(|(key, asset)| name.contains(key).then_some(*asset))
}

/// Every image the fixtures point at, as its URL and the artwork a scene
/// should seed under it.
///
/// The bytes stay with the scene rather than here. Only the gallery
/// compiles a scene file, so artwork kept there cannot reach the widget's
/// own binary however this module is later used.
#[must_use]
pub fn image_seeds() -> Vec<(ImageKind, ImageUrl, AssetName)> {
    let mut seeds = Vec::new();
    for (driver, country, team, ..) in &GRID {
        seeds.push((
            ImageKind::Headshot,
            headshot_url(driver),
            headshot_asset(driver),
        ));
        if let Some(flag) = flag_asset(country) {
            seeds.push((ImageKind::Flag, flag_url(country), flag));
        }
        if let Some(key) = team_logo_key(team) {
            seeds.push((ImageKind::TeamLogo, logo_url(team), key));
        }
    }
    for driver in &DRIVERS {
        seeds.push((
            ImageKind::Headshot,
            headshot_url(driver.name),
            headshot_asset(driver.name),
        ));
        if let Some(flag) = flag_asset(driver.nationality) {
            seeds.push((ImageKind::Flag, flag_url(driver.nationality), flag));
        }
        if let Some(key) = team_logo_key(driver.team) {
            seeds.push((ImageKind::TeamLogo, logo_url(driver.team), key));
        }
    }
    if let Some(flag) = flag_asset("Netherlands") {
        seeds.push((ImageKind::Flag, flag_url("Netherlands"), flag));
    }
    seeds.push((
        ImageKind::Circuit,
        image_url("circuit", "Circuit Zandvoort"),
        CIRCUIT,
    ));
    seeds
}

#[must_use]
pub fn standings_rows() -> Vec<StandingsRow> {
    GRID.iter()
        .enumerate()
        .map(
            |(index, (driver, country, team, livery, points))| StandingsRow {
                position: u8::try_from(index + 1).ok(),
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

#[must_use]
pub fn standings_unranked(bucket: SizeBucket) -> StandingsViewData {
    let mut view = standings(bucket);
    for row in &mut view.rows {
        row.position = None;
    }
    view
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

/// The longest names on the grid against three-digit scores.
/// The server names a constructor plainly — `Red Bull Racing`,
/// never the sponsors in front of it — so this is as wide
/// as the columns are ever asked to be.
/// Rows differ so the eye can tell which column is losing.
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
        row.points = 400 + row.position.map_or(0, u16::from);
    }
    view
}

/// The weekend below opens on Friday the 21st of August 2026 and runs
/// to Sunday the 23rd, so each day advances the weekday by one.
const OPENING_DAY: u8 = 21;
const FRIDAY: u8 = 4;

/// A day of that weekend, carrying the weekday
/// the host would have resolved for it.
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

/// Zandvoort as the server sends it — a sprint weekend,
/// so the schedule is the longest the screens have to seat.
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

/// A weekend the upstream knows of but has no detail for yet
/// — every stat null, no session times published.
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
        nationality: "New Zealand",
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

/// A fixture's join key, taken from the surname rather than listed
/// against every driver: it has to be unique and stable here, not equal
/// to whatever the upstream registry calls them.
fn slug_of(name: &str) -> String {
    name.rsplit(' ').next().unwrap_or(name).to_lowercase()
}

/// Every driver on the grid, in the order the standings put them.
#[must_use]
pub fn drivers() -> Vec<DriverStats> {
    DRIVERS
        .iter()
        .map(|driver| DriverStats {
            jolpica_id: slug_of(driver.name),
            name: driver.name.to_owned(),
            number: CarNumber::new(driver.number),
            headshot_url: headshot_url(driver.name),
            team: driver.team.to_owned(),
            team_color: team_color(driver.livery),
            ranking: Some(driver.ranking),
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

/// One driver's card, with the mark the teams table would have supplied.
#[must_use]
pub fn driver_card(bucket: SizeBucket, driver: Option<DriverStats>) -> DriverViewData {
    let team_logo_url = driver
        .as_ref()
        .map(|it| logo_url(&it.team))
        .unwrap_or_default();
    DriverViewData {
        bucket,
        driver,
        team_logo_url,
    }
}

/// The championship leader, whom the design's own frame shows.
#[must_use]
pub fn driver(bucket: SizeBucket) -> DriverViewData {
    driver_card(bucket, drivers().into_iter().next())
}

/// Fills the name and team columns, the two the standings cut.
#[must_use]
pub fn standings_ruler(bucket: SizeBucket, value: &str) -> StandingsViewData {
    let mut view = standings(bucket);
    for row in &mut view.rows {
        value.clone_into(&mut row.driver_name);
        value.clone_into(&mut row.team_name);
    }
    view
}

/// Fills the tyre compounds, the only info value the server
/// writes as text; the rest are figures this widget formats.
#[must_use]
pub fn next_race_ruler(bucket: SizeBucket, value: &str) -> NextRaceViewData {
    let mut view = next_race(bucket);
    if let Some(race) = view.race.as_mut() {
        race.tire_compounds = Some(value.to_owned());
    }
    view
}

/// Fills every text field of the card.
#[must_use]
pub fn driver_widest(bucket: SizeBucket, value: &str) -> DriverViewData {
    let mut driver = drivers()
        .into_iter()
        .next()
        .expect("BUG: fixtures name a driver");
    value.clone_into(&mut driver.team);
    value.clone_into(&mut driver.nationality);
    driver.race_engineer = Some(value.to_owned());
    driver_card(bucket, Some(driver))
}

#[must_use]
pub fn driver_unranked(bucket: SizeBucket) -> DriverViewData {
    let mut driver = drivers()
        .into_iter()
        .next()
        .expect("BUG: fixtures name a driver");
    driver.ranking = None;
    driver_card(bucket, Some(driver))
}

/// A rookie: no engineer named yet, no debut season, nothing won.
#[must_use]
pub fn driver_sparse(bucket: SizeBucket) -> DriverViewData {
    driver_card(
        bucket,
        drivers().into_iter().find(|it| it.race_engineer.is_none()),
    )
}

/// Between seasons, or before the first reply has landed.
#[must_use]
pub fn driver_unavailable(bucket: SizeBucket) -> DriverViewData {
    driver_card(bucket, None)
}

/// Pad `value` to `width` digits.
///
/// Hand-rolled because the timing formats need it and nothing here can
/// supply it: `fmt!` takes no width, and `format!` is banned widget-side.
fn zero_padded(value: u64, width: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(width.max(digits.len()));
    for _ in digits.len()..width {
        out.push('0');
    }
    out.push_str(&digits);
    out
}

/// A lap time as the server formats one: `m:ss.mmm`.
fn lap_time(seconds: f64) -> TimingText {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a lap time is a small positive number of seconds"
    )]
    let total_ms = (seconds * 1000.0).round().max(0.0) as u64;
    let minutes = total_ms.div_euclid(60_000);
    let rest_ms = total_ms.rem_euclid(60_000);
    let mut out = minutes.to_string();
    out.push(':');
    out.push_str(&zero_padded(rest_ms.div_euclid(1000), 2));
    out.push('.');
    out.push_str(&zero_padded(rest_ms.rem_euclid(1000), 3));
    TimingText::from(out)
}

/// A gap or interval, as the server formats one: `+s.mmm`.
fn gap_time(seconds: f64) -> TimingText {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a gap is a small positive number of seconds"
    )]
    let total_ms = (seconds * 1000.0).round().max(0.0) as u64;
    let mut out = String::from("+");
    out.push_str(&total_ms.div_euclid(1000).to_string());
    out.push('.');
    out.push_str(&zero_padded(total_ms.rem_euclid(1000), 3));
    TimingText::from(out)
}

/// One timing row, from the field's index and how far back it runs.
///
/// Off the whole 22-driver table rather than the standings' top ten:
/// the widest qualifying frame halves the field into two tables, and
/// ten rows would not show what that frame is for.
fn timing_row(index: usize, behind: f64) -> TimingRow {
    let entry = &DRIVERS[index];
    let code: String = entry
        .name
        .split_whitespace()
        .next_back()
        .unwrap_or(entry.name)
        .to_uppercase()
        .chars()
        .take(3)
        .collect();
    let lap = 78.4 + behind * 0.05;
    let sector = |share: f64, color: SectorColor| {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "sector times are read for display only"
        )]
        Some(Sector {
            seconds: (lap * share) as f32,
            color,
        })
    };
    TimingRow {
        position: u8::try_from(index + 1).ok(),
        driver_code: code,
        driver_name: entry.name.to_owned(),
        team_logo_url: logo_url(entry.team),
        team_color: team_color(entry.livery),
        // The leaders climb, the midfield holds, the tail slips back.
        position_change: match index {
            0..=2 => i8::try_from(3 - index).unwrap_or(0),
            3..=6 => 0,
            _ => -i8::try_from(index - 6).unwrap_or(0),
        },
        gap_to_leader: if index == 0 {
            TimingText::from("LEADER".to_owned())
        } else {
            gap_time(behind)
        },
        interval: if index == 0 {
            TimingText::default()
        } else {
            gap_time(behind / f64::from(u32::try_from(index).unwrap_or(1)))
        },
        last_lap_time: lap_time(lap),
        best_lap_time: lap_time(lap - 0.4),
        total_time: lap_time(lap * 12.0 + behind),
        tire_compound: [
            Some(TireCompound::Soft),
            Some(TireCompound::Medium),
            Some(TireCompound::Hard),
        ][index % 3],
        tire_age: u8::try_from(index * 4 % 31).unwrap_or(0),
        sectors: [
            sector(0.31, SectorColor::Normal),
            sector(0.36, SectorColor::PersonalBest),
            sector(0.33, SectorColor::OverallBest),
        ],
        in_pit: false,
        is_out_lap: false,
        fastest_lap: index == 1,
        status: Some(DriverStatus::Running),
    }
}

/// A session mid-flight: gaps opening down the order, a fastest lap,
/// one car in the pits and one retired.
#[must_use]
pub fn timing_board(label: &str) -> TimingBoard {
    // Another 1.7 s back for each place down the order,
    // so the gap column climbs while the interval stays roughly even.
    let mut rows: Vec<TimingRow> = (0..DRIVERS.len())
        .map(|index| timing_row(index, f64::from(u32::try_from(index).unwrap_or(0)) * 1.734))
        .collect();
    if let Some(pitted) = rows.get_mut(3) {
        pitted.in_pit = true;
    }
    if let Some(out) = rows.get_mut(5) {
        out.is_out_lap = true;
    }
    if let Some(gone) = rows.last_mut() {
        gone.status = Some(DriverStatus::DidNotFinish);
        gone.gap_to_leader = TimingText::from("-".to_owned());
        gone.sectors = [None, None, None];
    }
    TimingBoard {
        session_label: label.to_owned(),
        gp_name: "Dutch GP".to_owned(),
        country_flag_url: flag_url("Netherlands"),
        current_lap: Some(12),
        total_laps: Some(72),
        rows,
    }
}

/// A running session, as each board draws it.
#[must_use]
pub fn live(bucket: SizeBucket, label: &str) -> LiveViewData {
    LiveViewData {
        bucket,
        board: LiveBoard::from_board(timing_board(label)),
    }
}

#[must_use]
pub fn live_lapped(bucket: SizeBucket) -> LiveViewData {
    let mut board = timing_board("Race");
    for row in board.rows.iter_mut().skip(3) {
        row.gap_to_leader = TimingText::from("+1 LAP".to_owned());
        row.interval = TimingText::from("+1 LAP".to_owned());
    }
    LiveViewData {
        bucket,
        board: LiveBoard::from_board(board),
    }
}

#[must_use]
pub fn live_unranked(bucket: SizeBucket) -> LiveViewData {
    let mut board = timing_board("Race");
    board.current_lap = None;
    board.total_laps = None;
    for row in &mut board.rows {
        row.position = None;
    }
    LiveViewData {
        bucket,
        board: LiveBoard::from_board(board),
    }
}

/// The quiet week: no session running, so the board says so.
#[must_use]
pub fn live_idle(bucket: SizeBucket) -> LiveViewData {
    LiveViewData {
        bucket,
        board: LiveBoard::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::{DRIVERS, GRID, flag_asset, team_logo_key};

    /// An unmapped country seeds no flag at all, and a screen missing one
    /// draws the empty state rather than failing — so the two tables here
    /// are the only thing standing between a renamed country and a board
    /// that quietly stops showing flags.
    #[test]
    fn every_country_the_fixtures_name_has_a_flag() {
        for (_, country, ..) in &GRID {
            assert!(flag_asset(country).is_some(), "no flag for `{country}`");
        }
        for driver in &DRIVERS {
            let country = driver.nationality;
            assert!(flag_asset(country).is_some(), "no flag for `{country}`");
        }
    }

    #[test]
    fn a_team_is_found_however_its_sponsors_dress_the_name() {
        for name in [
            "Ferrari",
            "Scuderia Ferrari",
            "Oracle Red Bull Racing",
            "Visa Cash App Racing Bulls",
            "Mercedes-AMG Petronas",
            "Atlassian Williams",
        ] {
            assert!(team_logo_key(name).is_some(), "no artwork for `{name}`");
        }
    }

    /// Asserted as identities rather than asset names:
    /// the marks are numbered arbitrarily, so only who-matches-whom is meaningful.
    #[test]
    fn an_engine_supplier_in_the_name_does_not_win_over_the_team() {
        assert_eq!(team_logo_key("Haas Ferrari"), team_logo_key("Haas"));
        assert_ne!(team_logo_key("Haas Ferrari"), team_logo_key("Ferrari"));
        assert_eq!(
            team_logo_key("Visa Cash App Racing Bulls"),
            team_logo_key("Racing Bulls")
        );
        assert_ne!(
            team_logo_key("Visa Cash App Racing Bulls"),
            team_logo_key("Red Bull Racing")
        );
    }

    /// One mark per constructor, or two teams on one screen look alike.
    #[test]
    fn no_two_teams_share_a_mark() {
        let mut seen = std::collections::HashSet::new();
        for (key, asset) in super::LOGO_KEYS {
            assert!(seen.insert(asset), "{key} shares {asset} with another team");
        }
    }

    #[test]
    fn a_team_no_scene_has_artwork_for_falls_back() {
        assert!(team_logo_key("Stake F1 Kick Sauber").is_none());
    }
}
