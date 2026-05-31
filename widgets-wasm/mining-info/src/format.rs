// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::model::{Availability, Currency, Money, TemperatureRange};

pub(crate) const NOT_AVAILABLE: &str = "N/A";
pub(crate) const PUBLIC_NOT_AVAILABLE: &str = "--";

pub(crate) fn unavailable() -> String {
    NOT_AVAILABLE.to_owned()
}

pub(crate) fn public_unavailable() -> String {
    PUBLIC_NOT_AVAILABLE.to_owned()
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

pub(crate) fn fixed(value: Availability<f64>, decimals: u32) -> String {
    match value {
        Availability::Available(value) => {
            let mut out = String::new();
            if value < 0.0 {
                out.push('-');
            }
            out.push_str(&group(value.abs(), decimals));
            out
        }
        Availability::Unavailable => unavailable(),
    }
}

pub(crate) fn fixed_strip_zero_fraction(value: Availability<f64>, decimals: u32) -> String {
    let mut out = fixed(value, decimals);
    if decimals == 0 || out == NOT_AVAILABLE {
        return out;
    }
    if out.ends_with(&"0".repeat(decimals as usize)) {
        out.truncate(out.len() - decimals as usize);
        out.pop();
    }
    out
}

pub(crate) fn approx_fixed(value: Availability<f64>, decimals: u32) -> String {
    match value {
        Availability::Available(_) => {
            let mut out = String::from("~ ");
            out.push_str(&fixed(value, decimals));
            out
        }
        Availability::Unavailable => unavailable(),
    }
}

pub(crate) fn signed_percent(value: Availability<f64>, decimals: u32) -> String {
    match value {
        Availability::Available(value) => {
            let mut out = String::new();
            out.push(if value >= 0.0 { '+' } else { '-' });
            out.push_str(&group(value.abs(), decimals));
            out
        }
        Availability::Unavailable => unavailable(),
    }
}

pub(crate) fn temperature(value: Availability<TemperatureRange>) -> String {
    match value {
        Availability::Available(value) => {
            let mut out = String::new();
            push_fixed_abs(&mut out, value.board_c, 0);
            out.push('-');
            push_fixed_abs(&mut out, value.chip_c, 0);
            out
        }
        Availability::Unavailable => unavailable(),
    }
}

pub(crate) fn temperature_mean(value: Availability<TemperatureRange>) -> String {
    match value {
        Availability::Available(value) => {
            let mut out = String::new();
            push_fixed_abs(&mut out, f64::midpoint(value.board_c, value.chip_c), 0);
            out
        }
        Availability::Unavailable => unavailable(),
    }
}

pub(crate) fn money(value: Availability<Money>, decimals: u32) -> String {
    match value {
        Availability::Available(Money { currency, value }) => {
            let symbol = match currency {
                Currency::Usd => "$",
                Currency::Eur => "€",
            };
            let mut out = String::from(symbol);
            out.push(' ');
            out.push_str(&group(value.abs(), decimals));
            out
        }
        Availability::Unavailable => unavailable(),
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

pub(crate) fn public_integer(value: Availability<u64>) -> String {
    match value {
        Availability::Available(value) => {
            #[expect(
                clippy::cast_precision_loss,
                reason = "block height stays well within f64's exact integer range"
            )]
            let magnitude = value as f64;
            group(magnitude, 0)
        }
        Availability::Unavailable => public_unavailable(),
    }
}

pub(crate) fn uptime(value: Availability<u64>) -> String {
    let Availability::Available(total) = value else {
        return unavailable();
    };
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_temperature_range_like_boser() {
        let range = TemperatureRange {
            board_c: 61.2,
            chip_c: 74.4,
        };
        assert_eq!(temperature(Availability::Available(range)), "61-74");
    }

    #[test]
    fn averages_temperature_into_single_value() {
        let range = TemperatureRange {
            board_c: 61.2,
            chip_c: 74.4,
        };
        assert_eq!(temperature_mean(Availability::Available(range)), "68");
        assert_eq!(temperature_mean(Availability::Unavailable), "N/A");
    }

    #[test]
    fn formats_signed_percent_with_explicit_sign() {
        assert_eq!(signed_percent(Availability::Available(1.82), 2), "+1.82");
        assert_eq!(signed_percent(Availability::Available(-0.77), 2), "-0.77");
    }

    #[test]
    fn formats_uptime_compactly() {
        assert_eq!(uptime(Availability::Available(187_020)), "2d 3h 57m");
        assert_eq!(uptime(Availability::Unavailable), "N/A");
    }

    #[test]
    fn formats_currency_symbol() {
        let usd = Money {
            currency: Currency::Usd,
            value: 104_250.4,
        };
        assert_eq!(money(Availability::Available(usd), 0), "$ 104250");
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
            fixed_strip_zero_fraction(Availability::Available(50.0), 2),
            "50"
        );
        assert_eq!(
            fixed_strip_zero_fraction(Availability::Available(50.25), 2),
            "50.25"
        );
        assert_eq!(
            fixed_strip_zero_fraction(Availability::Unavailable, 2),
            "N/A"
        );
    }

    #[test]
    fn formats_approximate_fixed_value_like_boser() {
        assert_eq!(approx_fixed(Availability::Available(0.1234), 3), "~ 0.123");
        assert_eq!(approx_fixed(Availability::Unavailable, 3), "N/A");
    }
}
