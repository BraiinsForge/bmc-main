// Copyright (C) 2025  Braiins Systems s.r.o.
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

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{ActionPayload, SettingUpdate, Settings, SizeInfo};

/// Messages sent from the application to a widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppMessage {
    Init {
        size: SizeInfo,
        params: Value,
        settings: Settings,
    },
    SettingsUpdate {
        #[serde(flatten)]
        update: SettingUpdate,
    },
    Shutdown,
}

/// Messages sent from a widget to the application.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetMessage {
    Ready,
    Error {
        message: String,
        recoverable: bool,
    },
    #[serde(rename = "action")]
    Action(ActionPayload),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LedEffect, RgbColor, SizeType};

    #[test]
    fn init_message_serializes_correctly() {
        let msg = AppMessage::Init {
            size: SizeInfo {
                name: SizeType::Large,
                width: 638,
                height: 480,
            },
            params: serde_json::json!({
                "style": "modern",
                "showSeconds": false
            }),
            settings: Settings {
                timezone: Some("Europe/Prague".to_owned()),
                night_mode: Some(false),
                ..Default::default()
            },
        };
        let json = serde_json::to_value(&msg)
            .expect("BUG: serialization of valid message should not fail");
        assert_eq!(json["type"], "init");
        assert_eq!(json["size"]["name"], "large");
        assert_eq!(json["size"]["width"], 638);
        assert_eq!(json["params"]["style"], "modern");
        assert_eq!(json["settings"]["timezone"], "Europe/Prague");
        assert_eq!(json["settings"]["nightMode"], false);
    }

    #[test]
    fn settings_update_serializes_correctly() {
        let msg = AppMessage::SettingsUpdate {
            update: crate::types::SettingUpdate::NightMode(true),
        };
        let json = serde_json::to_value(&msg)
            .expect("BUG: serialization of valid message should not fail");
        assert_eq!(json["type"], "settings_update");
        assert_eq!(json["key"], "nightMode");
        assert_eq!(json["value"], true);
    }

    #[test]
    fn shutdown_serializes_correctly() {
        let msg = AppMessage::Shutdown;
        let json = serde_json::to_value(&msg)
            .expect("BUG: serialization of valid message should not fail");
        assert_eq!(json["type"], "shutdown");
    }

    #[test]
    fn ready_serializes_correctly() {
        let msg = WidgetMessage::Ready;
        let json = serde_json::to_value(&msg)
            .expect("BUG: serialization of valid message should not fail");
        assert_eq!(json["type"], "ready");
    }

    #[test]
    fn error_serializes_correctly() {
        let msg = WidgetMessage::Error {
            message: "Failed to fetch weather data".to_owned(),
            recoverable: true,
        };
        let json = serde_json::to_value(&msg)
            .expect("BUG: serialization of valid message should not fail");
        assert_eq!(json["type"], "error");
        assert_eq!(json["message"], "Failed to fetch weather data");
        assert_eq!(json["recoverable"], true);
    }

    #[test]
    fn action_serializes_correctly() {
        let msg = WidgetMessage::Action(ActionPayload::Led {
            effect: LedEffect::Breathe,
            color: RgbColor { r: 255, g: 0, b: 0 },
            duration: Some(5000),
        });
        let json = serde_json::to_value(&msg)
            .expect("BUG: serialization of valid message should not fail");
        assert_eq!(json["type"], "action");
        assert_eq!(json["name"], "led");
        assert_eq!(json["payload"]["effect"], "breathe");
    }
}
