// Copyright (C) 2026  Braiins Systems s.r.o.

use std::time::{Duration, Instant};

use bmc_wasm_host::main_loop::HostLifetime;

#[test]
fn bootstrap_idle_waits_indefinitely() {
    let lt = HostLifetime::new();
    let now = Instant::now();
    assert!(lt.should_continue(0, false, now));
    assert_eq!(lt.poll_timeout_contribution(now), None);
}

#[test]
fn first_accept_flips_ever_had_slot() {
    let mut lt = HostLifetime::new();
    lt.note_accept();
    assert!(lt.should_continue(1, false, Instant::now()));
}

#[test]
fn post_disconnect_grace_window() {
    let mut lt = HostLifetime::new();
    lt.note_accept();
    let t0 = Instant::now();
    lt.note_disconnect(t0);
    assert!(lt.should_continue(0, false, t0 + Duration::from_millis(50)));
    assert!(lt.should_continue(0, false, t0 + Duration::from_millis(99)));
    assert!(!lt.should_continue(0, false, t0 + Duration::from_millis(100)));
}

#[test]
fn reaccept_clears_disconnect_state() {
    let mut lt = HostLifetime::new();
    lt.note_accept();
    lt.note_disconnect(Instant::now());
    lt.note_accept();
    assert!(lt.should_continue(1, false, Instant::now() + Duration::from_millis(200)));
}

#[test]
fn failed_first_load_uses_disconnect_grace_window() {
    let mut lt = HostLifetime::new();
    let t0 = Instant::now();

    lt.note_failed_load(t0);

    assert!(lt.should_continue(0, false, t0));
    assert!(lt.should_continue(0, false, t0 + Duration::from_millis(99)));
    assert!(!lt.should_continue(0, false, t0 + Duration::from_millis(100)));
    assert_eq!(
        lt.poll_timeout_contribution(t0 + Duration::from_millis(25)),
        Some(Duration::from_millis(75)),
    );
}

#[test]
fn accept_after_failed_load_clears_grace_state() {
    let mut lt = HostLifetime::new();
    let t0 = Instant::now();

    lt.note_failed_load(t0);
    lt.note_accept();

    assert!(lt.should_continue(1, false, t0 + Duration::from_millis(200)));
    assert_eq!(lt.poll_timeout_contribution(t0), None);
}

#[test]
fn active_overlay_keeps_host_alive_after_grace_expires() {
    let mut lt = HostLifetime::new();
    lt.note_accept();
    let t0 = Instant::now();
    lt.note_disconnect(t0);
    assert!(!lt.should_continue(0, false, t0 + Duration::from_millis(100)));
    assert!(lt.should_continue(0, true, t0 + Duration::from_millis(100)));
}
