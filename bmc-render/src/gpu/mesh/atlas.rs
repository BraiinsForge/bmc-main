// Copyright (C) 2026  Braiins Systems s.r.o.

//! Per-slot dirty-state and atlas geometry constants.
//!
//! The mesh renderer renders meshes into one of `MAX_SLOTS` cells of a
//! shared atlas texture. Each slot tracks the last-rendered draw args so
//! the next frame can skip GL work when the parameters haven't changed.

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
/// Sentinel returned when mesh registration fails.
pub(super) const INVALID_MESH_ID: u16 = 0;

/// Dirty-check state for a single atlas slot. Stores the last-rendered
/// `(mesh_id, args)` pair; a `None` `prev_args` represents "never rendered
/// yet" and forces a first-frame draw.
#[derive(Clone)]
pub(super) struct SlotState {
    prev_mesh_id: u16,
    prev_args: Option<MeshDrawArgs>,
}

impl SlotState {
    pub(super) fn new() -> Self {
        Self {
            prev_mesh_id: u16::MAX,
            prev_args: None,
        }
    }

    /// Returns true if the slot needs re-rendering for the given parameters.
    pub(super) fn check_and_update(&mut self, mesh_id: u16, args: &MeshDrawArgs) -> bool {
        let dirty = self.prev_mesh_id != mesh_id
            || self
                .prev_args
                .as_ref()
                .is_none_or(|prev| args.dirty_against(prev));
        if dirty {
            self.prev_mesh_id = mesh_id;
            self.prev_args = Some(*args);
        }
        dirty
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

pub(super) fn mesh_id_from_storage_index(index: usize) -> Option<u16> {
    let one_based = index.checked_add(1)?;
    u16::try_from(one_based).ok()
}

pub(super) fn mesh_id_to_storage_index(mesh_id: u16) -> Option<usize> {
    mesh_id.checked_sub(1).map(usize::from)
}

#[cfg(test)]
mod tests {
    use super::{
        DIRTY_EPSILON, INVALID_MESH_ID, is_dirty, mesh_id_from_storage_index,
        mesh_id_to_storage_index,
    };

    #[test]
    fn mesh_ids_are_one_based() {
        assert_eq!(mesh_id_from_storage_index(0), Some(1));
        assert_eq!(mesh_id_to_storage_index(1), Some(0));
    }

    #[test]
    fn invalid_mesh_id_does_not_map_to_storage() {
        assert_eq!(mesh_id_to_storage_index(INVALID_MESH_ID), None);
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
}
