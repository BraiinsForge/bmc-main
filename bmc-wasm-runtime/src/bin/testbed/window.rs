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

//! The window and its GL context.
//!
//! Adapted from `egui_glow`'s `pure_glow` example, which in turn lifted it
//! from eframe — so this is the same bootstrap eframe ran for us before,
//! with the pieces the testbed needs to reach now exposed.

use std::num::NonZeroU32;

use anyhow::{Context as _, Result};
use glutin::context::NotCurrentGlContext as _;
use glutin::display::{GetGlDisplay as _, GlDisplay as _};
use glutin::prelude::GlSurface as _;
use winit::raw_window_handle::HasWindowHandle as _;

/// Smallest window the chrome stays usable in — the right sidebar alone
/// claims 320 px, and below this the preview area stops being worth showing.
const MIN_INNER_SIZE: winit::dpi::LogicalSize<f64> = winit::dpi::LogicalSize {
    width: 1024.0,
    height: 640.0,
};

pub(crate) struct GlWindow {
    gl_context: glutin::context::PossiblyCurrentContext,
    gl_surface: glutin::surface::Surface<glutin::surface::WindowSurface>,
    window: winit::window::Window,
}

impl GlWindow {
    /// Build the window, pick a config, and make the context current.
    ///
    /// Starts hidden and is shown after the first paint, so the operator never
    /// sees an unpainted frame.
    pub(crate) fn new(
        event_loop: &winit::event_loop::ActiveEventLoop,
        inner_size: winit::dpi::LogicalSize<f64>,
        title: &str,
    ) -> Result<(Self, egui_glow::glow::Context, super::paint::GlProcAddress)> {
        let attributes = winit::window::WindowAttributes::default()
            .with_resizable(true)
            .with_inner_size(inner_size)
            .with_min_inner_size(MIN_INNER_SIZE)
            .with_title(title)
            .with_visible(false);

        // Depth and stencil are zero here: widget FBOs carry their own
        // attachments, and egui itself needs neither.
        let config_template = glutin::config::ConfigTemplateBuilder::new()
            .prefer_hardware_accelerated(None)
            .with_depth_size(0)
            .with_stencil_size(0)
            .with_transparency(false);

        // `FallbackEgl` matches eframe: GLX first, EGL when it is unavailable.
        let (mut window, gl_config) = glutin_winit::DisplayBuilder::new()
            .with_preference(glutin_winit::ApiPreference::FallbackEgl)
            .with_window_attributes(Some(attributes.clone()))
            .build(event_loop, config_template, |mut configs| {
                configs
                    .next()
                    .expect("BUG: glutin offered no matching GL config")
            })
            .map_err(|e| anyhow::anyhow!("GL config: {e}"))?;

        let gl_display = gl_config.display();
        let raw_window_handle = window
            .as_ref()
            .map(|w| w.window_handle().map(|h| h.as_raw()))
            .transpose()
            .context("window handle")?;

        let context_attributes =
            glutin::context::ContextAttributesBuilder::new().build(raw_window_handle);
        let gles_attributes = glutin::context::ContextAttributesBuilder::new()
            .with_context_api(glutin::context::ContextApi::Gles(None))
            .build(raw_window_handle);

        // SAFETY: the handle above comes from a live window this call keeps alive.
        let not_current = unsafe {
            gl_display
                .create_context(&gl_config, &context_attributes)
                .or_else(|_| gl_display.create_context(&gl_config, &gles_attributes))
                .map_err(|e| anyhow::anyhow!("GL context: {e}"))?
        };

        // `DisplayBuilder` only builds the window on platforms where the config
        // search needs one; elsewhere it lands here.
        let window = match window.take() {
            Some(window) => window,
            None => glutin_winit::finalize_window(event_loop, attributes, &gl_config)
                .map_err(|e| anyhow::anyhow!("window: {e}"))?,
        };

        let (width, height): (u32, u32) = window.inner_size().into();
        let surface_attributes =
            glutin::surface::SurfaceAttributesBuilder::<glutin::surface::WindowSurface>::new()
                .build(
                    window.window_handle().context("window handle")?.as_raw(),
                    NonZeroU32::new(width).unwrap_or(NonZeroU32::MIN),
                    NonZeroU32::new(height).unwrap_or(NonZeroU32::MIN),
                );

        // SAFETY: same live window as above, and the surface is dropped with it.
        let gl_surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attributes)
                .map_err(|e| anyhow::anyhow!("GL surface: {e}"))?
        };

        let gl_context = not_current
            .make_current(&gl_surface)
            .map_err(|e| anyhow::anyhow!("make current: {e}"))?;

        gl_surface
            .set_swap_interval(
                &gl_context,
                glutin::surface::SwapInterval::Wait(NonZeroU32::MIN),
            )
            .map_err(|e| anyhow::anyhow!("vsync: {e}"))?;

        // SAFETY: the loader is valid while `gl_display` lives,
        // which the returned context owns transitively through its surface.
        let gl = unsafe {
            egui_glow::glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s))
        };

        // Handed out rather than kept, because each view's `FemtoVgRenderer`
        // builds its own glow context from this same loader, on a glow one
        // major behind the one egui uses.
        let proc_address: super::paint::GlProcAddress =
            std::sync::Arc::new(move |name: &std::ffi::CStr| gl_display.get_proc_address(name));

        Ok((
            Self {
                gl_context,
                gl_surface,
                window,
            },
            gl,
            proc_address,
        ))
    }

    pub(crate) fn window(&self) -> &winit::window::Window {
        &self.window
    }

    pub(crate) fn show(&self) {
        self.window.set_visible(true);
    }

    pub(crate) fn resize(&self, size: winit::dpi::PhysicalSize<u32>) {
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        self.gl_surface.resize(&self.gl_context, width, height);
    }

    pub(crate) fn swap_buffers(&self) -> Result<()> {
        self.gl_surface
            .swap_buffers(&self.gl_context)
            .map_err(|e| anyhow::anyhow!("swap: {e}"))
    }
}
