// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::time::{Duration, Instant};

use bmc_wasm_host::lifecycle::LifecycleState;
use bmc_wasm_host::slot::{
    RenderGate, SlotRenderInputs, dirty_render_allowed, refresh_runtime_frame_due_at,
    slot_needs_render_from_inputs,
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

#[test]
fn dirty_surface_defers_off_screen_after_warmup_frame() {
    for state in [
        LifecycleState::Prepared,
        LifecycleState::Entering,
        LifecycleState::Leaving,
    ] {
        assert!(
            !dirty_render_allowed(state, true, false),
            "{state:?} with a committed buffer must keep presenting it — off-screen \
             re-renders are what made swipes stutter (BDK-658)"
        );
    }
}

#[test]
fn dirty_surface_renders_when_visible() {
    assert!(
        dirty_render_allowed(LifecycleState::Visible, true, false),
        "a held-back dirty flag must resolve into a render once the widget is Visible"
    );
}

#[test]
fn warmup_frame_renders_in_every_render_state() {
    for state in [
        LifecycleState::Prepared,
        LifecycleState::Entering,
        LifecycleState::Visible,
        LifecycleState::Leaving,
    ] {
        assert!(
            dirty_render_allowed(state, false, false),
            "a fresh render target has nothing to present — the warm-up frame for \
             {state:?} must paint or the compositor shows garbage"
        );
    }
}

#[test]
fn transition_incoming_forces_a_pre_transition_frame() {
    assert!(
        dirty_render_allowed(LifecycleState::Prepared, true, true),
        "the compositor's transition_incoming demands fresh content before an automatic \
         transition; the off-screen gate must not hold that frame back"
    );
}
