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

//! mDNS service discovery and registration for WASM widgets.
//!
//! Provides `mdns_browse()` for discovering LAN services and `mdns_register()`
//! for advertising services. The host manages the mDNS daemon in background
//! threads and delivers events by calling the `__on_mdns_event` export.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::mdns::*;
//!
//! fn init(width: u32, height: u32) {
//!     mdns_browse(&["_googlecast._tcp", "_touch-able._tcp"], on_mdns_event);
//! }
//!
//! fn on_mdns_event(browse: MdnsBrowse, event: &MdnsEvent<'_>) {
//!     match event {
//!         MdnsEvent::Found(json) => log_info!("found: {json}"),
//!         MdnsEvent::Removed(name) => log_info!("removed: {name}"),
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_protocol::{MdnsBrowseId, MdnsRegId};

use crate::fmt;

// Host function imports
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_mdns_browse(svc_types_ptr: *const u8, svc_types_len: u32) -> u32;
    fn host_mdns_stop(browse_id: u32);
    fn host_mdns_register(
        svc_ptr: *const u8,
        svc_len: u32,
        name_ptr: *const u8,
        name_len: u32,
        port: u32,
        txt_ptr: *const u8,
        txt_len: u32,
    ) -> u32;
    fn host_mdns_unregister(reg_id: u32);
}

/// Callback type: `fn(browse, event)`.
pub type BrowseCallback = fn(MdnsBrowse, &MdnsEvent<'_>);

/// Handle to an active mDNS browse session.
#[derive(Clone, Copy, Debug)]
pub struct MdnsBrowse(pub MdnsBrowseId);

impl MdnsBrowse {
    /// Stop this browse session.
    pub fn stop(&self) {
        unsafe { host_mdns_stop(self.0.to_wire()) }
    }
}

/// Handle to an active mDNS service registration.
#[derive(Clone, Copy, Debug)]
pub struct MdnsRegistration(pub MdnsRegId);

impl MdnsRegistration {
    /// Unregister this service.
    pub fn unregister(&self) {
        unsafe { host_mdns_unregister(self.0.to_wire()) }
    }
}

/// Event delivered from the host for an mDNS browse session.
#[derive(Debug)]
pub enum MdnsEvent<'a> {
    /// Service found/resolved. Data is JSON with service details:
    /// `{"service_type":"...","name":"...","host":"...","port":N,"txt":{...}}`
    Found(&'a str),
    /// Service removed. Data is the service full name.
    Removed(&'a str),
}

thread_local! {
    static CALLBACKS: RefCell<Vec<BrowseCallback>> = const { RefCell::new(Vec::new()) };
    static BROWSES: RefCell<HashMap<MdnsBrowseId, usize>> = RefCell::new(HashMap::new());
}

fn register_callback(cb: BrowseCallback) -> usize {
    CALLBACKS.with(|cbs| {
        let mut cbs = cbs.borrow_mut();
        for (i, existing) in cbs.iter().enumerate() {
            if *existing as usize == cb as usize {
                return i;
            }
        }
        let idx = cbs.len();
        cbs.push(cb);
        idx
    })
}

/// Browse for mDNS services of the given types. Events are delivered to
/// `callback` when services are found or removed.
///
/// Service types should be like `"_googlecast._tcp"` (the host appends `.local.`).
///
/// Returns `None` if the host rejects the browse before it is queued.
#[must_use]
pub fn mdns_browse(service_types: &[&str], callback: BrowseCallback) -> Option<MdnsBrowse> {
    let cb_idx = register_callback(callback);
    let joined = service_types.join("\n");
    let browse_id =
        MdnsBrowseId::from_wire(unsafe { host_mdns_browse(joined.as_ptr(), joined.len() as u32) })?;
    BROWSES.with(|b| b.borrow_mut().insert(browse_id, cb_idx));
    Some(MdnsBrowse(browse_id))
}

/// Register an mDNS service for advertisement on the local network.
///
/// TXT records are key-value pairs used for service metadata.
#[must_use]
pub fn mdns_register(
    service_type: &str,
    name: &str,
    port: u16,
    txt: &[(&str, &str)],
) -> Option<MdnsRegistration> {
    let txt_str: String = txt
        .iter()
        .map(|(k, v)| fmt!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");
    MdnsRegId::from_wire(unsafe {
        host_mdns_register(
            service_type.as_ptr(),
            service_type.len() as u32,
            name.as_ptr(),
            name.len() as u32,
            u32::from(port),
            txt_str.as_ptr(),
            txt_str.len() as u32,
        )
    })
    .map(MdnsRegistration)
}

/// Called by the host when an mDNS event is ready.
#[unsafe(no_mangle)]
pub extern "C" fn __on_mdns_event(browse_id: u32, event_type: u32, data_ptr: u32, data_len: u32) {
    let Some(browse_id) = MdnsBrowseId::from_wire(browse_id) else {
        return;
    };

    // Take ownership first, then borrow — avoids dangling reference.
    let owned = if data_len > 0 && data_ptr != 0 {
        unsafe { Vec::from_raw_parts(data_ptr as *mut u8, data_len as usize, data_len as usize) }
    } else {
        Vec::new()
    };
    let data = core::str::from_utf8(&owned).unwrap_or("");

    let event = match event_type {
        0 => MdnsEvent::Found(data),
        1 => MdnsEvent::Removed(data),
        _ => return,
    };

    let browse = MdnsBrowse(browse_id);

    let cb = BROWSES
        .with(|b| b.borrow().get(&browse_id).copied())
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        cb(browse, &event);
    }
}
