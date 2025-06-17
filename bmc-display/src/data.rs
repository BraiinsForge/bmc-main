// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::UIScreen;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NumberFontStyle {
    Light,
    Medium,
    Bold,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockAnalogRoundConfig {
    pub show_date: bool,
    pub show_timezone: bool,
    pub number_font_style: NumberFontStyle,
    pub timezone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockAnalogRectConfig {
    pub show_date: bool,
    pub show_timezone: bool,
    pub number_font_style: NumberFontStyle,
    pub timezone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockDigitalConfig {
    pub show_date: bool,
    pub show_seconds: bool,
    pub show_timezone: bool,
    pub number_font_style: NumberFontStyle,
    pub timezone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "widget_type", content = "widget_config")]
pub enum WidgetType {
    ClockAnalogRoundSmall(ClockAnalogRoundConfig),
    ClockAnalogRoundMedium(ClockAnalogRoundConfig),
    ClockAnalogRoundLarge(ClockAnalogRoundConfig),
    ClockAnalogRoundFull(ClockAnalogRoundConfig),
    ClockAnalogRectSmall(ClockAnalogRectConfig),
    ClockAnalogRectMedium(ClockAnalogRectConfig),
    ClockAnalogRectLarge(ClockAnalogRectConfig),
    ClockAnalogRectFull(ClockAnalogRectConfig),
    ClockDigitalSmall(ClockDigitalConfig),
    ClockDigitalMedium(ClockDigitalConfig),
    ClockDigitalLarge(ClockDigitalConfig),
    ClockDigitalFull(ClockDigitalConfig),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Widget {
    pub row: i32,
    pub col: i32,
    #[serde(flatten)]
    pub widget_type: WidgetType,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub id: u32,
    pub widgets: Vec<Widget>,
}

#[derive(Debug)]
pub enum Screen {
    Void,
    DownloadFirmware,
    Upgrade,
    UpgradeFailed,
    UpgradeSuccess,
}

impl From<Screen> for UIScreen {
    fn from(value: Screen) -> Self {
        match value {
            Screen::Void => UIScreen::Void,
            Screen::DownloadFirmware => UIScreen::UpgradeDownload,
            Screen::Upgrade => UIScreen::UpgradeProgress,
            Screen::UpgradeFailed => UIScreen::UpgradeFailed,
            Screen::UpgradeSuccess => UIScreen::UpgradeSuccess,
        }
    }
}
