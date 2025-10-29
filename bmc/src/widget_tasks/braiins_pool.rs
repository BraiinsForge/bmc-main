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
use chrono::{DateTime, Utc};
use reqwest::Client;
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

    info!(
        ?pool_style,
        ?chart_frame,
        ?account_id,
        download_rewards,
        download_hashrate_history,
        download_workers_stats,
        download_workers_history,
        download_payout_stats,
        download_recent_payouts,
        "Params"
    );

    let Some(account_id) = account_id else {
        warn!(widget_id = %widget_id, "Widget is missing Account ID");
        return;
    };

    let client = match Client::builder().timeout(API_TIMEOUT).build() {
        Ok(client) => client,
        Err(err) => {
            error!(?err, "Failed to create reqwest client, stopping");
            return;
        }
    };

    let mut interval = interval(DATA_REFRESH_PERIOD);

    loop {
        interval.tick().await;

        let (api_key, account_name) =
            if let Some(account) = config_handle.read().await.accounts.get(&account_id) {
                let auth = match &account.authentication {
                    AuthenticationType::ApiKey(api_key) => api_key.clone(),
                };
                (auth, account.name.clone())
            } else {
                warn!(account_id = %account_id, "Missing account with ID");
                return;
            };

        let number_format = config_handle
            .read()
            .await
            .localization_config()
            .number_format;

        display_controller.update_account_name(scene_id.clone(), widget_id.clone(), account_name);

        debug!("Fetching user current hashrate data");
        let current_hashrate = download_current_hashrate_data(&client, &api_key).await;
        display_controller.update_current_user_hashrate(
            scene_id.clone(),
            widget_id.clone(),
            current_hashrate,
            number_format.clone(),
        );

        if download_rewards {
            debug!("Fetching user latest rewards data");
            let latest_rewards = download_latest_rewards_data(&client, &api_key).await;
            display_controller.update_rewards_latest(
                scene_id.clone(),
                widget_id.clone(),
                latest_rewards,
                number_format.clone(),
            );
        }

        if download_hashrate_history {
            debug!("Fetching user hashrate history data");
            let to_timestamp = Utc::now();
            let from_timestamp = to_timestamp
                .checked_sub_signed(chart_frame.clone().into())
                // We don't expect this operation will fail
                .unwrap_or_default();

            let hashrate_history = download_paginated_hashrate_history_data(
                &client,
                &api_key,
                from_timestamp,
                to_timestamp,
            )
            .await;

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
            debug!("Fetching current workers data");
            let workers_stats = download_worker_stats_data(&client, &api_key).await;
            display_controller.update_current_workers(
                scene_id.clone(),
                widget_id.clone(),
                workers_stats,
                number_format.clone(),
            );
        }

        if download_workers_history {
            debug!("Fetching user worker history data");
            let to_timestamp = Utc::now();
            let from_timestamp = to_timestamp
                .checked_sub_signed(chart_frame.clone().into())
                // We don't expect this operation will fail
                .unwrap_or_default();

            let worker_history = download_paginated_worker_history_data(
                &client,
                &api_key,
                from_timestamp,
                to_timestamp,
            )
            .await;

            display_controller.update_worker_history(
                scene_id.clone(),
                widget_id.clone(),
                worker_history,
                number_format.clone(),
            );
        }

        if download_payout_stats {
            debug!("Fetching user financials data");
            let user_financials = download_user_financials_data(&client, &api_key).await;
            debug!("Fetching user recent payouts data");
            let recent_payouts = download_recent_payouts_data(&client, &api_key).await;
            display_controller.update_payout_stats(
                scene_id.clone(),
                widget_id.clone(),
                user_financials,
                recent_payouts,
                number_format,
            );
        }

        if download_recent_payouts {
            debug!("Fetching user recent payouts data");
            let to_timestamp = Utc::now();
            let from_timestamp = to_timestamp
                .checked_sub_signed(chart_frame.clone().into())
                // We don't expect this operation will fail
                .unwrap_or_default();

            let recent_payouts = download_paginated_recent_payouts_data(
                &client,
                &api_key,
                from_timestamp,
                to_timestamp,
            )
            .await;

            display_controller.update_recent_payouts(
                scene_id.clone(),
                widget_id.clone(),
                recent_payouts,
            );
        }
    }
}

async fn download_current_hashrate_data(client: &Client, api_key: &str) -> CurrentUserHashrate {
    let request = client
        .get(format!(
            "{}{}",
            pool_data::POOL_API_URL,
            pool_data::USER_HASHRATE_CURRENT
        ))
        .header("X-API-Key", api_key);

    match request.send().await {
        Ok(response) => response
            .json::<CurrentUserHashrate>()
            .await
            .inspect_err(
                |err| error!(error = %err, "Failed to parse user current hashrate JSON response"),
            )
            .unwrap_or_default(),
        Err(err) => {
            warn!(error = %err, "Failed to fetch user current hashrate data from API");
            CurrentUserHashrate::default()
        }
    }
}

async fn download_latest_rewards_data(client: &Client, api_key: &str) -> LatestUserRewards {
    let request = client
        .get(format!(
            "{}{}",
            pool_data::POOL_API_URL,
            pool_data::USER_REWARD_LATEST
        ))
        .header("X-API-Key", api_key);

    match request.send().await {
        Ok(response) => response
            .json::<LatestUserRewards>()
            .await
            .inspect_err(
                |err| error!(error = %err, "Failed to parse user latest rewards JSON response"),
            )
            .unwrap_or_default(),
        Err(err) => {
            warn!(error = %err, "Failed to fetch user latest rewards data from API");
            LatestUserRewards::default()
        }
    }
}

async fn download_paginated_hashrate_history_data(
    client: &Client,
    api_key: &str,
    from_timestamp: DateTime<Utc>,
    to_timestamp: DateTime<Utc>,
) -> UserHashrateHistory {
    let from_timestamp = from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to_timestamp = to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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

        let request = client
            .get(&url)
            .header("X-API-Key", api_key)
            .query(&query_params);

        let hashrate_history_partial = match request.send().await {
            Ok(response) => match response.json::<UserHashrateHistory>().await {
                Ok(data) => data,
                Err(err) => {
                    error!(error = %err, "Failed to parse user hashrate history JSON response");
                    break;
                }
            },
            Err(err) => {
                warn!(error = %err, "Failed to fetch user hashrate history data from API");
                break;
            }
        };
        hashrate_history.merge_and_sort(&hashrate_history_partial);
        next_cursor = hashrate_history_partial.next_cursor();
        if next_cursor.is_none() {
            break;
        }
    }

    hashrate_history
}

async fn download_worker_stats_data(client: &Client, api_key: &str) -> CurrentUserWorkerStats {
    let request = client
        .get(format!(
            "{}{}",
            pool_data::POOL_API_URL,
            pool_data::USER_WORKERS_CURRENT
        ))
        .header("X-API-Key", api_key);

    match request.send().await {
        Ok(response) => response
            .json::<CurrentUserWorkerStats>()
            .await
            .inspect_err(
                |err| error!(error = %err, "Failed to parse current workers JSON response"),
            )
            .unwrap_or_default(),
        Err(err) => {
            warn!(error = %err, "Failed to fetch current workers data from API");
            CurrentUserWorkerStats::default()
        }
    }
}

async fn download_paginated_worker_history_data(
    client: &Client,
    api_key: &str,
    from_timestamp: DateTime<Utc>,
    to_timestamp: DateTime<Utc>,
) -> UserWorkerHistory {
    let from_timestamp = from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to_timestamp = to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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

        let request = client
            .get(&url)
            .header("X-API-Key", api_key)
            .query(&query_params);

        let worker_history_partial = match request.send().await {
            Ok(response) => match response.json::<UserWorkerHistory>().await {
                Ok(data) => data,
                Err(err) => {
                    error!(error = %err, "Failed to parse user worker history JSON response");
                    break;
                }
            },
            Err(err) => {
                warn!(error = %err, "Failed to fetch user worker history data from API");
                break;
            }
        };
        worker_history.merge_and_sort(&worker_history_partial);
        next_cursor = worker_history_partial.next_cursor();
        if next_cursor.is_none() {
            break;
        }
    }

    worker_history
}

async fn download_user_financials_data(client: &Client, api_key: &str) -> UserFinancials {
    let request = client
        .get(format!(
            "{}{}",
            pool_data::POOL_API_URL,
            pool_data::USER_FINANCIALS
        ))
        .header("X-API-Key", api_key);

    match request.send().await {
        Ok(response) => response
            .json::<UserFinancials>()
            .await
            .inspect_err(
                |err| error!(error = %err, "Failed to parse user financials JSON response"),
            )
            .unwrap_or_default(),
        Err(err) => {
            warn!(error = %err, "Failed to fetch user financials data from API");
            UserFinancials::default()
        }
    }
}

async fn download_recent_payouts_data(client: &Client, api_key: &str) -> RecentUserPayouts {
    let request = client
        .get(format!(
            "{}{}",
            pool_data::POOL_API_URL,
            pool_data::USER_PAYOUTS_RECENT
        ))
        .header("X-API-Key", api_key)
        .query(&[(pool_data::PAGE_LIMIT, pool_data::PAGE_LIMIT_MAX)]);

    match request.send().await {
        Ok(response) => response
            .json::<RecentUserPayouts>()
            .await
            .inspect_err(
                |err| error!(error = %err, "Failed to parse user recent payouts JSON response"),
            )
            .unwrap_or_default(),
        Err(err) => {
            warn!(error = %err, "Failed to fetch user recent payouts data from API");
            RecentUserPayouts::default()
        }
    }
}

async fn download_paginated_recent_payouts_data(
    client: &Client,
    api_key: &str,
    from_timestamp: DateTime<Utc>,
    to_timestamp: DateTime<Utc>,
) -> RecentUserPayouts {
    let from_timestamp = from_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to_timestamp = to_timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
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

        let request = client
            .get(&url)
            .header("X-API-Key", api_key)
            .query(&query_params);

        let recent_payouts_partial = match request.send().await {
            Ok(response) => match response.json::<RecentUserPayouts>().await {
                Ok(data) => data,
                Err(err) => {
                    error!(error = %err, "Failed to parse user recent payouts JSON response");
                    break;
                }
            },
            Err(err) => {
                warn!(error = %err, "Failed to fetch user recent payouts data from API");
                break;
            }
        };
        recent_payouts.merge_and_sort(&recent_payouts_partial);
        next_cursor = recent_payouts_partial.next_cursor();
        if next_cursor.is_none() {
            break;
        }
    }

    recent_payouts
}
