// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_host::lifecycle::{
    LifecycleState, frame_callback_enabled, has_render_target, should_render,
};

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

#[test]
fn transition_incoming_requests_one_render_for_target_owning_states() {
    for state in [
        LifecycleState::Prepared,
        LifecycleState::Entering,
        LifecycleState::Visible,
        LifecycleState::Leaving,
    ] {
        assert!(
            should_render(state),
            "{state:?} should render once for transition warm-up"
        );
    }
}

#[test]
fn transition_incoming_does_not_render_dormant_widget() {
    assert!(
        !should_render(LifecycleState::Dormant),
        "Dormant widgets have no render target to warm"
    );
}
