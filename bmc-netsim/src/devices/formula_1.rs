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

//! Nexus Formula 1 profile — a cloud API like [`super::braiins_pool`]:
//! nothing is announced, the resource is reached by its port through the
//! testbed's `--rewrite-url`.
//!
//! Serves the resource subset the `formula-1` widget reads
//! (`widgets-wasm/formula-1/src/api.rs`), in the Nexus envelope.
//! A live session is a pure function of scenario time and device seed:
//! every driver runs laps whose times derive from both,
//! and positions, gaps, sectors and pit stops all fall out of those
//! sums — so overtakes happen, and every rerun replays them identically.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{Body, EndpointSpec, RequestCtx, ResourceSpec};
use crate::http_status::HttpStatus;

/// One lap of the simulated circuit, seconds.
const LAP_SECS: f64 = 72.0;
/// The race distance; a session past it wraps back to its start,
/// so a long-running sim stays live instead of going quiet.
const TOTAL_LAPS: u32 = 72;
/// Which lap retires the one retiring driver.
const RETIREMENT_LAP: u32 = 31;
/// Laps between pit stops; the seed staggers each driver around them.
const STINT_LAPS: u32 = 24;

/// One simulated driver. The slug is the widget's `driver` param value
/// (`widgets-wasm/formula-1/manifest.json`); `team` indexes [`TEAMS`].
struct SimDriver {
    code: &'static str,
    slug: &'static str,
    name: &'static str,
    number: u8,
    team: usize,
    country: &'static str,
    country_name: &'static str,
    points: u16,
    gp_wins: u16,
    world_titles: u16,
}

/// The season's grid, in championship order.
const GRID: [SimDriver; 22] = [
    SimDriver {
        code: "ANT",
        slug: "antonelli",
        name: "Kimi Antonelli",
        number: 12,
        team: 0,
        country: "ITA",
        country_name: "Italy",
        points: 219,
        gp_wins: 6,
        world_titles: 0,
    },
    SimDriver {
        code: "HAM",
        slug: "hamilton",
        name: "Lewis Hamilton",
        number: 44,
        team: 1,
        country: "GBR",
        country_name: "Great Britain",
        points: 169,
        gp_wins: 106,
        world_titles: 7,
    },
    SimDriver {
        code: "RUS",
        slug: "russell",
        name: "George Russell",
        number: 63,
        team: 0,
        country: "GBR",
        country_name: "Great Britain",
        points: 160,
        gp_wins: 7,
        world_titles: 0,
    },
    SimDriver {
        code: "LEC",
        slug: "leclerc",
        name: "Charles Leclerc",
        number: 16,
        team: 1,
        country: "MON",
        country_name: "Monaco",
        points: 138,
        gp_wins: 9,
        world_titles: 0,
    },
    SimDriver {
        code: "NOR",
        slug: "norris",
        name: "Lando Norris",
        number: 4,
        team: 2,
        country: "GBR",
        country_name: "Great Britain",
        points: 128,
        gp_wins: 12,
        world_titles: 1,
    },
    SimDriver {
        code: "VER",
        slug: "max_verstappen",
        name: "Max Verstappen",
        number: 1,
        team: 3,
        country: "NED",
        country_name: "Netherlands",
        points: 109,
        gp_wins: 71,
        world_titles: 4,
    },
    SimDriver {
        code: "PIA",
        slug: "piastri",
        name: "Oscar Piastri",
        number: 81,
        team: 2,
        country: "AUS",
        country_name: "Australia",
        points: 92,
        gp_wins: 9,
        world_titles: 0,
    },
    SimDriver {
        code: "HAD",
        slug: "hadjar",
        name: "Isack Hadjar",
        number: 6,
        team: 3,
        country: "FRA",
        country_name: "France",
        points: 68,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "LAW",
        slug: "lawson",
        name: "Liam Lawson",
        number: 30,
        team: 4,
        country: "AUS",
        country_name: "Australia",
        points: 43,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "GAS",
        slug: "gasly",
        name: "Pierre Gasly",
        number: 10,
        team: 5,
        country: "FRA",
        country_name: "France",
        points: 42,
        gp_wins: 1,
        world_titles: 0,
    },
    SimDriver {
        code: "LIN",
        slug: "arvid_lindblad",
        name: "Arvid Lindblad",
        number: 41,
        team: 4,
        country: "GBR",
        country_name: "Great Britain",
        points: 23,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "COL",
        slug: "colapinto",
        name: "Franco Colapinto",
        number: 43,
        team: 5,
        country: "ARG",
        country_name: "Argentina",
        points: 19,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "BEA",
        slug: "bearman",
        name: "Oliver Bearman",
        number: 87,
        team: 6,
        country: "GBR",
        country_name: "Great Britain",
        points: 18,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "BOR",
        slug: "bortoleto",
        name: "Gabriel Bortoleto",
        number: 5,
        team: 7,
        country: "BRA",
        country_name: "Brazil",
        points: 10,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "SAI",
        slug: "sainz",
        name: "Carlos Sainz",
        number: 55,
        team: 8,
        country: "ESP",
        country_name: "Spain",
        points: 6,
        gp_wins: 4,
        world_titles: 0,
    },
    SimDriver {
        code: "ALB",
        slug: "albon",
        name: "Alexander Albon",
        number: 23,
        team: 8,
        country: "THA",
        country_name: "Thailand",
        points: 5,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "OCO",
        slug: "ocon",
        name: "Esteban Ocon",
        number: 31,
        team: 6,
        country: "FRA",
        country_name: "France",
        points: 3,
        gp_wins: 1,
        world_titles: 0,
    },
    SimDriver {
        code: "HUL",
        slug: "hulkenberg",
        name: "Nico Hulkenberg",
        number: 27,
        team: 7,
        country: "GER",
        country_name: "Germany",
        points: 2,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "ALO",
        slug: "alonso",
        name: "Fernando Alonso",
        number: 14,
        team: 9,
        country: "ESP",
        country_name: "Spain",
        points: 1,
        gp_wins: 32,
        world_titles: 2,
    },
    SimDriver {
        code: "STR",
        slug: "stroll",
        name: "Lance Stroll",
        number: 18,
        team: 9,
        country: "CAN",
        country_name: "Canada",
        points: 0,
        gp_wins: 0,
        world_titles: 0,
    },
    SimDriver {
        code: "BOT",
        slug: "bottas",
        name: "Valtteri Bottas",
        number: 77,
        team: 10,
        country: "FIN",
        country_name: "Finland",
        points: 0,
        gp_wins: 10,
        world_titles: 0,
    },
    SimDriver {
        code: "PER",
        slug: "perez",
        name: "Sergio Perez",
        number: 11,
        team: 10,
        country: "MEX",
        country_name: "Mexico",
        points: 0,
        gp_wins: 6,
        world_titles: 0,
    },
];

/// Constructors by [`GRID`]'s team index: name and livery colour.
const TEAMS: [(&str, &str); 11] = [
    ("Mercedes", "00D7B6"),
    ("Ferrari", "ED1131"),
    ("McLaren", "F47600"),
    ("Red Bull Racing", "0600EF"),
    ("Racing Bulls", "2B4562"),
    ("Alpine", "0090FF"),
    ("Haas", "B6BABD"),
    ("Audi", "900000"),
    ("Williams", "005AFF"),
    ("Aston Martin", "006F62"),
    ("Cadillac", "909090"),
];

/// The rookies whose race engineer the real upstream sends as the
/// literal text `null` — the quirk the widget's parser has to absorb.
const NULL_ENGINEER: [&str; 4] = ["LIN", "COL", "BOT", "PER"];

/// Which session the live boards report as running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Session {
    /// No session: every live resource answers `{"live": false}`.
    Idle,
    Race,
    Quali,
    Practice,
}

/// Tunables for a simulated Nexus Formula 1 deployment.
// Strict, unlike persisted state: a blueprint is hand-authored, so a
// mistyped key is a fault that would silently never fire rather than a
// forward-compatible field to skip over.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "Formula1Params")]
pub struct Params {
    /// The running session; `idle` scripts a quiet week.
    pub session: Session,
    /// `false` scripts the off-season: no next race, empty standings.
    pub season_underway: bool,
    /// Added to every envelope's `cache_age_secs`;
    /// larger than the resource's `ttl_secs` scripts stale-while-revalidate.
    pub stale_secs: u32,
    /// Session times as RFC 3339 with offset,
    /// previewing the format the real deployment has agreed to move to.
    /// `false` replays today's naive wall-clock strings,
    /// which the widget refuses to place.
    pub rfc3339_sessions: bool,
    /// HTTP status every endpoint returns.
    pub status: HttpStatus,
    /// Status of the careers-derived resources (the statistics table,
    /// the driver index, the per-driver cards) —
    /// the trio the real deployment 503s while a fresh instance warms up.
    pub careers_status: HttpStatus,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            session: Session::Idle,
            season_underway: true,
            stale_secs: 0,
            rfc3339_sessions: false,
            status: HttpStatus::OK,
            careers_status: HttpStatus::OK,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let mut endpoints = vec![
            self.endpoint("standings", self.status, standings(self.season_underway)),
            self.endpoint("driver-stats", self.careers_status, driver_stats()),
            self.endpoint("drivers", self.careers_status, drivers_index()),
            self.endpoint("teams", self.status, teams()),
            self.endpoint(
                "next-race",
                self.status,
                next_race(self.season_underway, self.rfc3339_sessions),
            ),
        ];
        for (index, driver) in GRID.iter().enumerate() {
            endpoints.push(self.endpoint(
                &format!("driver/{}", driver.slug),
                self.careers_status,
                driver_card(index),
            ));
        }
        for (resource, session) in [
            ("live-race", Session::Race),
            ("live-quali", Session::Quali),
            ("live-practice", Session::Practice),
        ] {
            let running = self.session == session;
            let stale = self.stale_secs;
            endpoints.push(EndpointSpec {
                method: "GET".to_owned(),
                path: format!("/api/v1/data/formula-1/{resource}"),
                body: Body::respond(move |ctx| {
                    let data = if running {
                        board(session, ctx)
                    } else {
                        json!({ "live": false })
                    };
                    envelope(resource, &data, 3, stale)
                }),
                status: self.status,
            });
        }
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: None,
            endpoints,
            sampler: None,
        }
    }

    /// A static resource: its payload never varies within a scenario,
    /// only the envelope's freshness does.
    fn endpoint(&self, resource: &str, status: HttpStatus, data: Json) -> EndpointSpec {
        let name = resource.to_owned();
        let stale = self.stale_secs;
        EndpointSpec {
            method: "GET".to_owned(),
            path: format!("/api/v1/data/formula-1/{resource}"),
            body: Body::respond(move |_| envelope(&name, &data, 60, stale)),
            status,
        }
    }
}

/// The Nexus reply envelope every resource answers in.
fn envelope(resource: &str, data: &Json, ttl_secs: u32, stale_secs: u32) -> Json {
    json!({
        "resource": format!("formula-1/{resource}"),
        "data": data,
        "cache_age_secs": stale_secs,
        "ttl_secs": ttl_secs,
    })
}

fn image_url(kind: &str, key: &str) -> String {
    format!("https://cdn.nexus.sim/{kind}/{}.png", key.to_lowercase())
}

/// A deterministic unit-interval sample from the seed and salts —
/// splitmix64 over their combination.
fn noise(seed: u64, salts: &[u64]) -> f64 {
    let mut state = seed;
    for salt in salts {
        state = state
            .wrapping_add(0x9e37_79b9_7f4a_7c15)
            .wrapping_add(*salt);
        state = (state ^ (state >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        state = (state ^ (state >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        state ^= state >> 31;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "53 mantissa bits are plenty for simulator noise"
    )]
    let unit = (state >> 11) as f64 / (1_u64 << 53) as f64;
    unit
}

fn standings(season_underway: bool) -> Json {
    if !season_underway {
        return json!([]);
    }
    let rows: Vec<Json> = GRID
        .iter()
        .enumerate()
        .map(|(index, driver)| {
            let (team_name, color) = TEAMS[driver.team];
            json!({
                "position": index + 1,
                "driver_name": driver.name,
                "driver_code": driver.code,
                "team_name": team_name,
                "team_logo_url": image_url("logo", team_name),
                "team_color": color,
                "country_code": driver.country,
                "country_flag_url": image_url("flag", driver.country),
                "points": driver.points,
                "headshot_url": image_url("headshot", driver.code),
            })
        })
        .collect();
    Json::Array(rows)
}

/// One row of the statistics table, shared with the per-driver cards.
fn driver_row(index: usize) -> Json {
    let driver = &GRID[index];
    let (team_name, color) = TEAMS[driver.team];
    let engineer = if NULL_ENGINEER.contains(&driver.code) {
        "null".to_owned()
    } else {
        format!("Engineer of {}", driver.name)
    };
    json!({
        "name": driver.name,
        "number": driver.number,
        "headshot_url": image_url("headshot", driver.code),
        "team": team_name,
        "team_color": color,
        "ranking": index + 1,
        "points": driver.points,
        "nationality": driver.country_name,
        "nationality_flag_url": image_url("flag", driver.country),
        "gp_wins": driver.gp_wins,
        "world_titles": driver.world_titles,
        // Measurements the sim invents: varied, bounded, index-stable.
        "age": 19 + (index * 7) % 27,
        "weight_kg": 63 + (index * 5) % 16,
        "height_cm": 167 + (index * 3) % 20,
        "race_engineer": engineer,
        "debut_year": 2001 + (index * 3) % 25,
    })
}

fn driver_stats() -> Json {
    Json::Array((0..GRID.len()).map(driver_row).collect())
}

fn driver_card(index: usize) -> Json {
    driver_row(index)
}

fn drivers_index() -> Json {
    let entries: Vec<Json> = GRID
        .iter()
        .map(|driver| json!({ "jolpica_id": driver.slug, "display_name": driver.name }))
        .collect();
    Json::Array(entries)
}

fn teams() -> Json {
    let entries: Vec<Json> = TEAMS
        .iter()
        .map(|(name, color)| {
            json!({
                "name": name,
                "color": color,
                "logo_url": image_url("logo", name),
            })
        })
        .collect();
    Json::Array(entries)
}

/// The upcoming weekend: dates on the calendar relative to the
/// wall clock, session times in the venue's zone.
fn next_race(season_underway: bool, rfc3339: bool) -> Json {
    if !season_underway {
        return json!({});
    }
    let now = chrono::Utc::now();
    // The race lands on the upcoming Sunday, the weekend opening two
    // days before it.
    let days_to_sunday = i64::from(7 - chrono::Datelike::weekday(&now).number_from_monday()).max(1);
    let sunday = now.date_naive() + chrono::Days::new(u64::try_from(days_to_sunday).unwrap_or(1));
    let friday = sunday - chrono::Days::new(2);
    let saturday = sunday - chrono::Days::new(1);
    let at = |day: chrono::NaiveDate, clock: &str| {
        if rfc3339 {
            format!("{day}T{clock}:00+02:00")
        } else {
            format!("{day} {clock}:00")
        }
    };
    json!({
        "gp_name": "Dutch GP",
        "country_name": "Netherlands",
        "country_flag_url": image_url("flag", "NED"),
        "venue_timezone": "Europe/Amsterdam",
        "date_start": friday.to_string(),
        "date_end": sunday.to_string(),
        "circuit_name": "Circuit Zandvoort",
        "circuit_image_url": image_url("circuit", "zandvoort"),
        "track_length_km": 4.259,
        "total_laps": TOTAL_LAPS,
        "race_distance_km": 306.648,
        "drs_zones": 2,
        "tire_compounds": "C1, C2, C3",
        "sessions": [
            { "name": "Practice 1", "date_start": at(friday, "10:30") },
            { "name": "Practice 2", "date_start": at(friday, "14:00") },
            { "name": "Practice 3", "date_start": at(saturday, "10:30") },
            { "name": "Qualifying", "date_start": at(saturday, "14:00") },
            { "name": "Race", "date_start": at(sunday, "13:00") },
        ],
    })
}

/// A driver's `lap`-th lap time: their pace band by championship rank,
/// plus per-lap noise. Front-runners are faster on average,
/// but the noise overlaps the bands, so positions trade within reach.
fn lap_time(driver: usize, lap: u32, seed: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a grid index is tiny; exact in f64"
    )]
    let band = driver as f64 * 0.028;
    // Small against the bands' spread: the walk lets neighbours trade
    // places over a stint without shuffling the field into noise.
    let jitter = noise(seed, &[driver as u64, u64::from(lap)]) * 0.35;
    let pit = if pits(driver, lap, seed) { 22.0 } else { 0.0 };
    LAP_SECS + band + jitter + pit
}

/// Salt separating pit-stagger noise from lap-time noise.
const PIT_SALT: u64 = 0x9174;
/// Salt picking the retiring driver.
const RETIRE_SALT: u64 = 0xdead;

/// Whether `lap` is a pit lap: each stint boundary,
/// staggered per driver by the seed so the field does not pit in one lap.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a stagger of at most seven laps"
)]
fn pits(driver: usize, lap: u32, seed: u64) -> bool {
    if lap < 2 {
        return false;
    }
    let stagger = (noise(seed, &[driver as u64, PIT_SALT]) * 7.0) as u32;
    lap % STINT_LAPS == (2 + stagger) % STINT_LAPS
}

/// Which driver's race ends at [`RETIREMENT_LAP`].
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "an index into the 22-driver grid"
)]
fn retiree(seed: u64) -> usize {
    (noise(seed, &[RETIRE_SALT]) * GRID.len() as f64) as usize
}

/// A timing line: mm:ss.mmm over a minute, ss.mmm under it.
fn clock(secs: f64) -> String {
    if secs >= 60.0 {
        let minutes = (secs / 60.0).floor();
        format!("{}:{:06.3}", minutes, secs - minutes * 60.0)
    } else {
        format!("{secs:.3}")
    }
}

fn gap(secs: f64) -> String {
    format!("+{secs:.3}")
}

/// One driver's race so far: total time, last lap, best lap, laps done.
struct Progress {
    driver: usize,
    total: f64,
    last: f64,
    best: f64,
    laps: u32,
    retired: bool,
}

/// The whole field's state at scenario time `t_s`, running order first.
fn field_at(t_s: f64, seed: u64) -> (Vec<Progress>, u32) {
    // The session wraps past the race distance, so a sim left running
    // overnight is still mid-race whenever the testbed connects.
    let race_secs = f64::from(TOTAL_LAPS) * LAP_SECS;
    let into = t_s.rem_euclid(race_secs).max(LAP_SECS);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a lap count bounded by the race distance"
    )]
    let lead_lap = (into / LAP_SECS) as u32;
    let out = retiree(seed);
    let mut field: Vec<Progress> = (0..GRID.len())
        .map(|driver| {
            let retired = driver == out && lead_lap >= RETIREMENT_LAP;
            let laps = if retired { RETIREMENT_LAP } else { lead_lap };
            let times: Vec<f64> = (1..=laps).map(|lap| lap_time(driver, lap, seed)).collect();
            Progress {
                driver,
                total: times.iter().sum(),
                last: times.last().copied().unwrap_or(0.0),
                best: times.iter().copied().fold(f64::INFINITY, f64::min),
                laps,
                retired,
            }
        })
        .collect();
    // Retirees sink to the tail; everyone else runs in total-time order.
    field.sort_by(|a, b| a.retired.cmp(&b.retired).then(a.total.total_cmp(&b.total)));
    (field, lead_lap)
}

/// The positions one lap earlier, for the per-row position change.
fn order_at(lap: u32, seed: u64) -> Vec<usize> {
    let mut totals: Vec<(usize, f64)> = (0..GRID.len())
        .map(|driver| {
            let total: f64 = (1..=lap).map(|l| lap_time(driver, l, seed)).sum();
            (driver, total)
        })
        .collect();
    totals.sort_by(|a, b| a.1.total_cmp(&b.1));
    totals.into_iter().map(|(driver, _)| driver).collect()
}

fn board(session: Session, ctx: &RequestCtx) -> Json {
    let (field, lead_lap) = field_at(ctx.t_s, ctx.seed);
    let previous = order_at(lead_lap.saturating_sub(1), ctx.seed);
    let session_best = field
        .iter()
        .filter(|p| !p.retired)
        .map(|p| p.best)
        .fold(f64::INFINITY, f64::min);
    let leader_total = field.first().map_or(0.0, |p| p.total);
    let mut prior_total = leader_total;
    let entries: Vec<Json> = field
        .iter()
        .enumerate()
        .map(|(position, run)| {
            let driver = &GRID[run.driver];
            let (team_name, color) = TEAMS[driver.team];
            let was = previous
                .iter()
                .position(|d| *d == run.driver)
                .unwrap_or(position);
            let stint_age = (2..=run.laps)
                .rev()
                .take_while(|lap| !pits(run.driver, *lap, ctx.seed))
                .count();
            let stint = usize::try_from(run.laps.div_euclid(STINT_LAPS)).unwrap_or(0);
            let compound = ["M", "H", "S"][stint % 3];
            let in_pit = !run.retired && pits(run.driver, run.laps, ctx.seed);
            let (gap_text, interval) = if run.retired {
                ("-".to_owned(), "-".to_owned())
            } else if position == 0 {
                ("LEADER".to_owned(), "-".to_owned())
            } else if in_pit {
                ("PIT".to_owned(), "PIT".to_owned())
            } else {
                (gap(run.total - leader_total), gap(run.total - prior_total))
            };
            prior_total = run.total;
            let sectors: Json = if run.retired {
                json!({})
            } else {
                let splits = [0.31, 0.36, 0.33];
                let color_of = |which: usize| {
                    if (run.last - session_best).abs() < 0.05 {
                        "purple"
                    } else if which == run.laps as usize % 3 {
                        "green"
                    } else {
                        "white"
                    }
                };
                json!({
                    "sector1": { "time": run.last * splits[0], "color": color_of(0) },
                    "sector2": { "time": run.last * splits[1], "color": color_of(1) },
                    "sector3": { "time": run.last * splits[2], "color": color_of(2) },
                })
            };
            let mut entry = json!({
                "position": position + 1,
                "driver_code": driver.code,
                "driver_name": driver.name,
                "team_logo_url": image_url("logo", team_name),
                "team_color": color,
                "position_change": i64::try_from(was).unwrap_or(0) - i64::try_from(position).unwrap_or(0),
                "gap_to_leader": gap_text,
                "interval": interval,
                "last_lap_time": if run.retired { "-".to_owned() } else { clock(run.last) },
                "best_lap_time": clock(run.best),
                "total_time": clock(run.total),
                "tire_compound": compound,
                "tire_age": stint_age,
                "in_pit": in_pit,
                "is_out_lap": false,
                "fastest_lap": !run.retired && (run.best - session_best).abs() < f64::EPSILON,
                "status": if run.retired { "DNF" } else { "RUN" },
            });
            if let (Json::Object(map), Json::Object(sector_map)) = (&mut entry, sectors) {
                map.extend(sector_map);
            }
            entry
        })
        .collect();
    let label = match session {
        Session::Race => "Race",
        Session::Quali => "Qualifying",
        Session::Practice => "Practice",
        Session::Idle => unreachable!("BUG: an idle session never builds a board"),
    };
    json!({
        "live": true,
        "session_label": label,
        "gp_name": "Dutch GP",
        "country_flag_url": image_url("flag", "NED"),
        "current_lap": lead_lap,
        "total_laps": TOTAL_LAPS,
        "entries": entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(t_s: f64) -> RequestCtx {
        RequestCtx {
            query: std::collections::BTreeMap::new(),
            t_s,
            seed: 0xF1,
        }
    }

    #[test]
    fn every_widget_resource_is_served() {
        let spec = Params::default().resource("f1", 20_100);
        for resource in [
            "standings",
            "driver-stats",
            "drivers",
            "teams",
            "next-race",
            "driver/max_verstappen",
            "driver/antonelli",
            "live-race",
            "live-quali",
            "live-practice",
        ] {
            let path = format!("/api/v1/data/formula-1/{resource}");
            assert!(
                spec.endpoints.iter().any(|e| e.path == path),
                "missing endpoint {path}"
            );
        }
    }

    #[test]
    fn an_idle_week_answers_live_false_in_the_envelope() {
        let spec = Params::default().resource("f1", 20_100);
        let live = spec
            .endpoints
            .iter()
            .find(|e| e.path.ends_with("live-race"))
            .expect("BUG: live-race endpoint");
        let Body::Respond(responder) = &live.body else {
            panic!("BUG: live boards answer per request");
        };
        let reply = responder(&ctx(100.0));
        assert_eq!(reply["data"]["live"], false);
        assert_eq!(reply["ttl_secs"], 3);
    }

    #[test]
    fn a_race_replays_identically_and_positions_close_up() {
        let one = board(Session::Race, &ctx(1_800.0));
        let two = board(Session::Race, &ctx(1_800.0));
        assert_eq!(one, two, "same time and seed must replay the same board");
        let entries = one["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), GRID.len());
        assert_eq!(entries[0]["gap_to_leader"], "LEADER");
        let order: Vec<&str> = entries
            .iter()
            .map(|entry| entry["driver_code"].as_str().expect("code"))
            .collect();
        let championship: Vec<&str> = GRID.iter().map(|driver| driver.code).collect();
        assert_ne!(
            order, championship,
            "half an hour of pace noise must have traded some position \
             (fails only if the model collapsed to championship order)"
        );
    }

    #[test]
    fn the_retiree_sinks_to_the_tail_as_a_dnf() {
        let secs_past_retirement = f64::from(RETIREMENT_LAP + 5) * LAP_SECS;
        let reply = board(Session::Race, &ctx(secs_past_retirement));
        let entries = reply["entries"].as_array().expect("entries");
        let last = entries.last().expect("a tail");
        assert_eq!(last["status"], "DNF");
        assert_eq!(last["gap_to_leader"], "-");
    }

    #[test]
    fn the_rookie_quirk_ships_the_literal_null_text() {
        let stats = driver_stats();
        let lindblad = stats
            .as_array()
            .expect("rows")
            .iter()
            .find(|row| row["name"] == "Arvid Lindblad")
            .expect("Lindblad row")
            .clone();
        assert_eq!(lindblad["race_engineer"], "null");
    }

    #[test]
    fn the_off_season_empties_the_calendar_and_the_table() {
        assert_eq!(standings(false), serde_json::json!([]));
        assert_eq!(next_race(false, false), serde_json::json!({}));
    }

    #[test]
    fn session_times_are_naive_until_the_upstream_fixes_them() {
        let race = next_race(true, false);
        let start = race["sessions"][0]["date_start"].as_str().expect("start");
        assert!(
            !start.contains('T') && !start.contains('+'),
            "today's deployment sends naive wall clock: {start}"
        );
        let fixed = next_race(true, true);
        let start = fixed["sessions"][0]["date_start"].as_str().expect("start");
        assert!(
            start.contains('+'),
            "the preview carries an offset: {start}"
        );
    }
}
