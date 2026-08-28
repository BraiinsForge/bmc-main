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

//! Handing a frame's GL target back after an offscreen pass.

use std::num::NonZeroU32;

use glow::HasContext;

/// GL state an offscreen pass overwrites, so it can hand back what it found.
///
/// femtovg re-applies the framebuffer and viewport only while processing a
/// `SetRenderTarget` command, which a frame staying on `RenderTarget::Screen`
/// never emits — so whatever a pass leaves bound is what the flush replays the
/// whole frame through. Framebuffer `0` is `GL_FRAMEBUFFER_UNDEFINED` on a
/// surfaceless context and fails every draw; a viewport left at the pass's own
/// size rescales them.
pub(super) struct OffscreenPassState {
    framebuffer: Option<glow::NativeFramebuffer>,
    viewport: [i32; 4],
    scissor_box: [i32; 4],
    scissor_enabled: bool,
}

impl OffscreenPassState {
    /// # Safety
    ///
    /// The caller's GL context must be current.
    pub(super) unsafe fn capture(gl: &glow::Context) -> Self {
        // SAFETY: the caller guarantees a current context.
        unsafe {
            let bound = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let mut scissor_box = [0; 4];
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut scissor_box);
            Self {
                framebuffer: NonZeroU32::new(bound.cast_unsigned()).map(glow::NativeFramebuffer),
                viewport,
                scissor_box,
                scissor_enabled: gl.is_enabled(glow::SCISSOR_TEST),
            }
        }
    }

    /// # Safety
    ///
    /// The caller's GL context must be current.
    pub(super) unsafe fn restore(&self, gl: &glow::Context) {
        // SAFETY: the caller guarantees a current context.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, self.framebuffer);
            let [x, y, width, height] = self.viewport;
            gl.viewport(x, y, width, height);
            let [sx, sy, swidth, sheight] = self.scissor_box;
            gl.scissor(sx, sy, swidth, sheight);
            if self.scissor_enabled {
                gl.enable(glow::SCISSOR_TEST);
            } else {
                gl.disable(glow::SCISSOR_TEST);
            }
        }
    }
}
