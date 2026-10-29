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

//! Recorder check that an overlay frees the static layer it captured.

use bmc_render::renderer::Renderer;
use bmc_render::renderer::test_support::ShapingRecorder;

use crate::SystemOverlay;

/// Draw one frame of `overlay` through `render` on a `size` surface,
/// then assert the frame captured a layer and releasing the overlay frees it.
///
/// # Panics
///
/// Panics when the frame captured no layer, or when the release leaves one behind.
pub fn assert_release_frees_the_layer<O: SystemOverlay>(
    overlay: &mut O,
    (width, height): (u32, u32),
    render: impl FnOnce(&mut O, &mut dyn Renderer),
) {
    #[expect(clippy::cast_precision_loss, reason = "surface sizes are small")]
    let mut layers = ShapingRecorder::new(width as f32, height as f32).with_static_layers();
    render(overlay, &mut layers);
    assert_eq!(
        layers.static_layer_owners().len(),
        1,
        "the frame captures the overlay's layer"
    );
    overlay.release_renderer_resources(&mut layers);
    assert!(
        layers.static_layer_owners().is_empty(),
        "the release frees the layer"
    );
}
