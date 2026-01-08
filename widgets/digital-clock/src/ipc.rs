// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bmc_shared_time::time::TimeSystem;
use bmc_widget::wayland::{WidgetEventHandler, WidgetProtocolClient};
use bmc_widget::{EnvError, env};
use bmc_widget_protocol::SettingUpdate;
use slint::Timer;

use crate::{Config, Params, WidgetSize};

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("environment variable error: {0}")]
    Env(#[from] EnvError),

    #[error("wayland error: {0}")]
    Wayland(#[from] bmc_widget::WaylandError),
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

pub fn read_config() -> Result<(String, Config), IpcError> {
    let instance_id = env::read_instance_id()?;
    let size = env::read_size()?;
    let params: Params = env::read_params()?;
    let settings = env::read_settings()?;

    let mut config = Config {
        width: size.width,
        height: size.height,
        size: size.name.into(),
        show_seconds: params.show_seconds,
        show_timezone: params.show_timezone,
        font_style: params.font_style.into(),
        ..Config::default()
    };

    // Prefer params timezone over settings timezone
    if let Some(tz) = params.timezone {
        config.timezone = tz;
    } else if let Some(ref tz) = settings.timezone {
        config.timezone.clone_from(tz);
    }

    if let Some(ref loc) = settings.localization {
        config.is_24_format = loc.time_format == TimeSystem::Hour24;
        config.date_format = loc.date_format;
    }

    Ok((instance_id, config))
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
            SettingUpdate::NightMode(_) => {
                // Clock widget doesn't use night mode
            }
            SettingUpdate::Localization(ref loc) => {
                self.is_24_format
                    .store(loc.time_format == TimeSystem::Hour24, Ordering::Relaxed);
                self.date_format
                    .store(loc.date_format as u8, Ordering::Relaxed);
            }
        }
    }

    fn on_shutdown(&mut self) {
        self.shutdown_requested.store(true, Ordering::Relaxed);
        slint::quit_event_loop().ok();
    }
}

/// Sets up a separate Wayland connection for `deck_widget_v1` protocol events.
/// Returns a timer that polls the connection - must be kept alive while widget runs.
pub fn setup_wayland_events(
    instance_id: &str,
    date_format: Arc<AtomicU8>,
    timezone: Arc<RwLock<String>>,
    is_24_format: Arc<AtomicBool>,
) -> Result<(Timer, Arc<AtomicBool>), IpcError> {
    let mut protocol_client = WidgetProtocolClient::connect()?;
    protocol_client.create_widget_surface(instance_id);
    protocol_client.flush()?;

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

    Ok((timer, shutdown_requested))
}
