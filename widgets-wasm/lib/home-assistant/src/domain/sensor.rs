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

//! Sensors: a reading an integration reports, such as a temperature, a time or a word.

use super::known_state;
use crate::Entity;
use crate::format::{is_numeric, state_number, written_decimals};

pub(super) const DOMAIN: &str = "sensor";
const UNIT: &str = "unit_of_measurement";
const DEVICE_CLASS: &str = "device_class";
const STATE_CLASS: &str = "state_class";
pub const ATTRIBUTES: &[&str] = &[UNIT, DEVICE_CLASS, STATE_CLASS];
const TIMESTAMP: &str = "timestamp";
const UPTIME: &str = "uptime";

/// What a sensor's state reads as, as the frontend shows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    Number {
        value: f64,
        /// Shown only where no display precision is set.
        written_decimals: u32,
    },
    /// When something happened or will, as Home Assistant's ISO 8601 timestamp.
    Timestamp(&'a str),
    /// When something started, as Home Assistant's ISO 8601 timestamp: the frontend shows how long ago.
    Uptime(&'a str),
    /// The state as written, such as an enum sensor's option.
    Text(&'a str),
}

/// None before the sensor's first state, or while it cannot be reached.
/// A time is checked before a number, as the Tile card's state display does.
#[must_use]
pub fn value(sensor: &Entity) -> Option<Value<'_>> {
    let state = known_state(sensor)?;
    let device_class = device_class(sensor);
    match device_class {
        Some(TIMESTAMP) => return Some(Value::Timestamp(state)),
        Some(UPTIME) => return Some(Value::Uptime(state)),
        _ => {}
    }
    let numeric = is_numeric(DOMAIN, unit(sensor), sensor.text(STATE_CLASS), device_class);
    Some(
        state_number(state)
            .filter(|_| numeric)
            .map_or(Value::Text(state), |value| Value::Number {
                value,
                written_decimals: written_decimals(state),
            }),
    )
}

/// None for a sensor without a unit, or with an empty one.
#[must_use]
pub fn unit(sensor: &Entity) -> Option<&str> {
    sensor.text(UNIT).filter(|unit| !unit.is_empty())
}

/// What the sensor measures, such as `temperature`, as Home Assistant names it.
#[must_use]
pub fn device_class(sensor: &Entity) -> Option<&str> {
    sensor.text(DEVICE_CLASS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AttributeValue;
    use crate::domain::{UNAVAILABLE, UNKNOWN};
    use bmc_wasm_sdk::wasm_test;

    fn sensor(state: &str, attributes: &[(&str, &str)]) -> Entity {
        Entity::tracked(
            state,
            attributes
                .iter()
                .map(|(name, value)| (*name, AttributeValue::Text((*value).to_owned())))
                .collect(),
            ATTRIBUTES,
        )
    }

    #[wasm_test]
    fn a_state_with_a_unit_reads_as_a_number() {
        let outdoor = sensor("21.5", &[(UNIT, "°C"), (DEVICE_CLASS, "temperature")]);
        assert_eq!(
            value(&outdoor),
            Some(Value::Number {
                value: 21.5,
                written_decimals: 1
            })
        );
        assert_eq!(unit(&outdoor), Some("°C"));
        assert_eq!(device_class(&outdoor), Some("temperature"));
    }

    #[wasm_test]
    fn a_state_class_or_a_numeric_device_class_also_makes_a_number() {
        assert_eq!(
            value(&sensor("7", &[(STATE_CLASS, "measurement")])),
            Some(Value::Number {
                value: 7.0,
                written_decimals: 0
            })
        );
        assert_eq!(
            value(&sensor("7", &[(DEVICE_CLASS, "aqi")])),
            Some(Value::Number {
                value: 7.0,
                written_decimals: 0
            })
        );
    }

    #[wasm_test]
    fn digits_without_a_unit_or_a_class_read_as_written() {
        assert_eq!(value(&sensor("42", &[])), Some(Value::Text("42")));
    }

    #[wasm_test]
    fn a_numeric_sensor_with_a_word_for_a_state_reads_as_written() {
        let outdoor = sensor("Calibrating", &[(UNIT, "°C")]);
        assert_eq!(value(&outdoor), Some(Value::Text("Calibrating")));
    }

    #[wasm_test]
    fn a_time_reads_as_a_timestamp_or_an_uptime_by_its_class() {
        let at = "2026-11-20T10:00:00+00:00";
        assert_eq!(
            value(&sensor(at, &[(DEVICE_CLASS, TIMESTAMP)])),
            Some(Value::Timestamp(at))
        );
        assert_eq!(
            value(&sensor(at, &[(DEVICE_CLASS, UPTIME)])),
            Some(Value::Uptime(at))
        );
    }

    #[wasm_test]
    fn an_empty_unit_is_none() {
        assert_eq!(unit(&sensor("7", &[(UNIT, "")])), None);
    }

    #[wasm_test]
    fn a_sensor_without_a_state_has_no_value() {
        for state in [UNKNOWN, UNAVAILABLE] {
            assert_eq!(value(&sensor(state, &[(UNIT, "°C")])), None, "{state}");
        }
    }
}
