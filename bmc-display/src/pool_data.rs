// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::{BraiinsPoolPayouts, MainWindow, Palette, PoolPayoutType, PoolWorkerStatus};
use crate::graph_utils::{self, ColorPalette};
use bmc_shared_time::time::{DateFormat, Timezone};
use bmc_shared_utils::number_format::NumberFormat;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use slint::{Global, Image, ModelRc, SharedString, VecModel};
use svg::Document;

pub const POOL_API_URL: &str = "https://api.braiins.com/pool/v2";

pub const USER_HASHRATE_CURRENT: &str = "/user/hashrate/current";
pub const USER_REWARD_LATEST: &str = "/user/rewards/latest";
pub const USER_WORKERS_CURRENT: &str = "/user/workers/current";
pub const USER_HASHRATE_HISTORY: &str = "/user/hashrate/history";
pub const USER_WORKERS_HISTORY: &str = "/user/workers/history";
pub const USER_FINANCIALS: &str = "/user/financials";
pub const USER_PAYOUTS_RECENT: &str = "/user/payouts/recent";

pub const FROM_TIMESTAMP: &str = "from_timestamp";
pub const TO_TIMESTAMP: &str = "to_timestamp";
pub const PAGE_LIMIT: &str = "page_limit";
pub const PAGE_LIMIT_MAX: &str = "1000";
pub const CURSOR: &str = "page_cursor";

const FORMAT_24H: &str = "%H:%M";
const FORMAT_12H: &str = "%I:%M %p";

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CurrentUserHashrate {
    hashrate_th_per_sec: f32,
}

impl CurrentUserHashrate {
    #[must_use]
    pub fn hashrate_as_shared(&self, number_format: NumberFormat) -> SharedString {
        let hashrate_per_sec = if self.hashrate_th_per_sec > 1000.0 {
            self.hashrate_th_per_sec / 1000.0
        } else {
            self.hashrate_th_per_sec
        };
        SharedString::from(number_format.format_number(hashrate_per_sec, 1))
    }

    #[must_use]
    pub fn hashrate_units(&self) -> SharedString {
        if self.hashrate_th_per_sec > 1000.0 {
            SharedString::from("PH/s")
        } else {
            SharedString::from("TH/s")
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct HashrateSlot {
    slot_start: DateTime<Utc>,
    hashrate_th_per_sec: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PaginationMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    has_next: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct UserHashrateHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    from_timestamp: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_timestamp: Option<DateTime<Utc>>,
    slots: Vec<HashrateSlot>,
    pagination: PaginationMetadata,
}

impl UserHashrateHistory {
    #[must_use]
    pub fn into_graph_image(
        &self,
        main_window: &MainWindow,
        width: u32,
        height: u32,
        draw_extra_line: bool,
    ) -> Image {
        let data: Vec<f64> = self
            .slots
            .iter()
            .map(|slot| slot.hashrate_th_per_sec)
            .collect();

        let palette = Palette::get(main_window);
        let palette = ColorPalette::new(&palette);
        // Align horizontal lines with y axis units
        let height = height - 24;

        let canvas =
            graph_utils::draw_canvas(width, height, draw_extra_line, false, &palette.gray_80);
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

        graph_utils::svg_into_image(&document, width, height)
    }

    #[must_use]
    pub fn graph_units(&self, number_format: NumberFormat) -> ModelRc<SharedString> {
        let max = self
            .slots
            .iter()
            .map(|slot| slot.hashrate_th_per_sec)
            .filter(|x| !x.is_nan())
            .max_by(f64::total_cmp)
            .unwrap_or(3.0);

        let max = graph_utils::y_axis_max(max, false);
        // TH/s -> PH/s
        let max = if max > 1000.0 { max / 1000.0 } else { max };
        ModelRc::new(VecModel::from_iter(
            [max, 2.0 * max / 3.0, max / 3.0, 0.0]
                .iter()
                .map(|unit| SharedString::from(number_format.format_number(*unit, 1)))
                .collect::<Vec<SharedString>>(),
        ))
    }

    #[must_use]
    pub fn hashrate_units(&self) -> SharedString {
        let max = self
            .slots
            .iter()
            .map(|slot| slot.hashrate_th_per_sec)
            .filter(|x| !x.is_nan())
            .max_by(f64::total_cmp)
            .unwrap_or(3.0);

        if max > 1000.0 {
            SharedString::from("PH/s")
        } else {
            SharedString::from("TH/s")
        }
    }

    #[must_use]
    pub fn timestamps(
        self,
        system_timezone: &Timezone,
        is_24_format: bool,
        date_format: DateFormat,
    ) -> ModelRc<SharedString> {
        let Some(from_timestamp) = self.from_timestamp else {
            return ModelRc::default();
        };
        let Some(to_timestamp) = self.to_timestamp else {
            return ModelRc::default();
        };
        let from_timestamp = from_timestamp.with_timezone(system_timezone.chrono());
        let to_timestamp = to_timestamp.with_timezone(system_timezone.chrono());
        let time_interval = to_timestamp - from_timestamp;

        let num_days = time_interval.num_days();
        let (count, str_format) = if num_days > 1 {
            let display_format = match date_format {
                DateFormat::DdMmYyyyDot => "%d.%m",
                DateFormat::DdMmYyyyDash => "%d-%m",
                DateFormat::DMYyyySlash => "%-d/%-m",
                DateFormat::DdMmYyyySlash => "%d/%m",
                DateFormat::MDYyyySlash | DateFormat::YyyyMDSlash => "%-m/%-d",
                DateFormat::YyyyMmDdDot => "%m.%d",
                DateFormat::YyyyMmDdDash => "%m-%d",
            };
            #[expect(clippy::cast_possible_truncation)]
            (1 + num_days as i32, display_format)
        } else {
            (5, if is_24_format { FORMAT_24H } else { FORMAT_12H })
        };
        let time_increment = time_interval / (count - 1);

        #[expect(clippy::cast_sign_loss)]
        let mut timestamps = Vec::with_capacity(count as usize);
        for i in 0..count {
            timestamps.push(from_timestamp + time_increment * i);
        }

        ModelRc::new(VecModel::from_iter(
            timestamps
                .iter()
                .map(|timestamp| timestamp.format(str_format).to_string())
                .map(SharedString::from)
                .collect::<Vec<SharedString>>(),
        ))
    }

    #[must_use]
    pub fn next_cursor(&self) -> Option<String> {
        self.pagination
            .has_next
            .and(self.pagination.next_cursor.clone())
    }

    pub fn merge_and_sort(&mut self, other: &Self) {
        self.from_timestamp = match (self.from_timestamp, other.from_timestamp) {
            (Some(self_time), Some(other_time)) => Some(self_time.min(other_time)),
            (Some(self_time), None) => Some(self_time),
            (None, Some(other_time)) => Some(other_time),
            (None, None) => None,
        };
        self.to_timestamp = match (self.to_timestamp, other.to_timestamp) {
            (Some(self_time), Some(other_time)) => Some(self_time.max(other_time)),
            (Some(self_time), None) => Some(self_time),
            (None, Some(other_time)) => Some(other_time),
            (None, None) => None,
        };
        self.slots.extend(other.slots.clone());
        self.slots.sort_by_key(|slot| slot.slot_start);
    }
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerSlot {
    slot_start: DateTime<Utc>,
    active_workers: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct UserWorkerHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    from_timestamp: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_timestamp: Option<DateTime<Utc>>,
    slots: Vec<WorkerSlot>,
    pagination: PaginationMetadata,
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
        let data: Vec<f64> = self
            .slots
            .into_iter()
            .map(|slot| f64::from(slot.active_workers))
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
                graph_utils::blend_svg_with_image(&document, &bg_buffer, width, height)
            {
                blended_image
            } else {
                original_image.clone()
            }
        } else {
            let canvas =
                graph_utils::draw_canvas(width, height, draw_extra_line, false, &palette.gray_80);
            let document = canvas.add(path);
            graph_utils::svg_into_image(&document, width, height)
        }
    }

    #[must_use]
    pub fn graph_units(&self, number_format: NumberFormat) -> ModelRc<SharedString> {
        let max = self
            .slots
            .iter()
            .map(|slot| slot.active_workers)
            .max()
            .unwrap_or(0)
            .max(3); // Default unit max

        // Shift max
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let max = 2 * graph_utils::y_axis_max(f64::from(max), true) as u32;

        let units: Vec<SharedString> = if max > 3000 {
            #[expect(clippy::integer_division)]
            [max / 1000, 2 * max / 3000, max / 3000, 0]
                .map(|unit| {
                    SharedString::from(format!("{}k", number_format.format_number(unit, 1)))
                })
                .to_vec()
        } else if max >= 3 {
            #[expect(clippy::integer_division)]
            [max, 2 * max / 3, max / 3, 0]
                .map(|unit| SharedString::from(number_format.format_number(unit, 0)))
                .to_vec()
        } else {
            [max, 0]
                .map(|unit| SharedString::from(format!("{unit}")))
                .to_vec()
        };

        ModelRc::new(VecModel::from_iter(units))
    }

    #[must_use]
    pub fn next_cursor(&self) -> Option<String> {
        self.pagination
            .has_next
            .and(self.pagination.next_cursor.clone())
    }

    pub fn merge_and_sort(&mut self, other: &Self) {
        self.from_timestamp = match (self.from_timestamp, other.from_timestamp) {
            (Some(self_time), Some(other_time)) => Some(self_time.min(other_time)),
            (Some(self_time), None) => Some(self_time),
            (None, Some(other_time)) => Some(other_time),
            (None, None) => None,
        };
        self.to_timestamp = match (self.to_timestamp, other.to_timestamp) {
            (Some(self_time), Some(other_time)) => Some(self_time.max(other_time)),
            (Some(self_time), None) => Some(self_time),
            (None, Some(other_time)) => Some(other_time),
            (None, None) => None,
        };
        self.slots.extend(other.slots.clone());
        self.slots.sort_by_key(|slot| slot.slot_start);
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CurrentUserWorkerStats {
    active_workers: u32,
    low_workers: u32,
    offline_workers: u32,
}

impl CurrentUserWorkerStats {
    #[must_use]
    pub fn worker_stats(self, number_format: NumberFormat) -> PoolWorkerStatus {
        PoolWorkerStatus {
            total: SharedString::from(number_format.format_number(
                self.active_workers + self.low_workers + self.offline_workers,
                0,
            )),
            active: SharedString::from(number_format.format_number(self.active_workers, 0)),
            low: SharedString::from(number_format.format_number(self.low_workers, 0)),
            offline: SharedString::from(number_format.format_number(self.offline_workers, 0)),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct FinancialAccount {
    #[serde(skip_serializing_if = "Option::is_none")]
    next_payout_at_estimate: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
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

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LatestUserRewards {
    todays_reward_estimate_btc: f32,
    todays_reward_estimate_usd: f32,
}

impl LatestUserRewards {
    #[must_use]
    pub fn today_reward_btc(&self, number_format: NumberFormat) -> SharedString {
        SharedString::from(format!(
            "{} BTC",
            number_format.format_number(self.todays_reward_estimate_btc, 6)
        ))
    }

    #[must_use]
    pub fn today_reward_usd(&self, number_format: NumberFormat) -> SharedString {
        SharedString::from(format!(
            "~ {} USD",
            number_format.format_number(self.todays_reward_estimate_usd, 3)
        ))
    }
}

#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PayoutType {
    #[default]
    Onchain,
    Lightning,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum PayoutStatus {
    #[default]
    Pending,
    Failed,
    Completed,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Payout {
    occurred_at: DateTime<Utc>,
    amount_btc: f32,
    r#type: PayoutType,
    status: PayoutStatus,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RecentUserPayouts {
    #[serde(skip_serializing_if = "Option::is_none")]
    from_timestamp: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_timestamp: Option<DateTime<Utc>>,
    payouts: Vec<Payout>,
    pagination: PaginationMetadata,
}

impl RecentUserPayouts {
    #[must_use]
    pub fn last_payout_to_shared(&self, number_format: NumberFormat) -> SharedString {
        let now = Utc::now();
        self.payouts
            .iter()
            .filter(|payout| payout.status == PayoutStatus::Completed)
            .min_by_key(|payout| (now - payout.occurred_at).abs().num_seconds())
            .map_or(SharedString::default(), |payout| {
                SharedString::from(format!(
                    "{} BTC",
                    number_format.format_number(payout.amount_btc, 6)
                ))
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

    #[must_use]
    pub fn payouts(self) -> ModelRc<BraiinsPoolPayouts> {
        let Some(from_timestamp) = self.from_timestamp else {
            return ModelRc::default();
        };
        let Some(to_timestamp) = self.to_timestamp else {
            return ModelRc::default();
        };
        let total_interval = to_timestamp - from_timestamp;

        let braiins_pool_payouts: Vec<BraiinsPoolPayouts> = self
            .payouts
            .iter()
            .filter(|payout| payout.status == PayoutStatus::Completed)
            .filter_map(|payout| {
                let payout_interval = payout.occurred_at - from_timestamp;
                if payout_interval.num_seconds() >= 0 {
                    #[expect(clippy::cast_precision_loss)]
                    let fraction = 100.0 * payout_interval.num_seconds() as f32
                        / total_interval.num_seconds() as f32;
                    Some(BraiinsPoolPayouts {
                        payout_time_fraction: fraction,
                        payout_type: payout.r#type.into(),
                    })
                } else {
                    None
                }
            })
            .collect();

        ModelRc::new(VecModel::from(braiins_pool_payouts))
    }

    #[must_use]
    pub fn next_cursor(&self) -> Option<String> {
        self.pagination
            .has_next
            .and(self.pagination.next_cursor.clone())
    }

    pub fn merge_and_sort(&mut self, other: &Self) {
        self.from_timestamp = match (self.from_timestamp, other.from_timestamp) {
            (Some(self_time), Some(other_time)) => Some(self_time.min(other_time)),
            (Some(self_time), None) => Some(self_time),
            (None, Some(other_time)) => Some(other_time),
            (None, None) => None,
        };
        self.to_timestamp = match (self.to_timestamp, other.to_timestamp) {
            (Some(self_time), Some(other_time)) => Some(self_time.max(other_time)),
            (Some(self_time), None) => Some(self_time),
            (None, Some(other_time)) => Some(other_time),
            (None, None) => None,
        };
        self.payouts.extend(other.payouts.clone());
        self.payouts.sort_by_key(|payout| payout.occurred_at);
    }
}

impl From<PayoutType> for PoolPayoutType {
    fn from(value: PayoutType) -> Self {
        match value {
            PayoutType::Onchain => PoolPayoutType::Onchain,
            PayoutType::Lightning => PoolPayoutType::Lighting,
        }
    }
}
