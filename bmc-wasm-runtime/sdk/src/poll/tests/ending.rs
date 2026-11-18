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

#[wasm_test]
fn an_ended_poll_cancels_its_live_fetch() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);

    reg.release(h, &mut be);

    assert_eq!(be.cancels.len(), 1);
}

#[wasm_test]
fn an_ended_poll_does_not_cancel_again_what_the_host_refused_to() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    invalidate(&mut reg, &mut be, h);

    reg.release(h, &mut be);

    assert_eq!(
        be.cancels.len(),
        1,
        "the host refuses only a request already away, so asking again cannot stop it"
    );
}

#[wasm_test]
fn a_reply_already_queued_for_an_ended_poll_is_not_delivered() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);

    reg.release(h, &mut be);

    let action = deliver(&mut reg, &mut be, 1, true);
    assert!(
        matches!(action, ReplyAction::Ignore),
        "a reply to an ended poll is ignored: {action:?}"
    );
    assert_eq!(be.sends.len(), 1, "an ended poll never fetches again");
}

#[wasm_test]
fn an_ended_poll_ignores_later_calls() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    reg.release(h, &mut be);

    invalidate(&mut reg, &mut be, h);
    set_enabled(&mut reg, &mut be, h, false);
    set_enabled(&mut reg, &mut be, h, true);
    set_enabled_after(&mut reg, &mut be, h, 1_000);

    assert_eq!(be.sends.len(), 1);
    assert!(!reg.enabled(h));
}

#[wasm_test]
fn a_poll_ended_by_its_reply_handler_is_not_rescheduled() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    let id = FetchRequestId::from_wire(1).expect("BUG: test id is nonzero");
    let action = begin_reply(&mut reg, &mut be, id);
    assert!(
        matches!(action, ReplyAction::Deliver(index, _) if index == h),
        "a reply to the live request goes to the handler: {action:?}"
    );

    reg.release(h, &mut be);
    reschedule(&mut reg, &mut be, h, true, 0);

    assert_eq!(be.sends.len(), 1);
}

#[wasm_test]
fn an_ended_polls_index_is_never_reused() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let first = register_in(&mut reg, build_url, CFG);
    reg.release(first, &mut be);

    let second = register_in(&mut reg, build_url, CFG);

    assert_ne!(first, second);
}
