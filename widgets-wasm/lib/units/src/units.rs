// Copyright (C) 2026  Braiins Systems s.r.o.

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
