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

// Native builds (the gallery) can't reach the wasm-exported logic; suppress
// the false dead_code there only — wasm and tests still catch the real thing.
#![cfg_attr(
    not(any(target_arch = "wasm32", test)),
    expect(dead_code, reason = "reachable only via wasm exports and tests")
)]

mod adapter;
mod device;
mod discovery;
mod families;
mod filter;
mod history;
mod model;
mod naming;
mod paging;
pub mod screens;
mod session;
mod summary;
mod telemetry;
pub mod view;
mod view_data;

#[cfg(test)]
mod contract;

mod manifest_params;

#[cfg(target_arch = "wasm32")]
use bmc_wasm_sdk::*;

#[cfg(target_arch = "wasm32")]
use std::cell::{Cell, RefCell};

#[cfg(target_arch = "wasm32")]
use adapter::FamilyAdapter;
#[cfg(target_arch = "wasm32")]
use bmc_wasm_sdk::timer::{self, Timer};
#[cfg(target_arch = "wasm32")]
use device::{DeviceFamily, DeviceId, DeviceList, families_on_new_credentials, family_label};
#[cfg(target_arch = "wasm32")]
use families::bitaxe::BitaxeAdapter;
#[cfg(target_arch = "wasm32")]
use families::bos::BosAdapter;
#[cfg(target_arch = "wasm32")]
use families::ubos::UbosAdapter;
#[cfg(target_arch = "wasm32")]
use screens::dashboard::DashboardViewData;
#[cfg(target_arch = "wasm32")]
use screens::table::TableViewData;

#[cfg(target_arch = "wasm32")]
thread_local! {
    pub(crate) static DEVICES: RefCell<DeviceList> = RefCell::new(DeviceList::new());
    static DERIVED: RefCell<Option<DerivedView>> = const { RefCell::new(None) };
    static VIEW: RefCell<view::ViewState> = const { RefCell::new(view::ViewState::new()) };
    static HISTORY: RefCell<history::HashrateHistory> =
        RefCell::new(history::HashrateHistory::default());
    // Keep both browses running for the widget's lifetime.
    static HTTP_BROWSE: RefCell<Option<mdns::MdnsBrowse>> = const { RefCell::new(None) };
    static UBOS_BROWSE: RefCell<Option<mdns::MdnsBrowse>> = const { RefCell::new(None) };
    static TICK_TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
    static LIFECYCLE: Cell<Lifecycle> = const { Cell::new(Lifecycle::Dormant) };
}

/// Cadence of the fleet fold (summarize + history + snapshot) while awake.
/// Every poll bumps the devices sequence, so folding per poll
/// would re-fold the ~90-device fleet N times a round.
#[cfg(target_arch = "wasm32")]
const TICK_PERIOD_MS: u32 = 1_000;

/// Cadence of the tick while dormant, where it only samples the fleet charts
/// once per slot of the finest tier.
/// It lands twice in every slot, so one late tick cannot cost the charts a slot.
#[cfg(any(target_arch = "wasm32", test))]
const DORMANT_TICK_PERIOD_MS: u32 = 30_000;

/// Whether the host has the widget awake or dormant.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
enum Lifecycle {
    Awake,
    Dormant,
}

#[cfg(target_arch = "wasm32")]
impl Lifecycle {
    const fn tick_period_ms(self) -> u32 {
        match self {
            Self::Awake => TICK_PERIOD_MS,
            Self::Dormant => DORMANT_TICK_PERIOD_MS,
        }
    }
}

/// The render-ready fleet summary, cached so the filter → group → fold → sort pipeline
/// runs only when the fleet or the params change, not on every render frame.
/// The model-detail rows are cached per selection and cleared on every rebuild.
#[cfg(target_arch = "wasm32")]
struct DerivedView {
    devices_seq: u64,
    params_version: u64,
    summary: summary::FleetSummary,
    filters: filter::Filters,
    fleet_name: String,
    model_detail: Option<ModelDetailCache>,
}

/// Folded device rows for the drilled-into group, keyed by its partition
/// key. Cleared whenever the summary rebuilds so the rows always derive
/// from the same device snapshot.
#[cfg(target_arch = "wasm32")]
struct ModelDetailCache {
    family: Option<DeviceFamily>,
    label: String,
    rows: Vec<(DeviceId, summary::GroupSummary, summary::DeviceStatus)>,
}

/// The base mDNS type BOS and AxeOS both advertise under.
#[cfg(target_arch = "wasm32")]
const HTTP_SERVICE_TYPES: &[&str] = &["_http._tcp"];

/// BOS and AxeOS share `_http._tcp`. Unlike the flaky `_sub` subtype PTRs,
/// the base type resolves reliably despite co-located mDNS responders.
/// Each family is told apart by its TXT keys; any other host on the type
/// is ignored, so it never receives a request, let alone BOS credentials.
#[cfg(target_arch = "wasm32")]
fn on_http_event(_browse: &mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => {
            let doc = JsonDoc::parse(json.as_bytes());
            match discovery::classify_http_found(&doc) {
                Some(DeviceFamily::Bitaxe) => ingest(&BitaxeAdapter, &doc),
                Some(DeviceFamily::Bos) => ingest(&BosAdapter, &doc),
                Some(DeviceFamily::Ubos) => {}
                None => log_debug!(
                    "fleet: ignoring _http._tcp service {} at {}: no BOS or AxeOS TXT",
                    doc.str("/name").unwrap_or_default(),
                    doc.str("/host").unwrap_or_default(),
                ),
            }
        }
        // A base-type removal carries no family, so drop under both ids.
        mdns::MdnsEvent::Removed(name) => {
            on_removed(DeviceFamily::Bos, name);
            on_removed(DeviceFamily::Bitaxe, name);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn on_ubos_event(_browse: &mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => {
            let doc = JsonDoc::parse(json.as_bytes());
            ingest(&UbosAdapter, &doc);
        }
        mdns::MdnsEvent::Removed(name) => on_removed(DeviceFamily::Ubos, name),
    }
}

/// React to an mDNS `ServiceRemoved`;
/// [`DeviceList::on_mdns_removed`] decides whether the device leaves.
/// The family namespaces the id, matching how the device was inserted.
#[cfg(target_arch = "wasm32")]
fn on_removed(family: DeviceFamily, name: &str) {
    let id = DeviceId::for_family(family, name);
    // Removed before reacting so a ring rebuild excludes the gone device
    // instead of re-polling it.
    match DEVICES.with(|d| d.borrow_mut().on_mdns_removed(&id)) {
        None => {}
        Some(device::MdnsRemoval::Kept) => log_info!(
            "fleet: kept {} {} despite mDNS removal",
            family_label(family),
            name
        ),
        Some(device::MdnsRemoval::Removed { model }) => {
            log_info!(
                "fleet: removed {} {} ({})",
                family_label(family),
                name,
                model.map_or_else(|| "unknown model".to_owned(), |m| m.name)
            );
            session::remove_token(&id);
            forget_device_charts(std::slice::from_ref(&id));
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn ingest(adapter: &dyn FamilyAdapter, doc: &JsonDoc) {
    if let Some(found) = adapter.parse_found(doc) {
        let identity = found.identity;
        let model_hint = found.model_hint;
        let family = identity.family;
        let name = identity.name.clone();
        let model = model_hint
            .as_ref()
            .map_or_else(|| "model pending".to_owned(), |m| m.name.clone());
        let is_new = DEVICES.with(|d| {
            let mut devices = d.borrow_mut();
            let is_new = devices.upsert_with_model_hint(identity, model_hint);
            if !session::account_bound(family) {
                devices.mark_account_unusable(family);
            }
            is_new
        });
        if is_new {
            log_info!(
                "fleet: discovered {} {} ({})",
                family_label(family),
                name,
                model
            );
        }
        session::on_discovered(family, is_new);
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    restore_history();
    // Every host loads a widget dormant and wakes it before its first frame.
    set_lifecycle(Lifecycle::Dormant);
    // One base-type browse for BOS + AxeOS (reliable co-located), plus uBOS's own type.
    let http_browse = mdns::mdns_browse(HTTP_SERVICE_TYPES, on_http_event);
    if http_browse.is_none() {
        log_warn!("fleet: HTTP mDNS browse rejected by host runtime limits");
    }
    HTTP_BROWSE.set(http_browse);
    let ubos_browse = mdns::mdns_browse(UbosAdapter.browse_service_types(), on_ubos_event);
    if ubos_browse.is_none() {
        log_warn!("fleet: Forge Firmware mDNS browse rejected by host runtime limits");
    }
    UBOS_BROWSE.set(ubos_browse);
    request_frame();
}

/// Restore the persisted hashrate charts (fleet-total + per-model) so a restart
/// doesn't blank them. A corrupt entry — unparseable blob or out-of-range timestamp —
/// is evicted and the charts refill live, never surfacing as data.
#[cfg(target_arch = "wasm32")]
#[expect(clippy::integer_division, reason = "cache saved_at is epoch ms → s")]
fn restore_history() {
    let Some(entry) = cache::read_bytes(history::CACHE_TAG) else {
        return;
    };
    let Ok(saved_at) = i64::try_from(entry.saved_at / 1000) else {
        cache::evict(history::CACHE_TAG);
        return;
    };
    let now = SystemTime::now().unix_secs;
    let restored = HISTORY.with(|h| h.borrow_mut().restore(&entry.bytes, saved_at, now));
    if !restored {
        cache::evict(history::CACHE_TAG);
    }
}

/// Fold the fleet into a fresh view and feed the total and per-model charts.
#[cfg(target_arch = "wasm32")]
fn fold() -> DerivedView {
    let devices_seq = DEVICES.with(|d| d.borrow().seq());
    let params_version = bmc_wasm_sdk::params::version();
    let params = manifest_params::Params::current();
    let filters = filter::Filters::from(&params);
    let summary = {
        let _s = profile::span("summarize");
        DEVICES.with(|d| summary::summarize(&d.borrow(), &filters))
    };
    record_fleet_history(SystemTime::now().unix_secs, &summary.hashrate());
    DerivedView {
        devices_seq,
        params_version,
        summary,
        filters,
        fleet_name: params.fleet_name,
        model_detail: None,
    }
}

/// Feed the total and per-model charts,
/// persisting them when a snapshot is due.
#[cfg(target_arch = "wasm32")]
fn record_fleet_history(now: i64, fleet: &summary::FleetHashrate<'_>) {
    let _s = profile::span("history");
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        h.record_fleet(now, fleet);
        if let Some(blob) = h.take_snapshot(now) {
            cache::put(history::CACHE_TAG, &[], &blob);
        }
    });
}

/// Sample the total and per-model charts in a slot no fold has sampled,
/// from the hashrate alone rather than the full fold.
#[cfg(target_arch = "wasm32")]
fn sample_fleet_if_due() {
    let now = SystemTime::now().unix_secs;
    if !HISTORY.with(|h| h.borrow().fleet_sample_due(now)) {
        return;
    }
    let filters = filter::Filters::from(&manifest_params::Params::current());
    DEVICES.with(|d| {
        let devices = d.borrow();
        record_fleet_history(now, &summary::fleet_hashrate(&devices, &filters));
    });
}

/// Record a device's chart point once its poll result has landed in [`DEVICES`].
#[cfg(target_arch = "wasm32")]
pub(crate) fn record_device_hashrate(id: &DeviceId) {
    let now = SystemTime::now().unix_secs;
    DEVICES.with(|d| {
        HISTORY.with(|h| record_polled_hashrate(&mut h.borrow_mut(), &d.borrow(), now, id));
    });
}

/// Record the hashrate `devices` holds for `id` at `now`.
/// A poll that lands after its device left the fleet records nothing.
#[cfg(any(target_arch = "wasm32", test))]
fn record_polled_hashrate(
    history: &mut history::HashrateHistory,
    devices: &device::DeviceList,
    now: i64,
    id: &device::DeviceId,
) {
    if let Some(dev) = devices.get(id) {
        history.record_device(now, id, dev.hashrate());
    }
}

/// Drop the device charts of `ids`, which just left the fleet.
#[cfg(target_arch = "wasm32")]
pub(crate) fn forget_device_charts(ids: &[DeviceId]) {
    HISTORY.with(|h| h.borrow_mut().forget_devices(ids));
}

/// Note the host's `lifecycle` and run the tick at its period.
#[cfg(target_arch = "wasm32")]
fn set_lifecycle(lifecycle: Lifecycle) {
    LIFECYCLE.set(lifecycle);
    set_tick_period(lifecycle.tick_period_ms());
}

/// Run the tick every `period_ms`,
/// starting its timer if the host has not granted one yet.
#[cfg(target_arch = "wasm32")]
fn set_tick_period(period_ms: u32) {
    TICK_TIMER.with_borrow_mut(|tick| {
        if let Some(running) = tick {
            if !running.set_period(period_ms) {
                log_error!(
                    "fleet: host refused a {} ms tick period; the tick keeps its previous one",
                    period_ms
                );
            }
        } else {
            *tick = timer::start(period_ms, on_tick);
            if tick.is_none() {
                log_error!("fleet: tick timer refused by the host; the fleet view stops updating");
            }
        }
    });
}

/// History buckets by wall-clock slot, so a late tick needs no catch-up.
/// Awake, every tick redraws, so the chart's time axis keeps moving
/// while the fleet holds still, as it does once the poller parks,
/// and the charts still get a sample in every slot no fold reached.
#[cfg(target_arch = "wasm32")]
fn on_tick(_timer: &Timer, _elapsed_periods: u32) {
    match LIFECYCLE.get() {
        Lifecycle::Awake => {
            fold_if_stale();
            sample_fleet_if_due();
            request_frame();
        }
        Lifecycle::Dormant => sample_fleet_if_due(),
    }
}

/// Refold when the fleet or the params moved since the last fold,
/// and ask for the frame that shows it.
#[cfg(target_arch = "wasm32")]
fn fold_if_stale() {
    let devices_seq = DEVICES.with(|d| d.borrow().seq());
    let params_version = bmc_wasm_sdk::params::version();
    let stale = DERIVED.with(|cell| {
        let mut cell = cell.borrow_mut();
        let stale = cell
            .as_ref()
            .is_none_or(|d| d.devices_seq != devices_seq || d.params_version != params_version);
        if stale {
            *cell = Some(fold());
        }
        stale
    });
    if stale {
        request_frame();
    }
}

/// Fold the per-device rows for the drilled-into group, under the same
/// filters the cached summary was built with.
#[cfg(target_arch = "wasm32")]
fn rebuild_model_detail_rows(
    filters: &filter::Filters,
    sel: &view::ModelDetailSelection,
) -> Vec<(DeviceId, summary::GroupSummary, summary::DeviceStatus)> {
    DEVICES.with(|d| {
        summary::model_detail_rows(&d.borrow(), filters, sel.family, &sel.label, |dev| {
            naming::display_name(&dev.identity.name).to_owned()
        })
    })
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
#[expect(
    clippy::too_many_lines,
    reason = "the render entry orchestrates derive, view dispatch, and click routing"
)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetViewport { width, height, .. } = widget_viewport();
    let layout = screens::Layout::of(width, height);
    let params_version = bmc_wasm_sdk::params::version();
    DERIVED.with(|cell| {
        let mut cell = cell.borrow_mut();
        // Fleet changes arrive through the tick.
        // A params edit folds here, so the frame it asked for already shows it.
        if cell
            .as_ref()
            .is_none_or(|d| d.params_version != params_version)
        {
            *cell = Some(fold());
        }
        let derived = cell.as_mut().expect("BUG: derived view populated above");
        let chart_span =
            history::ChartSpan::from(manifest_params::Params::current().chart_span_minutes);

        // The chart's time axis: render now back two slots short of one range,
        // so points place by time with the newest at the right edge.
        let chart_window = history::ChartWindow {
            end: SystemTime::now().unix_secs,
            span_secs: chart_span.window_secs(),
        };
        VIEW.with(|view_state| {
            let mut nav = view_state.borrow_mut();
            let selected = nav
                .model_detail
                .as_ref()
                .and_then(|sel| view::selected_index(&derived.summary.groups, sel));
            // A selection whose group vanished (filtered out, devices gone)
            // falls back to the fleet table.
            if nav.model_detail.is_some() && selected.is_none() {
                nav.model_detail = None;
                derived.model_detail = None;
            }
            if let Some(sel) = nav.model_detail.as_ref() {
                let fresh = derived
                    .model_detail
                    .as_ref()
                    .is_some_and(|c| c.family == sel.family && c.label == sel.label);
                if !fresh {
                    let rows = rebuild_model_detail_rows(&derived.filters, sel);
                    derived.model_detail = Some(ModelDetailCache {
                        family: sel.family,
                        label: sel.label.clone(),
                        rows,
                    });
                }
            }
            let (root, page_count) = if let Some(sel) = nav.model_detail.as_ref() {
                // A live selection (its group survived the fallback above); the
                // folded device rows are cached in `derived.model_detail`.
                let rows = &derived
                    .model_detail
                    .as_ref()
                    .expect("BUG: model-detail cache rebuilt above")
                    .rows;
                if let Some(device_id) = sel.device.as_deref() {
                    // The drilled-into device's detail, from its fold-of-one row and its raw reading.
                    // A device gone between tap and render falls back to the model breakdown.
                    let detail = rows
                        .iter()
                        .find(|(id, _, _)| id.as_str() == device_id)
                        .and_then(|(id, group, _)| {
                            HISTORY.with(|h| {
                                let h = h.borrow();
                                let history = h.view(chart_span, chart_window);
                                DEVICES.with(|devs| {
                                    devs.borrow().get(id).map(|dev| {
                                        screens::device_detail::DeviceDetailData::from_device(
                                            &derived.fleet_name,
                                            &sel.label,
                                            group,
                                            dev,
                                            &history,
                                        )
                                    })
                                })
                            })
                        });
                    if let Some(data) = detail {
                        (layout.device_detail(&data), 1)
                    } else {
                        let data = HISTORY.with(|h| {
                            screens::model_detail::ModelDetailViewData::from_summary(
                                &derived.fleet_name,
                                &sel.label,
                                rows,
                                sel.page,
                                &h.borrow().view(chart_span, chart_window),
                            )
                        });
                        let page_count = data.page_count;
                        (layout.model_detail(&data), page_count)
                    }
                } else {
                    let data = HISTORY.with(|h| {
                        screens::model_detail::ModelDetailViewData::from_summary(
                            &derived.fleet_name,
                            &sel.label,
                            rows,
                            sel.page,
                            &h.borrow().view(chart_span, chart_window),
                        )
                    });
                    let page_count = data.page_count;
                    (layout.model_detail(&data), page_count)
                }
            } else if derived.summary.groups.is_empty() {
                (layout.searching(), 1)
            } else {
                match nav.mode {
                    view::ViewMode::Grid => {
                        let data = HISTORY.with(|h| {
                            DashboardViewData::from_summary(
                                &derived.summary,
                                &derived.fleet_name,
                                &h.borrow().view(chart_span, chart_window),
                            )
                        });
                        (layout.dashboard(&data), 1)
                    }
                    view::ViewMode::List => {
                        let table = HISTORY.with(|h| {
                            TableViewData::from_summary(
                                &derived.summary,
                                &derived.fleet_name,
                                nav.fleet_page,
                                &h.borrow().view(chart_span, chart_window),
                            )
                        });
                        let page_count = table.page_count;
                        (layout.table(&table), page_count)
                    }
                }
            };
            let result = {
                let _s = profile::span("submit");
                render_ui(width, height, root)
            };
            let mut changed = false;
            for id in result.clicks.keys() {
                if let Some(action) = view::parse_click(id) {
                    changed |= view::apply(&mut nav, action, page_count);
                }
            }
            if changed {
                request_frame();
            }
        });
    });
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_wake() {
    set_lifecycle(Lifecycle::Awake);
    // Nothing folds while dormant, so fold before the first frame back.
    fold_if_stale();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_sleep() {
    set_lifecycle(Lifecycle::Dormant);
}

/// Touch activity notification: request a frame so the tap is consumed by
/// the next render's click readback. Without this export the host never
/// renders on touch and every button stays inert.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_touch() {
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let current = manifest_params::Params::current();
    session::refresh_params();
    let changed = manifest_params::Params::previous().map(|prev| current.changed_keys(&prev));
    if let Some(keys) = changed.as_ref() {
        for family in DeviceFamily::ALL {
            let Some(key) = filter::family_enabled_key(family) else {
                continue;
            };
            if !keys.contains(&key) {
                continue;
            }
            // Enabling resumes polling; disabling stops it mid-pass while mDNS
            // discovery keeps the devices listed.
            if session::family_enabled(family) {
                session::ensure_running(family);
            } else {
                session::stop(family);
            }
        }
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_credentials_update() {
    for family in families_on_new_credentials(&credentials::current(), &credentials::previous()) {
        restart_family(family);
    }
}

/// Start one family over on its new account, leaving the others' readings alone.
#[cfg(target_arch = "wasm32")]
fn restart_family(family: DeviceFamily) {
    let bound = session::account_bound(family);
    session::clear_tokens_for(family);
    DEVICES.with(|d| d.borrow_mut().reset_for_rebind(family, bound));
    // `stop` bumps the generation, so a response to a fetch built
    // on the old account is dropped rather than applied after the clear.
    session::stop(family);
    if bound && session::family_enabled(family) {
        session::ensure_running(family);
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::wasm_test;

    use super::{
        DORMANT_TICK_PERIOD_MS,
        device::{DeviceFamily, DeviceId, DeviceIdentity, DeviceList},
        history::{ChartSpan, HashrateHistory},
        record_polled_hashrate,
    };

    #[wasm_test]
    fn a_poll_landing_after_its_device_left_records_nothing() {
        let mut h = HashrateHistory::default();
        let mut list = DeviceList::new();
        let gone = DeviceId::new("bos/a");
        list.upsert(DeviceIdentity {
            id: gone.clone(),
            family: DeviceFamily::Bos,
            name: "a".to_owned(),
            host: "10.0.0.1".to_owned(),
            port: 80,
        });
        record_polled_hashrate(&mut h, &list, 0, &gone);
        list.remove(&gone);
        h.forget_devices(std::slice::from_ref(&gone));
        record_polled_hashrate(&mut h, &list, 5, &gone);
        assert!(
            !h.has_device_series(&gone),
            "a late poll does not bring the departed device's series back"
        );
    }

    #[wasm_test]
    fn the_dormant_tick_lands_in_every_slot_of_the_finest_tier() {
        let finest_secs = ChartSpan::ALL
            .iter()
            .map(|span| span.interval_secs())
            .min()
            .expect("BUG: there is a chart span");
        assert!(
            2 * i64::from(DORMANT_TICK_PERIOD_MS) <= finest_secs * 1_000,
            "a late {DORMANT_TICK_PERIOD_MS} ms tick can skip one of the {finest_secs} s slots"
        );
    }
}
