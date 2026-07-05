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
    lt.note_accept();
    assert!(lt.should_continue(1, false));
}

#[test]
fn exits_immediately_after_last_disconnect() {
    let mut lt = HostLifetime::new();
    lt.note_accept();
    assert!(!lt.should_continue(0, false));
}

#[test]
fn failed_first_load_exits_immediately() {
    let mut lt = HostLifetime::new();
    lt.note_failed_load();
    assert!(!lt.should_continue(0, false));
}

#[test]
fn active_overlay_keeps_host_alive_without_slots() {
    let mut lt = HostLifetime::new();
    lt.note_accept();
    assert!(lt.should_continue(0, true));
    assert!(!lt.should_continue(0, false));
}
