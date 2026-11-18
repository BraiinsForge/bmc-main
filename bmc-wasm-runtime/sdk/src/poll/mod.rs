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

//! Fetches the SDK repeats on a widget's behalf.
//!
//! `register`, re-exported as `register_poll`, starts a poll and returns the only [`Handle`] to it.
//! The SDK sends the poll's request, hands each reply to its `OnReply` handler
//! with the borrowed handle, and sends the next request on the [`Config`] cadence.
//! Dropping the handle, or calling `Handle::stop`, ends the poll and cancels its live fetch.

#[cfg(any(target_arch = "wasm32", test))]
use std::collections::HashMap;
#[cfg(any(target_arch = "wasm32", test))]
use std::mem::ManuallyDrop;
use std::time::Duration;

use bmc_wasm_protocol::FetchRequestId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

/// Where the host delivers a fetch response body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FetchBodyDelivery {
    /// Copy the response body into guest memory.
    #[default]
    Guest,
    /// Keep the response body in host memory for callback-scoped processing.
    Host,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchSpec {
    pub method: Method,
    pub url: String,
    pub headers: Option<String>,
    pub body: Option<Vec<u8>>,
    /// Per-call timeout; `None` defers to the SDK-wide
    /// default applied by `FetchRequest`.
    pub timeout: Option<Duration>,
    pub body_delivery: FetchBodyDelivery,
}

impl FetchSpec {
    #[must_use]
    pub fn get(url: impl Into<String>) -> Self {
        Self::verb(Method::Get, url)
    }

    #[must_use]
    pub fn post(url: impl Into<String>) -> Self {
        Self::verb(Method::Post, url)
    }

    #[must_use]
    pub fn put(url: impl Into<String>) -> Self {
        Self::verb(Method::Put, url)
    }

    #[must_use]
    pub fn delete(url: impl Into<String>) -> Self {
        Self::verb(Method::Delete, url)
    }

    fn verb(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: None,
            body: None,
            timeout: None,
            body_delivery: FetchBodyDelivery::Guest,
        }
    }

    #[must_use]
    pub fn headers(mut self, headers: impl Into<String>) -> Self {
        self.headers = Some(headers.into());
        self
    }

    #[must_use]
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Keep the response body in host memory for callback-scoped processing.
    /// `FetchResponse::body` is empty, `FetchResponse::text` returns `None`, and
    /// `FetchResponse::json` panics for host-retained bodies.
    #[must_use]
    pub fn host_body(mut self) -> Self {
        self.body_delivery = FetchBodyDelivery::Host;
        self
    }

    /// Override the per-call timeout (DNS, connect, send, recv).
    /// Defaults to the SDK-wide `DEFAULT_FETCH_TIMEOUT` when unset.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub interval_ms: Option<u32>,
    pub retry_ms: u32,
    pub debounce_ms: u32,
    pub enabled: bool,
    /// `interval_ms` multiplier past which an outstanding failure reads as stale.
    pub stale_factor: f32,
}

impl Default for Config {
    /// The common widget polling cadence; override what differs.
    fn default() -> Self {
        Self {
            interval_ms: None,
            retry_ms: 10_000,
            debounce_ms: 300,
            enabled: true,
            stale_factor: 1.5,
        }
    }
}

/// The only handle to a registered poll; dropping it ends the poll.
///
/// Once the poll has ended, as on a reply handler's argument after the handler stopped the stored handle,
/// its methods do nothing: `enabled` reads `false`, and neither `is_stale` nor `is_offline` holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Handle(usize);

impl Handle {
    /// The poll's position in registration order; an ended poll's index is never reused.
    #[must_use]
    pub fn index(&self) -> usize {
        self.0
    }
}

/// Builds the poll's next request, or `None` to skip it.
///
/// It runs while the SDK holds its poll registry,
/// so calling any poll handle method from it but [`Handle::index`] panics,
/// and so does dropping a poll handle.
pub type Build = fn(&Handle) -> Option<FetchSpec>;

pub trait FetchBackend {
    fn send(&mut self, spec: &FetchSpec, delay_ms: Option<u32>) -> Option<FetchRequestId>;
    fn cancel(&mut self, id: FetchRequestId) -> bool;
}

#[cfg(any(target_arch = "wasm32", test))]
struct Poll {
    build: Build,
    config: Config,
    enabled: bool,
    live: Option<FetchRequestId>,
    pending: bool,
    force_retry: Option<u32>,
    /// Epoch secs of the last ok reply; `None` before the first success.
    last_success_secs: Option<i64>,
    /// The anchor before the last ok reply; `mark_stale` reverts to it.
    prev_success_secs: Option<i64>,
    /// The last delivered reply failed.
    last_failed: bool,
    /// Epoch secs the failing streak began; anchors the offline grace.
    failing_since_secs: Option<i64>,
}

/// An ended poll's slot stays empty for good:
/// reusing it would hand its index to a new poll,
/// and widgets key their own state on [`Handle::index`].
/// The cost is one empty slot per ended poll for the widget's lifetime,
/// which only matters to a widget that registers polls over and over.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Default)]
pub(crate) struct Registry {
    polls: Vec<Option<Poll>>,
    routes: HashMap<FetchRequestId, usize>,
}

#[cfg(any(target_arch = "wasm32", test))]
fn poll_mut(polls: &mut [Option<Poll>], index: usize) -> Option<&mut Poll> {
    polls.get_mut(index)?.as_mut()
}

#[cfg(any(target_arch = "wasm32", test))]
impl Registry {
    pub(crate) fn register(&mut self, build: Build, config: Config) -> usize {
        let index = self.polls.len();
        self.polls.push(Some(Poll {
            build,
            config,
            enabled: config.enabled,
            live: None,
            pending: false,
            force_retry: None,
            last_success_secs: None,
            prev_success_secs: None,
            last_failed: false,
            failing_since_secs: None,
        }));
        index
    }

    /// End the poll: cancel its live fetch and stop routing replies to it.
    pub(crate) fn release(&mut self, index: usize, backend: &mut dyn FetchBackend) {
        let Some(poll) = self.polls.get_mut(index).and_then(Option::take) else {
            return;
        };
        if let Some(id) = poll.live {
            // A reply the host already queued finds no route and is dropped.
            backend.cancel(id);
            self.routes.remove(&id);
        }
    }

    fn poll(&self, index: usize) -> Option<&Poll> {
        self.polls.get(index)?.as_ref()
    }

    fn kick(&mut self, index: usize, delay_ms: Option<u32>, backend: &mut dyn FetchBackend) {
        let Some(poll) = self.poll(index) else {
            return;
        };
        if !poll.enabled || poll.live.is_some() {
            return;
        }
        let Some(spec) = (poll.build)(&ManuallyDrop::new(Handle(index))) else {
            return;
        };
        let retry_ms = poll.config.retry_ms;
        // Single-flight per poll keeps the in-flight count well under the host budget,
        // so a send should not be rejected; treat an unexpected rejection as a transient
        // failure and re-attempt after retry_ms so a budget hiccup self-heals instead
        // of leaving the poll dormant forever.
        let id = match backend.send(&spec, delay_ms) {
            Some(id) => Some(id),
            None => backend.send(&spec, Some(retry_ms)),
        };
        if let Some(id) = id
            && let Some(poll) = poll_mut(&mut self.polls, index)
        {
            poll.live = Some(id);
            self.routes.insert(id, index);
        }
    }

    pub(crate) fn start(&mut self, index: usize, backend: &mut dyn FetchBackend) {
        self.kick(index, None, backend);
    }

    pub(crate) fn invalidate(&mut self, index: usize, backend: &mut dyn FetchBackend) {
        let Some(poll) = poll_mut(&mut self.polls, index) else {
            return;
        };
        if !poll.enabled {
            return;
        }
        let debounce = poll.config.debounce_ms;
        match poll.live {
            Some(id) => {
                if backend.cancel(id) {
                    poll.live = None;
                    self.routes.remove(&id);
                    self.kick(index, Some(debounce), backend);
                } else {
                    poll.pending = true;
                }
            }
            None => self.kick(index, Some(debounce), backend),
        }
    }

    pub(crate) fn begin_reply(
        &mut self,
        id: FetchRequestId,
        backend: &mut dyn FetchBackend,
    ) -> ReplyAction {
        let Some(index) = self.routes.remove(&id) else {
            return ReplyAction::Ignore;
        };
        let Some(poll) = poll_mut(&mut self.polls, index) else {
            return ReplyAction::Ignore;
        };
        if poll.live != Some(id) {
            return ReplyAction::Ignore;
        }
        poll.live = None;
        let pending = std::mem::take(&mut poll.pending);
        if !poll.enabled {
            return ReplyAction::Stopped;
        }
        if pending {
            self.kick(index, None, backend);
            return ReplyAction::Reissued;
        }
        ReplyAction::Deliver(index)
    }

    /// Mark the in-flight reply as unusable: a 2xx whose body
    /// the handler cannot use, malformed or out of range.
    ///
    /// It then counts as no refresh: the poll reads as failing, the anchor
    /// holds where the last good reply left it, and the next attempt waits
    /// `retry_ms` rather than the poll interval.
    pub(crate) fn request_retry(&mut self, index: usize) {
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.force_retry = Some(poll.config.retry_ms);
        }
    }

    /// Like `request_retry`, but the next attempt waits `delay_ms` rather than
    /// `retry_ms`, letting a reply handler apply its own backoff.
    pub(crate) fn request_retry_after(&mut self, index: usize, delay_ms: u32) {
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.force_retry = Some(delay_ms);
        }
    }

    /// A just-banked reply proved bad (a late decode failure):
    /// flag failing and revert the anchor it displaced.
    pub(crate) fn mark_stale(&mut self, index: usize) {
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.last_failed = true;
            poll.last_success_secs = poll.prev_success_secs;
        }
    }

    /// Seed the last-success anchor from persisted state;
    /// a no-op once a live reply has banked one.
    pub(crate) fn restore_anchor(&mut self, index: usize, secs: i64) {
        if let Some(poll) = poll_mut(&mut self.polls, index)
            && poll.last_success_secs.is_none()
        {
            poll.last_success_secs = Some(secs);
        }
    }

    /// Forget all staleness history, as if the poll had never run.
    pub(crate) fn reset_staleness(&mut self, index: usize) {
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.last_success_secs = None;
            poll.prev_success_secs = None;
            poll.last_failed = false;
            poll.failing_since_secs = None;
        }
    }

    pub(crate) fn reschedule(
        &mut self,
        index: usize,
        ok: bool,
        now_secs: i64,
        backend: &mut dyn FetchBackend,
    ) {
        let Some(poll) = poll_mut(&mut self.polls, index) else {
            return;
        };
        let forced = std::mem::take(&mut poll.force_retry);
        // A forced retry means the handler rejected this reply, so it is no
        // refresh however the HTTP call went.
        if ok && forced.is_none() {
            poll.prev_success_secs = poll.last_success_secs;
            poll.last_success_secs = Some(now_secs);
            poll.last_failed = false;
            poll.failing_since_secs = None;
        } else {
            poll.last_failed = true;
            poll.failing_since_secs.get_or_insert(now_secs);
        }
        if !poll.enabled || poll.live.is_some() {
            return;
        }
        let delay = match forced {
            Some(delay_ms) => delay_ms,
            None if ok => match poll.config.interval_ms {
                Some(ms) => ms,
                None => return,
            },
            None => poll.config.retry_ms,
        };
        self.kick(index, Some(delay), backend);
    }

    pub(crate) fn set_enabled(
        &mut self,
        index: usize,
        enabled: bool,
        backend: &mut dyn FetchBackend,
    ) {
        let Some(poll) = poll_mut(&mut self.polls, index) else {
            return;
        };
        if poll.enabled == enabled {
            return;
        }
        poll.enabled = enabled;
        if enabled {
            let debounce = poll.config.debounce_ms;
            self.kick(index, Some(debounce), backend);
        } else if let Some(id) = poll.live
            && backend.cancel(id)
        {
            poll.live = None;
            self.routes.remove(&id);
        }
    }

    /// Enable and schedule the next fetch after `delay_ms` (no immediate kick).
    pub(crate) fn set_enabled_after(
        &mut self,
        index: usize,
        delay_ms: u32,
        backend: &mut dyn FetchBackend,
    ) {
        let Some(poll) = poll_mut(&mut self.polls, index) else {
            return;
        };
        poll.enabled = true;
        self.kick(index, Some(delay_ms), backend);
    }

    /// Retarget the poll cadence and `is_stale` threshold; reschedule to apply.
    pub(crate) fn set_interval(&mut self, index: usize, interval_ms: u32) {
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.config.interval_ms = Some(interval_ms);
        }
    }

    pub(crate) fn enabled(&self, index: usize) -> bool {
        self.poll(index).is_some_and(|poll| poll.enabled)
    }

    /// Age of the last successful reply; `None` before the first success.
    pub(crate) fn last_success_age(&self, index: usize, now_secs: i64) -> Option<Duration> {
        let last = self.poll(index)?.last_success_secs?;
        let secs = now_secs.saturating_sub(last).max(0);
        #[expect(clippy::cast_sign_loss, reason = "secs is clamped to >= 0")]
        Some(Duration::from_secs(secs as u64))
    }

    /// Epoch secs of the last successful reply — the relative-time anchor.
    pub(crate) fn last_success_secs(&self, index: usize) -> Option<i64> {
        self.poll(index)?.last_success_secs
    }

    /// Failing, with the last good reply aged past `stale_factor × interval_ms`.
    pub(crate) fn is_stale(&self, index: usize, now_secs: i64) -> bool {
        let Some(poll) = self.poll(index) else {
            return false;
        };
        if !poll.last_failed {
            return false;
        }
        let (Some(interval_ms), Some(last)) = (poll.config.interval_ms, poll.last_success_secs)
        else {
            return false;
        };
        let age_ms = now_secs.saturating_sub(last).max(0).saturating_mul(1_000);
        #[expect(clippy::cast_precision_loss, reason = "poll ages are small")]
        let stale = age_ms as f64 > f64::from(interval_ms) * f64::from(poll.config.stale_factor);
        stale
    }

    /// Never loaded and failing past `stale_factor × interval_ms` —
    /// the never-loaded counterpart to `is_stale`.
    pub(crate) fn is_offline(&self, index: usize, now_secs: i64) -> bool {
        let Some(poll) = self.poll(index) else {
            return false;
        };
        if poll.last_success_secs.is_some() {
            return false;
        }
        let (Some(interval_ms), Some(since)) = (poll.config.interval_ms, poll.failing_since_secs)
        else {
            return false;
        };
        let age_ms = now_secs.saturating_sub(since).max(0).saturating_mul(1_000);
        #[expect(clippy::cast_precision_loss, reason = "poll ages are small")]
        let offline = age_ms as f64 > f64::from(interval_ms) * f64::from(poll.config.stale_factor);
        offline
    }

    #[cfg(test)]
    fn is_pending(&self, index: usize) -> bool {
        self.poll(index).is_some_and(|poll| poll.pending)
    }

    #[cfg(test)]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the glue tests that use it run off-device")
    )]
    fn is_registered(&self, index: usize) -> bool {
        self.poll(index).is_some()
    }
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplyAction {
    Ignore,
    Reissued,
    Stopped,
    Deliver(usize),
}

#[cfg(target_arch = "wasm32")]
mod backend {
    use crate::host::HostClock as _;
    use crate::net::{self, FetchRequest};

    use super::{FetchBackend, FetchBodyDelivery, FetchSpec, Method};

    pub(super) struct HostBackend;

    impl FetchBackend for HostBackend {
        fn send(
            &mut self,
            spec: &FetchSpec,
            delay_ms: Option<u32>,
        ) -> Option<bmc_wasm_protocol::FetchRequestId> {
            let req = match spec.method {
                Method::Get => FetchRequest::get(&spec.url),
                Method::Post => FetchRequest::post(&spec.url),
                Method::Put => FetchRequest::put(&spec.url),
                Method::Delete => FetchRequest::delete(&spec.url),
            }
            .headers_opt(spec.headers.as_deref());
            let req = match spec.timeout {
                Some(timeout) => req.timeout(timeout),
                None => req,
            };
            let req = match spec.body.as_deref() {
                Some(body) => req.body(body),
                None => req,
            };
            let req = match spec.body_delivery {
                FetchBodyDelivery::Guest => req,
                FetchBodyDelivery::Host => req.host_body(),
            };
            match delay_ms {
                Some(ms) => req.send_after(ms, super::glue::trampoline),
                None => req.send(super::glue::trampoline),
            }
        }

        fn cancel(&mut self, id: bmc_wasm_protocol::FetchRequestId) -> bool {
            net::cancel(id)
        }
    }

    pub(super) fn now_secs() -> i64 {
        crate::host::SystemTime::now().unix_secs
    }
}

/// Counts sends and cancels in place of the host's fetch, handing out request ids.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod backend {
    use core::cell::Cell;

    use bmc_wasm_protocol::FetchRequestId;

    use super::{FetchBackend, FetchSpec};

    thread_local! {
        static NEXT_ID: Cell<u32> = const { Cell::new(1) };
        static SENDS: Cell<usize> = const { Cell::new(0) };
        static CANCELS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) struct HostBackend;

    impl FetchBackend for HostBackend {
        fn send(&mut self, _spec: &FetchSpec, _delay_ms: Option<u32>) -> Option<FetchRequestId> {
            SENDS.set(SENDS.get() + 1);
            let mut next = NEXT_ID.get();
            let id = FetchRequestId::alloc(&mut next);
            NEXT_ID.set(next);
            Some(id)
        }

        fn cancel(&mut self, _id: FetchRequestId) -> bool {
            CANCELS.set(CANCELS.get() + 1);
            true
        }
    }

    pub(super) fn now_secs() -> i64 {
        0
    }

    pub(super) fn sends() -> usize {
        SENDS.get()
    }

    pub(super) fn cancels() -> usize {
        CANCELS.get()
    }
}

#[cfg(any(target_arch = "wasm32", test))]
mod glue {
    use std::cell::RefCell;
    use std::mem::ManuallyDrop;

    use crate::net::FetchResponse;

    use super::backend::{HostBackend, now_secs};
    use super::{Config, Handle, Registry};

    /// Receives each reply with the borrowed handle of the poll it belongs to.
    pub type OnReply = fn(&Handle, &FetchResponse);

    thread_local! {
        pub(super) static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
        static HANDLERS: RefCell<Vec<OnReply>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn trampoline(response: &FetchResponse) {
        let action = REGISTRY.with(|r| {
            r.borrow_mut()
                .begin_reply(response.request_id, &mut HostBackend)
        });
        if let super::ReplyAction::Deliver(index) = action {
            let on_reply = HANDLERS.with(|h| h.borrow()[index]);
            // Borrowed for the reply only; dropping it would end the poll.
            on_reply(&ManuallyDrop::new(Handle(index)), response);
            let now = now_secs();
            REGISTRY.with(|r| {
                r.borrow_mut()
                    .reschedule(index, response.ok(), now, &mut HostBackend);
            });
        }
    }

    /// Register a poll and send its first request, unless `config.enabled` is false.
    ///
    /// An enabled poll's `build` runs once before this returns;
    /// a disabled one's waits until it is enabled.
    /// Returns the only handle to the poll; dropping it ends the poll.
    #[must_use = "dropping the returned Handle ends the poll"]
    pub fn register(build: super::Build, on_reply: OnReply, config: Config) -> Handle {
        let index = REGISTRY.with(|r| r.borrow_mut().register(build, config));
        HANDLERS.with(|h| {
            let mut h = h.borrow_mut();
            debug_assert_eq!(h.len(), index);
            h.push(on_reply);
        });
        REGISTRY.with(|r| r.borrow_mut().start(index, &mut HostBackend));
        Handle(index)
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            REGISTRY.with(|r| r.borrow_mut().release(self.0, &mut HostBackend));
        }
    }

    impl Handle {
        /// End the poll: cancel its live fetch, and drop a reply already on its way.
        pub fn stop(self) {
            drop(self);
        }

        pub fn set_enabled(&self, enabled: bool) {
            REGISTRY.with(|r| {
                r.borrow_mut()
                    .set_enabled(self.0, enabled, &mut HostBackend);
            });
        }

        /// Enable with the next fetch after `delay_ms` instead of the debounce.
        pub fn enable_after(&self, delay_ms: u32) {
            REGISTRY.with(|r| {
                r.borrow_mut()
                    .set_enabled_after(self.0, delay_ms, &mut HostBackend);
            });
        }

        /// Retarget the poll cadence and `is_stale` threshold; pair with `invalidate` to apply now.
        pub fn set_interval(&self, interval_ms: u32) {
            REGISTRY.with(|r| r.borrow_mut().set_interval(self.0, interval_ms));
        }

        pub fn invalidate(&self) {
            REGISTRY.with(|r| r.borrow_mut().invalidate(self.0, &mut HostBackend));
        }

        /// Treat the reply being delivered as unusable, for a body
        /// this handler cannot use despite the 2xx.
        ///
        /// The poll then reads as failing and keeps the anchor it had,
        /// and the next attempt waits `retry_ms` rather than the interval.
        pub fn retry(&self) {
            REGISTRY.with(|r| r.borrow_mut().request_retry(self.0));
        }

        /// Like [`retry`](Self::retry), including counting as no refresh,
        /// but the next attempt waits `delay_ms` instead of `retry_ms`.
        pub fn retry_after(&self, delay_ms: u32) {
            REGISTRY.with(|r| r.borrow_mut().request_retry_after(self.0, delay_ms));
        }

        /// Flag stale after a reply was banked ok (e.g. a late decode failure).
        pub fn mark_stale(&self) {
            REGISTRY.with(|r| r.borrow_mut().mark_stale(self.0));
        }

        /// Seed the last-success anchor from a restored cache entry
        /// so staleness reads the true age across a reboot.
        pub fn restore_anchor(&self, unix_secs: i64) {
            REGISTRY.with(|r| r.borrow_mut().restore_anchor(self.0, unix_secs));
        }

        /// Forget all staleness history — for when the widget blanks its data.
        pub fn reset_staleness(&self) {
            REGISTRY.with(|r| r.borrow_mut().reset_staleness(self.0));
        }

        #[must_use]
        pub fn enabled(&self) -> bool {
            REGISTRY.with(|r| r.borrow().enabled(self.0))
        }

        /// Whether the poll is failing with stale data.
        #[must_use]
        pub fn is_stale(&self) -> bool {
            let now = now_secs();
            REGISTRY.with(|r| r.borrow().is_stale(self.0, now))
        }

        /// Never loaded and failing past the stale grace.
        #[must_use]
        pub fn is_offline(&self) -> bool {
            let now = now_secs();
            REGISTRY.with(|r| r.borrow().is_offline(self.0, now))
        }

        /// Age of the last successful reply, or `None` if it has never succeeded.
        #[must_use]
        pub fn last_success_age(&self) -> Option<std::time::Duration> {
            let now = now_secs();
            REGISTRY.with(|r| r.borrow().last_success_age(self.0, now))
        }

        /// The last successful reply's instant — the relative-time anchor.
        #[must_use]
        pub fn last_success_time(&self) -> Option<crate::host::SystemTime> {
            REGISTRY.with(|r| {
                r.borrow()
                    .last_success_secs(self.0)
                    .map(|unix_secs| crate::host::SystemTime { unix_secs })
            })
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
pub use glue::{OnReply, register};

#[cfg(test)]
mod tests;
