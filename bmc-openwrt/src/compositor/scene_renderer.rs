// Copyright (C) 2025  Braiins Systems s.r.o.

//! Scene renderer for compositing multiple widgets.

use anyhow::{Context, Result};
use smithay::{
    backend::renderer::{Bind, Color32F, Frame, ImportDma, ImportMemWl, Renderer, Texture},
    reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    utils::{Buffer as BufferCoord, Rectangle, Size, Transform},
    wayland::dmabuf::get_dmabuf,
};

use super::render::{BufferPool, DrmOutput, EglContext};
use super::widget_tracker::WidgetTracker;

const BACKGROUND_COLOR: Color32F = Color32F::new(0.05, 0.05, 0.1, 1.0);

pub struct SceneRenderer {
    egl: EglContext,
    output: DrmOutput,
    buffers: BufferPool,
}

impl SceneRenderer {
    pub fn new(egl: EglContext, output: DrmOutput) -> Self {
        let (width, height) = (output.width(), output.height());
        Self {
            egl,
            output,
            buffers: BufferPool::new(width, height),
        }
    }

    pub fn output(&self) -> &DrmOutput {
        &self.output
    }

    pub fn output_mut(&mut self) -> &mut DrmOutput {
        &mut self.output
    }

    pub fn logical_size(&self) -> (u32, u32) {
        self.output.logical_size()
    }

    pub fn render_scene(&mut self, widgets: &WidgetTracker, buffers: &[(WlBuffer, bmc::compositor::InstanceId)]) -> Result<()> {
        if self.output.is_flip_pending() {
            return Ok(());
        }

        let buffer = self.buffers.back_buffer(&self.output)?;
        let mut dmabuf = buffer.dmabuf.clone();
        let fb = buffer.fb;

        let scene = widgets.active_scene();

        // Import all textures BEFORE creating the frame (renderer borrow constraint)
        let renderer = self.egl.renderer();
        let imported: Vec<_> = buffers
            .iter()
            .filter_map(|(client_buffer, instance_id)| {
                let placement = scene.widgets.iter().find(|w| &w.instance_id == instance_id && w.visible)?;

                let texture = if let Ok(dmabuf) = get_dmabuf(client_buffer) {
                    renderer.import_dmabuf(dmabuf, None).ok()
                } else {
                    renderer.import_shm_buffer(client_buffer, None, &[]).ok()
                };

                let texture = texture.or_else(|| {
                    tracing::warn!("Failed to import buffer for widget {}", instance_id);
                    None
                })?;

                Some((texture, placement.clone()))
            })
            .collect();

        let mut framebuffer = renderer.bind(&mut dmabuf).context("Failed to bind render target")?;

        #[expect(clippy::cast_possible_wrap)]
        let output_size = Size::from((self.output.width() as i32, self.output.height() as i32));

        let mut frame = renderer
            .render(&mut framebuffer, output_size, Transform::Normal)
            .context("Failed to begin frame")?;

        frame
            .clear(BACKGROUND_COLOR, &[Rectangle::from_size(output_size)])
            .context("Failed to clear")?;

        for (texture, placement) in &imported {
            let tex_size = texture.size();
            let src: Rectangle<f64, BufferCoord> = Rectangle::from_loc_and_size(
                (0.0, 0.0),
                (f64::from(tex_size.w), f64::from(tex_size.h)),
            );

            #[expect(clippy::cast_possible_wrap)]
            let dst = Rectangle::from_loc_and_size(
                (placement.position.y as i32, placement.position.x as i32),
                (tex_size.h, tex_size.w),
            );

            tracing::debug!(
                "Rendering widget {} at ({}, {}) size {}x{}",
                placement.instance_id,
                placement.position.x,
                placement.position.y,
                placement.size.width,
                placement.size.height
            );

            if let Err(e) = frame.render_texture_from_to(
                texture,
                src,
                dst,
                &[dst],
                &[],
                Transform::_270,
                1.0,
                None,
                &[],
            ) {
                tracing::warn!("Failed to render widget {}: {:?}", placement.instance_id, e);
            }
        }

        let _sync = frame.finish().context("Failed to finish frame")?;
        drop(framebuffer);

        self.egl.finish_rendering()?;
        self.output.page_flip(fb)?;
        self.buffers.swap();

        Ok(())
    }
}
