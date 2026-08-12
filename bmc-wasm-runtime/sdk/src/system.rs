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

//! Guest-side deck-wide system snapshot.
//!
//! Parallel to [`crate::params`]: an owned snapshot of the deck-wide settings
//! (timezone, formatting preferences, next-alarm) the host has delivered
//! to this widget instance. Widgets read fields through typed accessors
//! that walk the byte buffer lazily — no upfront tree allocation, no serde.
//!
//! ## Wire format
//!
//! Snapshots arrive from the host as a kind-tagged byte buffer in little-endian order.
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
//!
//! ## Snapshot lifecycle
//!
//! Mirrors [`crate::params`]: [`current`] returns the latest snapshot,
//! [`previous`] the one just before it. The `on_system_update`
//! lifecycle hook fires on system-snapshot changes (sibling of `on_params_update`
//! for the system channel); diff `current()` vs `previous()` inside the hook
//! to react only to fields whose value actually changed.

use bmc_wasm_protocol::system::SystemFieldKind;

// Re-export the wasmi-wire enums under `bmc_wasm_sdk::system::*`
// so widget authors writing `system::TimeFormat` don't have to
// dig through the protocol crate's module hierarchy.
//
// `SystemFieldKind` stays internal to the SDK's
// snapshot walker — widgets don't need it.
pub use bmc_wasm_protocol::system::{
    DateFormat, NumberFormat, TemperatureUnit, TimeFormat, UnitSystem, Weekday,
};

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

// `FromHostBytes` lets the generic snapshot cache construct `Snapshot`
// from the raw bytes the host writes. Mirror of the params impl.
#[cfg(any(target_arch = "wasm32", test))]
impl crate::snapshot_cache::FromHostBytes for Snapshot {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_bytes(bytes)
    }
}

/// Latest system snapshot delivered for this widget instance.
///
/// First call inside `init` fetches via `host_system_snapshot`. Subsequent
/// calls reuse the cached bytes until `host_system_version` changes; at that
/// point the old snapshot is moved into [`previous`] and the new one is
/// fetched.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn current() -> Snapshot {
    SYSTEM_CACHE.with(|c| crate::snapshot_cache::current_using(&WasmHost, &mut c.borrow_mut()))
}

/// Snapshot delivered immediately before [`current`].
///
/// [`Snapshot::default`] until at least one update has been observed.
/// Inside `on_system_update`, holds the just-replaced snapshot
/// — diff against [`current`] to react only to fields whose value actually changed.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn previous() -> Snapshot {
    SYSTEM_CACHE.with(|c| crate::snapshot_cache::previous_using(&WasmHost, &mut c.borrow_mut()))
}

// ── Native-target stubs ─────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
std::thread_local! {
    static NATIVE_SNAPSHOT: core::cell::RefCell<Snapshot> =
        const { core::cell::RefCell::new(Snapshot::empty()) };
}

/// Off-device there is no host to deliver a snapshot, so one is held per
/// thread. It starts empty: every accessor `None`, every caller left on
/// its own default.
///
/// [`set_current`] installs one, which is how the storybook and the tests
/// render a view the way an operator on imperial units, or a twelve-hour
/// clock, would see it.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn current() -> Snapshot {
    NATIVE_SNAPSHOT.with(|it| it.borrow().clone())
}

/// Install the snapshot [`current`] returns on this thread.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_current(snapshot: Snapshot) {
    NATIVE_SNAPSHOT.with(|it| *it.borrow_mut() = snapshot);
}

/// Always empty off-device: nothing delivers a second snapshot, so there
/// is no rotation to remember. Diffing it against [`current`] reports
/// every field as changed rather than nothing.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn previous() -> Snapshot {
    Snapshot::default()
}

// ── Wasm host bindings ──────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_system_snapshot(out_ptr: *mut u8, out_cap: u32) -> u32;
    fn host_system_version() -> u64;
}

#[cfg(target_arch = "wasm32")]
struct WasmHost;

#[cfg(target_arch = "wasm32")]
impl crate::snapshot_cache::HostSnapshotProvider for WasmHost {
    fn version(&self) -> u64 {
        // SAFETY: `host_system_version` has no out-params and is safe to call.
        unsafe { host_system_version() }
    }

    fn fill_snapshot(&self, out: &mut [u8]) -> usize {
        let cap = u32::try_from(out.len())
            .expect("BUG: snapshot buffer length must fit in u32 (wire-format guarantee)");
        let written = if out.is_empty() {
            // SAFETY: passing a null pointer is sound when `out_cap == 0` —
            // the host implementation explicitly checks the cap before writing
            // and returns the required length without touching the pointer.
            unsafe { host_system_snapshot(core::ptr::null_mut(), 0) }
        } else {
            // SAFETY: `out` is uniquely borrowed with length `cap`;
            // the host writes at most `out_cap` bytes starting at `out_ptr`.
            unsafe { host_system_snapshot(out.as_mut_ptr(), cap) }
        };
        usize::try_from(written).expect("BUG: host_system_snapshot return must fit in usize")
    }
}

// ── Cache state ─────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
std::thread_local! {
    static SYSTEM_CACHE: core::cell::RefCell<crate::snapshot_cache::Cache<Snapshot>> =
        core::cell::RefCell::new(crate::snapshot_cache::Cache::new());
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
    fn each_field_decodes_round_trip() {
        let s = SnapshotBuilder::new()
            .timezone("Europe/Bratislava")
            .time_format(TimeFormat::Hour12)
            .date_format(DateFormat::YyyyMmDdDot)
            .number_format(NumberFormat::CommaGroupDotDecimal)
            .first_day_of_week(Weekday::Sunday)
            .temperature_unit(TemperatureUnit::Fahrenheit)
            .unit_system(UnitSystem::Imperial)
            .next_alarm_some(1_700_000_000_000, "Wake up")
            .night_mode(true)
            .build();
        assert_eq!(s.timezone(), Some("Europe/Bratislava"));
        assert_eq!(s.time_format(), Some(TimeFormat::Hour12));
        assert_eq!(s.date_format(), Some(DateFormat::YyyyMmDdDot));
        assert_eq!(s.number_format(), Some(NumberFormat::CommaGroupDotDecimal));
        assert_eq!(s.first_day_of_week(), Some(Weekday::Sunday));
        assert_eq!(s.temperature_unit(), Some(TemperatureUnit::Fahrenheit));
        assert_eq!(s.unit_system(), Some(UnitSystem::Imperial));
        let next = s
            .next_alarm()
            .expect("BUG: sample bytes encode next_alarm = Some(...)");
        assert_eq!(next.fire_at_utc_ms, 1_700_000_000_000);
        assert_eq!(next.name, "Wake up");
        assert_eq!(s.night_mode(), Some(true));
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

    #[test]
    fn current_and_previous_are_none_on_native() {
        assert_eq!(current().timezone(), None);
        assert_eq!(previous().timezone(), None);
    }
}
