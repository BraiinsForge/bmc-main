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

    fn on_reply_enable_after_nine_seconds(&mut self, handle: &Handle, response: &FetchResponse) {
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
