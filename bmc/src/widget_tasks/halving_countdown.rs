// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use bmc_display::blockheight_data::BlockheightData;
use bmc_display::data::{SceneId, WidgetId};
use bmc_display::display_controller::DisplayController;
use bmc_display::halving_data::{AVG_BLOCK_TIME_SECS, next_halving_block};
use bmc_shared_time::time::{DateFormat, Timezone};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, Utc};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, watch};
use tokio::{select, time::interval};
use tracing::{debug, instrument};

#[instrument(name = "halving_countdown", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    config_handle: Arc<RwLock<ConfigHandle>>,
    mut system_timezone_receiver: watch::Receiver<Timezone>,
    mut blockheight_receiver: watch::Receiver<BlockheightData>,
    scene_id: SceneId,
    widget_id: WidgetId,
) {
    let mut current_data = BlockheightData::default();
    let mut predicted_halving: Option<DateTime<Utc>> = None;
    let mut blocks_remaining: u32 = 0;
    let mut target_block: u32 = 0;

    let mut localization_change_listener =
        config_handle.read().await.subscribe_localization_change();

    let mut tick_interval = interval(Duration::from_secs(60));

    loop {
        select! {
            _ = tick_interval.tick() => {}
            Ok(_) = localization_change_listener.recv() => {}
            Ok(()) = system_timezone_receiver.changed() => {}
            Ok(()) = blockheight_receiver.changed() => {
                let data = blockheight_receiver.borrow_and_update().clone();
                let Some(height) = data.height() else {
                    continue;
                };
                if current_data.height() == Some(height) {
                    continue;
                }
                debug!(
                    old_height = current_data.height(),
                    new_height = height,
                    "Block height updated"
                );

                target_block = next_halving_block(height);
                blocks_remaining = target_block.saturating_sub(height);
                current_data = data;

                // Compute predicted halving datetime from block timestamp
                predicted_halving = current_data.timestamp_as_datetime().map(|ts| {
                    let secs = i64::from(blocks_remaining) * i64::from(AVG_BLOCK_TIME_SECS);
                    ts + ChronoDuration::seconds(secs)
                });
            }
        }

        let Some(halving) = predicted_halving else {
            continue;
        };

        let total_seconds = (halving - Utc::now()).num_seconds().max(0);

        update_display(
            &display_controller,
            &config_handle,
            &mut system_timezone_receiver,
            &scene_id,
            &widget_id,
            total_seconds,
            blocks_remaining,
            target_block,
            halving,
        )
        .await;
    }
}

fn format_predicted_datetime(
    predicted_halving: DateTime<Utc>,
    timezone: &Timezone,
    is_24_format: bool,
    date_format: DateFormat,
) -> (String, String) {
    let predicted: DateTime<FixedOffset> = predicted_halving
        .with_timezone(timezone.chrono())
        .fixed_offset();

    let date = predicted.format(date_format.format_string()).to_string();
    let time = if is_24_format {
        format!("{} {}", predicted.format("%H:%M"), timezone)
    } else {
        format!("{} {}", predicted.format("%-I:%M %p"), timezone)
    };

    (date, time)
}

#[expect(clippy::too_many_arguments)]
async fn update_display(
    display_controller: &DisplayController,
    config_handle: &Arc<RwLock<ConfigHandle>>,
    system_timezone_receiver: &mut watch::Receiver<Timezone>,
    scene_id: &SceneId,
    widget_id: &WidgetId,
    total_seconds: i64,
    blocks_remaining: u32,
    target_block: u32,
    predicted_halving: DateTime<Utc>,
) {
    let timezone = system_timezone_receiver.borrow_and_update().clone();
    let localization = config_handle.read().await.localization_config();
    let is_24_format = localization.time_system.is_24();
    let date_format = localization.date_format;
    let target_block_formatted = localization.number_format.format_number(target_block, 0);

    let (predicted_date, predicted_time) =
        format_predicted_datetime(predicted_halving, &timezone, is_24_format, date_format);

    display_controller.update_halving_countdown(
        scene_id.clone(),
        widget_id.clone(),
        total_seconds,
        blocks_remaining,
        predicted_date,
        predicted_time,
        target_block_formatted,
    );
}
