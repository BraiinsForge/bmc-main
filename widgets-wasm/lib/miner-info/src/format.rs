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

use core::time::Duration;

use bmc_wasm_sdk::{ElectricPower, Hashrate, Hashvalue, MiningEfficiency, Ratio};

use crate::model::{Availability, Money, TemperatureRange};

pub const NOT_AVAILABLE: &str = "N/A";

#[must_use]
pub(crate) fn unavailable() -> String {
    NOT_AVAILABLE.to_owned()
}

/// A value and the unit it reads in, kept apart
/// so a face can size the two differently.
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

/// What these faces need of a quantity in order to render it:
/// the number to print and the unit to print after it.
///
/// The SDK quantities each expose their own `as_*` accessor rather
/// than a common trait, so this names the one the faces read.
///
/// `magnitude` is the value *as displayed*,
/// which is why [`Ratio`] answers in percent.
pub trait Measured: Copy {
    const UNIT: &'static str;
    fn magnitude(self) -> f64;
}

impl Measured for Hashrate {
    const UNIT: &'static str = Self::UNIT;
    fn magnitude(self) -> f64 {
        self.as_terahashes_per_second()
    }
}

impl Measured for ElectricPower {
    const UNIT: &'static str = Self::UNIT;
    fn magnitude(self) -> f64 {
        self.as_watts()
    }
}

impl Measured for MiningEfficiency {
    const UNIT: &'static str = Self::UNIT;
    fn magnitude(self) -> f64 {
        self.as_joules_per_terahash()
    }
}

impl Measured for Ratio {
    const UNIT: &'static str = Self::UNIT;
    fn magnitude(self) -> f64 {
        self.as_percent()
    }
}

impl Measured for Hashvalue {
    const UNIT: &'static str = Self::UNIT;
    fn magnitude(self) -> f64 {
        self.as_satoshis_per_terahash_day()
    }
}

pub(crate) fn push_int(out: &mut String, value: u64) {
    if value >= 10 {
        push_int(out, value.div_euclid(10));
    }
    out.push(char::from(
        b'0' + u8::try_from(value.rem_euclid(10)).expect("BUG: decimal digit fits u8"),
    ));
}

pub(crate) fn push_fixed_abs(out: &mut String, value: f64, decimals: u32) {
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

/// Digit grouping and decimal mark as the operator configured them.
#[must_use]
pub(crate) fn group(magnitude: f64, decimals: u32) -> String {
    bmc_wasm_sdk::format_number!(magnitude, decimals)
}

#[must_use]
pub(crate) fn fixed<Q: Measured>(value: Availability<Q>, decimals: u32) -> Rendered {
    match value {
        Availability::Available(value) => {
            let value = value.magnitude();
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
        // A number has nothing to say about why it is missing;
        // a screen that wants to distinguish the two reads the state, not this string.
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

#[must_use]
pub(crate) fn fixed_strip_zero_fraction<Q: Measured>(
    value: Availability<Q>,
    decimals: u32,
) -> Rendered {
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

/// Scale the miner reports board and chip temperature in.
/// The pair renders as one `61-74 °C` reading, so the unit
/// is appended once rather than read off either half.
const UNIT_CELSIUS: &str = "°C";

#[must_use]
pub(crate) fn approx_fixed<Q: Measured>(value: Availability<Q>, decimals: u32) -> Rendered {
    match value {
        Availability::Available(_) => {
            let mut out = String::from("~ ");
            out.push_str(&fixed(value, decimals).value);
            Rendered {
                value: out,
                unit: Some(Q::UNIT),
            }
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

#[must_use]
pub(crate) fn signed_percent<Q: Measured>(value: Availability<Q>, decimals: u32) -> Rendered {
    match value {
        Availability::Available(value) => {
            let value = value.magnitude();
            let mut out = String::new();
            out.push(if value >= 0.0 { '+' } else { '-' });
            out.push_str(&group(value.abs(), decimals));
            Rendered {
                value: out,
                unit: Some(Q::UNIT),
            }
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

#[must_use]
pub(crate) fn signed_percent_unit<Q: Measured>(value: Availability<Q>, decimals: u32) -> String {
    let Availability::Available(_) = value else {
        return unavailable();
    };
    let mut out = signed_percent(value, decimals).value;
    out.push('%');
    out
}

#[must_use]
pub(crate) fn temperature(value: Availability<TemperatureRange>) -> Rendered {
    match value {
        Availability::Available(value) => {
            let mut out = String::new();
            push_fixed_abs(&mut out, value.board.as_celsius(), 0);
            out.push('-');
            push_fixed_abs(&mut out, value.chip.as_celsius(), 0);
            Rendered {
                value: out,
                unit: Some(UNIT_CELSIUS),
            }
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

#[must_use]
pub(crate) fn chip_temperature(value: Availability<TemperatureRange>) -> Rendered {
    match value {
        Availability::Available(value) => {
            let mut out = String::new();
            push_fixed_abs(&mut out, value.chip.as_celsius(), 0);
            Rendered {
                value: out,
                unit: Some(UNIT_CELSIUS),
            }
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

#[must_use]
pub(crate) fn money(value: Availability<Money>, decimals: u32) -> Rendered {
    match value {
        Availability::Available(money) => {
            let mut out = String::from(money.currency.symbol());
            out.push(' ');
            out.push_str(&group(money.amount.abs(), decimals));
            out.into()
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

// Currency symbol on its own, for layouts that render the symbol at a smaller
// size than the amount (round clusters). `None` when the value is unavailable,
// so the caller omits the symbol element entirely.
#[must_use]
pub(crate) fn money_symbol(value: Availability<Money>) -> Option<&'static str> {
    match value {
        Availability::Available(money) => Some(money.currency.symbol()),
        Availability::Unavailable | Availability::Failed => None,
    }
}

// The grouped amount without the currency symbol, the companion to `money_symbol`.
#[must_use]
pub(crate) fn money_amount(value: Availability<Money>, decimals: u32) -> String {
    match value {
        Availability::Available(money) => group(money.amount.abs(), decimals),
        Availability::Unavailable | Availability::Failed => unavailable(),
    }
}

#[must_use]
pub(crate) fn public_integer(value: Availability<u64>) -> Rendered {
    match value {
        Availability::Available(value) => {
            #[expect(
                clippy::cast_precision_loss,
                reason = "block height stays well within f64's exact integer range"
            )]
            let magnitude = value as f64;
            group(magnitude, 0).into()
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

#[must_use]
pub(crate) fn uptime(value: Availability<Duration>) -> Rendered {
    let Availability::Available(total) = value else {
        return unavailable().into();
    };
    let total = total.as_secs();
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let minutes = (total % 3_600) / 60;
    let mut out = String::new();
    if days > 0 {
        push_int(&mut out, days);
        out.push_str("d ");
        push_int(&mut out, hours);
        out.push_str("h ");
        push_int(&mut out, minutes);
        out.push('m');
    } else if hours > 0 {
        push_int(&mut out, hours);
        out.push_str("h ");
        push_int(&mut out, minutes);
        out.push('m');
    } else {
        push_int(&mut out, minutes);
        out.push('m');
    }
    out.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::Temperature;

    #[test]
    fn formats_temperature_range_like_boser() {
        let range = TemperatureRange {
            board: Temperature::from_celsius(61.2),
            chip: Temperature::from_celsius(74.4),
        };
        assert_eq!(temperature(Availability::Available(range)).value, "61-74");
    }

    #[test]
    fn formats_chip_temperature_as_single_value() {
        let range = TemperatureRange {
            board: Temperature::from_celsius(61.2),
            chip: Temperature::from_celsius(74.4),
        };
        assert_eq!(chip_temperature(Availability::Available(range)).value, "74");
        assert_eq!(chip_temperature(Availability::Unavailable).value, "N/A");
    }

    #[test]
    fn formats_signed_percent_with_explicit_sign() {
        assert_eq!(
            signed_percent(Availability::Available(Ratio::from_percent(1.82)), 2).value,
            "+1,82"
        );
        assert_eq!(
            signed_percent(Availability::Available(Ratio::from_percent(-0.77)), 2).value,
            "-0,77"
        );
    }

    #[test]
    fn signed_percent_unit_omits_percent_when_unavailable() {
        assert_eq!(
            signed_percent_unit(Availability::Available(Ratio::from_percent(1.82)), 2),
            "+1,82%"
        );
        assert_eq!(
            signed_percent_unit(Availability::<Ratio>::Unavailable, 2),
            "N/A"
        );
    }

    #[test]
    fn formats_uptime_compactly() {
        assert_eq!(
            uptime(Availability::Available(Duration::from_secs(187_020))).value,
            "2d 3h 57m"
        );
        assert_eq!(uptime(Availability::Unavailable).value, "N/A");
    }

    #[test]
    fn formats_currency_symbol() {
        let usd = Money::new(104_250.4, crate::model::Currency::Usd);
        assert_eq!(
            money(Availability::Available(usd), 0).value,
            "$ 104\u{a0}250"
        );
    }

    #[test]
    fn splits_money_into_symbol_and_amount() {
        let usd = Money::new(104_250.4, crate::model::Currency::Usd);
        assert_eq!(money_symbol(Availability::Available(usd)), Some("$"));
        assert_eq!(
            money_amount(Availability::Available(usd), 0),
            "104\u{a0}250"
        );
        assert_eq!(money_symbol(Availability::Unavailable), None);
        assert_eq!(money_amount(Availability::Unavailable, 0), "N/A");
    }

    #[test]
    fn formats_approximate_fixed_value_like_boser() {
        assert_eq!(
            approx_fixed(Availability::Available(Ratio::from_percent(0.1234)), 3).value,
            "~ 0,123"
        );
        assert_eq!(
            approx_fixed(Availability::<Ratio>::Unavailable, 3).value,
            "N/A"
        );
    }

    #[test]
    fn unavailable_public_integer_reads_not_available() {
        assert_eq!(
            public_integer(Availability::Available(870_123)).value,
            "870\u{a0}123"
        );
        assert_eq!(public_integer(Availability::Unavailable).value, "N/A");
    }

    #[test]
    fn temperature_range_unit_is_celsius() {
        let range = TemperatureRange {
            board: Temperature::from_celsius(61.0),
            chip: Temperature::from_celsius(74.0),
        };
        assert_eq!(temperature(Availability::Available(range)).unit, Some("°C"));
    }
}
