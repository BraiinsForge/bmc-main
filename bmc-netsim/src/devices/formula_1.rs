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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{
    EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseData, ResponseSpec,
};
use crate::http_status::HttpStatus;

/// One lap of the simulated circuit, seconds.
const LAP_SECS: f64 = 72.0;
/// The race distance; a session past it wraps back to its start,
/// so a long-running sim stays live instead of going quiet.
const TOTAL_LAPS: u32 = 72;
/// Which lap retires the one retiring driver.
const RETIREMENT_LAP: u32 = 31;
/// Which lap excludes the one disqualified driver, late enough that a
/// board can show a retirement without one.
const DISQUALIFICATION_LAP: u32 = 40;
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
        country: "NZL",
        country_name: "New Zealand",
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

/// The slug names both the logo URL and the fixture behind it, so it
/// follows the widget's own file names rather than anything derivable
/// from `name`.
struct Team {
    name: &'static str,
    livery: &'static str,
    slug: &'static str,
}

/// The season the constructors snapshot names beside its rows.
const SEASON_ID: usize = 26_733;

/// Upstream numbers teams in the high six figures and the drivers index
/// joins to those ids; any stable distinct run does here.
const TEAM_ID_BASE: usize = 276_180;

/// Constructors by [`GRID`]'s team index.
const TEAMS: [Team; 11] = [
    Team {
        name: "Mercedes",
        livery: "00D7B6",
        slug: "mercedes",
    },
    Team {
        name: "Ferrari",
        livery: "ED1131",
        slug: "ferrari",
    },
    Team {
        name: "McLaren",
        livery: "F47600",
        slug: "mclaren",
    },
    Team {
        name: "Red Bull Racing",
        livery: "0600EF",
        slug: "red-bull",
    },
    Team {
        name: "Racing Bulls",
        livery: "2B4562",
        slug: "racing-bulls",
    },
    Team {
        name: "Alpine",
        livery: "0090FF",
        slug: "alpine",
    },
    Team {
        name: "Haas",
        livery: "B6BABD",
        slug: "haas",
    },
    Team {
        name: "Audi",
        livery: "900000",
        slug: "audi",
    },
    Team {
        name: "Williams",
        livery: "005AFF",
        slug: "williams",
    },
    Team {
        name: "Aston Martin",
        livery: "006F62",
        slug: "aston-martin",
    },
    Team {
        name: "Cadillac",
        livery: "909090",
        slug: "cadillac",
    },
];

/// The rookies whose race engineer the real upstream sends as the
/// literal text `null` — the quirk the widget's parser has to absorb.
const NULL_ENGINEER: [&str; 4] = ["LIN", "COL", "BOT", "PER"];

/// Race engineers as the upstream gives them: a person's name.
/// A stand-in phrase would size the row to something no payload sends.
const ENGINEERS: [&str; 9] = [
    "Bryan Bozzi",
    "Gianpiero Lambiase",
    "Will Joseph",
    "Tom Stallard",
    "Peter Bonnington",
    "Riccardo Musconi",
    "Marcus Dudley",
    "Michael Italiano",
    "Laura Mueller",
];

/// The circuit the scripted next race is held at, keying its image.
const RACE_CIRCUIT: &str = "zandvoort";

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

impl Session {
    /// Whether the clock judges the session rather than distance covered.
    /// That decides both how the field is classified and whose laps still
    /// count once they have stopped.
    const fn is_timed(self) -> bool {
        match self {
            Self::Quali | Self::Practice => true,
            Self::Race | Self::Idle => false,
        }
    }
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
    /// The lap the session is already on when the simulator starts.
    ///
    /// Gaps, tyre ages, pit stops and retirements accumulate over a race,
    /// so a session opened at its first lap shows a widget almost nothing.
    ///
    /// A scenario names the lap whose state it exists to show. Note that a
    /// race board seats only its leading rows, so whatever sorts to the tail
    /// — a retirement, an exclusion, a car a lap down — is visible on the
    /// full-field board alone.
    pub start_lap: u32,
    /// `false` scripts the off-season: no next race, empty standings.
    pub season_underway: bool,
    /// Whether the weekend runs a sprint, which replaces two practices
    /// with a sprint and a qualifying of its own.
    pub sprint: bool,
    /// Added to every envelope's `cache_age_secs`;
    /// larger than the resource's `ttl_secs` scripts stale-while-revalidate.
    pub stale_secs: u32,
    /// HTTP status every endpoint returns.
    pub status: HttpStatus,
    /// Status of the careers-derived resources (the statistics table,
    /// the driver index, the per-driver cards) — the trio the real deployment
    /// 503s while a fresh instance warms up.
    pub careers_status: HttpStatus,
    /// Images the payloads point at, in a directory relative to the blueprint.
    /// Laid out as the widget's own artwork is: `headshots/<NN>.gif`
    /// `logos/<NN>.png` and `flags/<country>.png`, beside `circuit.png`.
    /// Omitted, nothing is served and every image URL 404s onto the widget's placeholders.
    pub image_dir: Option<PathBuf>,
}

impl Params {
    /// A blueprint is committed beside the images it names, so its paths
    /// are relative to itself, not to wherever the simulator runs.
    pub fn resolve_paths(&mut self, base: &Path) {
        if let Some(dir) = self.image_dir.take() {
            self.image_dir = Some(base.join(dir));
        }
    }
}

impl Default for Params {
    fn default() -> Self {
        Self {
            session: Session::Idle,
            start_lap: 0,
            season_underway: true,
            sprint: false,
            stale_secs: 0,
            status: HttpStatus::OK,
            careers_status: HttpStatus::OK,
            image_dir: None,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let home = format!("127.0.0.1:{port}");
        let mut endpoints = vec![
            self.endpoint(
                &home,
                "standings",
                self.status,
                standings(self.season_underway),
            ),
            self.endpoint(&home, "driver-stats", self.careers_status, driver_stats()),
            self.endpoint(&home, "drivers", self.careers_status, drivers_index()),
            self.endpoint(&home, "teams", self.status, teams()),
            self.endpoint(
                &home,
                "next-race",
                self.status,
                next_race(self.season_underway, self.sprint),
            ),
        ];
        for (index, driver) in GRID.iter().enumerate() {
            endpoints.push(self.endpoint(
                &home,
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
            let start_lap = self.start_lap;
            let status = self.status;
            let home = home.clone();
            endpoints.push(EndpointSpec {
                method: "GET".to_owned(),
                path: format!("/api/v1/data/formula-1/{resource}"),
                response: ResponseSpec::computed(move |ctx| {
                    let data = if running {
                        board(session, ctx, start_lap)
                    } else {
                        json!({ "live": false })
                    };
                    Response::new(
                        status,
                        envelope(resource, &data, 3, stale, host_of(ctx, &home)),
                    )
                }),
            });
        }
        endpoints.extend(self.image_endpoints());
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: None,
            endpoints,
            sampler: None,
        }
    }

    /// A resource whose payload never varies within a scenario.
    /// It is computed only so the envelope can name the host that reached us.
    fn endpoint(&self, home: &str, resource: &str, status: HttpStatus, data: Json) -> EndpointSpec {
        let name = resource.to_owned();
        let stale = self.stale_secs;
        let home = home.to_owned();
        EndpointSpec {
            method: "GET".to_owned(),
            path: format!("/api/v1/data/formula-1/{resource}"),
            response: ResponseSpec::computed(move |ctx| {
                Response::new(
                    status,
                    envelope(&name, &data, 60, stale, host_of(ctx, &home)),
                )
            }),
        }
    }

    /// An unreadable fixture is left unserved rather than failing the run:
    /// the widget draws its placeholder,
    /// and an incomplete image set is a state worth being able to script.
    fn image_endpoints(&self) -> Vec<EndpointSpec> {
        let Some(dir) = self.image_dir.as_ref() else {
            return Vec::new();
        };
        let mut loaded: BTreeMap<PathBuf, Option<Arc<[u8]>>> = BTreeMap::new();
        let mut endpoints = Vec::new();
        for ImageFixture { kind, key, file } in image_fixtures() {
            let path = dir.join(&file);
            let bytes = loaded
                .entry(path.clone())
                .or_insert_with(|| match std::fs::read(&path) {
                    Ok(bytes) => Some(Arc::from(bytes.into_boxed_slice())),
                    Err(err) => {
                        tracing::warn!(path = %path.display(), error = %err, "formula-1 image unreadable");
                        None
                    }
                });
            let Some(data) = bytes.clone() else { continue };
            let ext = image_ext(kind);
            endpoints.push(EndpointSpec {
                method: "GET".to_owned(),
                path: format!("/{IMAGE_ROUTE}/{kind}/{key}.{ext}"),
                response: ResponseSpec::Static(Response::ok(ResponseData::bytes(
                    &format!("image/{ext}"),
                    data,
                ))),
            });
        }
        endpoints
    }
}

/// The Nexus reply envelope every resource answers in, its image URLs
/// finished with `host` — see [`RequestCtx::host`].
fn envelope(resource: &str, data: &Json, ttl_secs: u32, stale_secs: u32, host: &str) -> Json {
    json!({
        "resource": format!("formula-1/{resource}"),
        "data": with_host(data, host),
        "cache_age_secs": stale_secs,
        "ttl_secs": ttl_secs,
    })
}

/// Stands in for the serving address until a request supplies one.
const HOST_TOKEN: &str = "{{host}}";

const IMAGE_ROUTE: &str = "img";

/// `home` covers a client that sent no `Host`.
fn host_of<'a>(ctx: &'a RequestCtx, home: &'a str) -> &'a str {
    ctx.host.as_deref().unwrap_or(home)
}

struct ImageFixture {
    kind: &'static str,
    key: String,
    file: PathBuf,
}

/// Every image the payloads point at, from the widget's own artwork:
/// a headshot per driver, a logo per team, and a flag per nationality
/// the grid holds.
fn image_fixtures() -> Vec<ImageFixture> {
    // The headshots are numbered rather than named, so a driver takes
    // the one at their place on the grid, as the gallery does.
    let mut fixtures: Vec<ImageFixture> = GRID
        .iter()
        .enumerate()
        .map(|(index, driver)| ImageFixture {
            kind: "headshot",
            key: driver.code.to_lowercase(),
            file: PathBuf::from(format!("headshots/{:02}.gif", index + 1)),
        })
        .collect();
    let mut countries: Vec<&str> = GRID.iter().map(|driver| driver.country).collect();
    countries.sort_unstable();
    countries.dedup();
    fixtures.extend(countries.into_iter().map(|country| {
        // Filed under the nationality the payload publishes, so a grid that
        // gains one gains an unserved flag rather than a wrong one.
        let key = country.to_lowercase();
        ImageFixture {
            kind: "flag",
            file: PathBuf::from(format!("flags/{key}.png")),
            key,
        }
    }));
    // Numbered, not named for the team: the marks are invented and
    // assigned arbitrarily, the same way the headshots depict nobody.
    fixtures.extend(TEAMS.iter().enumerate().map(|(index, team)| ImageFixture {
        kind: "logo",
        key: team.slug.to_owned(),
        file: PathBuf::from(format!("logos/{:02}.png", index + 1)),
    }));
    fixtures.push(ImageFixture {
        kind: "circuit",
        key: RACE_CIRCUIT.to_lowercase(),
        file: PathBuf::from("circuit.png"),
    });
    fixtures
}

/// Headshots are published as indexed GIFs, every other image as PNG.
///
/// One rule, so the URL and the endpoint behind it cannot disagree.
fn image_ext(kind: &str) -> &'static str {
    if kind == "headshot" { "gif" } else { "png" }
}

/// The URL an image is published at, still holding [`HOST_TOKEN`].
fn image_url(kind: &str, key: &str) -> String {
    format!(
        "http://{HOST_TOKEN}/{IMAGE_ROUTE}/{kind}/{}.{}",
        key.to_lowercase(),
        image_ext(kind)
    )
}

/// Substitute `host` into every string leaf that carries the placeholder.
fn with_host(value: &Json, host: &str) -> Json {
    match value {
        Json::String(text) if text.contains(HOST_TOKEN) => {
            Json::String(text.replace(HOST_TOKEN, host))
        }
        Json::Array(items) => Json::Array(items.iter().map(|it| with_host(it, host)).collect()),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, val)| (key.clone(), with_host(val, host)))
                .collect(),
        ),
        Json::String(_) | Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
    }
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
            let team = &TEAMS[driver.team];
            json!({
                "position": index + 1,
                "driver_name": driver.name,
                "driver_code": driver.code,
                "team_name": team.name,
                "team_logo_url": image_url("logo", team.slug),
                "team_color": team.livery,
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
/// The season a driver of `age` first raced in, taken from their age
/// rather than invented beside it.
///
/// Two independent formulas over the same index put nineteen-year-olds on
/// the grid since 2001. A debut follows an eighteenth birthday, so it is
/// the years raced that vary, never the arithmetic joining the pair.
fn debut_year(index: usize, age: usize) -> i32 {
    use chrono::Datelike as _;

    let debut_age = 18 + index % 4;
    let raced = age.saturating_sub(debut_age);
    chrono::Utc::now().year() - i32::try_from(raced).unwrap_or(0)
}

fn driver_row(index: usize) -> Json {
    let driver = &GRID[index];
    let team = &TEAMS[driver.team];
    let age = 19 + (index * 7) % 27;
    let engineer = if NULL_ENGINEER.contains(&driver.code) {
        "null".to_owned()
    } else {
        ENGINEERS[index % ENGINEERS.len()].to_owned()
    };
    json!({
        // Required on every row since BDK-335, so a card joins its table
        // row on a key rather than on a car number.
        "jolpica_id": driver.slug,
        "name": driver.name,
        "number": driver.number,
        "headshot_url": image_url("headshot", driver.code),
        "team": team.name,
        "team_color": team.livery,
        "ranking": index + 1,
        "points": driver.points,
        "nationality": driver.country_name,
        "nationality_flag_url": image_url("flag", driver.country),
        "gp_wins": driver.gp_wins,
        "world_titles": driver.world_titles,
        // Measurements the sim invents: varied, bounded, index-stable.
        "age": age,
        "weight_kg": 63 + (index * 5) % 16,
        "height_cm": 167 + (index * 3) % 20,
        "race_engineer": engineer,
        "debut_year": debut_year(index, age),
    })
}

fn driver_stats() -> Json {
    Json::Array((0..GRID.len()).map(driver_row).collect())
}

/// The per-driver resource: who the driver is, without the season's
/// figures. BDK-335 added the join key here rather than copying the
/// whole statistics row in, so a card is only ever half the screen —
/// the other half arrives by joining on `jolpica_id`.
fn driver_card(index: usize) -> Json {
    let mut card = driver_row(index);
    for derived in ["ranking", "points", "gp_wins", "world_titles"] {
        card.as_object_mut()
            .expect("BUG: a driver row is an object")
            .remove(derived);
    }
    card
}

/// Every driver the index knows, the grid flagged apart from the reserves
/// a picker filters out. The sim seats a whole championship, so the flag
/// is only ever false for the two who never race.
fn drivers_index() -> Json {
    let reserve = [
        ("bearman_reserve", "Oliver Bearman (reserve)"),
        ("drugovich", "Felipe Drugovich"),
    ];
    let entries: Vec<Json> = GRID
        .iter()
        .map(|driver| {
            json!({
                "jolpica_id": driver.slug,
                "display_name": driver.name,
                "in_standings": true,
                "team_id": TEAM_ID_BASE + driver.team,
            })
        })
        // A driver who races for nobody carries no id, which is how a
        // consumer joining to the constructors snapshot finds no mark.
        .chain(reserve.into_iter().map(|(slug, name)| {
            json!({
                "jolpica_id": slug,
                "display_name": name,
                "in_standings": false,
                "team_id": Json::Null,
            })
        }))
        .collect();
    Json::Array(entries)
}

fn teams() -> Json {
    let entries: Vec<Json> = TEAMS
        .iter()
        .enumerate()
        .map(|(index, team)| {
            json!({
                "id": TEAM_ID_BASE + index,
                "name": team.name,
                "resolved_color": team.livery,
                "image_path": image_url("logo", team.slug),
            })
        })
        .collect();
    json!({ "season_id": SEASON_ID, "teams": entries })
}

/// The upcoming weekend: dates on the calendar relative to the
/// wall clock, session times in the venue's zone.
///
/// A sprint weekend runs a different set of sessions, trading two of the
/// practices for a sprint and its own qualifying. Real calendars hold a
/// handful a year, so scripting it is the only way to seat that schedule
/// on demand.
fn next_race(season_underway: bool, sprint: bool) -> Json {
    if !season_underway {
        return json!({});
    }
    let now = chrono::Utc::now();
    // The race lands on the upcoming Sunday, the weekend opening two days
    // before it. On a Sunday that is the next one rather than today, which
    // would otherwise announce a weekend already run.
    let to_sunday = match 7 - chrono::Datelike::weekday(&now).number_from_monday() {
        0 => 7,
        days => days,
    };
    let sunday = now.date_naive() + chrono::Days::new(u64::from(to_sunday));
    let friday = sunday - chrono::Days::new(2);
    let saturday = sunday - chrono::Days::new(1);
    // UTC, as the deployment sends it. Zandvoort runs two hours ahead over
    // a summer, so `venue_timezone` reads a session back at the wall time
    // its name implies — an hour out in winter, which a fixture may be.
    let named = |name: &str, day: chrono::NaiveDate, clock: &str| {
        let (hour, minute) = clock
            .split_once(':')
            .expect("BUG: a session clock is written HH:MM");
        let hour_utc = hour
            .parse::<u32>()
            .expect("BUG: a session clock's hour is a number")
            .saturating_sub(2);
        json!({
            "name": name,
            // A session's start is an instant under the same key
            // the weekend uses for a calendar date.
            "date_start": format!("{day}T{hour_utc:02}:{minute}:00Z"),
        })
    };
    json!({
        "gp_name": "Dutch GP",
        "country_name": "Netherlands",
        "country_flag_url": image_url("flag", "NED"),
        "venue_timezone": "Europe/Amsterdam",
        "date_start": friday.to_string(),
        "date_end": sunday.to_string(),
        "circuit_name": "Circuit Zandvoort",
        "circuit_image_url": image_url("circuit", RACE_CIRCUIT),
        "track_length_km": 4.259,
        "total_laps": TOTAL_LAPS,
        "race_distance_km": 306.648,
        "drs_zones": 2,
        "tire_compounds": "C1, C2, C3",
        "sessions": if sprint {
            [
                named("Practice 1", friday, "10:30"),
                named("Sprint Qualifying", friday, "14:30"),
                named("Sprint", saturday, "11:00"),
                named("Qualifying", saturday, "15:00"),
                named("Race", sunday, "13:00"),
            ]
        } else {
            [
                named("Practice 1", friday, "10:30"),
                named("Practice 2", friday, "14:00"),
                named("Practice 3", saturday, "10:30"),
                named("Qualifying", saturday, "14:00"),
                named("Race", sunday, "13:00"),
            ]
        },
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
    // A driver's form on the day, worth several places over a race and
    // independent of where they stand in the championship. Without it the
    // pace is monotonic in grid order, so the field finishes as it started
    // and every row reports a position change of zero.
    let form = (noise(seed, &[driver as u64, FORM_SALT]) - 0.5) * 0.6;
    // Small against the bands' spread: the walk lets neighbours trade
    // places over a stint without shuffling the field into noise.
    let jitter = noise(seed, &[driver as u64, u64::from(lap)]) * 0.35;
    let evolution = f64::from(lap) * EVOLUTION_SECS_PER_LAP;
    let pit = if pits(driver, lap, seed) { 22.0 } else { 0.0 };
    let damage = if driver == limper(seed) && lap >= DAMAGE_LAP {
        DAMAGE_PENALTY_SECS
    } else {
        0.0
    };
    LAP_SECS + band + form + jitter + pit + damage - evolution
}

/// How many laps `driver` has completed within `elapsed` seconds.
fn laps_by(driver: usize, elapsed: f64, seed: u64) -> u32 {
    let mut done = 0;
    let mut total = 0.0;
    while done < TOTAL_LAPS {
        total += lap_time(driver, done + 1, seed);
        if total > elapsed {
            break;
        }
        done += 1;
    }
    done
}

/// A driver's `total`, `last` and `best` over their first `laps` laps.
fn run_over(driver: usize, laps: u32, seed: u64) -> (f64, f64, f64) {
    let times: Vec<f64> = (1..=laps).map(|lap| lap_time(driver, lap, seed)).collect();
    (
        times.iter().sum(),
        times.last().copied().unwrap_or(0.0),
        times.iter().copied().fold(f64::INFINITY, f64::min),
    )
}

/// A lap's three sector times, rescaled to sum back to the lap itself.
///
/// Two components decide the split. A driver's strength in a sector holds
/// all race, so the field's best sectors belong to different cars rather
/// than all to the quickest one; a smaller per-lap wobble on top means a
/// driver's best sector need not come from their best lap.
fn sector_times(driver: usize, lap: u32, seed: u64) -> [f64; 3] {
    const SPLITS: [f64; 3] = [0.31, 0.36, 0.33];
    let mut parts = [0.0; 3];
    for (index, split) in SPLITS.iter().enumerate() {
        let strength = noise(seed, &[driver as u64, SECTOR_SALT, index as u64]) - 0.5;
        let wobble = noise(
            seed,
            &[driver as u64, SECTOR_SALT, index as u64, u64::from(lap)],
        ) - 0.5;
        parts[index] = split * wobble.mul_add(0.01, strength.mul_add(0.05, 1.0));
    }
    let whole: f64 = parts.iter().sum();
    let lap_secs = lap_time(driver, lap, seed);
    parts.map(|part| lap_secs * part / whole)
}

/// A driver's own best time in each sector over the laps they have run.
fn best_sectors(driver: usize, laps: u32, seed: u64) -> [f64; 3] {
    (1..=laps).fold([f64::INFINITY; 3], |mut best, lap| {
        for (slot, time) in best.iter_mut().zip(sector_times(driver, lap, seed)) {
            *slot = slot.min(time);
        }
        best
    })
}

/// Salt separating pit-stagger noise from lap-time noise.
const PIT_SALT: u64 = 0x9174;
/// Salt picking the retiring driver.
const RETIRE_SALT: u64 = 0xdead;
const DSQ_SALT: u64 = 0x5c1a;
/// Salt picking the driver who picks up damage, and the one separating
/// sector splits from lap times.
const DAMAGE_SALT: u64 = 0x0bad;
const SECTOR_SALT: u64 = 0x53c7;
/// Salt for a driver's form, which reorders the grid over a race.
const FORM_SALT: u64 = 0xf0f1;

/// Which lap leaves one driver damaged, and what it costs them a lap.
/// Enough that they are a lap down before the race is out, which is the
/// only way the boards ever show a gap counted in laps.
const DAMAGE_LAP: u32 = 12;
const DAMAGE_PENALTY_SECS: f64 = 2.4;

/// What a lap gains on the one before it as rubber goes down and fuel
/// burns off. Without the trend every best lands in the opening laps and
/// is never beaten, so nothing is ever purple.
const EVOLUTION_SECS_PER_LAP: f64 = 0.006;

/// Which driver picks up that damage, never one already leaving the race.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "an index into the 22-driver grid"
)]
fn limper(seed: u64) -> usize {
    let mut pick = (noise(seed, &[DAMAGE_SALT]) * GRID.len() as f64) as usize;
    while pick == retiree(seed) || pick == excluded(seed) {
        pick = (pick + 1) % GRID.len();
    }
    pick
}

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

/// Which driver is excluded at [`DISQUALIFICATION_LAP`], never the retiree,
/// so a board can show both ways out of a race at once.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "an index into the 22-driver grid"
)]
fn excluded(seed: u64) -> usize {
    let pick = (noise(seed, &[DSQ_SALT]) * GRID.len() as f64) as usize;
    if pick == retiree(seed) {
        (pick + 1) % GRID.len()
    } else {
        pick
    }
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
    out: Option<&'static str>,
}

impl Progress {
    fn is_out(&self) -> bool {
        self.out.is_some()
    }
}

/// A car to measure against: the leader, or whoever is directly ahead.
#[derive(Clone, Copy)]
struct Ahead {
    laps: u32,
    total: f64,
}

/// What a row reports as its gap to the leader and interval to the car
/// ahead.
///
/// A car a lap down is reported in laps, not seconds: the two totals cover
/// different distances, so subtracting them means nothing.
fn distances(
    run: &Progress,
    position: usize,
    in_pit: bool,
    leader: Ahead,
    ahead: Ahead,
) -> (String, String) {
    let laps_down = |other: u32| match other.saturating_sub(run.laps) {
        0 => None,
        1 => Some("+1 LAP".to_owned()),
        behind => Some(format!("+{behind} LAPS")),
    };
    if run.is_out() {
        ("-".to_owned(), "-".to_owned())
    } else if position == 0 {
        ("LEADER".to_owned(), "-".to_owned())
    } else if in_pit {
        ("PIT".to_owned(), "PIT".to_owned())
    } else {
        (
            laps_down(leader.laps).unwrap_or_else(|| gap(run.total - leader.total)),
            laps_down(ahead.laps).unwrap_or_else(|| gap(run.total - ahead.total)),
        )
    }
}

/// The distance from the leader's best lap, which is what a timed session
/// calls a gap: tenths, and never a lap count.
fn best_lap_gap(run: &Progress, position: usize, leader_best: f64) -> String {
    if position == 0 {
        "LEADER".to_owned()
    } else {
        gap(run.best - leader_best)
    }
}

/// One driver's latest sectors, coloured for the times they show: purple
/// where a time is the session's best, green where it is the driver's own.
fn painted_sectors(run: &Progress, own: [f64; 3], session: [f64; 3], seed: u64) -> Json {
    let times = sector_times(run.driver, run.laps, seed);
    let color_of = |which: usize| {
        if times[which] <= session[which] {
            "purple"
        } else if times[which] <= own[which] {
            "green"
        } else {
            "white"
        }
    };
    json!({
        "sector1": { "time": times[0], "color": color_of(0) },
        "sector2": { "time": times[1], "color": color_of(1) },
        "sector3": { "time": times[2], "color": color_of(2) },
    })
}

/// The whole field's state at scenario time `t_s`,
/// in the order the session classifies it.
fn field_at(session: Session, t_s: f64, seed: u64) -> (Vec<Progress>, u32) {
    // The session wraps past the race distance, so a sim left running
    // overnight is still mid-race whenever the testbed connects.
    let race_secs = f64::from(TOTAL_LAPS) * LAP_SECS;
    let into = t_s.rem_euclid(race_secs).max(LAP_SECS);
    let retired_driver = retiree(seed);
    let excluded_driver = excluded(seed);
    let mut field: Vec<Progress> = (0..GRID.len())
        .map(|driver| {
            // Each driver's own laps, not the leader's: a car slow enough
            // over a stint drops a whole lap behind, which is the state a
            // gap counted in laps exists to report.
            let run_to = laps_by(driver, into, seed);
            let out = if driver == retired_driver && run_to >= RETIREMENT_LAP {
                Some(("DNF", RETIREMENT_LAP))
            } else if driver == excluded_driver && run_to >= DISQUALIFICATION_LAP {
                Some(("DSQ", DISQUALIFICATION_LAP))
            } else {
                None
            };
            let laps = out.map_or(run_to, |(_, lap)| lap);
            let (total, last, best) = run_over(driver, laps, seed);
            Progress {
                driver,
                total,
                last,
                best,
                laps,
                out: out.map(|(status, _)| status),
            }
        })
        .collect();
    match session {
        // A race classifies on distance covered. Whoever is out sinks to
        // the tail; the rest run by laps completed, and then by how long
        // those laps took.
        Session::Race => field.sort_by(|a, b| {
            a.is_out()
                .cmp(&b.is_out())
                .then(b.laps.cmp(&a.laps))
                .then(a.total.total_cmp(&b.total))
        }),
        // A timed session classifies on the best lap alone, and a set lap
        // stands: a driver who stops keeps the place that lap earned,
        // rather than sinking for the laps they no longer run.
        Session::Quali | Session::Practice => field.sort_by(|a, b| a.best.total_cmp(&b.best)),
        Session::Idle => unreachable!("BUG: an idle session never builds a board"),
    }
    // The furthest anyone has run, not the first row's, so the header
    // reads the same whichever way the session just sorted the field.
    let lead_lap = field
        .iter()
        .filter(|run| !run.is_out())
        .map(|run| run.laps)
        .max()
        .unwrap_or(0);
    (field, lead_lap)
}

fn board(session: Session, ctx: &RequestCtx, start_lap: u32) -> Json {
    let elapsed = f64::from(start_lap).mul_add(LAP_SECS, ctx.t_s);
    let (field, lead_lap) = field_at(session, elapsed, ctx.seed);
    // Where the clock judges the session a stopped driver's lap still
    // counts, so the board cannot hand the fastest lap to a driver
    // slower than the row above.
    let session_best = field
        .iter()
        .filter(|p| session.is_timed() || !p.is_out())
        .map(|p| p.best)
        .fold(f64::INFINITY, f64::min);
    // Each driver's own sector bests, and the field's best of those:
    // a latest lap matching the first paints green, the second purple.
    let own_bests: Vec<[f64; 3]> = field
        .iter()
        .map(|run| best_sectors(run.driver, run.laps, ctx.seed))
        .collect();
    // Unlike the session's best lap, this stays with the running.
    // Nobody who has stopped publishes sectors, so crediting one of them
    // would leave the colour on no row at all.
    let session_sectors = field
        .iter()
        .zip(&own_bests)
        .filter(|(run, _)| !run.is_out())
        .fold([f64::INFINITY; 3], |mut bests, (_, own)| {
            for (best, time) in bests.iter_mut().zip(own) {
                *best = best.min(*time);
            }
            bests
        });
    let leader_total = field.first().map_or(0.0, |p| p.total);
    let leader_best = field.first().map_or(0.0, |p| p.best);
    let mut prior_total = leader_total;
    let mut prior_laps = lead_lap;
    let entries: Vec<Json> = field
        .iter()
        .enumerate()
        .map(|(position, run)| {
            let driver = &GRID[run.driver];
            let team = &TEAMS[driver.team];
            // Movement since the grid, which is what the field publishes —
            // and the only window wide enough to hold any: over one lap the
            // order barely trades, so a board reads all zeros.
            let started = run.driver;
            let stint_age = (2..=run.laps)
                .rev()
                .take_while(|lap| !pits(run.driver, *lap, ctx.seed))
                .count();
            let stint = usize::try_from(run.laps.div_euclid(STINT_LAPS)).unwrap_or(0);

            // Offset per driver, not by stint alone: a real grid runs
            // several strategies at once, and a field sharing one compound
            // shows the widget only one of the three tyres it draws.
            let compound = ["S", "M", "H"][(stint + run.driver) % 3];
            let in_pit = !run.is_out() && pits(run.driver, run.laps, ctx.seed);
            let (gap_text, interval) = if session.is_timed() {
                (best_lap_gap(run, position, leader_best), String::new())
            } else {
                distances(
                    run,
                    position,
                    in_pit,
                    Ahead {
                        laps: lead_lap,
                        total: leader_total,
                    },
                    Ahead {
                        laps: prior_laps,
                        total: prior_total,
                    },
                )
            };
            prior_total = run.total;
            prior_laps = run.laps;
            let sectors = if run.is_out() {
                json!({})
            } else {
                painted_sectors(run, own_bests[position], session_sectors, ctx.seed)
            };
            entry(
                session,
                Row {
                    position,
                    run,
                    driver,
                    team,
                    sectors,
                    position_change: i64::try_from(started).unwrap_or(0)
                        - i64::try_from(position).unwrap_or(0),
                    gap_to_leader: gap_text,
                    interval,
                    compound,
                    tire_age: stint_age,
                    in_pit,
                    // The lap leaving the pits, which the practice board marks.
                    is_out_lap: !run.is_out()
                        && run.laps > 2
                        && pits(run.driver, run.laps - 1, ctx.seed),
                    fastest_lap: (session.is_timed() || !run.is_out())
                        && (run.best - session_best).abs() < f64::EPSILON,
                },
            )
        })
        .collect();
    json!({
        "live": true,
        "session_label": session_label(session),
        "gp_name": "Dutch GP",
        "country_flag_url": image_url("flag", "NED"),
        "current_lap": lead_lap,
        "total_laps": TOTAL_LAPS,
        "entries": entries,
    })
}

/// One row's figures, before a board decides which of them it publishes.
struct Row<'a> {
    position: usize,
    run: &'a Progress,
    driver: &'a SimDriver,
    team: &'a Team,
    sectors: Json,
    position_change: i64,
    gap_to_leader: String,
    interval: String,
    compound: &'static str,
    tire_age: usize,
    in_pit: bool,
    is_out_lap: bool,
    fastest_lap: bool,
}

/// `row` carrying what the session's own entry type defines, and no more.
///
/// Upstream's three live entries are not one shape with fields left out.
/// A race row reports an interval, a tyre's age, a pit and a status;
/// a qualifying row reports none of those, only the lap that ranks it.
/// Serving one row to all three would let a widget read a field on a board
/// that never carries it and still look right here — a bug only a deck shows.
///
/// Mirrored by hand from the `LiveEntry`, `QualiEntry` and `PracticeEntry`
/// of `nexus-data`; the plan doc records the revision they were read at.
fn entry(session: Session, row: Row<'_>) -> Json {
    let run = row.run;
    let last_lap = if run.is_out() {
        "-".to_owned()
    } else {
        clock(run.last)
    };
    let mut entry = json!({
        "position": row.position + 1,
        "driver_code": row.driver.code,
        "driver_name": row.driver.name,
        "team_logo_url": image_url("logo", row.team.slug),
        "team_color": row.team.livery,
        "fastest_lap": row.fastest_lap,
    });
    let Json::Object(map) = &mut entry else {
        unreachable!("BUG: json! built something other than an object")
    };
    if let Json::Object(sectors) = row.sectors {
        map.extend(sectors);
    }
    match session {
        Session::Race => {
            map.insert("position_change".to_owned(), row.position_change.into());
            map.insert("gap_to_leader".to_owned(), row.gap_to_leader.into());
            map.insert("interval".to_owned(), row.interval.into());
            map.insert("last_lap_time".to_owned(), last_lap.into());
            map.insert("tire_compound".to_owned(), row.compound.into());
            map.insert("tire_age".to_owned(), row.tire_age.into());
            map.insert("in_pit".to_owned(), row.in_pit.into());
            map.insert("status".to_owned(), run.out.unwrap_or("RUN").into());
        }
        Session::Quali => {
            map.insert("position_change".to_owned(), row.position_change.into());
            // A lap, not a sum: `QualiEntry` documents this as the lap
            // its sectors add up to, and builds it from the best one.
            map.insert("total_time".to_owned(), clock(run.best).into());
        }
        Session::Practice => {
            map.insert("best_lap_time".to_owned(), clock(run.best).into());
            map.insert("tire_compound".to_owned(), row.compound.into());
            map.insert("gap_to_leader".to_owned(), row.gap_to_leader.into());
            map.insert("last_lap_time".to_owned(), last_lap.into());
            map.insert("is_out_lap".to_owned(), row.is_out_lap.into());
        }
        Session::Idle => unreachable!("BUG: an idle session never builds a board"),
    }
    entry
}

/// The header a board names itself by.
fn session_label(session: Session) -> &'static str {
    match session {
        Session::Race => "Race",
        Session::Quali => "Qualifying",
        Session::Practice => "Practice",
        Session::Idle => unreachable!("BUG: an idle session never builds a board"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn ctx(t_s: f64) -> RequestCtx {
        RequestCtx {
            query: BTreeMap::new(),
            t_s,
            seed: 0xF1,
            host: None,
            cache: Arc::new(crate::cache::Cache::new::<Vec<_>>(Vec::new())),
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
        let ResponseSpec::Computed(responder) = &live.response else {
            panic!("BUG: live boards answer per request");
        };
        let reply = json_of(responder(&ctx(100.0)));
        assert_eq!(reply["data"]["live"], false);
        assert_eq!(reply["ttl_secs"], 3);
    }

    #[test]
    fn a_race_replays_identically_and_positions_close_up() {
        let one = board(Session::Race, &ctx(1_800.0), 0);
        let two = board(Session::Race, &ctx(1_800.0), 0);
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
        let reply = board(Session::Race, &ctx(secs_past_retirement), 0);
        let entries = reply["entries"].as_array().expect("entries");
        let last = entries.last().expect("a tail");
        assert_eq!(last["status"], "DNF");
        assert_eq!(last["gap_to_leader"], "-");
    }

    /// A race ranks on distance and a timed session on the best lap, so a
    /// board that sorts one way for both seats a driver above another it
    /// has just reported as slower — and a driver whose session ended keeps
    /// the lap they set rather than sinking under drivers who never beat it.
    #[test]
    fn a_timed_session_classifies_on_the_best_lap() {
        let (field, _) = field_at(Session::Quali, 55.0 * LAP_SECS, ctx(0.0).seed);
        for pair in field.windows(2) {
            assert!(
                pair[0].best <= pair[1].best,
                "P{} laps {:.3} and sits above P{} on {:.3}",
                pair[0].driver + 1,
                pair[0].best,
                pair[1].driver + 1,
                pair[1].best,
            );
        }
        assert!(
            field.iter().any(Progress::is_out),
            "a session with nobody out cannot show that a set lap stands",
        );
    }

    /// `QualiEntry.total_time` reads as a running total and is not one:
    /// upstream documents it as the lap the sectors add up to and fills it
    /// from the driver's best. Filling it with a session total instead draws
    /// a board that looks right and reports a figure no deployment sends.
    #[test]
    fn a_qualifying_row_reports_the_lap_it_ranks_on() {
        let at = ctx(0.0);
        let (field, _) = field_at(Session::Quali, 55.0 * LAP_SECS, at.seed);
        let quali = board(Session::Quali, &at, 55);
        let rows = quali["entries"].as_array().expect("entries");
        for (entry, run) in rows.iter().zip(&field) {
            assert_eq!(
                entry["total_time"].as_str().expect("a lap"),
                clock(run.best),
                "a qualifying row reports the best lap it ranks on: {entry}"
            );
        }
    }

    /// A timed session gaps on the best lap, in tenths. The race's
    /// cumulative gap puts seconds beside laps a thousandth apart, and can
    /// count laps on a session that never classifies by distance.
    #[test]
    fn a_timed_session_gaps_on_the_best_lap_and_never_on_laps() {
        let at = ctx(0.0);
        let (field, _) = field_at(Session::Practice, 51.0 * LAP_SECS, at.seed);
        let reply = board(Session::Practice, &at, 51);
        let rows = reply["entries"].as_array().expect("entries");
        assert_eq!(rows[0]["gap_to_leader"], "LEADER");
        for (entry, run) in rows.iter().zip(&field).skip(1) {
            let reported = entry["gap_to_leader"].as_str().expect("a gap");
            assert!(
                !reported.contains("LAP"),
                "a timed session counts no laps: {reported}"
            );
            assert_eq!(reported, gap(run.best - field[0].best), "row {entry}");
        }
    }

    /// The lap `sim-blueprint.json5` opens practice at.
    const PRACTICE_START_LAP: u32 = 53;

    /// Practice ranks on the best lap, so the driver leaving the pits need
    /// not land in the rows a frame seats — and unseated is unshown.
    #[test]
    fn the_practice_scenario_seats_a_driver_on_an_out_lap() {
        // What the narrowest frame carrying the column seats.
        const SEATS: usize = 5;
        let seats_one = |start_lap: u32| {
            let reply = board(Session::Practice, &ctx(0.0), start_lap);
            reply["entries"].as_array().expect("entries")[..SEATS]
                .iter()
                .any(|entry| entry["is_out_lap"] == true)
        };
        assert!(
            seats_one(PRACTICE_START_LAP),
            "lap {PRACTICE_START_LAP} seats no out lap; these would: {:?}",
            (40..70).filter(|lap| seats_one(*lap)).collect::<Vec<_>>()
        );
    }

    /// A card reads as one person, so its invented figures have to agree.
    /// An age and a debut drawn from the same index by unrelated formulas
    /// put a nineteen-year-old on the grid since 2001.
    #[test]
    fn no_driver_debuts_before_they_were_old_enough_to_drive() {
        use chrono::Datelike as _;

        let season = i64::from(chrono::Utc::now().year());
        for entry in driver_stats().as_array().expect("rows") {
            let age = entry["age"].as_i64().expect("an age");
            let debut = entry["debut_year"].as_i64().expect("a debut");
            let age_at_debut = age - (season - debut);
            assert!(
                (18..=21).contains(&age_at_debut),
                "{} first raced at {age_at_debut}: {entry}",
                entry["driver_name"]
            );
        }
    }

    /// No frame owes a purple, but a session that never improves on itself
    /// would never paint one.
    #[test]
    fn a_session_sets_new_best_sectors_as_it_runs() {
        let mut painters = BTreeSet::new();
        for lap in 5..60 {
            let reply = board(Session::Race, &ctx(0.0), lap);
            for entry in reply["entries"].as_array().expect("entries") {
                let purple =
                    (1..=3).any(|which| entry[format!("sector{which}")]["color"] == "purple");
                if purple {
                    painters.insert(entry["driver_code"].as_str().expect("code").to_owned());
                }
            }
        }
        assert!(
            painters.len() > 1,
            "the session never improves on itself: {painters:?}"
        );
    }

    /// The colour describes the time beside it, so no purple may sit above
    /// a quicker white — which is what colouring by holder produces, since
    /// the cell shows a latest lap rather than a best.
    #[test]
    fn a_purple_sector_is_the_quickest_the_board_shows() {
        let reply = board(Session::Practice, &ctx(0.0), 51);
        let rows = reply["entries"].as_array().expect("entries");
        for which in 1..=3 {
            let key = format!("sector{which}");
            let quickest = rows
                .iter()
                .filter_map(|entry| entry[&key]["time"].as_f64())
                .fold(f64::INFINITY, f64::min);
            for entry in rows {
                let Some(time) = entry[&key]["time"].as_f64() else {
                    continue;
                };
                if entry[&key]["color"] == "purple" {
                    assert!(
                        time <= quickest,
                        "{key} paints {time:.3} purple over {quickest:.3}"
                    );
                }
            }
        }
    }

    /// The three live entries are different shapes, not one shape with
    /// fields left out: a race row reports an interval, a tyre's age and a
    /// status; a qualifying row only the lap that ranks it. Serving one row
    /// to every board lets a widget read a field on a board that never
    /// carries it and still look right here — a bug only a deck shows.
    ///
    /// Keys mirrored from `nexus-data`'s `LiveEntry`, `QualiEntry` and
    /// `PracticeEntry`.
    #[test]
    fn each_board_publishes_the_fields_its_own_entry_defines() {
        const COMMON: [&str; 6] = [
            "position",
            "driver_code",
            "driver_name",
            "team_logo_url",
            "team_color",
            "fastest_lap",
        ];
        let defined = |session| -> BTreeSet<String> {
            let own: &[&str] = match session {
                Session::Race => &[
                    "position_change",
                    "gap_to_leader",
                    "interval",
                    "last_lap_time",
                    "tire_compound",
                    "tire_age",
                    "in_pit",
                    "status",
                ],
                Session::Quali => &["position_change", "total_time"],
                Session::Practice => &[
                    "best_lap_time",
                    "tire_compound",
                    "gap_to_leader",
                    "last_lap_time",
                    "is_out_lap",
                ],
                Session::Idle => &[],
            };
            COMMON
                .iter()
                .chain(own)
                .map(|key| (*key).to_owned())
                .collect()
        };
        for session in [Session::Race, Session::Quali, Session::Practice] {
            let reply = board(session, &ctx(0.0), 50);
            for entry in reply["entries"].as_array().expect("entries") {
                let mut keys: BTreeSet<String> = entry
                    .as_object()
                    .expect("an object")
                    .keys()
                    .cloned()
                    .collect();
                // The one optional group, on all three: a stopped driver
                // publishes no sectors at all.
                for which in 1..=3 {
                    keys.remove(&format!("sector{which}"));
                }
                assert_eq!(keys, defined(session), "{session:?} row: {entry}");
            }
        }
    }

    /// A board exists to show the widget's states, and these were all once
    /// a function of lap alone: the whole grid shared a compound and turned
    /// the same sector green at the same moment.
    #[test]
    fn one_frame_of_a_race_shows_the_grid_apart() {
        let reply = board(Session::Race, &ctx(0.0), 50);
        let entries = reply["entries"].as_array().expect("entries");

        let compounds: BTreeMap<&str, usize> =
            entries.iter().fold(BTreeMap::new(), |mut counts, entry| {
                *counts
                    .entry(entry["tire_compound"].as_str().unwrap_or_default())
                    .or_default() += 1;
                counts
            });
        assert!(
            compounds.len() >= 3,
            "the grid runs one strategy: {compounds:?}"
        );

        let painted: BTreeMap<&str, usize> = entries
            .iter()
            .flat_map(|entry| {
                (1..=3).filter_map(move |which| entry[format!("sector{which}")]["color"].as_str())
            })
            .fold(BTreeMap::new(), |mut counts, color| {
                *counts.entry(color).or_default() += 1;
                counts
            });
        // Purple belongs to the lap that set the best, so a frame may hold
        // none; `a_session_sets_new_best_sectors_as_it_runs` covers it.
        for color in ["green", "white"] {
            assert!(
                painted.contains_key(color),
                "no sector is {color}: {painted:?}"
            );
        }

        let moved: Vec<i64> = entries
            .iter()
            .filter_map(|entry| entry["position_change"].as_i64())
            .filter(|change| *change != 0)
            .collect();
        assert!(
            moved.iter().any(|change| *change > 0) && moved.iter().any(|change| *change < 0),
            "the grid finished as it started: {moved:?}"
        );

        let statuses: Vec<&str> = entries
            .iter()
            .filter_map(|entry| entry["status"].as_str())
            .filter(|status| *status != "RUN")
            .collect();
        assert_eq!(
            statuses.len(),
            2,
            "a race this far in has both a retirement and an exclusion: {statuses:?}"
        );
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

    /// A missing fixture is only a warning at runtime.
    /// When the artwork moved, every image 404'd onto a placeholder,
    /// and the sim looked unfinished rather than broken.
    #[test]
    fn every_fixture_names_artwork_the_widget_actually_ships() {
        let artwork = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../widgets-wasm/formula-1/gallery/artwork");
        for ImageFixture { kind, key, file } in image_fixtures() {
            let path = artwork.join(&file);
            assert!(
                path.is_file(),
                "{kind}/{key} names {}, which the widget does not ship",
                file.display()
            );
        }
    }

    #[test]
    fn the_off_season_empties_the_calendar_and_the_table() {
        assert_eq!(standings(false), serde_json::json!([]));
        assert_eq!(next_race(false, false), serde_json::json!({}));
    }

    /// Reply from the endpoint serving `resource`, as `host` reached it.
    fn reply(spec: &ResourceSpec, resource: &str, host: Option<&str>) -> Json {
        let path = format!("/api/v1/data/formula-1/{resource}");
        let endpoint = spec
            .endpoints
            .iter()
            .find(|e| e.path == path)
            .expect("BUG: the resource must be served");
        let ResponseSpec::Computed(responder) = &endpoint.response else {
            panic!("BUG: a resource answers per request");
        };
        json_of(responder(&RequestCtx {
            host: host.map(str::to_owned),
            ..ctx(0.0)
        }))
    }

    /// The JSON a computed endpoint answered with.
    fn json_of(response: Response) -> Json {
        match response.data {
            ResponseData::Json(json) => json,
            ResponseData::Bytes { .. } => panic!("BUG: this endpoint answers JSON"),
        }
    }

    #[test]
    fn image_urls_name_the_address_the_request_arrived_on() {
        let spec = Params::default().resource("f1", 20_100);
        let standings = reply(&spec, "standings", Some("192.168.1.50:20100"));
        let logo = standings["data"][0]["team_logo_url"]
            .as_str()
            .expect("a logo url");
        assert_eq!(logo, "http://192.168.1.50:20100/img/logo/mercedes.png");
    }

    #[test]
    fn a_caller_sending_no_host_still_gets_a_dialable_url() {
        let spec = Params::default().resource("f1", 20_100);
        let standings = reply(&spec, "standings", None);
        let logo = standings["data"][0]["team_logo_url"]
            .as_str()
            .expect("a logo url");
        assert_eq!(logo, "http://127.0.0.1:20100/img/logo/mercedes.png");
    }

    /// Every URL a payload publishes must have an endpoint behind it.
    /// The two are built from separate lists,
    /// so an image added to one but not the other is a 404 that nobody
    /// notices until the art goes missing.
    #[test]
    fn every_published_image_url_is_served() {
        let dir = tempfile::tempdir().expect("BUG: a temp dir must be creatable");
        for fixture in image_fixtures() {
            let path = dir.path().join(&fixture.file);
            std::fs::create_dir_all(path.parent().expect("BUG: a fixture sits in a directory"))
                .expect("BUG: the fixture tree must be creatable");
            std::fs::write(&path, b"png").expect("BUG: the fixture must be writable");
        }
        let params = Params {
            image_dir: Some(dir.path().to_owned()),
            season_underway: true,
            ..Params::default()
        };
        let spec = params.resource("f1", 20_100);
        let served: Vec<&str> = spec.endpoints.iter().map(|e| e.path.as_str()).collect();
        let host = "sim.test";
        for resource in ["standings", "driver-stats", "teams", "next-race"] {
            let payload = reply(&spec, resource, Some(host)).to_string();
            for url in payload.split('"').filter(|part| part.contains("/img/")) {
                let path = url
                    .strip_prefix(&format!("http://{host}"))
                    .expect("BUG: an image url names the serving host");
                assert!(
                    served.contains(&path),
                    "{resource} publishes unserved {path}"
                );
            }
        }
    }

    #[test]
    fn an_unreadable_fixture_leaves_its_image_unserved() {
        let params = Params {
            image_dir: Some(PathBuf::from("/nonexistent-fixture-dir")),
            ..Params::default()
        };
        let spec = params.resource("f1", 20_100);
        assert!(
            !spec.endpoints.iter().any(|e| e.path.contains("/img/")),
            "a missing fixture tree serves no image, rather than failing the run"
        );
    }

    /// The weekend is dated from the day the sim runs, so a day-of-week
    /// slip shows only on the weekday that triggers it — a Sunday run
    /// announced a race on the Monday until this held it.
    /// A card without the season's figures is what makes the widget's
    /// join observable: served the whole row, a failed join would draw
    /// the same screen as a working one.
    #[test]
    fn a_driver_card_leaves_the_seasons_figures_to_the_table() {
        let card = driver_card(0);
        assert_eq!(card["jolpica_id"], json!(GRID[0].slug));
        for derived in ["ranking", "points", "gp_wins", "world_titles"] {
            assert!(card[derived].is_null(), "the card carries no {derived}");
        }
        let row = driver_row(0);
        for shared in ["jolpica_id", "name", "number", "team"] {
            assert_eq!(card[shared], row[shared], "{shared} differs from the table");
        }
    }

    #[test]
    fn the_weekend_falls_on_the_days_it_is_named_for() {
        use chrono::Datelike as _;

        let race = next_race(true, false);
        let day = |field: &str| {
            race[field]
                .as_str()
                .expect("a weekend date")
                .parse::<chrono::NaiveDate>()
                .expect("BUG: a weekend date is written YYYY-MM-DD")
        };
        assert_eq!(day("date_start").weekday(), chrono::Weekday::Fri);
        assert_eq!(day("date_end").weekday(), chrono::Weekday::Sun);
        assert!(
            day("date_start") > chrono::Utc::now().date_naive(),
            "the weekend is upcoming, never one already run"
        );
    }

    /// A handful of weekends a year run a sprint, so the schedule it seats
    /// is otherwise a matter of waiting for the calendar.
    #[test]
    fn a_sprint_weekend_trades_two_practices_for_a_sprint() {
        let names = |race: &Json| {
            race["sessions"]
                .as_array()
                .expect("sessions")
                .iter()
                .filter_map(|session| session["name"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&next_race(true, true)),
            [
                "Practice 1",
                "Sprint Qualifying",
                "Sprint",
                "Qualifying",
                "Race"
            ]
        );
        assert!(
            names(&next_race(true, false)).contains(&"Practice 3".to_owned()),
            "an ordinary weekend keeps all three practices"
        );
    }

    #[test]
    fn a_session_start_is_an_instant_under_the_weekends_date_key() {
        let race = next_race(true, false);
        let session = &race["sessions"][0];
        assert!(
            session["starts_at"].is_null(),
            "`starts_at` is the deriver's rust field, never a wire key"
        );
        let start = session["date_start"].as_str().expect("a session start");
        assert!(
            start.ends_with('Z'),
            "a session names an instant where the weekend names a day: {start}"
        );
    }

    #[test]
    fn the_constructors_snapshot_nests_its_rows_beside_the_season() {
        let snapshot = teams();
        assert!(
            snapshot["teams"].is_array(),
            "the rows hang off a key, unlike every other table payload"
        );
        let first = &snapshot["teams"][0];
        for field in ["id", "name", "image_path", "resolved_color"] {
            assert!(!first[field].is_null(), "a constructor carries {field}");
        }
    }
}
