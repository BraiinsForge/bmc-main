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
//! so this module only decides how many decimals a value gets and where its unit goes.

/// 2^53: past it, an `f64` misses whole numbers and holds no fraction digits.
pub(crate) const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;

/// The fewest decimals that show `value` to `max` places,
/// as the frontend's `maximumFractionDigits` does.
#[must_use]
pub fn trimmed_decimals(value: f64, max: u32) -> u32 {
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
#[must_use]
pub fn with_unit(mut number: String, unit: Option<&str>) -> String {
    if let Some(unit) = unit {
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
    fn a_value_without_digits_to_trim_gets_none() {
        for value in [f64::NAN, f64::INFINITY] {
            assert_eq!(trimmed_decimals(value, 3), 0, "{value}");
        }
    }

    #[wasm_test]
    fn a_unit_is_spaced_as_home_assistant_spaces_it() {
        assert_eq!(with_unit("21.5".to_owned(), Some("°C")), "21.5 °C");
        assert_eq!(with_unit("40".to_owned(), Some("%")), "40%");
        assert_eq!(with_unit("90".to_owned(), Some("°")), "90°");
        assert_eq!(with_unit("7".to_owned(), None), "7");
    }
}
