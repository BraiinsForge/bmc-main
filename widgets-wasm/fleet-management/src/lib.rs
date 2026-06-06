// Copyright (C) 2026  Braiins Systems s.r.o.

mod adapter;
mod device;
mod discovery;
mod families;
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
use device::{DeviceId, DeviceList, family_label};
#[cfg(target_arch = "wasm32")]
use families::bitaxe::BitaxeAdapter;
#[cfg(target_arch = "wasm32")]
use families::bos::BosAdapter;
#[cfg(target_arch = "wasm32")]
use families::ubos::UbosAdapter;

#[cfg(target_arch = "wasm32")]
thread_local! {
    pub(crate) static DEVICES: RefCell<DeviceList> = RefCell::new(DeviceList::new());
}

#[cfg(target_arch = "wasm32")]
fn on_bos_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&BosAdapter, json),
        mdns::MdnsEvent::Removed(name) => on_removed(name),
    }
}

#[cfg(target_arch = "wasm32")]
fn on_ubos_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&UbosAdapter, json),
        mdns::MdnsEvent::Removed(name) => on_removed(name),
    }
}

#[cfg(target_arch = "wasm32")]
fn on_bitaxe_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&BitaxeAdapter, json),
        mdns::MdnsEvent::Removed(name) => on_removed(name),
    }
}

/// Drop a device discovery reported as gone, logging its family and model
/// before it leaves the list.
#[cfg(target_arch = "wasm32")]
fn on_removed(name: &str) {
    let id = DeviceId::new(name);
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
    // Remove from the device list before reacting, so a driver re-snapshot
    // triggered by the removal (a `Redefer`) cannot re-arm a kick to the gone
    // device. This matches the manual-reconcile order (`reconcile_manual_hosts`).
    DEVICES.with(|d| d.borrow_mut().remove(&id));
    session::remove_token(&id);
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

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width,
        height,
        variant,
    } = widget_size();
    let title = manifest_params::Params::current().fleet_name;
    let root = DEVICES.with(|d| render::view(&d.borrow(), variant, &title));
    let _ = render_ui(width, height, root);
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let creds_changed = manifest_params::Params::previous().is_none_or(|prev| {
        manifest_params::Params::current()
            .changed_keys(&prev)
            .iter()
            .any(|k| matches!(*k, "bos_password" | "ubos_username" | "ubos_password"))
    });
    if creds_changed {
        session::clear_tokens();
        DEVICES.with(|d| d.borrow_mut().clear_all_telemetry());
    }
    request_frame();
}
