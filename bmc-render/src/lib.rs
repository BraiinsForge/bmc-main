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

//! Rendering engine for the WASM widget system.
//!
//! Extracted from `bmc-wasm-runtime` to enable reuse by the gallery dev tool
//! and other native-compilation consumers. Provides:
//!
//! - [`renderer::Renderer`] trait and [`gpu::FemtoVgRenderer`] implementation
//! - Tree deserialization, layout (via taffy), and rendering
//! - Interaction state (hit testing, touch events)
//! - UI components (buttons, notifications)
//! - Animation and transition interpolation
//!
//! # `glam` is intentionally part of this crate's public surface.
//!
//! [`PrevDrawValues`] embeds [`glam::Quat`] and [`glam::Vec3`] directly, and
//! this crate re-exports both types. Anything that consumes host-side draw
//! state transitively depends on `glam`. That is deliberate: the host's
//! animation and transition pipeline is built around glam math (see
//! [`gpu::mesh`]'s `compute_mvp` / `quat_to_mat3` etc.), so swapping in a
//! private wrapper would only push the dependency one level outward without
//! buying anything.
//!
//! The guest-side SDK keeps glam at arm's length — `bmc_wasm_sdk::Orientation`
//! is the public guest type and converts to/from `glam::Quat` only behind
//! `From` impls. Don't try to mirror that pattern here; the host is glam-bound
//! by design.

pub mod animation;
pub mod components;
pub mod gpu;
pub mod interaction;
#[cfg(any(feature = "profiling", test))]
pub mod proc_mem;
#[cfg(any(feature = "profiling", test))]
pub mod profile;
pub mod renderer;
pub mod tree;

/// Maximum image size, in RGBA pixels, the host will decode.
pub const MAX_DECODE_IMAGE_PIXELS: u64 = 4_194_304;
/// Decoder allocation budget, above RGBA output so decoders keep working buffers.
pub const MAX_DECODE_IMAGE_ALLOC_BYTES: u64 = 24 * 1024 * 1024;

/// Decode image bytes to RGBA off the render thread — fit within `w`×`h`
/// (letterbox) or cover-crop to exactly `w`×`h` (fill).
pub use gpu::bitmap::{decode_scaled_to_cover, decode_scaled_to_fit};
pub use renderer::RendererAssetResolver;

#[cfg(all(test, target_os = "linux"))]
mod test_harness;

// Re-export colors and color macro from protocol crate
pub mod colors {
    pub use bmc_wasm_protocol::colors::*;
}
pub use tree::{
    ProcessContext, deserialize_tree, layout_and_render, layout_and_render_with_asset_resolver,
    process_tree,
};

// ── State types needed by layout_and_render ─────────────────────────

pub use glam::{Quat, Vec3};

use bmc_wasm_protocol::colors::Color;

/// State for a single running animation instance.
#[derive(Debug, Clone)]
pub struct AnimationState {
    pub elapsed_ms: u32,
    /// Frame counter when last seen (for GC).
    pub last_seen_frame: u64,
}

/// Captured static values of a draw command for transition interpolation.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PrevDrawValues {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Color,
    /// Orbit angle (for `Orbit` draw commands).
    pub angle: f32,
    pub radius: f32,
    /// Rotation angle (for `Rotated` draw commands).
    pub rotation: f32,
    /// Arc start angle in radians.
    pub arc_start_angle: f32,
    /// Arc sweep in radians.
    pub arc_sweep: f32,
    /// Arc stroke width.
    pub arc_width: f32,
    /// Sphere camera center latitude (degrees).
    pub center_lat: f32,
    /// Sphere camera center longitude (degrees).
    pub center_lon: f32,
    /// Sphere camera distance (unitless, in sphere radii).
    pub zoom: f32,
    /// Light direction latitude (degrees).
    pub light_lat: f32,
    /// Light direction longitude (degrees).
    pub light_lon: f32,
    // -- Mesh fields (for 3D mesh transitions) --
    /// Orientation quaternion [x, y, z, w].
    pub orientation: Quat,
    /// Camera field of view (degrees).
    pub fov: f32,
    /// Camera distance from origin.
    pub distance: f32,
    /// Uniform scale factor.
    pub mesh_scale: f32,
    /// Position offset [x, y, z].
    pub position: Vec3,
    /// Light direction pitch (degrees).
    pub light_pitch: f32,
    /// Light direction yaw (degrees).
    pub light_yaw: f32,
    /// Ambient light level (0.0–1.0).
    pub ambient: f32,
    /// Specular highlight strength (0.0–1.0).
    pub specular: f32,
}

/// State for a single transition instance.
#[derive(Debug, Clone)]
pub struct TransitionState {
    /// Values we are interpolating from.
    pub from: PrevDrawValues,
    /// Previous target values (to detect changes).
    pub target: PrevDrawValues,
    pub elapsed_ms: u32,
    pub last_seen_frame: u64,
}

/// Composite key for the host's `transition_states` map:
/// `(canvas_index, transition_id_hash)`.
///
/// The hash side is the FNV1a-32 digest of the widget's
/// `Draw::transition(id, ...)` argument. Keying on the widget-supplied
/// id (instead of the draw's position within the canvas) lets transition
/// state follow the logical draw across tree-shape changes — an optional
/// sibling appearing or disappearing no longer reshuffles state into
/// the wrong draws.
pub type TransitionStateKey = (u16, u32);

/// State for a modal dialog (animation, scroll)
#[derive(Debug, Default)]
pub struct ModalState {
    /// Current open state (tracked for transition detection)
    pub is_open: bool,
    /// Animation progress: 0.0 = closed, 1.0 = fully open
    pub animation_progress: f32,
}

/// State for a scroll container
#[derive(Debug, Default)]
pub struct ScrollState {
    /// Current scroll offset (pixels from top)
    pub scroll_offset: f32,
}

/// Per-frame timing breakdown (microseconds).
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTimings {
    /// Total WASM interpreter time (outer envelope, includes tree processing).
    pub wasm_us: u32,
    /// Tree binary deserialization.
    pub deserialize_us: u32,
    /// Taffy tree build + layout computation.
    pub layout_us: u32,
    /// render_taffy_node + modal rendering.
    pub render_us: u32,
    /// FemtoVG canvas.flush().
    pub flush_us: u32,
}
