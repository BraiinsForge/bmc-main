// Copyright (C) 2026  Braiins Systems s.r.o.

use std::time::{Duration, Instant};

use bmc_wasm_host::slot::{
    RenderGate, SlotRenderInputs, refresh_runtime_frame_due_at, slot_needs_render_from_inputs,
};

#[test]
fn visible_runtime_frame_due_renders_without_surface_dirty() {
    let inputs = SlotRenderInputs {
        gate: RenderGate::Renderable,
        runtime_frame_due: true,
        ..SlotRenderInputs::default()
    };

    assert!(
        slot_needs_render_from_inputs(inputs),
        "expired runtime frame deadlines must render even without a Wayland event"
    );
}

#[test]
fn delivery_refresh_does_not_postpone_existing_runtime_deadline() {
    let scheduled_at = Instant::now();
    let existing_due_at = scheduled_at + Duration::from_millis(16);
    let late_refresh_at = scheduled_at + Duration::from_millis(30);

    let refreshed =
        refresh_runtime_frame_due_at(Some(existing_due_at), true, Some(16), late_refresh_at);

    assert_eq!(
        refreshed,
        Some(existing_due_at),
        "an overdue frame remains due instead of being re-anchored after delivery polling"
    );
}
