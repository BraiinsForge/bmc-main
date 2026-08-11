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

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_protocol::UdpBroadcastId;

// Host function imports
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_udp_broadcast(port: u32, msg_ptr: *const u8, msg_len: u32, timeout_secs: u32) -> u32;
    fn host_udp_broadcast_stop(broadcast_id: u32);
}

/// Callback type: `fn(broadcast, event)`.
pub type BroadcastCallback = fn(UdpBroadcast, &UdpBroadcastEvent<'_>);

/// Handle to an active UDP broadcast session.
#[derive(Clone, Copy, Debug)]
pub struct UdpBroadcast(pub UdpBroadcastId);

impl UdpBroadcast {
    /// Stop this broadcast session.
    pub fn stop(&self) {
        unsafe { host_udp_broadcast_stop(self.0.to_wire()) }
    }
}

/// Event delivered from the host for a UDP broadcast session.
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
/// Returns `None` if the host rejects the broadcast before it is queued.
#[must_use]
pub fn udp_broadcast(
    port: u32,
    message: &str,
    timeout_secs: u32,
    callback: BroadcastCallback,
) -> Option<UdpBroadcast> {
    let cb_idx = register_callback(callback);
    let broadcast_id = UdpBroadcastId::from_wire(unsafe {
        host_udp_broadcast(port, message.as_ptr(), message.len() as u32, timeout_secs)
    })?;
    BROADCASTS.with(|b| b.borrow_mut().insert(broadcast_id, cb_idx));
    Some(UdpBroadcast(broadcast_id))
}

/// Called by the host when a UDP broadcast event is ready.
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
        unsafe { Vec::from_raw_parts(data_ptr as *mut u8, data_len as usize, data_len as usize) }
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

    let event = UdpBroadcastEvent::Response { data, source };
    let broadcast = UdpBroadcast(broadcast_id);

    let cb = BROADCASTS
        .with(|b| b.borrow().get(&broadcast_id).copied())
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        cb(broadcast, &event);
    }
}
