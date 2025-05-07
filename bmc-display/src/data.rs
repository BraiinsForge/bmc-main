// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{ClockLarge, ClockMedium, ClockSmall, UIScreen};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum WidgetType {
    ClockSmall(ClockSmall),
    ClockMedium(ClockMedium),
    ClockLarge(ClockLarge),
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
