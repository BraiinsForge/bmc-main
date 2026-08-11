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

//! SSDP (UPnP) device discovery for WASM widgets.
//!
//! Provides `ssdp_search()` for discovering UPnP/DLNA devices on the local
//! network via SSDP M-SEARCH. The host manages the search in a background
//! thread and delivers events by calling the `__on_ssdp_event` export.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::ssdp::*;
//!
//! fn init(width: u32, height: u32) {
//!     ssdp_search("urn:schemas-upnp-org:device:MediaRenderer:1", 5, on_ssdp_event);
//! }
//!
//! fn on_ssdp_event(search: SsdpSearch, event: &SsdpEvent<'_>) {
//!     match event {
//!         SsdpEvent::Found(json) => log_info!("found: {json}"),
//!         SsdpEvent::Removed(usn) => log_info!("removed: {usn}"),
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_protocol::SsdpSearchId;

// Host function imports
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_ssdp_search(st_ptr: *const u8, st_len: u32, timeout: u32) -> u32;
    fn host_ssdp_stop(search_id: u32);
}

/// Callback type: `fn(search, event)`.
pub type SearchCallback = fn(SsdpSearch, &SsdpEvent<'_>);

/// Handle to an active SSDP search session.
#[derive(Clone, Copy, Debug)]
pub struct SsdpSearch(pub SsdpSearchId);

impl SsdpSearch {
    /// Stop this search session.
    pub fn stop(&self) {
        unsafe { host_ssdp_stop(self.0.to_wire()) }
    }
}

/// Event delivered from the host for an SSDP search session.
#[derive(Debug)]
pub enum SsdpEvent<'a> {
    /// Device found. Data is JSON with device details:
    /// `{"usn":"...","location":"...","name":"...","host":"...","port":N,
    ///   "av_transport_path":"...","rendering_control_path":"..."}`
    Found(&'a str),
    /// Device removed. Data is the USN string.
    Removed(&'a str),
}

thread_local! {
    static CALLBACKS: RefCell<Vec<SearchCallback>> = const { RefCell::new(Vec::new()) };
    static SEARCHES: RefCell<HashMap<SsdpSearchId, usize>> = RefCell::new(HashMap::new());
}

fn register_callback(cb: SearchCallback) -> usize {
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

/// Start an SSDP M-SEARCH for devices matching the given search target.
///
/// The `search_target` is a UPnP device/service URN, e.g.
/// `"urn:schemas-upnp-org:device:MediaRenderer:1"`.
///
/// `timeout_secs` is the MX value for the M-SEARCH (how long devices may
/// delay their responses).
///
/// Returns `None` if the host rejects the search before it is queued.
#[must_use]
pub fn ssdp_search(
    search_target: &str,
    timeout_secs: u32,
    callback: SearchCallback,
) -> Option<SsdpSearch> {
    let cb_idx = register_callback(callback);
    let search_id = SsdpSearchId::from_wire(unsafe {
        host_ssdp_search(
            search_target.as_ptr(),
            search_target.len() as u32,
            timeout_secs,
        )
    })?;
    SEARCHES.with(|s| s.borrow_mut().insert(search_id, cb_idx));
    Some(SsdpSearch(search_id))
}

/// Called by the host when an SSDP event is ready.
#[unsafe(no_mangle)]
pub extern "C" fn __on_ssdp_event(search_id: u32, event_type: u32, data_ptr: u32, data_len: u32) {
    let Some(search_id) = SsdpSearchId::from_wire(search_id) else {
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
        0 => SsdpEvent::Found(data),
        1 => SsdpEvent::Removed(data),
        _ => return,
    };

    let search = SsdpSearch(search_id);

    let cb = SEARCHES
        .with(|s| s.borrow().get(&search_id).copied())
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        cb(search, &event);
    }
}
