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

//! The decimal SI prefixes, so a quantity scales by naming the prefix
//! an intake quotes rather than by a constant per pair of units.

/// A decimal SI prefix; `One` is the bare unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiPrefix {
    Quecto,
    Ronto,
    Yocto,
    Zepto,
    Atto,
    Femto,
    Pico,
    Nano,
    Micro,
    Milli,
    Centi,
    Deci,
    One,
    Deca,
    Hecto,
    Kilo,
    Mega,
    Giga,
    Tera,
    Peta,
    Exa,
    Zetta,
    Yotta,
    Ronna,
    Quetta,
}

/// What one prefix is: its exponent of ten, that power as the double
/// nearest to it, and its symbol.
#[derive(Clone, Copy)]
struct Row {
    prefix: SiPrefix,
    exponent: i32,
    factor: f64,
    symbol: &'static str,
}

impl Row {
    const fn new(prefix: SiPrefix, exponent: i32, factor: f64, symbol: &'static str) -> Self {
        Self {
            prefix,
            exponent,
            factor,
            symbol,
        }
    }
}

const ROWS: [Row; 25] = [
    Row::new(SiPrefix::Quecto, -30, 1e-30, "q"),
    Row::new(SiPrefix::Ronto, -27, 1e-27, "r"),
    Row::new(SiPrefix::Yocto, -24, 1e-24, "y"),
    Row::new(SiPrefix::Zepto, -21, 1e-21, "z"),
    Row::new(SiPrefix::Atto, -18, 1e-18, "a"),
    Row::new(SiPrefix::Femto, -15, 1e-15, "f"),
    Row::new(SiPrefix::Pico, -12, 1e-12, "p"),
    Row::new(SiPrefix::Nano, -9, 1e-9, "n"),
    Row::new(SiPrefix::Micro, -6, 1e-6, "µ"),
    Row::new(SiPrefix::Milli, -3, 1e-3, "m"),
    Row::new(SiPrefix::Centi, -2, 1e-2, "c"),
    Row::new(SiPrefix::Deci, -1, 1e-1, "d"),
    Row::new(SiPrefix::One, 0, 1.0, ""),
    Row::new(SiPrefix::Deca, 1, 1e1, "da"),
    Row::new(SiPrefix::Hecto, 2, 1e2, "h"),
    Row::new(SiPrefix::Kilo, 3, 1e3, "k"),
    Row::new(SiPrefix::Mega, 6, 1e6, "M"),
    Row::new(SiPrefix::Giga, 9, 1e9, "G"),
    Row::new(SiPrefix::Tera, 12, 1e12, "T"),
    Row::new(SiPrefix::Peta, 15, 1e15, "P"),
    Row::new(SiPrefix::Exa, 18, 1e18, "E"),
    Row::new(SiPrefix::Zetta, 21, 1e21, "Z"),
    Row::new(SiPrefix::Yotta, 24, 1e24, "Y"),
    Row::new(SiPrefix::Ronna, 27, 1e27, "R"),
    Row::new(SiPrefix::Quetta, 30, 1e30, "Q"),
];

// A prefix reads its row by discriminant, so the rows sit in declaration order.
const _: () = {
    let mut index = 0;
    while index < ROWS.len() {
        assert!(ROWS[index].prefix as usize == index);
        index += 1;
    }
};

impl SiPrefix {
    /// Every prefix, smallest first.
    pub const ALL: [Self; 25] = {
        let mut all = [Self::One; 25];
        let mut index = 0;
        while index < ROWS.len() {
            all[index] = ROWS[index].prefix;
            index += 1;
        }
        all
    };

    const fn row(self) -> Row {
        ROWS[self as usize]
    }

    /// The exponent of ten the prefix stands for.
    #[must_use]
    pub const fn exponent(self) -> i32 {
        self.row().exponent
    }

    /// The power of ten the prefix scales by.
    #[must_use]
    pub const fn factor(self) -> f64 {
        self.row().factor
    }

    #[must_use]
    pub const fn symbol(self) -> &'static str {
        self.row().symbol
    }

    #[must_use]
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        ROWS.iter()
            .find(|row| row.symbol == symbol)
            .map(|row| row.prefix)
    }

    /// `value` in `from` units, expressed in `to` units.
    ///
    /// One multiply or one divide by the power of ten between the two
    /// exponents, whichever the direction allows, so scaling down lands
    /// on the same double as a plain division would.
    #[must_use]
    pub const fn convert(value: f64, from: Self, to: Self) -> f64 {
        let shift = from.exponent() - to.exponent();
        if shift >= 0 {
            value * pow10(shift.unsigned_abs())
        } else {
            value / pow10(shift.unsigned_abs())
        }
    }
}

/// `10^k` as the double nearest to it, for every gap two prefixes can have.
/// Each is a literal, since a quotient of two rounded factors is not that:
/// `1e-3 / 1e-6` lands an ulp above `1e3`.
const fn pow10(k: u32) -> f64 {
    const POWERS: [f64; 61] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
        1e17, 1e18, 1e19, 1e20, 1e21, 1e22, 1e23, 1e24, 1e25, 1e26, 1e27, 1e28, 1e29, 1e30, 1e31,
        1e32, 1e33, 1e34, 1e35, 1e36, 1e37, 1e38, 1e39, 1e40, 1e41, 1e42, 1e43, 1e44, 1e45, 1e46,
        1e47, 1e48, 1e49, 1e50, 1e51, 1e52, 1e53, 1e54, 1e55, 1e56, 1e57, 1e58, 1e59, 1e60,
    ];
    POWERS[k as usize]
}

#[cfg(test)]
mod tests {
    use super::{SiPrefix, pow10};
    use crate::fmt;

    fn literal(exponent: i32) -> f64 {
        fmt!("1e{}", exponent)
            .parse()
            .expect("BUG: an integer exponent parses as a float")
    }

    #[test]
    fn every_symbol_reads_back_to_its_prefix() {
        for prefix in SiPrefix::ALL {
            assert_eq!(SiPrefix::from_symbol(prefix.symbol()), Some(prefix));
        }
        assert_eq!(SiPrefix::from_symbol("x"), None);
    }

    #[test]
    fn the_bare_unit_has_no_symbol() {
        assert_eq!(SiPrefix::One.symbol(), "");
        assert!((SiPrefix::One.factor() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn factors_ascend_with_the_prefixes() {
        for pair in SiPrefix::ALL.windows(2) {
            assert!(pair[0].factor() < pair[1].factor(), "{pair:?}");
        }
    }

    /// A row's factor is the double nearest to ten raised to its exponent,
    /// which is what the literal parses to.
    #[test]
    fn every_factor_is_the_literal_of_its_exponent() {
        for prefix in SiPrefix::ALL {
            let expected = literal(prefix.exponent());
            assert_eq!(prefix.factor().to_bits(), expected.to_bits(), "{prefix:?}");
        }
    }

    /// The table reaches the widest gap, and each entry is its own literal.
    #[test]
    fn every_power_of_ten_is_the_double_nearest_to_it() {
        let widest_gap = SiPrefix::Quetta.exponent() - SiPrefix::Quecto.exponent();
        for k in 0..=widest_gap.unsigned_abs() {
            let expected = literal(i32::try_from(k).expect("BUG: a gap fits i32"));
            assert_eq!(pow10(k).to_bits(), expected.to_bits(), "10^{k}");
        }
    }

    /// Neighbouring prefixes are the case a quotient of factors gets wrong.
    #[test]
    fn adjacent_prefixes_convert_by_the_exact_power_of_ten() {
        assert_eq!(
            SiPrefix::convert(0.1, SiPrefix::Micro, SiPrefix::Milli).to_bits(),
            (0.1_f64 / 1_000.0).to_bits()
        );
        for pair in SiPrefix::ALL.windows(2) {
            let (small, large) = (pair[0], pair[1]);
            let gap = match large.exponent() - small.exponent() {
                1 => 10.0,
                2 => 100.0,
                3 => 1_000.0,
                other => panic!("BUG: prefixes are 1, 2 or 3 decades apart, not {other}"),
            };
            assert_eq!(
                SiPrefix::convert(0.1, large, small).to_bits(),
                (0.1_f64 * gap).to_bits(),
                "{large:?} -> {small:?}"
            );
            assert_eq!(
                SiPrefix::convert(0.1, small, large).to_bits(),
                (0.1_f64 / gap).to_bits(),
                "{small:?} -> {large:?}"
            );
        }
    }

    /// Scaling down divides by the integral ratio rather than multiplying
    /// by its reciprocal, which would drift by an ulp.
    #[test]
    fn convert_is_the_plain_division_or_multiplication() {
        assert_eq!(
            SiPrefix::convert(122_480.0, SiPrefix::Giga, SiPrefix::Tera).to_bits(),
            (122_480.0_f64 / 1_000.0).to_bits()
        );
        assert_eq!(
            SiPrefix::convert(1_071_197_300_000.0, SiPrefix::One, SiPrefix::Tera).to_bits(),
            (1_071_197_300_000.0_f64 / 1e12).to_bits()
        );
        assert_eq!(
            SiPrefix::convert(35_000.0, SiPrefix::Milli, SiPrefix::One).to_bits(),
            (35_000.0_f64 / 1_000.0).to_bits()
        );
        assert_eq!(
            SiPrefix::convert(17.08, SiPrefix::Tera, SiPrefix::One).to_bits(),
            (17.08_f64 * 1e12).to_bits()
        );
    }

    #[test]
    fn converting_to_the_same_prefix_is_the_identity() {
        for prefix in SiPrefix::ALL {
            assert_eq!(
                SiPrefix::convert(0.3, prefix, prefix).to_bits(),
                0.3_f64.to_bits(),
                "{prefix:?}"
            );
        }
    }
}
