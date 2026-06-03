// Copyright (C) 2026  Braiins Systems s.r.o.

mod adapter;
mod device;
mod discovery;
mod families;
mod model;
mod session;
mod telemetry;

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
use device::{DeviceId, DeviceList};
#[cfg(target_arch = "wasm32")]
use families::bos::BosAdapter;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static DEVICES: RefCell<DeviceList> = RefCell::new(DeviceList::new());
}

#[cfg(target_arch = "wasm32")]
fn on_bos_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&BosAdapter, json),
        mdns::MdnsEvent::Removed(name) => {
            DEVICES.with(|d| d.borrow_mut().remove(&DeviceId::new(*name)));
            request_frame();
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn ingest(adapter: &dyn FamilyAdapter, json: &str) {
    let doc = JsonDoc::parse(json.as_bytes());
    if let Some(found) = adapter.parse_found(&doc) {
        DEVICES.with(|d| d.borrow_mut().upsert(found.identity));
        request_frame();
    }
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    if mdns::mdns_browse(BosAdapter.browse_service_types(), on_bos_event).is_none() {
        log_warn!("fleet: BOS mDNS browse rejected by host runtime limits");
    }
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize { width, height, .. } = widget_size();
    let root = DEVICES.with(|d| render::view(&d.borrow(), width, height));
    let _ = render_ui(width, height, root);
}
