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

use super::*;
use crate::wasm_test;

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
fn build_url(_h: &Handle) -> Option<FetchSpec> {
    Some(FetchSpec::get("http://x/data"))
}

fn build_none(_h: &Handle) -> Option<FetchSpec> {
    None
}

thread_local! {
    static TOGGLE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

fn build_toggle(_h: &Handle) -> Option<FetchSpec> {
    TOGGLE
        .with(std::cell::Cell::get)
        .then(|| FetchSpec::get("http://x/data"))
}

/// Perform `kick` as the glue does, with `build` outside the registry borrow.
fn perform(reg: &mut Registry, be: &mut FakeBackend, kick: Option<Kick>) {
    let Some(kick) = kick else {
        return;
    };
    let Some(build) = reg.prepare(&kick) else {
        return;
    };
    let spec = build(kick.index).expect("BUG: a test poll's build has no part to lose");
    reg.finish(&kick, spec.as_ref(), be);
}

fn start(reg: &mut Registry, be: &mut FakeBackend, h: usize) {
    let kick = reg.start(h);
    perform(reg, be, kick);
}

fn invalidate(reg: &mut Registry, be: &mut FakeBackend, h: usize) {
    let kick = reg.invalidate(h, be);
    perform(reg, be, kick);
}

fn set_enabled(reg: &mut Registry, be: &mut FakeBackend, h: usize, enabled: bool) {
    let kick = reg.set_enabled(h, enabled, be);
    perform(reg, be, kick);
}

fn set_enabled_after(reg: &mut Registry, be: &mut FakeBackend, h: usize, delay_ms: u32) {
    let kick = reg.set_enabled_after(h, delay_ms);
    perform(reg, be, kick);
}

fn reschedule(reg: &mut Registry, be: &mut FakeBackend, h: usize, ok: bool, now_secs: i64) {
    reschedule_judged(reg, be, h, ok, Verdict::Accept, now_secs);
}

fn reschedule_judged(
    reg: &mut Registry,
    be: &mut FakeBackend,
    h: usize,
    ok: bool,
    verdict: Verdict,
    now_secs: i64,
) {
    let delay = reg.bank_reply(h, ok, verdict, now_secs);
    let kick = reg.kick_after(h, delay);
    perform(reg, be, kick);
}

fn begin_reply(reg: &mut Registry, be: &mut FakeBackend, id: FetchRequestId) -> ReplyAction {
    let action = reg.begin_reply(id);
    if let ReplyAction::Reissued(kick) = action {
        perform(reg, be, Some(kick));
    }
    action
}

fn register_in(reg: &mut Registry, build: Build, config: Config) -> usize {
    reg.register(
        move |index| Ok(build(&Handle::borrowed(index))),
        |_, _: &FetchResponse| Ok(Verdict::Accept),
        BuildTiming::Now,
        config,
    )
}

fn deliver_at(
    reg: &mut Registry,
    be: &mut FakeBackend,
    id: u32,
    ok: bool,
    now_secs: i64,
) -> ReplyAction {
    let id = FetchRequestId::from_wire(id).expect("BUG: test id is nonzero");
    let action = begin_reply(reg, be, id);
    if let ReplyAction::Deliver(h, _) = action {
        reschedule(reg, be, h, ok, now_secs);
    }
    action
}

fn deliver(reg: &mut Registry, be: &mut FakeBackend, id: u32, ok: bool) -> ReplyAction {
    deliver_at(reg, be, id, ok, 0)
}

/// Delivers reply `id` as a 2xx at `now_secs` that its handler judged as `verdict`.
fn deliver_judged(
    reg: &mut Registry,
    be: &mut FakeBackend,
    id: u32,
    verdict: Verdict,
    now_secs: i64,
) {
    let id = FetchRequestId::from_wire(id).expect("BUG: test id is nonzero");
    let ReplyAction::Deliver(h, _) = begin_reply(reg, be, id) else {
        panic!("BUG: the test delivers the live request's reply");
    };
    reschedule_judged(reg, be, h, true, verdict, now_secs);
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
#[wasm_test]
fn spec_timeout_defaults_to_sdk_default_until_overridden() {
    assert_eq!(FetchSpec::get("http://x/data").timeout, None);
    let spec = FetchSpec::get("http://x/data").timeout(Duration::from_millis(100));
    assert_eq!(spec.timeout, Some(Duration::from_millis(100)));
}

#[wasm_test]
fn start_sends_immediately_when_enabled() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    assert_eq!(be.sends.len(), 1);
    assert_eq!(be.sends[0].1, None);
    assert_eq!(be.sends[0].0.url, "http://x/data");
}

#[wasm_test]
fn start_does_nothing_when_disabled() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(
        &mut reg,
        build_url,
        Config {
            enabled: false,
            ..CFG
        },
    );
    start(&mut reg, &mut be, h);
    assert!(be.sends.is_empty());
}

#[wasm_test]
fn rejected_send_retries_after_retry_ms() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.reject_next_sends = 1;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    assert_eq!(be.sends.len(), 2);
    assert_eq!(be.sends[0].1, None);
    assert_eq!(be.sends[1].1, Some(1_000));
}

#[wasm_test]
fn invalidate_with_queued_request_cancels_and_reschedules_after_debounce() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);
    assert_eq!(be.cancels.len(), 1);
    assert_eq!(be.sends.len(), 2);
    assert_eq!(be.sends[1].1, Some(300));
}

#[wasm_test]
fn invalidate_with_in_flight_request_supersedes_it_and_does_not_send() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);
    assert_eq!(be.cancels.len(), 1);
    assert_eq!(be.sends.len(), 1);
    assert!(matches!(reg.flight(h), Some(Flight::Superseded { .. })));
}

#[wasm_test]
fn a_second_refresh_does_not_cancel_again_what_the_host_refused_to() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);

    invalidate(&mut reg, &mut be, h);

    assert_eq!(
        be.cancels.len(),
        1,
        "the host refuses only a request already away, so asking again cannot stop it"
    );
}

#[wasm_test]
fn invalidate_when_idle_kicks_after_debounce() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    // Enabled but never started, so the slot is idle with no live request.
    let h = register_in(&mut reg, build_url, CFG);
    invalidate(&mut reg, &mut be, h);
    assert_eq!(be.sends.len(), 1);
    assert_eq!(be.sends[0].1, Some(300));
}

#[wasm_test]
fn successful_reply_reschedules_after_interval() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    let action = deliver(&mut reg, &mut be, 1, true);
    assert!(
        matches!(action, ReplyAction::Deliver(index, _) if index == h),
        "a reply to the live request goes to the handler: {action:?}"
    );
    assert_eq!(be.sends.len(), 2);
    assert_eq!(be.sends[1].1, Some(5_000));
}

#[wasm_test]
fn set_interval_retargets_the_reschedule_cadence() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG); // interval 5_000
    start(&mut reg, &mut be, h);
    reg.set_interval(h, 60_000);
    deliver(&mut reg, &mut be, 1, true);
    assert_eq!(
        be.sends.last().expect("BUG: a fetch was scheduled").1,
        Some(60_000),
        "a success reschedules at the new interval"
    );
}

#[wasm_test]
fn one_shot_reply_does_not_reschedule() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(
        &mut reg,
        build_url,
        Config {
            interval_ms: None,
            ..CFG
        },
    );
    start(&mut reg, &mut be, h);
    deliver(&mut reg, &mut be, 1, true);
    assert_eq!(be.sends.len(), 1);
}

#[wasm_test]
fn failed_reply_reschedules_after_retry() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    deliver(&mut reg, &mut be, 1, false);
    assert_eq!(be.sends.len(), 2);
    assert_eq!(be.sends[1].1, Some(1_000));
}

#[wasm_test]
fn retry_request_reschedules_after_retry_despite_ok() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    // A 200 whose payload the handler couldn't use: it calls retry()
    // before the engine reschedules, so the next attempt must come after
    // retry_ms rather than the full interval.
    let id = FetchRequestId::from_wire(1).expect("BUG: test id is nonzero");
    let action = begin_reply(&mut reg, &mut be, id);
    assert!(
        matches!(action, ReplyAction::Deliver(index, _) if index == h),
        "a reply to the live request goes to the handler: {action:?}"
    );
    reg.request_retry(h);
    reschedule(&mut reg, &mut be, h, true, 0);
    assert_eq!(be.sends.len(), 2);
    assert_eq!(be.sends[1].1, Some(1_000));
}

#[wasm_test]
fn retry_after_reschedules_with_the_requested_delay_despite_ok() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    // A reply handler applying its own backoff: the next attempt must wait
    // the requested delay, not retry_ms and not the poll interval.
    let id = FetchRequestId::from_wire(1).expect("BUG: test id is nonzero");
    let action = begin_reply(&mut reg, &mut be, id);
    assert!(
        matches!(action, ReplyAction::Deliver(index, _) if index == h),
        "a reply to the live request goes to the handler: {action:?}"
    );
    reg.request_retry_after(h, 42_000);
    reschedule(&mut reg, &mut be, h, true, 0);
    assert_eq!(be.sends.len(), 2);
    assert_eq!(be.sends[1].1, Some(42_000));
}

#[wasm_test]
fn a_reply_judged_retry_is_retried_after_retry_ms() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    deliver_judged(&mut reg, &mut be, 1, Verdict::Retry, 0);
    assert_eq!(
        be.sends.last().map(|send| send.1),
        Some(Some(CFG.retry_ms)),
        "a 2xx its handler rejected is retried soon, not after the interval"
    );
}

#[wasm_test]
fn a_reply_judged_retry_after_waits_the_delay_it_names() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    deliver_judged(&mut reg, &mut be, 1, Verdict::RetryAfter(42_000), 0);
    assert_eq!(
        be.sends.last().map(|send| send.1),
        Some(Some(42_000)),
        "the handler's own backoff wins over retry_ms and the interval"
    );
}

#[wasm_test]
fn reply_to_unknown_id_is_ignored() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let id = FetchRequestId::from_wire(99).expect("BUG: nonzero");
    let action = begin_reply(&mut reg, &mut be, id);
    assert!(
        matches!(action, ReplyAction::Ignore),
        "a reply to an unknown id is ignored: {action:?}"
    );
}

#[wasm_test]
fn a_superseded_reply_reissues_after_the_debounce_and_skips_delivery() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);
    let action = deliver(&mut reg, &mut be, 1, true);
    assert!(
        matches!(action, ReplyAction::Reissued(_)),
        "a reply to an invalidated request reissues it: {action:?}"
    );
    assert_eq!(be.sends.len(), 2);
    assert_eq!(
        be.sends[1].1,
        Some(CFG.debounce_ms),
        "the refresh keeps the debounce invalidate asked for"
    );
}

#[wasm_test]
fn builder_none_sends_nothing_on_start() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_none, CFG);
    start(&mut reg, &mut be, h);
    assert!(be.sends.is_empty());
}

#[wasm_test]
fn builder_none_on_reschedule_goes_dormant() {
    TOGGLE.with(|t| t.set(true));
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_toggle, CFG);
    start(&mut reg, &mut be, h);
    TOGGLE.with(|t| t.set(false));
    deliver(&mut reg, &mut be, 1, true);
    assert_eq!(be.sends.len(), 1);
}

#[wasm_test]
fn rapid_invalidations_cancel_the_queued_send_and_keep_one() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);
    assert_eq!(be.cancels.len(), 2);
    assert_eq!(be.sends.len(), 3);
    assert_eq!(be.sends[2].1, Some(300));
}

mod enabling;
mod ending;
mod staleness;

mod off_device;
