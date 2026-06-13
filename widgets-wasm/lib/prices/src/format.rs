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

//! Host-pure price/percent formatting shared by both ticker widgets. Builds
//! strings without `std::format!` (the `no-fmt-in-wasm` gate); the grouped
//! magnitude itself goes through the SDK `format_number!` host call in the
//! widget render path, while sign/decimal composition stays here and host-tested.

/// How to render a price: a fixed fraction-digit count for the host
/// formatter, or a "below the smallest bucket" marker the caller renders as
/// a `<0.000001`-style literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PricePrecision {
    /// Format with exactly this many fraction digits (host pads with zeros).
    Fraction(u32),
    /// The price is positive but under [`MIN_PRICE`]; show `< MIN_PRICE`.
    BelowMin,
}

/// Smallest price rendered as digits; anything positive below this is
/// [`PricePrecision::BelowMin`].
pub const MIN_PRICE: f64 = 1e-6;

/// Fiat codes recognised for the FX-pair precision override. Both sides of a
/// `BASE-QUOTE` symbol must match for the override to apply, so `BTC-USD`
/// stays on the magnitude rule.
const FIAT_CODES: &[&str] = &[
    "USD", "EUR", "JPY", "GBP", "CHF", "CAD", "AUD", "NZD", "CNY", "HKD", "SGD", "SEK", "NOK",
    "DKK", "PLN", "CZK", "HUF", "RON", "TRY", "ZAR", "MXN", "BRL", "INR", "KRW", "THB", "ILS",
];

/// Display precision for a price: FX pairs (both symbol halves fiat) follow
/// the forex quote convention (5 fraction digits, 3 for a JPY quote) while the
/// rate fits the column (< 1000); everything else uses magnitude buckets that
/// keep roughly four significant digits below 1 and the classic two decimals
/// above (none from 100 000 up). A value that rounds across its bucket
/// boundary takes the wider bucket's digits, so 99 999.995 renders `100 000`,
/// not `100 000.00`.
#[must_use]
pub fn price_precision(symbol: &str, value: f64) -> PricePrecision {
    let v = value.abs();
    if v < MIN_PRICE {
        return if v > 0.0 {
            PricePrecision::BelowMin
        } else {
            PricePrecision::Fraction(0)
        };
    }
    if let Some(quote) = fx_quote(symbol)
        && v < 1000.0
    {
        let digits = if quote.eq_ignore_ascii_case("JPY") {
            3
        } else {
            5
        };
        return PricePrecision::Fraction(digits);
    }
    let digits = bucket_digits(v);
    let scale = 10f64.powi(i32::try_from(digits).expect("BUG: bucket digits fit i32"));
    let rounded = (v * scale).round() / scale;
    PricePrecision::Fraction(bucket_digits(rounded))
}

/// Fraction digits for a positive in-range magnitude.
fn bucket_digits(v: f64) -> u32 {
    if v >= 100_000.0 {
        0
    } else if v >= 1.0 {
        2
    } else if v >= 0.001 {
        4
    } else if v >= 0.000_1 {
        5
    } else if v >= 0.000_01 {
        6
    } else {
        7
    }
}

/// The quote half of an FX pair symbol, when both halves are fiat codes.
fn fx_quote(symbol: &str) -> Option<&str> {
    let symbol = symbol.strip_suffix("=X").unwrap_or(symbol);
    let (base, quote) = if let Some(pair) = symbol.split_once('-') {
        pair
    } else if symbol.is_ascii() && symbol.len() == 6 {
        symbol.split_at(3)
    } else {
        return None;
    };
    let is_fiat = |code: &str| {
        FIAT_CODES
            .iter()
            .any(|fiat| fiat.eq_ignore_ascii_case(code))
    };
    (is_fiat(base) && is_fiat(quote)).then_some(quote)
}

/// Percentage change from a starting price to the current one. A zero start
/// price yields `0.0` (rather than a non-finite value); a negative one yields
/// a finite best-effort percentage. Shared so both ticker widgets agree.
#[must_use]
pub fn price_change(first: f64, current: f64) -> f64 {
    if first == 0.0 {
        0.0
    } else {
        (current - first) / first * 100.0
    }
}

/// The signed change badge text, one decimal, e.g. `+5.3%` / `-2.8%`.
/// Uses a plain `.` decimal separator regardless of locale. `0.0` → `+0.0%`.
#[must_use]
pub fn change_text(change_pct: f64) -> String {
    let mut out = String::new();
    let rounded_magnitude = (change_pct.abs() * 10.0).round();
    out.push(if change_pct >= 0.0 || rounded_magnitude == 0.0 {
        '+'
    } else {
        '-'
    });
    push_fixed1(&mut out, change_pct.abs());
    out.push('%');
    out
}

/// Append `magnitude` (non-negative) to `out` with exactly one decimal,
/// using `.` as the separator.
fn push_fixed1(out: &mut String, magnitude: f64) {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "magnitude is a non-negative percentage; one-decimal scaling stays well within i64"
    )]
    let scaled = (magnitude * 10.0).round() as i64;
    let int_part = scaled / 10;
    let frac = scaled % 10;
    push_uint(
        out,
        u64::try_from(int_part).expect("BUG: int_part is non-negative"),
    );
    out.push('.');
    out.push(char::from(
        b'0' + u8::try_from(frac).expect("BUG: frac is a single 0..=9 digit"),
    ));
}

/// Append a non-negative integer to `out` in base 10, without `std::format!`
/// (the no-fmt-in-wasm gate). Shared by the price/percent formatters.
pub fn push_uint(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + u8::try_from(n % 10).expect("BUG: n % 10 is one decimal digit");
        n /= 10;
    }
    for &d in &digits[i..] {
        out.push(char::from(d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use PricePrecision::{BelowMin, Fraction};

    #[test]
    fn large_and_mid_prices_keep_classic_decimals() {
        assert_eq!(price_precision("NVDA", 169_420.0), Fraction(0));
        assert_eq!(price_precision("BTC-USD", 106_234.56), Fraction(0));
        assert_eq!(price_precision("NVDA", 291.74), Fraction(2));
        assert_eq!(price_precision("NVDA", 291.70), Fraction(2));
        assert_eq!(price_precision("NVDA", 1.0), Fraction(2));
    }

    #[test]
    fn sub_one_prices_widen_by_magnitude_bucket() {
        assert_eq!(price_precision("X", 0.123_456), Fraction(4));
        assert_eq!(price_precision("X", 0.001), Fraction(4));
        assert_eq!(price_precision("X", 0.000_994), Fraction(5));
        assert_eq!(price_precision("X", 0.000_1), Fraction(5));
        assert_eq!(price_precision("SHIB-USD", 0.000_012_34), Fraction(6));
        assert_eq!(price_precision("X", 0.000_001_234), Fraction(7));
    }

    #[test]
    fn dust_prices_collapse_to_below_min() {
        assert_eq!(price_precision("X", 0.000_000_49), BelowMin);
    }

    #[test]
    fn zero_renders_as_plain_zero() {
        assert_eq!(price_precision("X", 0.0), Fraction(0));
    }

    #[test]
    fn rounding_into_the_next_bucket_uses_that_buckets_digits() {
        // 0.99995 would render "1.0000" with sub-one digits; carry to "1.00"
        assert_eq!(price_precision("X", 0.999_95), Fraction(2));
        // 99999.995 would render "100 000.00"; carry to "100 000"
        assert_eq!(price_precision("X", 99_999.995), Fraction(0));
        assert_eq!(price_precision("X", 99_999.99), Fraction(2));
    }

    #[test]
    fn fiat_pairs_use_fx_quote_precision() {
        assert_eq!(price_precision("EUR-USD", 1.164_27), Fraction(5));
        assert_eq!(price_precision("eur-usd", 1.164_27), Fraction(5));
        assert_eq!(price_precision("EUR-GBP", 0.85), Fraction(5));
        assert_eq!(price_precision("USD-JPY", 155.123_4), Fraction(3));
        assert_eq!(price_precision("EURUSD=X", 1.164_27), Fraction(5));
        assert_eq!(price_precision("USDJPY=X", 155.123_4), Fraction(3));
        assert_eq!(price_precision("EURUSD=X", 0.0), Fraction(0));
    }

    #[test]
    fn fx_override_skips_non_fiat_and_wide_rates() {
        // crypto/base or class shares are not FX pairs
        assert_eq!(price_precision("BTC-USD", 1.16), Fraction(2));
        assert_eq!(price_precision("BRK-B", 491.62), Fraction(2));
        // a rate >= 1000 falls back to the magnitude rule (width guard)
        assert_eq!(price_precision("USD-HUF", 1234.5), Fraction(2));
    }

    #[test]
    fn change_text_is_signed_one_decimal() {
        assert_eq!(change_text(5.31), "+5.3%");
        assert_eq!(change_text(-2.81), "-2.8%");
        assert_eq!(change_text(0.0), "+0.0%");
        assert_eq!(change_text(0.39), "+0.4%");
        assert_eq!(change_text(-0.04), "+0.0%");
    }

    #[test]
    fn price_change_handles_zero_and_negative_first() {
        assert!((price_change(100.0, 110.0) - 10.0).abs() < 1e-9);
        assert!((price_change(100.0, 90.0) - (-10.0)).abs() < 1e-9);
        assert!((price_change(0.0, 5.0)).abs() < 1e-9);
        // negative first close: finite best-effort, not NaN/inf
        assert!(price_change(-1.0, 5.0).is_finite());
    }
}
