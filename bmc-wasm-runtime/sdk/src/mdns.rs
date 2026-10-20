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
//! An [`MdnsBrowse`] is the only handle to its browse,
//! and an [`MdnsRegistration`] the only one to its advertisement;
//! dropping either ends it, so a widget keeps the handle for as long as it wants the browse
//! or the advertisement.
//! The browse callback borrows the browse its event belongs to;
//! to stop the browse from there, take the stored handle.
//!
//! # Example
//!
//! ```ignore
//! use std::cell::RefCell;
//!
//! use bmc_wasm_sdk::*;
//! use bmc_wasm_sdk::mdns::*;
//!
//! thread_local! {
//!     static BROWSE: RefCell<Option<MdnsBrowse>> = const { RefCell::new(None) };
//! }
//!
//! #[unsafe(no_mangle)]
//! pub extern "C" fn init() {
//!     BROWSE.set(mdns_browse(&["_googlecast._tcp", "_touch-able._tcp"], on_mdns_event));
//! }
//!
//! fn on_mdns_event(_browse: &MdnsBrowse, event: &MdnsEvent<'_>) {
//!     match event {
//!         MdnsEvent::Found(json) => log_info!("found: {json}"),
//!         MdnsEvent::Removed(name) => log_info!("removed: {name}"),
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use bmc_wasm_protocol::{MdnsBrowseId, MdnsRegId};

#[cfg(target_arch = "wasm32")]
use crate::fmt;

/// Callback type: `fn(browse, event)`.
pub type BrowseCallback = fn(&MdnsBrowse, &MdnsEvent<'_>);

/// The only handle to an mDNS browse; dropping it stops the browse.
#[derive(Debug, PartialEq, Eq)]
pub struct MdnsBrowse(MdnsBrowseId);

impl MdnsBrowse {
    /// Stop the browse. An event already due in the same delivery pass is dropped.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for MdnsBrowse {
    fn drop(&mut self) {
        BROWSES.with_borrow_mut(|browses| browses.remove(&self.0));
        #[cfg(target_arch = "wasm32")]
        Platform::stop(self.0);
    }
}

/// The only handle to an mDNS service registration; dropping it unregisters the service.
#[derive(Debug, PartialEq, Eq)]
pub struct MdnsRegistration(MdnsRegId);

impl MdnsRegistration {
    /// Unregister the service.
    pub fn unregister(self) {
        drop(self);
    }
}

impl Drop for MdnsRegistration {
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        Platform::unregister(self.0);
    }
}

/// Event delivered from the host for an mDNS browse.
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

#[cfg(any(target_arch = "wasm32", test))]
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
/// Returns `None` if the host refuses the browse before it is queued.
#[must_use = "dropping the returned MdnsBrowse stops it"]
#[cfg(target_arch = "wasm32")]
pub fn mdns_browse(service_types: &[&str], callback: BrowseCallback) -> Option<MdnsBrowse> {
    let joined = service_types.join("\n");
    let browse_id = Platform::browse(&joined)?;
    let cb_idx = register_callback(callback);
    BROWSES.with(|b| b.borrow_mut().insert(browse_id, cb_idx));
    Some(MdnsBrowse(browse_id))
}

/// Register an mDNS service for advertisement on the local network.
///
/// TXT records are key-value pairs used for service metadata.
///
/// Returns `None` if the host refuses the registration.
#[must_use = "dropping the returned MdnsRegistration unregisters the service"]
#[cfg(target_arch = "wasm32")]
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
    Platform::register(service_type, name, port, &txt_str).map(MdnsRegistration)
}

/// Hand an event to the callback of its browse, if a handle still holds it.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "off-target no host delivers events")
)]
fn deliver(browse_id: MdnsBrowseId, event: &MdnsEvent<'_>) {
    let cb = BROWSES
        .with(|b| b.borrow().get(&browse_id).copied())
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        // Borrowed for the event only; dropping it would stop the browse.
        cb(&ManuallyDrop::new(MdnsBrowse(browse_id)), event);
    }
}

#[cfg(target_arch = "wasm32")]
trait MdnsDaemon {
    fn browse(service_types: &str) -> Option<MdnsBrowseId>;
    fn stop(browse_id: MdnsBrowseId);
    fn register(service_type: &str, name: &str, port: u16, txt: &str) -> Option<MdnsRegId>;
    fn unregister(reg_id: MdnsRegId);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_wasm_protocol::{MdnsBrowseId, MdnsRegId};

    use super::{MdnsEvent, deliver};

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

    pub(super) struct Host;

    impl super::MdnsDaemon for Host {
        fn browse(service_types: &str) -> Option<MdnsBrowseId> {
            MdnsBrowseId::from_wire(unsafe {
                host_mdns_browse(service_types.as_ptr(), crate::ffi_u32(service_types.len()))
            })
        }

        fn stop(browse_id: MdnsBrowseId) {
            unsafe { host_mdns_stop(browse_id.to_wire()) }
        }

        fn register(service_type: &str, name: &str, port: u16, txt: &str) -> Option<MdnsRegId> {
            MdnsRegId::from_wire(unsafe {
                host_mdns_register(
                    service_type.as_ptr(),
                    crate::ffi_u32(service_type.len()),
                    name.as_ptr(),
                    crate::ffi_u32(name.len()),
                    u32::from(port),
                    txt.as_ptr(),
                    crate::ffi_u32(txt.len()),
                )
            })
        }

        fn unregister(reg_id: MdnsRegId) {
            unsafe { host_mdns_unregister(reg_id.to_wire()) }
        }
    }

    /// Called by the host when an mDNS event is ready.
    #[expect(
        clippy::same_length_and_capacity,
        reason = "the host hands back the size it asked __alloc for"
    )]
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_mdns_event(
        browse_id: u32,
        event_type: u32,
        data_ptr: u32,
        data_len: u32,
    ) {
        let Some(browse_id) = MdnsBrowseId::from_wire(browse_id) else {
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
            0 => MdnsEvent::Found(data),
            1 => MdnsEvent::Removed(data),
            _ => return,
        };

        deliver(browse_id, &event);
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use bmc_wasm_protocol::MdnsBrowseId;

    use super::{BROWSES, BrowseCallback, MdnsBrowse, MdnsEvent, register_callback};
    use crate::wasm_test;

    thread_local! {
        static SEEN: Cell<Option<MdnsBrowseId>> = const { Cell::new(None) };
        static STORED: RefCell<Option<MdnsBrowse>> = const { RefCell::new(None) };
    }

    fn browse(callback: BrowseCallback) -> MdnsBrowse {
        let id = MdnsBrowseId::from_wire(1).expect("BUG: test ids are non-zero");
        let cb_idx = register_callback(callback);
        BROWSES.with_borrow_mut(|browses| browses.insert(id, cb_idx));
        MdnsBrowse(id)
    }

    fn deliver(id: MdnsBrowseId) {
        super::deliver(id, &MdnsEvent::Found(""));
    }

    fn is_registered(id: MdnsBrowseId) -> bool {
        BROWSES.with_borrow(|registered| registered.contains_key(&id))
    }

    fn record(browse: &MdnsBrowse, _event: &MdnsEvent<'_>) {
        SEEN.set(Some(browse.0));
    }

    fn stop_stored(_browse: &MdnsBrowse, _event: &MdnsEvent<'_>) {
        if let Some(stored) = STORED.take() {
            stored.stop();
        }
    }

    #[wasm_test]
    fn an_event_reaches_its_callback_with_its_browse() {
        let browse = browse(record);

        deliver(browse.0);

        assert_eq!(SEEN.get(), Some(browse.0));
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_test]
    fn a_find_from_the_host_reaches_its_browse() {
        let browse = browse(record);

        super::wasm::__on_mdns_event(browse.0.to_wire(), 0, 0, 0);

        assert_eq!(SEEN.get(), Some(browse.0));
    }

    #[wasm_test]
    fn an_event_leaves_its_browse_running() {
        let browse = browse(record);

        deliver(browse.0);

        assert!(is_registered(browse.0));
    }

    #[wasm_test]
    fn a_dropped_handle_stops_its_events() {
        let browse = browse(record);
        let id = browse.0;

        drop(browse);
        deliver(id);

        assert_eq!(SEEN.get(), None);
    }

    #[wasm_test]
    fn a_callback_can_stop_its_own_browse_through_the_stored_handle() {
        let browse = browse(stop_stored);
        let id = browse.0;
        STORED.set(Some(browse));

        deliver(id);

        assert!(!is_registered(id));
    }
}
