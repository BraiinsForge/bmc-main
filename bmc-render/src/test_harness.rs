// Copyright (C) 2026  Braiins Systems s.r.o.

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
pub(crate) struct GlHarness {
    pub gl: glow::Context,
    /// Boxed (surface, context) — opaque to keep this type small and
    /// platform-neutral at the use site.
    _keepalive: Box<dyn Any>,
    /// Function pointer loader, kept around so `OpenGl::new_from_function`
    /// can be invoked lazily by `with_canvas`.
    proc_addr: ProcAddrFn,
}

type ProcAddrFn = Box<dyn Fn(&str) -> *const std::ffi::c_void>;

impl GlHarness {
    /// Boot a fresh headless GLES 2.0 context via EGL surfaceless + Mesa
    /// llvmpipe. Each test gets its own context; tests run on separate
    /// threads under nextest by default and EGL state is per-thread.
    pub(crate) fn new() -> Result<Self> {
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
    pub(crate) fn load_fn(&self) -> impl FnMut(&str) -> *const std::ffi::c_void + use<'_> {
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
