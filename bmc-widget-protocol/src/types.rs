// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_time::time::{DateFormat, TimeSystem, WeekDay};
use bmc_shared_utils::number_format::NumberFormat;
use bmc_shared_utils::temperature::TemperatureUnit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SizeType {
    Small,
    Medium,
    Large,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeInfo {
    pub name: SizeType,
    pub width: u32,
    pub height: u32,
}

/// Full initial configuration for a widget instance.
///
/// The coordinator pushes one of these into the compositor before spawning
/// the widget process. On `get_widget_surface` the compositor looks up this
/// record by peer-credential pid and emits it as a batch of typed events
/// (`configure`, `params`, setting events, `configure_done`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetInitialConfig {
    pub size: SizeType,
    pub width: u32,
    pub height: u32,
    /// Widget-specific params keyed by manifest entry name. Empty map
    /// means the widget has no configured params.
    #[serde(default)]
    pub params: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Localization {
    pub date_format: DateFormat,
    pub time_format: TimeSystem,
    pub number_format: NumberFormat,
    pub temperature_unit: TemperatureUnit,
    pub first_day_of_week: WeekDay,
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
}

impl SettingUpdate {
    /// Expand a full `Localization` struct into the 5 per-field
    /// `SettingUpdate` values that should be broadcast together.
    #[must_use]
    pub fn from_localization(loc: &Localization) -> [SettingUpdate; 5] {
        [
            SettingUpdate::DateFormat(loc.date_format),
            SettingUpdate::TimeFormat(loc.time_format),
            SettingUpdate::NumberFormat(loc.number_format),
            SettingUpdate::TemperatureUnit(loc.temperature_unit),
            SettingUpdate::FirstDayOfWeek(loc.first_day_of_week),
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
        effect: LedEffect,
        color: RgbColor,
        duration_ms: u32,
    },
    LedEndless {
        effect: LedEffect,
        color: RgbColor,
    },
    StopLed {},
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_type_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&SizeType::Small).expect("BUG: serialization should not fail"),
            r#""small""#
        );
        assert_eq!(
            serde_json::to_string(&SizeType::Medium).expect("BUG: serialization should not fail"),
            r#""medium""#
        );
        assert_eq!(
            serde_json::to_string(&SizeType::Large).expect("BUG: serialization should not fail"),
            r#""large""#
        );
        assert_eq!(
            serde_json::to_string(&SizeType::Full).expect("BUG: serialization should not fail"),
            r#""full""#
        );
    }

    #[test]
    fn size_info_serializes_correctly() {
        let size = SizeInfo {
            name: SizeType::Large,
            width: 638,
            height: 480,
        };
        let json = serde_json::to_value(&size).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "large");
        assert_eq!(json["width"], 638);
        assert_eq!(json["height"], 480);
    }

    #[test]
    fn localization_uses_camel_case() {
        let loc = Localization {
            date_format: DateFormat::DdMmYyyyDot,
            time_format: TimeSystem::Hour24,
            number_format: NumberFormat::SpaceGroupCommaDecimal,
            temperature_unit: TemperatureUnit::Celsius,
            first_day_of_week: WeekDay::Monday,
        };
        let json = serde_json::to_value(&loc).expect("BUG: serialization should not fail");
        assert!(json["dateFormat"].is_string());
        assert!(json["timeFormat"].is_string());
        assert!(json["numberFormat"].is_string());
        assert!(json["temperatureUnit"].is_string());
        assert!(json["firstDayOfWeek"].is_string());
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
            effect: LedEffect::Breathe,
            color: RgbColor { r: 255, g: 0, b: 0 },
            duration_ms: 5000,
        };
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "led_temporary");
        assert_eq!(json["payload"]["effect"], "breathe");
        assert_eq!(json["payload"]["color"]["r"], 255);
        assert_eq!(json["payload"]["color"]["g"], 0);
        assert_eq!(json["payload"]["color"]["b"], 0);
        assert_eq!(json["payload"]["duration_ms"], 5000);
    }

    #[test]
    fn action_led_endless_serializes_correctly() {
        let action = ActionPayload::LedEndless {
            effect: LedEffect::Solid,
            color: RgbColor { r: 0, g: 255, b: 0 },
        };
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "led_endless");
        assert_eq!(json["payload"]["effect"], "solid");
        assert_eq!(json["payload"]["color"]["g"], 255);
    }

    #[test]
    fn action_stop_sound_serializes_correctly() {
        let action = ActionPayload::StopSound {};
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "stop_sound");
    }

    #[test]
    fn action_stop_led_serializes_correctly() {
        let action = ActionPayload::StopLed {};
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "stop_led");
    }
}
