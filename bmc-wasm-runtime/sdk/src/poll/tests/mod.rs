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

use std::mem::ManuallyDrop;

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
    let spec = build(&ManuallyDrop::new(Handle(kick.index)));
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
    let kick = reg.reschedule(h, ok, now_secs);
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
        build,
        |_: &Handle, _: &FetchResponse| {},
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
fn invalidate_with_in_flight_request_marks_pending_and_does_not_send() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);
    assert_eq!(be.cancels.len(), 1);
    assert_eq!(be.sends.len(), 1);
    assert!(reg.is_pending(h));
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
    assert!(matches!(action, ReplyAction::Deliver(index, _) if index == h));
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
    assert!(
        matches!(begin_reply(&mut reg, &mut be, id), ReplyAction::Deliver(index, _) if index == h)
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
    assert!(
        matches!(begin_reply(&mut reg, &mut be, id), ReplyAction::Deliver(index, _) if index == h)
    );
    reg.request_retry_after(h, 42_000);
    reschedule(&mut reg, &mut be, h, true, 0);
    assert_eq!(be.sends.len(), 2);
    assert_eq!(be.sends[1].1, Some(42_000));
}

#[wasm_test]
fn reply_to_unknown_id_is_ignored() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let id = FetchRequestId::from_wire(99).expect("BUG: nonzero");
    assert!(matches!(
        begin_reply(&mut reg, &mut be, id),
        ReplyAction::Ignore
    ));
}

#[wasm_test]
fn pending_reply_reissues_immediately_and_skips_delivery() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);
    let action = deliver(&mut reg, &mut be, 1, true);
    assert!(matches!(action, ReplyAction::Reissued(_)));
    assert_eq!(be.sends.len(), 2);
    assert_eq!(be.sends[1].1, None);
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

/// The glue's tests count sends and cancels through the off-target test backend.
#[cfg(not(target_arch = "wasm32"))]
mod off_device {
    use super::*;
    use crate::net::FetchResponse;
    use crate::widget::{self, Part, PartHandle};

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

    fn registered(build: Build, on_reply: OnReply) -> Handle {
        register(build, on_reply, CFG)
    }

    /// Reply as the host does, with the work the reply asked for done before it returns.
    fn reply_to_first_fetch() {
        let id = FetchRequestId::from_wire(1).expect("BUG: test id is nonzero");
        glue::trampoline(&FetchResponse::empty(id, 200));
        widget::drain();
    }

    #[test]
    fn a_reply_reaches_its_handler_with_its_handle() {
        let handle = registered(build_url, record_index);

        reply_to_first_fetch();

        assert_eq!(REPLIED_TO.get(), Some(handle.index()));
    }

    #[test]
    fn a_dropped_handle_stops_its_replies() {
        let handle = registered(build_url, count_reply);

        drop(handle);
        reply_to_first_fetch();

        assert_eq!(REPLIES.get(), 0);
    }

    #[test]
    fn a_dropped_handle_ends_its_poll() {
        let handle = registered(build_url, count_reply);
        let index = handle.index();

        drop(handle);

        assert!(!is_registered(index));
        assert_eq!(backend::cancels(), 1, "the live fetch is cancelled");
    }

    #[test]
    fn a_reply_leaves_its_poll_registered() {
        let handle = registered(build_url, count_reply);

        reply_to_first_fetch();

        assert_eq!(REPLIES.get(), 1);
        assert!(is_registered(handle.index()));
    }

    #[test]
    fn a_reply_handler_can_stop_its_own_poll_through_the_stored_handle() {
        let handle = registered(build_url, stop_stored);
        let index = handle.index();
        STORED.set(Some(handle));

        reply_to_first_fetch();

        assert!(!is_registered(index));
        assert_eq!(backend::sends(), 1, "a stopped poll is not rescheduled");
    }

    #[expect(clippy::unnecessary_wraps, reason = "a Build")]
    fn build_using_the_handle(handle: &Handle) -> Option<FetchSpec> {
        assert!(handle.enabled(), "the handle works from build");
        Some(FetchSpec::get("http://x/data"))
    }

    fn enable_after_nine_seconds(handle: &Handle, _response: &FetchResponse) {
        handle.set_enabled(false);
        handle.enable_after(9_000);
    }

    #[test]
    fn a_plain_poll_sends_where_it_is_registered() {
        let _handle = register(build_url, count_reply, CFG);

        assert_eq!(backend::sends(), 1);
    }

    #[test]
    fn build_may_use_the_handle() {
        let _handle = registered(build_using_the_handle, count_reply);

        assert_eq!(backend::sends(), 1);
    }

    #[test]
    fn a_reply_handlers_enable_after_decides_the_next_fetch() {
        let _handle = registered(build_url, enable_after_nine_seconds);

        reply_to_first_fetch();

        assert_eq!((backend::sends(), backend::last_delay()), (2, Some(9_000)));
    }

    struct Poller {
        builds: usize,
        replies: usize,
        handle: Option<Handle>,
    }

    impl Poller {
        fn new() -> Part<Self> {
            Part::new(Self {
                builds: 0,
                replies: 0,
                handle: None,
            })
        }

        #[expect(clippy::unnecessary_wraps, reason = "a Build method")]
        fn build(&mut self, _handle: &Handle) -> Option<FetchSpec> {
            self.builds += 1;
            Some(FetchSpec::get("http://x/data"))
        }

        fn on_reply(&mut self, _handle: &Handle, _response: &FetchResponse) {
            self.replies += 1;
        }

        fn on_reply_enable_after_nine_seconds(
            &mut self,
            handle: &Handle,
            response: &FetchResponse,
        ) {
            self.on_reply(handle, response);
            enable_after_nine_seconds(handle, response);
        }

        fn refresh(&mut self) {
            self.handle
                .as_ref()
                .expect("BUG: the test stores the handle first")
                .invalidate();
            assert_eq!(self.builds, 1, "build waits for the method to return");
        }

        fn stored(&self) -> &Handle {
            self.handle
                .as_ref()
                .expect("BUG: the test stores the handle first")
        }

        fn pause(&mut self) {
            self.stored().set_enabled(false);
        }

        fn refresh_then_pause(&mut self) {
            self.stored().invalidate();
            self.stored().set_enabled(false);
        }

        fn resume_then_resume_later(&mut self) {
            self.stored().set_enabled(true);
            self.stored().enable_after(9_000);
        }
    }

    /// A part with its poll started, inside an export as the SDK requires.
    fn poller() -> Part<Poller> {
        widget::enter();
        let mut part = Poller::new();
        let handle = Handle::register(&part, Poller::build, Poller::on_reply, CFG);
        part.with_mut(|value| value.handle = Some(handle));
        widget::drain();
        part
    }

    #[test]
    fn a_parts_poll_builds_and_replies_as_its_methods() {
        let part = poller();

        reply_to_first_fetch();

        assert_eq!(part.with(|value| (value.builds, value.replies)), (2, 1));
    }

    #[test]
    fn a_method_poll_sends_once_the_export_ended() {
        widget::enter();
        let part = Poller::new();
        let _handle = Handle::register(&part, Poller::build, Poller::on_reply, CFG);
        let before = backend::sends();

        widget::drain();

        assert_eq!((before, backend::sends()), (0, 1));
    }

    #[test]
    #[should_panic(expected = "outside an SDK export")]
    fn a_method_poll_driven_outside_an_export_is_a_bug() {
        let part = Poller::new();

        let _handle = Handle::register(&part, Poller::build, Poller::on_reply, CFG);
    }

    #[test]
    fn a_refresh_asked_for_inside_a_method_builds_after_it() {
        let part = poller();

        part.target().deliver(Poller::refresh);
        widget::drain();

        assert_eq!(part.with(|value| value.builds), 2);
    }

    #[test]
    fn of_two_requests_one_method_asks_for_the_last_wins() {
        let part = poller();
        part.target().deliver(Poller::pause);

        part.target().deliver(Poller::resume_then_resume_later);
        widget::drain();

        assert_eq!(
            (backend::sends(), backend::last_delay()),
            (2, Some(9_000)),
            "the later enable_after replaces the debounced kick asked for before it"
        );
    }

    #[test]
    fn a_dropped_part_leaves_its_poll_dormant() {
        let part = poller();
        let handle = Handle::register(&part, Poller::build, Poller::on_reply, CFG);
        drop(part);

        widget::drain();

        assert_eq!(backend::sends(), 1);
        assert!(is_registered(handle.index()));
    }

    /// A part and a handle it could keep to itself, to register through later.
    fn poller_and_handle() -> (Part<Poller>, PartHandle<Poller>) {
        let mut handle = None;
        let part = Part::new_cyclic(|me| {
            handle = Some(me.handle());
            Poller {
                builds: 0,
                replies: 0,
                handle: None,
            }
        });
        (part, handle.expect("BUG: set by the build"))
    }

    #[test]
    fn a_poll_through_a_handle_to_a_dropped_part_is_refused() {
        widget::enter();
        let (part, handle) = poller_and_handle();
        drop(part);

        let refused = Handle::register(&handle, Poller::build, Poller::on_reply, CFG).is_none();
        widget::drain();

        assert!(
            refused && backend::sends() == 0,
            "nothing registered, nothing sent"
        );
    }

    #[test]
    fn a_poll_through_a_handle_to_a_live_part_registers_and_sends() {
        widget::enter();
        let (part, handle) = poller_and_handle();

        let handle = Handle::register(&handle, Poller::build, Poller::on_reply, CFG);
        widget::drain();

        assert_eq!(
            (handle.is_some(), part.with(|value| value.builds)),
            (true, 1),
            "a live part's poll registers and builds on it"
        );
    }

    #[test]
    fn a_poll_registered_while_its_part_is_built_builds_on_the_built_part() {
        widget::enter();
        let part = Part::new_cyclic(|me| Poller {
            builds: 0,
            replies: 0,
            handle: Some(Handle::register(me, Poller::build, Poller::on_reply, CFG)),
        });

        widget::drain();

        assert_eq!(
            (part.with(|value| value.builds), backend::sends()),
            (1, 1),
            "the first build waits for the value it was registered on"
        );
    }

    struct Quitter {
        handle: Option<Handle>,
    }

    impl Quitter {
        #[expect(clippy::unnecessary_wraps, reason = "a Build method")]
        fn build_and_end(&mut self, _handle: &Handle) -> Option<FetchSpec> {
            self.handle = None;
            Some(FetchSpec::get("http://x/data"))
        }

        #[expect(
            clippy::unused_self,
            reason = "the poll ends in build, so no reply arrives"
        )]
        fn on_reply(&mut self, _handle: &Handle, _response: &FetchResponse) {}
    }

    #[test]
    fn a_build_that_ends_its_poll_sends_nothing() {
        widget::enter();
        let mut part = Part::new(Quitter { handle: None });

        let handle = Handle::register(&part, Quitter::build_and_end, Quitter::on_reply, CFG);
        part.with_mut(|value| value.handle = Some(handle));

        widget::drain();

        assert_eq!(backend::sends(), 0);
    }

    #[test]
    fn a_method_reply_handlers_enable_after_decides_the_next_fetch() {
        widget::enter();
        let mut part = Poller::new();
        let handle = Handle::register(
            &part,
            Poller::build,
            Poller::on_reply_enable_after_nine_seconds,
            CFG,
        );
        part.with_mut(|value| value.handle = Some(handle));
        widget::drain();

        reply_to_first_fetch();

        assert_eq!((backend::sends(), backend::last_delay()), (2, Some(9_000)));
    }

    #[test]
    fn a_refresh_then_a_disable_in_one_method_sends_nothing_more() {
        let part = poller();

        part.target().deliver(Poller::refresh_then_pause);
        widget::drain();

        assert_eq!(backend::sends(), 1, "only the start's request went out");
    }

    thread_local! {
        static HELD: std::cell::RefCell<Option<Part<Leaver>>> = const { std::cell::RefCell::new(None) };
    }

    struct Leaver;

    impl Leaver {
        #[expect(clippy::unused_self, reason = "a Build method")]
        #[expect(clippy::unnecessary_wraps, reason = "a Build method")]
        fn build_and_drop_the_part(&mut self, _handle: &Handle) -> Option<FetchSpec> {
            HELD.take();
            Some(FetchSpec::get("http://x/data"))
        }

        #[expect(
            clippy::unused_self,
            reason = "the part goes in build, so no reply arrives"
        )]
        fn on_reply(&mut self, _handle: &Handle, _response: &FetchResponse) {}
    }

    #[test]
    fn a_build_that_drops_its_part_sends_nothing() {
        widget::enter();
        let part = Part::new(Leaver);
        let _handle = Handle::register(
            &part,
            Leaver::build_and_drop_the_part,
            Leaver::on_reply,
            CFG,
        );
        HELD.set(Some(part));

        widget::drain();

        assert_eq!(backend::sends(), 0);
    }

    struct Pauser;

    impl Pauser {
        #[expect(clippy::unused_self, reason = "a Build method")]
        #[expect(clippy::unnecessary_wraps, reason = "a Build method")]
        fn build_and_disable(&mut self, handle: &Handle) -> Option<FetchSpec> {
            handle.set_enabled(false);
            Some(FetchSpec::get("http://x/data"))
        }

        #[expect(
            clippy::unused_self,
            reason = "the poll is disabled in build, so no reply arrives"
        )]
        fn on_reply(&mut self, _handle: &Handle, _response: &FetchResponse) {}
    }

    #[test]
    fn a_build_that_disables_its_poll_sends_nothing() {
        widget::enter();
        let part = Part::new(Pauser);
        let _handle = Handle::register(&part, Pauser::build_and_disable, Pauser::on_reply, CFG);

        widget::drain();

        assert_eq!(backend::sends(), 0);
    }

    thread_local! {
        static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Without the guard these builds run again and again; cap them so the test still ends.
    fn count_build() {
        BUILDS.set(BUILDS.get() + 1);
        assert!(BUILDS.get() < 50, "build re-ran {} times", BUILDS.get());
    }

    fn build_and_invalidate(handle: &Handle) -> Option<FetchSpec> {
        count_build();
        handle.invalidate();
        None
    }

    fn build_and_enable_after(handle: &Handle) -> Option<FetchSpec> {
        count_build();
        handle.enable_after(9_000);
        None
    }

    fn build_and_enable(handle: &Handle) -> Option<FetchSpec> {
        count_build();
        handle.set_enabled(true);
        None
    }

    fn build_and_invalidate_stored(_handle: &Handle) -> Option<FetchSpec> {
        count_build();
        STORED.with_borrow(|stored| {
            stored
                .as_ref()
                .expect("BUG: the test stores the other poll first")
                .invalidate();
        });
        None
    }

    #[expect(clippy::unnecessary_wraps, reason = "a Build")]
    fn build_and_retry(handle: &Handle) -> Option<FetchSpec> {
        handle.retry();
        Some(FetchSpec::get("http://x/data"))
    }

    #[test]
    #[should_panic(expected = "build asked a poll for a request")]
    fn a_build_that_invalidates_its_own_poll_is_a_bug() {
        let _handle = registered(build_and_invalidate, count_reply);
    }

    #[test]
    #[should_panic(expected = "build asked a poll for a request")]
    fn a_build_that_enables_its_own_poll_later_is_a_bug() {
        let _handle = registered(build_and_enable_after, count_reply);
    }

    #[test]
    #[should_panic(expected = "build asked a poll for a request")]
    fn a_build_that_enables_its_own_enabled_poll_is_a_bug() {
        let _handle = registered(build_and_enable, count_reply);
    }

    #[test]
    #[should_panic(expected = "build asked a poll for a request")]
    fn a_build_that_invalidates_another_poll_is_a_bug() {
        let other = registered(build_url, count_reply);
        STORED.set(Some(other));

        let _handle = registered(build_and_invalidate_stored, count_reply);
    }

    #[test]
    #[should_panic(expected = "build asked a poll for a request")]
    fn a_build_that_registers_a_poll_is_a_bug() {
        fn build_and_register(_handle: &Handle) -> Option<FetchSpec> {
            count_build();
            STORED.set(Some(register(build_url, count_reply, CFG)));
            None
        }

        let _handle = registered(build_and_register, count_reply);
    }

    #[test]
    fn a_build_may_retry_its_poll() {
        let _handle = registered(build_and_retry, count_reply);

        assert_eq!(backend::sends(), 1);
    }

    fn build_nothing(_handle: &Handle) -> Option<FetchSpec> {
        count_build();
        None
    }

    #[test]
    fn a_poll_that_built_nothing_may_be_asked_again() {
        let handle = registered(build_nothing, count_reply);

        handle.invalidate();

        assert_eq!(BUILDS.get(), 2, "the invalidate runs the build again");
    }

    struct Refresher;

    impl Refresher {
        #[expect(clippy::unused_self, reason = "a Build method")]
        fn build_and_refresh(&mut self, handle: &Handle) -> Option<FetchSpec> {
            count_build();
            handle.invalidate();
            None
        }

        #[expect(clippy::unused_self, reason = "the build panics, so no reply arrives")]
        fn on_reply(&mut self, _handle: &Handle, _response: &FetchResponse) {}
    }

    #[test]
    #[should_panic(expected = "build asked a poll for a request")]
    fn a_method_build_that_invalidates_its_own_poll_is_a_bug() {
        widget::enter();
        let part = Part::new(Refresher);
        let _handle = Handle::register(
            &part,
            Refresher::build_and_refresh,
            Refresher::on_reply,
            CFG,
        );

        widget::drain();
    }

    /// Two polls whose builds ask each other for a request.
    struct Pair {
        first: Option<Handle>,
        second: Option<Handle>,
    }

    impl Pair {
        fn build_first(&mut self, _handle: &Handle) -> Option<FetchSpec> {
            count_build();
            if let Some(second) = &self.second {
                second.invalidate();
            }
            None
        }

        fn build_second(&mut self, _handle: &Handle) -> Option<FetchSpec> {
            count_build();
            if let Some(first) = &self.first {
                first.invalidate();
            }
            None
        }

        #[expect(clippy::unused_self, reason = "the builds panic, so no reply arrives")]
        fn on_reply(&mut self, _handle: &Handle, _response: &FetchResponse) {}
    }

    #[test]
    #[should_panic(expected = "build asked a poll for a request")]
    fn method_builds_that_invalidate_each_other_are_a_bug() {
        widget::enter();
        let mut part = Part::new(Pair {
            first: None,
            second: None,
        });
        let first = Handle::register(&part, Pair::build_first, Pair::on_reply, CFG);
        let second = Handle::register(&part, Pair::build_second, Pair::on_reply, CFG);
        part.with_mut(|value| {
            value.first = Some(first);
            value.second = Some(second);
        });

        widget::drain();
    }
}
