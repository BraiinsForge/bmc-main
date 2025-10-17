// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use crate::widget_tasks::{
    API_TIMEOUT, BTC_HISTORY_API_URL, DATA_HISTORY_TIMEFRAME_PARAM, DATA_REFRESH_PERIOD,
};
use bmc_display::blockheight_data;
use bmc_display::blockheight_data::BlockheightData;
use bmc_display::btc_history_data::BtcHistoryData;
use bmc_display::data::{SceneId, TickerTimeFrame, WidgetId, WidgetSize};
use bmc_display::diff_hashrate_data::DiffHashrateData;
use bmc_display::display_controller::DisplayController;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, warn};

const CURRENCY_API_PARAM: &str = "currency";
const DIFF_HASHRATE_API_URL: &str =
    "https://public-api.braiins.com/v1/hashrate-and-difficulty-history";

#[expect(clippy::too_many_lines)]
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

    let mut interval = interval(DATA_REFRESH_PERIOD);
    let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
        error!("HTTP Client init failed");
        return;
    };

    loop {
        interval.tick().await;

        if download_btc_history {
            debug!("Getting bitcoin history data...");
            let btc_history_data = match client
                .get(BTC_HISTORY_API_URL)
                .query(&[(
                    DATA_HISTORY_TIMEFRAME_PARAM,
                    String::from(TickerTimeFrame::Day1),
                )])
                .send()
                .await
            {
                Ok(response) => response
                    .json::<BtcHistoryData>()
                    .await
                    .map_err(|e| warn!("Failed to parse btc history JSON: {e}"))
                    .unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to get btc history data from API: {e}");
                    BtcHistoryData::default()
                }
            };

            display_controller.update_blockchain_btc_graph(
                scene_id.clone(),
                widget_id.clone(),
                btc_history_data,
            );
        }
        if download_diff_and_hashrate_history {
            debug!("Getting difficulty and hashrate history data...");
            let hashrate_data = match client
                .get(DIFF_HASHRATE_API_URL)
                .query(&[(
                    DATA_HISTORY_TIMEFRAME_PARAM,
                    String::from(TickerTimeFrame::Day1),
                )])
                .send()
                .await
            {
                Ok(response) => response
                    .json::<DiffHashrateData>()
                    .await
                    .map_err(|e| {
                        warn!("Failed to parse difficulty and hashrate history JSON: {e}");
                    })
                    .unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to get difficulty and hashrate history data from API: {e}");
                    DiffHashrateData::default()
                }
            };
            let difficulty_data = match client
                .get(DIFF_HASHRATE_API_URL)
                .query(&[(
                    DATA_HISTORY_TIMEFRAME_PARAM,
                    String::from(TickerTimeFrame::Year1),
                )])
                .send()
                .await
            {
                Ok(response) => response
                    .json::<DiffHashrateData>()
                    .await
                    .map_err(|e| {
                        warn!("Failed to parse difficulty and hashrate history JSON: {e}");
                    })
                    .unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to get difficulty and hashrate history data from API: {e}");
                    DiffHashrateData::default()
                }
            };

            let number_format = config_handle
                .read()
                .await
                .localization_config()
                .number_format;

            display_controller.update_hashrate_info(
                scene_id.clone(),
                widget_id.clone(),
                hashrate_data,
                number_format,
            );

            display_controller.update_difficulty_graph(
                scene_id.clone(),
                widget_id.clone(),
                difficulty_data,
            );
        }
        if download_blocks_history {
            debug!("Getting blocks history data...");
            let blockheight_history = match client
                .get(blockheight_data::BLOCK_HEIGHT_API_URL)
                .query(&[
                    (blockheight_data::BLOCK_HEIGHT_LIMIT_API_PARAM, "200"),
                    (CURRENCY_API_PARAM, "usd"),
                ])
                .send()
                .await
            {
                Ok(response) => response
                    .json::<Vec<BlockheightData>>()
                    .await
                    .map_err(|e| warn!("Failed to parse blockheight history JSON: {e}"))
                    .unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to get blockheight history from API: {e}");
                    Vec::default()
                }
            };

            display_controller.update_blocks_last_24h(
                scene_id.clone(),
                widget_id.clone(),
                blockheight_history,
            );
        }
    }
}
