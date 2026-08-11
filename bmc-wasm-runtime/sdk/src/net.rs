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

//! Network fetching for WASM widgets.
//!
//! Provides `fetch()` and `fetch_after()` for HTTP requests. The host performs
//! the actual network I/O in the background and delivers responses by calling
//! the `__on_fetch_response` export.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::*;
//!
//! fn init(width: u32, height: u32) {
//!     fetch("https://api.example.com/data", on_data);
//! }
//!
//! fn on_data(response: &FetchResponse) {
//!     if response.ok() {
//!         let json = response.json();
//!         let name = json.str("/name").unwrap_or_default();
//!         // ... update state
//!     }
//!     // Re-fetch every 5 minutes
//!     fetch_after(300_000, "https://api.example.com/data", on_data);
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use bmc_wasm_protocol::{FetchOutcome, FetchRequestId};

use crate::json::JsonDoc;

/// Default per-call cap on every fetch operation (DNS, connect, send, recv),
/// applied to any [`FetchRequest`] that does not set its own [`timeout`].
///
/// [`timeout`]: FetchRequest::timeout
pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Response from an HTTP fetch request.
#[derive(Debug)]
pub struct FetchResponse {
    /// Raw wire status. Prefer [`FetchResponse::outcome`], which types it.
    pub status: u32,
    /// Request ID returned by [`FetchRequest::send`], for correlating responses.
    pub request_id: FetchRequestId,
    body: Vec<u8>,
}

impl FetchResponse {
    /// Whether the response has a 2xx status code.
    #[must_use]
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// How the fetch ended. `None` when the host reported an outcome newer
    /// than this build knows — treat that as a failure, never as success.
    #[must_use]
    pub fn outcome(&self) -> Option<FetchOutcome> {
        FetchOutcome::from_wire(self.status)
    }

    /// Response body as bytes.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Response body as a UTF-8 string.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        core::str::from_utf8(&self.body).ok()
    }

    /// Parse the response body as JSON via the host-side parser.
    ///
    /// Returns a `JsonDoc` handle for querying fields with JSON Pointer paths.
    #[must_use]
    pub fn json(&self) -> JsonDoc {
        JsonDoc::parse(&self.body)
    }
}

// Host function imports
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_fetch(
        timeout_ms: u32,
        method_ptr: *const u8,
        method_len: u32,
        url_ptr: *const u8,
        url_len: u32,
        headers_ptr: *const u8,
        headers_len: u32,
        body_ptr: *const u8,
        body_len: u32,
    ) -> u32;
    fn host_fetch_after(
        delay_ms: u32,
        timeout_ms: u32,
        method_ptr: *const u8,
        method_len: u32,
        url_ptr: *const u8,
        url_len: u32,
        headers_ptr: *const u8,
        headers_len: u32,
        body_ptr: *const u8,
        body_len: u32,
    ) -> u32;
    fn host_fetch_cancel(request_id: u32) -> u32;
}

type Callback = fn(&FetchResponse);

thread_local! {
    /// Registered callbacks indexed by position.
    static CALLBACKS: RefCell<Vec<Callback>> = const { RefCell::new(Vec::new()) };
    /// Maps request_id → callback index.
    static PENDING: RefCell<HashMap<FetchRequestId, usize>> = RefCell::new(HashMap::new());
}

/// Register a callback and return its index.
fn register_callback(cb: Callback) -> usize {
    CALLBACKS.with(|cbs| {
        let mut cbs = cbs.borrow_mut();
        // Reuse existing slot if same function pointer
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

/// Fetch a URL with GET method. The host performs the request in the background.
/// When the response arrives, `callback` is called with the response data.
///
/// `headers` is an optional newline-separated list of `Key: Value` pairs.
#[must_use]
pub fn fetch(url: &str, headers: Option<&str>, callback: Callback) -> Option<FetchRequestId> {
    FetchRequest::get(url).headers_opt(headers).send(callback)
}

/// Fetch a URL with GET after a delay (in milliseconds).
/// Useful for periodic re-fetching (e.g., `fetch_after(300_000, url, None, cb)` for 5-minute refresh).
///
/// `headers` is an optional newline-separated list of `Key: Value` pairs.
#[must_use]
pub fn fetch_after(
    delay_ms: u32,
    url: &str,
    headers: Option<&str>,
    callback: Callback,
) -> Option<FetchRequestId> {
    FetchRequest::get(url)
        .headers_opt(headers)
        .send_after(delay_ms, callback)
}

/// Cancel a previously scheduled fetch by its [`FetchRequestId`].
///
/// `true`: it was still queued and is gone — no callback, and its fetch slot
/// freed within this call, so a replacement can be sent immediately.
/// `false`: it is already away and cannot be stopped — its callback still
/// runs, once, with [`FetchOutcome::Aborted`] rather than with data.
#[must_use]
pub fn cancel(request_id: FetchRequestId) -> bool {
    let stopped = unsafe { host_fetch_cancel(request_id.to_wire()) } != 0;
    if stopped {
        PENDING.with(|p| p.borrow_mut().remove(&request_id));
    }
    stopped
}

/// Builder for HTTP fetch requests with method, headers, and optional body.
#[derive(Debug)]
pub struct FetchRequest<'a> {
    method: &'a str,
    url: &'a str,
    headers: Option<&'a str>,
    body: Option<&'a [u8]>,
    timeout: Duration,
}

impl<'a> FetchRequest<'a> {
    /// Create a GET request.
    #[must_use]
    pub fn get(url: &'a str) -> Self {
        Self {
            method: "GET",
            url,
            headers: None,
            body: None,
            timeout: DEFAULT_FETCH_TIMEOUT,
        }
    }

    /// Create a POST request.
    #[must_use]
    pub fn post(url: &'a str) -> Self {
        Self {
            method: "POST",
            url,
            headers: None,
            body: None,
            timeout: DEFAULT_FETCH_TIMEOUT,
        }
    }

    /// Create a PUT request.
    #[must_use]
    pub fn put(url: &'a str) -> Self {
        Self {
            method: "PUT",
            url,
            headers: None,
            body: None,
            timeout: DEFAULT_FETCH_TIMEOUT,
        }
    }

    /// Create a DELETE request.
    #[must_use]
    pub fn delete(url: &'a str) -> Self {
        Self {
            method: "DELETE",
            url,
            headers: None,
            body: None,
            timeout: DEFAULT_FETCH_TIMEOUT,
        }
    }

    /// Set headers (newline-separated `Key: Value` pairs).
    #[must_use]
    pub fn headers(mut self, headers: &'a str) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Set headers from an `Option`.
    #[must_use]
    pub fn headers_opt(mut self, headers: Option<&'a str>) -> Self {
        self.headers = headers;
        self
    }

    /// Set request body bytes.
    #[must_use]
    pub fn body(mut self, body: &'a [u8]) -> Self {
        self.body = Some(body);
        self
    }

    /// Override the per-call timeout (DNS, connect, send, recv). Defaults to
    /// [`DEFAULT_FETCH_TIMEOUT`] when unset.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Send the request immediately. Returns `None` if the host rejects the
    /// request before it is queued, for example because the runtime hit its
    /// resource limit.
    #[must_use]
    pub fn send(self, callback: Callback) -> Option<FetchRequestId> {
        let cb_idx = register_callback(callback);
        let (m_ptr, m_len) = (self.method.as_ptr(), self.method.len() as u32);
        let (h_ptr, h_len) = optional_raw(self.headers);
        let (b_ptr, b_len) = optional_bytes_raw(self.body);
        let request_id = FetchRequestId::from_wire(unsafe {
            host_fetch(
                timeout_ms(self.timeout),
                m_ptr,
                m_len,
                self.url.as_ptr(),
                self.url.len() as u32,
                h_ptr,
                h_len,
                b_ptr,
                b_len,
            )
        })?;
        PENDING.with(|p| p.borrow_mut().insert(request_id, cb_idx));
        Some(request_id)
    }

    /// Send the request after a delay (in milliseconds).
    ///
    /// Returns `None` if the host rejects the request before it is queued.
    #[must_use]
    pub fn send_after(self, delay_ms: u32, callback: Callback) -> Option<FetchRequestId> {
        let cb_idx = register_callback(callback);
        let (m_ptr, m_len) = (self.method.as_ptr(), self.method.len() as u32);
        let (h_ptr, h_len) = optional_raw(self.headers);
        let (b_ptr, b_len) = optional_bytes_raw(self.body);
        let request_id = FetchRequestId::from_wire(unsafe {
            host_fetch_after(
                delay_ms,
                timeout_ms(self.timeout),
                m_ptr,
                m_len,
                self.url.as_ptr(),
                self.url.len() as u32,
                h_ptr,
                h_len,
                b_ptr,
                b_len,
            )
        })?;
        PENDING.with(|p| p.borrow_mut().insert(request_id, cb_idx));
        Some(request_id)
    }
}

fn timeout_ms(timeout: Duration) -> u32 {
    u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX)
}

fn optional_raw(s: Option<&str>) -> (*const u8, u32) {
    match s {
        Some(s) => (s.as_ptr(), s.len() as u32),
        None => (core::ptr::null(), 0),
    }
}

fn optional_bytes_raw(b: Option<&[u8]>) -> (*const u8, u32) {
    match b {
        Some(b) => (b.as_ptr(), b.len() as u32),
        None => (core::ptr::null(), 0),
    }
}

/// Called by the host when a fetch response is ready.
///
/// The host allocates WASM memory via `__alloc`, writes the body there,
/// then calls this export. We reconstruct the body Vec and dispatch to
/// the registered callback.
#[unsafe(no_mangle)]
pub extern "C" fn __on_fetch_response(request_id: u32, status: u32, body_ptr: u32, body_len: u32) {
    let Some(request_id) = FetchRequestId::from_wire(request_id) else {
        return;
    };

    // Reconstruct the body from the host-allocated buffer
    let body = if body_len > 0 && body_ptr != 0 {
        unsafe { Vec::from_raw_parts(body_ptr as *mut u8, body_len as usize, body_len as usize) }
    } else {
        Vec::new()
    };

    let response = FetchResponse {
        status,
        request_id,
        body,
    };

    // Look up the registered callback. We must copy the function pointer out
    // before invoking it, because the callback may call `fetch()` which needs
    // mutable access to CALLBACKS via `register_callback()`.
    let cb = PENDING
        .with(|p| p.borrow_mut().remove(&request_id))
        .and_then(|idx| CALLBACKS.with(|cbs| cbs.borrow().get(idx).copied()));
    if let Some(cb) = cb {
        cb(&response);
    }
}
