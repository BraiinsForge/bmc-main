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

#![cfg_attr(
    test,
    expect(
        clippy::cast_possible_truncation,
        reason = "native compilation only exposes the wasm32 host ABI for response tests"
    )
)]

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
use std::marker::PhantomData;
use std::time::Duration;

use bmc_wasm_protocol::{FetchOutcome, FetchRequestId};

#[cfg(target_arch = "wasm32")]
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
    content_type: Option<String>,
    body: FetchResponseBody,
}

#[derive(Debug)]
enum FetchResponseBody {
    Guest(Vec<u8>),
    Host { len: u32 },
}

/// A response body kept in host memory for the duration of its fetch callback.
#[derive(Debug, PartialEq, Eq)]
pub struct FetchBodyRef<'a> {
    request_id: FetchRequestId,
    len: u32,
    _response: PhantomData<&'a FetchResponse>,
}

impl FetchBodyRef<'_> {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) const fn request_id_wire(&self) -> u32 {
        self.request_id.to_wire()
    }
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

    /// The origin's `Content-Type`, absent when it sent none — or when
    /// the outcome was the host's own rather than an origin's answer.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Response body as bytes, or an empty slice when [`Self::body_ref`] owns it.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        match &self.body {
            FetchResponseBody::Guest(body) => body,
            FetchResponseBody::Host { .. } => &[],
        }
    }

    /// Callback-scoped host body, when the request opted out of guest delivery.
    #[must_use]
    pub const fn body_ref(&self) -> Option<FetchBodyRef<'_>> {
        match &self.body {
            FetchResponseBody::Guest(_) => None,
            FetchResponseBody::Host { len } => Some(FetchBodyRef {
                request_id: self.request_id,
                len: *len,
                _response: PhantomData,
            }),
        }
    }

    /// Guest-delivered response body as UTF-8, or `None` for a host-owned body.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match &self.body {
            FetchResponseBody::Guest(body) => core::str::from_utf8(body).ok(),
            FetchResponseBody::Host { .. } => None,
        }
    }

    /// Parse the guest-delivered response body as JSON via the host-side parser.
    ///
    /// # Panics
    ///
    /// Panics when the body is host-owned and unavailable to the JSON parser.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn json(&self) -> JsonDoc {
        JsonDoc::parse(self.json_body())
    }

    fn json_body(&self) -> &[u8] {
        match &self.body {
            FetchResponseBody::Guest(body) => body,
            FetchResponseBody::Host { .. } => {
                panic!("BUG: a host-owned fetch body cannot be parsed as guest JSON")
            }
        }
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
    fn host_fetch_response_body_ref(request_id: u32) -> u32;
    fn host_fetch_content_type(request_id: u32, out_ptr: *mut u8, out_cap: u32) -> i32;
}

type Callback = fn(&FetchResponse);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseBodyDelivery {
    Guest,
    Host,
}

#[derive(Debug, Clone, Copy)]
struct PendingFetch {
    callback_idx: usize,
    body_delivery: ResponseBodyDelivery,
}

thread_local! {
    /// Registered callbacks indexed by position.
    static CALLBACKS: RefCell<Vec<Callback>> = const { RefCell::new(Vec::new()) };
    /// Maps request IDs to callback and body-delivery metadata.
    static PENDING: RefCell<HashMap<FetchRequestId, PendingFetch>> = RefCell::new(HashMap::new());
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
    body_delivery: ResponseBodyDelivery,
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
            body_delivery: ResponseBodyDelivery::Guest,
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
            body_delivery: ResponseBodyDelivery::Guest,
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
            body_delivery: ResponseBodyDelivery::Guest,
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
            body_delivery: ResponseBodyDelivery::Guest,
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

    /// Keep the response body in host memory and expose a callback-scoped reference.
    #[must_use]
    pub(crate) fn host_body(mut self) -> Self {
        self.body_delivery = ResponseBodyDelivery::Host;
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
        insert_pending(request_id, cb_idx, self.body_delivery);
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
        insert_pending(request_id, cb_idx, self.body_delivery);
        Some(request_id)
    }
}

fn insert_pending(
    request_id: FetchRequestId,
    callback_idx: usize,
    requested_delivery: ResponseBodyDelivery,
) {
    let body_delivery = match requested_delivery {
        ResponseBodyDelivery::Guest => ResponseBodyDelivery::Guest,
        ResponseBodyDelivery::Host => {
            if unsafe { host_fetch_response_body_ref(request_id.to_wire()) } != 0 {
                ResponseBodyDelivery::Host
            } else {
                ResponseBodyDelivery::Guest
            }
        }
    };
    PENDING.with(|pending| {
        pending.borrow_mut().insert(
            request_id,
            PendingFetch {
                callback_idx,
                body_delivery,
            },
        );
    });
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

/// The delivered response's `Content-Type`, readable only
/// while its own `__on_fetch_response` runs.
fn read_content_type(request_id: FetchRequestId) -> Option<String> {
    let mut buf = vec![0_u8; 128];
    #[expect(
        clippy::cast_possible_truncation,
        reason = "buffer lengths fit u32 on wasm32"
    )]
    let actual = unsafe {
        host_fetch_content_type(request_id.to_wire(), buf.as_mut_ptr(), buf.len() as u32)
    };
    let actual = usize::try_from(actual).ok()?;
    if actual > buf.len() {
        buf = vec![0_u8; actual];
        #[expect(
            clippy::cast_possible_truncation,
            reason = "buffer lengths fit u32 on wasm32"
        )]
        let again = unsafe {
            host_fetch_content_type(request_id.to_wire(), buf.as_mut_ptr(), buf.len() as u32)
        };
        if usize::try_from(again).ok()? != actual {
            return None;
        }
    }
    buf.truncate(actual);
    String::from_utf8(buf).ok()
}

/// Called by the host when a fetch response is ready.
///
/// Guest-delivered bodies arrive through `__alloc`; opted-in bodies remain
/// host-owned and are valid only while the registered callback runs.
#[expect(
    clippy::same_length_and_capacity,
    reason = "__alloc returns exactly body_len bytes"
)]
#[unsafe(no_mangle)]
pub extern "C" fn __on_fetch_response(request_id: u32, status: u32, body_ptr: u32, body_len: u32) {
    let Some(request_id) = FetchRequestId::from_wire(request_id) else {
        return;
    };

    let pending = PENDING.with(|p| p.borrow_mut().remove(&request_id));
    let body = match pending.map(|pending| pending.body_delivery) {
        Some(ResponseBodyDelivery::Host) => FetchResponseBody::Host { len: body_len },
        Some(ResponseBodyDelivery::Guest) | None => {
            let body = if body_len > 0 && body_ptr != 0 {
                unsafe {
                    Vec::from_raw_parts(body_ptr as *mut u8, body_len as usize, body_len as usize)
                }
            } else {
                Vec::new()
            };
            FetchResponseBody::Guest(body)
        }
    };

    let response = FetchResponse {
        status,
        request_id,
        content_type: read_content_type(request_id),
        body,
    };

    // Copy the callback out before invoking it because it may register another fetch.
    let cb = pending
        .and_then(|pending| CALLBACKS.with(|cbs| cbs.borrow().get(pending.callback_idx).copied()));
    if let Some(cb) = cb {
        cb(&response);
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::FetchRequestId;

    use super::{FetchRequest, FetchResponse, FetchResponseBody, ResponseBodyDelivery};

    fn response(body: FetchResponseBody) -> FetchResponse {
        FetchResponse {
            status: 200,
            request_id: FetchRequestId::from_wire(1).expect("BUG: one is a valid request ID"),
            content_type: None,
            body,
        }
    }

    #[test]
    fn guest_body_remains_available_as_text() {
        let response = response(FetchResponseBody::Guest(b"hello".to_vec()));

        assert_eq!(response.text(), Some("hello"));
    }

    #[test]
    fn host_body_request_opts_out_of_guest_delivery() {
        let request = FetchRequest::get("https://example.com").host_body();

        assert_eq!(request.body_delivery, ResponseBodyDelivery::Host);
    }

    #[test]
    #[should_panic(expected = "BUG: a host-owned fetch body cannot be parsed as guest JSON")]
    fn host_body_is_not_reinterpreted_as_empty_json() {
        let response = response(FetchResponseBody::Host { len: 5 });

        let _ = response.json_body();
    }

    #[test]
    fn host_body_is_not_reinterpreted_as_empty_text() {
        let response = response(FetchResponseBody::Host { len: 5 });

        assert_eq!(response.text(), None);
        assert_eq!(
            response.body_ref().map(|body| body.request_id_wire()),
            Some(1)
        );
    }
}
