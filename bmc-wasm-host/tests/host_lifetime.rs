// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_host::main_loop::HostLifetime;

#[test]
fn bootstrap_idle_waits_indefinitely() {
    let lt = HostLifetime::new();
    assert!(lt.should_continue(0, false));
}

#[test]
fn first_accept_flips_ever_had_slot() {
    let mut lt = HostLifetime::new();
    lt.note_accept_burst(1, 0, 1);
    assert!(lt.should_continue(1, false));
}

#[test]
fn exits_immediately_after_last_disconnect() {
    let mut lt = HostLifetime::new();
    lt.note_accept_burst(1, 0, 1);
    assert!(!lt.should_continue(0, false));
}

#[test]
fn lone_failed_load_exits_immediately() {
    let mut lt = HostLifetime::new();
    lt.note_accept_burst(0, 1, 0);
    assert!(!lt.should_continue(0, false));
}

#[test]
fn rejection_among_healthy_siblings_keeps_host_alive() {
    let mut lt = HostLifetime::new();
    lt.note_accept_burst(2, 1, 2);
    assert!(lt.should_continue(2, false));
    assert!(!lt.should_continue(0, false));
}

#[test]
fn rejection_after_prior_slots_does_not_force_exit_while_slots_live() {
    let mut lt = HostLifetime::new();
    lt.note_accept_burst(1, 0, 1);
    lt.note_accept_burst(0, 1, 1);
    assert!(lt.should_continue(1, false));
}

#[test]
fn empty_burst_leaves_bootstrap_waiting() {
    let mut lt = HostLifetime::new();
    lt.note_accept_burst(0, 0, 0);
    assert!(lt.should_continue(0, false));
}

#[test]
fn active_overlay_keeps_host_alive_without_slots() {
    let mut lt = HostLifetime::new();
    lt.note_accept_burst(1, 0, 1);
    assert!(lt.should_continue(0, true));
    assert!(!lt.should_continue(0, false));
}
