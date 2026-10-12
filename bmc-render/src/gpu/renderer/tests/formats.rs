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

use crate::test_harness::GlHarness;
use glow::HasContext;

/// Probes which single-channel texture formats this GL context accepts as a
/// framebuffer colour attachment. GLES2 guarantees none of them: only
/// RGBA4/RGB5_A1/RGB565 renderbuffers are required to be colour-renderable,
/// so femtovg's `Gray8` (`GL_LUMINANCE`) has to be checked per driver.
///
/// Run with `-- --nocapture`. `GlHarness` is Mesa llvmpipe, far more
/// permissive than the embedded GPUs this targets, so a pass here does not
/// carry to the device — re-run the check there.
#[test]
#[expect(
    clippy::cast_possible_wrap,
    reason = "small fixed GL enum values fit i32"
)]
fn probe_single_channel_color_renderable_formats() {
    const GL_RED_EXT: u32 = 0x1903;

    // Raw GL completeness answers only, independent of femtovg's own init.
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;

    unsafe {
        let version = gl.get_parameter_string(glow::VERSION);
        let renderer_str = gl.get_parameter_string(glow::RENDERER);
        let vendor = gl.get_parameter_string(glow::VENDOR);
        let extensions = gl.get_parameter_string(glow::EXTENSIONS);

        eprintln!("== GL context ==");
        eprintln!("VERSION:  {version}");
        eprintln!("RENDERER: {renderer_str}");
        eprintln!("VENDOR:   {vendor}");
        eprintln!(
            "EXT_texture_rg present: {}",
            extensions.contains("GL_EXT_texture_rg")
        );

        eprintln!("== Framebuffer completeness (color attachment 0, 4x4) ==");

        // Control: RGBA8 is colour-renderable everywhere, GLES2 included.
        let rgba_ok = probe_format(
            gl,
            glow::RGBA as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            "RGBA8 (control)",
        );
        assert!(
            rgba_ok,
            "control case RGBA8 must be renderable — harness is broken"
        );

        // Candidates for a 1-byte-per-pixel indexed buffer.
        probe_format(
            gl,
            glow::LUMINANCE as i32,
            glow::LUMINANCE,
            glow::UNSIGNED_BYTE,
            "LUMINANCE (femtovg Gray8)",
        );
        probe_format(
            gl,
            glow::ALPHA as i32,
            glow::ALPHA,
            glow::UNSIGNED_BYTE,
            "ALPHA",
        );

        probe_format(
            gl,
            GL_RED_EXT as i32,
            GL_RED_EXT,
            glow::UNSIGNED_BYTE,
            "RED_EXT (GL_EXT_texture_rg)",
        );
    }
}

/// Attach a `4x4` texture of this format as colour attachment 0 of a fresh
/// FBO and report completeness.
#[expect(
    clippy::cast_possible_wrap,
    reason = "small fixed GL enum values fit i32"
)]
unsafe fn probe_format(
    gl: &glow::Context,
    internal_format: i32,
    format: u32,
    ty: u32,
    label: &str,
) -> bool {
    unsafe {
        let texture = gl.create_texture().expect("BUG: create_texture failed");
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            internal_format,
            4,
            4,
            0,
            format,
            ty,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);

        let fbo = gl
            .create_framebuffer()
            .expect("BUG: create_framebuffer failed");
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.delete_framebuffer(fbo);
        gl.delete_texture(texture);

        let complete = status == glow::FRAMEBUFFER_COMPLETE;
        eprintln!("{label:<32} complete={complete:<5} status=0x{status:04X}");
        complete
    }
}
