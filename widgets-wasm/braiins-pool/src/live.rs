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
    PoolData, Series, SizeBucket, Source, chart_frame_secs, quantize_window_end, size_bucket,
    source_needed,
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
    /// the window their request was built with (so follow-up
    /// pages repeat it) and the pages merged so far.
    static HASHRATE_CHAIN: RefCell<Option<PageChain>> = const { RefCell::new(None) };
    static WORKERS_CHAIN: RefCell<Option<PageChain>> = const { RefCell::new(None) };
    static PAYOUTS_CHAIN: RefCell<Option<PageChain>> = const { RefCell::new(None) };
    /// The window each chaining source's outstanding request was built with,
    /// handed to the chain when its first page lands.
    /// Re-deriving it there would straddle the fetch and can land a quantum
    /// later, chaining page 1's cursor onto URLs for a window it never covered.
    static HASHRATE_REQUEST: RefCell<Option<Window>> = const { RefCell::new(None) };
    static WORKERS_REQUEST: RefCell<Option<Window>> = const { RefCell::new(None) };
    static PAYOUTS_REQUEST: RefCell<Option<Window>> = const { RefCell::new(None) };
}

/// A query window as the API takes it: RFC 3339 UTC `from` and `to`.
type Window = (String, String);

/// The sources that page through a listing, and so chain cursors.
const CHAINING: [Source; 3] = [
    Source::HashrateHistory,
    Source::WorkersHistory,
    Source::PayoutsRecent,
];

struct PageChain {
    /// `None` for the Overview payout page, which asks for no window
    /// and so has none to repeat on a follow-up.
    window: Option<Window>,
    /// The follow-up page this chain waits on, cancelled when the chain
    /// goes [`Drop`].
    outstanding: Option<FetchRequestId>,
    series: Series,
    payouts: Vec<crate::model::Payout>,
}

/// Losing the chain cancels the page it waits on, so a reply that outlives
/// its chain settles as `Aborted` and [`continue_chain`] drops it, rather
/// than merging into whatever chain holds the cell by then.
///
/// On the drop, not at each site that replaces a chain: page one refreshes
/// on the poll's own cadence, so a slow listing is still running when the
/// next one starts, and every such site would have to remember.
///
/// The cell is borrowed while this runs — replacing a chain drops the old
/// one in place — so nothing here may touch it. `net::cancel` reaches the
/// host, not the cell.
impl Drop for PageChain {
    fn drop(&mut self) {
        if let Some(id) = self.outstanding {
            let _ = net::cancel(id);
        }
    }
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

/// A page-1 URL for `source` over the current window, recording that window
/// so the chain built from the reply repeats it rather than a later one.
fn windowed_request(source: Source) -> String {
    let (from, to) = window();
    let url = source.windowed_page_url(&from, &to, None);
    record_request(source, Some((from, to)));
    url
}

fn record_request(source: Source, window: Option<Window>) {
    request_cell(source).with(|cell| *cell.borrow_mut() = window);
}

/// The chart frame's query window as RFC 3339 UTC strings.
fn window() -> (String, String) {
    let to = quantize_window_end(SystemTime::now().unix_secs);
    let from = to - chart_frame_secs(Params::current().chart_frame);
    (
        format::strftime(from, RFC3339_UTC),
        format::strftime(to, RFC3339_UTC),
    )
}

fn build(handle: PollHandle) -> Option<FetchSpec> {
    if !pool_bound() {
        return None;
    }
    let source = source_of(handle);
    let url = match source {
        Source::HashrateHistory | Source::WorkersHistory => windowed_request(source),
        Source::PayoutsRecent => match Params::current().style {
            Style::Overview => {
                record_request(source, None);
                source.recent_page_url()
            }
            Style::BigChart => windowed_request(source),
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
        log_debug!(
            "{} fetch failed with status {}",
            source.name(),
            response.status
        );
        let changed = DATA.with(|data| {
            let mut data = data.borrow_mut();
            let mut changed = data.mark_failed(source);
            // The API knows the key and still refuses its reads — every source fails
            // alike, so the whole screen says so rather than each slot on its own.
            if let Some(FetchOutcome::Http(401 | 403)) = response.outcome()
                && !data.access_denied
            {
                data.access_denied = true;
                changed = true;
            }
            changed
        });
        // An outage repeats this reply every tick; only the first one is news.
        if changed {
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
                } else {
                    unreadable(&mut data, source);
                }
            }
            Source::RewardsLatest => {
                if let Some(rewards) = pool_api::parse_rewards(&json) {
                    data.rewards = units::availability::Availability::Available(rewards);
                } else {
                    unreadable(&mut data, source);
                }
            }
            Source::WorkersCurrent => {
                if let Some(counts) = pool_api::parse_workers_current(&json) {
                    data.workers = units::availability::Availability::Available(counts);
                } else {
                    unreadable(&mut data, source);
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

/// A reply that arrived whole but says nothing the parser can use: the source
/// answered, so its slots stop waiting.
fn unreadable(data: &mut crate::model::PoolData, source: Source) {
    log_warn!("{} reply did not parse", source.name());
    data.mark_failed(source);
}

fn start_chain(source: Source, json: &json::JsonDoc, date: pool_api::ParseDate<'_>) {
    // Taken, not read: the window belongs to the request that just replied,
    // and the next request records its own.
    let window = request_cell(source).with(|cell| cell.borrow_mut().take());
    let chain = PageChain {
        window,
        outstanding: None,
        series: Series::default(),
        payouts: Vec::new(),
    };
    // Replacing the chain drops the one it displaces, cancelling the page
    // that chain waited on [`PageChain::drop`].
    chain_cell(source).with(|cell| *cell.borrow_mut() = Some(chain));
    absorb_page(source, json, date, None);
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

fn request_cell(source: Source) -> &'static std::thread::LocalKey<RefCell<Option<Window>>> {
    match source {
        Source::HashrateHistory => &HASHRATE_REQUEST,
        Source::WorkersHistory => &WORKERS_REQUEST,
        Source::PayoutsRecent => &PAYOUTS_REQUEST,
        Source::HashrateCurrent
        | Source::RewardsLatest
        | Source::WorkersCurrent
        | Source::Financials => unreachable!("BUG: only windowed sources chain pages"),
    }
}

/// What absorbing a page leaves the chain in.
enum PageOutcome {
    /// Another page is on its way; nothing to commit yet.
    Following,
    /// The listing is whole and ready for [`DATA`].
    Complete,
    /// The listing cannot be completed, so it is dropped
    /// and whatever [`DATA`] already holds stands.
    Abandoned,
    /// A page the chain holding the cell is not waiting for. Whichever chain
    /// asked for it is gone, so the page is dropped — and the source is not
    /// failing, since the chain that replaced it is still working.
    Ignored,
}

/// Merge a history page, reporting whether it could be read at all.
fn merge_history(
    chain: &mut PageChain,
    json: &json::JsonDoc,
    value_field: &str,
    date: pool_api::ParseDate<'_>,
) -> bool {
    match pool_api::parse_history_page(json, value_field, date) {
        Some(page) => {
            chain.series.merge(page);
            true
        }
        None => false,
    }
}

/// Merge one page into the source's chain; follow the cursor
/// when the reply names a next page, otherwise commit
/// the merged result into [`DATA`].
///
/// `from` names the request this page answers — `None` for a listing's first
/// page, which its poll owns rather than the chain.
fn absorb_page(
    source: Source,
    json: &json::JsonDoc,
    date: pool_api::ParseDate<'_>,
    from: Option<FetchRequestId>,
) {
    let next = pool_api::next_page(json);
    let outcome = chain_cell(source).with(|cell| {
        let mut cell = cell.borrow_mut();
        let Some(chain) = cell.as_mut() else {
            return PageOutcome::Ignored;
        };
        // The page a chain absorbs is the one it asked for and no other:
        // a first page into a chain awaiting nothing, a follow-up into the
        // chain that named it.
        if chain.outstanding != from {
            return PageOutcome::Ignored;
        }
        chain.outstanding = None;
        let parsed = match source {
            Source::HashrateHistory => merge_history(chain, json, "hashrate_th_per_sec", date),
            Source::WorkersHistory => merge_history(chain, json, "active_workers", date),
            Source::PayoutsRecent => match pool_api::parse_payouts_page(json, date) {
                Some(page) => {
                    chain.payouts.extend(page);
                    true
                }
                None => false,
            },
            Source::HashrateCurrent
            | Source::RewardsLatest
            | Source::WorkersCurrent
            | Source::Financials => unreachable!("BUG: only windowed sources chain pages"),
        };
        // Both leave the listing incomplete. Committing the pages gathered so far
        // would pass a hole off as the whole picture, so the chain is dropped
        // and last cycle's data stands.
        if !parsed {
            log_warn!(
                "{} page did not parse; keeping the last good data",
                source.name()
            );
            *cell = None;
            return PageOutcome::Abandoned;
        }
        if next == pool_api::NextPage::Malformed {
            log_warn!(
                "{} claims another page without naming it; keeping the last good data",
                source.name()
            );
            *cell = None;
            return PageOutcome::Abandoned;
        }
        // A follow-up repeats the window page 1 asked for.
        // The Overview payout query asked for none — it wants one latest page,
        // and its cursor would lead into a different query — so it never follows.
        let (pool_api::NextPage::Cursor(cursor), Some((from, to))) = (&next, &chain.window) else {
            return PageOutcome::Complete;
        };
        let url = source.windowed_page_url(from, to, Some(cursor));
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
            return PageOutcome::Abandoned;
        }
        chain.outstanding = started;
        PageOutcome::Following
    });
    match outcome {
        PageOutcome::Complete => {
            commit_chain(source);
            request_frame();
        }
        // Last cycle's data stands where there is any; a source still on its
        // first listing has none, and says so instead of loading forever.
        PageOutcome::Abandoned => {
            if DATA.with(|data| data.borrow_mut().mark_failed(source)) {
                request_frame();
            }
        }
        PageOutcome::Following | PageOutcome::Ignored => {}
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

/// Drop each source's chain, which cancels the page it waits on.
///
/// Invalidating a poll does not reach these hand-rolled follow-ups.
/// Without this, a page fetched under the old account or window merges into
/// the chain started under the new one.
fn abandon_chains(sources: &[Source]) {
    for &source in sources {
        chain_cell(source).with(|cell| cell.borrow_mut().take());
    }
}

/// Whether the source's open chain is waiting for exactly this page.
fn chain_awaits(source: Source, request: Option<FetchRequestId>) -> bool {
    chain_cell(source).with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|chain| chain.outstanding == request)
    })
}

fn continue_chain(source: Source, response: &FetchResponse) {
    if response.outcome() == Some(FetchOutcome::Aborted) {
        // Whoever cancelled this page already dropped the chain it belonged to.
        // Anything in the cell now was started afterwards, so leave it be.
        return;
    }
    let from = Some(response.request_id);
    if !chain_awaits(source, from) {
        // Neither this page nor its failure is the open chain's business:
        // the chain that asked for it is gone.
        return;
    }
    if !response.ok() {
        // A broken chain keeps last cycle's committed data;
        // the next poll tick starts a fresh chain.
        chain_cell(source).with(|cell| {
            if let Some(mut chain) = cell.borrow_mut().take() {
                // This failure is the awaited settlement; the drop has nothing to cancel.
                chain.outstanding = None;
            }
        });
        if DATA.with(|data| data.borrow_mut().mark_failed(source)) {
            request_frame();
        }
        return;
    }
    let json = response.json();
    absorb_page(source, &json, &|s| parse_date(s), from);
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
    // A param outdates whatever its requests were built from.
    // The frame sets every window; the style only decides whether the payout
    // query carries one, the histories asking the same URL under either.
    let outdated: &[Source] = if changed.contains(&"chart_frame") {
        &CHAINING
    } else if changed.contains(&"style") {
        &[Source::PayoutsRecent]
    } else {
        &[]
    };
    if !outdated.is_empty() {
        abandon_chains(outdated);
        HANDLES.with(|handles| {
            if let Some(handles) = handles.borrow().as_ref() {
                for handle in handles {
                    if outdated.contains(&source_of(*handle)) {
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
    abandon_chains(&CHAINING);
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
