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

//! UDP broadcast discovery for WASM widgets.
//!
//! Provides `udp_broadcast()` for sending a UDP broadcast message and receiving
//! responses from devices on the local network. The host manages the broadcast
//! in a background thread and delivers events by calling the
//! `__on_udp_broadcast_event` export.
//!
//! A [`UdpBroadcast`] is the only handle to its broadcast,
//! and dropping it stops the broadcast, resends included,
//! so a widget keeps the handle for as long as it wants responses.
//! The callback borrows the broadcast its event belongs to;
//! to stop the broadcast from there, take the stored handle.
//!
//! Off-target there is no host to send broadcasts, so starting one panics.
//!
//! # Example
//!
//! ```ignore
//! use std::cell::RefCell;
//!
//! use bmc_wasm_sdk::*;
//! use bmc_wasm_sdk::udp_broadcast::*;
//!
//! thread_local! {
//!     static DISCOVERY: RefCell<Option<UdpBroadcast>> = const { RefCell::new(None) };
//! }
//!
//! #[unsafe(no_mangle)]
//! pub extern "C" fn init() {
//!     DISCOVERY.set(udp_broadcast(7359, "Who is JellyfinServer?", 5, on_response));
//! }
//!
//! fn on_response(_broadcast: &UdpBroadcast, event: &UdpBroadcastEvent<'_>) {
//!     let UdpBroadcastEvent::Response { data, source } = event;
//!     log_info!("{source} answered: {data}");
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use bmc_wasm_protocol::UdpBroadcastId;

/// Callback type: `fn(broadcast, event)`.
pub type BroadcastCallback = fn(&UdpBroadcast, &UdpBroadcastEvent<'_>);

/// The only handle to a UDP broadcast; dropping it stops the broadcast.
#[derive(Debug, PartialEq, Eq)]
pub struct UdpBroadcast(UdpBroadcastId);

impl UdpBroadcast {
    /// Stop the broadcast. An event already due in the same delivery pass is dropped.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for UdpBroadcast {
    fn drop(&mut self) {
        BROADCASTS.with_borrow_mut(|broadcasts| broadcasts.remove(&self.0));
        Platform::stop(self.0);
    }
}

/// Event delivered from the host for a UDP broadcast.
#[derive(Debug)]
pub enum UdpBroadcastEvent<'a> {
    /// Response received: data payload and source address string.
    Response { data: &'a str, source: &'a str },
}

thread_local! {
    static CALLBACKS: RefCell<Vec<BroadcastCallback>> = const { RefCell::new(Vec::new()) };
    static BROADCASTS: RefCell<HashMap<UdpBroadcastId, usize>> = RefCell::new(HashMap::new());
}

fn register_callback(cb: BroadcastCallback) -> usize {
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

/// Send a UDP broadcast message to the given port and receive responses.
///
/// The `message` is sent as a UDP broadcast to `255.255.255.255:port`.
/// Responses are delivered via the callback. The host thread resends
/// periodically (every 30s) and listens for `timeout_secs` after each send.
///
/// Returns `None` if the host refuses the broadcast before it is queued.
#[must_use = "dropping the returned UdpBroadcast stops it"]
pub fn udp_broadcast(
    port: u32,
    message: &str,
    timeout_secs: u32,
    callback: BroadcastCallback,
) -> Option<UdpBroadcast> {
    let broadcast_id = Platform::start(port, message, timeout_secs)?;
    let cb_idx = register_callback(callback);
    BROADCASTS.with(|b| b.borrow_mut().insert(broadcast_id, cb_idx));
    Some(UdpBroadcast(broadcast_id))
}

/// Hand an event to the callback of its broadcast, if a handle still holds it.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "off-target no host delivers events")
)]
fn deliver(broadcast_id: UdpBroadcastId, data: &str, source: &str) {
    let event = UdpBroadcastEvent::Response { data, source };

    let cb = BROADCASTS
        .with(|b| b.borrow().get(&broadcast_id).copied())
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        // Borrowed for the event only; dropping it would stop the broadcast.
        cb(&ManuallyDrop::new(UdpBroadcast(broadcast_id)), &event);
    }
}

trait Broadcaster {
    fn start(port: u32, message: &str, timeout_secs: u32) -> Option<UdpBroadcastId>;
    fn stop(broadcast_id: UdpBroadcastId);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_wasm_protocol::UdpBroadcastId;

    use super::deliver;

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_udp_broadcast(
            port: u32,
            msg_ptr: *const u8,
            msg_len: u32,
            timeout_secs: u32,
        ) -> u32;
        fn host_udp_broadcast_stop(broadcast_id: u32);
    }

    pub(super) struct Host;

    impl super::Broadcaster for Host {
        fn start(port: u32, message: &str, timeout_secs: u32) -> Option<UdpBroadcastId> {
            UdpBroadcastId::from_wire(unsafe {
                host_udp_broadcast(port, message.as_ptr(), message.len() as u32, timeout_secs)
            })
        }

        fn stop(broadcast_id: UdpBroadcastId) {
            unsafe { host_udp_broadcast_stop(broadcast_id.to_wire()) }
        }
    }

    /// Called by the host when a UDP broadcast event is ready.
    #[expect(
        clippy::same_length_and_capacity,
        reason = "the host hands back the sizes it asked __alloc for"
    )]
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_udp_broadcast_event(
        broadcast_id: u32,
        data_ptr: u32,
        data_len: u32,
        source_ptr: u32,
        source_len: u32,
    ) {
        let Some(broadcast_id) = UdpBroadcastId::from_wire(broadcast_id) else {
            return;
        };

        // Take ownership first, then borrow — avoids dangling reference.
        let owned_data = if data_len > 0 && data_ptr != 0 {
            unsafe {
                Vec::from_raw_parts(data_ptr as *mut u8, data_len as usize, data_len as usize)
            }
        } else {
            Vec::new()
        };
        let owned_source = if source_len > 0 && source_ptr != 0 {
            unsafe {
                Vec::from_raw_parts(
                    source_ptr as *mut u8,
                    source_len as usize,
                    source_len as usize,
                )
            }
        } else {
            Vec::new()
        };
        let data = core::str::from_utf8(&owned_data).unwrap_or("");
        let source = core::str::from_utf8(&owned_source).unwrap_or("");

        deliver(broadcast_id, data, source);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_wasm_protocol::UdpBroadcastId;

    pub(super) struct Host;

    impl super::Broadcaster for Host {
        fn start(_: u32, _: &str, _: u32) -> Option<UdpBroadcastId> {
            panic!("BUG: udp_broadcast() called off-target — no host sends broadcasts")
        }

        /// Only a test holds a handle off-target, and there is no host broadcast to stop.
        fn stop(_: UdpBroadcastId) {}
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use bmc_wasm_protocol::UdpBroadcastId;

    use super::{
        BROADCASTS, BroadcastCallback, UdpBroadcast, UdpBroadcastEvent, register_callback,
    };
    use crate::wasm_test;

    thread_local! {
        static SEEN: Cell<Option<UdpBroadcastId>> = const { Cell::new(None) };
        static STORED: RefCell<Option<UdpBroadcast>> = const { RefCell::new(None) };
    }

    fn broadcast(callback: BroadcastCallback) -> UdpBroadcast {
        let id = UdpBroadcastId::from_wire(1).expect("BUG: test ids are non-zero");
        let cb_idx = register_callback(callback);
        BROADCASTS.with_borrow_mut(|broadcasts| broadcasts.insert(id, cb_idx));
        UdpBroadcast(id)
    }

    fn deliver(id: UdpBroadcastId) {
        super::deliver(id, "", "");
    }

    fn is_registered(id: UdpBroadcastId) -> bool {
        BROADCASTS.with_borrow(|registered| registered.contains_key(&id))
    }

    fn record(broadcast: &UdpBroadcast, _event: &UdpBroadcastEvent<'_>) {
        SEEN.set(Some(broadcast.0));
    }

    fn stop_stored(_broadcast: &UdpBroadcast, _event: &UdpBroadcastEvent<'_>) {
        if let Some(stored) = STORED.take() {
            stored.stop();
        }
    }

    #[wasm_test]
    fn an_event_reaches_its_callback_with_its_broadcast() {
        let broadcast = broadcast(record);

        deliver(broadcast.0);

        assert_eq!(SEEN.get(), Some(broadcast.0));
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_test]
    fn a_datagram_from_the_host_reaches_its_broadcast() {
        let broadcast = broadcast(record);

        super::wasm::__on_udp_broadcast_event(broadcast.0.to_wire(), 0, 0, 0, 0);

        assert_eq!(SEEN.get(), Some(broadcast.0));
    }

    #[wasm_test]
    fn an_event_leaves_its_broadcast_running() {
        let broadcast = broadcast(record);

        deliver(broadcast.0);

        assert!(is_registered(broadcast.0));
    }

    #[wasm_test]
    fn a_dropped_handle_stops_its_events() {
        let broadcast = broadcast(record);
        let id = broadcast.0;

        drop(broadcast);
        deliver(id);

        assert_eq!(SEEN.get(), None);
    }

    #[wasm_test]
    fn a_callback_can_stop_its_own_broadcast_through_the_stored_handle() {
        let broadcast = broadcast(stop_stored);
        let id = broadcast.0;
        STORED.set(Some(broadcast));

        deliver(id);

        assert!(!is_registered(id));
    }
}
