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

//! Wasmi-wire types for the deck-wide `system` snapshot channel.
//!
//! Mirrors the `params` split: only the *wire-level* surface lives here
//! — the enum tag values, the field-kind discriminator, and the decode-error shape
//! — so the host encoder (`bmc-wasm-runtime::system`) and the guest decoder
//! (`bmc-wasm-sdk::system`) cannot drift on the constants.
//!
//! The cross-validating round-trip test sits next to those wire constants
//! in this module's `tests`.
//!
//! The host side keeps using the rich domain types (`bmc_shared_time::TimeSystem`,
//! `bmc_shared_utils::unit_system::UnitSystem`, …) and converts to the wire enums at the encoder
//! boundary. The SDK side exposes the wire enums directly to widget authors.

/// Per-field discriminator tagging each entry in the `system` wire snapshot.
///
/// The encoder emits one entry per known field in declaration order,
/// each prefixed by its [`SystemFieldKind`] byte.
///
/// The decoder dispatches on the tag. Adding a new field to the snapshot
/// grows this enum (and both sides' field-by-field handling) without
/// renumbering existing tags — preserving forward compatibility across
/// SDK / host version skew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SystemFieldKind {
    Timezone = 0,
    TimeFormat = 1,
    DateFormat = 2,
    NumberFormat = 3,
    FirstDayOfWeek = 4,
    TemperatureUnit = 5,
    UnitSystem = 6,
    NextAlarm = 7,
    NightMode = 8,
}

impl TryFrom<u8> for SystemFieldKind {
    /// `Err(tag)` surfaces the unknown byte so the caller can include
    /// it in its own error variant.
    type Error = u8;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::Timezone),
            1 => Ok(Self::TimeFormat),
            2 => Ok(Self::DateFormat),
            3 => Ok(Self::NumberFormat),
            4 => Ok(Self::FirstDayOfWeek),
            5 => Ok(Self::TemperatureUnit),
            6 => Ok(Self::UnitSystem),
            7 => Ok(Self::NextAlarm),
            8 => Ok(Self::NightMode),
            _ => Err(tag),
        }
    }
}

/// 12-hour vs 24-hour clock display.
/// Wasmi-wire mirror of `bmc_shared_time::TimeSystem`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[repr(u8)]
pub enum TimeFormat {
    Hour12 = 0,
    #[default]
    Hour24 = 1,
}

impl TryFrom<u8> for TimeFormat {
    type Error = u8;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::Hour12),
            1 => Ok(Self::Hour24),
            _ => Err(tag),
        }
    }
}

/// Date layout / separator choice. Wasmi-wire mirror of `bmc_shared_time::DateFormat`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[repr(u8)]
pub enum DateFormat {
    #[default]
    DdMmYyyyDot = 0,
    DdMmYyyySlash = 1,
    #[cfg_attr(feature = "serde", serde(rename = "d_m_yyyy_slash"))]
    DMYyyySlash = 2,
    #[cfg_attr(feature = "serde", serde(rename = "m_d_yyyy_slash"))]
    MDYyyySlash = 3,
    DdMmYyyyDash = 4,
    #[cfg_attr(feature = "serde", serde(rename = "yyyy_m_d_slash"))]
    YyyyMDSlash = 5,
    YyyyMmDdDot = 6,
    YyyyMmDdDash = 7,
}

impl TryFrom<u8> for DateFormat {
    type Error = u8;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::DdMmYyyyDot),
            1 => Ok(Self::DdMmYyyySlash),
            2 => Ok(Self::DMYyyySlash),
            3 => Ok(Self::MDYyyySlash),
            4 => Ok(Self::DdMmYyyyDash),
            5 => Ok(Self::YyyyMDSlash),
            6 => Ok(Self::YyyyMmDdDot),
            7 => Ok(Self::YyyyMmDdDash),
            _ => Err(tag),
        }
    }
}

/// Group / decimal separator choice.
/// Wasmi-wire mirror of `bmc_shared_utils::NumberFormat`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[repr(u8)]
pub enum NumberFormat {
    /// `1 234 567,89`
    #[default]
    SpaceGroupCommaDecimal = 0,
    /// `1,234,567.89`
    CommaGroupDotDecimal = 1,
    /// `1.234.567,89`
    DotGroupCommaDecimal = 2,
    /// `1 234 567.89`
    SpaceGroupDotDecimal = 3,
}

impl TryFrom<u8> for NumberFormat {
    type Error = u8;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::SpaceGroupCommaDecimal),
            1 => Ok(Self::CommaGroupDotDecimal),
            2 => Ok(Self::DotGroupCommaDecimal),
            3 => Ok(Self::SpaceGroupDotDecimal),
            _ => Err(tag),
        }
    }
}

/// First day of the week for calendar widgets.
/// Wasmi-wire mirror of `bmc_shared_time::WeekDay`.
///
/// Wire form uses 1-indexed Monday-first ordering, matching
/// the host enum's numeric layout exactly — `host_value as u8`
/// is a zero-cost cast at the encoder and the SDK exposes
/// the same Monday=1..Sunday=7 semantics widget authors
/// will recognise from the host-side rustdoc.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[repr(u8)]
pub enum Weekday {
    #[default]
    Monday = 1,
    Tuesday = 2,
    Wednesday = 3,
    Thursday = 4,
    Friday = 5,
    Saturday = 6,
    Sunday = 7,
}

impl TryFrom<u8> for Weekday {
    type Error = u8;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            1 => Ok(Self::Monday),
            2 => Ok(Self::Tuesday),
            3 => Ok(Self::Wednesday),
            4 => Ok(Self::Thursday),
            5 => Ok(Self::Friday),
            6 => Ok(Self::Saturday),
            7 => Ok(Self::Sunday),
            _ => Err(tag),
        }
    }
}

/// Operator-selected temperature unit.
/// Wasmi-wire mirror of `bmc_shared_utils::TemperatureUnit`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[repr(u8)]
pub enum TemperatureUnit {
    #[default]
    Celsius = 0,
    Fahrenheit = 1,
}

impl TryFrom<u8> for TemperatureUnit {
    type Error = u8;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::Celsius),
            1 => Ok(Self::Fahrenheit),
            _ => Err(tag),
        }
    }
}

/// Operator-selected measurement system for non-temperature units (km vs miles, kg vs lbs).
/// Wasmi-wire mirror of `bmc_shared_utils::unit_system::UnitSystem`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[repr(u8)]
pub enum UnitSystem {
    #[default]
    Metric = 0,
    Imperial = 1,
}

impl TryFrom<u8> for UnitSystem {
    type Error = u8;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::Metric),
            1 => Ok(Self::Imperial),
            _ => Err(tag),
        }
    }
}

// Wire → domain conversions, feature-gated so the wasm SDK path stays formato-free.
#[cfg(feature = "domain")]
impl From<NumberFormat> for bmc_shared_utils::number_format::NumberFormat {
    fn from(value: NumberFormat) -> Self {
        match value {
            NumberFormat::SpaceGroupCommaDecimal => Self::SpaceGroupCommaDecimal,
            NumberFormat::CommaGroupDotDecimal => Self::CommaGroupDotDecimal,
            NumberFormat::DotGroupCommaDecimal => Self::DotGroupCommaDecimal,
            NumberFormat::SpaceGroupDotDecimal => Self::SpaceGroupDotDecimal,
        }
    }
}

#[cfg(feature = "domain")]
impl From<TemperatureUnit> for bmc_shared_utils::temperature::TemperatureUnit {
    fn from(value: TemperatureUnit) -> Self {
        match value {
            TemperatureUnit::Celsius => Self::Celsius,
            TemperatureUnit::Fahrenheit => Self::Fahrenheit,
        }
    }
}

#[cfg(feature = "domain")]
impl From<UnitSystem> for bmc_shared_utils::unit_system::UnitSystem {
    fn from(value: UnitSystem) -> Self {
        match value {
            UnitSystem::Metric => Self::Metric,
            UnitSystem::Imperial => Self::Imperial,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_field_kind_round_trips_every_tag() {
        let expected = [
            (0, SystemFieldKind::Timezone),
            (1, SystemFieldKind::TimeFormat),
            (2, SystemFieldKind::DateFormat),
            (3, SystemFieldKind::NumberFormat),
            (4, SystemFieldKind::FirstDayOfWeek),
            (5, SystemFieldKind::TemperatureUnit),
            (6, SystemFieldKind::UnitSystem),
            (7, SystemFieldKind::NextAlarm),
            (8, SystemFieldKind::NightMode),
        ];
        for (tag, variant) in expected {
            assert_eq!(
                SystemFieldKind::try_from(tag)
                    .expect("BUG: tag in test table must round-trip to its variant"),
                variant
            );
            assert_eq!(variant as u8, tag);
        }
        assert_eq!(SystemFieldKind::try_from(9), Err(9));
        assert_eq!(SystemFieldKind::try_from(255), Err(255));
    }

    #[test]
    fn time_format_round_trips_every_tag() {
        for (tag, variant) in [(0, TimeFormat::Hour12), (1, TimeFormat::Hour24)] {
            assert_eq!(
                TimeFormat::try_from(tag)
                    .expect("BUG: tag in test table must round-trip to its variant"),
                variant
            );
            assert_eq!(variant as u8, tag);
        }
        assert_eq!(TimeFormat::try_from(2), Err(2));
    }

    #[test]
    fn date_format_round_trips_every_tag() {
        let expected = [
            (0, DateFormat::DdMmYyyyDot),
            (1, DateFormat::DdMmYyyySlash),
            (2, DateFormat::DMYyyySlash),
            (3, DateFormat::MDYyyySlash),
            (4, DateFormat::DdMmYyyyDash),
            (5, DateFormat::YyyyMDSlash),
            (6, DateFormat::YyyyMmDdDot),
            (7, DateFormat::YyyyMmDdDash),
        ];
        for (tag, variant) in expected {
            assert_eq!(
                DateFormat::try_from(tag)
                    .expect("BUG: tag in test table must round-trip to its variant"),
                variant
            );
            assert_eq!(variant as u8, tag);
        }
        assert_eq!(DateFormat::try_from(8), Err(8));
    }

    #[test]
    fn number_format_round_trips_every_tag() {
        let expected = [
            (0, NumberFormat::SpaceGroupCommaDecimal),
            (1, NumberFormat::CommaGroupDotDecimal),
            (2, NumberFormat::DotGroupCommaDecimal),
            (3, NumberFormat::SpaceGroupDotDecimal),
        ];
        for (tag, variant) in expected {
            assert_eq!(
                NumberFormat::try_from(tag)
                    .expect("BUG: tag in test table must round-trip to its variant"),
                variant
            );
            assert_eq!(variant as u8, tag);
        }
        assert_eq!(NumberFormat::try_from(4), Err(4));
    }

    #[test]
    fn weekday_round_trips_every_tag() {
        let expected = [
            (1, Weekday::Monday),
            (2, Weekday::Tuesday),
            (3, Weekday::Wednesday),
            (4, Weekday::Thursday),
            (5, Weekday::Friday),
            (6, Weekday::Saturday),
            (7, Weekday::Sunday),
        ];
        for (tag, variant) in expected {
            assert_eq!(
                Weekday::try_from(tag)
                    .expect("BUG: tag in test table must round-trip to its variant"),
                variant
            );
            assert_eq!(variant as u8, tag);
        }
        assert_eq!(Weekday::try_from(0), Err(0));
        assert_eq!(Weekday::try_from(8), Err(8));
    }

    #[test]
    fn temperature_unit_round_trips_every_tag() {
        for (tag, variant) in [
            (0, TemperatureUnit::Celsius),
            (1, TemperatureUnit::Fahrenheit),
        ] {
            assert_eq!(
                TemperatureUnit::try_from(tag)
                    .expect("BUG: tag in test table must round-trip to its variant"),
                variant
            );
            assert_eq!(variant as u8, tag);
        }
        assert_eq!(TemperatureUnit::try_from(2), Err(2));
    }

    #[test]
    fn unit_system_round_trips_every_tag() {
        for (tag, variant) in [(0, UnitSystem::Metric), (1, UnitSystem::Imperial)] {
            assert_eq!(
                UnitSystem::try_from(tag)
                    .expect("BUG: tag in test table must round-trip to its variant"),
                variant
            );
            assert_eq!(variant as u8, tag);
        }
        assert_eq!(UnitSystem::try_from(2), Err(2));
    }
}
