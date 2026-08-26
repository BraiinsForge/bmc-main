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

//! Shared helpers for `bmc-wasm-runtime` integration tests.
//!
//! Cargo treats files under `tests/<dir>/mod.rs` as non-test helpers, unlike
//! a top-level `tests/<file>.rs` which Cargo would compile as its own test
//! binary. Each integration test that needs a helper here declares
//! `mod common;` at the top.

/// Rectangular logical display matching a test viewport. The runtime has no
/// display default by design, so tests that do not exercise geometry supply
/// this explicit fixture rather than relying on a fabricated default.
#[must_use]
pub fn test_display(width: u32, height: u32) -> bmc_wasm_runtime::RuntimeDisplayInfo {
    bmc_wasm_runtime::RuntimeDisplayInfo {
        width,
        height,
        shape: bmc_wasm_protocol::DisplayShape::Rectangular,
        dpi: 1,
    }
}

pub mod headless_egl {
    use anyhow::{Context, Result};
    use glutin::api::egl::device::Device;
    use glutin::api::egl::display::Display as EglDisplay;
    use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
    use glutin::context::{ContextApi, ContextAttributesBuilder};
    use glutin::display::{Display, GetGlDisplay};
    use glutin::prelude::*;
    use glutin::surface::{PbufferSurface, Surface, SurfaceAttributesBuilder};
    use std::ffi::{CString, c_void};
    use std::num::NonZeroU32;

    /// Headless GL context tied to a pbuffer and its drop-ordered resources.
    pub struct HeadlessGl {
        pub display: Display,
        pub fbo_id: u32,
        resources: GlResources,
    }

    /// Ownership root for the GL resources `HeadlessGl` keeps alive. Fields drop in declaration
    /// order, which matters: the pbuffer surface must drop before the context that made it
    /// current (glutin enforces this at runtime), and texture / context-wrapped glow handles
    /// release their underlying GL state once the context goes.
    ///
    /// The GL handle also supports framebuffer inspection through `HeadlessGl`'s `AsRef` impl.
    #[expect(dead_code, reason = "ownership markers — see struct doc")]
    struct GlResources {
        surface: Surface<PbufferSurface>,
        context: glutin::context::PossiblyCurrentContext,
        gl: glow::Context,
        texture: glow::Texture,
    }

    impl HeadlessGl {
        /// Return a `get_proc_address` closure suitable for handing to `WasmWidgetRuntime::new`.
        pub fn proc_address(&self) -> impl FnMut(&str) -> *const c_void + '_ {
            let display = self.display.clone();
            move |s: &str| display.get_proc_address(&CString::new(s).unwrap_or_default())
        }
    }

    impl AsRef<glow::Context> for HeadlessGl {
        fn as_ref(&self) -> &glow::Context {
            &self.resources.gl
        }
    }

    /// Force Mesa onto its software (llvmpipe) path before EGL/libdrm see any context.
    /// Mirrors the `ci` Nix build profile so local and CI runs probe the same backend;
    /// also sidesteps libdrm's `pci id for fd N:` stderr noise on dev boxes with a real GPU.
    ///
    /// `std::sync::Once` is fine because EGL is only touched through this module —
    /// nothing else in the test binary loads it earlier.
    fn force_software_egl() {
        static SET_ENV: std::sync::Once = std::sync::Once::new();
        SET_ENV.call_once(|| {
            // SAFETY: single-threaded test init; env-var mutation is safe before any other
            // thread or dlopen'd library reads these. Re-entry is guarded by `Once`.
            unsafe {
                std::env::set_var("EGL_PLATFORM", "surfaceless");
                std::env::set_var("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe");
                std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
            }
        });
    }

    /// Set by the Nix `ci` profile, the one that supplies Mesa.
    /// A failed init there means a broken profile, not a machine
    /// without a GPU, and a skip would pass — hiding exactly that.
    const REQUIRE_EGL: &str = "BMC_REQUIRE_HEADLESS_EGL";

    /// Try to build a surfaceless EGL display + GL context at `w × h`.
    /// Returns `None` and logs a skip-reason when EGL initialization fails
    /// — the common case is running `cargo test` outside the Nix `ci` profile,
    /// which is fine for local dev but means these tests are no-ops there.
    ///
    /// A skip is a *pass*, which is why `.config/nextest.toml` surfaces it
    /// and [`REQUIRE_EGL`] makes it fatal where Mesa is supplied.
    pub fn try_init(w: u32, h: u32) -> Option<HeadlessGl> {
        match init(w, h) {
            Ok(state) => Some(state),
            Err(err) => {
                assert!(
                    std::env::var_os(REQUIRE_EGL).is_none(),
                    "{REQUIRE_EGL} is set, so skipping here would report a green run \
                     for tests that never executed: {err:#}"
                );
                eprintln!(
                    "skipping integration test: headless EGL init failed — {err:#}\n\
                     (this test requires the `ci` build profile from nix/profiles.nix, \
                     which supplies Mesa + llvmpipe + surfaceless EGL)"
                );
                None
            }
        }
    }

    fn init(w: u32, h: u32) -> Result<HeadlessGl> {
        force_software_egl();
        let devices: Vec<_> = Device::query_devices()
            .context("EGL device enumeration not supported (missing EGL_EXT_device_query)")?
            .collect();
        let device = devices
            .iter()
            .find(|d| d.extensions().contains("EGL_MESA_device_software"))
            .or_else(|| devices.first())
            .context("no EGL devices found — is libEGL.so.1 resolvable?")?;
        let egl_display = unsafe { EglDisplay::with_device(device, None) }
            .context("failed to create EGL display from device")?;
        let display = Display::Egl(egl_display);

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_stencil_size(8)
            .with_surface_type(ConfigSurfaceTypes::PBUFFER)
            .build();
        let gl_config = unsafe { display.find_configs(template) }
            .map_err(|e| anyhow::anyhow!("failed to find GL configs: {e}"))?
            .next()
            .context("no suitable GL config found")?;

        let gl_display = gl_config.display();
        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(glutin::context::Version::new(2, 0))))
            .build(None);
        let gl_context = unsafe { gl_display.create_context(&gl_config, &context_attrs) }
            .context("failed to create GL context")?;

        let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
            NonZeroU32::new(w).expect("BUG: zero width"),
            NonZeroU32::new(h).expect("BUG: zero height"),
        );
        let surface = unsafe { gl_display.create_pbuffer_surface(&gl_config, &surface_attrs) }
            .context("failed to create pbuffer surface")?;
        let gl_context = gl_context
            .make_current(&surface)
            .context("failed to make GL context current")?;

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                gl_display.get_proc_address(&CString::new(s).unwrap_or_default())
            })
        };

        let (fbo, texture) = create_fbo(&gl, w, h)?;
        let fbo_id = fbo.0.get();

        Ok(HeadlessGl {
            display,
            fbo_id,
            resources: GlResources {
                surface,
                context: gl_context,
                gl,
                texture,
            },
        })
    }

    fn create_fbo(
        gl: &glow::Context,
        w: u32,
        h: u32,
    ) -> Result<(glow::Framebuffer, glow::Texture)> {
        use glow::HasContext;
        // GL constants and dimensions arrive as `u32` from glow but the GL API takes `GLint`
        // (`i32`). The values in play (texture size enums, RGBA, LINEAR) are all well under
        // `i32::MAX`, so the cast is a wire-format pass-through, not a numeric narrowing.
        #[expect(
            clippy::cast_possible_wrap,
            reason = "glow constants + texture dimensions are bounded well below i32::MAX; \
                      these are GL-API u32→GLint pass-throughs"
        )]
        unsafe {
            let texture = gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("create_texture: {e}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                w as i32,
                h as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );

            let fbo = gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("create_framebuffer: {e}"))?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("FBO incomplete: {status:#x}");
            }
            Ok((fbo, texture))
        }
    }
}
