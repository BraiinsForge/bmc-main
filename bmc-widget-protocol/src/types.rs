// Copyright (C) 2025  Braiins Systems s.r.o.
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

use bmc_shared_time::time::{DateFormat, TimeSystem, WeekDay};
use bmc_shared_utils::number_format::NumberFormat;
use bmc_shared_utils::temperature::TemperatureUnit;
use bmc_shared_utils::unit_system::UnitSystem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayShape {
    Rectangular,
    Round,
}

/// Widget viewport shape. Mirrors the Wayland `viewport_shape` enum.
/// Declared separately from `DisplayShape` so the two can diverge in
/// future versions of the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewportShape {
    Rectangular,
    Round,
}

/// Active display geometry and shape delivered to a widget in the initial
/// configure batch. `dpi` is the platform's real display density and is
/// advisory for layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub shape: DisplayShape,
    pub dpi: u32,
}

impl DisplayInfo {
    /// BMC100 Deck display: logical `1280x480`, rectangular, `dpi=217`.
    pub const BMC100: Self = Self {
        width: 1_280,
        height: 480,
        shape: DisplayShape::Rectangular,
        dpi: 217,
    };
}

/// Secret field values per credential slot, JSON-shaped
/// as the wire carries them: `{"<slot>": {"<field>": "<value>"}}`.
///
/// A newtype rather than a bare map, so redaction rides on the type.
/// `Debug` renders every slot against `<redacted>`,
/// and serde skips the field wherever it is stored,
/// so no log line or serialized config can carry a secret.
#[derive(Clone, Default, PartialEq)]
pub struct CredentialSecrets(serde_json::Map<String, serde_json::Value>);

/// A slot a widget declares, with the field names its credential type
/// defines — the two things a hand-written secrets map is judged against.
///
/// The caller resolves the field names, so this crate needs no view
/// of the credential catalog: it knows a slot's shape only by being told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredSlot {
    pub name: String,
    pub fields: Vec<String>,
}

/// Why a hand-written secrets map could not become [`CredentialSecrets`].
/// Each names the mistake rather than degrading to a value whose lookups
/// all miss — that would surface only as a refused fetch,
/// reading as an outage rather than a typo.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SecretsShapeError {
    #[error("names slot {slot:?}, which the widget does not declare (declared: {declared:?})")]
    UndeclaredSlot { slot: String, declared: Vec<String> },
    #[error("slot {slot:?} is not an object of fields")]
    NotAnObject { slot: String },
    #[error("slot {slot:?} carries no fields")]
    NoFields { slot: String },
    #[error(
        "slot {slot:?} names field {field:?}, which its type does not define (defines: {known:?})"
    )]
    UnknownField {
        slot: String,
        field: String,
        known: Vec<String>,
    },
}

impl CredentialSecrets {
    /// Build from a hand-written map,
    /// validating it against the slots the manifest declares.
    ///
    /// Accepts this type's own delivery shape, `{"<slot>": {"fields": {…}}}`,
    /// and the bare `{"<slot>": {"<field>": …}}` that is easier to write
    /// by hand, normalising the latter into the former. `allow_hosts`
    /// stays beside the fields in either shape, never inside them.
    pub fn from_editable(
        map: serde_json::Map<String, serde_json::Value>,
        declared: &[DeclaredSlot],
    ) -> Result<Self, SecretsShapeError> {
        let mut out = serde_json::Map::new();
        for (slot, mut entry) in map {
            let Some(spec) = declared.iter().find(|spec| spec.name == slot) else {
                return Err(SecretsShapeError::UndeclaredSlot {
                    slot,
                    declared: declared.iter().map(|spec| spec.name.clone()).collect(),
                });
            };
            // In the delivery shape the pin sits beside "fields", so it must
            // come off the entry before the descent into the field map.
            let outer_hosts = entry
                .as_object_mut()
                .and_then(|entry| entry.remove("allow_hosts"));
            let mut fields = entry.get("fields").cloned().unwrap_or(entry);
            let Some(fields) = fields.as_object_mut() else {
                return Err(SecretsShapeError::NotAnObject { slot });
            };
            let allow_hosts = outer_hosts.or_else(|| fields.remove("allow_hosts"));
            if fields.is_empty() {
                return Err(SecretsShapeError::NoFields { slot });
            }
            // A field the type does not define misses every lookup,
            // just as an undeclared slot would.
            if let Some(unknown) = fields.keys().find(|field| !spec.fields.contains(field)) {
                return Err(SecretsShapeError::UnknownField {
                    field: unknown.clone(),
                    known: spec.fields.clone(),
                    slot,
                });
            }
            let mut wire = serde_json::Map::new();
            wire.insert(
                "fields".to_owned(),
                serde_json::Value::Object(fields.clone()),
            );
            if let Some(hosts) = allow_hosts {
                wire.insert("allow_hosts".to_owned(), hosts);
            }
            out.insert(slot, serde_json::Value::Object(wire));
        }
        Ok(Self(out))
    }

    #[must_use]
    pub fn new(secrets: serde_json::Map<String, serde_json::Value>) -> Self {
        Self(secrets)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of slots carrying secrets. Safe to log, unlike the values.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.0.len()
    }

    /// Whether an account is bound to this slot, regardless of its fields.
    /// Distinguishes an unbound slot from a mistyped field name.
    #[must_use]
    pub fn has_slot(&self, slot: &str) -> bool {
        self.0.contains_key(slot)
    }

    /// One field's value, for the host to substitute at egress.
    /// The only way at a secret, and it hands back one field at a time.
    #[must_use]
    pub fn field(&self, slot: &str, field: &str) -> Option<&str> {
        self.0.get(slot)?.get("fields")?.get(field)?.as_str()
    }

    /// The account's own egress pin for this slot, empty when it has none.
    /// Not secret; it rides this channel because the channel already stops
    /// at the host, which is the one place the pin is enforced.
    #[must_use]
    pub fn allow_hosts(&self, slot: &str) -> Vec<&str> {
        self.0
            .get(slot)
            .and_then(|v| v.get("allow_hosts"))
            .and_then(|v| v.as_array())
            .map(|entries| entries.iter().filter_map(|e| e.as_str()).collect())
            .unwrap_or_default()
    }

    /// The JSON text emitted on the `credential_secrets` event.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        serde_json::Value::Object(self.0.clone()).to_string()
    }
}

impl std::fmt::Debug for CredentialSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|slot| (slot, "<redacted>")))
            .finish()
    }
}

/// Full initial configuration for a widget instance.
///
/// The coordinator pushes one of these into the compositor before spawning
/// the widget process. On `get_widget_surface` the compositor looks up this
/// record by peer-credential pid and emits it as a batch of typed events
/// (`configure`, `params`, setting events, `configure_done`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetInitialConfig {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_viewport_shape")]
    pub viewport_shape: ViewportShape,
    /// Active display geometry/shape emitted to the widget as `display_info`.
    /// Defaults to the BMC100 Deck display so records written before this
    /// field existed still deserialize.
    #[serde(default = "default_display_info")]
    pub display: DisplayInfo,
    /// Widget-specific params keyed by manifest entry name. Empty map
    /// means the widget has no configured params.
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
    /// Keyed by manifest slot name; an absent slot is unbound.
    #[serde(default)]
    pub credentials: serde_json::Map<String, serde_json::Value>,
    /// Stored so a respawned widget does not render a frame
    /// with its slots falsely unbound.
    ///
    /// Skipped by serde: a record that round-trips
    /// through JSON must not be able to carry a secret.
    #[serde(skip)]
    pub credential_secrets: CredentialSecrets,
    /// Opaque, stable per-instance token the compositor mints; keys
    /// per-instance resources (e.g. the asset cache), stable across
    /// dormancy and restart.
    pub token: String,
}

fn default_viewport_shape() -> ViewportShape {
    ViewportShape::Rectangular
}

fn default_display_info() -> DisplayInfo {
    DisplayInfo::BMC100
}

impl From<DisplayShape> for crate::server::deck_widget_surface_v1::DisplayShape {
    fn from(shape: DisplayShape) -> Self {
        match shape {
            DisplayShape::Rectangular => Self::Rectangular,
            DisplayShape::Round => Self::Round,
        }
    }
}

impl From<crate::client::deck_widget_surface_v1::DisplayShape> for DisplayShape {
    fn from(wire: crate::client::deck_widget_surface_v1::DisplayShape) -> Self {
        use crate::client::deck_widget_surface_v1::DisplayShape as P;
        match wire {
            P::Rectangular => Self::Rectangular,
            P::Round => Self::Round,
        }
    }
}

impl From<ViewportShape> for crate::server::deck_widget_surface_v1::ViewportShape {
    fn from(shape: ViewportShape) -> Self {
        match shape {
            ViewportShape::Rectangular => Self::Rectangular,
            ViewportShape::Round => Self::Round,
        }
    }
}

impl From<crate::client::deck_widget_surface_v1::ViewportShape> for ViewportShape {
    fn from(wire: crate::client::deck_widget_surface_v1::ViewportShape) -> Self {
        use crate::client::deck_widget_surface_v1::ViewportShape as P;
        match wire {
            P::Rectangular => Self::Rectangular,
            P::Round => Self::Round,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Localization {
    pub date_format: DateFormat,
    pub time_format: TimeSystem,
    pub number_format: NumberFormat,
    pub temperature_unit: TemperatureUnit,
    pub first_day_of_week: WeekDay,
    pub unit_system: UnitSystem,
}

/// Resolved soonest-to-fire alarm derived host-side
/// from the operator's alarm list.
///
/// Distinct from the alarms-list storage shape itself
/// — widgets only see the next-to-fire entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextAlarm {
    /// UTC milliseconds since the Unix epoch at which the alarm
    /// fires next. Timezone-invariant; widgets pair this with the
    /// `Timezone` setting for local-time rendering.
    pub fire_at_utc_ms: i64,
    /// Operator-typed display name.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub localization: Option<Localization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub night_mode: Option<bool>,
}

/// One atomic setting change broadcast from the compositor to widgets.
///
/// Each variant maps 1:1 to a typed event in the `deck_widget_v1`
/// protocol. Splitting the previously-bundled `Localization` variant into
/// per-field ones lets us add new locale fields later without breaking
/// existing widgets — old widgets simply ignore unknown events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "key", content = "value", rename_all = "camelCase")]
pub enum SettingUpdate {
    Timezone(String),
    NightMode(bool),
    DateFormat(DateFormat),
    TimeFormat(TimeSystem),
    NumberFormat(NumberFormat),
    TemperatureUnit(TemperatureUnit),
    FirstDayOfWeek(WeekDay),
    UnitSystem(UnitSystem),
    NextAlarm(Option<NextAlarm>),
}

impl SettingUpdate {
    /// Expand a full `Localization` struct into the per-field
    /// `SettingUpdate` values that should be broadcast together.
    #[must_use]
    pub fn from_localization(loc: &Localization) -> [SettingUpdate; 6] {
        [
            SettingUpdate::DateFormat(loc.date_format),
            SettingUpdate::TimeFormat(loc.time_format),
            SettingUpdate::NumberFormat(loc.number_format),
            SettingUpdate::TemperatureUnit(loc.temperature_unit),
            SettingUpdate::FirstDayOfWeek(loc.first_day_of_week),
            SettingUpdate::UnitSystem(loc.unit_system),
        ]
    }
}

/// Convert the wayland-generated client `Weekday` enum into the domain
/// `WeekDay`. Widgets receive the latter via `SettingUpdate::FirstDayOfWeek`;
/// this impl lives here so the per-widget protocol adapters don't have
/// to hand-roll the identity mapping.
impl From<crate::client::deck_widget_surface_v1::Weekday> for WeekDay {
    fn from(w: crate::client::deck_widget_surface_v1::Weekday) -> Self {
        use crate::client::deck_widget_surface_v1::Weekday as P;
        match w {
            P::Monday => Self::Monday,
            P::Tuesday => Self::Tuesday,
            P::Wednesday => Self::Wednesday,
            P::Thursday => Self::Thursday,
            P::Friday => Self::Friday,
            P::Saturday => Self::Saturday,
            P::Sunday => Self::Sunday,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedEffect {
    Chase,
    KnightRider,
    Scan,
    Snake,
    Breathe,
    Solid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedScope {
    Local,
    Global,
}

/// Widget-allocated identifier for a single LED request.
///
/// The widget owns its own u32 namespace per surface; uniqueness on the
/// host side is keyed on `(instance_id, request_id)`. The reserved
/// value `0` is invalid for `led_temporary`/`led_endless` and means
/// "stop everything I have outstanding" on `stop_led`.
pub type LedRequestId = u32;

/// Reserved value of [`LedRequestId`] — denotes "all of this widget's
/// requests" on `stop_led` and is invalid as an allocation.
pub const LED_REQUEST_ID_ALL: LedRequestId = 0;

/// Lifecycle status reported back to the widget for a previous
/// `LedTemporary`/`LedEndless` request.
///
/// `Expired` only fires for `LedTemporary`: its duration ran out, on
/// the widget's logical-time schedule, regardless of whether the strip
/// was showing it at the moment. `Superseded` fires when an endless is
/// displaced from its tier — by `stop_led`, by another endless landing
/// on the same tier, or by the widget disconnecting; the request will
/// not come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedRequestStatus {
    Accepted,
    Rejected,
    Superseded,
    Expired,
}

/// One typed action a widget can request from the compositor.
///
/// Each variant maps 1:1 to a typed request in the `deck_widget_v1`
/// protocol (no JSON envelope).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "name", content = "payload", rename_all = "snake_case")]
pub enum ActionPayload {
    PlaySound {
        sound: String,
    },
    StopSound {},
    LedTemporary {
        request_id: LedRequestId,
        effect: LedEffect,
        color: RgbColor,
        period_ms: u32,
        duration_ms: u32,
        scope: LedScope,
    },
    LedEndless {
        request_id: LedRequestId,
        effect: LedEffect,
        color: RgbColor,
        period_ms: u32,
        scope: LedScope,
    },
    StopLed {
        /// `0` (== [`LED_REQUEST_ID_ALL`]) cancels every outstanding
        /// LED request from the same widget; any other value cancels
        /// just that request.
        request_id: LedRequestId,
    },
}

#[cfg(test)]
mod editable_shape_tests {
    use super::*;

    /// Every fixture slot takes the braiins-pool type's field set.
    fn parse(json: &str, declared: &[&str]) -> Result<CredentialSecrets, SecretsShapeError> {
        let map = serde_json::from_str(json).expect("BUG: fixture is valid JSON");
        let declared: Vec<DeclaredSlot> = declared
            .iter()
            .map(|name| DeclaredSlot {
                name: (*name).to_owned(),
                fields: vec!["token".to_owned()],
            })
            .collect();
        CredentialSecrets::from_editable(map, &declared)
    }

    #[test]
    fn both_shapes_reach_the_same_field() {
        let bare = parse(r#"{"pool":{"token":"abc"}}"#, &["pool"]).expect("bare shape loads");
        let nested = parse(r#"{"pool":{"fields":{"token":"abc"}}}"#, &["pool"])
            .expect("delivery shape loads");
        assert_eq!(bare.field("pool", "token"), Some("abc"));
        assert_eq!(nested.field("pool", "token"), Some("abc"));
    }

    #[test]
    fn allow_hosts_stays_beside_the_fields_in_either_shape() {
        let secrets = parse(
            r#"{"pool":{"token":"abc","allow_hosts":["api.example.com"]}}"#,
            &["pool"],
        )
        .expect("bare shape with a pin loads");
        assert_eq!(secrets.allow_hosts("pool"), vec!["api.example.com"]);
        assert_eq!(secrets.field("pool", "allow_hosts"), None);
    }

    #[test]
    fn the_delivery_shape_keeps_its_sibling_pin() {
        let secrets = parse(
            r#"{"pool":{"fields":{"token":"abc"},"allow_hosts":["10.0.0.5:8080"]}}"#,
            &["pool"],
        )
        .expect("delivery shape with a pin loads");
        assert_eq!(secrets.allow_hosts("pool"), vec!["10.0.0.5:8080"]);
        assert_eq!(secrets.field("pool", "token"), Some("abc"));
    }

    #[test]
    fn an_undeclared_slot_names_what_the_widget_does_declare() {
        let err = parse(r#"{"poool":{"token":"abc"}}"#, &["pool"]).expect_err("typo is rejected");
        assert_eq!(
            err,
            SecretsShapeError::UndeclaredSlot {
                slot: "poool".to_owned(),
                declared: vec!["pool".to_owned()],
            }
        );
    }

    /// A field typo misses every lookup exactly as a slot typo does,
    /// reading as an outage rather than as the mistake it is.
    #[test]
    fn a_field_the_type_does_not_define_names_the_ones_it_does() {
        assert_eq!(
            parse(r#"{"pool":{"tokn":"abc"}}"#, &["pool"]).expect_err("typo is rejected"),
            SecretsShapeError::UnknownField {
                slot: "pool".to_owned(),
                field: "tokn".to_owned(),
                known: vec!["token".to_owned()],
            }
        );
    }

    #[test]
    fn a_slot_without_fields_is_rejected_rather_than_missing_every_lookup() {
        assert_eq!(
            parse(r#"{"pool":{}}"#, &["pool"]).expect_err("empty slot is rejected"),
            SecretsShapeError::NoFields {
                slot: "pool".to_owned()
            }
        );
        assert_eq!(
            parse(r#"{"pool":"abc"}"#, &["pool"]).expect_err("scalar slot is rejected"),
            SecretsShapeError::NotAnObject {
                slot: "pool".to_owned()
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets_of(slot: &str, field: &str, value: &str) -> CredentialSecrets {
        let mut slots = serde_json::Map::new();
        slots.insert(
            slot.to_owned(),
            serde_json::json!({ "fields": { field: value } }),
        );

        CredentialSecrets::new(slots)
    }

    fn pin_of(value: &serde_json::Value) -> CredentialSecrets {
        let mut slots = serde_json::Map::new();
        slots.insert(
            "pool".to_owned(),
            serde_json::json!({ "allow_hosts": value }),
        );

        CredentialSecrets::new(slots)
    }

    #[test]
    fn a_pin_reads_back_as_the_hosts_it_lists() {
        assert_eq!(
            pin_of(&serde_json::json!(["a.example", "b.example"])).allow_hosts("pool"),
            vec!["a.example", "b.example"]
        );
    }

    #[test]
    fn an_account_without_a_pin_lists_no_hosts() {
        assert!(
            secrets_of("pool", "token", "s3cr3t")
                .allow_hosts("pool")
                .is_empty()
        );
    }

    #[test]
    fn debugging_credential_secrets_names_the_slot_but_not_the_value() {
        let rendered = format!("{:?}", secrets_of("pool", "token", "s3cr3t"));

        assert!(rendered.contains("pool"), "the slot must stay debuggable");
        assert!(
            !rendered.contains("s3cr3t") && !rendered.contains("token"),
            "neither the value nor the field name may reach a log: {rendered}"
        );
    }

    #[test]
    fn debugging_a_config_holding_secrets_leaks_no_value() {
        let config = WidgetInitialConfig {
            width: 1,
            height: 1,
            viewport_shape: ViewportShape::Rectangular,
            display: DisplayInfo::BMC100,
            params: serde_json::Map::new(),
            credentials: serde_json::Map::new(),
            credential_secrets: secrets_of("pool", "token", "s3cr3t"),
            token: "t".to_owned(),
        };

        assert!(
            !format!("{config:?}").contains("s3cr3t"),
            "a derived Debug on the record must not be a way out for a secret"
        );
    }

    #[test]
    fn widget_initial_config_defaults_viewport_shape_to_rectangular() {
        let json = r#"{ "width": 317, "height": 238, "token": "test-token" }"#;
        let config: WidgetInitialConfig = serde_json::from_str(json).expect("BUG: parse");
        assert_eq!(config.viewport_shape, ViewportShape::Rectangular);
    }

    #[test]
    fn localization_uses_camel_case() {
        let loc = Localization {
            date_format: DateFormat::DdMmYyyyDot,
            time_format: TimeSystem::Hour24,
            number_format: NumberFormat::SpaceGroupCommaDecimal,
            temperature_unit: TemperatureUnit::Celsius,
            first_day_of_week: WeekDay::Monday,
            unit_system: UnitSystem::Metric,
        };
        let json = serde_json::to_value(loc).expect("BUG: serialization should not fail");
        assert!(json["dateFormat"].is_string());
        assert!(json["timeFormat"].is_string());
        assert!(json["numberFormat"].is_string());
        assert!(json["temperatureUnit"].is_string());
        assert!(json["firstDayOfWeek"].is_string());
        assert!(json["unitSystem"].is_string());
    }

    #[test]
    fn action_play_sound_serializes_correctly() {
        let action = ActionPayload::PlaySound {
            sound: "confirmation".to_owned(),
        };
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "play_sound");
        assert_eq!(json["payload"]["sound"], "confirmation");
    }

    #[test]
    fn action_led_temporary_serializes_correctly() {
        let action = ActionPayload::LedTemporary {
            request_id: 7,
            effect: LedEffect::Breathe,
            color: RgbColor { r: 255, g: 0, b: 0 },
            period_ms: 750,
            duration_ms: 5000,
            scope: LedScope::Local,
        };
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "led_temporary");
        assert_eq!(json["payload"]["request_id"], 7);
        assert_eq!(json["payload"]["effect"], "breathe");
        assert_eq!(json["payload"]["color"]["r"], 255);
        assert_eq!(json["payload"]["color"]["g"], 0);
        assert_eq!(json["payload"]["color"]["b"], 0);
        assert_eq!(json["payload"]["period_ms"], 750);
        assert_eq!(json["payload"]["duration_ms"], 5000);
        assert_eq!(json["payload"]["scope"], "local");
    }

    #[test]
    fn action_led_endless_serializes_correctly() {
        let action = ActionPayload::LedEndless {
            request_id: 9,
            effect: LedEffect::Solid,
            color: RgbColor { r: 0, g: 255, b: 0 },
            period_ms: 0,
            scope: LedScope::Global,
        };
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "led_endless");
        assert_eq!(json["payload"]["request_id"], 9);
        assert_eq!(json["payload"]["effect"], "solid");
        assert_eq!(json["payload"]["color"]["g"], 255);
        assert_eq!(json["payload"]["period_ms"], 0);
        assert_eq!(json["payload"]["scope"], "global");
    }

    #[test]
    fn action_stop_sound_serializes_correctly() {
        let action = ActionPayload::StopSound {};
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "stop_sound");
    }

    #[test]
    fn action_stop_led_serializes_correctly() {
        let action = ActionPayload::StopLed {
            request_id: LED_REQUEST_ID_ALL,
        };
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "stop_led");
        assert_eq!(json["payload"]["request_id"], 0);
    }

    #[test]
    fn led_request_status_serializes_snake_case() {
        let cases = [
            (LedRequestStatus::Accepted, "accepted"),
            (LedRequestStatus::Rejected, "rejected"),
            (LedRequestStatus::Superseded, "superseded"),
            (LedRequestStatus::Expired, "expired"),
        ];
        for (status, expected) in cases {
            let json = serde_json::to_value(status).expect("BUG: serialization should not fail");
            assert_eq!(json, serde_json::Value::String(expected.to_owned()));
        }
    }

    #[test]
    fn display_info_bmc100_default_matches_deck_geometry() {
        let display = DisplayInfo::BMC100;
        assert_eq!(display.width, 1_280);
        assert_eq!(display.height, 480);
        assert_eq!(display.shape, DisplayShape::Rectangular);
        assert_eq!(display.dpi, 217);
    }

    #[test]
    fn widget_initial_config_defaults_display_to_bmc100_when_absent() {
        let json = r#"{ "width": 100, "height": 100, "token": "test-token" }"#;
        let config: WidgetInitialConfig =
            serde_json::from_str(json).expect("BUG: config should deserialize");
        assert_eq!(config.display, DisplayInfo::BMC100);
    }

    #[test]
    fn display_shape_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&DisplayShape::Rectangular)
                .expect("BUG: serialization should not fail"),
            r#""rectangular""#
        );
        assert_eq!(
            serde_json::to_string(&DisplayShape::Round)
                .expect("BUG: serialization should not fail"),
            r#""round""#
        );
    }
}
