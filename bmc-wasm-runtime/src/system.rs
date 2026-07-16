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

//! Host-side `SystemSnapshot` bundling deck-wide settings and the next-alarm
//! state delivered to wasm widgets.
//!
//! The fields are typed against the wasmi-wire enums in
//! [`bmc_wasm_protocol::system`]. `bmc-wasm-runtime` intentionally has no
//! dependency on `bmc` / `bmc-shared-*`; the bmc-shared-side rust-land enums
//! get converted to these wasmi-wire enums at the widget-binary boundary
//! (see `widgets/wasm/src/wayland.rs`).
//!
//! ## Wire format
//!
//! Kind-tagged, mirroring the `params` channel: a `u32` LE entry count, then
//! one entry per known field. Each entry begins with a
//! [`SystemFieldKind`] byte; the payload that follows is field-specific:
//!
//! ```text
//! u32 entry_count
//!  entries… each:
//!   u8 kind = SystemFieldKind::Timezone        → u16 len, utf-8 bytes
//!   u8 kind = SystemFieldKind::TimeFormat      → u8 wire tag
//!   u8 kind = SystemFieldKind::DateFormat      → u8 wire tag
//!   u8 kind = SystemFieldKind::NumberFormat    → u8 wire tag
//!   u8 kind = SystemFieldKind::FirstDayOfWeek  → u8 wire tag
//!   u8 kind = SystemFieldKind::TemperatureUnit → u8 wire tag
//!   u8 kind = SystemFieldKind::UnitSystem      → u8 wire tag
//!   u8 kind = SystemFieldKind::NextAlarm       → u8 present (0 = None, 1 = Some),
//!                                                if present followed by i64 LE fire_at_utc_ms + u16 LE name_len + utf-8 bytes
//!   u8 kind = SystemFieldKind::NightMode       → u8 active (0 = inactive, 1 = active)
//! ```
//!
//! Per-field kind-tagging keeps wire layout extensible:
//! a future field gets a new `SystemFieldKind` variant without renumbering
//! and without bumping a version. The SDK decoder dispatches on the byte
//! and silently skips unknown kinds — widgets compiled against an older
//! SDK degrade to default field values rather than refusing the snapshot.

use bmc_wasm_protocol::system::{
    DateFormat, NumberFormat, SystemFieldKind, TemperatureUnit, TimeFormat, UnitSystem, Weekday,
};
use bmc_wasm_protocol::versioned_snapshot::WireEncode;

/// Deck-wide system state delivered to wasm widgets.
///
/// Bundles operator settings and the resolved next-alarm entry.
/// The widget binary maintains this snapshot, applying each per-field
/// wayland delta locally, and pushes the bundle into the runtime
/// via `WasmWidgetRuntime::deliver_system_update`.
///
/// Doubles as the fixture-on-disk shape: `serde` derives round-trip
/// through the JSON form recorded by the testbed and replayed
/// by capture, so the schema check fires at fixture-load time
/// rather than at replay-time `.expect`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SystemSnapshot {
    #[serde(default)]
    pub settings: SystemSettings,
    #[serde(default)]
    pub next_alarm: Option<NextAlarm>,
    /// Whether deck-wide night mode is currently active.
    /// Derived by `bmc::night_mode::NightModeController` from
    /// the enable toggle, the operator-configured time interval,
    /// and the wall-clock crossing the interval bounds; only
    /// the derived bool reaches widgets.
    #[serde(default)]
    pub night_mode: bool,
}

/// Operator-controlled settings affecting widget rendering.
///
/// Sourced from `SystemService.GetTimezone` and `ConfigurationService.GetGeneralSettingsData`
/// on the compositor side, fanned out as per-field [`bmc_widget_protocol::SettingUpdate`] events
/// on the wayland wire, and accumulated into this struct on the widget binary.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SystemSettings {
    /// IANA timezone identifier (e.g. `Europe/Bratislava`).
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub time_format: TimeFormat,
    #[serde(default)]
    pub date_format: DateFormat,
    #[serde(default)]
    pub number_format: NumberFormat,
    #[serde(default)]
    pub first_day_of_week: Weekday,
    #[serde(default)]
    pub temperature_unit: TemperatureUnit,
    #[serde(default)]
    pub unit_system: UnitSystem,
}

/// Resolved soonest upcoming alarm.
///
/// The full alarm list stays host-side; widgets only see
/// this derived entry to avoid replicating schedule resolution.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NextAlarm {
    /// UTC milliseconds since the Unix epoch at which the alarm
    /// fires next. Timezone-invariant; widgets pair this with
    /// `Snapshot::timezone()` for local-time rendering.
    pub fire_at_utc_ms: i64,
    /// Display name shown alongside the time (operator-typed).
    pub name: String,
}

/// Encode a [`SystemSnapshot`] into the wasmi-wire bytes the SDK decoder parses.
///
/// See the module-level rustdoc for the byte layout.
#[must_use]
pub fn encode(snapshot: &SystemSnapshot) -> Vec<u8> {
    let mut out = Vec::with_capacity(estimate_size(snapshot));

    // Reserve four bytes for the entry count; written at the end so each
    // append below is a straight push.
    out.extend_from_slice(&0_u32.to_le_bytes());
    let mut count: u32 = 0;

    push_kind(&mut out, SystemFieldKind::Timezone);
    push_str(&mut out, &snapshot.settings.timezone);
    count += 1;

    push_kind(&mut out, SystemFieldKind::TimeFormat);
    out.push(snapshot.settings.time_format as u8);
    count += 1;

    push_kind(&mut out, SystemFieldKind::DateFormat);
    out.push(snapshot.settings.date_format as u8);
    count += 1;

    push_kind(&mut out, SystemFieldKind::NumberFormat);
    out.push(snapshot.settings.number_format as u8);
    count += 1;

    push_kind(&mut out, SystemFieldKind::FirstDayOfWeek);
    out.push(snapshot.settings.first_day_of_week as u8);
    count += 1;

    push_kind(&mut out, SystemFieldKind::TemperatureUnit);
    out.push(snapshot.settings.temperature_unit as u8);
    count += 1;

    push_kind(&mut out, SystemFieldKind::UnitSystem);
    out.push(snapshot.settings.unit_system as u8);
    count += 1;

    push_kind(&mut out, SystemFieldKind::NextAlarm);
    match &snapshot.next_alarm {
        None => out.push(0),
        Some(next) => {
            out.push(1);
            out.extend_from_slice(&next.fire_at_utc_ms.to_le_bytes());
            push_str(&mut out, &next.name);
        }
    }
    count += 1;

    push_kind(&mut out, SystemFieldKind::NightMode);
    out.push(u8::from(snapshot.night_mode));
    count += 1;

    out[0..4].copy_from_slice(&count.to_le_bytes());
    out
}

impl WireEncode for SystemSnapshot {
    fn encode(&self) -> Vec<u8> {
        encode(self)
    }
}

fn push_kind(out: &mut Vec<u8>, kind: SystemFieldKind) {
    out.push(kind as u8);
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    // Defensive truncation at the wire layer. Upstream caps (gRPC
    // `ALARM_NAME_MAX_BYTES`, compositor `cap_alarm_name`) mean strings
    // reaching here are already ≤ 256 bytes in production; the `u16::MAX`
    // cap below is belt-and-braces for fixtures / future fields.
    // Walks back to a UTF-8 char boundary so the SDK's `from_utf8` decode
    // doesn't reject the field for a mid-codepoint cut.
    let original_len = s.len();
    let mut len = original_len.min(u16::MAX as usize);
    while len > 0 && !s.is_char_boundary(len) {
        len -= 1;
    }
    if len < original_len {
        tracing::warn!(
            original_len,
            truncated_to = len,
            "wasmi-wire string truncated at encoder; upstream input cap should have caught this"
        );
    }
    let len_u16 = u16::try_from(len).expect("BUG: len capped at u16::MAX");
    out.extend_from_slice(&len_u16.to_le_bytes());
    out.extend_from_slice(&s.as_bytes()[..len]);
}

fn estimate_size(snapshot: &SystemSnapshot) -> usize {
    // 4-byte count header
    // + per-field framing: 1 kind byte + payload
    //   timezone   : 1 + 2 + len
    //   six enums  : 1 + 1 = 2 each, six times
    //   next_alarm : 1 + 1 [+ 8 + 2 + name_len when Some]
    //   night_mode : 1 + 1
    let mut total = 4 + 1 + 2 + snapshot.settings.timezone.len() + 6 * 2 + 2 + 2;
    if let Some(next) = &snapshot.next_alarm {
        total += 8 + 2 + next.name.len();
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SystemSnapshot {
        SystemSnapshot {
            settings: SystemSettings {
                timezone: "Europe/Bratislava".into(),
                time_format: TimeFormat::Hour12,
                date_format: DateFormat::YyyyMmDdDot,
                number_format: NumberFormat::CommaGroupDotDecimal,
                first_day_of_week: Weekday::Sunday,
                temperature_unit: TemperatureUnit::Fahrenheit,
                unit_system: UnitSystem::Imperial,
            },
            next_alarm: Some(NextAlarm {
                fire_at_utc_ms: 1_700_000_000_000,
                name: "Wake up".into(),
            }),
            night_mode: true,
        }
    }

    #[test]
    fn encode_emits_correct_count_header_and_kind_bytes() {
        let bytes = encode(&sample());
        // count = 9 (timezone + 6 enums + next_alarm + night_mode)
        assert_eq!(&bytes[0..4], &9_u32.to_le_bytes());

        // First entry's kind byte is `Timezone`.
        assert_eq!(bytes[4], SystemFieldKind::Timezone as u8);
    }

    #[test]
    fn encode_round_trips_through_decode_path() {
        // The decoder lives SDK-side; here we sanity-check the layout
        // by walking the kind-tagged stream and confirming every field
        // serialises to its documented form.
        let snap = sample();
        let bytes = encode(&snap);
        let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(count, 9);

        let mut offset = 4;
        // Timezone
        assert_eq!(bytes[offset], SystemFieldKind::Timezone as u8);
        offset += 1;
        let tz_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        offset += 2;
        assert_eq!(
            std::str::from_utf8(&bytes[offset..offset + tz_len]),
            Ok("Europe/Bratislava")
        );
        offset += tz_len;

        // Six enum entries — each: kind byte + tag byte.
        for kind in [
            SystemFieldKind::TimeFormat,
            SystemFieldKind::DateFormat,
            SystemFieldKind::NumberFormat,
            SystemFieldKind::FirstDayOfWeek,
            SystemFieldKind::TemperatureUnit,
            SystemFieldKind::UnitSystem,
        ] {
            assert_eq!(bytes[offset], kind as u8);
            offset += 2;
        }

        // NextAlarm: kind + present byte + i64 + name framing.
        assert_eq!(bytes[offset], SystemFieldKind::NextAlarm as u8);
        offset += 1;
        assert_eq!(bytes[offset], 1, "next_alarm present");
        offset += 1;
        let mut fire = [0_u8; 8];
        fire.copy_from_slice(&bytes[offset..offset + 8]);
        assert_eq!(i64::from_le_bytes(fire), 1_700_000_000_000);
        offset += 8;
        let name_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        offset += 2;
        assert_eq!(
            std::str::from_utf8(&bytes[offset..offset + name_len]),
            Ok("Wake up")
        );
        offset += name_len;

        // NightMode: kind + active byte.
        assert_eq!(bytes[offset], SystemFieldKind::NightMode as u8);
        offset += 1;
        assert_eq!(bytes[offset], 1, "night_mode active");
        offset += 1;
        assert_eq!(offset, bytes.len());
    }

    #[test]
    fn encode_serialises_next_alarm_none_compactly() {
        let snap = SystemSnapshot {
            settings: SystemSettings::default(),
            next_alarm: None,
            night_mode: false,
        };
        let bytes = encode(&snap);
        // Trailing layout: NextAlarm + 0 (None), then NightMode + 0 (inactive).
        assert_eq!(bytes[bytes.len() - 4], SystemFieldKind::NextAlarm as u8);
        assert_eq!(bytes[bytes.len() - 3], 0);
        assert_eq!(bytes[bytes.len() - 2], SystemFieldKind::NightMode as u8);
        assert_eq!(bytes[bytes.len() - 1], 0);
    }

    #[test]
    fn encode_emits_night_mode_active_when_set() {
        let snap = SystemSnapshot {
            settings: SystemSettings::default(),
            next_alarm: None,
            night_mode: true,
        };
        let bytes = encode(&snap);
        assert_eq!(bytes[bytes.len() - 2], SystemFieldKind::NightMode as u8);
        assert_eq!(bytes[bytes.len() - 1], 1);
    }

    /// Walk the encoded stream and return the single-byte payload
    /// of the entry tagged with `kind`.
    /// Only meaningful for the six `u8`-tagged enums — panics otherwise.
    fn enum_tag_byte(bytes: &[u8], kind: SystemFieldKind) -> u8 {
        let mut offset = 4; // skip count header
        while offset < bytes.len() {
            let entry_kind = SystemFieldKind::try_from(bytes[offset])
                .expect("BUG: encoder only emits known SystemFieldKind variants");
            offset += 1;
            if entry_kind == kind {
                return bytes[offset];
            }
            offset += match entry_kind {
                SystemFieldKind::Timezone => {
                    let len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
                    2 + len
                }
                SystemFieldKind::TimeFormat
                | SystemFieldKind::DateFormat
                | SystemFieldKind::NumberFormat
                | SystemFieldKind::FirstDayOfWeek
                | SystemFieldKind::TemperatureUnit
                | SystemFieldKind::UnitSystem
                | SystemFieldKind::NightMode => 1,
                SystemFieldKind::NextAlarm => {
                    if bytes[offset] == 0 {
                        1
                    } else {
                        let name_len_off = offset + 1 + 8;
                        let name_len =
                            u16::from_le_bytes([bytes[name_len_off], bytes[name_len_off + 1]])
                                as usize;
                        1 + 8 + 2 + name_len
                    }
                }
            };
        }
        panic!("kind {kind:?} not found in encoded bytes");
    }

    #[test]
    fn encode_uses_each_enum_variant_with_expected_tag() {
        // One snapshot per enum variant for every multi-variant enum;
        // the payload byte for that entry must match `variant as u8`.
        // Regression guard against silent renumbering on either side of the wire.
        for tf in [TimeFormat::Hour12, TimeFormat::Hour24] {
            let snap = SystemSnapshot {
                settings: SystemSettings {
                    time_format: tf,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert_eq!(
                enum_tag_byte(&encode(&snap), SystemFieldKind::TimeFormat),
                tf as u8
            );
        }
        for df in [
            DateFormat::DdMmYyyyDot,
            DateFormat::DdMmYyyySlash,
            DateFormat::DMYyyySlash,
            DateFormat::MDYyyySlash,
            DateFormat::DdMmYyyyDash,
            DateFormat::YyyyMDSlash,
            DateFormat::YyyyMmDdDot,
            DateFormat::YyyyMmDdDash,
        ] {
            let snap = SystemSnapshot {
                settings: SystemSettings {
                    date_format: df,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert_eq!(
                enum_tag_byte(&encode(&snap), SystemFieldKind::DateFormat),
                df as u8
            );
        }
        for nf in [
            NumberFormat::SpaceGroupCommaDecimal,
            NumberFormat::CommaGroupDotDecimal,
            NumberFormat::DotGroupCommaDecimal,
            NumberFormat::SpaceGroupDotDecimal,
        ] {
            let snap = SystemSnapshot {
                settings: SystemSettings {
                    number_format: nf,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert_eq!(
                enum_tag_byte(&encode(&snap), SystemFieldKind::NumberFormat),
                nf as u8
            );
        }
        for wd in [
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
            Weekday::Saturday,
            Weekday::Sunday,
        ] {
            let snap = SystemSnapshot {
                settings: SystemSettings {
                    first_day_of_week: wd,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert_eq!(
                enum_tag_byte(&encode(&snap), SystemFieldKind::FirstDayOfWeek),
                wd as u8
            );
        }
        for tu in [TemperatureUnit::Celsius, TemperatureUnit::Fahrenheit] {
            let snap = SystemSnapshot {
                settings: SystemSettings {
                    temperature_unit: tu,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert_eq!(
                enum_tag_byte(&encode(&snap), SystemFieldKind::TemperatureUnit),
                tu as u8
            );
        }
        for us in [UnitSystem::Metric, UnitSystem::Imperial] {
            let snap = SystemSnapshot {
                settings: SystemSettings {
                    unit_system: us,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert_eq!(
                enum_tag_byte(&encode(&snap), SystemFieldKind::UnitSystem),
                us as u8
            );
        }
    }

    #[test]
    fn wire_encode_trait_delegates_to_free_function() {
        let snap = sample();
        let direct = encode(&snap);
        let via_trait = <SystemSnapshot as WireEncode>::encode(&snap);
        assert_eq!(direct, via_trait);
    }

    /// Cross-validate the host encoder against the SDK decoder end-to-end.
    ///
    /// The two sides share `bmc_wasm_protocol::system::*` for kind tags
    /// and enum wire values, so constant drift is impossible at compile time.
    ///
    /// This test pins the rest of the wire contract: kind discriminators,
    /// per-entry payload framing (u16 strings, u8 enum tags, i64 LE
    /// for the alarm fire time), and end-to-end variant fidelity.
    ///
    /// Companion to `params::tests::host_encoder_round_trips_through_sdk_parser`.
    #[test]
    fn host_encoder_round_trips_through_sdk_decoder() {
        use bmc_wasm_sdk::system::Snapshot;

        let snap = sample();
        let bytes = encode(&snap);
        let decoded = Snapshot::from_bytes(bytes);

        assert_eq!(decoded.timezone(), Some("Europe/Bratislava"));
        assert_eq!(decoded.time_format(), Some(TimeFormat::Hour12));
        assert_eq!(decoded.date_format(), Some(DateFormat::YyyyMmDdDot));
        assert_eq!(
            decoded.number_format(),
            Some(NumberFormat::CommaGroupDotDecimal)
        );
        assert_eq!(decoded.first_day_of_week(), Some(Weekday::Sunday));
        assert_eq!(
            decoded.temperature_unit(),
            Some(TemperatureUnit::Fahrenheit)
        );
        assert_eq!(decoded.unit_system(), Some(UnitSystem::Imperial));
        let next = decoded
            .next_alarm()
            .expect("BUG: sample() encodes next_alarm = Some(...)");
        assert_eq!(next.fire_at_utc_ms, 1_700_000_000_000);
        assert_eq!(next.name, "Wake up");
        assert_eq!(decoded.night_mode(), Some(true));
    }

    /// Cross-validate the `next_alarm: None` path — host encoder writes
    /// the `present=0` discriminator, SDK decoder must reflect it as `None`.
    #[test]
    fn host_encoder_next_alarm_none_round_trips_through_sdk_decoder() {
        use bmc_wasm_sdk::system::Snapshot;

        let snap = SystemSnapshot {
            settings: SystemSettings::default(),
            next_alarm: None,
            night_mode: false,
        };
        let bytes = encode(&snap);
        let decoded = Snapshot::from_bytes(bytes);

        assert_eq!(decoded.next_alarm(), None);
    }

    /// A `next_alarm.name` whose byte length straddles `u16::MAX` after
    /// the last multi-byte codepoint must truncate on a char boundary
    /// so the SDK's `core::str::from_utf8` accepts the field; without that,
    /// the whole `next_alarm` falls back to `None`.
    #[test]
    fn host_encoder_truncates_over_long_alarm_name_on_char_boundary() {
        use bmc_wasm_sdk::system::Snapshot;

        // 65 534 ASCII bytes + one 2-byte 'é' = 65 536-byte string;
        // a naive byte-cut at u16::MAX (= 65 535) would slice the 'é' in half.
        let mut name = "a".repeat(65_534);
        name.push('é');
        assert_eq!(name.len(), 65_536);

        let snap = SystemSnapshot {
            settings: SystemSettings::default(),
            next_alarm: Some(NextAlarm {
                fire_at_utc_ms: 1,
                name,
            }),
            night_mode: false,
        };
        let decoded = Snapshot::from_bytes(encode(&snap));
        let next = decoded
            .next_alarm()
            .expect("BUG: char-boundary truncation must keep name decodable");
        // 'a' × 65 534 — the 'é' is dropped entirely (would have crossed cap).
        assert_eq!(next.name.len(), 65_534);
        assert!(next.name.chars().all(|c| c == 'a'));
    }

    /// Snake-case JSON shape stored in fixtures must round-trip
    /// through the serde derives on `SystemSnapshot`.
    #[test]
    fn json_round_trip() {
        let original = sample();
        let json = serde_json::to_value(&original).expect("BUG: serialize");
        let parsed: SystemSnapshot = serde_json::from_value(json).expect("BUG: deserialize");
        assert_eq!(original, parsed);
    }

    #[test]
    fn json_round_trip_with_next_alarm_none() {
        let original = SystemSnapshot {
            settings: sample().settings,
            next_alarm: None,
            night_mode: false,
        };
        let json = serde_json::to_value(&original).expect("BUG: serialize");
        let parsed: SystemSnapshot = serde_json::from_value(json).expect("BUG: deserialize");
        assert_eq!(original, parsed);
    }

    /// Missing top-level fields (or whole `settings` / `next_alarm`)
    /// must fall through to defaults — fixtures recorded before a
    /// field was added shouldn't fail load.
    #[test]
    fn json_tolerates_missing_fields() {
        let json = serde_json::json!({});
        let parsed: SystemSnapshot = serde_json::from_value(json)
            .expect("BUG: empty object must deserialise to SystemSnapshot::default()");
        assert_eq!(parsed, SystemSnapshot::default());
    }

    /// Unknown enum-tagged strings (e.g. typo, future variant on a
    /// downgraded host) reject at fixture-load time rather than
    /// silently coercing to the default.
    #[test]
    fn json_rejects_unknown_enum_variant() {
        let json = serde_json::json!({
            "settings": { "time_format": "bogus" }
        });
        let err = serde_json::from_value::<SystemSnapshot>(json)
            .expect_err("unknown variant must reject");
        assert!(err.to_string().to_lowercase().contains("variant"));
    }

    /// Pin the on-disk JSON keys for the snake-case enum variants
    /// — these strings are the fixture-file contract, so a rename in
    /// `bmc_wasm_protocol` would silently invalidate all existing
    /// fixtures without this guard.
    #[test]
    fn json_pins_enum_wire_strings() {
        let snap = sample();
        let v = serde_json::to_value(&snap).expect("BUG: serialize");
        let s = &v["settings"];
        assert_eq!(s["time_format"], "hour12");
        assert_eq!(s["date_format"], "yyyy_mm_dd_dot");
        assert_eq!(s["number_format"], "comma_group_dot_decimal");
        assert_eq!(s["first_day_of_week"], "sunday");
        assert_eq!(s["temperature_unit"], "fahrenheit");
        assert_eq!(s["unit_system"], "imperial");
    }
}
