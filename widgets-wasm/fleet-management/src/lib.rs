// Copyright (C) 2026  Braiins Systems s.r.o.

mod adapter;
mod device;
mod discovery;
mod families;
mod filter;
mod layout;
mod manual;
mod model;
mod session;
mod summary;
mod telemetry;

#[cfg(target_arch = "wasm32")]
mod manifest_params;

#[cfg(target_arch = "wasm32")]
mod render;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(target_arch = "wasm32")]
use adapter::FamilyAdapter;
#[cfg(target_arch = "wasm32")]
use device::{DeviceFamily, DeviceId, DeviceList, family_label};
#[cfg(target_arch = "wasm32")]
use families::bitaxe::BitaxeAdapter;
#[cfg(target_arch = "wasm32")]
use families::bos::BosAdapter;
#[cfg(target_arch = "wasm32")]
use families::ubos::UbosAdapter;

#[cfg(target_arch = "wasm32")]
thread_local! {
    pub(crate) static DEVICES: RefCell<DeviceList> = RefCell::new(DeviceList::new());
    static DERIVED: RefCell<Option<DerivedView>> = const { RefCell::new(None) };
}

/// The render-ready fleet summary, cached so the filter → group → fold → sort
/// pipeline and the model-list parsing run only when the fleet or the params
/// actually change — not on every render frame (renders fire per discovery and
/// per telemetry event, hundreds per pass on a large fleet).
#[cfg(target_arch = "wasm32")]
struct DerivedView {
    devices_seq: u64,
    params_version: u64,
    summary: summary::FleetSummary,
    fleet_name: String,
}

#[cfg(target_arch = "wasm32")]
fn on_bos_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&BosAdapter, json),
        mdns::MdnsEvent::Removed(name) => on_removed(DeviceFamily::Bos, name),
    }
}

#[cfg(target_arch = "wasm32")]
fn on_ubos_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&UbosAdapter, json),
        mdns::MdnsEvent::Removed(name) => on_removed(DeviceFamily::Ubos, name),
    }
}

#[cfg(target_arch = "wasm32")]
fn on_bitaxe_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&BitaxeAdapter, json),
        mdns::MdnsEvent::Removed(name) => on_removed(DeviceFamily::Bitaxe, name),
    }
}

/// Drop a device discovery reported as gone, logging its family and model
/// before it leaves the list. The family namespaces the id, matching how the
/// device was inserted.
#[cfg(target_arch = "wasm32")]
fn on_removed(family: DeviceFamily, name: &str) {
    let id = DeviceId::for_family(family, name);
    let info = DEVICES.with(|d| {
        d.borrow()
            .iter()
            .find(|dev| dev.identity.id == id)
            .map(|dev| {
                (
                    dev.identity.family,
                    dev.model.as_ref().map(|m| m.name.clone()),
                )
            })
    });
    if let Some((family, model)) = info {
        let model = model.unwrap_or_else(|| "unknown model".to_owned());
        log_info!(
            "fleet: removed {} {} ({})",
            family_label(family),
            name,
            model
        );
    }
    session::remove_token(&id);
    DEVICES.with(|d| d.borrow_mut().remove(&id));
    request_frame();
}

#[cfg(target_arch = "wasm32")]
fn ingest(adapter: &dyn FamilyAdapter, json: &str) {
    let doc = JsonDoc::parse(json.as_bytes());
    if let Some(found) = adapter.parse_found(&doc) {
        let identity = found.identity;
        let model_hint = found.model_hint;
        let family = identity.family;
        let name = identity.name.clone();
        let model = model_hint
            .as_ref()
            .map_or_else(|| "model pending".to_owned(), |m| m.name.clone());
        let is_new = DEVICES.with(|d| d.borrow_mut().upsert_with_model_hint(identity, model_hint));
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

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    reconcile_manual_hosts();
    if mdns::mdns_browse(BosAdapter.browse_service_types(), on_bos_event).is_none() {
        log_warn!("fleet: BOS mDNS browse rejected by host runtime limits");
    }
    if mdns::mdns_browse(UbosAdapter.browse_service_types(), on_ubos_event).is_none() {
        log_warn!("fleet: uBOS mDNS browse rejected by host runtime limits");
    }
    if mdns::mdns_browse(BitaxeAdapter.browse_service_types(), on_bitaxe_event).is_none() {
        log_warn!("fleet: AxeOS mDNS browse rejected by host runtime limits");
    }
    request_frame();
}

/// The manual-host param string for a family.
#[cfg(target_arch = "wasm32")]
fn manual_hosts_param(family: DeviceFamily) -> String {
    let params = manifest_params::Params::current();
    match family {
        DeviceFamily::Bos => params.bos_hosts,
        DeviceFamily::Ubos => params.ubos_hosts,
        DeviceFamily::Bitaxe => params.axeos_hosts,
    }
}

/// Parse a JSON array of strings into its entries, or `None` if `raw` is not
/// valid JSON. A valid non-array (or array whose elements are not strings)
/// yields an empty `Vec`. Shared by the manual-host and model-list parsers.
#[cfg(target_arch = "wasm32")]
fn parse_string_array(raw: &str) -> Option<Vec<String>> {
    let doc = JsonDoc::parse(raw.as_bytes());
    if !doc.is_valid() {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(entry) = doc.str(&fmt!("/{i}")) {
        out.push(entry);
        i += 1;
    }
    Some(out)
}

/// True if `raw` is an empty JSON array, tolerating whitespace inside and
/// around the brackets (`[]`, `[ ]`, `[\n]`). This is the only spelling that
/// clears a family's manual hosts.
#[cfg(target_arch = "wasm32")]
fn is_empty_array(raw: &str) -> bool {
    raw.trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .is_some_and(|inner| inner.trim().is_empty())
}

/// Parse a manual-host param into raw entries. Returns `None` (leave the
/// family's manual set unchanged) for invalid JSON, or for any valid shape
/// that yields no entries except an empty array (which clears). This makes the
/// destructive clear require explicit intent and protects the fleet from a typo.
#[cfg(target_arch = "wasm32")]
fn parse_host_list(raw: &str) -> Option<Vec<String>> {
    let out = parse_string_array(raw)?;
    if out.is_empty() && !is_empty_array(raw) {
        return None;
    }
    Some(out)
}

/// Reconcile every family's manual hosts from the current params into `DEVICES`,
/// then drop tokens for removed devices and (re)start polling where devices
/// were added. Callable both at `init` and from `on_params_update`.
#[cfg(target_arch = "wasm32")]
fn reconcile_manual_hosts() {
    for family in DeviceFamily::ALL {
        let raw = manual_hosts_param(family);
        let Some(entries) = parse_host_list(&raw) else {
            log_warn!(
                "fleet: {} manual hosts param is not a valid host array; leaving manual hosts unchanged",
                family_label(family)
            );
            continue;
        };
        let adapter = session::adapter_for(family).expect("BUG: every DeviceFamily has an adapter");
        let default_port = adapter.default_port();
        let Some(desired) = manual::desired_identities(family, default_port, &entries) else {
            log_warn!(
                "fleet: {} manual hosts param has entries but none are valid; leaving manual hosts unchanged",
                family_label(family)
            );
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

/// Parse a JSON-array-of-strings operator param into model-name fragments.
/// An invalid or non-array value yields an empty list (no filtering).
#[cfg(target_arch = "wasm32")]
fn parse_model_list(raw: &str) -> Vec<String> {
    parse_string_array(raw).unwrap_or_else(|| {
        log_warn!("fleet: model-list param is not valid JSON; model filtering disabled");
        Vec::new()
    })
}

/// Parse a model-list param and normalize every fragment for matching. Done
/// once per filter build so `matches_any` compares against pre-normalized
/// fragments instead of re-normalizing per device.
#[cfg(target_arch = "wasm32")]
fn parse_normalized_model_list(raw: &str) -> Vec<String> {
    filter::normalized_fragments(parse_model_list(raw).iter().map(String::as_str))
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width,
        height,
        variant,
    } = widget_size();
    let seq = DEVICES.with(|d| d.borrow().seq());
    let params_version = bmc_wasm_sdk::params::version();
    DERIVED.with(|cell| {
        let mut cell = cell.borrow_mut();
        let stale = cell
            .as_ref()
            .is_none_or(|d| d.devices_seq != seq || d.params_version != params_version);
        if stale {
            let params = manifest_params::Params::current();
            // Normalize the model-list fragments once here, not per device in
            // `matches_any` — `summarize` runs per telemetry event.
            let filters = filter::Filters {
                whitelist: parse_normalized_model_list(&params.model_whitelist),
                blacklist: parse_normalized_model_list(&params.model_blacklist),
                bos_enabled: params.bos_enabled,
                ubos_enabled: params.ubos_enabled,
                axeos_enabled: params.axeos_enabled,
            };
            let summary = DEVICES.with(|d| summary::summarize(&d.borrow(), &filters));
            *cell = Some(DerivedView {
                devices_seq: seq,
                params_version,
                summary,
                fleet_name: params.fleet_name,
            });
        }
        let derived = cell.as_ref().expect("BUG: derived view populated above");
        let root = render::view(
            &derived.summary,
            width,
            height,
            variant,
            &derived.fleet_name,
        );
        let _ = render_ui(width, height, root);
    });
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let current = manifest_params::Params::current();
    session::refresh_params();
    let changed = manifest_params::Params::previous().map(|prev| current.changed_keys(&prev));
    let creds_changed = changed.as_ref().is_none_or(|keys| {
        keys.iter()
            .any(|k| matches!(*k, "bos_password" | "ubos_username" | "ubos_password"))
    });
    if creds_changed {
        session::clear_tokens();
        DEVICES.with(|d| d.borrow_mut().clear_all_telemetry());
        // Drop fetches already issued with the old credentials — `stop` bumps
        // the generation so their responses are ignored rather than applied
        // after the UI was cleared — then re-poll enabled families with the
        // new credentials.
        for family in DeviceFamily::ALL {
            session::stop(family);
            if session::family_enabled(family) {
                session::ensure_running(family);
            }
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
    if changed.as_ref().is_none_or(|keys| {
        keys.iter()
            .any(|k| matches!(*k, "bos_hosts" | "ubos_hosts" | "axeos_hosts"))
    }) {
        reconcile_manual_hosts();
    }
    request_frame();
}
