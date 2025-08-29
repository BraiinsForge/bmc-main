// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{MainWindow, Palette, PoolWorkerStatus};
use crate::graph_utils::{self, ColorPalette};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use slint::{Global, Image, SharedString};
use svg::Document;

pub const POOL_API_URL: &str = "https://pool.braiins.com/api/v1";
pub const USER_HASHRATE_CURRENT: &str = "/user/hashrate/current";
pub const USER_REWARD_LATEST: &str = "/user/rewards/latest";
pub const USER_WORKERS_CURRENT: &str = "/user/workers/current";
pub const USER_HASHRATE_HISTORY: &str = "/user/hashrate/history";
pub const USER_WORKERS_HISTORY: &str = "/user/workers/history";
pub const USER_FINANCIALS: &str = "/user/financials";
pub const USER_PAYOUTS_RECENT: &str = "/user/payouts/recent";
pub const FROM_TIMESTAMP: &str = "from_timestamp";
pub const TO_TIMESTAMP: &str = "to_timestamp";

#[derive(Debug, Default, Deserialize)]
pub struct CurrentUserHashrate {
    hashrate_th_per_sec: f32,
}

impl CurrentUserHashrate {
    #[must_use]
    pub fn hashrate_as_shared(self) -> SharedString {
        SharedString::from(format!("{:.1}", self.hashrate_th_per_sec))
    }
}

#[derive(Debug, Deserialize)]
struct HashrateSlot {
    hashrate_th_per_sec: f32,
}

#[derive(Debug, Default, Deserialize)]
pub struct UserHashrateHistory {
    slots: Vec<HashrateSlot>,
}

impl UserHashrateHistory {
    #[must_use]
    pub fn into_graph_image(
        self,
        main_window: &MainWindow,
        width: u32,
        height: u32,
        draw_extra_line: bool,
    ) -> Image {
        let data: Vec<f32> = self
            .slots
            .into_iter()
            .map(|slot| slot.hashrate_th_per_sec)
            .collect();

        let palette = Palette::get(main_window);
        let palette = ColorPalette::new(&palette);
        // Align horizontal lines with y axis units
        let height = height - 24;

        let canvas = graph_utils::draw_canvas(width, height, draw_extra_line, &palette.gray_80);
        let path = graph_utils::create_graph(
            &data,
            width,
            height,
            &palette.violet_60,
            // According to design the extra line corresponds to the use of absolute values
            draw_extra_line,
            false,
            None,
        )
        .unwrap_or_default();
        let document = canvas.add(path);

        graph_utils::svg_into_image(document, width, height)
    }
}

#[derive(Debug, Deserialize)]
struct WorkerSlot {
    active_workers: u32,
}

#[derive(Debug, Default, Deserialize)]
pub struct UserWorkerHistory {
    slots: Vec<WorkerSlot>,
}

impl UserWorkerHistory {
    #[must_use]
    pub fn into_graph_image(
        self,
        main_window: &MainWindow,
        width: u32,
        height: u32,
        draw_extra_line: bool,
        original_image: &Image,
    ) -> Image {
        let data: Vec<f32> = self
            .slots
            .into_iter()
            .map(|slot| slot.active_workers as f32)
            .collect();
        let palette = Palette::get(main_window);
        let palette = ColorPalette::new(&palette);
        // Align horizontal lines with y axis units
        let height = height - 24;

        let path = graph_utils::create_graph(
            &data,
            width,
            height,
            &palette.blue_30,
            true,
            true,
            Some(0.5),
        )
        .unwrap_or_default();

        if let Some(bg_buffer) = original_image.to_rgba8() {
            let document = Document::new()
                .set("viewBox", (0, 0, width, height))
                .set("width", width)
                .set("height", height)
                .add(path);
            if let Some(blended_image) =
                graph_utils::blend_svg_with_image(document, bg_buffer, width, height)
            {
                blended_image
            } else {
                original_image.clone()
            }
        } else {
            let canvas = graph_utils::draw_canvas(width, height, draw_extra_line, &palette.gray_80);
            let document = canvas.add(path);
            graph_utils::svg_into_image(document, width, height)
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CurrentUserWorkerStats {
    active_workers: u32,
    low_workers: u32,
    offline_workers: u32,
}

impl CurrentUserWorkerStats {
    #[must_use]
    pub fn worker_stats(self) -> PoolWorkerStatus {
        PoolWorkerStatus {
            total: SharedString::from(format!(
                "{}",
                self.active_workers + self.low_workers + self.offline_workers
            )),
            active: SharedString::from(format!("{}", self.active_workers)),
            low: SharedString::from(format!("{}", self.low_workers)),
            offline: SharedString::from(format!("{}", self.offline_workers)),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FinancialAccount {
    #[serde(skip_serializing_if = "Option::is_none")]
    next_payout_at_estimate: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UserFinancials {
    financial_accounts: Vec<FinancialAccount>,
}

impl UserFinancials {
    #[must_use]
    pub fn next_payout_estimate_to_shared(&self) -> SharedString {
        let now = Utc::now();
        let next_payout = self
            .financial_accounts
            .iter()
            .filter_map(|account| account.next_payout_at_estimate)
            .map(|dt| (now - dt).abs().num_minutes())
            .min();

        next_payout
            .map(|estimate| {
                if estimate > 60 {
                    let hours = estimate.div_euclid(60);
                    format!(" {} {}", hours, if hours == 1 { "Hour" } else { "Hours" })
                } else {
                    format!(
                        " {} {}",
                        estimate,
                        if estimate == 1 { "Minute" } else { "Minutes" }
                    )
                }
            })
            .map_or(SharedString::default(), SharedString::from)
    }

    #[must_use]
    pub fn next_payout_estimate(&self) -> Option<DateTime<Utc>> {
        let now = Utc::now();
        self.financial_accounts
            .iter()
            .filter_map(|account| account.next_payout_at_estimate)
            .min_by_key(|dt| (now - dt).abs().num_milliseconds())
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct LatestUserRewards {
    todays_reward_estimate_btc: f32,
    todays_reward_estimate_usd: f32,
}

impl LatestUserRewards {
    #[must_use]
    pub fn today_reward_btc(&self) -> SharedString {
        SharedString::from(format!("{:.6} BTC", self.todays_reward_estimate_btc))
    }

    #[must_use]
    pub fn today_reward_usd(&self) -> SharedString {
        SharedString::from(format!("~ {:.1} USD", self.todays_reward_estimate_usd))
    }
}

#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PayoutType {
    #[default]
    Onchain,
    Lightning,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PayoutStatus {
    #[default]
    Pending,
    Failed,
    Completed,
}

#[derive(Debug, Default, Deserialize)]
struct Payout {
    occurred_at: DateTime<Utc>,
    amount_btc: f32,
    r#type: PayoutType,
    status: PayoutStatus,
}

#[derive(Debug, Default, Deserialize)]
pub struct RecentUserPayouts {
    payouts: Vec<Payout>,
}

impl RecentUserPayouts {
    #[must_use]
    pub fn last_payout_to_shared(&self) -> SharedString {
        let now = Utc::now();
        self.payouts
            .iter()
            .filter(|payout| payout.status == PayoutStatus::Completed)
            .min_by_key(|payout| (now - payout.occurred_at).abs().num_seconds())
            .map_or(SharedString::default(), |payout| {
                SharedString::from(format!("{:.6} BTC", payout.amount_btc))
            })
    }

    #[must_use]
    pub fn last_payout_datetime(&self) -> Option<DateTime<Utc>> {
        let now = Utc::now();
        self.payouts
            .iter()
            .filter(|payout| payout.status == PayoutStatus::Completed)
            .min_by_key(|payout| (now - payout.occurred_at).abs().num_seconds())
            .map(|payout| payout.occurred_at)
    }
}
