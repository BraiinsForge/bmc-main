// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use bmc_display::blockheight_data::{
    BLOCK_HEIGHT_API_URL, BLOCK_HEIGHT_LIMIT_API_PARAM, BlockheightData,
};
use bmc_display::data::{SceneId, WidgetId};
use bmc_display::display_controller::DisplayController;
use bmc_display::halving_data::{HalvingCountdown, next_halving_block};
use bmc_shared_time::time::{DateFormat, Timezone};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset};
use reqwest::Client;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::time::Duration;
use tokio::sync::{RwLock, watch};
use tokio::time::interval;
use tracing::{debug, error, instrument, warn};

use crate::widget_tasks::API_TIMEOUT;

/// How often to re-fetch block height from API (5 minutes)
const BLOCK_HEIGHT_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

#[instrument(name = "halving_countdown", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    config_handle: Arc<RwLock<ConfigHandle>>,
    mut system_timezone_receiver: watch::Receiver<Timezone>,
    scene_id: SceneId,
    widget_id: WidgetId,
) {
    let client = match Client::builder().timeout(API_TIMEOUT).build() {
        Ok(client) => client,
        Err(err) => {
            error!(?err, "Failed to create reqwest client, stopping");
            return;
        }
    };

    let shared_height = Arc::new(AtomicU32::new(0));

    // Spawn background task that periodically fetches block height
    {
        let shared_height = Arc::clone(&shared_height);
        tokio::spawn(async move {
            let mut fetch_interval = interval(BLOCK_HEIGHT_REFRESH_INTERVAL);
            loop {
                fetch_interval.tick().await;
                if let Some(height) = fetch_block_height(&client).await {
                    shared_height.store(height, Relaxed);
                }
            }
        });
    }

    let mut current_height: u32 = 0;
    let mut countdown = HalvingCountdown::default();
    let mut target_block: u32 = 0;

    // Create interval for 1-second countdown ticks
    let mut tick_interval = interval(Duration::from_secs(1));

    loop {
        tick_interval.tick().await;

        let height = shared_height.load(Relaxed);

        // No data yet from the background fetcher
        if height == 0 {
            continue;
        }

        if height == current_height {
            countdown.tick();
        } else {
            debug!(
                old_height = current_height,
                new_height = height,
                "Block height updated"
            );
            current_height = height;
            countdown = HalvingCountdown::from_block_height(current_height);
            target_block = next_halving_block(current_height);
        }

        update_display(
            &display_controller,
            &config_handle,
            &mut system_timezone_receiver,
            &scene_id,
            &widget_id,
            &countdown,
            target_block,
        )
        .await;
    }
}

async fn fetch_block_height(client: &Client) -> Option<u32> {
    let request = client
        .get(BLOCK_HEIGHT_API_URL)
        .query(&[(BLOCK_HEIGHT_LIMIT_API_PARAM, "1"), ("currency", "usd")]);

    match request.send().await {
        Ok(response) => {
            let blocks: Vec<BlockheightData> = response
                .json()
                .await
                .inspect_err(
                    |err| error!(error = %err, "Failed to parse block height JSON response"),
                )
                .ok()?;

            blocks
                .first()
                .and_then(bmc_display::blockheight_data::BlockheightData::height)
        }
        Err(err) => {
            warn!(error = %err, "Failed to fetch block height from API");
            None
        }
    }
}

/// Format the predicted halving date and time
fn format_predicted_datetime(
    total_seconds: u64,
    timezone: &Timezone,
    is_24_format: bool,
    date_format: DateFormat,
) -> (String, String) {
    let now = chrono::Local::now().with_timezone(timezone.chrono());
    let secs = i64::try_from(total_seconds).unwrap_or(i64::MAX);
    let predicted = now + ChronoDuration::seconds(secs);
    let predicted: DateTime<FixedOffset> = predicted.fixed_offset();

    let date = predicted.format(date_format.format_string()).to_string();

    // Format time with timezone: "4:34 PM GMT+1" or "16:34 GMT+1"
    let time = if is_24_format {
        format!("{} {}", predicted.format("%H:%M"), timezone)
    } else {
        format!("{} {}", predicted.format("%-I:%M %p"), timezone)
    };

    (date, time)
}

async fn update_display(
    display_controller: &DisplayController,
    config_handle: &Arc<RwLock<ConfigHandle>>,
    system_timezone_receiver: &mut watch::Receiver<Timezone>,
    scene_id: &SceneId,
    widget_id: &WidgetId,
    countdown: &HalvingCountdown,
    target_block: u32,
) {
    let timezone = system_timezone_receiver.borrow_and_update().clone();
    let localization = config_handle.read().await.localization_config();
    let is_24_format = localization.time_system.is_24();
    let number_format = localization.number_format;
    let date_format = localization.date_format;

    let (predicted_date, predicted_time) = format_predicted_datetime(
        countdown.total_seconds,
        &timezone,
        is_24_format,
        date_format,
    );
    let target_block_formatted = number_format.format_number(target_block, 0);

    display_controller.update_halving_countdown(
        scene_id.clone(),
        widget_id.clone(),
        countdown.total_seconds,
        countdown.blocks_remaining,
        predicted_date,
        predicted_time,
        target_block_formatted,
    );
}
