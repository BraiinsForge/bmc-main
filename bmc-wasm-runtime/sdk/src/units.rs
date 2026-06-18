// Copyright (C) 2026  Braiins Systems s.r.o.

//! Dimensional quantity newtypes.
//!
//! Each type is named after the physical *dimension* (e.g. [`Length`], [`Speed`])
//! and stores one canonical SI value; the various units appear only as `from_*`
//! constructors and `as_*` accessors. `.format()` renders the value with
//! the operator's unit-system / number-format preferences by delegating
//! to the host formatters in [`crate::format`].

/// A length, stored canonically in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Length(f64);

impl Length {
    #[must_use]
    pub const fn from_meters(meters: f64) -> Self {
        Self(meters)
    }

    #[must_use]
    pub fn from_kilometers(kilometers: f64) -> Self {
        Self(kilometers * 1_000.0)
    }

    #[must_use]
    pub const fn as_meters(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn as_kilometers(self) -> f64 {
        self.0 / 1_000.0
    }

    /// Render with the operator's unit system (km, or miles when imperial)
    /// and number-format preferences.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        crate::format::_host_format_distance(self.as_kilometers(), decimals)
    }
}

/// A speed, stored canonically in metres per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Speed(f64);

impl Speed {
    #[must_use]
    pub const fn from_meters_per_second(mps: f64) -> Self {
        Self(mps)
    }

    #[must_use]
    pub fn from_kilometers_per_hour(kmh: f64) -> Self {
        Self(kmh / 3.6)
    }

    #[must_use]
    pub const fn as_meters_per_second(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn as_kilometers_per_hour(self) -> f64 {
        self.0 * 3.6
    }

    #[must_use]
    pub fn as_kilometers_per_second(self) -> f64 {
        self.0 / 1_000.0
    }

    /// Render with the operator's unit system (km/h, or mph when imperial)
    /// and number-format preferences.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        crate::format::_host_format_speed(self.as_kilometers_per_hour(), decimals, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        // Relative tolerance: at magnitudes around 1e4 (m/s), an absolute 1e-6
        // is tighter than the precision of the hand-written decimal literal.
        (a - b).abs() <= 1e-6 * b.abs().max(1.0)
    }

    #[test]
    fn length_round_trips_through_canonical_meters() {
        let l = Length::from_kilometers(420.0);
        assert!(approx(l.as_meters(), 420_000.0));
        assert!(approx(l.as_kilometers(), 420.0));
    }

    #[test]
    fn speed_converts_between_the_units_it_exposes() {
        // The ISS at ~27 600 km/h is the canonical 7.666… km/s.
        let s = Speed::from_kilometers_per_hour(27_600.0);
        assert!(approx(s.as_kilometers_per_hour(), 27_600.0));
        assert!(approx(s.as_kilometers_per_second(), 7.666_666_7));
        assert!(approx(s.as_meters_per_second(), 7_666.666_7));
    }
}
