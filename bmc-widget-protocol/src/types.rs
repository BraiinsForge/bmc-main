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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "key", content = "value", rename_all = "camelCase")]
pub enum SettingUpdate {
    Localization(Localization),
    Timezone(String),
    NightMode(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedEffect {
    Chase,
    KnightRider,
    Scan,
    Snake,
    Breathe,
    Solid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "name", content = "payload", rename_all = "snake_case")]
pub enum ActionPayload {
    PlaySound {
        sound: String,
    },
    StopSound {},
    Led {
        effect: LedEffect,
        color: RgbColor,
        duration: Option<u64>,
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
    fn action_led_serializes_correctly() {
        let action = ActionPayload::Led {
            effect: LedEffect::Breathe,
            color: RgbColor { r: 255, g: 0, b: 0 },
            duration: Some(5000),
        };
        let json = serde_json::to_value(&action).expect("BUG: serialization should not fail");
        assert_eq!(json["name"], "led");
        assert_eq!(json["payload"]["effect"], "breathe");
        assert_eq!(json["payload"]["color"]["r"], 255);
        assert_eq!(json["payload"]["color"]["g"], 0);
        assert_eq!(json["payload"]["color"]["b"], 0);
        assert_eq!(json["payload"]["duration"], 5000);
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
