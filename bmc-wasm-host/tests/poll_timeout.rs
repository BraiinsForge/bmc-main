// Copyright (C) 2026  Braiins Systems s.r.o.

use std::time::Duration;

use bmc_wasm_host::main_loop::{SlotPollInputs, compute_poll_timeout_from_inputs};

#[test]
fn no_slots_and_no_grace_returns_indefinite() {
    assert_eq!(compute_poll_timeout_from_inputs(&[], None), -1);
}

#[test]
fn renderable_dirty_slot_with_elapsed_floor_returns_zero() {
    let slot = SlotPollInputs {
        is_renderable: true,
        surface_needs_render: true,
        min_inter_frame_remaining: None,
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), 0);
}

#[test]
fn prepared_dirty_slot_without_frame_callbacks_returns_zero() {
    let slot = SlotPollInputs {
        is_renderable: true,
        frame_callback_enabled: false,
        surface_needs_render: true,
        min_inter_frame_remaining: None,
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), 0);
}

#[test]
fn renderable_dirty_slot_with_pending_floor_returns_remaining_ms() {
    let slot = SlotPollInputs {
        is_renderable: true,
        surface_needs_render: true,
        min_inter_frame_remaining: Some(Duration::from_millis(5)),
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), 5);
}

#[test]
fn renderable_dirty_slot_just_rendered_returns_min_inter_frame() {
    // Production `WidgetSlot::poll_inputs(now)` translates `last_render_at == now`
    // into `min_inter_frame_remaining == MIN_INTER_FRAME`. Keep this test at the
    // policy boundary so a later refactor cannot accidentally wake immediately
    // after a freshly-rendered dirty frame.
    let slot = SlotPollInputs {
        is_renderable: true,
        surface_needs_render: true,
        min_inter_frame_remaining: Some(Duration::from_millis(8)),
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), 8);
}

#[test]
fn async_io_ceiling_clamps_lower_than_slot_delay() {
    // Visible slot scheduling 250 ms ahead via next_frame_delay; has_pending_io
    // forces the 100 ms ceiling. The minimum across the two contributions wins.
    let slot = SlotPollInputs {
        is_renderable: true,
        frame_callback_enabled: true,
        next_frame_delay: Some(250),
        has_pending_io: true,
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), 100);
}

#[test]
fn non_rendering_slot_with_pending_io_wakes_for_delivery_polling() {
    let slot = SlotPollInputs {
        is_renderable: false,
        has_pending_io: true,
        ..SlotPollInputs::default()
    };

    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), 100);
}

#[test]
fn entering_slot_wanting_next_frame_does_not_force_zero_timeout() {
    // Locks in the frame-callback gate: Entering has frame_callback_enabled = false,
    // so even if the runtime returns wants_next_frame()/animation_wants_immediate,
    // the slot must NOT pull the poll timeout to 0. Without the gate, Entering
    // slots that misbehave would spin the host at 100 % CPU.
    let slot = SlotPollInputs {
        is_renderable: true,
        frame_callback_enabled: false, // Entering
        animation_wants_immediate: true,
        surface_needs_render: false,
        next_frame_delay: Some(0),
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), -1);
}

#[test]
fn prepared_slot_wanting_next_frame_without_dirty_surface_does_not_wake() {
    let slot = SlotPollInputs {
        is_renderable: true,
        frame_callback_enabled: false,
        animation_wants_immediate: true,
        surface_needs_render: false,
        next_frame_delay: Some(0),
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), -1);
}

#[test]
fn retry_timer_contributes_even_for_non_renderable_slot() {
    // A Dormant slot that is `resource_blocked` (allocation failed earlier)
    // must still wake the host so the 1 s retry can fire. Even with is_renderable
    // false, retry_in contributes to the timeout fold.
    let slot = SlotPollInputs {
        is_renderable: false,
        is_blocked: true,
        retry_in: Some(Duration::from_millis(750)),
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), 750);
}

#[test]
fn grace_remainder_contributes_when_no_slots() {
    assert_eq!(
        compute_poll_timeout_from_inputs(&[], Some(Duration::from_millis(80))),
        80,
    );
}

#[test]
fn grace_remainder_does_not_contribute_when_slot_is_active() {
    let slot = SlotPollInputs::default();
    assert_eq!(
        compute_poll_timeout_from_inputs(&[slot], Some(Duration::ZERO)),
        -1,
    );
}

#[test]
fn minimum_across_multiple_slot_contributions_wins() {
    let s1 = SlotPollInputs {
        is_renderable: true,
        surface_needs_render: true,
        min_inter_frame_remaining: Some(Duration::from_millis(7)),
        ..SlotPollInputs::default()
    };
    let s2 = SlotPollInputs {
        is_renderable: true,
        surface_needs_render: true,
        min_inter_frame_remaining: Some(Duration::from_millis(3)),
        ..SlotPollInputs::default()
    };
    assert_eq!(compute_poll_timeout_from_inputs(&[s1, s2], None), 3);
}

#[test]
fn blocked_renderable_slot_skips_frame_branches_but_contributes_retry() {
    // The classify-and-skip rule: a slot that is blocked must not advertise
    // a frame-cadence wake, but its retry_in still contributes.
    let slot = SlotPollInputs {
        is_renderable: true,
        is_blocked: true,
        surface_needs_render: true,
        min_inter_frame_remaining: Some(Duration::from_millis(5)),
        retry_in: Some(Duration::from_millis(900)),
        ..SlotPollInputs::default()
    };
    // Only retry_in contributes; surface_needs_render is gated off by is_blocked.
    assert_eq!(compute_poll_timeout_from_inputs(&[slot], None), 900);
}
