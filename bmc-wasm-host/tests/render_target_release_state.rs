// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_host::render_target::RenderSlotReleaseState;

#[test]
fn release_state_blocks_presented_slot_until_release() {
    let mut state = RenderSlotReleaseState::new();

    assert!(state.is_available(0));
    assert!(state.is_available(1));

    state.mark_presented(0);

    assert!(!state.is_available(0));
    assert!(state.is_available(1));

    state.mark_released(0);

    assert!(state.is_available(0));
    assert!(state.is_available(1));
}

#[test]
fn release_state_ignores_stale_out_of_range_slot_ids() {
    let mut state = RenderSlotReleaseState::new();

    assert!(!state.is_available(2));

    state.mark_presented(2);

    assert!(state.is_available(0));
    assert!(state.is_available(1));

    state.mark_presented(1);
    state.mark_released(2);

    assert!(state.is_available(0));
    assert!(!state.is_available(1));
}
