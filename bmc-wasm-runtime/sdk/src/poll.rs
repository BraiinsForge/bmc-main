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
mod tests;
