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

// Native builds (the storybook) can't reach the wasm-exported logic; suppress
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
mod manual;
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

#[cfg(target_arch = "wasm32")]
thread_local! {
    pub(crate) static DEVICES: RefCell<DeviceList> = RefCell::new(DeviceList::new());
    static DERIVED: RefCell<Option<DerivedView>> = const { RefCell::new(None) };
    static VIEW: RefCell<view::ViewState> = const { RefCell::new(view::ViewState::new()) };
    static HISTORY: RefCell<history::HashrateHistory> =
        RefCell::new(history::HashrateHistory::default());
    static DISPLAY_FRAME_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Coalescing window for display refreshes.
/// Each device's poll bumps the devices sequence, so re-deriving per poll
/// re-folds the whole fleet N times a round; batching updates into one render
/// per window makes that once a window instead.
///
/// Aggregates drift slowly (a device is re-polled only once a round),
/// so the sub-second lag is imperceptible.
#[cfg(target_arch = "wasm32")]
const DISPLAY_COALESCE_MS: u32 = 500;

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

/// BOS and AxeOS share `_http._tcp`; the base type resolves reliably even with a
/// co-located mDNS responder, unlike the flaky `_sub` subtype PTRs. AxeOS passes a
/// positive TXT test here, so it is identified and kept polled; BOS has no
/// discovery signal on the shared type and enters only as a candidate. Either way
/// the report waits for an answered poll — a non-miner responder never earns it.
#[cfg(target_arch = "wasm32")]
fn on_http_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => {
            let doc = JsonDoc::parse(json.as_bytes());
            let txt = |key| doc.str(key).is_some_and(|v| !v.is_empty());
            if txt("/txt/family") || txt("/txt/board") {
                ingest(&BitaxeAdapter, &doc, true);
            } else {
                ingest(&BosAdapter, &doc, false);
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
            ingest(&UbosAdapter, &doc, true);
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
    // Remove before reacting so a ring rebuild excludes the gone device instead
    // of re-polling it (matches the manual-reconcile order).
    DEVICES.with(|d| d.borrow_mut().remove(&id));
    session::remove_token(&id);
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn ingest(adapter: &dyn FamilyAdapter, doc: &JsonDoc, identified: bool) {
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
        // AxeOS/uBOS are positively family-identified at discovery, so keep polling
        // them until they answer; a base-type BOS is only a candidate. Neither
        // enters the report here — that waits for an answered poll.
        if identified {
            DEVICES.with(|d| d.borrow_mut().identify(&id));
        }
        if is_new {
            let verb = if identified {
                "discovered"
            } else {
                "sighted candidate"
            };
            log_info!(
                "fleet: {} {} {} ({})",
                verb,
                family_label(family),
                name,
                model
            );
        }
        session::on_discovered(family, is_new);
        request_frame();
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    reconcile_manual_hosts();
    // One base-type browse for BOS + AxeOS (reliable co-located), plus uBOS's own type.
    if mdns::mdns_browse(HTTP_SERVICE_TYPES, on_http_event).is_none() {
        log_warn!("fleet: HTTP mDNS browse rejected by host runtime limits");
    }
    if mdns::mdns_browse(UbosAdapter.browse_service_types(), on_ubos_event).is_none() {
        log_warn!("fleet: uBOS mDNS browse rejected by host runtime limits");
    }
    request_frame();
}

/// Hosts pinned at startup, beyond mDNS — a compile-time seam, no runtime config.
/// Empty in the shipped build; an entry reaches a device the venue's mDNS can't.
#[cfg(target_arch = "wasm32")]
const MANUAL_HOSTS: &[(DeviceFamily, &str)] = &[];

/// Reconcile [`MANUAL_HOSTS`] into `DEVICES` for every family: upsert the desired
/// identities and drop cached tokens for any row no longer wanted.
#[cfg(target_arch = "wasm32")]
fn reconcile_manual_hosts() {
    for family in DeviceFamily::ALL {
        let entries: Vec<String> = MANUAL_HOSTS
            .iter()
            .filter(|(f, _)| *f == family)
            .map(|(_, host)| (*host).to_owned())
            .collect();
        let adapter = session::adapter_for(family).expect("BUG: every DeviceFamily has an adapter");
        let Some(desired) = manual::desired_identities(family, adapter.default_port(), &entries)
        else {
            continue;
        };
        let outcome =
            DEVICES.with(|d| manual::reconcile_manual_into(&mut d.borrow_mut(), family, desired));
        for id in &outcome.removed_ids {
            session::remove_token(id);
        }
        if outcome.added_any {
            session::ensure_running(family);
        }
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
pub extern "C" fn render(_delta_ms: u32) {
    // Any render serves a pending coalesced refresh; re-arm only on the next update.
    DISPLAY_FRAME_PENDING.with(|p| p.set(false));
    let WidgetSize { width, height, .. } = widget_size();
    let seq = DEVICES.with(|d| d.borrow().seq());
    let params_version = bmc_wasm_sdk::params::version();
    DERIVED.with(|cell| {
        let mut cell = cell.borrow_mut();
        let stale = cell
            .as_ref()
            .is_none_or(|d| d.devices_seq != seq || d.params_version != params_version);
        if stale {
            let params = manifest_params::Params::current();
            let filters = filter::Filters {
                axeos_enabled: params.axeos_enabled,
                ..filter::Filters::default()
            };
            let summary = {
                let _s = profile::span("summarize");
                DEVICES.with(|d| summary::summarize(&d.borrow(), &filters))
            };
            // Append one hashrate sample per new devices sequence for the charts.
            {
                let _s = profile::span("history");
                DEVICES
                    .with(|d| HISTORY.with(|h| h.borrow_mut().record(seq, &summary, &d.borrow())));
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
                            let series = HISTORY.with(|h| h.borrow().device_series(id));
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
                                        )
                                    })
                            })
                        });
                    if let Some(data) = detail {
                        (screens::device_detail::device_detail_view(&data), 1)
                    } else {
                        let data = HISTORY.with(|h| {
                            screens::model_detail::ModelDetailViewData::from_summary(
                                &derived.fleet_name,
                                &sel.label,
                                rows,
                                sel.page,
                                &h.borrow(),
                            )
                        });
                        let page_count = data.page_count;
                        (screens::model_detail::model_detail_view(&data), page_count)
                    }
                } else {
                    let data = HISTORY.with(|h| {
                        screens::model_detail::ModelDetailViewData::from_summary(
                            &derived.fleet_name,
                            &sel.label,
                            rows,
                            sel.page,
                            &h.borrow(),
                        )
                    });
                    let page_count = data.page_count;
                    (screens::model_detail::model_detail_view(&data), page_count)
                }
            } else if derived.summary.groups.is_empty() {
                if let Some(data) = no_credentials(&derived.fleet_name) {
                    (screens::no_credentials::no_credentials_view(&data), 1)
                } else {
                    (screens::searching(), 1)
                }
            } else {
                match nav.mode {
                    view::ViewMode::Grid => {
                        let data = HISTORY.with(|h| {
                            DashboardViewData::from_summary(
                                &derived.summary,
                                &derived.fleet_name,
                                &h.borrow(),
                            )
                        });
                        (screens::dashboard::dashboard_view(&data), 1)
                    }
                    view::ViewMode::List => {
                        let table = HISTORY.with(|h| {
                            TableViewData::from_summary(
                                &derived.summary,
                                &derived.fleet_name,
                                nav.fleet_page,
                                &h.borrow(),
                            )
                        });
                        let page_count = table.page_count;
                        (screens::table::table_view(&table), page_count)
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
            if !keys.contains(&filter::family_enabled_key(family)) {
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
