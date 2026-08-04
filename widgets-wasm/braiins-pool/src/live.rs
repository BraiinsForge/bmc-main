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

//! The wasm-side runtime: polls the pool API per the (style × size)
//! gating matrix, chains cursor pages, and assembles live state into
//! the same view data the storybook renders from fixtures.

use std::cell::RefCell;
use std::time::Duration;

#[expect(
    clippy::wildcard_imports,
    reason = "runtime code uses many SDK builders, macros, and host shims"
)]
use bmc_wasm_sdk::*;

use crate::manifest_params::credentials as slots;
use crate::manifest_params::{ChartFrame, Params, Style};
use crate::model::{
    PoolData, Series, SizeBucket, Source, chart_frame_secs, size_bucket, source_needed,
};
use crate::pool_api::{self, RFC3339_UTC};
use crate::screens::big_chart::{BigChartViewData, big_chart_view};
use crate::screens::overview::{OverviewViewData, overview_view};

const REFRESH_MS: u32 = 60_000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// X-axis label count on the Big Chart, matching the design's five marks.
const X_LABEL_COUNT: usize = 5;

thread_local! {
    static DATA: RefCell<PoolData> = RefCell::new(PoolData::default());
    static HANDLES: RefCell<Option<[PollHandle; Source::ALL.len()]>> =
        const { RefCell::new(None) };
    /// In-flight cursor chains for the windowed sources:
    /// the query window (so follow-up pages repeat it)
    /// and the pages merged so far.
    static HASHRATE_CHAIN: RefCell<Option<PageChain>> = const { RefCell::new(None) };
    static WORKERS_CHAIN: RefCell<Option<PageChain>> = const { RefCell::new(None) };
    static PAYOUTS_CHAIN: RefCell<Option<PageChain>> = const { RefCell::new(None) };
}

struct PageChain {
    from: String,
    to: String,
    series: Series,
    payouts: Vec<crate::model::Payout>,
}

fn source_of(handle: PollHandle) -> Source {
    Source::ALL[handle.index()]
}

fn auth_header() -> String {
    fmt!("X-API-Key: {}", slots::pool::TOKEN)
}

/// Whether the pool slot is bound; unbound keeps every poll
/// dormant and the screens render their placeholder state.
fn pool_bound() -> bool {
    credentials::current().is_bound("pool")
}

/// The chart frame's query window as RFC 3339 UTC strings.
fn window() -> (String, String) {
    let now = SystemTime::now().unix_secs;
    let from = now - chart_frame_secs(Params::current().chart_frame);
    (
        format::strftime(from, RFC3339_UTC),
        format::strftime(now, RFC3339_UTC),
    )
}

fn build(handle: PollHandle) -> Option<FetchSpec> {
    if !pool_bound() {
        return None;
    }
    let source = source_of(handle);
    let url = match source {
        Source::HashrateHistory | Source::WorkersHistory => {
            let (from, to) = window();
            source.windowed_page_url(&from, &to, None)
        }
        Source::PayoutsRecent => match Params::current().style {
            Style::Overview => source.recent_page_url(),
            Style::BigChart => {
                let (from, to) = window();
                source.windowed_page_url(&from, &to, None)
            }
        },
        Source::HashrateCurrent
        | Source::RewardsLatest
        | Source::WorkersCurrent
        | Source::Financials => source.url(),
    };
    Some(
        FetchSpec::get(url)
            .headers(auth_header())
            .timeout(FETCH_TIMEOUT),
    )
}

fn on_reply(handle: PollHandle, response: &FetchResponse) {
    let source = source_of(handle);
    if !response.ok() {
        log_warn!(
            "{} fetch failed with status {}",
            source.name(),
            response.status
        );
        // The API knows the key and still refuses its reads — every source fails alike,
        // and the screens must say so rather than keep their loading skeletons up forever.
        if let Some(FetchOutcome::Http(401 | 403)) = response.outcome() {
            DATA.with(|data| data.borrow_mut().access_denied = true);
            request_frame();
        }
        return;
    }
    // Parsing a page costs fuel per field read, and a reply gets one frame's
    // worth: a span per source shows which page approaches the budget before
    // it trips it.
    let _reply = profile::span(source.name());
    let json = response.json();
    let to_unix = |s: &str| parse_date(s);
    DATA.with(|data| {
        let mut data = data.borrow_mut();
        data.access_denied = false;
        match source {
            Source::HashrateCurrent => {
                if let Some(th) = pool_api::parse_hashrate_current(&json) {
                    data.hashrate_5m = units::availability::Availability::Available(
                        Hashrate::from_terahashes_per_second(th),
                    );
                }
            }
            Source::RewardsLatest => {
                if let Some(rewards) = pool_api::parse_rewards(&json) {
                    data.rewards = units::availability::Availability::Available(rewards);
                }
            }
            Source::WorkersCurrent => {
                if let Some(counts) = pool_api::parse_workers_current(&json) {
                    data.workers = units::availability::Availability::Available(counts);
                }
            }
            Source::Financials => {
                data.next_payout = units::availability::Availability::Available(
                    pool_api::parse_financials(&json, &to_unix),
                );
            }
            Source::HashrateHistory | Source::WorkersHistory | Source::PayoutsRecent => {
                drop(data);
                start_chain(source, &json, &to_unix);
            }
        }
    });
    request_frame();
}

fn start_chain(source: Source, json: &json::JsonDoc, date: pool_api::ParseDate<'_>) {
    let (from, to) = window();
    let chain = PageChain {
        from,
        to,
        series: Series::default(),
        payouts: Vec::new(),
    };
    chain_cell(source).with(|cell| *cell.borrow_mut() = Some(chain));
    absorb_page(source, json, date);
}

fn chain_cell(source: Source) -> &'static std::thread::LocalKey<RefCell<Option<PageChain>>> {
    match source {
        Source::HashrateHistory => &HASHRATE_CHAIN,
        Source::WorkersHistory => &WORKERS_CHAIN,
        Source::PayoutsRecent => &PAYOUTS_CHAIN,
        Source::HashrateCurrent
        | Source::RewardsLatest
        | Source::WorkersCurrent
        | Source::Financials => unreachable!("BUG: only windowed sources chain pages"),
    }
}

/// Merge one page into the source's chain; follow the cursor
/// when the reply names a next page, otherwise commit
/// the merged result into [`DATA`].
fn absorb_page(source: Source, json: &json::JsonDoc, date: pool_api::ParseDate<'_>) {
    // The Overview's payout query is one unwindowed latest page; following
    // its cursor would continue with windowed page URLs, a different query.
    let follows_cursor =
        !(source == Source::PayoutsRecent && Params::current().style == Style::Overview);
    let next = pool_api::next_cursor(json).filter(|_| follows_cursor);
    let done = next.is_none();
    chain_cell(source).with(|cell| {
        let mut cell = cell.borrow_mut();
        let Some(chain) = cell.as_mut() else { return };
        match source {
            Source::HashrateHistory => {
                if let Some(page) = pool_api::parse_history_page(json, "hashrate_th_per_sec", date)
                {
                    chain.series.merge(page);
                }
            }
            Source::WorkersHistory => {
                if let Some(page) = pool_api::parse_history_page(json, "active_workers", date) {
                    chain.series.merge(page);
                }
            }
            Source::PayoutsRecent => {
                chain
                    .payouts
                    .extend(pool_api::parse_payouts_page(json, date));
            }
            Source::HashrateCurrent
            | Source::RewardsLatest
            | Source::WorkersCurrent
            | Source::Financials => unreachable!("BUG: only windowed sources chain pages"),
        }
        if let Some(cursor) = &next {
            let url = source.windowed_page_url(&chain.from, &chain.to, Some(cursor));
            let callback = match source {
                Source::HashrateHistory => continue_hashrate,
                Source::WorkersHistory => continue_workers,
                Source::PayoutsRecent => continue_payouts,
                Source::HashrateCurrent
                | Source::RewardsLatest
                | Source::WorkersCurrent
                | Source::Financials => unreachable!("BUG: only windowed sources chain pages"),
            };
            let started = net::FetchRequest::get(&url)
                .headers(&auth_header())
                .timeout(FETCH_TIMEOUT)
                .send(callback);
            if started.is_none() {
                // The follow-up page never launched; drop the chain and keep
                // last cycle's committed data — the next tick starts fresh.
                *cell = None;
            }
        }
    });
    if done {
        commit_chain(source);
        request_frame();
    }
}

fn commit_chain(source: Source) {
    chain_cell(source).with(|cell| {
        let Some(mut chain) = cell.borrow_mut().take() else {
            return;
        };
        DATA.with(|data| {
            let mut data = data.borrow_mut();
            match source {
                Source::HashrateHistory => {
                    data.hashrate_history = units::availability::Availability::Available(
                        std::mem::take(&mut chain.series),
                    );
                }
                Source::WorkersHistory => {
                    data.workers_history = units::availability::Availability::Available(
                        std::mem::take(&mut chain.series),
                    );
                }
                Source::PayoutsRecent => {
                    chain.payouts.sort_by_key(|payout| payout.at);
                    data.payouts = units::availability::Availability::Available(std::mem::take(
                        &mut chain.payouts,
                    ));
                }
                Source::HashrateCurrent
                | Source::RewardsLatest
                | Source::WorkersCurrent
                | Source::Financials => unreachable!("BUG: only windowed sources chain pages"),
            }
        });
    });
}

fn continue_hashrate(response: &FetchResponse) {
    continue_chain(Source::HashrateHistory, response);
}

fn continue_workers(response: &FetchResponse) {
    continue_chain(Source::WorkersHistory, response);
}

fn continue_payouts(response: &FetchResponse) {
    continue_chain(Source::PayoutsRecent, response);
}

fn continue_chain(source: Source, response: &FetchResponse) {
    if !response.ok() {
        // A broken chain keeps last cycle's committed data;
        // the next poll tick starts a fresh chain.
        chain_cell(source).with(|cell| *cell.borrow_mut() = None);
        return;
    }
    let json = response.json();
    absorb_page(source, &json, &|s| parse_date(s));
}

/// Enable exactly the polls the current (style × size × toggle) needs.
fn reconcile() {
    let params = Params::current();
    let size = widget_size();
    let bucket = size_bucket(size.width, size.height);
    let bound = pool_bound();
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            for handle in handles {
                let needed = source_needed(
                    source_of(*handle),
                    params.style,
                    bucket,
                    params.worker_states,
                );
                handle.set_enabled(bound && needed);
            }
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let handles = std::array::from_fn(|_| {
        register_poll(
            build,
            on_reply,
            PollConfig {
                interval_ms: Some(REFRESH_MS),
                enabled: false,
                ..Default::default()
            },
        )
    });
    HANDLES.with(|cell| *cell.borrow_mut() = Some(handles));
    reconcile();
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let changed = Params::previous()
        .map(|previous| Params::current().changed_keys(&previous))
        .unwrap_or_default();
    reconcile();
    if changed.contains(&"chart_frame") {
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                for handle in handles {
                    if matches!(
                        source_of(*handle),
                        Source::HashrateHistory | Source::WorkersHistory | Source::PayoutsRecent
                    ) {
                        handle.invalidate();
                    }
                }
            }
        });
    }
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn on_credentials_update() {
    // A rebind may point at a different account: blank and refetch everything.
    DATA.with(|data| *data.borrow_mut() = PoolData::default());
    reconcile();
    HANDLES.with(|handles| {
        if let Some(handles) = handles.borrow().as_ref() {
            for handle in handles {
                handle.invalidate();
            }
        }
    });
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let size = widget_size();
    let bucket = size_bucket(size.width, size.height);
    #[expect(
        clippy::cast_precision_loss,
        reason = "viewport pixel counts are far below f32's exact integer range"
    )]
    let (width, height) = (size.width as f32, size.height as f32);
    let params = Params::current();
    // Names the configuration in the fuel report, where page and clone costs
    // read very differently over a 4-hour frame than a 7-day one. Wraps the
    // whole render, so the fuel it carries is that frame's total.
    let _variant = profile::span(variant_label(params.style, params.chart_frame));
    let account = credentials::current()
        .get("pool")
        .map(|bound| bound.account_name.clone());
    let bind_hint = if account.is_none() {
        let net = network::info();
        crate::screens::parts::BindHint {
            url: if net.ip.is_empty() {
                String::new()
            } else {
                fmt!("http://{}", net.ip)
            },
            ssid: net.ssid,
        }
    } else {
        crate::screens::parts::BindHint::default()
    };
    // Copies every series the sources delivered, so a deep chart frame pays
    // per frame for samples its layout may not even draw.
    let data = {
        let _snapshot = profile::span("snapshot");
        DATA.with(|data| data.borrow().clone())
    };

    // The span closes with this block, so `submit` below measures the
    // serialization alone rather than nesting inside the tree's build.
    let root = {
        let _tree = profile::span("tree");
        match params.style {
            Style::Overview => overview_view(&OverviewViewData {
                bucket,
                width,
                height,
                account,
                bind_hint,
                worker_states: params.worker_states,
                data,
            }),
            Style::BigChart => {
                // Only the Fullscreen layout draws the time band.
                let x_labels = if bucket == SizeBucket::Full {
                    x_labels(&data)
                } else {
                    Vec::new()
                };
                big_chart_view(&BigChartViewData {
                    bucket,
                    width,
                    height,
                    account,
                    bind_hint,
                    worker_states: params.worker_states,
                    data,
                    x_labels,
                })
            }
        }
    };
    let _submit = profile::span("submit");
    let _ = render_ui(size.width, size.height, root);
}

/// The profiling label for a (style × chart frame) pair. Spelled out per pair
/// because a span's name must be `'static`, which rules out composing one.
fn variant_label(style: Style, frame: ChartFrame) -> &'static str {
    match (style, frame) {
        (Style::Overview, ChartFrame::Hours4) => "overview@4h",
        (Style::Overview, ChartFrame::Hours12) => "overview@12h",
        (Style::Overview, ChartFrame::Hours24) => "overview@24h",
        (Style::Overview, ChartFrame::Days7) => "overview@7d",
        (Style::BigChart, ChartFrame::Hours4) => "big-chart@4h",
        (Style::BigChart, ChartFrame::Hours12) => "big-chart@12h",
        (Style::BigChart, ChartFrame::Hours24) => "big-chart@24h",
        (Style::BigChart, ChartFrame::Days7) => "big-chart@7d",
    }
}

/// Time labels for the Big Chart's window, honoring the system preferences:
/// clock times (12/24 h, system timezone) within a day-long frame,
/// day-month dates in the system date format's order and separator beyond.
fn x_labels(data: &PoolData) -> Vec<(f32, String)> {
    let Some(series) = data.hashrate_history.as_option() else {
        return Vec::new();
    };
    let (Some(from), Some(to)) = (series.from, series.to) else {
        return Vec::new();
    };
    let day_pattern = (to - from > 24 * 3_600)
        .then(|| day_month_pattern(system::current().date_format().unwrap_or_default()));
    crate::chart::x_axis_marks(from, to, X_LABEL_COUNT)
        .into_iter()
        .map(|(at, fraction)| {
            let label = match day_pattern {
                Some(pattern) => format::strftime(at, pattern),
                None => format::format_time(
                    SystemTime { unix_secs: at },
                    format::FormatTimeOpts::default(),
                ),
            };
            (fraction, label)
        })
        .collect()
}

/// The day-month slice of the system date format: its component order
/// and separator, without the year the design's tick labels have no room for.
fn day_month_pattern(format: system::DateFormat) -> &'static str {
    use system::DateFormat as F;
    match format {
        F::DdMmYyyyDot => "%d.%m",
        F::DdMmYyyyDash => "%d-%m",
        F::DMYyyySlash => "%-d/%-m",
        F::DdMmYyyySlash => "%d/%m",
        F::MDYyyySlash | F::YyyyMDSlash => "%-m/%-d",
        F::YyyyMmDdDot => "%m.%d",
        F::YyyyMmDdDash => "%m-%d",
    }
}
