// Copyright (C) 2025  Braiins Systems s.r.o.

//! IPC module for flip-clock widget.
//!
//! Reads initial configuration from DECK_* environment variables.

use bmc_widget::{EnvError, env};

use crate::AnimationMode;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("environment variable error: {0}")]
    Env(#[from] EnvError),
}

/// Widget-specific parameters from `DECK_PARAMS`.
#[derive(Debug, Default, serde::Deserialize)]
pub struct Params {
    #[serde(default)]
    pub mode: ParamMode,
    pub timezone: Option<String>,
}

/// Animation mode parameter value.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamMode {
    Flat,
    #[default]
    Extruded,
}

impl From<ParamMode> for AnimationMode {
    fn from(p: ParamMode) -> Self {
        match p {
            ParamMode::Flat => Self::Flat,
            ParamMode::Extruded => Self::Extruded,
        }
    }
}

/// Configuration resolved from environment variables.
#[derive(Debug)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    pub mode: AnimationMode,
    pub timezone: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 640,
            height: 480,
            mode: AnimationMode::default(),
            timezone: String::from("UTC"),
        }
    }
}

/// Read initial configuration from DECK_* environment variables.
pub fn read_config() -> Result<(String, Config), IpcError> {
    let instance_id = env::read_instance_id()?;
    let size = env::read_size()?;
    let params: Params = env::read_params()?;
    let settings = env::read_settings()?;

    let mut config = Config {
        width: size.width,
        height: size.height,
        mode: params.mode.into(),
        ..Config::default()
    };

    if let Some(tz) = params.timezone {
        config.timezone = tz;
    } else if let Some(ref tz) = settings.timezone {
        config.timezone.clone_from(tz);
    }

    Ok((instance_id, config))
}
