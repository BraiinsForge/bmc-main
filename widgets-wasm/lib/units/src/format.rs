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

use crate::availability::Availability;
use crate::units::{DegreeCelsius, KilometerPerHour, Quantity};

pub const NOT_AVAILABLE: &str = "N/A";

#[must_use]
pub fn unavailable() -> String {
    NOT_AVAILABLE.to_owned()
}

#[derive(Debug)]
pub struct Rendered {
    pub value: String,
    pub unit: Option<&'static str>,
}

impl From<String> for Rendered {
    fn from(value: String) -> Self {
        Self { value, unit: None }
    }
}

impl Rendered {
    #[must_use]
    pub fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = Some(unit);
        self
    }
}

pub fn push_int(out: &mut String, value: u64) {
    if value >= 10 {
        push_int(out, value.div_euclid(10));
    }
    out.push(char::from(
        b'0' + u8::try_from(value.rem_euclid(10)).expect("BUG: decimal digit fits u8"),
    ));
}

pub fn push_fixed_abs(out: &mut String, value: f64, decimals: u32) {
    let scale = 10_u64.pow(decimals);
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "fixed-point formatting of bounded, non-negative miner values"
    )]
    let scaled = (value.abs() * scale as f64).round() as u64;
    push_int(out, scaled.div_euclid(scale));
    if decimals == 0 {
        return;
    }
    out.push('.');
    let frac = scaled.rem_euclid(scale);
    let mut divisor = scale.div_euclid(10);
    while divisor > 0 {
        out.push(char::from(
            b'0' + u8::try_from(frac.div_euclid(divisor).rem_euclid(10))
                .expect("BUG: decimal digit fits u8"),
        ));
        divisor = divisor.div_euclid(10);
    }
}

#[must_use]
pub fn group(magnitude: f64, decimals: u32) -> String {
    bmc_wasm_sdk::format_number!(magnitude, decimals)
}

#[must_use]
pub fn temperature(value: DegreeCelsius, decimals: u32) -> String {
    bmc_wasm_sdk::format_temperature!(value.raw(), decimals)
}

/// Degree-only temperature ("26°", no scale letter) for the dense forecast
/// strips, where the unit would crowd the layout.
#[must_use]
pub fn temperature_bare(value: DegreeCelsius, decimals: u32) -> String {
    bmc_wasm_sdk::format_temperature!(value.raw(), decimals, bare)
}

#[must_use]
pub fn speed(value: KilometerPerHour, decimals: u32) -> String {
    bmc_wasm_sdk::format_speed!(value.raw(), decimals, ms)
}

#[must_use]
pub fn fixed<Q: Quantity>(value: Availability<Q>, decimals: u32) -> Rendered {
    match value {
        Availability::Available(value) => {
            let value = value.raw();
            let mut out = String::new();
            if value < 0.0 {
                out.push('-');
            }
            out.push_str(&group(value.abs(), decimals));
            Rendered {
                value: out,
                unit: Some(Q::UNIT),
            }
        }
        // A number has nothing to say about why it is missing; a screen that
        // wants to distinguish the two reads the state, not this string.
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

#[must_use]
pub fn fixed_strip_zero_fraction<Q: Quantity>(value: Availability<Q>, decimals: u32) -> Rendered {
    let mut out = fixed(value, decimals);
    if decimals == 0 || out.value == NOT_AVAILABLE {
        return out;
    }
    if out.value.ends_with(&"0".repeat(decimals as usize)) {
        out.value.truncate(out.value.len() - decimals as usize);
        out.value.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{Percent, Watt};

    #[test]
    fn temperature_appends_the_celsius_scale() {
        assert_eq!(temperature(DegreeCelsius(20.0), 0), "20 °C");
        assert_eq!(temperature(DegreeCelsius(-3.4), 0), "-3 °C");
    }

    #[test]
    fn temperature_bare_drops_the_scale_letter() {
        assert_eq!(temperature_bare(DegreeCelsius(26.0), 0), "26°");
    }

    #[test]
    fn speed_converts_kilometers_per_hour_to_meters_per_second() {
        assert_eq!(speed(KilometerPerHour(12.6), 1), "3,5 m/s");
    }

    #[test]
    fn fixed_sources_unit_from_the_quantity_type() {
        let rendered = fixed(Availability::Available(Watt(120.0)), 0);
        assert_eq!(rendered.value, "120");
        assert_eq!(rendered.unit, Some("W"));
    }

    #[test]
    fn unavailable_quantity_has_no_unit() {
        let rendered = fixed(Availability::<Watt>::Unavailable, 0);
        assert_eq!(rendered.value, "N/A");
        assert_eq!(rendered.unit, None);
    }

    #[test]
    fn strips_zero_fraction_from_fixed_value() {
        assert_eq!(
            fixed_strip_zero_fraction(Availability::Available(Percent(50.0)), 2).value,
            "50"
        );
        assert_eq!(
            fixed_strip_zero_fraction(Availability::Available(Percent(50.25)), 2).value,
            "50,25"
        );
        assert_eq!(
            fixed_strip_zero_fraction(Availability::<Percent>::Unavailable, 2).value,
            "N/A"
        );
    }
}
