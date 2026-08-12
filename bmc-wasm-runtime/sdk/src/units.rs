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

//! Dimensional quantity newtypes: each is named after its physical *dimension*,
//! stores one canonical SI value, exposes units via `from_*`/`as_*`, and renders
//! through [`crate::format`] with the operator's preferences.

#![expect(
    clippy::used_underscore_items,
    reason = "delegates to the crate's own host-boundary `_host_format_*`"
)]

use crate::fmt;
use crate::system::UnitSystem;

// ── Unit-conversion factors ──────────────────────────────────────────

/// Metres per kilometre (also m/s per km/s).
const METERS_PER_KILOMETER: f64 = 1_000.0;
/// km/h per m/s.
const KMH_PER_MPS: f64 = 3.6;
const CENTIMETERS_PER_METER: f64 = 100.0;
const CENTIMETERS_PER_INCH: f64 = 2.54;
const INCHES_PER_FOOT: f64 = 12.0;
const POUNDS_PER_KILOGRAM: f64 = 2.204_622_6;
/// `°F = °C ·` this `+ `[`FREEZING_POINT_FAHRENHEIT`].
const CELSIUS_TO_FAHRENHEIT_SCALE: f64 = 9.0 / 5.0;
/// °F at 0 °C.
const FREEZING_POINT_FAHRENHEIT: f64 = 32.0;

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
        Self(kilometers * METERS_PER_KILOMETER)
    }

    #[must_use]
    pub const fn as_meters(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn as_kilometers(self) -> f64 {
        self.0 / METERS_PER_KILOMETER
    }

    #[must_use]
    pub fn from_centimeters(centimeters: f64) -> Self {
        Self(centimeters / CENTIMETERS_PER_METER)
    }

    #[must_use]
    pub fn as_centimeters(self) -> f64 {
        self.0 * CENTIMETERS_PER_METER
    }

    /// Render as km (miles when imperial) with the operator's number format.
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        crate::format::_host_format_distance(self.as_kilometers(), decimals)
    }

    /// Render at the scale a person or an object is measured at, not the
    /// scale of a journey: centimetres, or feet and inches when imperial.
    ///
    /// Imperial ignores `decimals`, since nobody quotes a height to a
    /// fraction of an inch, and keeps a zero inch so a table column holds
    /// still.
    #[must_use]
    pub fn format_short(self, decimals: u32) -> String {
        let centimeters = self.as_centimeters();
        if crate::system::current().unit_system().unwrap_or_default() == UnitSystem::Metric {
            return fmt!(
                "{} cm",
                crate::format::_host_format_number(centimeters, decimals)
            );
        }
        let (feet, inches) = feet_and_inches(centimeters);
        fmt!(
            "{}\u{2032}{}\u{2033}",
            crate::format::_host_format_number(feet, 0),
            crate::format::_host_format_number(inches, 0)
        )
    }
}

/// Whole feet and inches, the inches rounded.
///
/// Rounding can reach twelve inches, which is a foot, and `5′12″` is as
/// malformed as `10:60` — so it carries. The deckfeeder widget this came
/// from rounds without carrying, rendering a 182 cm driver as `5′12″`.
fn feet_and_inches(centimeters: f64) -> (f64, f64) {
    let total_inches = centimeters / CENTIMETERS_PER_INCH;
    let feet = (total_inches / INCHES_PER_FOOT).floor();
    let inches = (total_inches - feet * INCHES_PER_FOOT).round();
    if inches >= INCHES_PER_FOOT {
        (feet + 1.0, 0.0)
    } else {
        (feet, inches)
    }
}

/// A mass, stored canonically in kilograms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mass(f64);

impl Mass {
    #[must_use]
    pub const fn from_kilograms(kilograms: f64) -> Self {
        Self(kilograms)
    }

    #[must_use]
    pub fn from_pounds(pounds: f64) -> Self {
        Self(pounds / POUNDS_PER_KILOGRAM)
    }

    #[must_use]
    pub const fn as_kilograms(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn as_pounds(self) -> f64 {
        self.0 * POUNDS_PER_KILOGRAM
    }

    /// Render as kg (pounds when imperial) with the operator's number format.
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        let (value, unit) =
            if crate::system::current().unit_system().unwrap_or_default() == UnitSystem::Metric {
                (self.as_kilograms(), "kg")
            } else {
                (self.as_pounds(), "lbs")
            };
        fmt!(
            "{} {}",
            crate::format::_host_format_number(value, decimals),
            unit
        )
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
        Self(kmh / KMH_PER_MPS)
    }

    #[must_use]
    pub const fn as_meters_per_second(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn as_kilometers_per_hour(self) -> f64 {
        self.0 * KMH_PER_MPS
    }

    #[must_use]
    pub fn as_kilometers_per_second(self) -> f64 {
        self.0 / METERS_PER_KILOMETER
    }

    /// Render as km/h (mph when imperial) with the operator's number format.
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        crate::format::_host_format_speed(self.as_kilometers_per_hour(), decimals, 0)
    }
}

/// A temperature, stored canonically in degrees Celsius.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Temperature(f64);

impl Temperature {
    #[must_use]
    pub const fn from_celsius(celsius: f64) -> Self {
        Self(celsius)
    }

    #[must_use]
    pub fn from_fahrenheit(fahrenheit: f64) -> Self {
        Self((fahrenheit - FREEZING_POINT_FAHRENHEIT) / CELSIUS_TO_FAHRENHEIT_SCALE)
    }

    #[must_use]
    pub const fn as_celsius(self) -> f64 {
        self.0
    }

    #[must_use]
    pub fn as_fahrenheit(self) -> f64 {
        self.0 * CELSIUS_TO_FAHRENHEIT_SCALE + FREEZING_POINT_FAHRENHEIT
    }

    /// Render as °C (°F when imperial), e.g. `20 °C`.
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        crate::format::_host_format_temperature(self.as_celsius(), decimals, 1)
    }

    /// Degree-only form (`20°`) for dense strips.
    #[must_use]
    pub fn format_bare(self, decimals: u32) -> String {
        crate::format::_host_format_temperature(self.as_celsius(), decimals, 0)
    }

    /// The °C number alone, for split value/unit hardware-temp strips
    /// (conventionally Celsius); [`Self::format`] is the unit-aware form.
    #[must_use]
    pub fn format_value(self, decimals: u32) -> String {
        crate::format::_host_format_number(self.as_celsius(), decimals)
    }
}

/// Electrical power, stored canonically in watts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ElectricPower(f64);

impl ElectricPower {
    /// Unit appended by [`Self::format`].
    pub const UNIT: &'static str = "W";

    #[must_use]
    pub const fn from_watts(watts: f64) -> Self {
        Self(watts)
    }

    #[must_use]
    pub const fn as_watts(self) -> f64 {
        self.0
    }

    /// The number alone (no unit), for split value/unit rendering.
    #[must_use]
    pub fn format_value(self, decimals: u32) -> String {
        crate::format::_host_format_number(self.as_watts(), decimals)
    }

    /// Render with the operator's number format, e.g. `60 W`.
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        let mut s = self.format_value(decimals);
        s.push(' ');
        s.push_str(Self::UNIT);
        s
    }

    /// SI-prefixed at `sig_figs` significant digits, e.g. `13.2 kW`, so a big
    /// aggregate power stays compact.
    #[must_use]
    pub fn format_si(self, sig_figs: u32) -> String {
        crate::format::_host_format_si(self.as_watts(), sig_figs, Self::UNIT)
    }

    /// SI value and unit as split strings, for value/unit rendering.
    #[must_use]
    pub fn format_si_parts(self, sig_figs: u32) -> (String, String) {
        crate::format::_host_format_si_parts(self.as_watts(), sig_figs, Self::UNIT)
    }
}

/// Mining hashrate, stored canonically in terahashes per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hashrate(f64);

impl Hashrate {
    /// Unit appended by [`Self::format`].
    pub const UNIT: &'static str = "TH/s";

    #[must_use]
    pub const fn from_terahashes_per_second(ths: f64) -> Self {
        Self(ths)
    }

    #[must_use]
    pub const fn as_terahashes_per_second(self) -> f64 {
        self.0
    }

    /// The number alone (no unit), for split value/unit rendering.
    #[must_use]
    pub fn format_value(self, decimals: u32) -> String {
        crate::format::_host_format_number(self.as_terahashes_per_second(), decimals)
    }

    /// Render with the operator's number format, e.g. `17.08 TH/s`.
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        let mut s = self.format_value(decimals);
        s.push(' ');
        s.push_str(Self::UNIT);
        s
    }

    /// SI-prefixed at `sig_figs` sig figs, from the H/s base so the prefix is
    /// real (`313 TH/s`, `16.5 PH/s`), never `kTH/s`.
    #[must_use]
    pub fn format_si(self, sig_figs: u32) -> String {
        crate::format::_host_format_si(self.as_terahashes_per_second() * 1e12, sig_figs, "H/s")
    }

    /// SI value and unit as split strings, for value/unit rendering.
    #[must_use]
    pub fn format_si_parts(self, sig_figs: u32) -> (String, String) {
        crate::format::_host_format_si_parts(
            self.as_terahashes_per_second() * 1e12,
            sig_figs,
            "H/s",
        )
    }
}

/// Mining energy efficiency in joules per terahash. Lower is better — a cost, not a ratio.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MiningEfficiency(f64);

impl MiningEfficiency {
    /// Unit appended by [`Self::format`].
    pub const UNIT: &'static str = "J/TH";

    #[must_use]
    pub const fn from_joules_per_terahash(jth: f64) -> Self {
        Self(jth)
    }

    #[must_use]
    pub const fn as_joules_per_terahash(self) -> f64 {
        self.0
    }

    /// The number alone (no unit), for split value/unit rendering.
    #[must_use]
    pub fn format_value(self, decimals: u32) -> String {
        crate::format::_host_format_number(self.as_joules_per_terahash(), decimals)
    }

    /// Render with the operator's number format, e.g. `10.01 J/TH`.
    #[must_use]
    pub fn format(self, decimals: u32) -> String {
        let mut s = self.format_value(decimals);
        s.push(' ');
        s.push_str(Self::UNIT);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        // Relative tolerance — hand-written decimals aren't exact near 1e4.
        (a - b).abs() <= 1e-6 * b.abs().max(1.0)
    }

    #[test]
    fn length_round_trips_through_canonical_meters() {
        let l = Length::from_kilometers(420.0);
        assert!(approx(l.as_meters(), 420_000.0));
        assert!(approx(l.as_kilometers(), 420.0));
    }

    #[test]
    fn length_round_trips_through_the_scale_a_person_is_measured_at() {
        let l = Length::from_centimeters(180.0);
        assert!(approx(l.as_centimeters(), 180.0));
        assert!(approx(l.as_meters(), 1.8));
    }

    #[test]
    fn mass_converts_between_kilograms_and_pounds() {
        let m = Mass::from_kilograms(70.0);
        assert!(approx(m.as_kilograms(), 70.0));
        assert!(approx(m.as_pounds(), 154.323_582));
        assert!(approx(
            Mass::from_pounds(m.as_pounds()).as_kilograms(),
            70.0
        ));
    }

    /// With no snapshot installed the operator's units read as metric.
    #[test]
    fn a_short_length_renders_at_its_own_scale() {
        assert_eq!(Length::from_centimeters(180.0).format_short(0), "180 cm");
        assert_eq!(Mass::from_kilograms(70.0).format(0), "70 kg");
    }

    /// Off-device the operator's units are whatever the thread was given,
    /// so a story can render both systems without a host.
    #[test]
    fn imperial_units_render_off_device_once_a_snapshot_says_so() {
        crate::system::set_current(
            crate::system::SnapshotBuilder::new()
                .unit_system(UnitSystem::Imperial)
                .build(),
        );
        assert_eq!(
            Length::from_centimeters(182.0).format_short(0),
            "6\u{2032}0\u{2033}"
        );
        assert_eq!(Mass::from_kilograms(70.0).format(0), "154 lbs");

        crate::system::set_current(crate::system::Snapshot::empty());
        assert_eq!(Length::from_centimeters(182.0).format_short(0), "182 cm");
    }

    /// Heights off the 2026 grid, 182 cm being the one that rounds up
    /// into a whole foot.
    #[test]
    fn inches_rounding_into_a_foot_carries_rather_than_reading_twelve() {
        assert_eq!(feet_and_inches(180.0), (5.0, 11.0));
        assert_eq!(feet_and_inches(181.0), (5.0, 11.0));
        assert_eq!(feet_and_inches(182.0), (6.0, 0.0));
        assert_eq!(feet_and_inches(186.0), (6.0, 1.0));
    }

    #[test]
    fn speed_converts_between_the_units_it_exposes() {
        // The ISS at ~27 600 km/h is the canonical 7.666… km/s.
        let s = Speed::from_kilometers_per_hour(27_600.0);
        assert!(approx(s.as_kilometers_per_hour(), 27_600.0));
        assert!(approx(s.as_kilometers_per_second(), 7.666_666_7));
        assert!(approx(s.as_meters_per_second(), 7_666.666_7));
    }

    #[test]
    fn temperature_converts_between_celsius_and_fahrenheit() {
        let t = Temperature::from_celsius(20.0);
        assert!(approx(t.as_celsius(), 20.0));
        assert!(approx(t.as_fahrenheit(), 68.0));
        assert!(approx(
            Temperature::from_fahrenheit(68.0).as_celsius(),
            20.0
        ));
    }

    #[test]
    fn mining_quantities_round_trip_through_their_canonical_unit() {
        assert!(approx(ElectricPower::from_watts(60.0).as_watts(), 60.0));
        assert!(approx(
            Hashrate::from_terahashes_per_second(17.08).as_terahashes_per_second(),
            17.08
        ));
        assert!(approx(
            MiningEfficiency::from_joules_per_terahash(10.01).as_joules_per_terahash(),
            10.01
        ));
    }
}
