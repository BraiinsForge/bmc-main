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

//! HTTP listener for WASM widgets.
//!
//! Provides `http_listen()` for accepting inbound HTTP connections. The host
//! manages the TCP listener in a background thread and delivers requests by
//! calling the `__on_http_request` export. WASM responds via `HttpRequest::respond()`.
//!
//! This is needed for protocols like DACP where the remote device (iTunes/Music)
//! connects TO the client during pairing.
//!
//! An [`HttpListener`] is the only handle to its listener,
//! and dropping it closes the listener,
//! so a widget keeps the handle for as long as it wants requests.
//! The callback borrows the listener its request arrived on;
//! to close the listener from there, take the stored handle.
//!
//! Off-target there is no host to listen with,
//! so listening, reading the port or responding panics.
//!
//! # Example
//!
//! ```ignore
//! use std::cell::RefCell;
//!
//! use bmc_wasm_sdk::*;
//! use bmc_wasm_sdk::http_listener::*;
//!
//! thread_local! {
//!     static LISTENER: RefCell<Option<HttpListener>> = const { RefCell::new(None) };
//! }
//!
//! fn start_pairing() {
//!     let listener = http_listen(0, on_request); // port 0 = ephemeral
//!     if let Some(listener) = &listener {
//!         log_info!("listening on port {}", listener.port());
//!     }
//!     LISTENER.set(listener);
//! }
//!
//! fn on_request(_listener: &HttpListener, req: &HttpRequest) {
//!     log_info!("{} {}", req.method, req.path);
//!     req.respond(200, "Content-Type: text/plain", b"OK");
//!     if req.path == "/paired" {
//!         if let Some(listener) = LISTENER.take() {
//!             listener.close();
//!         }
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;

use bmc_wasm_protocol::{HttpListenerId, HttpRequestId};

/// Callback type: `fn(listener, request)`.
pub type RequestCallback = fn(&HttpListener, &HttpRequest);

/// The only handle to an HTTP listener; dropping it closes the listener.
#[derive(Debug, PartialEq, Eq)]
pub struct HttpListener(HttpListenerId);

impl HttpListener {
    /// The bound port, which the host picks when the listener asked for port 0.
    ///
    /// Returns 0 once the listener is closed,
    /// such as on a callback's argument after the callback closed the stored handle.
    #[must_use]
    pub fn port(&self) -> u16 {
        Platform::port(self.0)
    }

    /// Close the listener. A request already due in the same delivery pass is dropped.
    pub fn close(self) {
        drop(self);
    }
}

impl Drop for HttpListener {
    fn drop(&mut self) {
        LISTENERS.with_borrow_mut(|listeners| listeners.remove(&self.0));
        Platform::close(self.0);
    }
}

/// An inbound HTTP request delivered from the host.
#[derive(Debug)]
pub struct HttpRequest {
    pub request_id: HttpRequestId,
    pub method: String,
    pub path: String,
    pub headers: String,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Send an HTTP response for this request.
    ///
    /// Headers should be newline-delimited `"Key: Value"` pairs.
    pub fn respond(&self, status: u16, headers: &str, body: &[u8]) {
        Platform::respond(self.request_id, status, headers, body);
    }
}

thread_local! {
    static CALLBACKS: RefCell<Vec<RequestCallback>> = const { RefCell::new(Vec::new()) };
    static LISTENERS: RefCell<HashMap<HttpListenerId, usize>> = RefCell::new(HashMap::new());
}

fn register_callback(cb: RequestCallback) -> usize {
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

/// Start listening for inbound HTTP connections.
///
/// Use `port = 0` for an ephemeral port (check `listener.port()` after).
/// The `callback` is invoked for each inbound request.
///
/// Returns `None` if the host refuses the listener or cannot bind it.
#[must_use = "dropping the returned HttpListener closes it"]
pub fn http_listen(port: u16, callback: RequestCallback) -> Option<HttpListener> {
    let listener_id = Platform::listen(port)?;
    let cb_idx = register_callback(callback);
    LISTENERS.with_borrow_mut(|listeners| listeners.insert(listener_id, cb_idx));
    Some(HttpListener(listener_id))
}

/// Hand a request to the callback of its listener, if a handle still holds it.
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(dead_code, reason = "off-target no host delivers requests")
)]
fn deliver(listener_id: HttpListenerId, req: &HttpRequest) {
    let cb = LISTENERS
        .with(|l| l.borrow().get(&listener_id).copied())
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        // Borrowed for the request only; dropping it would close the listener.
        cb(&ManuallyDrop::new(HttpListener(listener_id)), req);
    }
}

trait HttpServer {
    fn listen(port: u16) -> Option<HttpListenerId>;
    fn port(listener_id: HttpListenerId) -> u16;
    fn close(listener_id: HttpListenerId);
    fn respond(request_id: HttpRequestId, status: u16, headers: &str, body: &[u8]);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_wasm_protocol::{HttpListenerId, HttpRequestId};

    use super::{HttpRequest, deliver};

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_http_listen(port: u32) -> u32;
        fn host_http_respond(
            request_id: u32,
            status: u32,
            headers_ptr: *const u8,
            headers_len: u32,
            body_ptr: *const u8,
            body_len: u32,
        );
        fn host_http_close_listener(listener_id: u32);
        fn host_http_get_port(listener_id: u32) -> u32;
    }

    pub(super) struct Host;

    impl super::HttpServer for Host {
        fn listen(port: u16) -> Option<HttpListenerId> {
            HttpListenerId::from_wire(unsafe { host_http_listen(u32::from(port)) })
        }

        fn port(listener_id: HttpListenerId) -> u16 {
            let port = unsafe { host_http_get_port(listener_id.to_wire()) };
            u16::try_from(port).expect("BUG: the host reports a TCP port, which fits u16")
        }

        fn close(listener_id: HttpListenerId) {
            unsafe { host_http_close_listener(listener_id.to_wire()) }
        }

        fn respond(request_id: HttpRequestId, status: u16, headers: &str, body: &[u8]) {
            unsafe {
                host_http_respond(
                    request_id.to_wire(),
                    u32::from(status),
                    headers.as_ptr(),
                    crate::ffi_u32(headers.len()),
                    body.as_ptr(),
                    crate::ffi_u32(body.len()),
                );
            }
        }
    }

    /// Called by the host when an inbound HTTP request is ready.
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_http_request(
        listener_id: u32,
        request_id: u32,
        method_ptr: u32,
        method_len: u32,
        path_ptr: u32,
        path_len: u32,
        headers_ptr: u32,
        headers_len: u32,
        body_ptr: u32,
        body_len: u32,
    ) {
        let Some(listener_id) = HttpListenerId::from_wire(listener_id) else {
            return;
        };
        let Some(request_id) = HttpRequestId::from_wire(request_id) else {
            return;
        };

        let method = take_string(method_ptr, method_len);
        let path = take_string(path_ptr, path_len);
        let headers = take_string(headers_ptr, headers_len);
        let body = take_vec(body_ptr, body_len);

        let req = HttpRequest {
            request_id,
            method,
            path,
            headers,
            body,
        };

        deliver(listener_id, &req);
    }

    /// Take ownership of a host-allocated string buffer.
    #[expect(
        clippy::same_length_and_capacity,
        reason = "the host hands back the size it asked __alloc for"
    )]
    fn take_string(ptr: u32, len: u32) -> String {
        if len == 0 || ptr == 0 {
            return String::new();
        }
        let bytes = unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) };
        String::from_utf8(bytes).unwrap_or_default()
    }

    /// Take ownership of a host-allocated byte buffer.
    #[expect(
        clippy::same_length_and_capacity,
        reason = "the host hands back the size it asked __alloc for"
    )]
    fn take_vec(ptr: u32, len: u32) -> Vec<u8> {
        if len == 0 || ptr == 0 {
            return Vec::new();
        }
        unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_wasm_protocol::{HttpListenerId, HttpRequestId};

    pub(super) struct Host;

    impl super::HttpServer for Host {
        fn listen(_: u16) -> Option<HttpListenerId> {
            panic!("BUG: http_listen() called off-target — no host to listen with")
        }

        fn port(_: HttpListenerId) -> u16 {
            panic!("BUG: HttpListener::port() called off-target — no host to listen with")
        }

        /// Only a test holds a handle off-target, and there is no host listener to close.
        fn close(_: HttpListenerId) {}

        fn respond(_: HttpRequestId, _: u16, _: &str, _: &[u8]) {
            panic!("BUG: HttpRequest::respond() called off-target — no host to listen with")
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use bmc_wasm_protocol::{HttpListenerId, HttpRequestId};

    use super::{HttpListener, HttpRequest, LISTENERS, RequestCallback, register_callback};
    use crate::wasm_test;

    thread_local! {
        static SEEN: Cell<Option<HttpListenerId>> = const { Cell::new(None) };
        static STORED: RefCell<Option<HttpListener>> = const { RefCell::new(None) };
    }

    fn listen(callback: RequestCallback) -> HttpListener {
        let id = HttpListenerId::from_wire(1).expect("BUG: test ids are non-zero");
        let cb_idx = register_callback(callback);
        LISTENERS.with_borrow_mut(|listeners| listeners.insert(id, cb_idx));
        HttpListener(id)
    }

    fn deliver(id: HttpListenerId) {
        let request = HttpRequest {
            request_id: HttpRequestId::from_wire(1).expect("BUG: test ids are non-zero"),
            method: String::new(),
            path: String::new(),
            headers: String::new(),
            body: Vec::new(),
        };
        super::deliver(id, &request);
    }

    fn is_registered(id: HttpListenerId) -> bool {
        LISTENERS.with_borrow(|registered| registered.contains_key(&id))
    }

    fn record(listener: &HttpListener, _request: &HttpRequest) {
        SEEN.set(Some(listener.0));
    }

    fn close_stored(_listener: &HttpListener, _request: &HttpRequest) {
        if let Some(stored) = STORED.take() {
            stored.close();
        }
    }

    #[wasm_test]
    fn a_request_reaches_its_callback_with_its_listener() {
        let listener = listen(record);

        deliver(listener.0);

        assert_eq!(SEEN.get(), Some(listener.0));
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_test]
    fn a_request_from_the_host_reaches_its_listener() {
        let listener = listen(record);

        super::wasm::__on_http_request(listener.0.to_wire(), 1, 0, 0, 0, 0, 0, 0, 0, 0);

        assert_eq!(SEEN.get(), Some(listener.0));
    }

    #[wasm_test]
    fn a_request_leaves_its_listener_open() {
        let listener = listen(record);

        deliver(listener.0);

        assert!(is_registered(listener.0));
    }

    #[wasm_test]
    fn a_dropped_handle_stops_its_requests() {
        let listener = listen(record);
        let id = listener.0;

        drop(listener);
        deliver(id);

        assert_eq!(SEEN.get(), None);
    }

    #[wasm_test]
    fn a_callback_can_close_its_own_listener_through_the_stored_handle() {
        let listener = listen(close_stored);
        let id = listener.0;
        STORED.set(Some(listener));

        deliver(id);

        assert!(!is_registered(id));
    }
}
