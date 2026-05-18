// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_host::slot::{RenderGate, SlotRenderInputs, slot_needs_render_from_inputs};

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
