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

// The host runs the same core, so native output matches the device.

use bmc_shared_utils::number_format::NumberFormat;
use bmc_shared_utils::temperature::TemperatureUnit;
use bmc_shared_utils::unit_system::UnitSystem;

use super::{FormatTextProvider, FormatZoneOffsetProvider};
use crate::tz::Tz;

fn number_format() -> NumberFormat {
    crate::system::current()
        .number_format()
        .map(NumberFormat::from)
        .unwrap_or_default()
}

fn temperature_unit() -> TemperatureUnit {
    crate::system::current()
        .temperature_unit()
        .map(TemperatureUnit::from)
        .unwrap_or_default()
}

fn unit_system() -> UnitSystem {
    crate::system::current()
        .unit_system()
        .map(UnitSystem::from)
        .unwrap_or_default()
}

pub(super) struct Host;

impl FormatTextProvider for Host {
    fn number(value: f64, decimals: u32) -> String {
        number_format().format_number(value, decimals as usize)
    }

    fn speed(value: f64, decimals: u32, metric_unit: u32) -> String {
        use bmc_shared_utils::unit_system::MetricSpeedUnit;
        let metric_unit = if metric_unit == 1 {
            MetricSpeedUnit::Ms
        } else {
            MetricSpeedUnit::KmH
        };
        unit_system().format_speed(number_format(), value, decimals as usize, metric_unit)
    }

    fn distance(value: f64, decimals: u32) -> String {
        unit_system().format_distance(number_format(), value, decimals as usize)
    }

    fn temperature(value: f64, decimals: u32, show_unit: u32) -> String {
        temperature_unit().format(number_format(), value, decimals as usize, show_unit != 0)
    }

    fn strftime(timestamp: i64, format: &str) -> String {
        bmc_wasm_protocol::time::strftime_utc(timestamp, format).unwrap_or_default()
    }
}

impl FormatZoneOffsetProvider for Host {
    fn zone_offset(tz: &Tz, unix_secs: i64) -> Option<i32> {
        bmc_wasm_protocol::time::zone_offset_secs(unix_secs, tz.iana())
    }
}
