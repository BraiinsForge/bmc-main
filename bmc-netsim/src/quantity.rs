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

//! Blueprint quantities, validated at load.
//!
//! A simulated miner reports whatever the blueprint says, so a negative
//! hashrate or a NaN becomes telemetry no real device could produce — and the
//! widget under test is measured against a fiction.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserialize, Deserializer, Visitor};

/// No temperature a device could report sits below this.
const ABSOLUTE_ZERO_C: f64 = -273.15;

/// A finite, non-negative blueprint figure — a hashrate, a power draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NonNegative(f64);

/// A finite temperature in degrees Celsius, above absolute zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Celsius(f64);

impl NonNegative {
    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

impl Celsius {
    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

impl From<f64> for NonNegative {
    /// For the profiles' own defaults; blueprint input is checked by
    /// [`Deserialize`] instead, which can report where the bad figure sits.
    fn from(value: f64) -> Self {
        debug_assert!(
            value.is_finite() && value >= 0.0,
            "{value} is not a non-negative figure"
        );
        Self(value)
    }
}

impl From<f64> for Celsius {
    /// For the profiles' own defaults; see [`NonNegative::from`].
    fn from(value: f64) -> Self {
        debug_assert!(
            value.is_finite() && value > ABSOLUTE_ZERO_C,
            "{value} °C is not above absolute zero"
        );
        Self(value)
    }
}

/// Validate a blueprint figure, or say why it cannot stand.
fn checked<E: de::Error>(value: f64, kind: Kind) -> Result<f64, E> {
    if !value.is_finite() {
        return Err(E::custom(format!("{value} is not a finite number")));
    }
    match kind {
        Kind::NonNegative if value < 0.0 => Err(E::custom(format!("{value} must not be negative"))),
        Kind::Celsius if value <= ABSOLUTE_ZERO_C => Err(E::custom(format!(
            "{value} °C is at or below absolute zero ({ABSOLUTE_ZERO_C} °C)"
        ))),
        Kind::NonNegative | Kind::Celsius => Ok(value),
    }
}

#[derive(Clone, Copy)]
enum Kind {
    NonNegative,
    Celsius,
}

impl Kind {
    fn expecting(self) -> &'static str {
        match self {
            Kind::NonNegative => "a non-negative number",
            Kind::Celsius => "a temperature in °C",
        }
    }
}

/// Read an `f64` through the visitor — see [`crate::http_status`] for why the
/// check cannot move to `#[serde(try_from)]` without losing the source position.
fn deserialize_checked<'de, D: Deserializer<'de>>(
    deserializer: D,
    kind: Kind,
) -> Result<f64, D::Error> {
    struct NumberVisitor(Kind);

    impl Visitor<'_> for NumberVisitor {
        type Value = f64;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0.expecting())
        }

        fn visit_f64<E: de::Error>(self, value: f64) -> Result<f64, E> {
            checked(value, self.0)
        }

        #[expect(
            clippy::cast_precision_loss,
            reason = "a hashrate or power draw never nears f64's exact-integer limit"
        )]
        fn visit_i64<E: de::Error>(self, value: i64) -> Result<f64, E> {
            checked(value as f64, self.0)
        }

        #[expect(
            clippy::cast_precision_loss,
            reason = "a hashrate or power draw never nears f64's exact-integer limit"
        )]
        fn visit_u64<E: de::Error>(self, value: u64) -> Result<f64, E> {
            checked(value as f64, self.0)
        }
    }

    deserializer.deserialize_f64(NumberVisitor(kind))
}

impl<'de> Deserialize<'de> for NonNegative {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_checked(deserializer, Kind::NonNegative).map(Self)
    }
}

impl<'de> Deserialize<'de> for Celsius {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_checked(deserializer, Kind::Celsius).map(Self)
    }
}

impl JsonSchema for NonNegative {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("NonNegative")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "number",
            "minimum": 0.0,
            "description": "A non-negative figure, such as a hashrate or a power draw"
        })
    }
}

impl JsonSchema for Celsius {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Celsius")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "number",
            "exclusiveMinimum": ABSOLUTE_ZERO_C,
            "description": "A temperature in degrees Celsius, above absolute zero"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Celsius, NonNegative};

    #[test]
    fn accepts_ordinary_figures() {
        assert!(
            (json5::from_str::<NonNegative>("4.5")
                .expect("BUG: valid")
                .get()
                - 4.5)
                .abs()
                < f64::EPSILON
        );
        assert!(
            (json5::from_str::<Celsius>("-40").expect("BUG: valid").get() + 40.0).abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn zero_is_a_legitimate_figure() {
        // An idle miner really does draw down to nothing on these readings.
        assert!(json5::from_str::<NonNegative>("0").is_ok());
    }

    #[test]
    fn rejects_a_negative_figure_with_a_located_error() {
        let err = json5::from_str::<NonNegative>("-1.5").expect_err("BUG: must be rejected");
        let json5::Error::Message { msg, location } = err;
        assert!(msg.contains("must not be negative"), "message was: {msg}");
        assert!(location.is_some(), "the rejection must carry a location");
    }

    #[test]
    fn rejects_a_temperature_below_absolute_zero() {
        let err = json5::from_str::<Celsius>("-300").expect_err("BUG: must be rejected");
        let json5::Error::Message { msg, .. } = err;
        assert!(msg.contains("absolute zero"), "message was: {msg}");
    }

    #[test]
    fn rejects_a_non_finite_figure() {
        for literal in ["NaN", "Infinity", "-Infinity"] {
            assert!(
                json5::from_str::<NonNegative>(literal).is_err(),
                "{literal} must be rejected"
            );
        }
    }
}
