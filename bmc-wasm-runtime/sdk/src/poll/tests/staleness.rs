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
fn not_stale_while_replies_succeed() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    deliver_at(&mut reg, &mut be, 1, true, 0);
    assert!(
        !reg.is_stale(h, 1_000_000),
        "no outstanding failure is never stale"
    );
}

#[wasm_test]
fn is_stale_once_failure_ages_past_stale_factor() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    deliver_at(&mut reg, &mut be, 1, true, 0);
    deliver_at(&mut reg, &mut be, 2, false, 8);
    assert!(!reg.is_stale(h, 7), "age 7 s is within the 7.5 s threshold");
    assert!(reg.is_stale(h, 8), "age 8 s exceeds the 7.5 s threshold");
}

#[wasm_test]
fn success_clears_staleness() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    deliver_at(&mut reg, &mut be, 1, true, 0);
    deliver_at(&mut reg, &mut be, 2, false, 8);
    assert!(reg.is_stale(h, 8), "stale once the failure ages out");
    deliver_at(&mut reg, &mut be, 3, true, 20);
    assert!(!reg.is_stale(h, 100), "a fresh success clears the failure");
    assert_eq!(reg.last_success_age(h, 25), Some(Duration::from_secs(5)));
    assert_eq!(reg.last_success_secs(h), Some(20));
}

#[wasm_test]
fn forced_retry_does_not_count_as_a_refresh() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    deliver_at(&mut reg, &mut be, 1, true, 0);
    // A 2xx the handler rejected via retry(): no refresh despite ok=true.
    let id = FetchRequestId::from_wire(2).expect("BUG: test id is nonzero");
    let action = begin_reply(&mut reg, &mut be, id);
    assert!(
        matches!(action, ReplyAction::Deliver(index, _) if index == h),
        "a reply to the live request goes to the handler: {action:?}"
    );
    reg.request_retry(h);
    reschedule(&mut reg, &mut be, h, true, 100);
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

#[wasm_test]
fn mark_stale_reverts_the_corrupt_reply_to_the_last_good_anchor() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    deliver_at(&mut reg, &mut be, 1, true, 0); // good image banked at t=0
    // A later 2xx banks ok (header passed) but its decode fails async.
    deliver_at(&mut reg, &mut be, 2, true, 100);
    assert_eq!(reg.last_success_secs(h), Some(100), "the reply banked ok");
    reg.mark_stale(h, 100);
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

#[wasm_test]
fn restore_anchor_seeds_only_before_the_first_success() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
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

#[wasm_test]
fn reset_staleness_forgets_all_history() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
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

#[wasm_test]
fn is_offline_after_failing_past_grace_without_ever_loading() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG); // interval 5s, stale_factor 1.5 → 7.5s grace
    start(&mut reg, &mut be, h);
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

#[wasm_test]
fn not_stale_without_prior_success_or_interval() {
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
    deliver_at(&mut reg, &mut be, 1, false, 0);
    assert!(
        !reg.is_stale(h, 1_000_000),
        "one-shot polls never stale on a cadence"
    );
    assert_eq!(reg.last_success_age(h, 5), None);
}

#[wasm_test]
fn a_first_reply_that_proves_bad_starts_the_offline_grace() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG); // 7.5 s grace
    start(&mut reg, &mut be, h);
    deliver_at(&mut reg, &mut be, 1, true, 0);

    reg.mark_stale(h, 10);

    assert!(
        !reg.is_offline(h, 17),
        "the grace runs from when the reply proved bad"
    );
    assert!(
        reg.is_offline(h, 18),
        "a poll that never loaded good data is offline once the grace runs out"
    );
}

#[wasm_test]
fn retries_inside_the_grace_do_not_restart_it() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG); // 1 s retry, 7.5 s grace
    start(&mut reg, &mut be, h);
    deliver_at(&mut reg, &mut be, 1, false, 0);
    deliver_at(&mut reg, &mut be, 2, false, 5);

    assert!(
        reg.is_offline(h, 8),
        "the grace runs from the first failure of the streak"
    );
}
