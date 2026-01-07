// Copyright (C) 2025  Braiins Systems s.r.o.

//! Protocol type conversions.

use bmc_widget_protocol::server::deck_widget_surface_v1::{self, SettingType};
use bmc_widget_protocol::{ActionPayload, SettingUpdate};

pub fn setting_to_protocol(setting: &SettingUpdate) -> (SettingType, String) {
    match setting {
        SettingUpdate::Timezone(tz) => (SettingType::Timezone, tz.clone()),
        SettingUpdate::NightMode(enabled) => (SettingType::NightMode, enabled.to_string()),
        SettingUpdate::Localization(localization) => {
            let json = serde_json::to_string(localization).unwrap_or_default();
            (SettingType::Localization, json)
        }
    }
}

pub fn action_from_protocol(action_type: u32, payload: &str) -> Option<ActionPayload> {
    use deck_widget_surface_v1::ActionType;

    match action_type {
        x if x == ActionType::PlaySound as u32 => Some(ActionPayload::PlaySound {
            sound: payload.to_owned(),
        }),
        x if x == ActionType::StopSound as u32 => Some(ActionPayload::StopSound {}),
        x if x == ActionType::Led as u32 => serde_json::from_str(payload).ok(),
        x if x == ActionType::StopLed as u32 => Some(ActionPayload::StopLed {}),
        _ => None,
    }
}
