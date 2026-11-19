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

//! The glue's tests count sends and cancels through the off-target test backend.

#![cfg(not(target_arch = "wasm32"))]

use super::*;
use crate::net::FetchResponse;
use crate::widget::{self, Part, PartHandle};

mod methods;

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
