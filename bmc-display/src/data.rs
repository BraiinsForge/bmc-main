// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{UIScreen, WidgetSize, WidgetSlint, WidgetType as WidgetTypeSlint};
use serde::{Deserialize, Serialize};
use slint::{ModelRc, SharedString, VecModel};

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
    InitialSetupStart,
    InitialSetupWifiConnecting,
    InitialSetupWifiConnected,
    InitialSetupWifiError,
    InitialSetupGeneralError,
    InitialSetupConnectInfo,
    InitialSetupCompleted,
}

impl From<Screen> for UIScreen {
    fn from(value: Screen) -> Self {
        match value {
            Screen::Void => UIScreen::Void,
            Screen::DownloadFirmware => UIScreen::UpgradeDownload,
            Screen::Upgrade => UIScreen::UpgradeProgress,
            Screen::UpgradeFailed => UIScreen::UpgradeFailed,
            Screen::UpgradeSuccess => UIScreen::UpgradeSuccess,
            Screen::InitialSetupStart => todo!(),
            Screen::InitialSetupWifiConnecting => todo!(),
            Screen::InitialSetupWifiConnected => todo!(),
            Screen::InitialSetupWifiError => todo!(),
            Screen::InitialSetupGeneralError => todo!(),
            Screen::InitialSetupConnectInfo => todo!(),
            Screen::InitialSetupCompleted => todo!(),
        }
    }
}

impl From<Widget> for WidgetSlint {
    fn from(value: Widget) -> Self {
        let col = value.col;
        let row = value.row;
        let widget_type = value.widget_type;
        let (widget_size, widget_data, widget_type_slint) = match widget_type {
            WidgetType::ClockAnalogRoundSmall(config) => (
                WidgetSize::Small,
                config.timezone,
                WidgetTypeSlint::ClockAnalogRound,
            ),
            WidgetType::ClockAnalogRoundMedium(config) => (
                WidgetSize::Medium,
                config.timezone,
                WidgetTypeSlint::ClockAnalogRound,
            ),
            WidgetType::ClockAnalogRoundLarge(config) => (
                WidgetSize::Large,
                config.timezone,
                WidgetTypeSlint::ClockAnalogRound,
            ),
            WidgetType::ClockAnalogRoundFull(config) => (
                WidgetSize::FullScreen,
                config.timezone,
                WidgetTypeSlint::ClockAnalogRound,
            ),
            WidgetType::ClockAnalogRectSmall(config) => (
                WidgetSize::Small,
                config.timezone,
                WidgetTypeSlint::ClockAnalogRect,
            ),
            WidgetType::ClockAnalogRectMedium(config) => (
                WidgetSize::Medium,
                config.timezone,
                WidgetTypeSlint::ClockAnalogRect,
            ),
            WidgetType::ClockAnalogRectLarge(config) => (
                WidgetSize::Large,
                config.timezone,
                WidgetTypeSlint::ClockAnalogRect,
            ),
            WidgetType::ClockAnalogRectFull(config) => (
                WidgetSize::FullScreen,
                config.timezone,
                WidgetTypeSlint::ClockAnalogRect,
            ),
            WidgetType::ClockDigitalSmall(config) => (
                WidgetSize::Small,
                config.timezone,
                WidgetTypeSlint::ClockDigital,
            ),
            WidgetType::ClockDigitalMedium(config) => (
                WidgetSize::Medium,
                config.timezone,
                WidgetTypeSlint::ClockDigital,
            ),
            WidgetType::ClockDigitalLarge(config) => (
                WidgetSize::Large,
                config.timezone,
                WidgetTypeSlint::ClockDigital,
            ),
            WidgetType::ClockDigitalFull(config) => (
                WidgetSize::FullScreen,
                config.timezone,
                WidgetTypeSlint::ClockDigital,
            ),
        };
        let widget_data = ModelRc::new(VecModel::from(vec![SharedString::from(widget_data)]));
        WidgetSlint {
            col,
            row,
            widget_data,
            widget_size,
            widget_type: widget_type_slint,
        }
    }
}
