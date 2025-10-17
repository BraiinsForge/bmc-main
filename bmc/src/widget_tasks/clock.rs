// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use bmc_display::clock_data::ClockData;
use bmc_display::data::{SceneId, WidgetId};
use bmc_display::display_controller::DisplayController;
use bmc_shared_time::time::Timezone;
use chrono::SubsecRound;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, watch};
use tokio::time::sleep;
use tracing::{info, instrument};

#[instrument(name = "clock", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    config_handle: Arc<RwLock<ConfigHandle>>,
    mut system_timezone_receiver: watch::Receiver<Timezone>,
    scene_id: SceneId,
    widget_id: WidgetId,
    timezone: Option<Timezone>,
) {
    info!(?timezone, "Params");

    loop {
        let current_tick = chrono::Local::now();

        let timezone = timezone
            .clone()
            .unwrap_or_else(|| system_timezone_receiver.borrow_and_update().clone());

        let now = chrono::Local::now()
            .with_timezone(timezone.chrono())
            .fixed_offset();

        let is_24_format = config_handle
            .read()
            .await
            .localization_config()
            .time_system
            .is_24();

        let clock_data = ClockData::new(now);

        display_controller.update_clock_widget(
            scene_id.clone(),
            widget_id.clone(),
            now,
            timezone.to_string(),
            is_24_format,
            clock_data,
        );

        // NOTE: we want to schedule next tick at next wall clock second
        let next_tick = current_tick.trunc_subsecs(0) + Duration::from_secs(1);
        let duration_to_next_tick = (next_tick - current_tick)
            .to_std()
            .expect("BUG: negative duration");

        sleep(duration_to_next_tick).await;
    }
}
