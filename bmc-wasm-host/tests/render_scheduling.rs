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
