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

use bmc_wasm_host::main_loop::HostLifetime;

#[test]
fn bootstrap_idle_waits_indefinitely() {
    let lt = HostLifetime::new();
    assert!(lt.should_continue(0, false));
}

#[test]
fn first_loaded_admission_marks_bootstrap_handled() {
    let mut lt = HostLifetime::new();
    lt.note_admission_outcomes(1, 0, 1);
    assert!(lt.should_continue(1, false));
}

#[test]
fn exits_immediately_after_last_disconnect() {
    let mut lt = HostLifetime::new();
    lt.note_admission_outcomes(1, 0, 1);
    assert!(!lt.should_continue(0, false));
}

#[test]
fn lone_failed_load_exits_immediately() {
    let mut lt = HostLifetime::new();
    lt.note_admission_outcomes(0, 1, 0);
    assert!(!lt.should_continue(0, false));
}

#[test]
fn rejection_among_active_admissions_keeps_host_alive() {
    let mut lt = HostLifetime::new();
    lt.note_admission_outcomes(2, 1, 2);
    assert!(lt.should_continue(2, false));
    assert!(!lt.should_continue(0, false));
}

#[test]
fn rejection_after_prior_slots_does_not_force_exit_while_slots_live() {
    let mut lt = HostLifetime::new();
    lt.note_admission_outcomes(1, 0, 1);
    lt.note_admission_outcomes(0, 1, 1);
    assert!(lt.should_continue(1, false));
}

#[test]
fn empty_burst_leaves_bootstrap_waiting() {
    let mut lt = HostLifetime::new();
    lt.note_admission_outcomes(0, 0, 0);
    assert!(lt.should_continue(0, false));
}

#[test]
fn active_overlay_keeps_host_alive_without_slots() {
    let mut lt = HostLifetime::new();
    lt.note_admission_outcomes(1, 0, 1);
    assert!(lt.should_continue(0, true));
    assert!(!lt.should_continue(0, false));
}

#[test]
fn rejection_does_not_isolate_a_pending_sibling() {
    let mut lt = HostLifetime::new();
    lt.note_admission_outcomes(0, 1, 1);
    assert!(lt.should_continue(1, false));

    lt.note_admission_outcomes(0, 1, 0);
    assert!(!lt.should_continue(0, false));
}
