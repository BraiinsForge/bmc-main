// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use crate::widget_tasks::{
    API_TIMEOUT, BTC_HISTORY_API_URL, DATA_HISTORY_TIMEFRAME_PARAM, DATA_REFRESH_PERIOD,
};
use bmc_display::blockheight_data::{
    BLOCK_HEIGHT_API_URL, BLOCK_HEIGHT_LIMIT_API_PARAM, BlockheightData,
};
use bmc_display::btc_history_data::BtcHistoryData;
use bmc_display::data::{SceneId, TickerTimeFrame, WidgetId, WidgetSize};
use bmc_display::diff_hashrate_data::DiffHashrateData;
use bmc_display::display_controller::DisplayController;
use reqwest::Client;
use std::sync::Arc;
use tap::TapFallible;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, instrument, warn};

const CURRENCY_API_PARAM: &str = "currency";
const DIFF_HASHRATE_API_URL: &str =
    "https://public-api.braiins.com/v1/hashrate-and-difficulty-history";

#[instrument(name = "blockchain_data", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    scene_id: SceneId,
    widget_id: WidgetId,
    widget_size: WidgetSize,
    config_handle: Arc<RwLock<ConfigHandle>>,
) {
    let download_btc_history = matches!(widget_size, WidgetSize::Full);
    let download_diff_and_hashrate_history =
        matches!(widget_size, WidgetSize::Full | WidgetSize::Large);
    let download_blocks_history = matches!(widget_size, WidgetSize::Full);

    let client = match Client::builder().timeout(API_TIMEOUT).build() {
        Ok(client) => client,
        Err(err) => {
            warn!(?err, "Failed to create reqwest client, stopping");
            return;
        }
    };

    let mut interval = interval(DATA_REFRESH_PERIOD);

    loop {
        interval.tick().await;

        if download_btc_history {
            debug!("Getting bitcoin history data...");
            let btc_history_data = download_btc_history_data(&client).await;

            display_controller.update_blockchain_btc_graph(
                scene_id.clone(),
                widget_id.clone(),
                btc_history_data,
            );
        }

        if download_diff_and_hashrate_history {
            debug!("Getting difficulty and hashrate history data...");
            let diff_and_hashrate_day =
                download_diff_and_hashrate_data(&client, TickerTimeFrame::Day1).await;

            let diff_and_hashrate_year =
                download_diff_and_hashrate_data(&client, TickerTimeFrame::Year1).await;

            let number_format = config_handle
                .read()
                .await
                .localization_config()
                .number_format;

            display_controller.update_hashrate_info(
                scene_id.clone(),
                widget_id.clone(),
                diff_and_hashrate_day,
                number_format,
            );

            display_controller.update_difficulty_graph(
                scene_id.clone(),
                widget_id.clone(),
                diff_and_hashrate_year,
            );
        }

        if download_blocks_history {
            debug!("Getting blocks history data...");
            let blockheight_history = download_blockheight_history(&client).await;

            display_controller.update_blocks_last_24h(
                scene_id.clone(),
                widget_id.clone(),
                blockheight_history,
            );
        }
    }
}

async fn download_btc_history_data(client: &Client) -> BtcHistoryData {
    let request = client.get(BTC_HISTORY_API_URL).query(&[(
        DATA_HISTORY_TIMEFRAME_PARAM,
        TickerTimeFrame::Day1.to_string(),
    )]);

    match request.send().await {
        Ok(response) => response
            .json::<BtcHistoryData>()
            .await
            .tap_err(|e| warn!("Failed to parse btc history JSON: {e}"))
            .unwrap_or_default(),
        Err(e) => {
            warn!("Failed to get btc history data from API: {e}");
            BtcHistoryData::default()
        }
    }
}

async fn download_diff_and_hashrate_data(
    client: &Client,
    timeframe: TickerTimeFrame,
) -> DiffHashrateData {
    let request = client
        .get(DIFF_HASHRATE_API_URL)
        .query(&[(DATA_HISTORY_TIMEFRAME_PARAM, timeframe.to_string())]);

    match request.send().await {
        Ok(response) => response
            .json::<DiffHashrateData>()
            .await
            .tap_err(|e| {
                warn!("Failed to parse difficulty and hashrate history JSON: {e}");
            })
            .unwrap_or_default(),
        Err(e) => {
            warn!("Failed to get difficulty and hashrate history data from API: {e}");
            DiffHashrateData::default()
        }
    }
}

async fn download_blockheight_history(client: &Client) -> Vec<BlockheightData> {
    let request = client.get(BLOCK_HEIGHT_API_URL).query(&[
        (BLOCK_HEIGHT_LIMIT_API_PARAM, "200"),
        (CURRENCY_API_PARAM, "usd"),
    ]);

    match request.send().await {
        Ok(response) => response
            .json::<Vec<BlockheightData>>()
            .await
            .tap_err(|e| warn!("Failed to parse blockheight history JSON: {e}"))
            .unwrap_or_default(),
        Err(e) => {
            warn!("Failed to get blockheight history from API: {e}");
            Vec::default()
        }
    }
}
