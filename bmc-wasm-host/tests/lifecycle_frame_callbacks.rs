// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_host::lifecycle::{LifecycleState, frame_callback_enabled, has_render_target};

#[test]
fn leaving_widget_keeps_render_target_but_does_not_run_animation_frames() {
    assert!(
        has_render_target(LifecycleState::Leaving),
        "leaving widgets must keep their last rendered buffer available for scene transitions"
    );
    assert!(
        !frame_callback_enabled(LifecycleState::Leaving),
        "leaving widgets are no longer active, so runtime animation wakeups must be gated"
    );
}

#[test]
fn visible_widget_runs_animation_frames() {
    assert!(
        frame_callback_enabled(LifecycleState::Visible),
        "the active visible widget must keep runtime animations alive"
    );
}
