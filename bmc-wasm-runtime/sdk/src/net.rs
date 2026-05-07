// Copyright (C) 2026  Braiins Systems s.r.o.

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

use bmc_wasm_protocol::FetchRequestId;

use crate::json::JsonDoc;

/// Response from an HTTP fetch request.
#[derive(Debug)]
pub struct FetchResponse {
    /// HTTP status code (200, 404, etc.). 0 if network error.
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
unsafe extern "C" {
    fn host_fetch(
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
        method_ptr: *const u8,
        method_len: u32,
        url_ptr: *const u8,
        url_len: u32,
        headers_ptr: *const u8,
        headers_len: u32,
        body_ptr: *const u8,
        body_len: u32,
    ) -> u32;
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

/// Builder for HTTP fetch requests with method, headers, and optional body.
#[derive(Debug)]
pub struct FetchRequest<'a> {
    method: &'a str,
    url: &'a str,
    headers: Option<&'a str>,
    body: Option<&'a [u8]>,
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
