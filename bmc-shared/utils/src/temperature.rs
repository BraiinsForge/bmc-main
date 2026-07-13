// Copyright (C) 2025  Braiins Systems s.r.o.
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

use serde::{Deserialize, Serialize};

use crate::number_format::NumberFormat;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

impl TemperatureUnit {
    /// Convert a Celsius value into this unit.
    #[must_use]
    pub fn convert(self, celsius: f64) -> f64 {
        match self {
            TemperatureUnit::Celsius => celsius,
            TemperatureUnit::Fahrenheit => celsius * 9.0 / 5.0 + 32.0,
        }
    }

    /// The scale letter for this unit (`"C"` / `"F"`).
    #[must_use]
    pub fn scale(self) -> &'static str {
        match self {
            TemperatureUnit::Celsius => "C",
            TemperatureUnit::Fahrenheit => "F",
        }
    }

    /// With `show_unit`, spells out the scale (`20 °C`); without, a bare degree (`20°`).
    #[must_use]
    pub fn format(
        self,
        number_format: NumberFormat,
        celsius: f64,
        precision: usize,
        show_unit: bool,
    ) -> String {
        let num = number_format.format_number(self.convert(celsius), precision);
        if show_unit {
            format!("{num} \u{00b0}{}", self.scale())
        } else {
            format!("{num}\u{00b0}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn celsius_keeps_the_value_and_scale() {
        let f = TemperatureUnit::Celsius.format(NumberFormat::SpaceGroupDotDecimal, 20.0, 0, true);
        assert_eq!(f, "20 \u{00b0}C");
    }

    #[test]
    fn fahrenheit_converts_from_celsius() {
        let f =
            TemperatureUnit::Fahrenheit.format(NumberFormat::SpaceGroupDotDecimal, 20.0, 0, true);
        assert_eq!(f, "68 \u{00b0}F");
    }

    #[test]
    fn bare_mode_drops_the_scale_letter_and_space() {
        let f = TemperatureUnit::Celsius.format(NumberFormat::SpaceGroupDotDecimal, 26.0, 0, false);
        assert_eq!(f, "26\u{00b0}");
    }
}
