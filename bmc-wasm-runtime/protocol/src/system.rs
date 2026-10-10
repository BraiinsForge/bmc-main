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

//! Wire format, reader and builder for the deck-wide `system` snapshot channel.
//!
//! The host encoder (`bmc-wasm-runtime::system`) and the [`Snapshot`] reader build
//! on the same enum tags and field-kind discriminator, so the two cannot drift on
//! the constants; the round-trip tests guard the layout.
//!
//! `bmc-wasm-sdk::system` re-exports the enums, the reader
//! and the builder, and adds the host-fed snapshot lifecycle.
//!
//! A [`Snapshot`] is an owned copy of the deck-wide settings (timezone, formatting
//! preferences, next-alarm) the host delivered to a widget instance. Fields are read
//! through typed accessors that walk the byte buffer lazily — no upfront tree
//! allocation, no serde.
//!
//! The host side keeps using the rich domain types (`bmc_shared_time::TimeSystem`,
//! `bmc_shared_utils::unit_system::UnitSystem`, …) and converts to the wire enums at the encoder
//! boundary. The SDK side exposes the wire enums directly to widget authors.
//!
//! ## Wire format
//!
//! Snapshots are a kind-tagged byte buffer in little-endian order.
//! Each entry is prefixed by a [`SystemFieldKind`] byte; the payload is field-specific:
//!
//! ```text
//! u32  count
//! for each entry:
//!   u8   kind = SystemFieldKind::*
//!   variant payload:
//!     Timezone        → u16 len, utf-8 bytes
//!     TimeFormat      → u8 wire tag
//!     DateFormat      → u8 wire tag
//!     NumberFormat    → u8 wire tag
//!     FirstDayOfWeek  → u8 wire tag
//!     TemperatureUnit → u8 wire tag
//!     UnitSystem      → u8 wire tag
//!     NextAlarm       → u8 present (0 = None, 1 = Some);
//!                       if present, i64 LE fire_at_utc_ms + u16 LE
//!                       name_len + utf-8 bytes
//!     NightMode       → u8 active (0 = inactive, 1 = active)
//! ```

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
#[cfg(feature = "host")]
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

#[cfg(feature = "host")]
impl From<TemperatureUnit> for bmc_shared_utils::temperature::TemperatureUnit {
    fn from(value: TemperatureUnit) -> Self {
        match value {
            TemperatureUnit::Celsius => Self::Celsius,
            TemperatureUnit::Fahrenheit => Self::Fahrenheit,
        }
    }
}

#[cfg(feature = "host")]
impl From<UnitSystem> for bmc_shared_utils::unit_system::UnitSystem {
    fn from(value: UnitSystem) -> Self {
        match value {
            UnitSystem::Metric => Self::Metric,
            UnitSystem::Imperial => Self::Imperial,
        }
    }
}

/// Owned snapshot of the host-delivered deck-wide system state.
///
/// Cheap to [`Clone`] — the underlying storage is a single `Vec<u8>`.
/// Accessors return borrowed views into the snapshot bytes,
/// so a `Snapshot` reference must outlive any view it produces.
#[derive(Clone, Default, Debug)]
pub struct Snapshot {
    bytes: Vec<u8>,
}

/// Borrowed view into a [`Snapshot`]'s next-alarm entry.
///
/// Lifetime tied to the parent `Snapshot` so the name slice
/// never outlives the buffer that backs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextAlarmView<'a> {
    /// UTC milliseconds since the Unix epoch at which the alarm
    /// fires next. Timezone-invariant; pair with [`Snapshot::timezone`]
    /// for local-time rendering.
    pub fire_at_utc_ms: i64,
    /// Operator-typed display name.
    pub name: &'a str,
}

impl Snapshot {
    /// Build a `Snapshot` from an owned packed-byte buffer.
    ///
    /// The host owns the wire layout; a malformed buffer here
    /// means the host messed up, not the widget.
    /// Accessors return `None` for a missing or malformed entry
    /// — widgets pick the fallback that fits the rendering
    /// at each call site.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// A snapshot carrying nothing, in a `const` context.
    #[must_use]
    pub const fn empty() -> Self {
        Self { bytes: Vec::new() }
    }

    /// IANA timezone identifier (e.g. `Europe/Bratislava`).
    #[must_use]
    pub fn timezone(&self) -> Option<&str> {
        self.find(SystemFieldKind::Timezone)
            .and_then(|payload| read_str(payload))
    }

    #[must_use]
    pub fn time_format(&self) -> Option<TimeFormat> {
        self.find(SystemFieldKind::TimeFormat)
            .and_then(read_u8_tag)
            .and_then(|t| TimeFormat::try_from(t).ok())
    }

    #[must_use]
    pub fn date_format(&self) -> Option<DateFormat> {
        self.find(SystemFieldKind::DateFormat)
            .and_then(read_u8_tag)
            .and_then(|t| DateFormat::try_from(t).ok())
    }

    #[must_use]
    pub fn number_format(&self) -> Option<NumberFormat> {
        self.find(SystemFieldKind::NumberFormat)
            .and_then(read_u8_tag)
            .and_then(|t| NumberFormat::try_from(t).ok())
    }

    #[must_use]
    pub fn first_day_of_week(&self) -> Option<Weekday> {
        self.find(SystemFieldKind::FirstDayOfWeek)
            .and_then(read_u8_tag)
            .and_then(|t| Weekday::try_from(t).ok())
    }

    #[must_use]
    pub fn temperature_unit(&self) -> Option<TemperatureUnit> {
        self.find(SystemFieldKind::TemperatureUnit)
            .and_then(read_u8_tag)
            .and_then(|t| TemperatureUnit::try_from(t).ok())
    }

    #[must_use]
    pub fn unit_system(&self) -> Option<UnitSystem> {
        self.find(SystemFieldKind::UnitSystem)
            .and_then(read_u8_tag)
            .and_then(|t| UnitSystem::try_from(t).ok())
    }

    /// Whether deck-wide night mode is currently active.
    /// Derived host-side from the enable toggle, the operator
    /// configured time interval, and the wall-clock crossing
    /// the interval bounds — widgets only see the resolved bool.
    #[must_use]
    pub fn night_mode(&self) -> Option<bool> {
        self.find(SystemFieldKind::NightMode)
            .and_then(read_u8_tag)
            .map(|b| b != 0)
    }

    /// Resolved next-to-fire alarm, or `None` when no alarm is scheduled.
    ///
    /// The returned view borrows the alarm's display name out of `self.bytes`;
    /// the snapshot must outlive any view it produces.
    #[must_use]
    pub fn next_alarm(&self) -> Option<NextAlarmView<'_>> {
        let payload = self.find(SystemFieldKind::NextAlarm)?;
        // Wire spec: 0 = None, 1 = Some. Anything else is malformed —
        // reject it as None rather than fabricate an alarm from garbage.
        match *payload.first()? {
            1 => {}
            _ => return None,
        }
        // present == 1 → i64 (8) + u16 name_len + name bytes
        let after_flag = payload.get(1..)?;
        let fire = after_flag.first_chunk::<8>()?;
        let fire_at_utc_ms = i64::from_le_bytes(*fire);
        let after_fire = after_flag.get(8..)?;
        let name_len = u16::from_le_bytes(*after_fire.first_chunk::<2>()?) as usize;
        let name_bytes = after_fire.get(2..2 + name_len)?;
        let name = core::str::from_utf8(name_bytes).ok()?;
        Some(NextAlarmView {
            fire_at_utc_ms,
            name,
        })
    }

    /// Walk the buffer looking for the entry tagged with `kind`.
    /// Returns the slice starting at the entry's payload
    /// (i.e. immediately after the kind byte).
    ///
    /// Stops at the first parse error and returns `None`.
    fn find(&self, kind: SystemFieldKind) -> Option<&[u8]> {
        let mut offset = 4_usize;
        let count = self
            .bytes
            .first_chunk::<4>()
            .map(|h| u32::from_le_bytes(*h))?;
        let mut remaining = count;
        while remaining > 0 {
            let entry_kind_byte = *self.bytes.get(offset)?;
            offset = offset.checked_add(1)?;
            let entry_kind = SystemFieldKind::try_from(entry_kind_byte).ok()?;
            let payload_start = offset;
            let payload_len = entry_payload_len(&self.bytes, payload_start, entry_kind)?;
            if entry_kind == kind {
                let end = payload_start.checked_add(payload_len)?;
                return self.bytes.get(payload_start..end);
            }
            offset = offset.checked_add(payload_len)?;
            remaining -= 1;
        }
        None
    }
}

/// Length in bytes of the payload following a given field's kind byte.
/// Reads from `bytes` starting at `offset`. Returns `None` on truncation.
fn entry_payload_len(bytes: &[u8], offset: usize, kind: SystemFieldKind) -> Option<usize> {
    match kind {
        SystemFieldKind::Timezone => {
            let len = u16::from_le_bytes(*bytes.get(offset..)?.first_chunk::<2>()?) as usize;
            Some(2 + len)
        }
        SystemFieldKind::TimeFormat
        | SystemFieldKind::DateFormat
        | SystemFieldKind::NumberFormat
        | SystemFieldKind::FirstDayOfWeek
        | SystemFieldKind::TemperatureUnit
        | SystemFieldKind::UnitSystem
        | SystemFieldKind::NightMode => Some(1),
        SystemFieldKind::NextAlarm => match *bytes.get(offset)? {
            0 => Some(1),
            1 => {
                // present(1) + fire_at_utc_ms(8) + name_len(2) + name bytes
                let name_len_off = offset.checked_add(9)?;
                let name_len =
                    u16::from_le_bytes(*bytes.get(name_len_off..)?.first_chunk::<2>()?) as usize;
                Some(1 + 8 + 2 + name_len)
            }
            _ => None,
        },
    }
}

fn read_u8_tag(payload: &[u8]) -> Option<u8> {
    payload.first().copied()
}

fn read_str(payload: &[u8]) -> Option<&str> {
    let len = u16::from_le_bytes(*payload.first_chunk::<2>()?) as usize;
    let bytes = payload.get(2..2 + len)?;
    core::str::from_utf8(bytes).ok()
}

/// Build a snapshot in the wire format the host emits, for the callers
/// that have no host: the storybook and the tests.
///
/// Mirrors the host-side encoder in `bmc-wasm-runtime/src/system.rs`.
#[derive(Debug, Default)]
pub struct SnapshotBuilder {
    out: Vec<u8>,
    count: u32,
}

impl SnapshotBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            out: vec![0; 4],
            count: 0,
        }
    }

    #[must_use]
    pub fn timezone(mut self, tz: &str) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::Timezone as u8);
        let len = u16::try_from(tz.len()).expect("BUG: timezone fits in u16");
        self.out.extend_from_slice(&len.to_le_bytes());
        self.out.extend_from_slice(tz.as_bytes());
        self
    }

    #[must_use]
    pub fn time_format(mut self, t: TimeFormat) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::TimeFormat as u8);
        self.out.push(t as u8);
        self
    }

    #[must_use]
    pub fn date_format(mut self, d: DateFormat) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::DateFormat as u8);
        self.out.push(d as u8);
        self
    }

    #[must_use]
    pub fn number_format(mut self, n: NumberFormat) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::NumberFormat as u8);
        self.out.push(n as u8);
        self
    }

    #[must_use]
    pub fn first_day_of_week(mut self, w: Weekday) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::FirstDayOfWeek as u8);
        self.out.push(w as u8);
        self
    }

    #[must_use]
    pub fn temperature_unit(mut self, u: TemperatureUnit) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::TemperatureUnit as u8);
        self.out.push(u as u8);
        self
    }

    #[must_use]
    pub fn unit_system(mut self, u: UnitSystem) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::UnitSystem as u8);
        self.out.push(u as u8);
        self
    }

    #[must_use]
    pub fn next_alarm_some(mut self, fire_at_utc_ms: i64, name: &str) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::NextAlarm as u8);
        self.out.push(1); // present
        self.out.extend_from_slice(&fire_at_utc_ms.to_le_bytes());
        let len = u16::try_from(name.len()).expect("BUG: alarm name fits in u16");
        self.out.extend_from_slice(&len.to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
        self
    }

    #[must_use]
    pub fn next_alarm_none(mut self) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::NextAlarm as u8);
        self.out.push(0);
        self
    }

    #[must_use]
    pub fn night_mode(mut self, active: bool) -> Self {
        self.count += 1;
        self.out.push(SystemFieldKind::NightMode as u8);
        self.out.push(u8::from(active));
        self
    }

    #[must_use]
    pub fn build(self) -> Snapshot {
        Snapshot::from_bytes(self.build_bytes())
    }

    /// The packed buffer itself, for tests that corrupt or truncate it.
    #[must_use]
    pub fn build_bytes(mut self) -> Vec<u8> {
        let head = self.count.to_le_bytes();
        self.out[0..4].copy_from_slice(&head);
        self.out
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

    #[test]
    fn default_snapshot_yields_none_on_every_field() {
        let s = Snapshot::default();
        assert_eq!(s.timezone(), None);
        assert_eq!(s.time_format(), None);
        assert_eq!(s.date_format(), None);
        assert_eq!(s.number_format(), None);
        assert_eq!(s.first_day_of_week(), None);
        assert_eq!(s.temperature_unit(), None);
        assert_eq!(s.unit_system(), None);
        assert_eq!(s.next_alarm(), None);
        assert_eq!(s.night_mode(), None);
    }

    #[test]
    fn each_field_decodes_its_own_entry() {
        struct Fields {
            timezone: &'static str,
            time: TimeFormat,
            date: DateFormat,
            number: NumberFormat,
            weekday: Weekday,
            temperature: TemperatureUnit,
            units: UnitSystem,
            alarm: (i64, &'static str),
            night: bool,
        }

        // The two-valued fields take the patterns 001, 010, 100 and 011 across the three,
        // apart from each other and from the nonzero bytes `night_mode` would misread as true.
        // The other fields change too, so a reader that ignores its entry fails.
        let snapshots = [
            Fields {
                timezone: "Europe/Bratislava",
                time: TimeFormat::Hour12,
                date: DateFormat::YyyyMmDdDot,
                number: NumberFormat::SpaceGroupDotDecimal,
                weekday: Weekday::Sunday,
                temperature: TemperatureUnit::Celsius,
                units: UnitSystem::Imperial,
                alarm: (1_700_000_000_000, "Wake up"),
                night: false,
            },
            Fields {
                timezone: "America/New_York",
                time: TimeFormat::Hour12,
                date: DateFormat::DdMmYyyyDash,
                number: NumberFormat::DotGroupCommaDecimal,
                weekday: Weekday::Friday,
                temperature: TemperatureUnit::Fahrenheit,
                units: UnitSystem::Metric,
                alarm: (1_800_000_000_000, "Stand-up"),
                night: true,
            },
            Fields {
                timezone: "Asia/Tokyo",
                time: TimeFormat::Hour24,
                date: DateFormat::YyyyMDSlash,
                number: NumberFormat::SpaceGroupDotDecimal,
                weekday: Weekday::Saturday,
                temperature: TemperatureUnit::Celsius,
                units: UnitSystem::Metric,
                alarm: (1_900_000_000_000, "Lunch"),
                night: true,
            },
        ];
        for fields in snapshots {
            let s = SnapshotBuilder::new()
                .timezone(fields.timezone)
                .time_format(fields.time)
                .date_format(fields.date)
                .number_format(fields.number)
                .first_day_of_week(fields.weekday)
                .temperature_unit(fields.temperature)
                .unit_system(fields.units)
                .next_alarm_some(fields.alarm.0, fields.alarm.1)
                .night_mode(fields.night)
                .build();
            assert_eq!(s.timezone(), Some(fields.timezone));
            assert_eq!(s.time_format(), Some(fields.time));
            assert_eq!(s.date_format(), Some(fields.date));
            assert_eq!(s.number_format(), Some(fields.number));
            assert_eq!(s.first_day_of_week(), Some(fields.weekday));
            assert_eq!(s.temperature_unit(), Some(fields.temperature));
            assert_eq!(s.unit_system(), Some(fields.units));
            let next = s
                .next_alarm()
                .expect("BUG: every snapshot here encodes an alarm");
            assert_eq!((next.fire_at_utc_ms, next.name), fields.alarm);
            assert_eq!(s.night_mode(), Some(fields.night));
        }
    }

    #[test]
    fn night_mode_inactive_decodes_false() {
        let s = SnapshotBuilder::new().night_mode(false).build();
        assert_eq!(s.night_mode(), Some(false));
    }

    #[test]
    fn night_mode_missing_entry_returns_none() {
        // No NightMode entry — accessor must yield None.
        let s = SnapshotBuilder::new()
            .timezone("UTC")
            .next_alarm_none()
            .build();
        assert_eq!(s.night_mode(), None);
    }

    #[test]
    fn next_alarm_none_returns_none() {
        let s = SnapshotBuilder::new().next_alarm_none().build();
        assert_eq!(s.next_alarm(), None);
    }

    #[test]
    fn next_alarm_rejects_invalid_present_byte() {
        // Wire spec: 0 = None, 1 = Some. A non-{0,1} present byte
        // must yield None rather than fabricate an alarm out of garbage.
        // Guards against a one-bit flip in the byte stream materialising
        // a phantom alarm.
        for present in [2_u8, 7, 42, 255] {
            let mut bytes = SnapshotBuilder::new()
                .next_alarm_some(1_700_000_000_000, "garbage")
                .build_bytes();
            // Locate and overwrite the present byte (the byte after
            // SystemFieldKind::NextAlarm in the stream).
            let tag = SystemFieldKind::NextAlarm as u8;
            let tag_idx = bytes
                .iter()
                .position(|&b| b == tag)
                .expect("BUG: builder emitted NextAlarm tag");
            bytes[tag_idx + 1] = present;
            let s = Snapshot::from_bytes(bytes);
            assert_eq!(
                s.next_alarm(),
                None,
                "present byte {present} must reject as None"
            );
        }
    }

    #[test]
    fn truncated_buffer_yields_none_past_the_cut() {
        let mut bytes = SnapshotBuilder::new()
            .timezone("Europe/Bratislava")
            .time_format(TimeFormat::Hour12)
            .build_bytes();
        bytes.truncate(bytes.len() - 1);
        let s = Snapshot::from_bytes(bytes);
        // Timezone fits before the truncation.
        // time_format can't be reached after the truncation
        // removes its payload byte, so the accessor yields None.
        assert_eq!(s.timezone(), Some("Europe/Bratislava"));
        assert_eq!(s.time_format(), None);
    }
}
