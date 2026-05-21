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

#[test]
fn prepared_compaction_drops_available_spare_slot_before_first_render() {
    let state = RenderSlotReleaseState::new();

    assert_eq!(state.prepared_compaction_slots([true, true], 0), vec![1]);
}

#[test]
fn prepared_compaction_drops_available_back_slot_after_submit() {
    let mut state = RenderSlotReleaseState::new();
    state.mark_presented(0);

    assert_eq!(state.prepared_compaction_slots([true, true], 1), vec![1]);
}

#[test]
fn prepared_compaction_keeps_the_only_allocated_slot() {
    let mut state = RenderSlotReleaseState::new();
    state.mark_presented(0);

    assert!(state.prepared_compaction_slots([true, false], 1).is_empty());
}
