// Copyright (C) 2026  Braiins Systems s.r.o.

//! Host-pure price/percent formatting shared by both ticker widgets. Builds
//! strings without `std::format!` (the `no-fmt-in-wasm` gate); the grouped
//! magnitude itself goes through the SDK `format_number!` host call in the
//! widget render path, while sign/decimal composition stays here and host-tested.

/// Number of fraction digits to show for a price, reproducing deckfeeder's
/// `maximumFractionDigits: 2` (up to two digits, trailing zeros dropped):
/// `169420 → 0`, `291.74 → 2`, `291.70 → 1`, sub-penny `0.000142 → 0` ("0").
#[must_use]
pub fn fraction_digits(value: f64) -> u32 {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract().abs() < 1e-9 {
        0
    } else if (rounded * 10.0).fract().abs() < 1e-9 {
        1
    } else {
        2
    }
}

/// The signed change badge text, one decimal, e.g. `+5.3%` / `-2.8%`.
/// Reproduces deckfeeder's `${sign}${priceChange.toFixed(1)}%` (plain `.`
/// decimal, locale-independent — `toFixed`, not `Intl`). `0.0` → `+0.0%`.
#[must_use]
pub fn change_text(change_pct: f64) -> String {
    let mut out = String::new();
    out.push(if change_pct >= 0.0 { '+' } else { '-' });
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
    push_i64(out, int_part);
    out.push('.');
    out.push(char::from(
        b'0' + u8::try_from(frac).expect("BUG: frac is a single 0..=9 digit"),
    ));
}

fn push_i64(out: &mut String, mut n: i64) {
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

    #[test]
    fn fraction_digits_drops_trailing_zeros_up_to_two() {
        assert_eq!(fraction_digits(169_420.0), 0);
        assert_eq!(fraction_digits(118_079.0), 0);
        assert_eq!(fraction_digits(291.74), 2);
        assert_eq!(fraction_digits(291.70), 1);
        // sub-penny rounds to 0.00 → shown as "0", matching JS maximumFractionDigits:2
        assert_eq!(fraction_digits(0.000_142), 0);
    }

    #[test]
    fn change_text_is_signed_one_decimal() {
        assert_eq!(change_text(5.31), "+5.3%");
        assert_eq!(change_text(-2.81), "-2.8%");
        assert_eq!(change_text(0.0), "+0.0%");
        assert_eq!(change_text(0.39), "+0.4%");
    }
}
