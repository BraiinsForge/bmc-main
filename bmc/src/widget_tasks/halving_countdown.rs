// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_display::blockheight_data::{
    BLOCK_HEIGHT_API_URL, BLOCK_HEIGHT_LIMIT_API_PARAM, BlockheightData,
};
use bmc_display::data::{SceneId, WidgetId};
use bmc_display::display_controller::DisplayController;
use bmc_display::halving_data::HalvingCountdown;
use reqwest::Client;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, error, instrument, warn};

use crate::widget_tasks::API_TIMEOUT;

/// How often to re-fetch block height from API (5 minutes)
const BLOCK_HEIGHT_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

#[instrument(name = "halving_countdown", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(display_controller: DisplayController, scene_id: SceneId, widget_id: WidgetId) {
    let client = match Client::builder().timeout(API_TIMEOUT).build() {
        Ok(client) => client,
        Err(err) => {
            error!(?err, "Failed to create reqwest client, stopping");
            return;
        }
    };

    // Initial fetch of block height
    let mut current_height = fetch_block_height(&client).await.unwrap_or(0);
    let mut countdown = HalvingCountdown::from_block_height(current_height);

    // Update display immediately
    update_display(&display_controller, &scene_id, &widget_id, &countdown);

    // Create interval for 1-second countdown ticks
    let mut tick_interval = interval(Duration::from_secs(1));

    // Track when to refresh block height
    let mut ticks_since_refresh: u64 = 0;
    let ticks_per_refresh = BLOCK_HEIGHT_REFRESH_INTERVAL.as_secs();

    loop {
        tick_interval.tick().await;
        ticks_since_refresh += 1;

        // Re-fetch block height periodically
        if ticks_since_refresh >= ticks_per_refresh {
            ticks_since_refresh = 0;

            if let Some(height) = fetch_block_height(&client).await {
                if height != current_height {
                    debug!(
                        old_height = current_height,
                        new_height = height,
                        "Block height updated"
                    );
                    current_height = height;
                    countdown = HalvingCountdown::from_block_height(current_height);
                }
            }
        } else {
            // Just tick down the countdown
            countdown.tick();
        }

        update_display(&display_controller, &scene_id, &widget_id, &countdown);
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

fn update_display(
    display_controller: &DisplayController,
    scene_id: &SceneId,
    widget_id: &WidgetId,
    countdown: &HalvingCountdown,
) {
    display_controller.update_halving_countdown(
        scene_id.clone(),
        widget_id.clone(),
        countdown.total_seconds,
        countdown.blocks_remaining,
    );
}
