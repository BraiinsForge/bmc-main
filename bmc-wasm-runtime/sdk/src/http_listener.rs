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
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::http_listener::*;
//!
//! fn start_pairing() {
//!     let listener = http_listen(0, on_request); // port 0 = ephemeral
//!     log_info!("listening on port {}", listener.port());
//! }
//!
//! fn on_request(listener: HttpListener, req: &HttpRequest) {
//!     log_info!("{} {}", req.method, req.path);
//!     req.respond(200, "Content-Type: text/plain", b"OK");
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_protocol::{HttpListenerId, HttpRequestId};

// Host function imports
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

/// Callback type: `fn(listener, request)`.
pub type RequestCallback = fn(HttpListener, &HttpRequest);

/// Handle to an active HTTP listener.
#[derive(Clone, Copy, Debug)]
pub struct HttpListener(pub Option<HttpListenerId>);

impl HttpListener {
    /// Get the actual bound port (useful when port=0 for ephemeral).
    /// Returns `0` if the listener failed to bind.
    #[must_use]
    pub fn port(&self) -> u16 {
        let Some(id) = self.0 else { return 0 };
        unsafe { host_http_get_port(id.to_wire()) as u16 }
    }

    /// Close this listener and stop accepting connections.
    pub fn close(&self) {
        let Some(id) = self.0 else { return };
        LISTENERS.with(|l| l.borrow_mut().remove(&id));
        unsafe { host_http_close_listener(id.to_wire()) }
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
        unsafe {
            host_http_respond(
                self.request_id.to_wire(),
                u32::from(status),
                headers.as_ptr(),
                headers.len() as u32,
                body.as_ptr(),
                body.len() as u32,
            );
        }
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
pub fn http_listen(port: u16, callback: RequestCallback) -> HttpListener {
    let cb_idx = register_callback(callback);
    let listener_id = HttpListenerId::from_wire(unsafe { host_http_listen(u32::from(port)) });
    if let Some(id) = listener_id {
        LISTENERS.with(|l| l.borrow_mut().insert(id, cb_idx));
    }
    HttpListener(listener_id)
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

    let listener = HttpListener(Some(listener_id));

    let cb = LISTENERS
        .with(|l| l.borrow().get(&listener_id).copied())
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));

    if let Some(cb) = cb {
        cb(listener, &req);
    }
}

/// Take ownership of a host-allocated string buffer.
fn take_string(ptr: u32, len: u32) -> String {
    if len == 0 || ptr == 0 {
        return String::new();
    }
    let bytes = unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) };
    String::from_utf8(bytes).unwrap_or_default()
}

/// Take ownership of a host-allocated byte buffer.
fn take_vec(ptr: u32, len: u32) -> Vec<u8> {
    if len == 0 || ptr == 0 {
        return Vec::new();
    }
    unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) }
}
