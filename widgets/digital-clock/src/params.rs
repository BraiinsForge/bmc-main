// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_time::time::DateFormat;
use serde::Deserialize;

use crate::{FontStyle, WidgetSize};

/// Widget parameters from the manifest/app configuration.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Params {
    pub show_seconds: bool,
    pub show_timezone: bool,
    pub font_style: ParamFontStyle,
    pub timezone: Option<String>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            show_seconds: true,
            show_timezone: true,
            font_style: ParamFontStyle::default(),
            timezone: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamFontStyle {
    Light,
    #[default]
    Medium,
    Bold,
}

impl From<ParamFontStyle> for FontStyle {
    fn from(style: ParamFontStyle) -> Self {
        match style {
            ParamFontStyle::Light => Self::Light,
            ParamFontStyle::Medium => Self::Medium,
            ParamFontStyle::Bold => Self::Bold,
        }
    }
}

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
