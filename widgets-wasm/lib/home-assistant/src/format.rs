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

//! Numbers as the Home Assistant frontend shows them.
//!
//! The widget writes the digits in the Deck's number format with the SDK,
//! so this module only reads a numeric state, decides how many decimals it gets,
//! and places its unit.

/// 2^53: past it, an `f64` misses whole numbers and holds no fraction digits.
pub(crate) const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;
/// The most decimals Home Assistant's entity settings offer for a display precision,
/// and the most a state is shown with as it was written.
const MAX_PRECISION: u32 = 6;

/// A numeric state. Home Assistant writes one as a Python float or int.
///
/// It only parses: whether the frontend shows a state as a number is [`is_numeric`]'s call.
#[must_use]
pub fn state_number(state: &str) -> Option<f64> {
    state.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// The sensor device classes the frontend shows as numbers: its `SENSOR_NUMERIC_DEVICE_CLASSES`
/// (`src/data/sensor_entity_constants.ts`, generated from core: every `SensorDeviceClass`
/// but its `NON_NUMERIC_DEVICE_CLASSES`). Sorted, for the binary search.
const SENSOR_NUMERIC_DEVICE_CLASSES: &[&str] = &[
    "absolute_humidity",
    "apparent_power",
    "aqi",
    "area",
    "atmospheric_pressure",
    "battery",
    "blood_glucose_concentration",
    "carbon_dioxide",
    "carbon_monoxide",
    "conductivity",
    "current",
    "data_rate",
    "data_size",
    "distance",
    "duration",
    "energy",
    "energy_distance",
    "energy_storage",
    "frequency",
    "gas",
    "humidity",
    "illuminance",
    "irradiance",
    "moisture",
    "monetary",
    "nitrogen_dioxide",
    "nitrogen_monoxide",
    "nitrous_oxide",
    "ozone",
    "ph",
    "pm1",
    "pm10",
    "pm25",
    "pm4",
    "power",
    "power_factor",
    "precipitation",
    "precipitation_intensity",
    "pressure",
    "radon",
    "reactive_energy",
    "reactive_power",
    "signal_strength",
    "sound_pressure",
    "speed",
    "sulphur_dioxide",
    "temperature",
    "temperature_delta",
    "volatile_organic_compounds",
    "volatile_organic_compounds_parts",
    "voltage",
    "volume",
    "volume_flow_rate",
    "volume_storage",
    "water",
    "weight",
    "wind_direction",
    "wind_speed",
];

/// Whether the frontend shows an entity's state as a number rather than as written,
/// as its `computeStateDisplay` decides: by a unit or a state class, by the domain,
/// or for a sensor by its device class.
///
/// A device class missing from the frontend's `SENSOR_NUMERIC_DEVICE_CLASSES`, such as one Home Assistant
/// adds later, reads as text, as in the frontend until it lists the class.
#[must_use]
pub fn is_numeric(
    domain: &str,
    unit: Option<&str>,
    state_class: Option<&str>,
    device_class: Option<&str>,
) -> bool {
    let set = |value: Option<&str>| value.is_some_and(|value| !value.is_empty());
    set(unit)
        || set(state_class)
        || matches!(domain, "counter" | "input_number" | "number")
        || domain == "sensor"
            && device_class
                .is_some_and(|class| SENSOR_NUMERIC_DEVICE_CLASSES.binary_search(&class).is_ok())
}

/// The decimals the frontend shows a sensor's `state` with:
/// its display `precision`, else as many as the state was written with.
/// Past six places the value is rounded to six, trailing zeros trimmed.
#[must_use]
pub fn sensor_decimals(state: &str, precision: Option<u32>) -> u32 {
    precision.unwrap_or_else(|| written_decimals(state))
}

/// What follows a state's `.`, as the frontend counts it, an exponent included.
fn written_decimals(state: &str) -> u32 {
    let Some((_, fraction)) = state.split_once('.') else {
        return 0;
    };
    match u32::try_from(fraction.len()) {
        Ok(digits) if digits <= MAX_PRECISION => digits,
        _ => state_number(state).map_or(0, |value| trimmed_decimals(value, MAX_PRECISION)),
    }
}

/// Home Assistant's display precision: the user's choice, then the integration's suggestion.
#[must_use]
pub(crate) fn precision(display: Option<f64>, suggested: Option<f64>) -> Option<u32> {
    display
        .and_then(precision_decimals)
        .or_else(|| suggested.and_then(precision_decimals))
}

fn precision_decimals(precision: f64) -> Option<u32> {
    if precision.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&precision) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked above to be a whole number within u32"
    )]
    let decimals = precision as u32;
    Some(decimals.min(MAX_PRECISION))
}

/// The fewest decimals that show `value` to `max` places,
/// as the frontend's `maximumFractionDigits` does.
/// `max` is capped at six places, the most a display precision offers.
#[must_use]
pub fn trimmed_decimals(value: f64, max: u32) -> u32 {
    let max = max.min(MAX_PRECISION);
    let mut scaled = (value.abs() * 10_f64.powi(max.cast_signed())).round();
    if scaled.is_nan() || scaled >= MAX_EXACT_INTEGER {
        return 0;
    }
    let mut decimals = max;
    while decimals > 0 && scaled % 10.0 == 0.0 {
        scaled /= 10.0;
        decimals -= 1;
    }
    decimals
}

/// `number` followed by `unit`, spaced as the Home Assistant frontend does in English.
/// An empty `unit` counts as none, as the frontend's does.
#[must_use]
pub fn with_unit(mut number: String, unit: Option<&str>) -> String {
    if let Some(unit) = unit.filter(|unit| !unit.is_empty()) {
        if unit != "%" && unit != "°" {
            number.push(' ');
        }
        number.push_str(unit);
    }
    number
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn a_value_is_trimmed_to_the_places_it_needs() {
        let cases = [
            (21.456_789, 3, 3),
            (21.45, 3, 2),
            (21.0, 3, 0),
            (0.0004, 3, 0),
            (0.0005, 3, 3),
            (-3.10, 3, 1),
            (21.46, 1, 1),
            (21.04, 1, 0),
            (1e20, 3, 0),
        ];
        for (value, max, decimals) in cases {
            assert_eq!(trimmed_decimals(value, max), decimals, "{value} to {max}");
        }
    }

    #[wasm_test]
    fn more_places_than_a_precision_offers_are_capped_at_six() {
        for max in [7, 20, u32::MAX] {
            assert_eq!(trimmed_decimals(1.5, max), 1, "1.5 to {max}");
            assert_eq!(
                trimmed_decimals(0.123_456_789, max),
                6,
                "0.123456789 to {max}"
            );
        }
    }

    #[wasm_test]
    fn a_value_without_digits_to_trim_gets_none() {
        for value in [f64::NAN, f64::INFINITY] {
            assert_eq!(trimmed_decimals(value, 3), 0, "{value}");
        }
    }

    #[wasm_test]
    fn a_numeric_state_is_a_finite_number() {
        assert_eq!(state_number("21.5"), Some(21.5));
        assert_eq!(state_number("-3"), Some(-3.0));
        assert_eq!(state_number("1e20"), Some(1e20));
        for state in ["unknown", "unavailable", "nan", "inf", "", "21.5 °C"] {
            assert_eq!(state_number(state), None, "{state:?}");
        }
    }

    #[wasm_test]
    fn a_sensor_shows_its_precision_or_the_places_it_was_written_with() {
        assert_eq!(sensor_decimals("21.456789", Some(1)), 1);
        assert_eq!(
            sensor_decimals("21", Some(2)),
            2,
            "a precision is not trimmed"
        );
        let cases = [
            ("0.0004", 4),
            ("21.50", 2),
            ("21.456789", 6),
            ("21", 0),
            ("1e-05", 0),
            ("1.5e-05", 5),
        ];
        for (state, decimals) in cases {
            assert_eq!(sensor_decimals(state, None), decimals, "{state}");
        }
    }

    #[wasm_test]
    fn a_state_written_past_six_places_is_rounded_to_six_and_trimmed() {
        assert_eq!(sensor_decimals("0.30000000000000004", None), 1);
        assert_eq!(sensor_decimals("21.4567891", None), 6);
    }

    #[wasm_test]
    fn a_state_is_numeric_by_unit_state_class_domain_or_device_class() {
        assert!(is_numeric("sensor", Some("°C"), None, None));
        assert!(is_numeric("sensor", None, Some("measurement"), None));
        assert!(is_numeric("input_number", None, None, None));
        assert!(is_numeric("sensor", None, None, Some("temperature")));
        assert!(is_numeric("sensor", None, None, Some("monetary")));
        for device_class in ["date", "enum", "timestamp", "uptime", "not_yet_defined"] {
            assert!(
                !is_numeric("sensor", None, None, Some(device_class)),
                "{device_class}"
            );
        }
        assert!(
            !is_numeric("sensor", Some(""), None, None),
            "a text sensor reading 10001 stays as written"
        );
        assert!(!is_numeric("binary_sensor", None, None, Some("door")));
    }

    #[wasm_test]
    fn the_numeric_device_classes_stay_sorted_for_the_search() {
        assert!(SENSOR_NUMERIC_DEVICE_CLASSES.is_sorted());
    }

    #[wasm_test]
    fn the_users_precision_comes_before_the_integrations() {
        assert_eq!(precision(Some(2.0), Some(1.0)), Some(2));
        assert_eq!(precision(None, Some(1.0)), Some(1));
        assert_eq!(precision(None, None), None);
    }

    #[wasm_test]
    fn a_malformed_precision_is_skipped() {
        assert_eq!(precision(Some(-1.0), Some(1.0)), Some(1));
        assert_eq!(precision(Some(1.5), None), None);
    }

    #[wasm_test]
    fn a_precision_is_capped_at_what_home_assistant_offers() {
        assert_eq!(precision(Some(12.0), None), Some(MAX_PRECISION));
    }

    #[wasm_test]
    fn a_unit_is_spaced_as_home_assistant_spaces_it() {
        assert_eq!(with_unit("21.5".to_owned(), Some("°C")), "21.5 °C");
        assert_eq!(with_unit("40".to_owned(), Some("%")), "40%");
        assert_eq!(with_unit("90".to_owned(), Some("°")), "90°");
        assert_eq!(with_unit("7".to_owned(), None), "7");
        assert_eq!(
            with_unit("7".to_owned(), Some("")),
            "7",
            "an empty unit is none"
        );
    }
}
