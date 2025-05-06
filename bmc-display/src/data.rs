// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{UIScreen, WidgetSize, WidgetType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Widget {
    pub row: i32,
    pub col: i32,
    pub widget_size: WidgetSize,
    pub widget_type: WidgetType,
    pub widget_data: Vec<String>,
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
