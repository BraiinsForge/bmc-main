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

//! Reading a Nexus reply into [`crate::model`].
//!
//! `JsonDoc` reads through the host,
//! so this module is only field plumbing —
//! every rule that plain values can decide lives in `model.rs`,
//! where it is natively testable.

use bmc_wasm_sdk::{CalendarDate, JsonDoc, Length, LocalDateTime, calendar};

use crate::api::wire;
use crate::model::{
    CarNumber, DriverStats, DriverStatus, ImageUrl, LiveBoard, NextRace, Sector, SectorColor,
    Session, StandingsRow, TimingBoard, TimingRow, TimingText, TireCompound, team_color,
};

/// Rows are read until a probe field comes back absent, which is how
/// an array payload's end is found through a pointer API.
fn each_row(json: &JsonDoc, probe: &str, mut read: impl FnMut(usize)) {
    for index in 0.. {
        if json.str(&wire::row(index, probe)).is_none() {
            break;
        }
        read(index);
    }
}

fn text(json: &JsonDoc, path: &str) -> String {
    json.str(path).unwrap_or_default()
}

fn image(json: &JsonDoc, path: &str) -> ImageUrl {
    ImageUrl::from(text(json, path))
}

fn timing(json: &JsonDoc, path: &str) -> TimingText {
    TimingText::from(text(json, path))
}

/// A session's start, resolved to an instant and put back into wall clock
/// for `zone`.
///
/// The instant is what the widget lacks while the server sends no offset:
/// the host's parser refuses such text rather than guessing a zone, so
/// these read as absent until Nexus sends one.
fn session_start(json: &JsonDoc, path: &str, zone: &str) -> Option<LocalDateTime> {
    let unix_secs = calendar::parse_datetime(&json.str(path)?)?;
    calendar::tz_convert(unix_secs, zone)
}

fn weekend_date(json: &JsonDoc, path: &str) -> Option<CalendarDate> {
    calendar::parse_calendar_date(&json.str(path)?)
}

fn kilometers(json: &JsonDoc, path: &str) -> Option<Length> {
    json.f64(path).map(Length::from_kilometers)
}

fn color(json: &JsonDoc, path: &str) -> bmc_wasm_sdk::Color {
    team_color(&text(json, path))
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "positions, points, and counts are small non-negative wire integers"
)]
fn small(json: &JsonDoc, path: &str) -> u8 {
    json.i64(path)
        .unwrap_or_default()
        .clamp(0, i64::from(u8::MAX)) as u8
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "lap counts and points fit a u16 on the wire"
)]
fn medium(json: &JsonDoc, path: &str) -> u16 {
    json.i64(path)
        .unwrap_or_default()
        .clamp(0, i64::from(u16::MAX)) as u16
}

#[must_use]
pub fn standings(json: &JsonDoc) -> Vec<StandingsRow> {
    let mut rows = Vec::new();
    each_row(json, "driver_name", |index| {
        let at = |field: &str| wire::row(index, field);
        rows.push(StandingsRow {
            position: small(json, &at("position")),
            driver_name: text(json, &at("driver_name")),
            driver_code: text(json, &at("driver_code")),
            team_name: text(json, &at("team_name")),
            team_logo_url: image(json, &at("team_logo_url")),
            team_color: color(json, &at("team_color")),
            country_code: text(json, &at("country_code")),
            country_flag_url: image(json, &at("country_flag_url")),
            points: medium(json, &at("points")),
            headshot_url: image(json, &at("headshot_url")),
        });
    });
    rows
}

/// One statistics row, read from `at` —
/// either an indexed row of the all-drivers table,
/// or the per-driver object.
fn driver_stats_at(json: &JsonDoc, at: &impl Fn(&str) -> String) -> DriverStats {
    DriverStats {
        name: text(json, &at("name")),
        number: CarNumber::new(small(json, &at("number"))),
        headshot_url: image(json, &at("headshot_url")),
        team: text(json, &at("team")),
        team_color: color(json, &at("team_color")),
        ranking: small(json, &at("ranking")),
        points: medium(json, &at("points")),
        nationality: text(json, &at("nationality")),
        nationality_flag_url: image(json, &at("nationality_flag_url")),
        gp_wins: json.i64(&at("gp_wins")).and_then(|v| u8::try_from(v).ok()),
        world_titles: json
            .i64(&at("world_titles"))
            .and_then(|v| u8::try_from(v).ok()),
        age: json.i64(&at("age")).and_then(|v| u8::try_from(v).ok()),
        weight_kg: json
            .i64(&at("weight_kg"))
            .and_then(|v| u8::try_from(v).ok()),
        height_cm: json
            .i64(&at("height_cm"))
            .and_then(|v| u16::try_from(v).ok()),
        race_engineer: json.str(&at("race_engineer")),
        debut_year: json
            .i64(&at("debut_year"))
            .and_then(|v| u16::try_from(v).ok()),
    }
}

#[must_use]
pub fn driver_stats(json: &JsonDoc) -> Vec<DriverStats> {
    let mut rows = Vec::new();
    each_row(json, "name", |index| {
        rows.push(driver_stats_at(json, &|field| wire::row(index, field)));
    });
    rows
}

#[must_use]
pub fn driver(json: &JsonDoc) -> Option<DriverStats> {
    let stats = driver_stats_at(json, &|field| wire::field(field));
    (!stats.name.is_empty()).then_some(stats)
}

#[must_use]
pub fn next_race(json: &JsonDoc, local_time: bool) -> Option<NextRace> {
    let gp_name = json.str(&wire::field("gp_name"))?;
    let venue_timezone = json.str(&wire::field("venue_timezone"));
    // The viewer's own zone is the host's `Local`; otherwise the schedule
    // stays on the circuit's clock, as a race weekend is always quoted.
    // Without the circuit's zone there is no circuit clock to show, and
    // the viewer's own is the only one we know — UTC would read as the
    // circuit's while being neither.
    let zone = if local_time {
        "Local"
    } else {
        venue_timezone.as_deref().unwrap_or("Local")
    };

    let mut sessions = Vec::new();
    for index in 0.. {
        let Some(name) = json.str(&wire::session(index, "name")) else {
            break;
        };
        sessions.push(Session {
            name,
            starts_at: session_start(json, &wire::session(index, "date_start"), zone),
        });
    }
    Some(NextRace {
        gp_name,
        country_name: text(json, &wire::field("country_name")),
        country_flag_url: image(json, &wire::field("country_flag_url")),
        venue_timezone,
        date_start: weekend_date(json, &wire::field("date_start")),
        date_end: weekend_date(json, &wire::field("date_end")),
        circuit_name: text(json, &wire::field("circuit_name")),
        circuit_image_url: image(json, &wire::field("circuit_image_url")),
        track_length: kilometers(json, &wire::field("track_length_km")),
        total_laps: json
            .i64(&wire::field("total_laps"))
            .and_then(|v| u16::try_from(v).ok()),
        race_distance: kilometers(json, &wire::field("race_distance_km")),
        drs_zones: json
            .i64(&wire::field("drs_zones"))
            .and_then(|v| u8::try_from(v).ok()),
        tire_compounds: json.str(&wire::field("tire_compounds")),
        sessions,
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "sector times are read for display only"
)]
fn sector(json: &JsonDoc, index: usize, which: u8) -> Option<Sector> {
    let seconds = json.f64(&wire::sector(index, which, "time"))?;
    Some(Sector {
        seconds: seconds as f32,
        color: SectorColor::from_wire(&text(json, &wire::sector(index, which, "color"))),
    })
}

/// A live board, or [`LiveBoard::Idle`] when no session is running.
/// The `live` flag is only present when idle,
/// so its absence means a session — subject to it having entries,
/// which [`LiveBoard::from_board`] decides.
#[must_use]
pub fn live_board(json: &JsonDoc) -> LiveBoard {
    if json.bool(wire::LIVE_FLAG) == Some(false) {
        return LiveBoard::Idle;
    }
    let mut rows = Vec::new();
    for index in 0.. {
        let entry = |field: &str| wire::entry(index, field);
        if json.str(&entry("driver_code")).is_none() {
            break;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a position change is a small signed wire integer"
        )]
        rows.push(TimingRow {
            position: small(json, &entry("position")),
            driver_code: text(json, &entry("driver_code")),
            driver_name: text(json, &entry("driver_name")),
            team_logo_url: image(json, &entry("team_logo_url")),
            team_color: color(json, &entry("team_color")),
            position_change: json
                .i64(&entry("position_change"))
                .unwrap_or_default()
                .clamp(i64::from(i8::MIN), i64::from(i8::MAX)) as i8,
            gap_to_leader: timing(json, &entry("gap_to_leader")),
            interval: timing(json, &entry("interval")),
            last_lap_time: timing(json, &entry("last_lap_time")),
            best_lap_time: timing(json, &entry("best_lap_time")),
            total_time: timing(json, &entry("total_time")),
            tire_compound: json
                .str(&entry("tire_compound"))
                .as_deref()
                .and_then(TireCompound::from_wire),
            tire_age: small(json, &entry("tire_age")),
            sectors: [
                sector(json, index, 1),
                sector(json, index, 2),
                sector(json, index, 3),
            ],
            in_pit: json.bool(&entry("in_pit")).unwrap_or_default(),
            is_out_lap: json.bool(&entry("is_out_lap")).unwrap_or_default(),
            fastest_lap: json.bool(&entry("fastest_lap")).unwrap_or_default(),
            status: json
                .str(&entry("status"))
                .as_deref()
                .and_then(DriverStatus::from_wire),
        });
    }
    LiveBoard::from_board(TimingBoard {
        session_label: text(json, &wire::field("session_label")),
        gp_name: text(json, &wire::field("gp_name")),
        country_flag_url: image(json, &wire::field("country_flag_url")),
        current_lap: medium(json, &wire::field("current_lap")),
        total_laps: medium(json, &wire::field("total_laps")),
        rows,
    })
}
