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
