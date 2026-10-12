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
//! An [`SsdpSearch`] is the only handle to its search,
//! and dropping it stops the search,
//! so a widget keeps the handle for as long as it wants the search.
//! The callback borrows the search its event belongs to;
//! to stop the search from there, take the stored handle.
//!
//! Off-target there is no host to search with, so starting a search panics.
//!
//! # Example
//!
//! ```ignore
//! use std::cell::RefCell;
//!
//! use bmc_wasm_sdk::*;
//! use bmc_wasm_sdk::ssdp::*;
//!
//! thread_local! {
//!     static SEARCH: RefCell<Option<SsdpSearch>> = const { RefCell::new(None) };
//! }
//!
//! #[unsafe(no_mangle)]
//! pub extern "C" fn init() {
//!     SEARCH.set(ssdp_search("urn:schemas-upnp-org:device:MediaRenderer:1", 5, on_ssdp_event));
//! }
//!
//! fn on_ssdp_event(_search: &SsdpSearch, event: &SsdpEvent<'_>) {
//!     match event {
//!         SsdpEvent::Found(json) => log_info!("found: {json}"),
//!         SsdpEvent::Removed(usn) => log_info!("removed: {usn}"),
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use bmc_wasm_protocol::SsdpSearchId;

/// Callback type: `fn(search, event)`.
pub type SearchCallback = fn(&SsdpSearch, &SsdpEvent<'_>);

/// The only handle to an SSDP search; dropping it stops the search.
#[derive(Debug, PartialEq, Eq)]
pub struct SsdpSearch(SsdpSearchId);

impl SsdpSearch {
    /// Stop the search. An event already due in the same delivery pass is dropped.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for SsdpSearch {
    fn drop(&mut self) {
        SEARCHES.with_borrow_mut(|searches| searches.remove(&self.0));
        Platform::stop(self.0);
    }
}

/// Event delivered from the host for an SSDP search.
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
/// Returns `None` if the host refuses the search before it is queued.
#[must_use = "dropping the returned SsdpSearch stops it"]
pub fn ssdp_search(
    search_target: &str,
    timeout_secs: u32,
    callback: SearchCallback,
) -> Option<SsdpSearch> {
    let search_id = Platform::start(search_target, timeout_secs)?;
    let cb_idx = register_callback(callback);
    SEARCHES.with(|s| s.borrow_mut().insert(search_id, cb_idx));
    Some(SsdpSearch(search_id))
}

/// Hand an event to the callback of its search, if a handle still holds it.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "off-target no host delivers events")
)]
fn deliver(search_id: SsdpSearchId, event: &SsdpEvent<'_>) {
    let cb = SEARCHES
        .with(|s| s.borrow().get(&search_id).copied())
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        // Borrowed for the event only; dropping it would stop the search.
        cb(&ManuallyDrop::new(SsdpSearch(search_id)), event);
    }
}

trait Searcher {
    fn start(search_target: &str, timeout_secs: u32) -> Option<SsdpSearchId>;
    fn stop(search_id: SsdpSearchId);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_wasm_protocol::SsdpSearchId;

    use super::{SsdpEvent, deliver};

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_ssdp_search(st_ptr: *const u8, st_len: u32, timeout: u32) -> u32;
        fn host_ssdp_stop(search_id: u32);
    }

    pub(super) struct Host;

    impl super::Searcher for Host {
        fn start(search_target: &str, timeout_secs: u32) -> Option<SsdpSearchId> {
            SsdpSearchId::from_wire(unsafe {
                host_ssdp_search(
                    search_target.as_ptr(),
                    search_target.len() as u32,
                    timeout_secs,
                )
            })
        }

        fn stop(search_id: SsdpSearchId) {
            unsafe { host_ssdp_stop(search_id.to_wire()) }
        }
    }

    /// Called by the host when an SSDP event is ready.
    #[expect(
        clippy::same_length_and_capacity,
        reason = "the host hands back the size it asked __alloc for"
    )]
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_ssdp_event(
        search_id: u32,
        event_type: u32,
        data_ptr: u32,
        data_len: u32,
    ) {
        let Some(search_id) = SsdpSearchId::from_wire(search_id) else {
            return;
        };

        // Take ownership first, then borrow — avoids dangling reference.
        let owned = if data_len > 0 && data_ptr != 0 {
            unsafe {
                Vec::from_raw_parts(data_ptr as *mut u8, data_len as usize, data_len as usize)
            }
        } else {
            Vec::new()
        };
        let data = core::str::from_utf8(&owned).unwrap_or("");

        let event = match event_type {
            0 => SsdpEvent::Found(data),
            1 => SsdpEvent::Removed(data),
            _ => return,
        };

        deliver(search_id, &event);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_wasm_protocol::SsdpSearchId;

    pub(super) struct Host;

    impl super::Searcher for Host {
        fn start(_: &str, _: u32) -> Option<SsdpSearchId> {
            panic!("BUG: ssdp_search() called off-target — no host searches the network")
        }

        /// Only a test holds a handle off-target, and there is no host search to stop.
        fn stop(_: SsdpSearchId) {}
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use bmc_wasm_protocol::SsdpSearchId;

    use super::{SEARCHES, SearchCallback, SsdpEvent, SsdpSearch, register_callback};
    use crate::wasm_test;

    thread_local! {
        static SEEN: Cell<Option<SsdpSearchId>> = const { Cell::new(None) };
        static STORED: RefCell<Option<SsdpSearch>> = const { RefCell::new(None) };
    }

    fn search(callback: SearchCallback) -> SsdpSearch {
        let id = SsdpSearchId::from_wire(1).expect("BUG: test ids are non-zero");
        let cb_idx = register_callback(callback);
        SEARCHES.with_borrow_mut(|searches| searches.insert(id, cb_idx));
        SsdpSearch(id)
    }

    fn deliver(id: SsdpSearchId) {
        super::deliver(id, &SsdpEvent::Found(""));
    }

    fn is_registered(id: SsdpSearchId) -> bool {
        SEARCHES.with_borrow(|registered| registered.contains_key(&id))
    }

    fn record(search: &SsdpSearch, _event: &SsdpEvent<'_>) {
        SEEN.set(Some(search.0));
    }

    fn stop_stored(_search: &SsdpSearch, _event: &SsdpEvent<'_>) {
        if let Some(stored) = STORED.take() {
            stored.stop();
        }
    }

    #[wasm_test]
    fn an_event_reaches_its_callback_with_its_search() {
        let search = search(record);

        deliver(search.0);

        assert_eq!(SEEN.get(), Some(search.0));
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_test]
    fn a_find_from_the_host_reaches_its_search() {
        let search = search(record);

        super::wasm::__on_ssdp_event(search.0.to_wire(), 0, 0, 0);

        assert_eq!(SEEN.get(), Some(search.0));
    }

    #[wasm_test]
    fn an_event_leaves_its_search_running() {
        let search = search(record);

        deliver(search.0);

        assert!(is_registered(search.0));
    }

    #[wasm_test]
    fn a_dropped_handle_stops_its_events() {
        let search = search(record);
        let id = search.0;

        drop(search);
        deliver(id);

        assert_eq!(SEEN.get(), None);
    }

    #[wasm_test]
    fn a_callback_can_stop_its_own_search_through_the_stored_handle() {
        let search = search(stop_stored);
        let id = search.0;
        STORED.set(Some(search));

        deliver(id);

        assert!(!is_registered(id));
    }
}
