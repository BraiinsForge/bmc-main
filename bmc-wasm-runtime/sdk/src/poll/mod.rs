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
//! [`Handle::register`], `PollHandle::register` at the crate root,
//! starts a poll whose build and replies run methods on its receiver,
//! and returns the only [`Handle`] to it.
//! The SDK sends the poll's request, hands each reply to its reply method
//! with the borrowed handle, and sends the next request on the [`Config`] cadence.
//! Dropping the handle, or calling `Handle::stop`, ends the poll and cancels its live fetch.
//! The free `register`, re-exported as `register_poll`, takes plain functions,
//! and stays only so that a widget not yet ported still builds.

#[cfg(any(target_arch = "wasm32", test))]
use std::collections::HashMap;
#[cfg(any(target_arch = "wasm32", test))]
use std::rc::Rc;
use std::time::Duration;

use bmc_wasm_protocol::FetchRequestId;

#[cfg(any(target_arch = "wasm32", test))]
use crate::net::FetchResponse;

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
/// It may read the handle, disable or end the poll, and call what asks for no request,
/// such as `set_interval`.
/// Asking any poll for a request panics: `invalidate`, `enable_after`, `set_enabled(true)`
/// or registering a poll would build again before this request goes out.
/// So does `retry` or `retry_after`, which mark the reply being delivered, and a build has none.
pub type Build = fn(&Handle) -> Option<FetchSpec>;

#[cfg(any(target_arch = "wasm32", test))]
type BuildFn = Rc<dyn Fn(&Handle) -> Option<FetchSpec>>;

#[cfg(any(target_arch = "wasm32", test))]
type OnReplyFn = Rc<dyn Fn(&Handle, &FetchResponse)>;

/// When a kick may run the poll's `build`.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuildTiming {
    /// A plain function: where the kick is asked for.
    Now,
    /// A widget method: once the method or callback that asked for the request has returned,
    /// before the export ends.
    AtExportEnd,
}

pub trait FetchBackend {
    fn send(&mut self, spec: &FetchSpec, delay_ms: Option<u32>) -> Option<FetchRequestId>;
    fn cancel(&mut self, id: FetchRequestId) -> bool;
}

/// Whether a poll is enabled, and what became of its request.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flight {
    /// Enabled, nothing in flight: the next kick sends.
    Idle,
    /// Enabled, a request in flight: its reply is delivered.
    Live(FetchRequestId),
    /// Enabled, with a request the host would not cancel for an `invalidate` or a re-enable.
    /// Its reply is dropped; the request is built again after the delay the latest call asked for.
    Superseded { id: FetchRequestId, delay_ms: u32 },
    /// Disabled, nothing in flight.
    Disabled,
    /// Disabled, with a request the host would not cancel: its reply is dropped.
    Stopping(FetchRequestId),
}

#[cfg(any(target_arch = "wasm32", test))]
impl Flight {
    fn is_enabled(self) -> bool {
        !matches!(self, Flight::Disabled | Flight::Stopping(_))
    }

    /// A request kept across a disable is superseded,
    /// as enabling promises a fresh `build`.
    fn enable(self, delay_ms: u32) -> Self {
        match self {
            Flight::Disabled => Flight::Idle,
            Flight::Stopping(id) | Flight::Superseded { id, .. } => {
                Flight::Superseded { id, delay_ms }
            }
            Flight::Idle | Flight::Live(_) => self,
        }
    }
}

/// How the poll's delivered replies have gone.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Health {
    /// The last delivered reply was good, or none has come yet.
    Ok,
    /// Failing since this epoch second; anchors the offline grace.
    Failing { since: i64 },
}

#[cfg(any(target_arch = "wasm32", test))]
impl Health {
    /// A failure at `now_secs`, continuing a streak already running.
    fn failing_at(self, now_secs: i64) -> Self {
        match self {
            Health::Ok => Health::Failing { since: now_secs },
            Health::Failing { .. } => self,
        }
    }
}

/// What an ok reply replaced, so that reply can be undone if it proves bad.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug)]
struct Undo {
    last_success_secs: Option<i64>,
    health: Health,
}

#[cfg(any(target_arch = "wasm32", test))]
struct Poll {
    build: BuildFn,
    on_reply: OnReplyFn,
    build_timing: BuildTiming,
    config: Config,
    flight: Flight,
    force_retry: Option<u32>,
    /// Epoch secs of the last ok reply; `None` before the first success.
    last_success_secs: Option<i64>,
    health: Health,
    /// What the last ok reply replaced; `None` before one, once it was undone, and after a reset.
    undo: Option<Undo>,
    /// Counts the kicks asked for; a kick performed after a newer one was asked for yields to it.
    revision: usize,
}

#[cfg(any(target_arch = "wasm32", test))]
impl Poll {
    /// Count the last ok reply as a failure after all, once, even with failed replies banked since;
    /// with none left to undo, count a failure at `now_secs`.
    fn undo_success(&mut self, now_secs: i64) {
        let failing = self.health.failing_at(now_secs);
        self.health = match self.undo.take() {
            Some(undo) => {
                self.last_success_secs = undo.last_success_secs;
                // A streak the undone reply broke runs on from its start.
                match undo.health {
                    Health::Ok => failing,
                    Health::Failing { .. } => undo.health,
                }
            }
            None => failing,
        };
    }
}

/// A request the registry wants sent, performed outside its borrow:
/// `build` runs between [`Registry::prepare`] and [`Registry::finish`].
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kick {
    index: usize,
    delay_ms: Option<u32>,
    build_timing: BuildTiming,
    /// The poll's kick count when this one was asked for.
    revision: usize,
}

#[cfg(any(target_arch = "wasm32", test))]
fn request_kick(poll: &mut Poll, index: usize, delay_ms: Option<u32>) -> Kick {
    poll.revision += 1;
    Kick {
        index,
        delay_ms,
        build_timing: poll.build_timing,
        revision: poll.revision,
    }
}

/// Whether a poll's `build` is running.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Activity {
    #[default]
    Idle,
    Building,
}

/// An ended poll's slot stays empty for good:
/// reusing it would hand its index to a new poll,
/// and widgets key their own state on [`Handle::index`].
/// The cost is one empty slot per ended poll for the widget's lifetime,
/// which only matters to a widget that registers polls over and over.
///
/// Nothing may call into the widget while the registry is borrowed.
/// A call that can lead to `build` returns a [`Kick`] for the glue to perform once the borrow ends;
/// a cancel reaches only the host, so it runs in place.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Default)]
pub(crate) struct Registry {
    polls: Vec<Option<Poll>>,
    routes: HashMap<FetchRequestId, usize>,
    activity: Activity,
}

#[cfg(any(target_arch = "wasm32", test))]
fn poll_mut(polls: &mut [Option<Poll>], index: usize) -> Option<&mut Poll> {
    polls.get_mut(index)?.as_mut()
}

#[cfg(any(target_arch = "wasm32", test))]
impl Registry {
    pub(crate) fn register(
        &mut self,
        build: impl Fn(&Handle) -> Option<FetchSpec> + 'static,
        on_reply: impl Fn(&Handle, &FetchResponse) + 'static,
        build_timing: BuildTiming,
        config: Config,
    ) -> usize {
        let index = self.polls.len();
        self.polls.push(Some(Poll {
            build: Rc::new(build),
            on_reply: Rc::new(on_reply),
            build_timing,
            config,
            flight: if config.enabled {
                Flight::Idle
            } else {
                Flight::Disabled
            },
            force_retry: None,
            last_success_secs: None,
            health: Health::Ok,
            undo: None,
            revision: 0,
        }));
        index
    }

    /// End the poll: cancel its live fetch and stop routing replies to it.
    fn release(&mut self, index: usize, backend: &mut dyn FetchBackend) {
        let Some(poll) = self.polls.get_mut(index).and_then(Option::take) else {
            return;
        };
        // A reply the host already queued finds no route and is dropped.
        match poll.flight {
            Flight::Live(id) => {
                backend.cancel(id);
                self.routes.remove(&id);
            }
            Flight::Superseded { id, .. } | Flight::Stopping(id) => {
                self.routes.remove(&id);
            }
            Flight::Idle | Flight::Disabled => {}
        }
    }

    fn poll(&self, index: usize) -> Option<&Poll> {
        self.polls.get(index)?.as_ref()
    }

    /// Refuse a request asked for from inside a `build`:
    /// it would build again before that build's request goes out, and could loop for good.
    fn refuse_while_building(&self) {
        assert!(
            self.activity == Activity::Idle,
            "BUG: a poll's build asked a poll for a request (invalidate, enable_after, set_enabled(true) or a registration)"
        );
    }

    /// Refuse a retry from inside a `build`: it marks the reply being delivered,
    /// so with none, it would fail the next reply, whatever that reply carries.
    fn refuse_retry_while_building(&self) {
        assert!(
            self.activity == Activity::Idle,
            "BUG: a poll's build retried a poll, and a build has no reply to retry"
        );
    }

    /// Ask for the poll's next request after `delay_ms`;
    /// `None` when the poll is gone, disabled or has a request live.
    fn kick(&mut self, index: usize, delay_ms: Option<u32>) -> Option<Kick> {
        let poll = poll_mut(&mut self.polls, index)?;
        if poll.flight != Flight::Idle {
            return None;
        }
        Some(request_kick(poll, index, delay_ms))
    }

    /// The poll `kick` is for, unless the kick went stale:
    /// the poll ended, was disabled, or asked for a newer kick.
    fn current(&self, kick: &Kick) -> Option<&Poll> {
        let poll = self.poll(kick.index)?;
        (poll.flight == Flight::Idle && poll.revision == kick.revision).then_some(poll)
    }

    /// The `build` to run for `kick`, or `None` for a stale kick.
    ///
    /// Until [`finish`](Self::finish), asking any poll for a request panics.
    pub(crate) fn prepare(&mut self, kick: &Kick) -> Option<BuildFn> {
        let build = Rc::clone(&self.current(kick)?.build);
        self.activity = Activity::Building;
        Some(build)
    }

    /// Send the request `build` made for `kick`, unless the kick went stale meanwhile.
    pub(crate) fn finish(
        &mut self,
        kick: &Kick,
        spec: Option<&FetchSpec>,
        backend: &mut dyn FetchBackend,
    ) {
        self.activity = Activity::Idle;
        let (Some(spec), Some(poll)) = (spec, self.current(kick)) else {
            return;
        };
        let retry_ms = poll.config.retry_ms;
        // Single-flight per poll keeps the in-flight count well under the host budget,
        // so a send should not be rejected; treat an unexpected rejection as a transient
        // failure and re-attempt after retry_ms so a budget hiccup self-heals instead
        // of leaving the poll dormant forever.
        let id = match backend.send(spec, kick.delay_ms) {
            Some(id) => Some(id),
            None => backend.send(spec, Some(retry_ms)),
        };
        if let Some(id) = id
            && let Some(poll) = poll_mut(&mut self.polls, kick.index)
        {
            poll.flight = Flight::Live(id);
            self.routes.insert(id, kick.index);
        }
    }

    pub(crate) fn start(&mut self, index: usize) -> Option<Kick> {
        self.refuse_while_building();
        self.kick(index, None)
    }

    /// Ask for a fresh request: cancel the live one in place, or, if the host
    /// would not cancel it, re-issue once its reply arrives.
    pub(crate) fn invalidate(
        &mut self,
        index: usize,
        backend: &mut dyn FetchBackend,
    ) -> Option<Kick> {
        self.refuse_while_building();
        let poll = poll_mut(&mut self.polls, index)?;
        let debounce = poll.config.debounce_ms;
        match poll.flight {
            Flight::Idle => {}
            Flight::Live(id) => {
                if !backend.cancel(id) {
                    poll.flight = Flight::Superseded {
                        id,
                        delay_ms: debounce,
                    };
                    return None;
                }
                poll.flight = Flight::Idle;
                self.routes.remove(&id);
            }
            Flight::Superseded { id, .. } => {
                poll.flight = Flight::Superseded {
                    id,
                    delay_ms: debounce,
                };
                return None;
            }
            Flight::Disabled | Flight::Stopping(_) => return None,
        }
        self.kick(index, Some(debounce))
    }

    pub(crate) fn begin_reply(&mut self, id: FetchRequestId) -> ReplyAction {
        let Some(index) = self.routes.remove(&id) else {
            return ReplyAction::Ignore;
        };
        let Some(poll) = poll_mut(&mut self.polls, index) else {
            return ReplyAction::Ignore;
        };
        match poll.flight {
            Flight::Live(live) if live == id => {
                poll.flight = Flight::Idle;
                ReplyAction::Deliver(index, Rc::clone(&poll.on_reply))
            }
            Flight::Superseded { id: live, delay_ms } if live == id => {
                poll.flight = Flight::Idle;
                ReplyAction::Reissued(request_kick(poll, index, Some(delay_ms)))
            }
            Flight::Stopping(live) if live == id => {
                poll.flight = Flight::Disabled;
                ReplyAction::Stopped
            }
            Flight::Idle
            | Flight::Live(_)
            | Flight::Superseded { .. }
            | Flight::Disabled
            | Flight::Stopping(_) => ReplyAction::Ignore,
        }
    }

    /// Mark the in-flight reply as unusable: a 2xx whose body
    /// the handler cannot use, malformed or out of range.
    ///
    /// It then counts as no refresh: the poll reads as failing, the anchor
    /// holds where the last good reply left it, and the next attempt waits
    /// `retry_ms` rather than the poll interval.
    pub(crate) fn request_retry(&mut self, index: usize) {
        self.refuse_retry_while_building();
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.force_retry = Some(poll.config.retry_ms);
        }
    }

    /// Like `request_retry`, but the next attempt waits `delay_ms` rather than
    /// `retry_ms`, letting a reply handler apply its own backoff.
    pub(crate) fn request_retry_after(&mut self, index: usize, delay_ms: u32) {
        self.refuse_retry_while_building();
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.force_retry = Some(delay_ms);
        }
    }

    /// The last ok reply proved bad (a late decode failure); see `Poll::undo_success`.
    pub(crate) fn mark_stale(&mut self, index: usize, now_secs: i64) {
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.undo_success(now_secs);
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
            poll.health = Health::Ok;
            poll.undo = None;
        }
    }

    /// Record the delivered reply as a refresh or a failure,
    /// and return how long the next attempt waits; `None` for no next attempt.
    pub(crate) fn bank_reply(&mut self, index: usize, ok: bool, now_secs: i64) -> Option<u32> {
        let poll = poll_mut(&mut self.polls, index)?;
        let forced = std::mem::take(&mut poll.force_retry);
        // A forced retry means the handler rejected this reply, so it is no
        // refresh however the HTTP call went.
        if ok && forced.is_none() {
            poll.undo = Some(Undo {
                last_success_secs: poll.last_success_secs,
                health: poll.health,
            });
            poll.last_success_secs = Some(now_secs);
            poll.health = Health::Ok;
        } else {
            poll.health = poll.health.failing_at(now_secs);
        }
        match forced {
            Some(delay_ms) => Some(delay_ms),
            None if ok => poll.config.interval_ms,
            None => Some(poll.config.retry_ms),
        }
    }

    /// Ask for the attempt after a banked reply, unless a request went out meanwhile.
    ///
    /// A retry asked for since the banking, by a widget the reply's handler notified,
    /// rejects the reply after all: a refresh it banked is undone, and the retry's delay wins.
    pub(crate) fn kick_after(
        &mut self,
        index: usize,
        banked_delay_ms: Option<u32>,
        now_secs: i64,
    ) -> Option<Kick> {
        let poll = poll_mut(&mut self.polls, index)?;
        let delay_ms = match poll.force_retry.take() {
            Some(forced) => {
                // Only an ok reply banked a success to undo;
                // after a failed one, the undo record is an earlier reply's.
                if poll.health == Health::Ok {
                    poll.undo_success(now_secs);
                }
                forced
            }
            None => banked_delay_ms?,
        };
        self.kick(index, Some(delay_ms))
    }

    /// Enable the poll and ask for a request, or disable it and cancel the live one in place.
    pub(crate) fn set_enabled(
        &mut self,
        index: usize,
        enabled: bool,
        backend: &mut dyn FetchBackend,
    ) -> Option<Kick> {
        if enabled {
            self.refuse_while_building();
        }
        let poll = poll_mut(&mut self.polls, index)?;
        if enabled {
            if poll.flight.is_enabled() {
                return None;
            }
            let debounce = poll.config.debounce_ms;
            poll.flight = poll.flight.enable(debounce);
            return self.kick(index, Some(debounce));
        }
        match poll.flight {
            Flight::Idle => poll.flight = Flight::Disabled,
            Flight::Live(id) => {
                if backend.cancel(id) {
                    poll.flight = Flight::Disabled;
                    self.routes.remove(&id);
                } else {
                    poll.flight = Flight::Stopping(id);
                }
            }
            Flight::Superseded { id, .. } => poll.flight = Flight::Stopping(id),
            Flight::Disabled | Flight::Stopping(_) => {}
        }
        None
    }

    /// Enable and schedule the next fetch after `delay_ms` (no immediate kick).
    pub(crate) fn set_enabled_after(&mut self, index: usize, delay_ms: u32) -> Option<Kick> {
        self.refuse_while_building();
        let poll = poll_mut(&mut self.polls, index)?;
        poll.flight = poll.flight.enable(delay_ms);
        self.kick(index, Some(delay_ms))
    }

    /// Retarget the poll cadence and `is_stale` threshold; reschedule to apply.
    pub(crate) fn set_interval(&mut self, index: usize, interval_ms: u32) {
        if let Some(poll) = poll_mut(&mut self.polls, index) {
            poll.config.interval_ms = Some(interval_ms);
        }
    }

    pub(crate) fn enabled(&self, index: usize) -> bool {
        self.poll(index)
            .is_some_and(|poll| poll.flight.is_enabled())
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
        if poll.health == Health::Ok {
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
        let (Some(interval_ms), Health::Failing { since }) = (poll.config.interval_ms, poll.health)
        else {
            return false;
        };
        let age_ms = now_secs.saturating_sub(since).max(0).saturating_mul(1_000);
        #[expect(clippy::cast_precision_loss, reason = "poll ages are small")]
        let offline = age_ms as f64 > f64::from(interval_ms) * f64::from(poll.config.stale_factor);
        offline
    }

    #[cfg(test)]
    fn flight(&self, index: usize) -> Option<Flight> {
        self.poll(index).map(|poll| poll.flight)
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
pub(crate) enum ReplyAction {
    /// The reply belongs to no request the poll still waits for.
    Ignore,
    /// The reply's request was superseded: a fresh one goes out instead of delivering it.
    Reissued(Kick),
    /// The reply ends a disabled poll's request; the glue drops it as it does `Ignore`.
    Stopped,
    /// The reply answers the poll's live request; the glue runs the poll's handler with it.
    Deliver(usize, OnReplyFn),
}

#[cfg(any(target_arch = "wasm32", test))]
impl std::fmt::Debug for ReplyAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ignore => f.write_str("Ignore"),
            Self::Reissued(kick) => f.debug_tuple("Reissued").field(kick).finish(),
            Self::Stopped => f.write_str("Stopped"),
            Self::Deliver(index, _) => f
                .debug_tuple("Deliver")
                .field(index)
                .finish_non_exhaustive(),
        }
    }
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
        static LAST_DELAY: Cell<Option<u32>> = const { Cell::new(None) };
        static CANCELS: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) struct HostBackend;

    impl FetchBackend for HostBackend {
        fn send(&mut self, _spec: &FetchSpec, delay_ms: Option<u32>) -> Option<FetchRequestId> {
            SENDS.set(SENDS.get() + 1);
            LAST_DELAY.set(delay_ms);
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

    /// The delay the last send asked for.
    pub(super) fn last_delay() -> Option<u32> {
        LAST_DELAY.get()
    }
}

#[cfg(any(target_arch = "wasm32", test))]
mod glue;

#[cfg(any(target_arch = "wasm32", test))]
pub use glue::{BuildMethod, OnReply, ReplyMethod, register};

#[cfg(test)]
mod tests;
