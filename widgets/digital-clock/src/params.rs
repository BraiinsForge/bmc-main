// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_time::time::DateFormat;

use crate::{FontStyle, WidgetSize};

/// Full configuration for initializing the widget.
#[derive(Debug)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    pub size: WidgetSize,
    pub show_seconds: bool,
    pub show_timezone: bool,
    pub font_style: FontStyle,
    pub timezone: String,
    pub is_24_format: bool,
    pub date_format: DateFormat,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 317,
            height: 238,
            size: WidgetSize::Small,
            show_seconds: true,
            show_timezone: true,
            font_style: FontStyle::Medium,
            timezone: "UTC".to_owned(),
            is_24_format: true,
            date_format: DateFormat::default(),
        }
    }
}
