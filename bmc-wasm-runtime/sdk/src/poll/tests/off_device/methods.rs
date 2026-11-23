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

//! Polls whose build and reply run as methods of a part.

#![cfg(not(target_arch = "wasm32"))]

use super::*;

const REJECTED_RETRY_MS: u32 = 7_000;

struct Poller {
    builds: usize,
    replies: usize,
    handle: Option<Poll>,
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
    fn build(&mut self, _handle: &Poll) -> Option<FetchSpec> {
        self.builds += 1;
        Some(FetchSpec::get("http://x/data"))
    }

    fn on_reply(&mut self, _handle: &Poll, _response: &FetchResponse) -> Verdict {
        self.replies += 1;
        Verdict::Accept
    }

    fn on_reply_enable_after_nine_seconds(
        &mut self,
        handle: &Poll,
        response: &FetchResponse,
    ) -> Verdict {
        enable_after_nine_seconds(handle, response);
        self.on_reply(handle, response)
    }

    fn on_reply_and_reject(&mut self, handle: &Poll, response: &FetchResponse) -> Verdict {
        self.on_reply(handle, response);
        Verdict::RetryAfter(REJECTED_RETRY_MS)
    }

    fn refresh(&mut self) {
        self.handle
            .as_ref()
            .expect("BUG: the test stores the handle first")
            .invalidate();
        assert_eq!(self.builds, 1, "build waits for the method to return");
    }

    fn stored(&self) -> &Poll {
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

    fn resume_later_then_refresh(&mut self) {
        self.stored().enable_after(9_000);
        self.stored().invalidate();
    }

    fn refresh_then_resume_later(&mut self) {
        self.stored().invalidate();
        self.stored().enable_after(9_000);
    }
}

/// A part with its poll started, inside an export as the SDK requires.
fn poller() -> Part<Poller> {
    widget::enter();
    let mut part = Poller::new();
    let handle = Poll::register(&part, Poller::build, Poller::on_reply, CFG);
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
    let _handle = Poll::register(&part, Poller::build, Poller::on_reply, CFG);
    let before = backend::sends();

    widget::drain();

    assert_eq!((before, backend::sends()), (0, 1));
}

#[test]
#[should_panic(expected = "driven outside an SDK export")]
fn a_method_poll_driven_outside_an_export_is_a_bug() {
    let part = Poller::new();

    let _handle = Poll::register(&part, Poller::build, Poller::on_reply, CFG);
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

    part.target().deliver(Poller::resume_later_then_refresh);
    widget::drain();

    assert_eq!(
        (backend::sends(), backend::last_delay()),
        (2, Some(CFG.debounce_ms)),
        "the later invalidate replaces the kick enable_after asked for before it"
    );
}

#[test]
fn an_enable_after_behind_a_refresh_in_one_method_keeps_the_debounce() {
    let part = poller();

    part.target().deliver(Poller::refresh_then_resume_later);
    widget::drain();

    assert_eq!(
        (backend::sends(), backend::last_delay()),
        (2, Some(CFG.debounce_ms)),
        "the queued refresh is the poll's fetch, as on a free poll whose refresh already went out"
    );
}

#[test]
fn an_enable_after_behind_an_enable_in_one_method_keeps_the_debounce() {
    let part = poller();
    part.target().deliver(Poller::pause);

    part.target().deliver(Poller::resume_then_resume_later);
    widget::drain();

    assert_eq!(
        (backend::sends(), backend::last_delay()),
        (2, Some(CFG.debounce_ms)),
        "the poll is enabled by then, so enable_after leaves the kick set_enabled asked for"
    );
}

#[test]
fn a_poll_whose_part_is_gone_before_it_builds_ends() {
    let part = poller();
    let handle = Poll::register(&part, Poller::build, Poller::on_reply, CFG);
    drop(part);

    widget::drain();

    assert_eq!(backend::sends(), 1, "only the first poller's poll sent");
    assert!(
        !is_registered(handle.index()) && !handle.enabled(),
        "a poll kept outside its gone part reads as ended, not as healthy and idle"
    );

    let next = Poller::new();
    let _next_handle = Poll::register(&next, Poller::build, Poller::on_reply, CFG);
    widget::drain();
    assert_eq!(
        backend::sends(),
        2,
        "an ended poll leaves the next one free to build"
    );
}

#[test]
fn a_poll_whose_part_is_gone_by_its_reply_ends_without_counting_it() {
    // Without an interval nothing builds again, so only the reply can end the poll.
    let one_shot = Config {
        interval_ms: None,
        ..CFG
    };
    widget::enter();
    let part = Poller::new();
    let handle = Poll::register(&part, Poller::build, Poller::on_reply, one_shot);
    widget::drain();
    drop(part);

    reply_to_first_fetch();

    assert_eq!(
        (backend::sends(), handle.last_success_age()),
        (1, None),
        "the reply nobody read is no refresh, and no next fetch goes out"
    );
    assert!(!is_registered(handle.index()) && !handle.enabled());
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
fn a_poll_through_a_handle_to_a_dropped_part_registers_nothing() {
    widget::enter();
    let (part, handle) = poller_and_handle();
    drop(part);

    let dropped = matches!(
        Poll::register(&handle, Poller::build, Poller::on_reply, CFG),
        Err(widget::PartError::NotLive)
    );
    widget::drain();

    assert!(
        dropped && backend::sends() == 0,
        "nothing registered, nothing sent"
    );
}

#[test]
fn a_poll_through_a_handle_to_a_live_part_registers_and_sends() {
    widget::enter();
    let (part, handle) = poller_and_handle();

    let handle = Poll::register(&handle, Poller::build, Poller::on_reply, CFG);
    widget::drain();

    assert_eq!(
        (handle.is_ok(), part.with(|value| value.builds)),
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
        handle: Some(Poll::register(me, Poller::build, Poller::on_reply, CFG)),
    });

    widget::drain();

    assert_eq!(
        (part.with(|value| value.builds), backend::sends()),
        (1, 1),
        "the first build waits for the value it was registered on"
    );
}

struct Quitter {
    handle: Option<Poll>,
}

impl Quitter {
    #[expect(clippy::unnecessary_wraps, reason = "a Build method")]
    fn build_and_end(&mut self, _handle: &Poll) -> Option<FetchSpec> {
        self.handle = None;
        Some(FetchSpec::get("http://x/data"))
    }

    #[expect(
        clippy::unused_self,
        reason = "the poll ends in build, so no reply arrives"
    )]
    fn on_reply(&mut self, _handle: &Poll, _response: &FetchResponse) -> Verdict {
        Verdict::Accept
    }
}

#[test]
fn a_build_that_ends_its_poll_sends_nothing() {
    widget::enter();
    let mut part = Part::new(Quitter { handle: None });

    let handle = Poll::register(&part, Quitter::build_and_end, Quitter::on_reply, CFG);
    part.with_mut(|value| value.handle = Some(handle));

    widget::drain();

    assert_eq!(backend::sends(), 0);
}

#[test]
fn a_method_reply_handlers_enable_after_decides_the_next_fetch() {
    widget::enter();
    let mut part = Poller::new();
    let handle = Poll::register(
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

/// A part whose reply method rejects every reply, with its first reply delivered.
fn poller_rejecting_its_first_reply() -> Part<Poller> {
    widget::enter();
    let mut part = Poller::new();
    let handle = Poll::register(&part, Poller::build, Poller::on_reply_and_reject, CFG);
    part.with_mut(|value| value.handle = Some(handle));
    widget::drain();

    reply_to_first_fetch();
    part
}

#[test]
fn a_rejected_reply_is_retried_after_the_delay_the_verdict_names() {
    let _part = poller_rejecting_its_first_reply();

    assert_eq!(
        backend::last_delay(),
        Some(REJECTED_RETRY_MS),
        "a rejected reply is retried after the method's delay, not the full interval"
    );
}

#[test]
fn a_rejected_reply_is_no_refresh() {
    let part = poller_rejecting_its_first_reply();

    assert_eq!(
        part.with(|poller| poller.stored().last_success_time()),
        None,
        "a reply its method rejected must not make the poll read as fresh"
    );
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
    fn build_and_drop_the_part(&mut self, _handle: &Poll) -> Option<FetchSpec> {
        HELD.take();
        Some(FetchSpec::get("http://x/data"))
    }

    #[expect(
        clippy::unused_self,
        reason = "the part goes in build, so no reply arrives"
    )]
    fn on_reply(&mut self, _handle: &Poll, _response: &FetchResponse) -> Verdict {
        Verdict::Accept
    }

    #[expect(clippy::unused_self, reason = "a Build method")]
    #[expect(clippy::unnecessary_wraps, reason = "a Build method")]
    fn build(&mut self, _handle: &Poll) -> Option<FetchSpec> {
        Some(FetchSpec::get("http://x/data"))
    }

    #[expect(clippy::unused_self, reason = "a Reply method")]
    fn reply_and_drop_the_part(&mut self, _handle: &Poll, _response: &FetchResponse) -> Verdict {
        HELD.take();
        Verdict::Accept
    }
}

#[test]
fn a_build_that_drops_its_part_sends_nothing() {
    widget::enter();
    let part = Part::new(Leaver);
    let _handle = Poll::register(
        &part,
        Leaver::build_and_drop_the_part,
        Leaver::on_reply,
        CFG,
    );
    HELD.set(Some(part));

    widget::drain();

    assert_eq!(backend::sends(), 0);
}

#[test]
fn a_reply_that_drops_its_part_ends_the_poll_without_counting_it() {
    // Without an interval nothing builds again, so only the reply can end the poll.
    let one_shot = Config {
        interval_ms: None,
        ..CFG
    };
    widget::enter();
    let part = Part::new(Leaver);
    let handle = Poll::register(
        &part,
        Leaver::build,
        Leaver::reply_and_drop_the_part,
        one_shot,
    );
    HELD.set(Some(part));
    widget::drain();

    reply_to_first_fetch();

    assert_eq!(
        handle.last_success_age(),
        None,
        "a reply that ended its part is no refresh"
    );
    assert!(!is_registered(handle.index()) && !handle.enabled());
}

struct Pauser;

impl Pauser {
    #[expect(clippy::unused_self, reason = "a Build method")]
    #[expect(clippy::unnecessary_wraps, reason = "a Build method")]
    fn build_and_disable(&mut self, handle: &Poll) -> Option<FetchSpec> {
        handle.set_enabled(false);
        Some(FetchSpec::get("http://x/data"))
    }

    #[expect(
        clippy::unused_self,
        reason = "the poll is disabled in build, so no reply arrives"
    )]
    fn on_reply(&mut self, _handle: &Poll, _response: &FetchResponse) -> Verdict {
        Verdict::Accept
    }
}

#[test]
fn a_build_that_disables_its_poll_sends_nothing() {
    widget::enter();
    let part = Part::new(Pauser);
    let _handle = Poll::register(&part, Pauser::build_and_disable, Pauser::on_reply, CFG);

    widget::drain();

    assert_eq!(backend::sends(), 0);
}

struct Refresher;

impl Refresher {
    #[expect(clippy::unused_self, reason = "a Build method")]
    fn build_and_refresh(&mut self, handle: &Poll) -> Option<FetchSpec> {
        count_build();
        handle.invalidate();
        None
    }

    #[expect(clippy::unused_self, reason = "the build panics, so no reply arrives")]
    fn on_reply(&mut self, _handle: &Poll, _response: &FetchResponse) -> Verdict {
        Verdict::Accept
    }
}

#[test]
#[should_panic(expected = "build asked a poll for a request")]
fn a_method_build_that_invalidates_its_own_poll_is_a_bug() {
    widget::enter();
    let part = Part::new(Refresher);
    let _handle = Poll::register(
        &part,
        Refresher::build_and_refresh,
        Refresher::on_reply,
        CFG,
    );

    widget::drain();
}

/// Two polls whose builds ask each other for a request.
struct Pair {
    first: Option<Poll>,
    second: Option<Poll>,
}

impl Pair {
    fn build_first(&mut self, _handle: &Poll) -> Option<FetchSpec> {
        count_build();
        if let Some(second) = &self.second {
            second.invalidate();
        }
        None
    }

    fn build_second(&mut self, _handle: &Poll) -> Option<FetchSpec> {
        count_build();
        if let Some(first) = &self.first {
            first.invalidate();
        }
        None
    }

    #[expect(clippy::unused_self, reason = "the builds panic, so no reply arrives")]
    fn on_reply(&mut self, _handle: &Poll, _response: &FetchResponse) -> Verdict {
        Verdict::Accept
    }
}

#[test]
#[should_panic(expected = "build asked a poll for a request")]
fn method_builds_that_invalidate_each_other_are_a_bug() {
    widget::enter();
    let mut part = Part::new(Pair {
        first: None,
        second: None,
    });
    let first = Poll::register(&part, Pair::build_first, Pair::on_reply, CFG);
    let second = Poll::register(&part, Pair::build_second, Pair::on_reply, CFG);
    part.with_mut(|value| {
        value.first = Some(first);
        value.second = Some(second);
    });

    widget::drain();
}

struct Teller {
    handle: Option<Poll>,
    told: crate::Notify<()>,
}

impl Teller {
    #[expect(
        clippy::unnecessary_wraps,
        clippy::unused_self,
        reason = "a Build method"
    )]
    fn build(&mut self, _handle: &Poll) -> Option<FetchSpec> {
        Some(FetchSpec::get("http://x/data"))
    }

    fn on_reply(&mut self, _handle: &Poll, _response: &FetchResponse) -> Verdict {
        self.told.send(());
        Verdict::Accept
    }
}

/// Runs `f` on the handle of the seated teller.
fn teller_handle<R>(seated: Option<&Part<Teller>>, f: impl FnOnce(&Poll) -> R) -> R {
    let teller = seated.expect("BUG: the test seats the teller first");
    teller.with(|teller| {
        f(teller
            .handle
            .as_ref()
            .expect("BUG: the test stores the handle first"))
    })
}

/// Seats `widget` behind a teller that tells it of each reply, then delivers the first one.
fn told_of_the_first_reply<W: 'static>(
    widget: W,
    on_told: fn(&mut W, ()),
    seat: fn(&mut W, Part<Teller>),
) -> Part<W> {
    widget::enter();
    let mut receiver = Part::new(widget);
    let mut teller = Part::new(Teller {
        handle: None,
        told: crate::Notify::new(&receiver, on_told),
    });
    let handle = Poll::register(&teller, Teller::build, Teller::on_reply, CFG);
    teller.with_mut(|teller| teller.handle = Some(handle));
    receiver.with_mut(|widget| seat(widget, teller));
    widget::drain();

    reply_to_first_fetch();
    receiver
}

#[derive(Default)]
struct Listener {
    teller: Option<Part<Teller>>,
    told_anchor: Option<i64>,
}

impl Listener {
    fn on_told(&mut self, (): ()) {
        self.told_anchor = teller_handle(self.teller.as_ref(), |handle| {
            handle.last_success_time().map(|time| time.unix_secs)
        });
    }
}

#[test]
fn a_part_told_of_a_reply_reads_the_poll_with_that_reply_banked() {
    let listener = told_of_the_first_reply(
        Listener::default(),
        Listener::on_told,
        |listener, teller| {
            listener.teller = Some(teller);
        },
    );

    assert_ne!(
        listener.with(|listener| listener.told_anchor),
        None,
        "the widget told of a fresh reply must not read the part's poll as never refreshed"
    );
}

/// Marks stale every reply its teller tells it of, as on a late decode failure.
#[derive(Default)]
struct Rejecter {
    teller: Option<Part<Teller>>,
}

impl Rejecter {
    fn on_told(&mut self, (): ()) {
        teller_handle(self.teller.as_ref(), Poll::mark_stale);
    }
}

#[test]
fn a_part_told_of_a_reply_can_still_reject_it() {
    let rejecter = told_of_the_first_reply(
        Rejecter::default(),
        Rejecter::on_told,
        |rejecter, teller| {
            rejecter.teller = Some(teller);
        },
    );

    assert_eq!(
        rejecter.with(|rejecter| teller_handle(rejecter.teller.as_ref(), Poll::last_success_time)),
        None,
        "a reply the told widget marked stale must not make the poll read as fresh"
    );
}
