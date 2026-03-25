// Copyright (C) 2026  Braiins Systems s.r.o.

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

// Host function imports
unsafe extern "C" {
    fn host_ssdp_search(st_ptr: *const u8, st_len: u32, timeout: u32) -> u32;
    fn host_ssdp_stop(search_id: u32);
}

/// Callback type: `fn(search, event)`.
pub type SearchCallback = fn(SsdpSearch, &SsdpEvent<'_>);

/// Handle to an active SSDP search session.
#[derive(Clone, Copy, Debug)]
pub struct SsdpSearch(pub u32);

impl SsdpSearch {
    /// Stop this search session.
    pub fn stop(&self) {
        unsafe { host_ssdp_stop(self.0) }
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
    static SEARCHES: RefCell<HashMap<u32, usize>> = RefCell::new(HashMap::new());
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
    let search_id = unsafe {
        host_ssdp_search(
            search_target.as_ptr(),
            search_target.len() as u32,
            timeout_secs,
        )
    };
    if search_id == 0 {
        return None;
    }
    SEARCHES.with(|s| s.borrow_mut().insert(search_id, cb_idx));
    Some(SsdpSearch(search_id))
}

/// Called by the host when an SSDP event is ready.
#[unsafe(no_mangle)]
pub extern "C" fn __on_ssdp_event(search_id: u32, event_type: u32, data_ptr: u32, data_len: u32) {
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
