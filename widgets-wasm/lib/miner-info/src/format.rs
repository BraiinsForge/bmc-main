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
use std::borrow::Cow;

use bmc_wasm_sdk::types::{
    BitcoinAmount, ElectricPower, Hashrate, Hashvalue, MiningEfficiency, Ratio,
};

use crate::model::{Availability, Money, TemperatureRange};

pub const NOT_AVAILABLE: &str = "N/A";

#[must_use]
pub(crate) fn unavailable() -> String {
    NOT_AVAILABLE.to_owned()
}

/// A value and the unit it reads in, kept apart
/// so a face can size the two differently.
/// The unit is owned only where an SI prefix picked it at render time.
#[derive(Debug)]
pub struct Rendered {
    pub value: String,
    pub unit: Option<Cow<'static, str>>,
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
                unit: Some(Q::UNIT.into()),
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
                unit: Some(Q::UNIT.into()),
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
                unit: Some(Q::UNIT.into()),
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
                unit: Some(UNIT_CELSIUS.into()),
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
                unit: Some(UNIT_CELSIUS.into()),
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

/// Significant digits of a network-wide hashrate, `650 EH/s` or `1.04 ZH/s`.
const NETWORK_HASHRATE_SIG_FIGS: u32 = 3;

/// SI-prefixed: the unit is picked per value, not fixed by the type.
#[must_use]
pub(crate) fn network_hashrate(value: Availability<Hashrate>) -> Rendered {
    match value {
        Availability::Available(rate) => {
            let (value, unit) = rate.format_si_parts(NETWORK_HASHRATE_SIG_FIGS);
            Rendered {
                value,
                unit: Some(unit.into()),
            }
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

/// An average is an estimate, so it drops the satoshi precision an amount keeps.
const FEE_AMOUNT_DECIMALS: u32 = 3;

/// `~ 0.055 BTC | 12.1%`, or whichever half is known.
#[must_use]
pub(crate) fn fees(amount: Availability<BitcoinAmount>, share: Availability<Ratio>) -> Rendered {
    let mut parts: Vec<String> = Vec::with_capacity(2);
    if let Availability::Available(amount) = amount {
        let mut part = String::from("~ ");
        part.push_str(&group(amount.as_bitcoin(), FEE_AMOUNT_DECIMALS));
        part.push(' ');
        part.push_str(BitcoinAmount::UNIT);
        parts.push(part);
    }
    if let Availability::Available(share) = share {
        let mut part = group(share.as_percent(), 1);
        part.push_str(Ratio::UNIT);
        parts.push(part);
    }
    if parts.is_empty() {
        return unavailable().into();
    }
    parts.join(" | ").into()
}

const SECS_PER_MINUTE: u64 = 60;
const SECS_PER_HOUR: u64 = 60 * SECS_PER_MINUTE;
const SECS_PER_DAY: u64 = 24 * SECS_PER_HOUR;

/// How far off the retarget is, `in ~ 3 days`,
/// in the coarsest unit that still rounds to at least one.
/// Rounding comes before the unit, so 23 h 50 min reads as a day, not 24 hours.
#[must_use]
pub(crate) fn epoch_eta(remaining: Duration) -> String {
    let secs = remaining.as_secs();
    let rounded = |unit: u64| secs.saturating_add(unit / 2) / unit;
    let (count, unit) = [
        (rounded(SECS_PER_DAY), "day"),
        (rounded(SECS_PER_HOUR), "hour"),
        (rounded(SECS_PER_MINUTE), "minute"),
    ]
    .into_iter()
    .find(|(count, _)| *count >= 1)
    .unwrap_or((1, "minute"));
    let plural = if count == 1 { "" } else { "s" };
    let mut out = String::from("in ~ ");
    push_int(&mut out, count);
    out.push(' ');
    out.push_str(unit);
    out.push_str(plural);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::types::{SiPrefix, Temperature};
    use bmc_wasm_sdk::typography::NBSP;

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
            format!("$ 104{NBSP}250")
        );
    }

    #[test]
    fn splits_money_into_symbol_and_amount() {
        let usd = Money::new(104_250.4, crate::model::Currency::Usd);
        assert_eq!(money_symbol(Availability::Available(usd)), Some("$"));
        assert_eq!(
            money_amount(Availability::Available(usd), 0),
            format!("104{NBSP}250")
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
            format!("870{NBSP}123")
        );
        assert_eq!(public_integer(Availability::Unavailable).value, "N/A");
    }

    #[test]
    fn temperature_range_unit_is_celsius() {
        let range = TemperatureRange {
            board: Temperature::from_celsius(61.0),
            chip: Temperature::from_celsius(74.0),
        };
        assert_eq!(
            temperature(Availability::Available(range)).unit.as_deref(),
            Some("°C")
        );
    }

    #[test]
    fn network_hashrate_picks_its_prefix_per_value() {
        let exa = network_hashrate(Availability::Available(Hashrate::from_si(
            650.0,
            SiPrefix::Exa,
        )));
        assert_eq!(
            (exa.value.as_str(), exa.unit.as_deref()),
            ("650", Some("EH/s"))
        );
        let zetta = network_hashrate(Availability::Available(Hashrate::from_si(
            1_036.15,
            SiPrefix::Exa,
        )));
        assert_eq!(
            (zetta.value.as_str(), zetta.unit.as_deref()),
            ("1,04", Some("ZH/s"))
        );
        assert_eq!(network_hashrate(Availability::Unavailable).value, "N/A");
    }

    #[test]
    fn fees_read_as_amount_and_share_or_whichever_is_known() {
        let amount = Availability::Available(BitcoinAmount::from_bitcoin(0.055));
        let share = Availability::Available(Ratio::from_percent(12.1));
        assert_eq!(fees(amount, share).value, "~ 0,055 BTC | 12,1%");
        assert_eq!(fees(amount, Availability::Unavailable).value, "~ 0,055 BTC");
        assert_eq!(fees(Availability::Unavailable, share).value, "12,1%");
        assert_eq!(
            fees(Availability::Unavailable, Availability::Unavailable).value,
            "N/A"
        );
    }

    #[test]
    fn epoch_eta_rounds_to_the_coarsest_unit_that_still_counts_one() {
        let eta = |secs| epoch_eta(Duration::from_secs(secs));
        assert_eq!(eta(262 * 600), "in ~ 2 days");
        assert_eq!(eta(SECS_PER_DAY + SECS_PER_HOUR), "in ~ 1 day");
        assert_eq!(
            eta(5 * SECS_PER_HOUR + 40 * SECS_PER_MINUTE),
            "in ~ 6 hours"
        );
        assert_eq!(eta(SECS_PER_HOUR), "in ~ 1 hour");
        assert_eq!(eta(20 * SECS_PER_MINUTE), "in ~ 20 minutes");
        assert_eq!(eta(0), "in ~ 1 minute");
    }

    /// A value within half a unit of the next one rounds into it
    /// rather than reading as 24 hours or 60 minutes.
    #[test]
    fn epoch_eta_promotes_what_rounds_past_a_unit() {
        let eta = |secs| epoch_eta(Duration::from_secs(secs));
        assert_eq!(eta(23 * SECS_PER_HOUR + 50 * SECS_PER_MINUTE), "in ~ 1 day");
        assert_eq!(eta(59 * SECS_PER_MINUTE + 40), "in ~ 1 hour");
        assert_eq!(
            eta(11 * SECS_PER_HOUR + 59 * SECS_PER_MINUTE),
            "in ~ 12 hours"
        );
        assert_eq!(eta(29 * SECS_PER_MINUTE + 59), "in ~ 30 minutes");
    }
}
