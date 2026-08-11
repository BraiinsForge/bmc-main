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

use crate::model::{Availability, Currency, Money, TemperatureRange};
#[cfg(target_arch = "wasm32")]
pub(crate) use units::format::{NOT_AVAILABLE, fixed_strip_zero_fraction};
pub(crate) use units::format::{Rendered, fixed, unavailable};
use units::format::{group, push_fixed_abs, push_int};
use units::units::{DegreeCelsius, Quantity, Seconds};

pub(crate) fn approx_fixed<Q: Quantity>(value: Availability<Q>, decimals: u32) -> Rendered {
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

pub(crate) fn signed_percent<Q: Quantity>(value: Availability<Q>, decimals: u32) -> Rendered {
    match value {
        Availability::Available(value) => {
            let value = value.raw();
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

pub(crate) fn signed_percent_unit<Q: Quantity>(value: Availability<Q>, decimals: u32) -> String {
    let Availability::Available(_) = value else {
        return unavailable();
    };
    let mut out = signed_percent(value, decimals).value;
    out.push('%');
    out
}

pub(crate) fn temperature(value: Availability<TemperatureRange>) -> Rendered {
    match value {
        Availability::Available(value) => {
            let mut out = String::new();
            push_fixed_abs(&mut out, value.board.raw(), 0);
            out.push('-');
            push_fixed_abs(&mut out, value.chip.raw(), 0);
            Rendered {
                value: out,
                unit: Some(DegreeCelsius::UNIT),
            }
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

pub(crate) fn chip_temperature(value: Availability<TemperatureRange>) -> Rendered {
    match value {
        Availability::Available(value) => {
            let mut out = String::new();
            push_fixed_abs(&mut out, value.chip.raw(), 0);
            Rendered {
                value: out,
                unit: Some(DegreeCelsius::UNIT),
            }
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

pub(crate) fn money(value: Availability<Money>, decimals: u32) -> Rendered {
    match value {
        Availability::Available(Money { currency, value }) => {
            let symbol = match currency {
                Currency::Usd => "$",
                Currency::Eur => "€",
            };
            let mut out = String::from(symbol);
            out.push(' ');
            out.push_str(&group(value.abs(), decimals));
            out.into()
        }
        Availability::Unavailable | Availability::Failed => unavailable().into(),
    }
}

// Currency symbol on its own, for layouts that render the symbol at a smaller
// size than the amount (round clusters). `None` when the value is unavailable
// or carries no symbol, so the caller omits the symbol element entirely.
pub(crate) fn money_symbol(value: Availability<Money>) -> Option<&'static str> {
    match value {
        Availability::Available(Money { currency, .. }) => Some(match currency {
            Currency::Usd => "$",
            Currency::Eur => "€",
        }),
        Availability::Unavailable | Availability::Failed => None,
    }
}

// The grouped amount without the currency symbol, the companion to
// `money_symbol`.
pub(crate) fn money_amount(value: Availability<Money>, decimals: u32) -> String {
    match value {
        Availability::Available(Money { value, .. }) => group(value.abs(), decimals),
        Availability::Unavailable | Availability::Failed => unavailable(),
    }
}

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

pub(crate) fn uptime(value: Availability<Seconds>) -> Rendered {
    let Availability::Available(total) = value else {
        return unavailable().into();
    };
    let total = total.raw();
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
    use units::units::{DegreeCelsius, Percent};

    #[test]
    fn formats_temperature_range_like_boser() {
        let range = TemperatureRange {
            board: DegreeCelsius(61.2),
            chip: DegreeCelsius(74.4),
        };
        assert_eq!(temperature(Availability::Available(range)).value, "61-74");
    }

    #[test]
    fn formats_chip_temperature_as_single_value() {
        let range = TemperatureRange {
            board: DegreeCelsius(61.2),
            chip: DegreeCelsius(74.4),
        };
        assert_eq!(chip_temperature(Availability::Available(range)).value, "74");
        assert_eq!(chip_temperature(Availability::Unavailable).value, "N/A");
    }

    #[test]
    fn formats_signed_percent_with_explicit_sign() {
        assert_eq!(
            signed_percent(Availability::Available(Percent(1.82)), 2).value,
            "+1,82"
        );
        assert_eq!(
            signed_percent(Availability::Available(Percent(-0.77)), 2).value,
            "-0,77"
        );
    }

    #[test]
    fn signed_percent_unit_omits_percent_when_unavailable() {
        assert_eq!(
            signed_percent_unit(Availability::Available(Percent(1.82)), 2),
            "+1,82%"
        );
        assert_eq!(
            signed_percent_unit(Availability::<Percent>::Unavailable, 2),
            "N/A"
        );
    }

    #[test]
    fn formats_uptime_compactly() {
        assert_eq!(
            uptime(Availability::Available(Seconds(187_020))).value,
            "2d 3h 57m"
        );
        assert_eq!(uptime(Availability::Unavailable).value, "N/A");
    }

    #[test]
    fn formats_currency_symbol() {
        let usd = Money {
            currency: Currency::Usd,
            value: 104_250.4,
        };
        assert_eq!(
            money(Availability::Available(usd), 0).value,
            "$ 104\u{a0}250"
        );
    }

    #[test]
    fn splits_money_into_symbol_and_amount() {
        let usd = Money {
            currency: Currency::Usd,
            value: 104_250.4,
        };
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
            approx_fixed(Availability::Available(Percent(0.1234)), 3).value,
            "~ 0,123"
        );
        assert_eq!(
            approx_fixed(Availability::<Percent>::Unavailable, 3).value,
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
            board: DegreeCelsius(61.0),
            chip: DegreeCelsius(74.0),
        };
        assert_eq!(temperature(Availability::Available(range)).unit, Some("°C"));
    }
}
