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

use crate::json::JsonDoc;

/// Response from an HTTP fetch request.
pub struct FetchResponse {
    /// HTTP status code (200, 404, etc.). 0 if network error.
    pub status: u32,
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
        url_ptr: *const u8,
        url_len: u32,
        headers_ptr: *const u8,
        headers_len: u32,
    ) -> u32;
    fn host_fetch_after(
        delay_ms: u32,
        url_ptr: *const u8,
        url_len: u32,
        headers_ptr: *const u8,
        headers_len: u32,
    ) -> u32;
}

type Callback = fn(&FetchResponse);

thread_local! {
    /// Registered callbacks indexed by position.
    static CALLBACKS: RefCell<Vec<Callback>> = const { RefCell::new(Vec::new()) };
    /// Maps request_id → callback index.
    static PENDING: RefCell<HashMap<u32, usize>> = RefCell::new(HashMap::new());
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

/// Fetch a URL. The host performs the request in the background.
/// When the response arrives, `callback` is called with the response data.
///
/// `headers` is an optional newline-separated list of `Key: Value` pairs.
pub fn fetch(url: &str, headers: Option<&str>, callback: Callback) {
    let cb_idx = register_callback(callback);
    let (h_ptr, h_len) = headers_raw(headers);
    let request_id = unsafe { host_fetch(url.as_ptr(), url.len() as u32, h_ptr, h_len) };
    PENDING.with(|p| p.borrow_mut().insert(request_id, cb_idx));
}

/// Fetch a URL after a delay (in milliseconds).
/// Useful for periodic re-fetching (e.g., `fetch_after(300_000, url, None, cb)` for 5-minute refresh).
///
/// `headers` is an optional newline-separated list of `Key: Value` pairs.
pub fn fetch_after(delay_ms: u32, url: &str, headers: Option<&str>, callback: Callback) {
    let cb_idx = register_callback(callback);
    let (h_ptr, h_len) = headers_raw(headers);
    let request_id =
        unsafe { host_fetch_after(delay_ms, url.as_ptr(), url.len() as u32, h_ptr, h_len) };
    PENDING.with(|p| p.borrow_mut().insert(request_id, cb_idx));
}

fn headers_raw(headers: Option<&str>) -> (*const u8, u32) {
    match headers {
        Some(h) => (h.as_ptr(), h.len() as u32),
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
    // Reconstruct the body from the host-allocated buffer
    let body = if body_len > 0 && body_ptr != 0 {
        unsafe { Vec::from_raw_parts(body_ptr as *mut u8, body_len as usize, body_len as usize) }
    } else {
        Vec::new()
    };

    let response = FetchResponse { status, body };

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
