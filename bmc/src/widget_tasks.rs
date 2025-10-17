// Copyright (C) 2025  Braiins Systems s.r.o.

mod braiins_pool;
mod clock;
mod remote_image;
mod ticker_btc;

use crate::config::ConfigHandle;
use bmc_display::blockheight_data::{self, BlockheightData};
use bmc_display::btc_history_data::BtcHistoryData;
use bmc_display::data::{SceneId, TickerTimeFrame, Widget, WidgetId, WidgetKind, WidgetSize};
use bmc_display::diff_hashrate_data::DiffHashrateData;
use bmc_display::display_controller::DisplayController;
use bmc_shared_time::time::Timezone;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use tokio::sync::{Mutex, RwLock, watch};
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{Instrument, debug, error, warn};

const BTC_HISTORY_API_URL: &str = "https://public-api.braiins.com/v1/price-history";
const DATA_HISTORY_TIMEFRAME_PARAM: &str = "timeframe";
const CURRENCY_API_PARAM: &str = "currency";
const DIFF_HASHRATE_API_URL: &str =
    "https://public-api.braiins.com/v1/hashrate-and-difficulty-history";
const API_TIMEOUT: Duration = Duration::from_secs(10);

const DATA_REFRESH_PERIOD: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct TaskHandle {
    scene_id: SceneId,
    widget_id: WidgetId,
    handle: JoinHandle<()>,
}

#[derive(Debug)]
pub(crate) struct WidgetTasks {
    task_handles: Arc<Mutex<Vec<TaskHandle>>>,
    display_controller: DisplayController,
    config_handle: Arc<RwLock<ConfigHandle>>,
    system_timezone_receiver: watch::Receiver<Timezone>,
}

impl WidgetTasks {
    pub(crate) fn new(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
        system_timezone_receiver: watch::Receiver<Timezone>,
    ) -> Self {
        Self {
            task_handles: Arc::default(),
            display_controller,
            config_handle,
            system_timezone_receiver,
        }
    }

    pub async fn spawn_all(
        &self,
        scene_id: &SceneId,
        widgets: impl ExactSizeIterator<Item = &Widget>,
    ) {
        if widgets.len() == 0 {
            return;
        }
        let mut task_handles = self.task_handles.lock().await;

        for widget in widgets {
            if let Some(task_handle) = self.spawn_internal(scene_id, widget) {
                task_handles.push(task_handle);
            }
        }
    }

    pub async fn spawn(&self, scene_id: &SceneId, widget: &Widget) {
        let mut task_handles = self.task_handles.lock().await;

        if let Some(task_handle) = self.spawn_internal(scene_id, widget) {
            task_handles.push(task_handle);
        }
    }

    fn spawn_internal(&self, scene_id: &SceneId, widget: &Widget) -> Option<TaskHandle> {
        let join_handle = match &widget.kind {
            WidgetKind::Clock(clock_widget) => Some(spawn(
                clock::run(
                    self.display_controller.clone(),
                    self.config_handle.clone(),
                    self.system_timezone_receiver.clone(),
                    scene_id.clone(),
                    widget.id.clone(),
                    clock_widget.timezone.clone(),
                )
                .in_current_span(),
            )),
            WidgetKind::TickerBtc(ticker_widget) => Some(spawn(
                ticker_btc::run(
                    self.display_controller.clone(),
                    scene_id.clone(),
                    widget.id.clone(),
                    ticker_widget.time_frame.clone(),
                )
                .in_current_span(),
            )),
            // BlockHeight widget does not have any widget specific data
            WidgetKind::BlockHeight(_) => None,
            WidgetKind::BraiinsPool(pool_widget) => Some(spawn(
                braiins_pool::run(
                    self.display_controller.clone(),
                    self.system_timezone_receiver.clone(),
                    scene_id.clone(),
                    widget.id.clone(),
                    widget.size,
                    self.config_handle.clone(),
                    pool_widget.pool_style,
                    pool_widget.chart_frame.clone(),
                    pool_widget.account_id.clone(),
                )
                .in_current_span(),
            )),
            WidgetKind::RemoteImage(remote_image_widget) => Some(spawn(
                remote_image::run(
                    self.display_controller.clone(),
                    scene_id.clone(),
                    widget.id.clone(),
                    widget.size,
                    remote_image_widget.url.clone(),
                    remote_image_widget.refresh_duration,
                )
                .in_current_span(),
            )),
            WidgetKind::BlockchainData => Some(spawn(self.make_blockchain_data_task(
                scene_id.clone(),
                widget.id.clone(),
                &widget.size,
                self.config_handle.clone(),
            ))),
        };

        join_handle
            .inspect(|_| debug!(%scene_id, widget_id = %widget.id, "Widget task spawned"))
            .map(|handle| TaskHandle {
                scene_id: scene_id.clone(),
                widget_id: widget.id.clone(),
                handle,
            })
    }

    pub async fn abort_all(&self, scene_id: &SceneId) {
        self.abort_internal(|task_handle| task_handle.scene_id == *scene_id)
            .await;
    }

    pub async fn abort(&self, scene_id: &SceneId, widget_id: &WidgetId) {
        self.abort_internal(|task_handle| {
            task_handle.scene_id == *scene_id && task_handle.widget_id == *widget_id
        })
        .await;
    }

    async fn abort_internal(&self, predicate: impl Fn(&TaskHandle) -> bool) {
        let mut task_handles = self.task_handles.lock().await;

        // NOTE: refactor code below to use `Vec::extract_if` after upgrade to Rust >= 1.87.0
        if !task_handles.iter().any(&predicate) {
            return;
        }

        let (to_abort, to_keep): (Vec<_>, Vec<_>) =
            task_handles.drain(..).partition(|task_handle| {
                let should_abort = predicate(task_handle);

                if should_abort {
                    task_handle.handle.abort();
                }

                should_abort
            });

        task_handles.extend(to_keep);

        for task_handle in to_abort {
            let _ = task_handle.handle.await;
            debug!(scene_id = %task_handle.scene_id, widget_id = %task_handle.widget_id, "Widget task aborted");
        }
    }

    #[expect(clippy::too_many_lines)]
    fn make_blockchain_data_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        widget_size: &WidgetSize,
        config_handle: Arc<RwLock<ConfigHandle>>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let display_controller = self.display_controller.clone();

        let download_btc_history = matches!(widget_size, WidgetSize::Full);
        let download_diff_and_hashrate_history =
            matches!(widget_size, WidgetSize::Full | WidgetSize::Large);
        let download_blocks_history = matches!(widget_size, WidgetSize::Full);

        async move {
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
                            warn!(
                                "Failed to get difficulty and hashrate history data from API: {e}"
                            );
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
                            warn!(
                                "Failed to get difficulty and hashrate history data from API: {e}"
                            );
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
    }
}

impl Clone for WidgetTasks {
    fn clone(&self) -> Self {
        Self {
            task_handles: self.task_handles.clone(),
            display_controller: self.display_controller.clone(),
            config_handle: self.config_handle.clone(),
            system_timezone_receiver: self.system_timezone_receiver.clone(),
        }
    }
}
