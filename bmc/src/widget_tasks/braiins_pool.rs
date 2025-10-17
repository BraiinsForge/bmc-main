// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use crate::widget_tasks::{API_TIMEOUT, DATA_REFRESH_PERIOD};
use bmc_display::data::{
    AccountId, AuthenticationType, PoolChartTimeFrame, PoolStyle, SceneId, WidgetId, WidgetSize,
};
use bmc_display::display_controller::DisplayController;
use bmc_display::pool_data;
use bmc_display::pool_data::{
    CurrentUserHashrate, CurrentUserWorkerStats, LatestUserRewards, RecentUserPayouts,
    UserFinancials, UserHashrateHistory, UserWorkerHistory,
};
use bmc_shared_time::time::Timezone;
use std::sync::Arc;
use tokio::sync::{RwLock, watch};
use tokio::time::interval;
use tracing::{debug, error, info, instrument, warn};

#[expect(clippy::too_many_arguments)]
#[instrument(name = "braiins_pool", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    mut system_timezone_receiver: watch::Receiver<Timezone>,
    scene_id: SceneId,
    widget_id: WidgetId,
    widget_size: WidgetSize,
    config_handle: Arc<RwLock<ConfigHandle>>,
    pool_style: PoolStyle,
    chart_frame: PoolChartTimeFrame,
    account_id: Option<AccountId>,
) {
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

        display_controller.update_account_name(scene_id.clone(), widget_id.clone(), account_name);

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
            let to_timestamp = to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let from_timestamp = from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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
            let to_timestamp = to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let from_timestamp = from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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
            let to_timestamp = to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let from_timestamp = from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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
