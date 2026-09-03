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

#[cfg(any(target_arch = "wasm32", test))]
use std::collections::HashMap;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(usize);

impl Handle {
    #[must_use]
    pub fn index(self) -> usize {
        self.0
    }
}

pub type Build = fn(Handle) -> Option<FetchSpec>;

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

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Default)]
pub(crate) struct Registry {
    polls: Vec<Poll>,
    routes: HashMap<FetchRequestId, usize>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl Registry {
    pub(crate) fn register(&mut self, build: Build, config: Config) -> Handle {
        let handle = Handle(self.polls.len());
        self.polls.push(Poll {
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
        });
        handle
    }

    fn kick(&mut self, idx: usize, delay_ms: Option<u32>, backend: &mut dyn FetchBackend) {
        let poll = &self.polls[idx];
        if !poll.enabled || poll.live.is_some() {
            return;
        }
        let Some(spec) = (poll.build)(Handle(idx)) else {
            return;
        };
        // Single-flight per poll keeps the in-flight count well under the host budget,
        // so a send should not be rejected; treat an unexpected rejection as a transient
        // failure and re-attempt after retry_ms so a budget hiccup self-heals instead
        // of leaving the poll dormant forever.
        let id = match backend.send(&spec, delay_ms) {
            Some(id) => Some(id),
            None => backend.send(&spec, Some(self.polls[idx].config.retry_ms)),
        };
        if let Some(id) = id {
            self.polls[idx].live = Some(id);
            self.routes.insert(id, idx);
        }
    }

    pub(crate) fn start(&mut self, handle: Handle, backend: &mut dyn FetchBackend) {
        self.kick(handle.0, None, backend);
    }

    pub(crate) fn invalidate(&mut self, handle: Handle, backend: &mut dyn FetchBackend) {
        let idx = handle.0;
        if !self.polls[idx].enabled {
            return;
        }
        let debounce = self.polls[idx].config.debounce_ms;
        match self.polls[idx].live {
            Some(id) => {
                if backend.cancel(id) {
                    self.routes.remove(&id);
                    self.polls[idx].live = None;
                    self.kick(idx, Some(debounce), backend);
                } else {
                    self.polls[idx].pending = true;
                }
            }
            None => self.kick(idx, Some(debounce), backend),
        }
    }

    pub(crate) fn begin_reply(
        &mut self,
        id: FetchRequestId,
        backend: &mut dyn FetchBackend,
    ) -> ReplyAction {
        let Some(idx) = self.routes.remove(&id) else {
            return ReplyAction::Ignore;
        };
        if self.polls[idx].live != Some(id) {
            return ReplyAction::Ignore;
        }
        self.polls[idx].live = None;
        let pending = std::mem::take(&mut self.polls[idx].pending);
        if !self.polls[idx].enabled {
            return ReplyAction::Stopped;
        }
        if pending {
            self.kick(idx, None, backend);
            return ReplyAction::Reissued;
        }
        ReplyAction::Deliver(Handle(idx))
    }

    /// Mark the in-flight reply as unusable: a 2xx whose body
    /// the handler cannot use, malformed or out of range.
    ///
    /// It then counts as no refresh: the poll reads as failing, the anchor
    /// holds where the last good reply left it, and the next attempt waits
    /// `retry_ms` rather than the poll interval.
    pub(crate) fn request_retry(&mut self, handle: Handle) {
        let delay = self.polls[handle.0].config.retry_ms;
        self.polls[handle.0].force_retry = Some(delay);
    }

    /// Like `request_retry`, but the next attempt waits `delay_ms` rather than
    /// `retry_ms`, letting a reply handler apply its own backoff.
    pub(crate) fn request_retry_after(&mut self, handle: Handle, delay_ms: u32) {
        self.polls[handle.0].force_retry = Some(delay_ms);
    }

    /// A just-banked reply proved bad (a late decode failure):
    /// flag failing and revert the anchor it displaced.
    pub(crate) fn mark_stale(&mut self, handle: Handle) {
        let poll = &mut self.polls[handle.0];
        poll.last_failed = true;
        poll.last_success_secs = poll.prev_success_secs;
    }

    /// Seed the last-success anchor from persisted state;
    /// a no-op once a live reply has banked one.
    pub(crate) fn restore_anchor(&mut self, handle: Handle, secs: i64) {
        let poll = &mut self.polls[handle.0];
        if poll.last_success_secs.is_none() {
            poll.last_success_secs = Some(secs);
        }
    }

    /// Forget all staleness history, as if the poll had never run.
    pub(crate) fn reset_staleness(&mut self, handle: Handle) {
        let poll = &mut self.polls[handle.0];
        poll.last_success_secs = None;
        poll.prev_success_secs = None;
        poll.last_failed = false;
        poll.failing_since_secs = None;
    }

    pub(crate) fn reschedule(
        &mut self,
        handle: Handle,
        ok: bool,
        now_secs: i64,
        backend: &mut dyn FetchBackend,
    ) {
        let idx = handle.0;
        let forced = std::mem::take(&mut self.polls[idx].force_retry);
        // A forced retry means the handler rejected this reply, so it is no
        // refresh however the HTTP call went.
        if ok && forced.is_none() {
            self.polls[idx].prev_success_secs = self.polls[idx].last_success_secs;
            self.polls[idx].last_success_secs = Some(now_secs);
            self.polls[idx].last_failed = false;
            self.polls[idx].failing_since_secs = None;
        } else {
            self.polls[idx].last_failed = true;
            self.polls[idx].failing_since_secs.get_or_insert(now_secs);
        }
        if !self.polls[idx].enabled || self.polls[idx].live.is_some() {
            return;
        }
        let delay = match forced {
            Some(delay_ms) => delay_ms,
            None if ok => match self.polls[idx].config.interval_ms {
                Some(ms) => ms,
                None => return,
            },
            None => self.polls[idx].config.retry_ms,
        };
        self.kick(idx, Some(delay), backend);
    }

    pub(crate) fn set_enabled(
        &mut self,
        handle: Handle,
        enabled: bool,
        backend: &mut dyn FetchBackend,
    ) {
        let idx = handle.0;
        if self.polls[idx].enabled == enabled {
            return;
        }
        self.polls[idx].enabled = enabled;
        if enabled {
            let debounce = self.polls[idx].config.debounce_ms;
            self.kick(idx, Some(debounce), backend);
        } else if let Some(id) = self.polls[idx].live
            && backend.cancel(id)
        {
            self.routes.remove(&id);
            self.polls[idx].live = None;
        }
    }

    /// Enable and schedule the next fetch after `delay_ms` (no immediate kick).
    pub(crate) fn set_enabled_after(
        &mut self,
        handle: Handle,
        delay_ms: u32,
        backend: &mut dyn FetchBackend,
    ) {
        let idx = handle.0;
        self.polls[idx].enabled = true;
        self.kick(idx, Some(delay_ms), backend);
    }

    /// Retarget the poll cadence and `is_stale` threshold; reschedule to apply.
    pub(crate) fn set_interval(&mut self, handle: Handle, interval_ms: u32) {
        self.polls[handle.0].config.interval_ms = Some(interval_ms);
    }

    pub(crate) fn enabled(&self, handle: Handle) -> bool {
        self.polls[handle.0].enabled
    }

    /// Age of the last successful reply; `None` before the first success.
    pub(crate) fn last_success_age(&self, handle: Handle, now_secs: i64) -> Option<Duration> {
        let last = self.polls[handle.0].last_success_secs?;
        let secs = now_secs.saturating_sub(last).max(0);
        #[expect(clippy::cast_sign_loss, reason = "secs is clamped to >= 0")]
        Some(Duration::from_secs(secs as u64))
    }

    /// Epoch secs of the last successful reply — the relative-time anchor.
    pub(crate) fn last_success_secs(&self, handle: Handle) -> Option<i64> {
        self.polls[handle.0].last_success_secs
    }

    /// Failing, with the last good reply aged past `stale_factor × interval_ms`.
    pub(crate) fn is_stale(&self, handle: Handle, now_secs: i64) -> bool {
        let poll = &self.polls[handle.0];
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
    pub(crate) fn is_offline(&self, handle: Handle, now_secs: i64) -> bool {
        let poll = &self.polls[handle.0];
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
    fn is_pending(&self, handle: Handle) -> bool {
        self.polls[handle.0].pending
    }
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplyAction {
    Ignore,
    Reissued,
    Stopped,
    Deliver(Handle),
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;

    use crate::net::{self, FetchRequest, FetchResponse};

    use super::{Config, FetchBackend, FetchBodyDelivery, FetchSpec, Handle, Method, Registry};

    pub type OnReply = fn(Handle, &FetchResponse);

    struct ProdBackend;

    impl FetchBackend for ProdBackend {
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
                Some(ms) => req.send_after(ms, trampoline),
                None => req.send(trampoline),
            }
        }

        fn cancel(&mut self, id: bmc_wasm_protocol::FetchRequestId) -> bool {
            net::cancel(id)
        }
    }

    thread_local! {
        static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
        static HANDLERS: RefCell<Vec<OnReply>> = const { RefCell::new(Vec::new()) };
    }

    fn trampoline(response: &FetchResponse) {
        let action = REGISTRY.with(|r| {
            r.borrow_mut()
                .begin_reply(response.request_id, &mut ProdBackend)
        });
        if let super::ReplyAction::Deliver(handle) = action {
            let on_reply = HANDLERS.with(|h| h.borrow()[handle.index()]);
            on_reply(handle, response);
            let now = crate::host::SystemTime::now().unix_secs;
            REGISTRY.with(|r| {
                r.borrow_mut()
                    .reschedule(handle, response.ok(), now, &mut ProdBackend);
            });
        }
    }

    #[must_use]
    pub fn register(build: super::Build, on_reply: OnReply, config: Config) -> Handle {
        let handle = REGISTRY.with(|r| r.borrow_mut().register(build, config));
        HANDLERS.with(|h| {
            let mut h = h.borrow_mut();
            debug_assert_eq!(h.len(), handle.index());
            h.push(on_reply);
        });
        REGISTRY.with(|r| r.borrow_mut().start(handle, &mut ProdBackend));
        handle
    }

    impl Handle {
        pub fn set_enabled(self, enabled: bool) {
            REGISTRY.with(|r| r.borrow_mut().set_enabled(self, enabled, &mut ProdBackend));
        }

        /// Enable with the next fetch after `delay_ms` instead of the debounce.
        pub fn enable_after(self, delay_ms: u32) {
            REGISTRY.with(|r| {
                r.borrow_mut()
                    .set_enabled_after(self, delay_ms, &mut ProdBackend)
            });
        }

        /// Retarget the poll cadence and `is_stale` threshold; pair with `invalidate` to apply now.
        pub fn set_interval(self, interval_ms: u32) {
            REGISTRY.with(|r| r.borrow_mut().set_interval(self, interval_ms));
        }

        pub fn invalidate(self) {
            REGISTRY.with(|r| r.borrow_mut().invalidate(self, &mut ProdBackend));
        }

        /// Treat the reply being delivered as unusable, for a body
        /// this handler cannot use despite the 2xx.
        ///
        /// The poll then reads as failing and keeps the anchor it had,
        /// and the next attempt waits `retry_ms` rather than the interval.
        pub fn retry(self) {
            REGISTRY.with(|r| r.borrow_mut().request_retry(self));
        }

        /// Like [`retry`](Self::retry), including counting as no refresh,
        /// but the next attempt waits `delay_ms` instead of `retry_ms`.
        pub fn retry_after(self, delay_ms: u32) {
            REGISTRY.with(|r| r.borrow_mut().request_retry_after(self, delay_ms));
        }

        /// Flag stale after a reply was banked ok (e.g. a late decode failure).
        pub fn mark_stale(self) {
            REGISTRY.with(|r| r.borrow_mut().mark_stale(self));
        }

        /// Seed the last-success anchor from a restored cache entry
        /// so staleness reads the true age across a reboot.
        pub fn restore_anchor(self, unix_secs: i64) {
            REGISTRY.with(|r| r.borrow_mut().restore_anchor(self, unix_secs));
        }

        /// Forget all staleness history — for when the widget blanks its data.
        pub fn reset_staleness(self) {
            REGISTRY.with(|r| r.borrow_mut().reset_staleness(self));
        }

        #[must_use]
        pub fn enabled(self) -> bool {
            REGISTRY.with(|r| r.borrow().enabled(self))
        }

        /// Whether the poll is failing with stale data.
        #[must_use]
        pub fn is_stale(self) -> bool {
            let now = crate::host::SystemTime::now().unix_secs;
            REGISTRY.with(|r| r.borrow().is_stale(self, now))
        }

        /// Never loaded and failing past the stale grace.
        #[must_use]
        pub fn is_offline(self) -> bool {
            let now = crate::host::SystemTime::now().unix_secs;
            REGISTRY.with(|r| r.borrow().is_offline(self, now))
        }

        /// Age of the last successful reply, or `None` if it has never succeeded.
        #[must_use]
        pub fn last_success_age(self) -> Option<std::time::Duration> {
            let now = crate::host::SystemTime::now().unix_secs;
            REGISTRY.with(|r| r.borrow().last_success_age(self, now))
        }

        /// The last successful reply's instant — the relative-time anchor.
        #[must_use]
        pub fn last_success_time(self) -> Option<crate::host::SystemTime> {
            REGISTRY.with(|r| {
                r.borrow()
                    .last_success_secs(self)
                    .map(|unix_secs| crate::host::SystemTime { unix_secs })
            })
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{OnReply, register};

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        next_id: u32,
        sends: Vec<(FetchSpec, Option<u32>)>,
        cancels: Vec<FetchRequestId>,
        cancel_result: bool,
        reject_next_sends: usize,
    }

    impl FakeBackend {
        fn new() -> Self {
            Self {
                next_id: 1,
                cancel_result: true,
                ..Self::default()
            }
        }
    }

    impl FetchBackend for FakeBackend {
        fn send(&mut self, spec: &FetchSpec, delay_ms: Option<u32>) -> Option<FetchRequestId> {
            self.sends.push((spec.clone(), delay_ms));
            if self.reject_next_sends > 0 {
                self.reject_next_sends -= 1;
                return None;
            }
            let id = FetchRequestId::alloc(&mut self.next_id);
            Some(id)
        }
        fn cancel(&mut self, id: FetchRequestId) -> bool {
            self.cancels.push(id);
            self.cancel_result
        }
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "must match the Build fn-pointer signature, which returns Option"
    )]
    fn build_url(_h: Handle) -> Option<FetchSpec> {
        Some(FetchSpec::get("http://x/data"))
    }

    fn build_none(_h: Handle) -> Option<FetchSpec> {
        None
    }

    thread_local! {
        static TOGGLE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    }

    fn build_toggle(_h: Handle) -> Option<FetchSpec> {
        TOGGLE
            .with(std::cell::Cell::get)
            .then(|| FetchSpec::get("http://x/data"))
    }

    fn deliver_at(
        reg: &mut Registry,
        be: &mut FakeBackend,
        id: u32,
        ok: bool,
        now_secs: i64,
    ) -> ReplyAction {
        let id = FetchRequestId::from_wire(id).expect("BUG: test id is nonzero");
        let action = reg.begin_reply(id, be);
        if let ReplyAction::Deliver(h) = action {
            reg.reschedule(h, ok, now_secs, be);
        }
        action
    }

    fn deliver(reg: &mut Registry, be: &mut FakeBackend, id: u32, ok: bool) -> ReplyAction {
        deliver_at(reg, be, id, ok, 0)
    }

    const CFG: Config = Config {
        interval_ms: Some(5_000),
        retry_ms: 1_000,
        debounce_ms: 300,
        enabled: true,
        stale_factor: 1.5,
    };

    // Poll fetches must inherit the SDK-wide default unless the builder opts
    // into a different per-call timeout, e.g. a short one for LAN devices.
    #[test]
    fn spec_timeout_defaults_to_sdk_default_until_overridden() {
        assert_eq!(FetchSpec::get("http://x/data").timeout, None);
        let spec = FetchSpec::get("http://x/data").timeout(Duration::from_millis(100));
        assert_eq!(spec.timeout, Some(Duration::from_millis(100)));
    }

    #[test]
    fn start_sends_immediately_when_enabled() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        assert_eq!(be.sends.len(), 1);
        assert_eq!(be.sends[0].1, None);
        assert_eq!(be.sends[0].0.url, "http://x/data");
    }

    #[test]
    fn start_does_nothing_when_disabled() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(
            build_url,
            Config {
                enabled: false,
                ..CFG
            },
        );
        reg.start(h, &mut be);
        assert!(be.sends.is_empty());
    }

    #[test]
    fn rejected_send_retries_after_retry_ms() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        be.reject_next_sends = 1;
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        assert_eq!(be.sends.len(), 2);
        assert_eq!(be.sends[0].1, None);
        assert_eq!(be.sends[1].1, Some(1_000));
    }

    #[test]
    fn invalidate_with_queued_request_cancels_and_reschedules_after_debounce() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        reg.invalidate(h, &mut be);
        assert_eq!(be.cancels.len(), 1);
        assert_eq!(be.sends.len(), 2);
        assert_eq!(be.sends[1].1, Some(300));
    }

    #[test]
    fn invalidate_with_in_flight_request_marks_pending_and_does_not_send() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        be.cancel_result = false;
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        reg.invalidate(h, &mut be);
        assert_eq!(be.cancels.len(), 1);
        assert_eq!(be.sends.len(), 1);
        assert!(reg.is_pending(h));
    }

    #[test]
    fn invalidate_when_idle_kicks_after_debounce() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        // Enabled but never started, so the slot is idle with no live request.
        let h = reg.register(build_url, CFG);
        reg.invalidate(h, &mut be);
        assert_eq!(be.sends.len(), 1);
        assert_eq!(be.sends[0].1, Some(300));
    }

    #[test]
    fn successful_reply_reschedules_after_interval() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        let action = deliver(&mut reg, &mut be, 1, true);
        assert_eq!(action, ReplyAction::Deliver(h));
        assert_eq!(be.sends.len(), 2);
        assert_eq!(be.sends[1].1, Some(5_000));
    }

    #[test]
    fn set_interval_retargets_the_reschedule_cadence() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG); // interval 5_000
        reg.start(h, &mut be);
        reg.set_interval(h, 60_000);
        deliver(&mut reg, &mut be, 1, true);
        assert_eq!(
            be.sends.last().expect("BUG: a fetch was scheduled").1,
            Some(60_000),
            "a success reschedules at the new interval"
        );
    }

    #[test]
    fn one_shot_reply_does_not_reschedule() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(
            build_url,
            Config {
                interval_ms: None,
                ..CFG
            },
        );
        reg.start(h, &mut be);
        deliver(&mut reg, &mut be, 1, true);
        assert_eq!(be.sends.len(), 1);
    }

    #[test]
    fn failed_reply_reschedules_after_retry() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        deliver(&mut reg, &mut be, 1, false);
        assert_eq!(be.sends.len(), 2);
        assert_eq!(be.sends[1].1, Some(1_000));
    }

    #[test]
    fn retry_request_reschedules_after_retry_despite_ok() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        // A 200 whose payload the handler couldn't use: it calls retry()
        // before the engine reschedules, so the next attempt must come after
        // retry_ms rather than the full interval.
        let id = FetchRequestId::from_wire(1).expect("BUG: test id is nonzero");
        assert_eq!(reg.begin_reply(id, &mut be), ReplyAction::Deliver(h));
        reg.request_retry(h);
        reg.reschedule(h, true, 0, &mut be);
        assert_eq!(be.sends.len(), 2);
        assert_eq!(be.sends[1].1, Some(1_000));
    }

    #[test]
    fn retry_after_reschedules_with_the_requested_delay_despite_ok() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        // A reply handler applying its own backoff: the next attempt must wait
        // the requested delay, not retry_ms and not the poll interval.
        let id = FetchRequestId::from_wire(1).expect("BUG: test id is nonzero");
        assert_eq!(reg.begin_reply(id, &mut be), ReplyAction::Deliver(h));
        reg.request_retry_after(h, 42_000);
        reg.reschedule(h, true, 0, &mut be);
        assert_eq!(be.sends.len(), 2);
        assert_eq!(be.sends[1].1, Some(42_000));
    }

    #[test]
    fn reply_to_unknown_id_is_ignored() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let id = FetchRequestId::from_wire(99).expect("BUG: nonzero");
        assert_eq!(reg.begin_reply(id, &mut be), ReplyAction::Ignore);
    }

    #[test]
    fn pending_reply_reissues_immediately_and_skips_delivery() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        be.cancel_result = false;
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        reg.invalidate(h, &mut be);
        let action = deliver(&mut reg, &mut be, 1, true);
        assert_eq!(action, ReplyAction::Reissued);
        assert_eq!(be.sends.len(), 2);
        assert_eq!(be.sends[1].1, None);
    }

    #[test]
    fn builder_none_sends_nothing_on_start() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_none, CFG);
        reg.start(h, &mut be);
        assert!(be.sends.is_empty());
    }

    #[test]
    fn builder_none_on_reschedule_goes_dormant() {
        TOGGLE.with(|t| t.set(true));
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_toggle, CFG);
        reg.start(h, &mut be);
        TOGGLE.with(|t| t.set(false));
        deliver(&mut reg, &mut be, 1, true);
        assert_eq!(be.sends.len(), 1);
    }

    #[test]
    fn rapid_invalidations_cancel_the_queued_send_and_keep_one() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        reg.invalidate(h, &mut be);
        reg.invalidate(h, &mut be);
        assert_eq!(be.cancels.len(), 2);
        assert_eq!(be.sends.len(), 3);
        assert_eq!(be.sends[2].1, Some(300));
    }

    #[test]
    fn enable_from_disabled_kicks_after_debounce() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(
            build_url,
            Config {
                enabled: false,
                ..CFG
            },
        );
        reg.set_enabled(h, true, &mut be);
        assert_eq!(be.sends.len(), 1);
        assert_eq!(be.sends[0].1, Some(300));
    }

    #[test]
    fn disable_cancels_queued_request() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        reg.set_enabled(h, false, &mut be);
        assert_eq!(be.cancels.len(), 1);
        assert!(!reg.enabled(h));
    }

    #[test]
    fn set_enabled_is_idempotent() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        reg.set_enabled(h, true, &mut be);
        assert_eq!(be.sends.len(), 1);
    }

    #[test]
    fn enable_after_schedules_at_the_given_delay() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(
            build_url,
            Config {
                enabled: false,
                ..CFG
            },
        );
        reg.set_enabled_after(h, 7_000, &mut be);
        assert!(reg.enabled(h));
        assert_eq!(be.sends.len(), 1);
        assert_eq!(be.sends[0].1, Some(7_000));
    }

    #[test]
    fn reply_to_disabled_poll_stops_without_reschedule() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        be.cancel_result = false;
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        reg.set_enabled(h, false, &mut be);
        let action = deliver(&mut reg, &mut be, 1, true);
        assert_eq!(action, ReplyAction::Stopped);
        assert_eq!(be.sends.len(), 1);
    }

    #[test]
    fn not_stale_while_replies_succeed() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        deliver_at(&mut reg, &mut be, 1, true, 0);
        assert!(
            !reg.is_stale(h, 1_000_000),
            "no outstanding failure is never stale"
        );
    }

    #[test]
    fn is_stale_once_failure_ages_past_stale_factor() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        deliver_at(&mut reg, &mut be, 1, true, 0);
        deliver_at(&mut reg, &mut be, 2, false, 8);
        assert!(!reg.is_stale(h, 7), "age 7 s is within the 7.5 s threshold");
        assert!(reg.is_stale(h, 8), "age 8 s exceeds the 7.5 s threshold");
    }

    #[test]
    fn success_clears_staleness() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        deliver_at(&mut reg, &mut be, 1, true, 0);
        deliver_at(&mut reg, &mut be, 2, false, 8);
        assert!(reg.is_stale(h, 8), "stale once the failure ages out");
        deliver_at(&mut reg, &mut be, 3, true, 20);
        assert!(!reg.is_stale(h, 100), "a fresh success clears the failure");
        assert_eq!(reg.last_success_age(h, 25), Some(Duration::from_secs(5)));
        assert_eq!(reg.last_success_secs(h), Some(20));
    }

    #[test]
    fn forced_retry_does_not_count_as_a_refresh() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        deliver_at(&mut reg, &mut be, 1, true, 0);
        // A 2xx the handler rejected via retry(): no refresh despite ok=true.
        let id = FetchRequestId::from_wire(2).expect("BUG: test id is nonzero");
        assert_eq!(reg.begin_reply(id, &mut be), ReplyAction::Deliver(h));
        reg.request_retry(h);
        reg.reschedule(h, true, 100, &mut be);
        assert_eq!(
            reg.last_success_secs(h),
            Some(0),
            "a rejected reply keeps the previous anchor"
        );
        assert!(
            reg.is_stale(h, 100),
            "a rejected reply leaves the poll stale"
        );
    }

    #[test]
    fn mark_stale_reverts_the_corrupt_reply_to_the_last_good_anchor() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        deliver_at(&mut reg, &mut be, 1, true, 0); // good image banked at t=0
        // A later 2xx banks ok (header passed) but its decode fails async.
        deliver_at(&mut reg, &mut be, 2, true, 100);
        assert_eq!(reg.last_success_secs(h), Some(100), "the reply banked ok");
        reg.mark_stale(h);
        assert_eq!(
            reg.last_success_secs(h),
            Some(0),
            "the anchor reverts to the last good reply, not the corrupt one"
        );
        assert!(
            reg.is_stale(h, 100),
            "and it reads stale from the good anchor"
        );
    }

    #[test]
    fn restore_anchor_seeds_only_before_the_first_success() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        // A restored anchor lets a later failed refresh read stale.
        reg.restore_anchor(h, 100);
        assert_eq!(reg.last_success_secs(h), Some(100));
        deliver_at(&mut reg, &mut be, 1, false, 200);
        assert!(
            reg.is_stale(h, 200),
            "failure over a restored anchor is stale"
        );
        // A live success wins; a later restore can't drag the anchor back.
        deliver_at(&mut reg, &mut be, 2, true, 300);
        reg.restore_anchor(h, 100);
        assert_eq!(
            reg.last_success_secs(h),
            Some(300),
            "a live success is never clobbered"
        );
    }

    #[test]
    fn reset_staleness_forgets_all_history() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG);
        reg.start(h, &mut be);
        deliver_at(&mut reg, &mut be, 1, true, 0); // loaded
        deliver_at(&mut reg, &mut be, 2, false, 100); // then failing → stale
        assert!(reg.is_stale(h, 100));
        reg.reset_staleness(h);
        assert_eq!(reg.last_success_secs(h), None, "anchor cleared");
        assert!(!reg.is_stale(h, 1_000_000), "no failure survives the reset");
        assert!(
            !reg.is_offline(h, 1_000_000),
            "no offline state survives it"
        );
    }

    #[test]
    fn is_offline_after_failing_past_grace_without_ever_loading() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(build_url, CFG); // interval 5s, stale_factor 1.5 → 7.5s grace
        reg.start(h, &mut be);
        assert!(
            !reg.is_offline(h, 100),
            "a still-loading poll is not offline"
        );
        deliver_at(&mut reg, &mut be, 1, false, 0);
        assert!(!reg.is_offline(h, 7), "within the grace, not yet offline");
        assert!(
            reg.is_offline(h, 8),
            "past the grace, a never-loaded failure is offline"
        );
        // Once it loads it is never offline again — that is is_stale's domain.
        deliver_at(&mut reg, &mut be, 2, true, 10);
        assert!(
            !reg.is_offline(h, 1_000_000),
            "a source that has loaded is not offline"
        );
    }

    #[test]
    fn not_stale_without_prior_success_or_interval() {
        let mut reg = Registry::default();
        let mut be = FakeBackend::new();
        let h = reg.register(
            build_url,
            Config {
                interval_ms: None,
                ..CFG
            },
        );
        reg.start(h, &mut be);
        deliver_at(&mut reg, &mut be, 1, false, 0);
        assert!(
            !reg.is_stale(h, 1_000_000),
            "one-shot polls never stale on a cadence"
        );
        assert_eq!(reg.last_success_age(h, 5), None);
    }
}
