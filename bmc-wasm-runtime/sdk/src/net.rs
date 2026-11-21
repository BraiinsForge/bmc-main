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
//! [`FetchRequest::send_on`] and [`FetchRequest::send_after_on`] send an HTTP request
//! whose response runs a method on the receiver they are given.
//! The host performs the network I/O in the background
//! and delivers responses by calling the `__on_fetch_response` export.
//! A request repeated on a cadence is a poll instead (see [`crate::poll`]).
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::*;
//!
//! struct Profile {
//!     name: Option<String>,
//!     fetch: Option<Fetch>,
//! }
//!
//! impl Profile {
//!     fn on_data(&mut self, response: &FetchResponse) {
//!         if response.ok() {
//!             self.name = response.json().str("/name");
//!             request_frame();
//!         }
//!     }
//! }
//!
//! #[widget]
//! impl Widget for Profile {
//!     fn init(me: &Building<Self>) -> Self {
//!         // A refused request leaves `name` None: the host has logged why.
//!         // Dropping the `Fetch` would cancel it.
//!         let fetch = FetchRequest::get("https://api.example.com/data")
//!             .send_on(me, Self::on_data)
//!             .ok();
//!         Self { name: None, fetch }
//!     }
//!
//!     fn render(&mut self, _delta_ms: u32) { /* … */ }
//! }
//! ```

use std::marker::PhantomData;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
use bmc_wasm_protocol::MediaTypePart;
use bmc_wasm_protocol::{FetchOutcome, FetchRequestId};

use crate::json::JsonDoc;
#[cfg(target_arch = "wasm32")]
use crate::widget::{Receiver, RegisterError, Target};

/// Default per-call cap on every fetch operation (DNS, connect, send, recv),
/// applied to any [`FetchRequest`] that does not set its own [`timeout`].
///
/// [`timeout`]: FetchRequest::timeout
pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Response from an HTTP fetch request.
///
/// Headers are not carried in here: the host holds them for this
/// callback's span, and [`header`] and [`media_type`] read them on demand.
///
/// [`header`]: FetchResponse::header
/// [`media_type`]: FetchResponse::media_type
#[derive(Debug)]
pub struct FetchResponse {
    /// Raw wire status. Prefer [`FetchResponse::outcome`], which types it.
    pub status: u32,
    /// Request ID returned by [`FetchRequest::send`], for correlating responses.
    pub request_id: FetchRequestId,
    body: FetchResponseBody,
}

#[derive(Debug)]
#[cfg_attr(
    all(not(target_arch = "wasm32"), not(test)),
    expect(
        dead_code,
        reason = "only the host's response export builds a response"
    )
)]
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

    /// One of the origin's headers, by name, matched case-insensitively.
    /// Absent for every outcome the host decided itself.
    ///
    /// Readable only while this response's own callback runs.
    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn header(&self, name: &str) -> Option<String> {
        Platform::header(self.request_id, name)
    }

    /// The origin's `Content-Type`, verbatim.
    /// Ask [`media_type`] when the question is what the type *means*.
    ///
    /// [`media_type`]: FetchResponse::media_type
    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn content_type(&self) -> Option<String> {
        self.header("content-type")
    }

    /// One piece of the `Content-Type`, parsed host-side. Absent when the
    /// header is missing, unparsable, or carries no such piece.
    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn media_type(&self, part: MediaTypePart) -> Option<String> {
        Platform::media_type(self.request_id, part)
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

#[cfg(target_arch = "wasm32")]
type Callback = fn(&FetchResponse);

/// A response as a method of the value it is delivered to.
pub type ResponseMethod<T> = fn(&mut T, &FetchResponse);

#[cfg(target_arch = "wasm32")]
type Handler = Rc<dyn Fn(&FetchResponse)>;

#[cfg(target_arch = "wasm32")]
fn on_target<T: 'static>(target: Target<T>, method: ResponseMethod<T>) -> Handler {
    Rc::new(move |response: &FetchResponse| {
        target.deliver(|value| method(value, response));
    })
}

/// A fetch whose response runs a method, from [`FetchRequest::send_on`].
///
/// Dropping it before the response cancels the fetch,
/// and the method does not run even when the request is already away.
#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
#[must_use = "dropping a `Fetch` cancels it"]
pub struct Fetch(FetchRequestId);

#[cfg(target_arch = "wasm32")]
impl Fetch {
    #[must_use]
    pub const fn id(&self) -> FetchRequestId {
        self.0
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for Fetch {
    fn drop(&mut self) {
        Platform::abandon(self.0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseBodyDelivery {
    Guest,
    #[cfg_attr(
        all(not(target_arch = "wasm32"), not(test)),
        expect(dead_code, reason = "off-target no request reaches the host")
    )]
    Host,
}

/// Fetch a URL with GET method. The host performs the request in the background.
/// When the response arrives, `callback` is called with the response data.
///
/// `headers` is an optional newline-separated list of `Key: Value` pairs.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn fetch(url: &str, headers: Option<&str>, callback: Callback) -> Option<FetchRequestId> {
    FetchRequest::get(url).headers_opt(headers).send(callback)
}

/// Fetch a URL with GET after a delay (in milliseconds).
/// A request repeated on a cadence is a poll instead,
/// registered with [`PollHandle::register`](crate::PollHandle::register).
///
/// `headers` is an optional newline-separated list of `Key: Value` pairs.
#[must_use]
#[cfg(target_arch = "wasm32")]
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
#[cfg(target_arch = "wasm32")]
pub fn cancel(request_id: FetchRequestId) -> bool {
    Platform::cancel(request_id)
}

/// Builder for HTTP fetch requests with method, headers, and optional body.
#[derive(Debug)]
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(dead_code, reason = "off-target no request reaches the host")
)]
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
    #[cfg_attr(
        all(not(target_arch = "wasm32"), not(test)),
        expect(dead_code, reason = "off-target no request reaches the host")
    )]
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
    #[cfg(target_arch = "wasm32")]
    pub fn send(self, callback: Callback) -> Option<FetchRequestId> {
        Platform::fetch(&self, Rc::new(callback))
    }

    /// Send the request; the response runs `method` on `to`.
    ///
    /// Otherwise as [`send`](Self::send),
    /// but a refusal is [`RegisterError`] rather than `None`,
    /// or [`HandleError`](crate::HandleError) through a [`PartHandle`](crate::PartHandle).
    /// A part dropped before the response gets none.
    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn send_on<T: 'static, R: Receiver<T>>(
        self,
        to: &R,
        method: ResponseMethod<T>,
    ) -> R::Out<Result<Fetch, RegisterError>> {
        to.__register(|target| {
            Platform::fetch(&self, on_target(target, method))
                .map(Fetch)
                .ok_or(RegisterError::new(()))
        })
    }

    /// Send the request after a delay (in milliseconds).
    ///
    /// Returns `None` if the host rejects the request before it is queued.
    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn send_after(self, delay_ms: u32, callback: Callback) -> Option<FetchRequestId> {
        Platform::fetch_after(delay_ms, &self, Rc::new(callback))
    }

    /// Send the request after a delay; the response runs `method` on `to`.
    ///
    /// Otherwise as [`send_after`](Self::send_after),
    /// but a refusal is [`RegisterError`] rather than `None`,
    /// or [`HandleError`](crate::HandleError) through a [`PartHandle`](crate::PartHandle).
    /// A part dropped before the response gets none.
    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn send_after_on<T: 'static, R: Receiver<T>>(
        self,
        to: &R,
        delay_ms: u32,
        method: ResponseMethod<T>,
    ) -> R::Out<Result<Fetch, RegisterError>> {
        to.__register(|target| {
            Platform::fetch_after(delay_ms, &self, on_target(target, method))
                .map(Fetch)
                .ok_or(RegisterError::new(()))
        })
    }
}

#[cfg(target_arch = "wasm32")]
trait Fetcher {
    fn fetch(request: &FetchRequest<'_>, handler: Handler) -> Option<FetchRequestId>;
    fn fetch_after(
        delay_ms: u32,
        request: &FetchRequest<'_>,
        handler: Handler,
    ) -> Option<FetchRequestId>;
    fn cancel(request_id: FetchRequestId) -> bool;
    /// Drop the response handler, and cancel the fetch if the handler had not run yet.
    fn abandon(request_id: FetchRequestId);
    fn header(request_id: FetchRequestId, name: &str) -> Option<String>;
    fn media_type(request_id: FetchRequestId, part: MediaTypePart) -> Option<String>;
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::time::Duration;

    use bmc_wasm_protocol::{FetchRequestId, MediaTypePart};

    use super::{
        FetchBodyRef, FetchRequest, FetchResponse, FetchResponseBody, Handler, ResponseBodyDelivery,
    };
    use crate::widget;

    struct PendingFetch {
        handler: Handler,
        body_delivery: ResponseBodyDelivery,
    }

    thread_local! {
        /// Maps request IDs to callback and body-delivery metadata.
        static PENDING: RefCell<HashMap<FetchRequestId, PendingFetch>> = RefCell::new(HashMap::new());
    }

    fn insert_pending(
        request_id: FetchRequestId,
        handler: Handler,
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
                    handler,
                    body_delivery,
                },
            );
        });
    }

    impl FetchBodyRef<'_> {
        pub(crate) const fn request_id_wire(&self) -> u32 {
            self.request_id.to_wire()
        }
    }

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
        fn host_fetch_header(
            request_id: u32,
            name_ptr: *const u8,
            name_len: u32,
            out_ptr: *mut u8,
            out_cap: u32,
        ) -> i32;
        fn host_fetch_media_type(request_id: u32, part: u32, out_ptr: *mut u8, out_cap: u32)
        -> i32;
    }

    pub(super) struct Host;

    impl super::Fetcher for Host {
        fn fetch(request: &FetchRequest<'_>, handler: Handler) -> Option<FetchRequestId> {
            let (m_ptr, m_len) = (
                request.method.as_ptr(),
                crate::ffi_u32(request.method.len()),
            );
            let (h_ptr, h_len) = optional_raw(request.headers);
            let (b_ptr, b_len) = optional_bytes_raw(request.body);
            let request_id = FetchRequestId::from_wire(unsafe {
                host_fetch(
                    timeout_ms(request.timeout),
                    m_ptr,
                    m_len,
                    request.url.as_ptr(),
                    crate::ffi_u32(request.url.len()),
                    h_ptr,
                    h_len,
                    b_ptr,
                    b_len,
                )
            })?;
            insert_pending(request_id, handler, request.body_delivery);
            Some(request_id)
        }

        fn fetch_after(
            delay_ms: u32,
            request: &FetchRequest<'_>,
            handler: Handler,
        ) -> Option<FetchRequestId> {
            let (m_ptr, m_len) = (
                request.method.as_ptr(),
                crate::ffi_u32(request.method.len()),
            );
            let (h_ptr, h_len) = optional_raw(request.headers);
            let (b_ptr, b_len) = optional_bytes_raw(request.body);
            let request_id = FetchRequestId::from_wire(unsafe {
                host_fetch_after(
                    delay_ms,
                    timeout_ms(request.timeout),
                    m_ptr,
                    m_len,
                    request.url.as_ptr(),
                    crate::ffi_u32(request.url.len()),
                    h_ptr,
                    h_len,
                    b_ptr,
                    b_len,
                )
            })?;
            insert_pending(request_id, handler, request.body_delivery);
            Some(request_id)
        }

        fn cancel(request_id: FetchRequestId) -> bool {
            let stopped = unsafe { host_fetch_cancel(request_id.to_wire()) } != 0;
            if stopped {
                PENDING.with(|p| p.borrow_mut().remove(&request_id));
            }
            stopped
        }

        fn abandon(request_id: FetchRequestId) {
            let pending = PENDING.with(|p| p.borrow_mut().remove(&request_id));
            if pending.is_some() {
                unsafe { host_fetch_cancel(request_id.to_wire()) };
            }
        }

        fn header(request_id: FetchRequestId, name: &str) -> Option<String> {
            read_host_string(|ptr, cap| unsafe {
                host_fetch_header(
                    request_id.to_wire(),
                    name.as_ptr(),
                    crate::ffi_u32(name.len()),
                    ptr,
                    cap,
                )
            })
        }

        fn media_type(request_id: FetchRequestId, part: MediaTypePart) -> Option<String> {
            read_host_string(|ptr, cap| unsafe {
                host_fetch_media_type(request_id.to_wire(), part.to_wire(), ptr, cap)
            })
        }
    }

    fn timeout_ms(timeout: Duration) -> u32 {
        u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX)
    }

    fn optional_raw(s: Option<&str>) -> (*const u8, u32) {
        match s {
            Some(s) => (s.as_ptr(), crate::ffi_u32(s.len())),
            None => (core::ptr::null(), 0),
        }
    }

    fn optional_bytes_raw(b: Option<&[u8]>) -> (*const u8, u32) {
        match b {
            Some(b) => (b.as_ptr(), crate::ffi_u32(b.len())),
            None => (core::ptr::null(), 0),
        }
    }

    /// Guessing wrong costs one extra host call;
    /// a `Content-Type` never needs it.
    const HEADER_READ_HINT: usize = 128;

    /// Read a host-written string through `call`, which reports the full byte
    /// length whatever buffer it got, and a negative value for one it lacks.
    ///
    /// A value that changed length between the calls is dropped, not truncated.
    fn read_host_string(mut call: impl FnMut(*mut u8, u32) -> i32) -> Option<String> {
        let mut buf = vec![0_u8; HEADER_READ_HINT];
        let actual = usize::try_from(call(buf.as_mut_ptr(), crate::ffi_u32(buf.len()))).ok()?;
        if actual > buf.len() {
            buf = vec![0_u8; actual];
            if usize::try_from(call(buf.as_mut_ptr(), crate::ffi_u32(buf.len()))).ok()? != actual {
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
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_fetch_response(
        request_id: u32,
        status: u32,
        body_ptr: u32,
        body_len: u32,
    ) {
        widget::export(|| on_response(request_id, status, body_ptr, body_len));
    }

    #[expect(
        clippy::same_length_and_capacity,
        reason = "the host hands back the size it asked __alloc for"
    )]
    fn on_response(request_id: u32, status: u32, body_ptr: u32, body_len: u32) {
        let Some(request_id) = FetchRequestId::from_wire(request_id) else {
            return;
        };

        let pending = PENDING.with(|p| p.borrow_mut().remove(&request_id));
        let body = match pending.as_ref().map(|pending| pending.body_delivery) {
            Some(ResponseBodyDelivery::Host) => FetchResponseBody::Host { len: body_len },
            Some(ResponseBodyDelivery::Guest) | None => {
                let body = if body_len > 0 && body_ptr != 0 {
                    unsafe {
                        Vec::from_raw_parts(
                            body_ptr as *mut u8,
                            body_len as usize,
                            body_len as usize,
                        )
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
            body,
        };

        if let Some(pending) = pending {
            (pending.handler)(&response);
        }
    }

    #[cfg(test)]
    mod tests {
        use std::cell::Cell;

        use bmc_wasm_protocol::FetchRequestId;

        use super::{__on_fetch_response, insert_pending};
        use crate::net::{Fetch, FetchRequest, FetchResponse, ResponseBodyDelivery, on_target};
        use crate::wasm_test;
        use crate::widget::Part;

        /// The host's default limit of fetches in flight per widget.
        const FETCH_LIMIT: usize = 16;

        thread_local! {
            static RESPONSES: Cell<usize> = const { Cell::new(0) };
        }

        struct Client {
            statuses: Vec<u32>,
        }

        impl Client {
            fn on_response(&mut self, response: &FetchResponse) {
                RESPONSES.set(RESPONSES.get() + 1);
                self.statuses.push(response.status);
            }
        }

        #[wasm_test]
        fn a_response_runs_as_a_method_of_the_part() {
            let part = Part::new(Client {
                statuses: Vec::new(),
            });
            let id = FetchRequestId::from_wire(1).expect("BUG: test ids are non-zero");
            insert_pending(
                id,
                on_target(part.target(), Client::on_response),
                ResponseBodyDelivery::Guest,
            );

            crate::widget::as_host(|| __on_fetch_response(id.to_wire(), 200, 0, 0));

            assert_eq!(part.with(|value| value.statuses.clone()), vec![200]);
        }

        #[wasm_test]
        fn a_response_after_the_part_is_dropped_reaches_nothing() {
            let part = Part::new(Client {
                statuses: Vec::new(),
            });
            let id = FetchRequestId::from_wire(2).expect("BUG: test ids are non-zero");
            insert_pending(
                id,
                on_target(part.target(), Client::on_response),
                ResponseBodyDelivery::Guest,
            );

            drop(part);
            crate::widget::as_host(|| __on_fetch_response(id.to_wire(), 200, 0, 0));

            assert_eq!(RESPONSES.get(), 0);
        }

        fn client_awaiting(wire_id: u32) -> (Part<Client>, Fetch) {
            let part = Part::new(Client {
                statuses: Vec::new(),
            });
            let id = FetchRequestId::from_wire(wire_id).expect("BUG: test ids are non-zero");
            insert_pending(
                id,
                on_target(part.target(), Client::on_response),
                ResponseBodyDelivery::Guest,
            );
            (part, Fetch(id))
        }

        #[wasm_test]
        fn a_response_to_a_dropped_fetch_runs_no_method() {
            let (part, fetch) = client_awaiting(3);
            let id = fetch.id();

            drop(fetch);
            crate::widget::as_host(|| __on_fetch_response(id.to_wire(), 200, 0, 0));

            assert_eq!(
                part.with(|value| value.statuses.len()),
                0,
                "a host past cancelling still answers, with nobody left to hear it"
            );
        }

        struct Owner {
            fetch: Option<Fetch>,
            responses: usize,
        }

        impl Owner {
            fn on_response(&mut self, _response: &FetchResponse) {
                self.responses += 1;
                self.fetch = None;
            }
        }

        #[wasm_test]
        fn a_method_may_drop_the_fetch_it_answers() {
            let mut part = Part::new(Owner {
                fetch: None,
                responses: 0,
            });
            let id = FetchRequestId::from_wire(5).expect("BUG: test ids are non-zero");
            insert_pending(
                id,
                on_target(part.target(), Owner::on_response),
                ResponseBodyDelivery::Guest,
            );
            part.with_mut(|value| value.fetch = Some(Fetch(id)));

            crate::widget::as_host(|| __on_fetch_response(id.to_wire(), 200, 0, 0));

            assert_eq!(part.with(|value| value.responses), 1);
        }

        #[wasm_test]
        fn a_dropped_scheduled_fetch_frees_its_slot_for_the_next() {
            let part = Part::new(Client {
                statuses: Vec::new(),
            });
            let send = || {
                FetchRequest::get("http://example.com/").send_after_on(
                    &part,
                    60_000,
                    Client::on_response,
                )
            };
            let held: Vec<_> = (0..FETCH_LIMIT).map(|_| send()).collect();
            assert!(
                held.iter().all(Result::is_ok) && send().is_err(),
                "the host holds exactly FETCH_LIMIT fetches, or this proves nothing"
            );

            drop(held);

            assert!(send().is_ok(), "dropping the fetches cancelled them");
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(test)]
impl FetchResponse {
    /// A reply to `request_id` with no body.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the poll glue tests that use it run off-device")
    )]
    pub(crate) fn empty(request_id: FetchRequestId, status: u32) -> Self {
        Self {
            status,
            request_id,
            body: FetchResponseBody::Guest(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::FetchRequestId;

    use super::{FetchRequest, FetchResponse, FetchResponseBody, ResponseBodyDelivery};
    use crate::wasm_test;

    fn response(body: FetchResponseBody) -> FetchResponse {
        FetchResponse {
            status: 200,
            request_id: FetchRequestId::from_wire(1).expect("BUG: one is a valid request ID"),
            body,
        }
    }

    #[wasm_test]
    fn guest_body_remains_available_as_text() {
        let response = response(FetchResponseBody::Guest(b"hello".to_vec()));

        assert_eq!(response.text(), Some("hello"));
    }

    #[wasm_test]
    fn host_body_request_opts_out_of_guest_delivery() {
        let request = FetchRequest::get("https://example.com").host_body();

        assert_eq!(request.body_delivery, ResponseBodyDelivery::Host);
    }

    #[wasm_test]
    #[should_panic(expected = "BUG: a host-owned fetch body cannot be parsed as guest JSON")]
    fn host_body_is_not_reinterpreted_as_empty_json() {
        let response = response(FetchResponseBody::Host { len: 5 });

        let _ = response.json_body();
    }

    #[wasm_test]
    fn host_body_is_not_reinterpreted_as_empty_text() {
        let response = response(FetchResponseBody::Host { len: 5 });

        assert_eq!(response.text(), None);
        assert_eq!(
            response.body_ref().map(|body| body.request_id.to_wire()),
            Some(1)
        );
    }
}
