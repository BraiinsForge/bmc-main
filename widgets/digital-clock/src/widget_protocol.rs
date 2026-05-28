// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bmc_shared_time::time::TimeSystem;
use bmc_widget::wayland::{ProtocolInitialState, WidgetEventHandler, WidgetProtocolClient};
use bmc_widget_protocol::SettingUpdate;
use serde::Deserialize;
use slint::Timer;

use crate::{Config, DigitalClock, FontStyle, WidgetSize};

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("wayland error: {0}")]
    Wayland(#[from] bmc_widget::WaylandError),

    #[error("param decode error: {0}")]
    Params(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestParams {
    font_style: FontStyleKind,
    show_seconds: bool,
    show_timezone: bool,
    #[serde(default)]
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

fn widget_size_from_dimensions(width: u32, height: u32) -> WidgetSize {
    match (width, height) {
        (1280, _) => WidgetSize::Full,
        (_, 480) => WidgetSize::Large,
        (_, 238) if width >= 638 => WidgetSize::Medium,
        _ => WidgetSize::Small,
    }
}

#[derive(Debug)]
pub struct InitialConfig {
    pub config: Config,
    pub system_timezone: String,
    pub timezone_override: Option<String>,
}

/// Connect to the compositor, wait for its initial configure batch, and
/// return the widget's fully-populated runtime config plus the live
/// [`WidgetProtocolClient`] that will continue feeding setting updates.
pub fn connect_and_read_config() -> Result<(WidgetProtocolClient, InitialConfig), IpcError> {
    let mut client = WidgetProtocolClient::connect()?;
    client.create_widget_surface();
    client.flush()?;

    let initial = client.wait_for_configure()?;
    let initial_config = build_initial_config(&initial)?;

    Ok((client, initial_config))
}

fn build_initial_config(initial: &ProtocolInitialState) -> Result<InitialConfig, IpcError> {
    let params: ManifestParams = serde_json::from_value(initial.params.clone().into())?;

    let defaults = Config::default();
    let mut system_timezone = defaults.timezone.clone();
    let mut config = Config {
        width: initial.width,
        height: initial.height,
        size: widget_size_from_dimensions(initial.width, initial.height),
        show_seconds: params.show_seconds,
        show_timezone: params.show_timezone,
        font_style: FontStyle::from(params.font_style),
        timezone: defaults.timezone,
        is_24_format: defaults.is_24_format,
        date_format: defaults.date_format,
    };

    for setting in &initial.settings {
        match setting {
            SettingUpdate::TimeFormat(t) => config.is_24_format = *t == TimeSystem::Hour24,
            SettingUpdate::DateFormat(d) => config.date_format = *d,
            SettingUpdate::Timezone(tz) => system_timezone.clone_from(tz),
            SettingUpdate::NightMode(_)
            | SettingUpdate::NumberFormat(_)
            | SettingUpdate::TemperatureUnit(_)
            | SettingUpdate::FirstDayOfWeek(_)
            | SettingUpdate::UnitSystem(_)
            | SettingUpdate::NextAlarm(_) => {}
        }
    }

    let timezone_override = params.timezone;
    config.timezone = timezone_override
        .clone()
        .unwrap_or_else(|| system_timezone.clone());

    Ok(InitialConfig {
        config,
        system_timezone,
        timezone_override,
    })
}

struct EventHandler {
    date_format: Arc<AtomicU8>,
    timezone: Arc<RwLock<String>>,
    is_24_format: Arc<AtomicBool>,
    shutdown_requested: Arc<AtomicBool>,
    /// Weak handle so runtime param updates can push fresh values into
    /// Slint without respawning the widget process.
    ui: slint::Weak<DigitalClock>,
    system_timezone: String,
    timezone_override: Option<String>,
}

impl EventHandler {
    fn write_effective_timezone(&self) {
        let effective = self
            .timezone_override
            .as_deref()
            .unwrap_or(self.system_timezone.as_str());
        effective.clone_into(&mut self.timezone.write().expect("BUG: timezone lock poisoned"));
    }
}

impl WidgetEventHandler for EventHandler {
    fn on_setting(&mut self, update: SettingUpdate) {
        match update {
            SettingUpdate::Timezone(tz_str) => {
                self.system_timezone = tz_str;
                self.write_effective_timezone();
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
            | SettingUpdate::FirstDayOfWeek(_)
            | SettingUpdate::UnitSystem(_)
            | SettingUpdate::NextAlarm(_) => {
                // Clock widget doesn't use these settings.
            }
        }
    }

    fn on_param_update(&mut self, params: serde_json::Map<String, serde_json::Value>) {
        let Some(ui) = self.ui.upgrade() else {
            tracing::debug!("on_param_update: UI handle gone, dropping update");
            return;
        };

        let manifest: ManifestParams = match serde_json::from_value(params.into()) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(
                    "on_param_update: failed to decode manifest-validated params — \
                     compositor and widget schemas have diverged: {e}"
                );
                return;
            }
        };

        ui.set_font_style(FontStyle::from(manifest.font_style));
        ui.set_show_seconds(manifest.show_seconds);
        ui.set_show_timezone(manifest.show_timezone);
        if manifest.timezone != self.timezone_override {
            self.timezone_override = manifest.timezone;
            self.write_effective_timezone();
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
    ui: slint::Weak<DigitalClock>,
    system_timezone: String,
    timezone_override: Option<String>,
) -> (Timer, Arc<AtomicBool>) {
    let shutdown_requested = Arc::new(AtomicBool::new(false));

    let mut handler = EventHandler {
        date_format,
        timezone,
        is_24_format,
        shutdown_requested: Arc::clone(&shutdown_requested),
        ui,
        system_timezone,
        timezone_override,
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
