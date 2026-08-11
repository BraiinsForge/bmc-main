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

//! Per-slot dirty-state and atlas geometry constants.
//!
//! The mesh renderer renders meshes into one of `MAX_SLOTS` cells of a
//! shared atlas texture. Each slot tracks the last-rendered draw args so
//! the next frame can skip GL work when the parameters haven't changed.

use bmc_wasm_protocol::MeshId;

use super::MeshDrawArgs;

/// AABB record (6 × `f32`) immediately follows the binary header.
pub(super) const AABB_SIZE: usize = 24;

/// Epsilon for dirty-checking mesh parameters.
const DIRTY_EPSILON: f32 = 0.001;

/// Atlas grid: 3 columns × 3 rows = 9 slots.
pub(super) const ATLAS_COLS: u32 = 3;
pub(super) const ATLAS_ROWS: u32 = 3;
/// Per-slot pixel size (atlas is `ATLAS_COLS * SLOT_SIZE` × `ATLAS_ROWS * SLOT_SIZE`).
pub(super) const SLOT_SIZE: u32 = 320;
/// Total atlas dimensions.
pub(super) const ATLAS_W: u32 = ATLAS_COLS * SLOT_SIZE;
pub(super) const ATLAS_H: u32 = ATLAS_ROWS * SLOT_SIZE;
/// Maximum number of atlas slots.
pub(super) const MAX_SLOTS: u32 = ATLAS_COLS * ATLAS_ROWS;

/// The atlas dimensions are cast unchecked to `i32` at every `glViewport`
/// call site. Lock the invariant in: anything that bumps `ATLAS_COLS *
/// SLOT_SIZE` (or rows) past `i32::MAX / 2` would silently produce negative
/// viewport arguments. The 2× margin keeps a comfortable safety buffer.
const _: () = assert!(ATLAS_W.saturating_mul(2) < i32::MAX as u32);
const _: () = assert!(ATLAS_H.saturating_mul(2) < i32::MAX as u32);
/// Dirty-check state for a single atlas slot. Stores the last-rendered
/// `(mesh_id, args)` pair; `None` for either field represents "never
/// rendered yet" and forces a first-frame draw.
#[derive(Clone, Default)]
pub(super) struct SlotState {
    prev_mesh_id: Option<MeshId>,
    prev_args: Option<MeshDrawArgs>,
}

impl SlotState {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Returns true if the slot needs re-rendering for the given parameters.
    pub(super) fn check_and_update(&mut self, mesh_id: MeshId, args: &MeshDrawArgs) -> bool {
        let dirty = self.prev_mesh_id != Some(mesh_id)
            || self
                .prev_args
                .as_ref()
                .is_none_or(|prev| args.dirty_against(prev));
        if dirty {
            self.prev_mesh_id = Some(mesh_id);
            self.prev_args = Some(*args);
        }
        dirty
    }

    pub(super) fn invalidate_mesh(&mut self, mesh_id: MeshId) {
        if self.prev_mesh_id == Some(mesh_id) {
            self.prev_mesh_id = None;
        }
    }
}

/// `NaN` is the sentinel for "lighting disabled" (see `MeshView::light: None`
/// in the SDK). A naive `(old - new).abs() > eps` returns `false` for any
/// `NaN` operand, so a `Some(...)` → `None` transition would never dirty the
/// slot and the cached lit frame would stick. Either operand being `NaN`
/// therefore forces a re-render.
pub(super) fn is_dirty(old: f32, new: f32) -> bool {
    old.is_nan() || new.is_nan() || (old - new).abs() > DIRTY_EPSILON
}

/// Once-per-process warning when a widget asks for more atlas slots than the
/// renderer offers. Logged at `warn` level so devs catch it during testing
/// without the stream being drowned in per-frame repeats once 9 dice roll
/// over to 10+.
pub(super) fn warn_slot_overflow_once(slot_index: u32) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "mesh: slot_index {slot_index} exceeds MAX_SLOTS ({MAX_SLOTS}); \
             excess draws are suppressed. Reduce concurrent meshes per frame."
        );
    }
}

pub(super) fn mesh_id_from_storage_index(index: usize) -> Option<MeshId> {
    let one_based = u16::try_from(index.checked_add(1)?).ok()?;
    MeshId::from_wire(one_based)
}

pub(super) fn mesh_id_to_storage_index(mesh_id: MeshId) -> usize {
    usize::from(mesh_id.to_wire() - 1)
}

#[cfg(test)]
mod tests {
    use super::{
        DIRTY_EPSILON, SlotState, is_dirty, mesh_id_from_storage_index, mesh_id_to_storage_index,
    };
    use crate::gpu::mesh::{MeshDrawArgs, MeshHighlight, MeshLighting, MeshTransform};

    fn draw_args() -> MeshDrawArgs {
        MeshDrawArgs {
            transform: MeshTransform {
                fov: 0.0,
                distance: 0.0,
                quat: [0.0; 4],
                position: [0.0; 3],
                scale: 0.0,
            },
            lighting: MeshLighting {
                pitch: 0.0,
                yaw: 0.0,
                ambient: 0.0,
                specular: 0.0,
            },
            highlight: MeshHighlight {
                u_min: 0.0,
                v_min: 0.0,
                u_max: 0.0,
                v_max: 0.0,
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
        }
    }

    #[test]
    fn mesh_ids_are_one_based() {
        let id = mesh_id_from_storage_index(0).expect("BUG: index 0 must round-trip");
        assert_eq!(id.to_wire(), 1);
        assert_eq!(mesh_id_to_storage_index(id), 0);
    }

    #[test]
    fn is_dirty_treats_either_nan_operand_as_dirty() {
        // Existing behavior: first render sentinel.
        assert!(is_dirty(f32::NAN, 30.0));
        // Regression: lighting toggled off (Some(pitch) → None ≡ NaN) must
        // re-render. Naive `(old - new).abs()` returns NaN > eps == false.
        assert!(is_dirty(30.0, f32::NAN));
        // Both NaN preserves the prior "first render forced" semantics.
        assert!(is_dirty(f32::NAN, f32::NAN));
    }

    #[test]
    fn is_dirty_compares_finite_values_against_epsilon() {
        assert!(!is_dirty(1.0, 1.0));
        assert!(!is_dirty(1.0, 1.0 + DIRTY_EPSILON / 2.0));
        assert!(is_dirty(1.0, 1.0 + DIRTY_EPSILON * 2.0));
    }

    #[test]
    fn invalidating_matching_mesh_keeps_args_and_forces_redraw() {
        let id = mesh_id_from_storage_index(0).expect("BUG: index 0 must produce an ID");
        let args = draw_args();
        let mut slot = SlotState::new();
        assert!(slot.check_and_update(id, &args));
        let prev_args = slot.prev_args;

        slot.invalidate_mesh(id);

        assert_eq!(slot.prev_mesh_id, None);
        assert_eq!(slot.prev_args, prev_args);
        assert!(slot.check_and_update(id, &args));
    }

    #[test]
    fn invalidating_nonmatching_mesh_leaves_slot_unchanged() {
        let id = mesh_id_from_storage_index(0).expect("BUG: index 0 must produce an ID");
        let other = mesh_id_from_storage_index(1).expect("BUG: index 1 must produce an ID");
        let args = draw_args();
        let mut slot = SlotState::new();
        assert!(slot.check_and_update(id, &args));
        let prev_args = slot.prev_args;

        slot.invalidate_mesh(other);

        assert_eq!(slot.prev_mesh_id, Some(id));
        assert_eq!(slot.prev_args, prev_args);
    }
}
