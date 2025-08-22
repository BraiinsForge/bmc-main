// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use bmc_display::btc_history_data::BtcHistoryData;
use bmc_display::clock_data::ClockData;
use bmc_display::data::{
    PoolChartFrame, PoolStyle, SceneId, TimeFrame, Widget, WidgetId, WidgetKind, WidgetSize,
};
use bmc_display::display_controller::DisplayController;
use bmc_display::pool_data::{
    self, CurrentUserHashrate, CurrentUserWorkerStats, LatestUserRewards, UserHashrateHistory,
    UserWorkerHistory,
};
use bmc_shared_time::time::Timezone;
use chrono::SubsecRound;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use tokio::sync::{Mutex, RwLock, watch};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep};
use tracing::{debug, warn};

const BTC_HISTORY_API_URL: &str = "https://public-api.braiins.com/v1/price-history";
const BTC_HISTORY_TIMEFRAME_API_PARAM: &str = "timeframe";
const API_TIMEOUT: Duration = Duration::from_secs(5);

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
            WidgetKind::Clock(clock_widget) => Some(spawn(self.make_clock_task(
                scene_id.clone(),
                widget.id.clone(),
                clock_widget.timezone.clone(),
            ))),
            WidgetKind::TickerBtc(ticker_widget) => Some(spawn(self.make_btc_graph_task(
                scene_id.clone(),
                widget.id.clone(),
                ticker_widget.time_frame.clone(),
            ))),
            // BlockHeight widget does not have any widget specific data
            WidgetKind::BlockHeight(_) => None,
            WidgetKind::BraiinsPool(pool_widget) => Some(spawn(self.make_braiins_pool_task(
                scene_id.clone(),
                widget.id.clone(),
                &widget.size,
                &pool_widget.pool_style,
                pool_widget.chart_frame.clone(),
            ))),
        };

        join_handle.map(|handle| TaskHandle {
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
        }
    }

    fn make_clock_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        timezone: Option<Timezone>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let display_controller = self.display_controller.clone();
        let config_handle = self.config_handle.clone();
        let mut system_timezone_receiver = self.system_timezone_receiver.clone();

        async move {
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
    }

    fn make_btc_graph_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        timeframe: TimeFrame,
    ) -> impl Future<Output = ()> + Send + 'static {
        let display_controller = self.display_controller.clone();

        async move {
            let mut interval = interval(Duration::from_secs(60));

            loop {
                interval.tick().await;

                debug!("Getting bitcoin history data...");
                let client = Client::new();
                let btc_history_data = if let Ok(response) = client
                    .get(BTC_HISTORY_API_URL)
                    .query(&[(
                        BTC_HISTORY_TIMEFRAME_API_PARAM,
                        Into::<String>::into(timeframe.clone()),
                    )])
                    .timeout(API_TIMEOUT)
                    .send()
                    .await
                {
                    response.json::<BtcHistoryData>().await.unwrap_or_default()
                } else {
                    warn!("Failed to get btc history data from API");
                    BtcHistoryData::default()
                };

                display_controller.update_btc_graph(
                    scene_id.clone(),
                    widget_id.clone(),
                    btc_history_data,
                );
            }
        }
    }

    #[expect(clippy::too_many_lines)]
    fn make_braiins_pool_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        widget_size: &WidgetSize,
        pool_style: &PoolStyle,
        chart_frame: PoolChartFrame,
    ) -> impl Future<Output = ()> + Send + 'static {
        let display_controller = self.display_controller.clone();

        let download_rewards = matches!(
            (pool_style, widget_size),
            (
                PoolStyle::Overview,
                WidgetSize::Full | WidgetSize::Large | WidgetSize::Medium
            )
        );
        let download_hashrate_history = matches!(
            (pool_style, widget_size),
            (PoolStyle::BigChart, _) | (PoolStyle::Overview, WidgetSize::Full | WidgetSize::Large)
        );
        let download_workers_stats = matches!(
            (pool_style, widget_size),
            (
                PoolStyle::BigChart,
                WidgetSize::Full | WidgetSize::Large | WidgetSize::Medium
            ) | (PoolStyle::Overview, WidgetSize::Full | WidgetSize::Medium)
        );
        let download_workers_history = matches!(
            (pool_style, widget_size),
            (
                PoolStyle::BigChart,
                WidgetSize::Full | WidgetSize::Large | WidgetSize::Medium
            ) | (PoolStyle::Overview, WidgetSize::Full)
        );

        async move {
            let mut interval = interval(Duration::from_secs(60));

            loop {
                interval.tick().await;

                debug!("Getting user current hashrate data...");
                let client = Client::new();
                let current_hashrate = if let Ok(response) = client
                    .get(format!(
                        "{}{}",
                        pool_data::POOL_API_URL,
                        pool_data::USER_HASHRATE_CURRENT
                    ))
                    .timeout(API_TIMEOUT)
                    .send()
                    .await
                {
                    response
                        .json::<CurrentUserHashrate>()
                        .await
                        .unwrap_or_default()
                } else {
                    warn!("Failed to get user current hashrate data from API");
                    CurrentUserHashrate::default()
                };
                display_controller.update_current_user_hashrate(
                    scene_id.clone(),
                    widget_id.clone(),
                    current_hashrate,
                );

                if download_rewards {
                    debug!("Getting user latest rewards data...");
                    let latest_rewards = if let Ok(response) = client
                        .get(format!(
                            "{}{}",
                            pool_data::POOL_API_URL,
                            pool_data::USER_REWARD_LATEST
                        ))
                        .timeout(API_TIMEOUT)
                        .send()
                        .await
                    {
                        response
                            .json::<LatestUserRewards>()
                            .await
                            .unwrap_or_default()
                    } else {
                        warn!("Failed to get user latest rewards data from API");
                        LatestUserRewards::default()
                    };
                    display_controller.update_rewards_latest(
                        scene_id.clone(),
                        widget_id.clone(),
                        latest_rewards,
                    );
                }

                if download_hashrate_history {
                    debug!("Getting user hashrate history data...");
                    let to_timestamp = chrono::Utc::now();
                    let from_timestamp = to_timestamp
                        .checked_sub_signed(chart_frame.clone().into())
                        // We don't expect this operation will fail
                        .unwrap_or_default();
                    let to_timestamp =
                        to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let from_timestamp =
                        from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let hashrate_history = if let Ok(response) = client
                        .get(format!(
                            "{}{}",
                            pool_data::POOL_API_URL,
                            pool_data::USER_HASHRATE_HISTORY
                        ))
                        .query(&[(pool_data::FROM_TIMESTAMP, &from_timestamp)])
                        .query(&[(pool_data::TO_TIMESTAMP, &to_timestamp)])
                        .timeout(API_TIMEOUT)
                        .send()
                        .await
                    {
                        response
                            .json::<UserHashrateHistory>()
                            .await
                            .unwrap_or_default()
                    } else {
                        warn!("Failed to get user hashrate history data from API");
                        UserHashrateHistory::default()
                    };
                    display_controller.update_hashrate_history(
                        scene_id.clone(),
                        widget_id.clone(),
                        hashrate_history,
                    );
                }

                if download_workers_stats {
                    debug!("Getting current workers data...");
                    let workers_stats = if let Ok(response) = client
                        .get(format!(
                            "{}{}",
                            pool_data::POOL_API_URL,
                            pool_data::USER_WORKERS_CURRENT
                        ))
                        .timeout(API_TIMEOUT)
                        .send()
                        .await
                    {
                        response
                            .json::<CurrentUserWorkerStats>()
                            .await
                            .unwrap_or_default()
                    } else {
                        warn!("Failed to get current workers data from API");
                        CurrentUserWorkerStats::default()
                    };
                    display_controller.update_current_workers(
                        scene_id.clone(),
                        widget_id.clone(),
                        workers_stats,
                    );
                }

                if download_workers_history {
                    debug!("Getting user worker history data...");
                    let to_timestamp = chrono::Utc::now();
                    let from_timestamp = to_timestamp
                        .checked_sub_signed(chart_frame.clone().into())
                        // We don't expect this operation will fail
                        .unwrap_or_default();
                    let to_timestamp =
                        to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let from_timestamp =
                        from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let worker_history = if let Ok(response) = client
                        .get(format!(
                            "{}{}",
                            pool_data::POOL_API_URL,
                            pool_data::USER_WORKERS_HISTORY
                        ))
                        .query(&[(pool_data::FROM_TIMESTAMP, &from_timestamp)])
                        .query(&[(pool_data::TO_TIMESTAMP, &to_timestamp)])
                        .timeout(API_TIMEOUT)
                        .send()
                        .await
                    {
                        response
                            .json::<UserWorkerHistory>()
                            .await
                            .unwrap_or_default()
                    } else {
                        warn!("Failed to get user worker history data from API");
                        UserWorkerHistory::default()
                    };
                    display_controller.update_worker_history(
                        scene_id.clone(),
                        widget_id.clone(),
                        worker_history,
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
