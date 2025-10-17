// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::widget_tasks::{
    API_TIMEOUT, BTC_HISTORY_API_URL, DATA_HISTORY_TIMEFRAME_PARAM, DATA_REFRESH_PERIOD,
};
use bmc_display::btc_history_data::BtcHistoryData;
use bmc_display::data::{SceneId, TickerTimeFrame, WidgetId};
use bmc_display::display_controller::DisplayController;
use reqwest::Client;
use tokio::time::interval;
use tracing::{debug, info, instrument, warn};

#[instrument(name = "ticker_btc", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    scene_id: SceneId,
    widget_id: WidgetId,
    timeframe: TickerTimeFrame,
) {
    info!(?timeframe, "Params");
    let mut interval = interval(DATA_REFRESH_PERIOD);

    loop {
        interval.tick().await;

        debug!("Getting bitcoin history data...");
        let client = Client::new();
        let btc_history_data = match client
            .get(BTC_HISTORY_API_URL)
            .query(&[(
                DATA_HISTORY_TIMEFRAME_PARAM,
                Into::<String>::into(timeframe.clone()),
            )])
            .timeout(API_TIMEOUT)
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

        display_controller.update_ticker_btc(scene_id.clone(), widget_id.clone(), btc_history_data);
    }
}
