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

pub trait Quantity: Copy {
    const UNIT: &'static str;
    fn raw(self) -> f64;
}

macro_rules! quantity {
    ($name:ident, $unit:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct $name(pub f64);

        impl Quantity for $name {
            const UNIT: &'static str = $unit;
            fn raw(self) -> f64 {
                self.0
            }
        }
    };
}

quantity!(TeraHashPerSecond, "TH/s");
quantity!(ExaHashPerSecond, "EH/s");
quantity!(Watt, "W");
quantity!(DegreeCelsius, "°C");
quantity!(JoulePerTeraHash, "J/TH");
quantity!(Percent, "%");
quantity!(Btc, "BTC");
quantity!(SatPerTeraHashDay, "SAT/TH/Day");
quantity!(KilometerPerHour, "km/h");
quantity!(Degree, "°");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seconds(pub u64);

impl Seconds {
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantity_exposes_unit_and_raw() {
        assert_eq!(Watt::UNIT, "W");
        assert_eq!(Percent::UNIT, "%");
        assert!((Watt(120.0).raw() - 120.0).abs() < f64::EPSILON);
    }
}
