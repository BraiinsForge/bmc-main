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

//! WebSocket support for WASM widgets.
//!
//! Provides persistent bidirectional connections to external services.
//! The host manages the actual TCP/TLS connection in a background thread
//! and delivers events by calling the `__on_ws_event` export each frame.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::*;
//! use bmc_wasm_sdk::ws::{Ws, WsEvent, ws_connect};
//!
//! fn init(width: u32, height: u32) {
//!     ws_connect("ws://192.168.1.10:8123/api/websocket", None, on_event);
//! }
//!
//! fn on_event(ws: Ws, event: &WsEvent) {
//!     match event {
//!         WsEvent::Open => ws.send(r#"{"type":"auth","access_token":"..."}"#),
//!         WsEvent::Message(text) => { /* handle message */ }
//!         WsEvent::Close(code) => { /* handle close */ }
//!     }
//! }
//! ```

use core::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_protocol::WebsocketId;

// Host function imports
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_ws_connect(
        url_ptr: *const u8,
        url_len: u32,
        headers_ptr: *const u8,
        headers_len: u32,
    ) -> u32;
    fn host_ws_send(ws_id: u32, msg_ptr: *const u8, msg_len: u32) -> u32;
    fn host_ws_close(ws_id: u32);
}

/// Handle to an active WebSocket connection.
#[derive(Debug, Clone, Copy)]
pub struct Ws(pub(crate) WebsocketId);

impl Ws {
    /// Send a text message over this connection.
    pub fn send(&self, message: &str) {
        unsafe {
            host_ws_send(self.0.to_wire(), message.as_ptr(), message.len() as u32);
        }
    }

    /// Close this connection.
    pub fn close(&self) {
        unsafe { host_ws_close(self.0.to_wire()) }
    }
}

/// Events delivered by the host for a WebSocket connection.
#[derive(Debug)]
pub enum WsEvent<'a> {
    /// Connection successfully opened.
    Open,
    /// A text message was received.
    Message(&'a str),
    /// Connection closed with a status code.
    Close(u16),
}

type Callback = fn(Ws, &WsEvent<'_>);

thread_local! {
    /// Registered callbacks indexed by position.
    static CALLBACKS: RefCell<Vec<Callback>> = const { RefCell::new(Vec::new()) };
    /// Maps ws_id → callback index.
    static CONNECTIONS: RefCell<HashMap<WebsocketId, usize>> = RefCell::new(HashMap::new());
}

/// Register a callback and return its index, reusing existing slots.
fn register_callback(cb: Callback) -> usize {
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

/// Open a WebSocket connection to `url`.
///
/// The host connects in a background thread and delivers events by calling
/// `on_event` with a [`Ws`] handle and a [`WsEvent`].
///
/// `headers` is an optional newline-separated list of `Key: Value` pairs
/// (same format as [`fetch`](crate::fetch)).
///
/// Returns `None` if the host rejects the connection before it is queued.
#[must_use]
pub fn ws_connect(url: &str, headers: Option<&str>, on_event: Callback) -> Option<Ws> {
    let cb_idx = register_callback(on_event);
    let (h_ptr, h_len) = match headers {
        Some(h) => (h.as_ptr(), h.len() as u32),
        None => (core::ptr::null(), 0),
    };
    let ws_id = WebsocketId::from_wire(unsafe {
        host_ws_connect(url.as_ptr(), url.len() as u32, h_ptr, h_len)
    })?;
    CONNECTIONS.with(|c| c.borrow_mut().insert(ws_id, cb_idx));
    Some(Ws(ws_id))
}

/// Called by the host when a WebSocket event occurs.
///
/// Event types:
/// - 0 = Open (data empty)
/// - 1 = Message (data = UTF-8 text)
/// - 2 = Close (data = 2 bytes LE close code)
#[unsafe(no_mangle)]
pub extern "C" fn __on_ws_event(ws_id: u32, event_type: u32, data_ptr: u32, data_len: u32) {
    let Some(ws_id) = WebsocketId::from_wire(ws_id) else {
        return;
    };

    let data = if data_len > 0 && data_ptr != 0 {
        unsafe { Vec::from_raw_parts(data_ptr as *mut u8, data_len as usize, data_len as usize) }
    } else {
        Vec::new()
    };

    let event = match event_type {
        0 => WsEvent::Open,
        1 => {
            let text = core::str::from_utf8(&data).unwrap_or_default();
            WsEvent::Message(text)
        }
        2 => {
            let code = if data.len() >= 2 {
                u16::from_le_bytes([data[0], data[1]])
            } else {
                1006
            };
            WsEvent::Close(code)
        }
        _ => return,
    };

    let is_close = matches!(event, WsEvent::Close(_));
    let ws = Ws(ws_id);

    // Copy the callback out before invoking (callback may call ws_connect)
    let cb = CONNECTIONS
        .with(|c| {
            if is_close {
                c.borrow_mut().remove(&ws_id)
            } else {
                c.borrow().get(&ws_id).copied()
            }
        })
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        cb(ws, &event);
    }
}
