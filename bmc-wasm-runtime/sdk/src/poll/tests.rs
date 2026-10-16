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
fn an_ended_poll_cancels_its_live_fetch() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = reg.register(build_url, CFG);
    reg.start(h, &mut be);

    reg.release(h, &mut be);

    assert_eq!(be.cancels.len(), 1);
}

#[test]
fn a_reply_already_queued_for_an_ended_poll_is_not_delivered() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = reg.register(build_url, CFG);
    reg.start(h, &mut be);

    reg.release(h, &mut be);

    assert_eq!(deliver(&mut reg, &mut be, 1, true), ReplyAction::Ignore);
    assert_eq!(be.sends.len(), 1, "an ended poll never fetches again");
}

#[test]
fn an_ended_poll_ignores_later_calls() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = reg.register(build_url, CFG);
    reg.start(h, &mut be);
    reg.release(h, &mut be);

    reg.invalidate(h, &mut be);
    reg.set_enabled(h, false, &mut be);
    reg.set_enabled(h, true, &mut be);
    reg.set_enabled_after(h, 1_000, &mut be);

    assert_eq!(be.sends.len(), 1);
    assert!(!reg.enabled(h));
}

#[test]
fn a_poll_ended_by_its_reply_handler_is_not_rescheduled() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = reg.register(build_url, CFG);
    reg.start(h, &mut be);
    let id = FetchRequestId::from_wire(1).expect("BUG: test id is nonzero");
    assert_eq!(reg.begin_reply(id, &mut be), ReplyAction::Deliver(h));

    reg.release(h, &mut be);
    reg.reschedule(h, true, 0, &mut be);

    assert_eq!(be.sends.len(), 1);
}

#[test]
fn an_ended_polls_index_is_never_reused() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let first = reg.register(build_url, CFG);
    reg.release(first, &mut be);

    let second = reg.register(build_url, CFG);

    assert_ne!(first, second);
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

thread_local! {
    static REPLIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static REPLIED_TO: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static STORED: std::cell::RefCell<Option<Handle>> = const { std::cell::RefCell::new(None) };
}

fn count_reply(_handle: &Handle, _response: &crate::net::FetchResponse) {
    REPLIES.set(REPLIES.get() + 1);
}

fn record_index(handle: &Handle, _response: &crate::net::FetchResponse) {
    REPLIED_TO.set(Some(handle.index()));
}

fn stop_stored(_handle: &Handle, _response: &crate::net::FetchResponse) {
    if let Some(stored) = STORED.take() {
        stored.stop();
    }
}

fn is_registered(index: usize) -> bool {
    glue::REGISTRY.with_borrow(|registry| registry.is_registered(index))
}

fn reply_to_first_fetch() {
    let id = FetchRequestId::from_wire(1).expect("BUG: test id is nonzero");
    glue::trampoline(&crate::net::FetchResponse::empty(id, 200));
}

#[test]
fn a_reply_reaches_its_handler_with_its_handle() {
    let handle = register(build_url, record_index, CFG);

    reply_to_first_fetch();

    assert_eq!(REPLIED_TO.get(), Some(handle.index()));
}

#[test]
fn a_dropped_handle_stops_its_replies() {
    let handle = register(build_url, count_reply, CFG);

    drop(handle);
    reply_to_first_fetch();

    assert_eq!(REPLIES.get(), 0);
}

#[test]
fn a_dropped_handle_ends_its_poll() {
    let handle = register(build_url, count_reply, CFG);
    let index = handle.index();

    drop(handle);

    assert!(!is_registered(index));
    assert_eq!(backend::cancels(), 1, "the live fetch is cancelled");
}

#[test]
fn a_reply_leaves_its_poll_registered() {
    let handle = register(build_url, count_reply, CFG);

    reply_to_first_fetch();

    assert_eq!(REPLIES.get(), 1);
    assert!(is_registered(handle.index()));
}

#[test]
fn a_reply_handler_can_stop_its_own_poll_through_the_stored_handle() {
    let handle = register(build_url, stop_stored, CFG);
    let index = handle.index();
    STORED.set(Some(handle));

    reply_to_first_fetch();

    assert!(!is_registered(index));
    assert_eq!(backend::sends(), 1, "a stopped poll is not rescheduled");
}
