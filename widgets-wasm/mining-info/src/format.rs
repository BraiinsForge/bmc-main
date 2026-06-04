// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::model::{Availability, Currency, Money, TemperatureRange};
use crate::units::{DegreeCelsius, Quantity, Seconds};

pub(crate) const NOT_AVAILABLE: &str = "N/A";

pub(crate) fn unavailable() -> String {
    NOT_AVAILABLE.to_owned()
}

pub(crate) struct Rendered {
    pub(crate) value: String,
    pub(crate) unit: Option<&'static str>,
}

impl From<String> for Rendered {
    fn from(value: String) -> Self {
        Self { value, unit: None }
    }
}

impl Rendered {
    pub(crate) fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = Some(unit);
        self
    }
}

fn push_int(out: &mut String, value: u64) {
    if value >= 10 {
        push_int(out, value / 10);
    }
    out.push(char::from(
        b'0' + u8::try_from(value % 10).expect("BUG: decimal digit fits u8"),
    ));
}

fn push_fixed_abs(out: &mut String, value: f64, decimals: u32) {
    let scale = 10_u64.pow(decimals);
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "fixed-point formatting of bounded, non-negative miner values"
    )]
    let scaled = (value.abs() * scale as f64).round() as u64;
    push_int(out, scaled / scale);
    if decimals == 0 {
        return;
    }
    out.push('.');
    let frac = scaled % scale;
    let mut divisor = scale / 10;
    while divisor > 0 {
        out.push(char::from(
            b'0' + u8::try_from((frac / divisor) % 10).expect("BUG: decimal digit fits u8"),
        ));
        divisor /= 10;
    }
}

// Group separators and the decimal mark come from the device `number_format`
// system setting via the host. The host path is wasm-only; the non-wasm
// fallback keeps the magnitude deterministic so the surrounding composition
// (sign, symbol, unit) stays unit-testable.
#[cfg(target_arch = "wasm32")]
fn group(magnitude: f64, decimals: u32) -> String {
    bmc_wasm_sdk::format_number!(magnitude, decimals)
}

#[cfg(not(target_arch = "wasm32"))]
fn group(magnitude: f64, decimals: u32) -> String {
    let mut out = String::new();
    push_fixed_abs(&mut out, magnitude, decimals);
    out
}

pub(crate) fn fixed<Q: Quantity>(value: Availability<Q>, decimals: u32) -> Rendered {
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
        Availability::Unavailable => unavailable().into(),
    }
}

pub(crate) fn fixed_strip_zero_fraction<Q: Quantity>(
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
        Availability::Unavailable => unavailable().into(),
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
        Availability::Unavailable => unavailable().into(),
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
        Availability::Unavailable => unavailable().into(),
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
        Availability::Unavailable => unavailable().into(),
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
        Availability::Unavailable => unavailable().into(),
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
        Availability::Unavailable => None,
    }
}

// The grouped amount without the currency symbol, the companion to
// `money_symbol`.
pub(crate) fn money_amount(value: Availability<Money>, decimals: u32) -> String {
    match value {
        Availability::Available(Money { value, .. }) => group(value.abs(), decimals),
        Availability::Unavailable => unavailable(),
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
        Availability::Unavailable => unavailable().into(),
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
    use crate::units::{DegreeCelsius, Percent, Watt};

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
            "+1.82"
        );
        assert_eq!(
            signed_percent(Availability::Available(Percent(-0.77)), 2).value,
            "-0.77"
        );
    }

    #[test]
    fn signed_percent_unit_omits_percent_when_unavailable() {
        assert_eq!(
            signed_percent_unit(Availability::Available(Percent(1.82)), 2),
            "+1.82%"
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
        assert_eq!(money(Availability::Available(usd), 0).value, "$ 104250");
    }

    #[test]
    fn splits_money_into_symbol_and_amount() {
        let usd = Money {
            currency: Currency::Usd,
            value: 104_250.4,
        };
        assert_eq!(money_symbol(Availability::Available(usd)), Some("$"));
        assert_eq!(money_amount(Availability::Available(usd), 0), "104250");
        assert_eq!(money_symbol(Availability::Unavailable), None);
        assert_eq!(money_amount(Availability::Unavailable, 0), "N/A");
    }

    #[test]
    fn strips_zero_fraction_from_fixed_value() {
        assert_eq!(
            fixed_strip_zero_fraction(Availability::Available(Percent(50.0)), 2).value,
            "50"
        );
        assert_eq!(
            fixed_strip_zero_fraction(Availability::Available(Percent(50.25)), 2).value,
            "50.25"
        );
        assert_eq!(
            fixed_strip_zero_fraction(Availability::<Percent>::Unavailable, 2).value,
            "N/A"
        );
    }

    #[test]
    fn formats_approximate_fixed_value_like_boser() {
        assert_eq!(
            approx_fixed(Availability::Available(Percent(0.1234)), 3).value,
            "~ 0.123"
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
            "870123"
        );
        assert_eq!(public_integer(Availability::Unavailable).value, "N/A");
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
    fn temperature_range_unit_is_celsius() {
        let range = TemperatureRange {
            board: DegreeCelsius(61.0),
            chip: DegreeCelsius(74.0),
        };
        assert_eq!(temperature(Availability::Available(range)).unit, Some("°C"));
    }
}
