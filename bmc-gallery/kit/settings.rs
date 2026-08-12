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

//! The deck-wide settings widgets format against, as knobs.
//!
//! Off-device nothing delivers them, so a scene that renders a
//! temperature, a distance or a clock would show one operator's settings
//! and no other. These put the settings on the Controls panel and
//! install what the scene reads back.

use bmc_wasm_sdk::system::{
    self, DateFormat, NumberFormat, Snapshot, SnapshotBuilder, TemperatureUnit, TimeFormat,
    UnitSystem,
};
use gallery::SceneCtx;

/// The deck-wide settings, as their own group on the Controls panel.
///
/// Installs the snapshot it builds, so everything staged afterwards
/// formats the way an operator with those settings sees it. Returns it
/// too, for a scene that wants to label what it is showing.
pub fn deck_settings(ctx: &mut SceneCtx) -> Snapshot {
    ctx.group("System settings");

    let units = ctx.radio("Units", &["Metric", "Imperial"], 0);
    let clock = ctx.radio("Clock", &["24-hour", "12-hour"], 0);
    let temperature = ctx.radio("Temperature", &["Celsius", "Fahrenheit"], 0);
    let numbers = ctx.select("Numbers", &["1 234,5", "1,234.5", "1.234,5", "1 234.5"], 0);
    let dates = ctx.radio("Dates", &["31.12.2026", "31/12/2026", "12/31/2026"], 0);

    let snapshot = SnapshotBuilder::new()
        .unit_system(match units {
            1 => UnitSystem::Imperial,
            _ => UnitSystem::Metric,
        })
        .time_format(match clock {
            1 => TimeFormat::Hour12,
            _ => TimeFormat::Hour24,
        })
        .temperature_unit(match temperature {
            1 => TemperatureUnit::Fahrenheit,
            _ => TemperatureUnit::Celsius,
        })
        .number_format(match numbers {
            1 => NumberFormat::CommaGroupDotDecimal,
            2 => NumberFormat::DotGroupCommaDecimal,
            3 => NumberFormat::SpaceGroupDotDecimal,
            _ => NumberFormat::SpaceGroupCommaDecimal,
        })
        .date_format(match dates {
            1 => DateFormat::DdMmYyyySlash,
            2 => DateFormat::MDYyyySlash,
            _ => DateFormat::DdMmYyyyDot,
        })
        .build();

    system::set_current(snapshot.clone());
    snapshot
}
