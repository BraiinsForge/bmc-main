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

//! Headless GLES 2.0 + femtovg canvas test harness.
//!
//! Mirrors the `capture` binary's EGL + Mesa llvmpipe surfaceless path so
//! tests across `gpu::*` registries can register/evict assets against real
//! GL handles without a window or display server. Linux-only.

use std::any::Any;
use std::ffi::CString;
use std::num::NonZeroU32;

use anyhow::{Context as _, Result, anyhow};
use femtovg::Canvas;
use femtovg::renderer::OpenGl;
use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::display::{Display, GetGlDisplay};
use glutin::prelude::*;
use glutin::surface::{PbufferSurface, SurfaceAttributesBuilder};

/// Real headless GL context + the resources keeping it alive.
///
/// The keepalive must outlive every call into `gl`; dropping `GlHarness`
/// releases the EGL surface / context after any femtovg `Canvas` built on
/// top has already been dropped (canvases store no owning reference to the
/// underlying GL state, so caller ordering matters).
pub struct GlHarness {
    pub gl: glow::Context,
    /// Boxed (surface, context) — opaque to keep this type small and
    /// platform-neutral at the use site.
    _keepalive: Box<dyn Any>,
    /// Function pointer loader, kept around so `OpenGl::new_from_function`
    /// can be invoked lazily by `with_canvas`.
    proc_addr: ProcAddrFn,
}

type ProcAddrFn = Box<dyn Fn(&str) -> *const std::ffi::c_void>;

impl std::fmt::Debug for GlHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlHarness").finish_non_exhaustive()
    }
}

impl GlHarness {
    /// Boot a fresh headless GLES 2.0 context via EGL surfaceless + Mesa
    /// llvmpipe. Each test gets its own context; tests run on separate
    /// threads under nextest by default and EGL state is per-thread.
    pub fn new() -> Result<Self> {
        let devices: Vec<_> = glutin::api::egl::device::Device::query_devices()
            .context("EGL device enumeration not supported")?
            .collect();
        let device = devices
            .iter()
            .find(|d| d.extensions().contains("EGL_MESA_device_software"))
            .or_else(|| devices.first())
            .ok_or_else(|| anyhow!("no EGL devices found"))?;
        let egl_display = unsafe { glutin::api::egl::display::Display::with_device(device, None) }
            .context("failed to create EGL display")?;
        let display = Display::Egl(egl_display);

        let template = ConfigTemplateBuilder::new()
            .with_surface_type(ConfigSurfaceTypes::PBUFFER)
            .build();
        let gl_config = unsafe { display.find_configs(template) }
            .map_err(|e| anyhow!("find_configs failed: {e}"))?
            .next()
            .ok_or_else(|| anyhow!("no GL configs"))?;
        let gl_display = gl_config.display();

        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
            .build(None);
        let gl_context = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .context("create_context failed")?
        };

        let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
            NonZeroU32::new(1).expect("BUG: const 1 is non-zero"),
            NonZeroU32::new(1).expect("BUG: const 1 is non-zero"),
        );
        let surface = unsafe {
            gl_display
                .create_pbuffer_surface(&gl_config, &surface_attrs)
                .context("create_pbuffer_surface failed")?
        };
        let gl_context = gl_context
            .make_current(&surface)
            .context("make_current failed")?;

        let proc_addr: ProcAddrFn = {
            let gl_display = gl_display.clone();
            Box::new(move |s: &str| {
                gl_display.get_proc_address(&CString::new(s).unwrap_or_default())
            })
        };
        let gl = unsafe { glow::Context::from_loader_function(|s| proc_addr(s)) };

        Ok(Self {
            gl,
            _keepalive: Box::new((surface, gl_context)),
            proc_addr,
        })
    }

    /// Build a femtovg `Canvas` over the harness's GL context. The canvas
    /// borrows nothing from `self`; ordering with the harness's drop must
    /// be enforced by the caller (drop the canvas first).
    pub(crate) fn build_canvas(&self) -> Result<Canvas<OpenGl>> {
        let renderer = unsafe { OpenGl::new_from_function(|s| (self.proc_addr)(s)) }
            .map_err(|e| anyhow!("OpenGl renderer init failed: {e}"))?;
        Canvas::new(renderer).map_err(|e| anyhow!("Canvas::new failed: {e}"))
    }

    /// FFI proc-addr loader for building higher-level renderers
    /// (`FemtoVgRenderer`) on top of this harness's GL context. Function
    /// pointers are baked into the renderer during its `new()`, so the
    /// returned closure only needs to outlive that call.
    pub fn load_fn(&self) -> impl FnMut(&str) -> *const std::ffi::c_void + use<'_> {
        |s: &str| (self.proc_addr)(s)
    }
}

/// Allocate a buffer and bind it once so `gl.is_buffer` reports `true`.
/// Per the GLES 2.0 spec, names returned by `glGenBuffers` only become
/// "real" buffers (queryable by `glIsBuffer`) once first bound.
pub(crate) fn create_real_buffer(gl: &glow::Context, target: u32) -> glow::Buffer {
    use glow::HasContext as _;
    let buf = unsafe { gl.create_buffer() }.expect("BUG: create_buffer failed");
    unsafe {
        gl.bind_buffer(target, Some(buf));
        gl.bind_buffer(target, None);
    }
    buf
}

/// Allocate a texture and bind it once — same reason as `create_real_buffer`.
pub(crate) fn create_real_texture(gl: &glow::Context) -> glow::Texture {
    use glow::HasContext as _;
    let tex = unsafe { gl.create_texture() }.expect("BUG: create_texture failed");
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    tex
}

/// Offscreen render target for pixel-level tests: a colour texture plus the
/// stencil attachment FemtoVG needs for concave fills. Returns the framebuffer
/// and its raw GL name, which is what [`crate::gpu::FemtoVgRenderer::new`] takes
/// as its screen target.
///
/// Like the `create_real_*` helpers above, the returned objects are not owned by
/// anything — the texture and renderbuffer live until the harness's context goes
/// away with the test. Nothing here is meant to outlive one `GlHarness`.
#[expect(clippy::cast_possible_wrap)]
pub fn create_readback_fbo(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> (glow::Framebuffer, u32) {
    use glow::HasContext as _;
    unsafe {
        let texture = gl.create_texture().expect("BUG: create_texture failed");
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
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
        // FemtoVG needs a stencil attachment for concave fills.
        let rbo = gl
            .create_renderbuffer()
            .expect("BUG: create_renderbuffer failed");
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            glow::DEPTH24_STENCIL8,
            width as i32,
            height as i32,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_STENCIL_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(rbo),
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        assert_eq!(
            status,
            glow::FRAMEBUFFER_COMPLETE,
            "FBO incomplete: {status:#x}"
        );
        (fbo, fbo.0.get())
    }
}

/// Read `fbo` back as row-major RGBA with row 0 = top. `glReadPixels` uses a
/// bottom-left origin, so the rows are flipped here and callers can index by
/// screen row directly.
///
/// Binds the plain `FRAMEBUFFER` target, like [`create_readback_fbo`]. There is
/// no blit here, so a `glReadPixels` off `FRAMEBUFFER` reads identically to one
/// off `READ_FRAMEBUFFER` — and `READ_FRAMEBUFFER` is core only from GLES 3.0,
/// which is more than [`GlHarness::new`] asks the driver for.
#[expect(clippy::cast_possible_wrap)]
pub fn read_pixels_top_down(
    gl: &glow::Context,
    fbo: glow::Framebuffer,
    width: u32,
    height: u32,
) -> Vec<[u8; 4]> {
    use glow::HasContext as _;
    let (w, h) = (width as usize, height as usize);
    let mut raw = vec![0_u8; w * h * 4];
    unsafe {
        assert_eq!(
            gl.get_error(),
            glow::NO_ERROR,
            "BUG: GL error predates the readback, so the render under test already failed",
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.read_pixels(
            0,
            0,
            width as i32,
            height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut raw)),
        );
        // A rejected bind is a silent no-op that reads whatever was bound
        // before, which would pass the caller's assertions on wrong pixels.
        assert_eq!(
            gl.get_error(),
            glow::NO_ERROR,
            "BUG: framebuffer bind or readback was rejected",
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    }
    let mut out = vec![[0_u8; 4]; w * h];
    for y in 0..h {
        let src = (h - 1 - y) * w;
        for x in 0..w {
            let i = (src + x) * 4;
            out[y * w + x] = [raw[i], raw[i + 1], raw[i + 2], raw[i + 3]];
        }
    }
    out
}
