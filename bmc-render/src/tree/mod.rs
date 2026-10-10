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

//! Tree deserialization and layout computation.

use std::sync::atomic::{AtomicBool, Ordering};

/// When `DEBUG_LAYOUT=1` env var is set, draw colored outlines around every layout node.
static DEBUG_LAYOUT: AtomicBool = AtomicBool::new(false);

/// When `BMC_GPU_PASS_TIMING=1` is set, block on the GPU at each render-pass
/// boundary so a frame's GPU cost can be attributed per pass.
///
/// Not free to observe: the fences serialise passes that normally pipeline, so
/// read the split as proportions rather than as absolute times.
static GPU_PASS_TIMING: AtomicBool = AtomicBool::new(false);

/// Call once at startup to check the `DEBUG_LAYOUT` env var.
pub fn init_debug_flags() {
    if std::env::var("DEBUG_LAYOUT").is_ok_and(|v| v == "1") {
        DEBUG_LAYOUT.store(true, Ordering::Relaxed);
    }
    if std::env::var("BMC_GPU_PASS_TIMING").is_ok_and(|v| v == "1") {
        GPU_PASS_TIMING.store(true, Ordering::Relaxed);
    }
}

/// Whether per-pass GPU fences are enabled; see [`GPU_PASS_TIMING`].
#[must_use]
pub fn gpu_pass_timing_enabled() -> bool {
    GPU_PASS_TIMING.load(Ordering::Relaxed)
}

/// Returns whether debug layout outlines are enabled.
pub fn debug_layout_enabled() -> bool {
    DEBUG_LAYOUT.load(Ordering::Relaxed)
}

/// Stop drawing outlines into the frame, and say whether `DEBUG_LAYOUT` asked for them.
///
/// For a host that draws [`TreeResult::layout_outlines`] over the frame itself.
/// Its frames feed damage tracking, where a flag the static layer's key ignores
/// leaves strokes behind in the buffers it reuses.
pub fn take_debug_layout() -> bool {
    DEBUG_LAYOUT.swap(false, Ordering::Relaxed)
}

mod decode;
mod frame;
mod layout;
mod paint;
mod walk;

// Re-export for other modules
pub use crate::components::notification::{
    measure_notification_banner, render_notification_banner,
};
pub use bmc_ui::tree::*;

pub(crate) use paint::render_taffy_node;

pub use layout::NodeContext;
pub(crate) use layout::{build_taffy_node, compute_taffy_layout, min_content_paragraph_width};

pub use frame::{
    LayerUse, ProcessContext, layout_and_render, layout_and_render_with_asset_resolver,
    process_tree,
};
pub(crate) use walk::{AnimationContext, outline_box};
#[cfg(test)]
pub(crate) use walk::{DynamicHalf, Outlines, stroke_inside};
pub use walk::{EmitMode, LayoutOutline, StaticLayer, TreeResult};

pub use decode::{deserialize_tree, deserialize_tree_keyed};

#[cfg(test)]
mod tests;
