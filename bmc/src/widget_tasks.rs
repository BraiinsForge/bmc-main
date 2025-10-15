// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use anyhow::{Context, bail};
use backon::{BackoffBuilder, ExponentialBuilder};
use bmc_display::blockheight_data::{self, BlockheightData};
use bmc_display::btc_history_data::BtcHistoryData;
use bmc_display::clock_data::ClockData;
use bmc_display::data::{
    AccountId, AuthenticationType, PoolChartTimeFrame, PoolStyle, SceneId, TickerTimeFrame, Widget,
    WidgetId, WidgetKind, WidgetSize,
};
use bmc_display::diff_hashrate_data::DiffHashrateData;
use bmc_display::display_controller::DisplayController;
use bmc_display::pool_data::{
    self, CurrentUserHashrate, CurrentUserWorkerStats, LatestUserRewards, RecentUserPayouts,
    UserFinancials, UserHashrateHistory, UserWorkerHistory,
};
use bmc_display::remote_image_data::RemoteImageState;
use bmc_display::{SharedImageBuffer, SharedPixelBuffer};
use bmc_shared_time::time::Timezone;
use chrono::SubsecRound;
use image::ImageDecoder;
use reqwest::Client;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use tokio::sync::{Mutex, RwLock, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval, sleep};
use tracing::{Instrument, debug, error, info, instrument, warn};
use url::Url;

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
                widget.size,
                self.config_handle.clone(),
                pool_widget.pool_style,
                pool_widget.chart_frame.clone(),
                pool_widget.account_id.clone(),
            ))),
            WidgetKind::RemoteImage(remote_image_widget) => {
                Some(spawn(self.make_remote_image_task(
                    scene_id.clone(),
                    widget.id.clone(),
                    widget.size,
                    remote_image_widget.url.clone(),
                    remote_image_widget.refresh_duration,
                )))
            }
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

    #[instrument(name = "clock", skip_all, fields(%scene_id, %widget_id))]
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
        .in_current_span()
    }

    #[instrument(name = "btc_graph", skip_all, fields(%scene_id, %widget_id))]
    fn make_btc_graph_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        timeframe: TickerTimeFrame,
    ) -> impl Future<Output = ()> + Send + 'static {
        let display_controller = self.display_controller.clone();

        async move {
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

                display_controller.update_btc_graph(
                    scene_id.clone(),
                    widget_id.clone(),
                    btc_history_data,
                );
            }
        }
        .in_current_span()
    }

    #[expect(clippy::too_many_arguments)]
    #[instrument(name = "braiins_pool", skip_all, fields(%scene_id, %widget_id))]
    fn make_braiins_pool_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        widget_size: WidgetSize,
        config_handle: Arc<RwLock<ConfigHandle>>,
        pool_style: PoolStyle,
        chart_frame: PoolChartTimeFrame,
        account_id: Option<AccountId>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let display_controller = self.display_controller.clone();
        let mut system_timezone_receiver = self.system_timezone_receiver.clone();

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
        let download_payout_stats = matches!(
            (pool_style, widget_size),
            (PoolStyle::Overview, WidgetSize::Full | WidgetSize::Large)
        );
        let download_recent_payouts = matches!(
            (pool_style, widget_size),
            (PoolStyle::BigChart, WidgetSize::Full)
        );

        async move {
            info!(?pool_style, ?chart_frame, ?account_id, "Params");
            let mut interval = interval(DATA_REFRESH_PERIOD);
            let Ok(client) = reqwest::ClientBuilder::new().timeout(API_TIMEOUT).build() else {
                error!("HTTP Client init failed");
                return;
            };
            let Some(account_id) = account_id else {
                warn!("Widget {widget_id} is missing Account ID");
                return;
            };

            loop {
                interval.tick().await;

                let (api_key, account_name) =
                    if let Some(account) = config_handle.read().await.accounts.get(&account_id) {
                        let auth = match &account.authentication {
                            AuthenticationType::ApiKey(api_key) => api_key.clone(),
                        };
                        (auth, account.name.clone())
                    } else {
                        warn!("Missing account with id: {account_id}");
                        return;
                    };

                let number_format = config_handle
                    .read()
                    .await
                    .localization_config()
                    .number_format;

                display_controller.update_account_name(
                    scene_id.clone(),
                    widget_id.clone(),
                    account_name,
                );

                debug!("Getting user current hashrate data...");
                let current_hashrate = match client
                    .get(format!(
                        "{}{}",
                        pool_data::POOL_API_URL,
                        pool_data::USER_HASHRATE_CURRENT
                    ))
                    .header("X-API-Key", &api_key)
                    .send()
                    .await
                {
                    Ok(response) => response
                        .json::<CurrentUserHashrate>()
                        .await
                        .map_err(|e| warn!("Failed to parse user current hashrate JSON: {e}"))
                        .unwrap_or_default(),
                    Err(e) => {
                        warn!("Failed to get user current hashrate data from API: {e}");
                        CurrentUserHashrate::default()
                    }
                };
                display_controller.update_current_user_hashrate(
                    scene_id.clone(),
                    widget_id.clone(),
                    current_hashrate,
                    number_format.clone(),
                );

                if download_rewards {
                    debug!("Getting user latest rewards data...");
                    let latest_rewards = match client
                        .get(format!(
                            "{}{}",
                            pool_data::POOL_API_URL,
                            pool_data::USER_REWARD_LATEST
                        ))
                        .header("X-API-Key", &api_key)
                        .send()
                        .await
                    {
                        Ok(response) => response
                            .json::<LatestUserRewards>()
                            .await
                            .map_err(|e| warn!("Failed to parse user latest rewards JSON: {e}"))
                            .unwrap_or_default(),
                        Err(e) => {
                            warn!("Failed to get user latest rewards data from API: {e}");
                            LatestUserRewards::default()
                        }
                    };
                    display_controller.update_rewards_latest(
                        scene_id.clone(),
                        widget_id.clone(),
                        latest_rewards,
                        number_format.clone(),
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
                    let mut hashrate_history = UserHashrateHistory::default();
                    let mut next_cursor: Option<String> = None;
                    let url = format!(
                        "{}{}",
                        pool_data::POOL_API_URL,
                        pool_data::USER_HASHRATE_HISTORY
                    );
                    loop {
                        let mut query_params = vec![
                            (pool_data::FROM_TIMESTAMP, from_timestamp.as_str()),
                            (pool_data::TO_TIMESTAMP, to_timestamp.as_str()),
                            (pool_data::PAGE_LIMIT, pool_data::PAGE_LIMIT_MAX),
                        ];

                        if let Some(cursor) = &next_cursor {
                            query_params.push((pool_data::CURSOR, cursor));
                        }

                        let hashrate_history_partial = match client
                            .get(&url)
                            .header("X-API-Key", &api_key)
                            .query(&query_params)
                            .send()
                            .await
                        {
                            Ok(response) => match response.json::<UserHashrateHistory>().await {
                                Ok(data) => data,
                                Err(e) => {
                                    warn!("Failed to parse user hashrate history JSON: {e}");
                                    break;
                                }
                            },
                            Err(e) => {
                                warn!("Failed to get user hashrate history data from API: {e}");
                                break;
                            }
                        };
                        hashrate_history.merge_and_sort(&hashrate_history_partial);
                        next_cursor = hashrate_history_partial.next_cursor();
                        if next_cursor.is_none() {
                            break;
                        }
                    }
                    let system_timezone = system_timezone_receiver.borrow_and_update().clone();
                    let is_24_format = config_handle
                        .read()
                        .await
                        .localization_config()
                        .time_system
                        .is_24();
                    let date_format = config_handle.read().await.localization_config().date_format;
                    display_controller.update_hashrate_history(
                        scene_id.clone(),
                        widget_id.clone(),
                        system_timezone,
                        is_24_format,
                        date_format,
                        hashrate_history,
                        number_format.clone(),
                    );
                }

                if download_workers_stats {
                    debug!("Getting current workers data...");
                    let workers_stats = match client
                        .get(format!(
                            "{}{}",
                            pool_data::POOL_API_URL,
                            pool_data::USER_WORKERS_CURRENT
                        ))
                        .header("X-API-Key", &api_key)
                        .send()
                        .await
                    {
                        Ok(response) => response
                            .json::<CurrentUserWorkerStats>()
                            .await
                            .map_err(|e| warn!("Failed to parse current workers JSON: {e}"))
                            .unwrap_or_default(),
                        Err(e) => {
                            warn!("Failed to get current workers data from API: {e}");
                            CurrentUserWorkerStats::default()
                        }
                    };
                    display_controller.update_current_workers(
                        scene_id.clone(),
                        widget_id.clone(),
                        workers_stats,
                        number_format.clone(),
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
                    let mut worker_history = UserWorkerHistory::default();
                    let mut next_cursor: Option<String> = None;
                    let url = format!(
                        "{}{}",
                        pool_data::POOL_API_URL,
                        pool_data::USER_WORKERS_HISTORY
                    );
                    loop {
                        let mut query_params = vec![
                            (pool_data::FROM_TIMESTAMP, from_timestamp.as_str()),
                            (pool_data::TO_TIMESTAMP, to_timestamp.as_str()),
                            (pool_data::PAGE_LIMIT, pool_data::PAGE_LIMIT_MAX),
                        ];

                        if let Some(cursor) = &next_cursor {
                            query_params.push((pool_data::CURSOR, cursor));
                        }

                        let worker_history_partial = match client
                            .get(&url)
                            .header("X-API-Key", &api_key)
                            .query(&query_params)
                            .send()
                            .await
                        {
                            Ok(response) => match response.json::<UserWorkerHistory>().await {
                                Ok(data) => data,
                                Err(e) => {
                                    warn!("Failed to parse user worker history JSON: {e}");
                                    break;
                                }
                            },
                            Err(e) => {
                                warn!("Failed to get user worker history data from API: {e}");
                                break;
                            }
                        };
                        worker_history.merge_and_sort(&worker_history_partial);
                        next_cursor = worker_history_partial.next_cursor();
                        if next_cursor.is_none() {
                            break;
                        }
                    }
                    display_controller.update_worker_history(
                        scene_id.clone(),
                        widget_id.clone(),
                        worker_history,
                        number_format.clone(),
                    );
                }

                if download_payout_stats {
                    debug!("Getting user financials data...");
                    let user_financials = match client
                        .get(format!(
                            "{}{}",
                            pool_data::POOL_API_URL,
                            pool_data::USER_FINANCIALS
                        ))
                        .header("X-API-Key", &api_key)
                        .send()
                        .await
                    {
                        Ok(response) => response
                            .json::<UserFinancials>()
                            .await
                            .map_err(|e| warn!("Failed to parse user financials JSON: {e}"))
                            .unwrap_or_default(),
                        Err(e) => {
                            warn!("Failed to get user financials data from API: {e}");
                            UserFinancials::default()
                        }
                    };
                    debug!("Getting user recent payouts data...");
                    let recent_payouts = match client
                        .get(format!(
                            "{}{}",
                            pool_data::POOL_API_URL,
                            pool_data::USER_PAYOUTS_RECENT
                        ))
                        .header("X-API-Key", &api_key)
                        .query(&[(pool_data::PAGE_LIMIT, pool_data::PAGE_LIMIT_MAX)])
                        .send()
                        .await
                    {
                        Ok(response) => response
                            .json::<RecentUserPayouts>()
                            .await
                            .map_err(|e| warn!("Failed to parse user recent payouts JSON: {e}"))
                            .unwrap_or_default(),
                        Err(e) => {
                            warn!("Failed to get user recent payouts data from API: {e}");
                            RecentUserPayouts::default()
                        }
                    };
                    display_controller.update_payout_stats(
                        scene_id.clone(),
                        widget_id.clone(),
                        user_financials,
                        recent_payouts,
                        number_format,
                    );
                }

                if download_recent_payouts {
                    debug!("Getting user recent payouts data...");
                    let to_timestamp = chrono::Utc::now();
                    let from_timestamp = to_timestamp
                        .checked_sub_signed(chart_frame.clone().into())
                        // We don't expect this operation will fail
                        .unwrap_or_default();
                    let to_timestamp =
                        to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let from_timestamp =
                        from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let mut recent_payouts = RecentUserPayouts::default();
                    let mut next_cursor: Option<String> = None;
                    let url = format!(
                        "{}{}",
                        pool_data::POOL_API_URL,
                        pool_data::USER_PAYOUTS_RECENT
                    );
                    loop {
                        let mut query_params = vec![
                            (pool_data::FROM_TIMESTAMP, from_timestamp.as_str()),
                            (pool_data::TO_TIMESTAMP, to_timestamp.as_str()),
                            (pool_data::PAGE_LIMIT, pool_data::PAGE_LIMIT_MAX),
                        ];

                        if let Some(cursor) = &next_cursor {
                            query_params.push((pool_data::CURSOR, cursor));
                        }

                        let recent_payouts_partial = match client
                            .get(&url)
                            .header("X-API-Key", &api_key)
                            .query(&query_params)
                            .send()
                            .await
                        {
                            Ok(response) => match response.json::<RecentUserPayouts>().await {
                                Ok(data) => data,
                                Err(e) => {
                                    warn!("Failed to parse user recent payouts JSON: {e}");
                                    break;
                                }
                            },
                            Err(e) => {
                                warn!("Failed to get user recent payouts data from API: {e}");
                                break;
                            }
                        };
                        recent_payouts.merge_and_sort(&recent_payouts_partial);
                        next_cursor = recent_payouts_partial.next_cursor();
                        if next_cursor.is_none() {
                            break;
                        }
                    }
                    display_controller.update_recent_payouts(
                        scene_id.clone(),
                        widget_id.clone(),
                        recent_payouts,
                    );
                }
            }
        }
    }

    #[instrument(name = "remote_image", skip_all, fields(%scene_id, %widget_id))]
    fn make_remote_image_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        widget_size: WidgetSize,
        url: String,
        refresh_duration: Duration,
    ) -> impl Future<Output = ()> + Send + 'static {
        struct ResetToInitialStateDropGuard {
            scene_id: SceneId,
            widget_id: WidgetId,
            display_controller: DisplayController,
        }

        impl Drop for ResetToInitialStateDropGuard {
            fn drop(&mut self) {
                self.display_controller.update_remote_image(
                    self.scene_id.clone(),
                    self.widget_id.clone(),
                    RemoteImageState::Initial,
                );
            }
        }

        let error_backoff_builder = ExponentialBuilder::new()
            .with_min_delay(Duration::from_secs(10))
            .with_max_delay(Duration::from_secs(60 * 5).min(refresh_duration))
            .with_factor(2.0)
            .without_max_times();

        let mut error_backoff = error_backoff_builder.build();

        let display_controller = self.display_controller.clone();
        let widget_width = widget_size.width();
        let widget_height = widget_size.height();

        async move {
            info!(
                ?widget_size,
                widget_width,
                widget_height,
                url,
                ?refresh_duration,
                ?error_backoff,
                "Params"
            );

            let mut parsed_url = match Url::parse(&url) {
                Ok(url) => url,
                Err(err) => {
                    warn!(?err, "Invalid URL, stopping");
                    display_controller.update_remote_image(
                        scene_id.clone(),
                        widget_id.clone(),
                        RemoteImageState::ConfigurationError,
                    );
                    return;
                }
            };

            // NOTE: we provide dimensions in query params for advanced users.
            // They can implement single endpoint, which can dynamically generate image.
            // `deck_image_` prefix is here to prevent collisions with query params provided by the user.
            parsed_url
                .query_pairs_mut()
                .append_pair("deck_image_width", &widget_width.to_string())
                .append_pair("deck_image_height", &widget_height.to_string());

            let client = match Client::builder()
                .timeout(Duration::from_secs(120))
                // NOTE: we don't care, since we are not sending any sensitive data
                .danger_accept_invalid_certs(true)
                .build()
            {
                Ok(client) => client,
                Err(err) => {
                    warn!(?err, "Failed to create reqwest client, stopping");
                    display_controller.update_remote_image(
                        scene_id.clone(),
                        widget_id.clone(),
                        RemoteImageState::UnexpectedError,
                    );
                    return;
                }
            };

            // NOTE: intentionally initialized here, not at the beginning of the async block.
            // This way it will be dropped only when task is aborted.
            let _drop_guard = ResetToInitialStateDropGuard {
                scene_id: scene_id.clone(),
                widget_id: widget_id.clone(),
                display_controller: display_controller.clone(),
            };

            let mut decoder_limits = image::Limits::no_limits();
            decoder_limits.max_image_width = Some(widget_width);
            decoder_limits.max_image_height = Some(widget_height);

            let mut interval = interval(refresh_duration);
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

            display_controller.update_remote_image(
                scene_id.clone(),
                widget_id.clone(),
                RemoteImageState::Loading,
            );

            loop {
                // NOTE: this might take `refresh_duration` or `error_refresh_duration` time
                interval.tick().await;

                let state = async {
                    info!("Sending request to get remote image");

                    let start = Instant::now();

                    let bytes = client
                        .get(parsed_url.clone())
                        .send()
                        .await
                        .context("Failed to get remote image")?
                        .error_for_status()
                        .context("Server returned an error")?
                        .bytes()
                        .await
                        .context("Failed to read bytes from the response")?;

                    info!(duration = ?start.elapsed(), "Response received successfully");

                    let mut reader = image::ImageReader::new(io::Cursor::new(bytes));
                    reader.limits(decoder_limits.clone());
                    reader.set_format(image::ImageFormat::Png);

                    let decoder = reader
                        .with_guessed_format()
                        .expect("BUG: Seek for io::Cursor cannot fail")
                        .into_decoder()
                        .context("Failed to initialize image decoder")?;

                    let (width, height) = decoder.dimensions();

                    if width != widget_width || height != widget_height {
                        warn!(
                            width,
                            height,
                            expected_width = widget_width,
                            expected_height = widget_height,
                            "Unexpected image dimensions"
                        );
                        bail!("Unexpected image dimensions");
                    }

                    #[expect(clippy::wildcard_enum_match_arm)]
                    match decoder.color_type() {
                        image::ColorType::Rgb8 => {
                            let mut buffer = SharedPixelBuffer::new(width, height);

                            decoder
                                .read_image(buffer.make_mut_bytes())
                                .map(|()| SharedImageBuffer::RGB8(buffer))
                        }
                        image::ColorType::Rgba8 => {
                            let mut buffer = SharedPixelBuffer::new(width, height);

                            decoder
                                .read_image(buffer.make_mut_bytes())
                                .map(|()| SharedImageBuffer::RGBA8(buffer))
                        }
                        color_type => {
                            bail!("Unexpected color type: {color_type:?}");
                        }
                    }
                    .context("Failed to decode image")
                }
                .in_current_span()
                .await
                .map_or_else(
                    RemoteImageState::LoadingError,
                    RemoteImageState::LoadingSuccess,
                );

                if let RemoteImageState::LoadingError(err) = &state {
                    warn!(?err);

                    if let Some(duration) = error_backoff.next() {
                        interval.reset_after(duration);
                    }
                } else {
                    // NOTE: backoff does not have `reset` method, so we need to recreate it
                    error_backoff = error_backoff_builder.build();
                }

                display_controller.update_remote_image(scene_id.clone(), widget_id.clone(), state);
            }
        }
        .in_current_span()
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
                    let diff_hashrate_data = match client
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

                    let number_format = config_handle
                        .read()
                        .await
                        .localization_config()
                        .number_format;

                    display_controller.update_diff_hashrate_graph(
                        scene_id.clone(),
                        widget_id.clone(),
                        diff_hashrate_data,
                        number_format,
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
