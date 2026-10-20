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
//! A [`Ws`] is the only handle to its connection,
//! and dropping it closes the connection,
//! so a widget keeps the handle for as long as it wants the connection.
//! The callback borrows the connection its event belongs to;
//! to close the connection from there, take the stored handle.
//! When the peer closes the connection, the callback gets [`WsEvent::Close`]
//! and the stored handle reaches nothing, so clear or replace it there.
//!
//! # Example
//!
//! ```ignore
//! use std::cell::RefCell;
//!
//! use bmc_wasm_sdk::ws::{Ws, WsEvent, ws_connect};
//!
//! thread_local! {
//!     static WS: RefCell<Option<Ws>> = const { RefCell::new(None) };
//! }
//!
//! #[unsafe(no_mangle)]
//! pub extern "C" fn init() {
//!     WS.set(ws_connect("ws://192.168.1.10:8123/api/websocket", None, on_event));
//! }
//!
//! fn on_event(ws: &Ws, event: &WsEvent<'_>) {
//!     match event {
//!         WsEvent::Open => ws.send(r#"{"type":"auth","access_token":"..."}"#),
//!         WsEvent::Message(r#"{"type":"auth_invalid"}"#) => {
//!             if let Some(ws) = WS.take() {
//!                 ws.close();
//!             }
//!         }
//!         WsEvent::Message(_) => { /* handle message */ }
//!         WsEvent::Close(_) => WS.set(None),
//!     }
//! }
//! ```

use core::cell::RefCell;
use core::mem::ManuallyDrop;
use std::collections::HashMap;

use bmc_wasm_protocol::WebsocketId;

/// The only handle to a WebSocket connection; dropping it closes the connection.
#[derive(Debug, PartialEq, Eq)]
pub struct Ws(WebsocketId);

impl Ws {
    /// Send a text message over this connection.
    ///
    /// Does nothing once the connection has closed,
    /// such as on a callback's argument after the callback closed the stored handle.
    #[cfg(target_arch = "wasm32")]
    pub fn send(&self, message: &str) {
        Platform::send(self.0, message);
    }

    /// Close the connection. An event already due in the same delivery pass is dropped.
    pub fn close(self) {
        drop(self);
    }
}

impl Drop for Ws {
    fn drop(&mut self) {
        CONNECTIONS.with_borrow_mut(|connections| connections.remove(&self.0));
        #[cfg(target_arch = "wasm32")]
        Platform::close(self.0);
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
    ///
    /// The widget's stored handle reaches nothing from here on; clear or replace it.
    Close(u16),
}

/// Callback type: `fn(ws, event)`.
pub type Callback = fn(&Ws, &WsEvent<'_>);

thread_local! {
    /// Registered callbacks indexed by position.
    static CALLBACKS: RefCell<Vec<Callback>> = const { RefCell::new(Vec::new()) };
    /// Maps ws_id → callback index.
    static CONNECTIONS: RefCell<HashMap<WebsocketId, usize>> = RefCell::new(HashMap::new());
}

/// Register a callback and return its index, reusing existing slots.
#[cfg(any(target_arch = "wasm32", test))]
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
/// `on_event` with the borrowed [`Ws`] and a [`WsEvent`].
///
/// `headers` is an optional newline-separated list of `Key: Value` pairs
/// (same format as [`fetch`](crate::fetch())).
///
/// Returns `None` if the host refuses the connection before it is queued.
#[must_use = "dropping the returned Ws closes it"]
#[cfg(target_arch = "wasm32")]
pub fn ws_connect(url: &str, headers: Option<&str>, on_event: Callback) -> Option<Ws> {
    let ws_id = Platform::connect(url, headers)?;
    let cb_idx = register_callback(on_event);
    CONNECTIONS.with(|c| c.borrow_mut().insert(ws_id, cb_idx));
    Some(Ws(ws_id))
}

/// Hand an event to the callback of its connection, if a handle still holds it.
///
/// A close unregisters the connection before the callback runs.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "off-target no host delivers events")
)]
fn deliver(ws_id: WebsocketId, event: &WsEvent<'_>) {
    let is_close = matches!(event, WsEvent::Close(_));

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
        // Borrowed for the event only; dropping it would close the connection.
        cb(&ManuallyDrop::new(Ws(ws_id)), event);
    }
}

#[cfg(target_arch = "wasm32")]
trait WebsocketClient {
    fn connect(url: &str, headers: Option<&str>) -> Option<WebsocketId>;
    fn send(ws_id: WebsocketId, message: &str);
    fn close(ws_id: WebsocketId);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_wasm_protocol::WebsocketId;

    use super::{WsEvent, deliver};

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

    pub(super) struct Host;

    impl super::WebsocketClient for Host {
        fn connect(url: &str, headers: Option<&str>) -> Option<WebsocketId> {
            let (h_ptr, h_len) = match headers {
                Some(h) => (h.as_ptr(), crate::ffi_u32(h.len())),
                None => (core::ptr::null(), 0),
            };
            WebsocketId::from_wire(unsafe {
                host_ws_connect(url.as_ptr(), crate::ffi_u32(url.len()), h_ptr, h_len)
            })
        }

        fn send(ws_id: WebsocketId, message: &str) {
            unsafe {
                host_ws_send(
                    ws_id.to_wire(),
                    message.as_ptr(),
                    crate::ffi_u32(message.len()),
                );
            }
        }

        fn close(ws_id: WebsocketId) {
            unsafe { host_ws_close(ws_id.to_wire()) }
        }
    }

    /// Called by the host when a WebSocket event occurs.
    ///
    /// Event types:
    /// - 0 = Open (data empty)
    /// - 1 = Message (data = UTF-8 text)
    /// - 2 = Close (data = 2 bytes LE close code)
    #[expect(
        clippy::same_length_and_capacity,
        reason = "the host hands back the size it asked __alloc for"
    )]
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_ws_event(ws_id: u32, event_type: u32, data_ptr: u32, data_len: u32) {
        let Some(ws_id) = WebsocketId::from_wire(ws_id) else {
            return;
        };

        let data = if data_len > 0 && data_ptr != 0 {
            unsafe {
                Vec::from_raw_parts(data_ptr as *mut u8, data_len as usize, data_len as usize)
            }
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

        deliver(ws_id, &event);
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use bmc_wasm_protocol::WebsocketId;

    use super::{CONNECTIONS, Callback, Ws, WsEvent, register_callback};
    use crate::wasm_test;

    thread_local! {
        static SEEN: Cell<Option<WebsocketId>> = const { Cell::new(None) };
        static STORED: RefCell<Option<Ws>> = const { RefCell::new(None) };
    }

    fn connect(callback: Callback) -> Ws {
        let id = WebsocketId::from_wire(1).expect("BUG: test ids are non-zero");
        let cb_idx = register_callback(callback);
        CONNECTIONS.with_borrow_mut(|connections| connections.insert(id, cb_idx));
        Ws(id)
    }

    fn deliver(id: WebsocketId) {
        super::deliver(id, &WsEvent::Open);
    }

    fn is_registered(id: WebsocketId) -> bool {
        CONNECTIONS.with_borrow(|registered| registered.contains_key(&id))
    }

    fn record(ws: &Ws, _event: &WsEvent<'_>) {
        SEEN.set(Some(ws.0));
    }

    fn close_stored(_ws: &Ws, _event: &WsEvent<'_>) {
        if let Some(stored) = STORED.take() {
            stored.close();
        }
    }

    #[wasm_test]
    fn an_event_reaches_its_callback_with_its_connection() {
        let ws = connect(record);

        deliver(ws.0);

        assert_eq!(SEEN.get(), Some(ws.0));
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_test]
    fn an_open_from_the_host_reaches_its_connection() {
        let ws = connect(record);

        super::wasm::__on_ws_event(ws.0.to_wire(), 0, 0, 0);

        assert_eq!(SEEN.get(), Some(ws.0));
    }

    #[wasm_test]
    fn an_event_leaves_its_connection_open() {
        let ws = connect(record);

        deliver(ws.0);

        assert!(is_registered(ws.0));
    }

    #[wasm_test]
    fn a_dropped_handle_stops_its_events() {
        let ws = connect(record);
        let id = ws.0;

        drop(ws);
        deliver(id);

        assert_eq!(SEEN.get(), None);
    }

    #[wasm_test]
    fn a_callback_can_close_its_own_connection_through_the_stored_handle() {
        let ws = connect(close_stored);
        let id = ws.0;
        STORED.set(Some(ws));

        deliver(id);

        assert!(!is_registered(id));
    }
}
