// Copyright (C) 2026  Braiins Systems s.r.o.

#[cfg(any(target_arch = "wasm32", test))]
use std::collections::HashMap;

use bmc_wasm_protocol::FetchRequestId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchSpec {
    pub method: Method,
    pub url: String,
    pub headers: Option<String>,
    pub body: Option<Vec<u8>>,
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
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub interval_ms: Option<u32>,
    pub retry_ms: u32,
    pub debounce_ms: u32,
    pub enabled: bool,
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
        // Single-flight per poll keeps the in-flight count well under the host
        // budget, so a send should not be rejected; treat an unexpected
        // rejection as a transient failure and re-attempt after retry_ms so a
        // budget hiccup self-heals instead of leaving the poll dormant forever.
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

    /// Mark the in-flight reply as unusable so the next reschedule retries
    /// after `retry_ms` even on an HTTP-ok response. Meant to be called from a
    /// reply handler when a 2xx body can't be used (malformed / out-of-range
    /// payload) — the engine keys retry timing off the HTTP status alone and
    /// would otherwise wait the full interval before trying again.
    pub(crate) fn request_retry(&mut self, handle: Handle) {
        let delay = self.polls[handle.0].config.retry_ms;
        self.polls[handle.0].force_retry = Some(delay);
    }

    /// Like `request_retry`, but the next attempt waits `delay_ms` rather than
    /// `retry_ms`, letting a reply handler apply its own backoff.
    pub(crate) fn request_retry_after(&mut self, handle: Handle, delay_ms: u32) {
        self.polls[handle.0].force_retry = Some(delay_ms);
    }

    pub(crate) fn reschedule(&mut self, handle: Handle, ok: bool, backend: &mut dyn FetchBackend) {
        let idx = handle.0;
        let forced = std::mem::take(&mut self.polls[idx].force_retry);
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

    pub(crate) fn enabled(&self, handle: Handle) -> bool {
        self.polls[handle.0].enabled
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

    fn deliver(reg: &mut Registry, be: &mut FakeBackend, id: u32, ok: bool) -> ReplyAction {
        let id = FetchRequestId::from_wire(id).expect("BUG: test id is nonzero");
        let action = reg.begin_reply(id, be);
        if let ReplyAction::Deliver(h) = action {
            reg.reschedule(h, ok, be);
        }
        action
    }

    const CFG: Config = Config {
        interval_ms: Some(5_000),
        retry_ms: 1_000,
        debounce_ms: 300,
        enabled: true,
    };

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
        reg.reschedule(h, true, &mut be);
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
        reg.reschedule(h, true, &mut be);
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
}
