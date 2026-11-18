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
fn enable_from_disabled_kicks_after_debounce() {
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
    set_enabled(&mut reg, &mut be, h, true);
    assert_eq!(be.sends.len(), 1);
    assert_eq!(be.sends[0].1, Some(300));
}

#[wasm_test]
fn disable_cancels_queued_request() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    set_enabled(&mut reg, &mut be, h, false);
    assert_eq!(be.cancels.len(), 1);
    assert!(!reg.enabled(h));
}

#[wasm_test]
fn set_enabled_is_idempotent() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    set_enabled(&mut reg, &mut be, h, true);
    assert_eq!(be.sends.len(), 1);
}

#[wasm_test]
fn enable_after_schedules_at_the_given_delay() {
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
    set_enabled_after(&mut reg, &mut be, h, 7_000);
    assert!(reg.enabled(h));
    assert_eq!(be.sends.len(), 1);
    assert_eq!(be.sends[0].1, Some(7_000));
}

#[wasm_test]
fn reply_to_disabled_poll_stops_without_reschedule() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    set_enabled(&mut reg, &mut be, h, false);
    let action = deliver(&mut reg, &mut be, 1, true);
    assert!(matches!(action, ReplyAction::Stopped));
    assert_eq!(be.sends.len(), 1);
}

#[wasm_test]
fn a_request_kept_across_a_disable_and_enable_is_built_again() {
    let mut reg = Registry::default();
    let mut be = FakeBackend::new();
    be.cancel_result = false;
    let h = register_in(&mut reg, build_url, CFG);
    start(&mut reg, &mut be, h);
    set_enabled(&mut reg, &mut be, h, false);

    set_enabled(&mut reg, &mut be, h, true);
    let action = deliver(&mut reg, &mut be, 1, true);

    assert!(
        matches!(action, ReplyAction::Reissued(_)),
        "enabling runs build again, so the request built before the disable is dropped"
    );
}
