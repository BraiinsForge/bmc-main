// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bmc_shared_time::time::TimeSystem;
use bmc_widget::wayland::{ProtocolInitialState, WidgetEventHandler, WidgetProtocolClient};
use bmc_widget_protocol::SettingUpdate;
use serde::Deserialize;
use slint::Timer;

use crate::{Config, FontStyle, WidgetSize};

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("wayland error: {0}")]
    Wayland(#[from] bmc_widget::WaylandError),

    #[error("param decode error: {0}")]
    Params(#[from] serde_json::Error),
}

/// Manifest-declared parameters for the digital-clock widget.
/// Every field is optional so an empty / partial params object from
/// the compositor falls back to [`Config::default`] values.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ManifestParams {
    font_style: Option<FontStyleKind>,
    show_seconds: Option<bool>,
    show_timezone: Option<bool>,
    timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FontStyleKind {
    Light,
    Medium,
    Bold,
}

impl From<FontStyleKind> for FontStyle {
    fn from(k: FontStyleKind) -> Self {
        match k {
            FontStyleKind::Light => Self::Light,
            FontStyleKind::Medium => Self::Medium,
            FontStyleKind::Bold => Self::Bold,
        }
    }
}

impl From<bmc_widget_protocol::SizeType> for WidgetSize {
    fn from(size: bmc_widget_protocol::SizeType) -> Self {
        match size {
            bmc_widget_protocol::SizeType::Small => Self::Small,
            bmc_widget_protocol::SizeType::Medium => Self::Medium,
            bmc_widget_protocol::SizeType::Large => Self::Large,
            bmc_widget_protocol::SizeType::Full => Self::Full,
        }
    }
}

/// Connect to the compositor, wait for its initial configure batch, and
/// return the widget's fully-populated runtime config plus the live
/// [`WidgetProtocolClient`] that will continue feeding setting updates.
pub fn connect_and_read_config() -> Result<(WidgetProtocolClient, Config), IpcError> {
    let mut client = WidgetProtocolClient::connect()?;
    client.create_widget_surface();
    client.flush()?;

    let initial = client.wait_for_configure()?;
    let config = build_config(&initial)?;

    Ok((client, config))
}

fn build_config(initial: &ProtocolInitialState) -> Result<Config, IpcError> {
    let params: ManifestParams = serde_json::from_value(initial.params.clone())?;

    let defaults = Config::default();
    let mut config = Config {
        width: initial.width,
        height: initial.height,
        size: initial.size.into(),
        show_seconds: params.show_seconds.unwrap_or(defaults.show_seconds),
        show_timezone: params.show_timezone.unwrap_or(defaults.show_timezone),
        font_style: params
            .font_style
            .map_or(defaults.font_style, FontStyle::from),
        timezone: defaults.timezone,
        is_24_format: defaults.is_24_format,
        date_format: defaults.date_format,
    };

    for setting in &initial.settings {
        match setting {
            SettingUpdate::TimeFormat(t) => config.is_24_format = *t == TimeSystem::Hour24,
            SettingUpdate::DateFormat(d) => config.date_format = *d,
            SettingUpdate::Timezone(tz) => config.timezone.clone_from(tz),
            SettingUpdate::NightMode(_)
            | SettingUpdate::NumberFormat(_)
            | SettingUpdate::TemperatureUnit(_)
            | SettingUpdate::FirstDayOfWeek(_) => {}
        }
    }

    // Per-widget timezone override wins over the system timezone from settings.
    if let Some(tz) = params.timezone {
        config.timezone = tz;
    }

    Ok(config)
}

struct EventHandler {
    date_format: Arc<AtomicU8>,
    timezone: Arc<RwLock<String>>,
    is_24_format: Arc<AtomicBool>,
    shutdown_requested: Arc<AtomicBool>,
}

impl WidgetEventHandler for EventHandler {
    fn on_setting(&mut self, update: SettingUpdate) {
        match update {
            SettingUpdate::Timezone(tz_str) => {
                *self.timezone.write().expect("BUG: timezone lock poisoned") = tz_str;
            }
            SettingUpdate::TimeFormat(t) => {
                self.is_24_format
                    .store(t == TimeSystem::Hour24, Ordering::Relaxed);
            }
            SettingUpdate::DateFormat(d) => {
                self.date_format.store(d as u8, Ordering::Relaxed);
            }
            SettingUpdate::NightMode(_)
            | SettingUpdate::NumberFormat(_)
            | SettingUpdate::TemperatureUnit(_)
            | SettingUpdate::FirstDayOfWeek(_) => {
                // Clock widget doesn't use these settings.
            }
        }
    }

    fn on_shutdown(&mut self) {
        self.shutdown_requested.store(true, Ordering::Relaxed);
        slint::quit_event_loop().ok();
    }
}

/// Install a Slint timer that pumps the already-connected protocol
/// client for runtime setting updates and shutdown events.
///
/// The widget's initial state has already been extracted by
/// [`connect_and_read_config`]; from here on the same connection just
/// streams changes.
pub fn spawn_runtime_handler(
    mut protocol_client: WidgetProtocolClient,
    date_format: Arc<AtomicU8>,
    timezone: Arc<RwLock<String>>,
    is_24_format: Arc<AtomicBool>,
) -> (Timer, Arc<AtomicBool>) {
    let shutdown_requested = Arc::new(AtomicBool::new(false));

    let mut handler = EventHandler {
        date_format,
        timezone,
        is_24_format,
        shutdown_requested: Arc::clone(&shutdown_requested),
    };

    let timer = Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            if protocol_client.poll_events().is_ok() {
                protocol_client.process_events(&mut handler);
            }
        },
    );

    (timer, shutdown_requested)
}
