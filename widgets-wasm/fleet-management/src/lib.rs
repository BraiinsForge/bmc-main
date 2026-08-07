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
mod layout;
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

#[cfg(target_arch = "wasm32")]
mod manifest_params;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

#[cfg(target_arch = "wasm32")]
use std::cell::{Cell, RefCell};

#[cfg(target_arch = "wasm32")]
use adapter::FamilyAdapter;
#[cfg(target_arch = "wasm32")]
use device::{DeviceFamily, DeviceId, DeviceList, credential_keys, family_label};
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

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderedScreen {
    Unknown,
    NoCredentials,
    Other,
}

#[cfg(any(target_arch = "wasm32", test))]
fn network_update_needs_frame(screen: RenderedScreen) -> bool {
    !matches!(screen, RenderedScreen::Other)
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    pub(crate) static DEVICES: RefCell<DeviceList> = RefCell::new(DeviceList::new());
    static DERIVED: RefCell<Option<DerivedView>> = const { RefCell::new(None) };
    static VIEW: RefCell<view::ViewState> = const { RefCell::new(view::ViewState::new()) };
    static HISTORY: RefCell<history::HashrateHistory> =
        RefCell::new(history::HashrateHistory::default());
    static DISPLAY_FRAME_PENDING: Cell<bool> = const { Cell::new(false) };
    static DERIVE_ELAPSED_MS: Cell<u32> = const { Cell::new(0) };
    static RENDERED_SCREEN: Cell<RenderedScreen> = const { Cell::new(RenderedScreen::Unknown) };
}

/// Coalescing window for display refreshes.
/// Each device's poll bumps the devices sequence, so re-deriving
/// per poll re-folds the whole fleet N times a round; batching updates
/// into one render per window makes that once a window instead.
///
/// Aggregates drift slowly (a device is re-polled only once a round),
/// so the sub-second lag is imperceptible.
#[cfg(target_arch = "wasm32")]
const DISPLAY_COALESCE_MS: u32 = 500;

/// Cadence for the fleet fold (summarize + history), decoupled from render
/// and poll `seq` bumps. The ~90-device fold is costly, so renders between
/// ticks read a cached snapshot instead of re-folding.
#[cfg(target_arch = "wasm32")]
const DERIVE_INTERVAL_MS: u32 = 1_000;

/// Request a coalesced display refresh: a no-op while one
/// is already scheduled, so a burst of device updates costs
/// one re-derive/render, not N.
#[cfg(target_arch = "wasm32")]
pub(crate) fn request_display_frame() {
    if !DISPLAY_FRAME_PENDING.with(|p| p.replace(true)) {
        request_frame_after(DISPLAY_COALESCE_MS);
    }
}

/// The render-ready fleet summary, cached so the filter → group → fold → sort
/// pipeline runs only when the fleet or the params
/// actually change — not on every render frame (renders fire per discovery and
/// per telemetry event, hundreds per pass on a large fleet). The model-detail
/// rows are cached per selection and cleared on any seq/params rebuild.
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
/// AxeOS passes a positive TXT test here, so it is identified straight away.
/// BOS has no discovery signal on the shared type, so it is fingerprinted first,
/// identified only when its version probe clears — a non-miner responder never
/// gets that far. Either way the report waits for an answered poll.
#[cfg(target_arch = "wasm32")]
fn on_http_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => {
            let doc = JsonDoc::parse(json.as_bytes());
            let txt = |key| doc.str(key).is_some_and(|v| !v.is_empty());
            if txt("/txt/family") || txt("/txt/board") {
                ingest(&BitaxeAdapter, &doc);
            } else {
                // BOS shares `_http._tcp` with arbitrary hosts; fingerprint the host
                // before crediting it, so root credentials never reach a non-BOS box.
                session::probe_bos_candidate(json);
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
fn on_ubos_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => {
            let doc = JsonDoc::parse(json.as_bytes());
            ingest(&UbosAdapter, &doc);
        }
        mdns::MdnsEvent::Removed(name) => on_removed(DeviceFamily::Ubos, name),
    }
}

/// React to an mDNS `ServiceRemoved`: drop an unconfirmed discovery,
/// but keep a confirmed miner (its liveness is polling-governed, not mDNS-governed).
/// The family namespaces the id, matching how the device was inserted.
#[cfg(target_arch = "wasm32")]
fn on_removed(family: DeviceFamily, name: &str) {
    let id = DeviceId::for_family(family, name);
    // mDNS `ServiceRemoved` fires on cache expiry — a missed multicast
    // refresh over lossy WiFi, not only a real departure.
    //
    // A confirmed miner (answered a poll) is kept regardless,
    // its liveness governed by polling from here, so a dropped
    // announcement can't churn it out; only a device that never
    // proved itself leaves on an mDNS removal.
    let found = DEVICES.with(|d| {
        d.borrow()
            .iter()
            .find(|dev| dev.identity.id == id)
            .map(|dev| {
                (
                    dev.identity.family,
                    dev.model.as_ref().map(|m| m.name.clone()),
                    dev.is_confirmed(),
                )
            })
    });
    let Some((family, model, confirmed)) = found else {
        return;
    };
    if confirmed {
        log_info!(
            "fleet: kept confirmed {} {} despite mDNS removal",
            family_label(family),
            name
        );
        return;
    }
    let model = model.unwrap_or_else(|| "unknown model".to_owned());
    log_info!(
        "fleet: removed {} {} ({})",
        family_label(family),
        name,
        model
    );
    // Remove before reacting so a ring rebuild excludes the gone device
    // instead of re-polling it.
    DEVICES.with(|d| d.borrow_mut().remove(&id));
    session::remove_token(&id);
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn ingest(adapter: &dyn FamilyAdapter, doc: &JsonDoc) {
    if let Some(found) = adapter.parse_found(doc) {
        let identity = found.identity;
        let model_hint = found.model_hint;
        let family = identity.family;
        let name = identity.name.clone();
        let id = identity.id.clone();
        let model = model_hint
            .as_ref()
            .map_or_else(|| "model pending".to_owned(), |m| m.name.clone());
        let is_new = DEVICES.with(|d| d.borrow_mut().upsert_with_model_hint(identity, model_hint));
        // Every family reaches here positively identified: AxeOS by its TXT test, uBOS
        // by its dedicated service type, a base-type BOS once its fingerprint clears.
        // So keep it polled — the report still waits for an answered poll.
        DEVICES.with(|d| d.borrow_mut().identify(&id));
        if is_new {
            log_info!(
                "fleet: discovered {} {} ({})",
                family_label(family),
                name,
                model
            );
        }
        session::on_discovered(family, is_new);
        request_frame();
    }
}

/// Ingest a BOS whose version fingerprint cleared — positive identification,
/// the same role AxeOS's discovery TXT test plays.
#[cfg(target_arch = "wasm32")]
pub(crate) fn ingest_probed_bos(json: &str) {
    let doc = JsonDoc::parse(json.as_bytes());
    ingest(&BosAdapter, &doc);
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    restore_history();
    // One base-type browse for BOS + AxeOS (reliable co-located), plus uBOS's own type.
    if mdns::mdns_browse(HTTP_SERVICE_TYPES, on_http_event).is_none() {
        log_warn!("fleet: HTTP mDNS browse rejected by host runtime limits");
    }
    if mdns::mdns_browse(UbosAdapter.browse_service_types(), on_ubos_event).is_none() {
        log_warn!("fleet: uBOS mDNS browse rejected by host runtime limits");
    }
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

/// The no-credentials fallback for an otherwise-empty fleet.
/// Unreachable as written: a fingerprinted BOS is identified
/// and so reports, which keeps the fleet non-empty and the gate shut.
///
/// Kept for the missing-credentials state BDK-434 adds,
/// to key on instead. `None` keeps the generic "Searching…" state.
#[cfg(target_arch = "wasm32")]
fn no_credentials(fleet_name: &str) -> Option<screens::no_credentials::NoCredentialsData> {
    let params = manifest_params::Params::current();
    if !params.bos_password.is_empty() {
        return None;
    }
    let seen_bos = DEVICES.with(|d| {
        d.borrow()
            .iter()
            .any(|dev| dev.identity.family == DeviceFamily::Bos)
    });
    if !seen_bos {
        return None;
    }
    let net = bmc_wasm_sdk::network::info();
    let url = if net.ip.is_empty() {
        String::new()
    } else {
        bmc_wasm_sdk::fmt!("http://{}", net.ip)
    };
    Some(screens::no_credentials::NoCredentialsData {
        fleet_name: fleet_name.to_owned(),
        ssid: net.ssid,
        url,
    })
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
#[expect(
    clippy::too_many_lines,
    reason = "the render entry orchestrates derive, view dispatch, and click routing"
)]
pub extern "C" fn render(delta_ms: u32) {
    // Any render serves a pending coalesced refresh; re-arm only on the next update.
    DISPLAY_FRAME_PENDING.with(|p| p.set(false));
    let WidgetSize { width, height, .. } = widget_size();
    let seq = DEVICES.with(|d| d.borrow().seq());
    let params_version = bmc_wasm_sdk::params::version();
    let elapsed = DERIVE_ELAPSED_MS.with(|e| {
        let acc = e.get().saturating_add(delta_ms);
        e.set(acc);
        acc
    });
    DERIVED.with(|cell| {
        let mut cell = cell.borrow_mut();
        // The fold runs on its own interval, not per render,
        // so poll `seq` churn no longer re-folds the whole fleet every frame.
        // The first frame and params changes still derive immediately.
        let stale = match cell.as_ref() {
            None => true,
            Some(d) => {
                d.params_version != params_version
                    || (d.devices_seq != seq && elapsed >= DERIVE_INTERVAL_MS)
            }
        };
        // A fold the interval held back still has to happen. Once the poller parks,
        // nothing else asks for the frame that would run it, so the last change
        // before an empty fleet would sit on screen until something unrelated arrives.
        // A running rotation already asks on every poll result, so this stays quiet.
        let fold_pending = cell.as_ref().is_some_and(|d| d.devices_seq != seq);
        if !stale && fold_pending && !session::is_polling() {
            request_frame_after(DERIVE_INTERVAL_MS.saturating_sub(elapsed));
        }

        if stale {
            DERIVE_ELAPSED_MS.with(|e| e.set(0));
            let params = manifest_params::Params::current();
            let filters = filter::Filters {
                axeos_enabled: params.axeos_enabled,
            };
            let summary = {
                let _s = profile::span("summarize");
                DEVICES.with(|d| summary::summarize(&d.borrow(), &filters))
            };
            // Feed the fold into every chart tier; each samples at its own rate,
            // then persist the aggregate tiers so the charts survive a restart.
            {
                let _s = profile::span("history");
                let now = SystemTime::now().unix_secs;
                DEVICES.with(|d| {
                    let devices = d.borrow();
                    HISTORY.with(|h| {
                        let mut h = h.borrow_mut();
                        h.record(now, &summary, &devices);
                        if let Some(blob) = h.take_snapshot(now) {
                            cache::put(history::CACHE_TAG, &[], &blob);
                        }
                    });
                });
            }
            *cell = Some(DerivedView {
                devices_seq: seq,
                params_version,
                summary,
                filters,
                fleet_name: params.fleet_name,
                model_detail: None,
            });
        }
        let derived = cell.as_mut().expect("BUG: derived view populated above");
        let chart_span =
            history::ChartSpan::from(manifest_params::Params::current().chart_span_minutes);

        // The chart's time axis: render now back one range, so points
        // place by timestamp with the newest at the right edge.
        let chart_window = history::ChartWindow {
            end: SystemTime::now().unix_secs,
            span_secs: i64::from(chart_span.minutes()) * 60,
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
            let (root, page_count, rendered_screen) = if let Some(sel) = nav.model_detail.as_ref() {
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
                            let series =
                                HISTORY.with(|h| h.borrow().view(chart_span).device_series(id));
                            DEVICES.with(|devs| {
                                devs.borrow()
                                    .iter()
                                    .find(|dev| dev.identity.id.as_str() == device_id)
                                    .map(|dev| {
                                        screens::device_detail::DeviceDetailData::from_device(
                                            &derived.fleet_name,
                                            &sel.label,
                                            group,
                                            dev,
                                            series,
                                            chart_window,
                                        )
                                    })
                            })
                        });
                    if let Some(data) = detail {
                        (
                            screens::device_detail::device_detail_view(&data),
                            1,
                            RenderedScreen::Other,
                        )
                    } else {
                        let data = HISTORY.with(|h| {
                            screens::model_detail::ModelDetailViewData::from_summary(
                                &derived.fleet_name,
                                &sel.label,
                                rows,
                                sel.page,
                                &h.borrow().view(chart_span),
                                chart_window,
                            )
                        });
                        let page_count = data.page_count;
                        (
                            screens::model_detail::model_detail_view(&data),
                            page_count,
                            RenderedScreen::Other,
                        )
                    }
                } else {
                    let data = HISTORY.with(|h| {
                        screens::model_detail::ModelDetailViewData::from_summary(
                            &derived.fleet_name,
                            &sel.label,
                            rows,
                            sel.page,
                            &h.borrow().view(chart_span),
                            chart_window,
                        )
                    });
                    let page_count = data.page_count;
                    (
                        screens::model_detail::model_detail_view(&data),
                        page_count,
                        RenderedScreen::Other,
                    )
                }
            } else if derived.summary.groups.is_empty() {
                if let Some(data) = no_credentials(&derived.fleet_name) {
                    (
                        screens::no_credentials::no_credentials_view(&data),
                        1,
                        RenderedScreen::NoCredentials,
                    )
                } else {
                    (screens::searching(), 1, RenderedScreen::Other)
                }
            } else {
                match nav.mode {
                    view::ViewMode::Grid => {
                        let data = HISTORY.with(|h| {
                            DashboardViewData::from_summary(
                                &derived.summary,
                                &derived.fleet_name,
                                &h.borrow().view(chart_span),
                                chart_window,
                            )
                        });
                        (
                            screens::dashboard::dashboard_view(&data),
                            1,
                            RenderedScreen::Other,
                        )
                    }
                    view::ViewMode::List => {
                        let table = HISTORY.with(|h| {
                            TableViewData::from_summary(
                                &derived.summary,
                                &derived.fleet_name,
                                nav.fleet_page,
                                &h.borrow().view(chart_span),
                                chart_window,
                            )
                        });
                        let page_count = table.page_count;
                        (
                            screens::table::table_view(&table),
                            page_count,
                            RenderedScreen::Other,
                        )
                    }
                }
            };
            let result = {
                let _s = profile::span("submit");
                render_ui(width, height, root)
            };
            RENDERED_SCREEN.with(|screen| screen.set(rendered_screen));
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

/// Touch activity notification: request a frame so the tap is consumed by
/// the next render's click readback. Without this export the host never
/// renders on touch and every button stays inert.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_touch() {
    request_frame();
}

/// The Deck's SSID/IP changed. Only the no-credentials screen displays them,
/// so every other screen ignores the notification.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_network_update() {
    if RENDERED_SCREEN.with(|screen| network_update_needs_frame(screen.get())) {
        request_frame();
    }
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
    // Reset only the family whose credentials actually changed: a BOS-password
    // edit must not blank uBOS/AxeOS. A credential-less family (empty
    // `credential_keys`) is never reset, and an unknown change (`changed` is
    // `None`, i.e. the first update) refreshes every family that has credentials.
    for family in DeviceFamily::ALL {
        let creds = credential_keys(family);
        let creds_changed = !creds.is_empty()
            && changed
                .as_ref()
                .is_none_or(|keys| creds.iter().any(|k| keys.contains(k)));
        if !creds_changed {
            continue;
        }
        session::clear_tokens_for(family);
        DEVICES.with(|d| d.borrow_mut().clear_telemetry_for(family));
        // Drop fetches already issued with the old credentials — `stop` bumps
        // the generation so their responses are ignored rather than applied
        // after the UI was cleared — then re-poll the family if it is enabled.
        session::stop(family);
        if session::family_enabled(family) {
            session::ensure_running(family);
        }
    }
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

#[cfg(test)]
mod rendered_screen_tests {
    use super::{RenderedScreen, network_update_needs_frame};

    #[test]
    fn network_update_before_first_render_requests_frame() {
        assert!(
            network_update_needs_frame(RenderedScreen::Unknown),
            "the first render must observe network info delivered during startup"
        );
    }

    #[test]
    fn network_update_refreshes_no_credentials_screen() {
        assert!(
            network_update_needs_frame(RenderedScreen::NoCredentials),
            "the no-credentials screen displays the Deck network"
        );
    }

    #[test]
    fn network_update_ignores_other_rendered_screens() {
        assert!(
            !network_update_needs_frame(RenderedScreen::Other),
            "screens without Deck network data must not repaint"
        );
    }
}
