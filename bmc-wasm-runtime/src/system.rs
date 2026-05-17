// Copyright (C) 2026  Braiins Systems s.r.o.

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
//! ```
//!
//! Per-field kind-tagging keeps wire layout extensible:
//! a future field gets a new `SystemFieldKind` variant without renumbering and without bumping a version.
//! The decoder dispatches on the byte; unknown kinds surface as a `DecodeError::UnknownFieldKind`.

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
    /// Wall-clock UTC milliseconds since the Unix epoch
    /// at which the alarm fires next.
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
    // The system schema does not currently include strings longer than
    // `u16::MAX`: timezone IANA names are short, and the alarm display name
    // is operator-typed with a UI-side cap well below 65 535 bytes.
    // Saturating keeps the encoder total — the decoder sees the actual byte
    // slice that was written.
    let len = u16::try_from(s.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&s.as_bytes()[..len as usize]);
}

fn estimate_size(snapshot: &SystemSnapshot) -> usize {
    // 4-byte count header
    // + per-field framing: 1 kind byte + payload
    //   timezone   : 1 + 2 + len
    //   six enums  : 1 + 1 = 2 each, six times
    //   next_alarm : 1 + 1 [+ 8 + 2 + name_len when Some]
    let mut total = 4 + 1 + 2 + snapshot.settings.timezone.len() + 6 * 2 + 2;
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
        }
    }

    #[test]
    fn encode_emits_correct_count_header_and_kind_bytes() {
        let bytes = encode(&sample());
        // count = 8 (timezone + 6 enums + next_alarm)
        assert_eq!(&bytes[0..4], &8_u32.to_le_bytes());

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
        assert_eq!(count, 8);

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
        assert_eq!(offset, bytes.len());
    }

    #[test]
    fn encode_serialises_next_alarm_none_compactly() {
        let snap = SystemSnapshot {
            settings: SystemSettings::default(),
            next_alarm: None,
        };
        let bytes = encode(&snap);
        // Last entry's kind byte is `NextAlarm`, followed by `0` (None) and
        // no payload.
        assert_eq!(bytes[bytes.len() - 2], SystemFieldKind::NextAlarm as u8);
        assert_eq!(bytes[bytes.len() - 1], 0);
    }

    /// Walk the encoded stream and return the single-byte payload
    /// of the entry tagged with `kind`.
    /// Only meaningful for the six `u8`-tagged enums — panics otherwise.
    fn enum_tag_byte(bytes: &[u8], kind: SystemFieldKind) -> u8 {
        let mut offset = 4; // skip count header
        while offset < bytes.len() {
            let entry_kind = SystemFieldKind::try_from_u8(bytes[offset])
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
                | SystemFieldKind::UnitSystem => 1,
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
        // One snapshot per enum variant for every multi-variant enum; the
        // payload byte for that entry must match `variant as u8`. Regression
        // guard against silent renumbering on either side of the wire.
        for tf in [TimeFormat::Hour12, TimeFormat::Hour24] {
            let snap = SystemSnapshot {
                settings: SystemSettings {
                    time_format: tf,
                    ..Default::default()
                },
                next_alarm: None,
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
        ] {
            let snap = SystemSnapshot {
                settings: SystemSettings {
                    date_format: df,
                    ..Default::default()
                },
                next_alarm: None,
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
                next_alarm: None,
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
                next_alarm: None,
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
                next_alarm: None,
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
                next_alarm: None,
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

        assert_eq!(decoded.timezone(), "Europe/Bratislava");
        assert_eq!(decoded.time_format(), TimeFormat::Hour12);
        assert_eq!(decoded.date_format(), DateFormat::YyyyMmDdDot);
        assert_eq!(decoded.number_format(), NumberFormat::CommaGroupDotDecimal);
        assert_eq!(decoded.first_day_of_week(), Weekday::Sunday);
        assert_eq!(decoded.temperature_unit(), TemperatureUnit::Fahrenheit);
        assert_eq!(decoded.unit_system(), UnitSystem::Imperial);
        let next = decoded
            .next_alarm()
            .expect("BUG: sample() encodes next_alarm = Some(...)");
        assert_eq!(next.fire_at_utc_ms, 1_700_000_000_000);
        assert_eq!(next.name, "Wake up");
    }

    /// Cross-validate the `next_alarm: None` path — host encoder writes
    /// the `present=0` discriminator, SDK decoder must reflect it as `None`.
    #[test]
    fn host_encoder_next_alarm_none_round_trips_through_sdk_decoder() {
        use bmc_wasm_sdk::system::Snapshot;

        let snap = SystemSnapshot {
            settings: SystemSettings::default(),
            next_alarm: None,
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
