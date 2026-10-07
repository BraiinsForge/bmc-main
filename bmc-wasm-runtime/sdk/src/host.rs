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

//! Host function bindings and types.
//!
//! Off-target, where there is no host to call, the bindings panic —
//! apart from a few that exist only on `wasm32`.

// Re-export from protocol — single source of truth for wire-format enums
pub use bmc_wasm_protocol::{ButtonSize, ButtonStyle};

mod assets;
pub use assets::{
    RetainedImageDimensions, decode_image, evict_all, evict_prefix, image_dimensions,
    max_image_pixels, register_audio, register_audio_package, register_bitmap, register_bitmap_fit,
    register_bitmap_from_cache, register_bitmap_nearest, register_bitmap_nearest_package,
    register_bitmap_package, register_mesh, register_mesh_package, register_svg,
    register_svg_package,
};
#[cfg(target_arch = "wasm32")]
pub use assets::{image_dimensions_ref, register_bitmap_fit_ref};

mod audio;
pub use audio::{Volume, audio_play, audio_stop};

mod clock;
pub use clock::SystemTime;

mod random;
pub use random::random_u32;

mod render;
pub use render::{button, draw_text, fill_rect, request_frame, request_frame_after, submit_tree};

mod touch;
pub use touch::{TouchHit, get_touch_click, get_touch_drag};

mod viewport;
pub use viewport::{
    DisplayInfo, DisplayShape, SizeVariant, ViewportShape, WidgetSize, WidgetViewport,
    display_info, scale_font, widget_size, widget_viewport,
};
