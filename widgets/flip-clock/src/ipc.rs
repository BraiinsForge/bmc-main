// Copyright (C) 2025  Braiins Systems s.r.o.

//! IPC module for flip-clock widget.
//!
//! Reads initial configuration from DECK_* environment variables and provides
//! a protocol client for receiving runtime setting updates from the compositor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use bmc_widget::wayland::{WidgetEventHandler, WidgetProtocolClient};
use bmc_widget::{EnvError, env};
use bmc_widget_protocol::SettingUpdate;

use crate::AnimationMode;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("environment variable error: {0}")]
    Env(#[from] EnvError),

    #[error("wayland error: {0}")]
    Wayland(#[from] bmc_widget::WaylandError),
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

    // Prefer params timezone over settings timezone
    if let Some(tz) = params.timezone {
        config.timezone = tz;
    } else if let Some(ref tz) = settings.timezone {
        config.timezone.clone_from(tz);
    }

    Ok((instance_id, config))
}

/// Event handler that updates shared state from protocol events.
#[derive(Debug)]
pub struct EventHandler {
    pub timezone: Arc<RwLock<String>>,
    pub shutdown_requested: Arc<AtomicBool>,
}

impl WidgetEventHandler for EventHandler {
    fn on_setting(&mut self, update: SettingUpdate) {
        match update {
            SettingUpdate::Timezone(tz_str) => {
                *self.timezone.write().expect("BUG: timezone lock poisoned") = tz_str;
            }
            SettingUpdate::NightMode(_) | SettingUpdate::Localization(_) => {
                // Flip-clock doesn't use night mode or localization
            }
        }
    }

    fn on_shutdown(&mut self) {
        self.shutdown_requested.store(true, Ordering::Relaxed);
    }
}

/// Set up the protocol client and create a widget surface.
///
/// Returns the protocol client and event handler for integration into the
/// caller's event loop. The caller must poll the protocol client regularly.
pub fn setup_protocol(
    instance_id: &str,
    timezone: Arc<RwLock<String>>,
    shutdown_requested: Arc<AtomicBool>,
) -> Result<(WidgetProtocolClient, EventHandler), IpcError> {
    let mut protocol_client = WidgetProtocolClient::connect()?;
    protocol_client.create_widget_surface(instance_id);
    protocol_client.flush()?;

    let handler = EventHandler {
        timezone,
        shutdown_requested,
    };

    Ok((protocol_client, handler))
}
