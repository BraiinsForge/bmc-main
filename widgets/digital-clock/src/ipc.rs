// Copyright (C) 2025  Braiins Systems s.r.o.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use bmc_ipc::SettingUpdate;
use bmc_shared_time::time::TimeSystem;
use bmc_widget::{WidgetClient, connect_widget, run_message_loop};

use crate::{Config, Params, WidgetSize};

impl From<bmc_ipc::SizeType> for WidgetSize {
    fn from(size: bmc_ipc::SizeType) -> Self {
        match size {
            bmc_ipc::SizeType::Small => Self::Small,
            bmc_ipc::SizeType::Medium => Self::Medium,
            bmc_ipc::SizeType::Large => Self::Large,
            bmc_ipc::SizeType::Full => Self::Full,
        }
    }
}

/// Connects to IPC and builds the widget configuration.
pub async fn connect() -> Result<(WidgetClient, Config), Box<dyn std::error::Error + Send + Sync>> {
    let (mut client, size, params, settings) = connect_widget::<Params>().await?;

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

    client.send_ready().await?;

    Ok((client, config))
}

/// Runs the IPC message loop with the widget's state.
pub async fn run(
    client: WidgetClient,
    date_format: Arc<AtomicU8>,
    timezone: Arc<RwLock<String>>,
    is_24_format: Arc<AtomicBool>,
) {
    run_message_loop(
        client,
        move |update| {
            handle_settings_update(update, &date_format, &timezone, &is_24_format);
        },
        || {
            slint::quit_event_loop().ok();
        },
    )
    .await;
}

fn handle_settings_update(
    update: SettingUpdate,
    date_format: &Arc<AtomicU8>,
    timezone: &Arc<RwLock<String>>,
    is_24_format: &Arc<AtomicBool>,
) {
    match update {
        SettingUpdate::Timezone(tz_str) => {
            *timezone.write().expect("BUG: timezone lock poisoned") = tz_str;
        }
        SettingUpdate::NightMode(_) => {}
        SettingUpdate::Localization(ref loc) => {
            is_24_format.store(loc.time_format == TimeSystem::Hour24, Ordering::Relaxed);
            date_format.store(loc.date_format as u8, Ordering::Relaxed);
        }
    }
}
