// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Braiins Pool profile — a cloud API, not a LAN device: nothing is
//! announced, the resource is reached by its port (through the testbed's
//! `--rewrite-url`, since the widget dials `api.braiins.com`).
//!
//! Serves the FPPS account subset the `braiins-pool` widget reads
//! (`widgets-wasm/braiins-pool/src/pool_api.rs`). History is generated on
//! demand: each five-minute slot is a pure function of its absolute slot
//! time and the device seed, so any window depth replays identically and
//! pagination needs no state — the cursor is the next slot's offset.
//!
//! Credentials are never inspected; a denied account is scripted with
//! `status: 403` like the miner profiles script their auth failures.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::blueprint::{EndpointSpec, RequestCtx, ResourceSpec, Response, ResponseSpec};
use crate::build::{drift, leaf};
use crate::http_status::HttpStatus;
use crate::quantity::NonNegative;
use crate::value::Value;

/// The pool's history resolution.
const SLOT_SECS: i64 = 300;

/// The API's page cap; larger requested limits clamp to it.
const PAGE_CAP: usize = 1_000;

/// How many payouts the unwindowed "recent" page answers with at most.
const RECENT_PAYOUTS: usize = 30;

/// The timestamp shape the widget emits and parses (RFC 3339, UTC, whole
/// seconds) — `pool_api.rs::RFC3339_UTC`.
const RFC3339_UTC: &str = "%Y-%m-%dT%H:%M:%SZ";

/// Tunables for a simulated Braiins Pool account.
// Strict, unlike persisted state: a blueprint is hand-authored, so a mistyped
// key is a fault that would silently never fire rather than a forward-compatible
// field to skip over.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(rename = "BraiinsPoolParams")]
pub struct Params {
    /// 5-minute-average hashrate the account hovers around, in TH/s.
    pub hashrate_ths: NonNegative,
    /// Worker counts by state, as `/user/workers/current` reports them.
    pub workers_active: u64,
    pub workers_low: u64,
    pub workers_offline: u64,
    pub workers_disabled: u64,
    /// Today's reward estimate, in BTC.
    pub reward_btc: NonNegative,
    /// The same estimate in USD.
    pub reward_usd: NonNegative,
    /// Seconds between payouts, landing on the unix-epoch grid (86400 =
    /// daily at midnight UTC). `null` = an account that has never paid out:
    /// no history, no estimate, null progress.
    pub payout_period_s: Option<NonZeroU64>,
    /// Amount of every completed payout, in BTC.
    pub payout_amount_btc: NonNegative,
    /// HTTP status every endpoint returns; 401/403 = the denied account.
    pub status: HttpStatus,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            hashrate_ths: NonNegative::from(480.0),
            workers_active: 24,
            workers_low: 2,
            workers_offline: 1,
            workers_disabled: 0,
            reward_btc: NonNegative::from(0.000_17),
            reward_usd: NonNegative::from(10.04),
            payout_period_s: NonZeroU64::new(86_400),
            payout_amount_btc: NonNegative::from(0.000_38),
            status: HttpStatus::OK,
        }
    }
}

impl Params {
    #[must_use]
    pub fn resource(&self, name: &str, port: u16) -> ResourceSpec {
        let period = self
            .payout_period_s
            .map(|period| i64::try_from(period.get()).unwrap_or(i64::MAX));
        let amount = self.payout_amount_btc.get();
        let hashrate = self.hashrate_ths.get();
        let workers = f64::from(u32::try_from(self.workers_active).unwrap_or(u32::MAX));
        let status = self.status;
        let endpoints = vec![
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/pool/v2/user/hashrate/current".to_owned(),
                response: ResponseSpec::Render {
                    status,
                    template: json!({ "hashrate_th_per_sec": leaf(drift(hashrate)) }),
                },
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/pool/v2/user/rewards/latest".to_owned(),
                response: ResponseSpec::Render {
                    status,
                    template: json!({
                        "todays_reward_estimate_btc": self.reward_btc.get(),
                        "todays_reward_estimate_usd": self.reward_usd.get(),
                    }),
                },
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/pool/v2/user/workers/current".to_owned(),
                response: ResponseSpec::Render {
                    status,
                    template: json!({
                        "active_workers": self.workers_active,
                        "low_workers": self.workers_low,
                        "offline_workers": self.workers_offline,
                        "disabled_workers": self.workers_disabled,
                    }),
                },
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/pool/v2/user/hashrate/history".to_owned(),
                response: ResponseSpec::computed(move |ctx| {
                    Response::new(
                        status,
                        history(ctx, series_value(hashrate), "hashrate_th_per_sec"),
                    )
                }),
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/pool/v2/user/workers/history".to_owned(),
                response: ResponseSpec::computed(move |ctx| {
                    Response::new(
                        status,
                        history(ctx, series_value(workers), "active_workers"),
                    )
                }),
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/pool/v2/user/financials".to_owned(),
                response: ResponseSpec::computed(move |_| {
                    Response::new(status, financials(period))
                }),
            },
            EndpointSpec {
                method: "GET".to_owned(),
                path: "/pool/v2/user/payouts/recent".to_owned(),
                response: ResponseSpec::computed(move |ctx| {
                    Response::new(status, payouts(ctx, period, amount))
                }),
            },
        ];
        ResourceSpec {
            name: name.to_owned(),
            port,
            announce: None,
            endpoints,
            sampler: None,
        }
    }
}

/// The history line's shape: slower than [`crate::build::drift`]'s
/// five-minute sine, which the five-minute slot grid would sample
/// at a constant phase.
fn series_value(center: f64) -> Value {
    Value::Drift {
        center,
        amp: center * 0.08,
        period_s: 3.0 * 3_600.0,
        jitter: center * 0.02,
    }
}

fn parse_rfc3339(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.timestamp())
}

fn rfc3339(unix: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(unix, 0)
        .map(|at| at.format(RFC3339_UTC).to_string())
        .unwrap_or_default()
}

/// The request's `from`/`to` window and page slice.
struct Page {
    from: i64,
    to: i64,
    offset: usize,
    limit: usize,
}

impl Page {
    fn of(query: &BTreeMap<String, String>) -> Option<Self> {
        let from = parse_rfc3339(query.get("from_timestamp")?)?;
        let to = parse_rfc3339(query.get("to_timestamp")?)?;
        Some(Self {
            from,
            to,
            offset: query
                .get("page_cursor")
                .and_then(|cursor| cursor.parse().ok())
                .unwrap_or(0),
            limit: query
                .get("page_limit")
                .and_then(|limit| limit.parse().ok())
                .unwrap_or(PAGE_CAP)
                .min(PAGE_CAP),
        })
    }

    /// Slice `[offset, offset + limit)` out of `total`, with the pagination
    /// object naming the next offset as the cursor.
    fn slice(&self, total: usize) -> (std::ops::Range<usize>, Json) {
        let end = self.offset.saturating_add(self.limit).min(total);
        let range = self.offset.min(end)..end;
        let pagination = if end < total {
            json!({ "has_next": true, "next_cursor": end.to_string() })
        } else {
            json!({ "has_next": false })
        };
        (range, pagination)
    }
}

/// A windowed history page: five-minute slots as a pure function of their
/// absolute slot time, so every page and every rerun agrees.
fn history(ctx: &RequestCtx, value: Value, field: &'static str) -> Json {
    let Some(page) = Page::of(&ctx.query) else {
        return json!({ "slots": [] });
    };
    let first = page.from.div_euclid(SLOT_SECS) * SLOT_SECS;
    let first = if first < page.from {
        first + SLOT_SECS
    } else {
        first
    };
    let total = usize::try_from((page.to - first).div_euclid(SLOT_SECS) + 1).unwrap_or(0);
    let (range, pagination) = page.slice(total);
    let slots: Vec<Json> = range
        .map(|index| {
            let at = first + i64::try_from(index).unwrap_or(0) * SLOT_SECS;
            #[expect(
                clippy::cast_precision_loss,
                reason = "unix seconds are far below f64's exact integer range"
            )]
            let sampled = value.eval(at as f64, ctx.seed);
            json!({ "slot_start": rfc3339(at), field: sampled })
        })
        .collect();
    json!({
        "from_timestamp": rfc3339(page.from),
        "to_timestamp": rfc3339(page.to),
        "slots": slots,
        "pagination": pagination,
    })
}

/// The next payout: estimate at the next period boundary, progress as the
/// elapsed share of the current period — it advances in real time.
fn financials(period: Option<i64>) -> Json {
    let Some(period) = period else {
        return json!({ "financial_accounts": [] });
    };
    let now = chrono::Utc::now().timestamp();
    let next = (now.div_euclid(period) + 1) * period;
    #[expect(
        clippy::cast_precision_loss,
        reason = "a payout period fits f64 exactly"
    )]
    let progress = now.rem_euclid(period) as f64 / period as f64 * 100.0;
    json!({
        "financial_accounts": [{
            "next_payout_at_estimate": rfc3339(next),
            "next_payout_progress_pct": progress,
        }],
    })
}

/// Completed payouts on the period grid, oldest first. Windowed when the
/// query carries a window (the Big Chart's marker feed), the latest page
/// otherwise (the Overview's "last payout").
fn payouts(ctx: &RequestCtx, period: Option<i64>, amount: f64) -> Json {
    let Some(period) = period else {
        return json!({ "payouts": [], "pagination": { "has_next": false } });
    };
    let now = chrono::Utc::now().timestamp();
    let latest = now.div_euclid(period);
    let entry = |epoch: i64| {
        json!({
            "status": "COMPLETED",
            "type": if epoch % 2 == 0 { "ONCHAIN" } else { "LIGHTNING" },
            "occurred_at": rfc3339(epoch * period),
            "amount_btc": amount,
        })
    };
    if let Some(page) = Page::of(&ctx.query) {
        let first = page.from.div_euclid(period) + 1;
        let last = page.to.div_euclid(period).min(latest);
        let total = usize::try_from(last - first + 1).unwrap_or(0);
        let (range, pagination) = page.slice(total);
        let payouts: Vec<Json> = range
            .map(|index| entry(first + i64::try_from(index).unwrap_or(0)))
            .collect();
        return json!({ "payouts": payouts, "pagination": pagination });
    }
    // `page_limit` is the caller's ceiling, not a target: an account's
    // recent page is the tail of its history, not every payout it ever
    // took (a daily account would answer with years of them).
    let limit = ctx
        .query
        .get("page_limit")
        .and_then(|limit| limit.parse().ok())
        .unwrap_or(RECENT_PAYOUTS)
        .min(RECENT_PAYOUTS);
    let count = i64::try_from(limit).unwrap_or(0).min(latest);
    let payouts: Vec<Json> = (latest - count + 1..=latest).map(entry).collect();
    json!({ "payouts": payouts, "pagination": { "has_next": false } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pairs: &[(&str, &str)]) -> RequestCtx {
        RequestCtx {
            query: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            t_s: 0.0,
            seed: 0x51ACE,
            host: None,
            cache: std::sync::Arc::new(crate::cache::Cache::new::<Vec<_>>(Vec::new())),
        }
    }

    /// A 7-day window: 2016 five-minute slots and a proper cursor chain.
    #[test]
    fn history_pages_a_week_window_through_the_cursor() {
        let value = series_value(480.0);
        let window = [
            ("from_timestamp", "2026-07-30T12:00:00Z"),
            ("to_timestamp", "2026-08-06T12:00:00Z"),
        ];
        let first = history(&ctx(&window), value, "hashrate_th_per_sec");
        assert_eq!(first["slots"].as_array().expect("slots").len(), 1_000);
        assert_eq!(first["pagination"]["has_next"], true);
        assert_eq!(first["from_timestamp"], "2026-07-30T12:00:00Z");
        assert_eq!(first["slots"][0]["slot_start"], "2026-07-30T12:00:00Z");

        let cursor = first["pagination"]["next_cursor"]
            .as_str()
            .expect("cursor")
            .to_owned();
        let second = history(
            &ctx(&[window[0], window[1], ("page_cursor", &cursor)]),
            value,
            "hashrate_th_per_sec",
        );
        assert_eq!(second["slots"].as_array().expect("slots").len(), 1_000);
        let cursor = second["pagination"]["next_cursor"]
            .as_str()
            .expect("cursor")
            .to_owned();
        let third = history(
            &ctx(&[window[0], window[1], ("page_cursor", &cursor)]),
            value,
            "hashrate_th_per_sec",
        );
        assert_eq!(third["slots"].as_array().expect("slots").len(), 17);
        assert_eq!(third["pagination"]["has_next"], false);
        assert_eq!(third["slots"][16]["slot_start"], "2026-08-06T12:00:00Z");
    }

    /// Slot values are a pure function of slot time — re-requesting a page
    /// (or reaching a slot through a different window) replays it.
    #[test]
    fn history_is_deterministic_across_requests_and_windows() {
        let value = series_value(480.0);
        let wide = history(
            &ctx(&[
                ("from_timestamp", "2026-08-06T00:00:00Z"),
                ("to_timestamp", "2026-08-06T06:00:00Z"),
            ]),
            value,
            "hashrate_th_per_sec",
        );
        let narrow = history(
            &ctx(&[
                ("from_timestamp", "2026-08-06T03:00:00Z"),
                ("to_timestamp", "2026-08-06T06:00:00Z"),
            ]),
            value,
            "hashrate_th_per_sec",
        );
        assert_eq!(wide["slots"][36], narrow["slots"][0]);
    }

    #[test]
    fn windowed_payouts_land_on_the_period_grid_oldest_first() {
        let day = Some(86_400);
        let paid = payouts(
            &ctx(&[
                ("from_timestamp", "2026-07-30T12:00:00Z"),
                ("to_timestamp", "2026-08-06T12:00:00Z"),
            ]),
            day,
            0.000_38,
        );
        let entries = paid["payouts"].as_array().expect("payouts");
        assert_eq!(entries.len(), 7, "a daily payout for each spanned midnight");
        assert_eq!(entries[0]["occurred_at"], "2026-07-31T00:00:00Z");
        assert_eq!(entries[6]["occurred_at"], "2026-08-06T00:00:00Z");
        assert!(entries.iter().all(|e| e["status"] == "COMPLETED"));
        assert_ne!(entries[0]["type"], entries[1]["type"], "kinds alternate");
    }

    #[test]
    fn recent_payouts_honor_the_page_limit_without_a_window() {
        let paid = payouts(&ctx(&[("page_limit", "3")]), Some(86_400), 0.000_38);
        assert_eq!(paid["payouts"].as_array().expect("payouts").len(), 3);
        assert_eq!(paid["pagination"]["has_next"], false);
    }

    /// The widget asks for 1000; a daily account has years of payouts and
    /// the recent page must still answer with a tail, not all of them.
    #[test]
    fn the_recent_page_is_a_tail_however_large_the_asked_limit() {
        let paid = payouts(&ctx(&[("page_limit", "1000")]), Some(86_400), 0.000_38);
        let entries = paid["payouts"].as_array().expect("payouts");
        assert_eq!(entries.len(), RECENT_PAYOUTS);
        let newest = entries.last().expect("a newest entry");
        let at = parse_rfc3339(newest["occurred_at"].as_str().expect("timestamp"))
            .expect("timestamp parses");
        assert!(
            chrono::Utc::now().timestamp() - at < 86_400,
            "the tail ends at the most recent payout"
        );
    }

    #[test]
    fn a_never_paying_account_answers_empty_not_missing() {
        assert_eq!(payouts(&ctx(&[]), None, 0.0)["payouts"], json!([]));
        assert_eq!(financials(None)["financial_accounts"], json!([]));
    }

    #[test]
    fn financials_estimate_the_next_boundary_with_progress_underway() {
        let paid = financials(Some(86_400));
        let account = &paid["financial_accounts"][0];
        let progress = account["next_payout_progress_pct"]
            .as_f64()
            .expect("progress");
        assert!((0.0..100.0).contains(&progress));
        let next = parse_rfc3339(
            account["next_payout_at_estimate"]
                .as_str()
                .expect("estimate"),
        )
        .expect("estimate parses");
        assert!(next > chrono::Utc::now().timestamp());
    }
}
