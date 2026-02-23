// Copyright (C) 2025  Braiins Systems s.r.o.

//! Scene renderer for compositing multiple widgets.

use std::collections::HashMap;

use anyhow::{Context, Result};
use smithay::{
    backend::renderer::gles::GlesTexture,
    backend::renderer::{Bind, Color32F, Frame, ImportDma, ImportMemWl, Renderer, Texture},
    reexports::wayland_server::{Resource, backend::ObjectId, protocol::wl_buffer::WlBuffer},
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
    /// Texture cache: maps WlBuffer ObjectId to cached GlesTexture
    texture_cache: HashMap<ObjectId, GlesTexture>,
    #[cfg(feature = "profiling")]
    bind_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    clear_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    compose_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    finish_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    flip_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    render_every: ii_stopwatch::Every,
}

impl SceneRenderer {
    pub fn new(egl: EglContext, output: DrmOutput) -> Self {
        let (width, height) = (output.width(), output.height());
        let (logical_w, logical_h) = output.logical_size();
        tracing::info!(
            "SceneRenderer: physical {}x{}, logical {}x{}",
            width,
            height,
            logical_w,
            logical_h
        );
        Self {
            egl,
            output,
            buffers: BufferPool::new(width, height),
            texture_cache: HashMap::new(),
            #[cfg(feature = "profiling")]
            bind_w: ii_stopwatch::StopWatch::default(),
            #[cfg(feature = "profiling")]
            clear_w: ii_stopwatch::StopWatch::default(),
            #[cfg(feature = "profiling")]
            compose_w: ii_stopwatch::StopWatch::default(),
            #[cfg(feature = "profiling")]
            finish_w: ii_stopwatch::StopWatch::default(),
            #[cfg(feature = "profiling")]
            flip_w: ii_stopwatch::StopWatch::default(),
            #[cfg(feature = "profiling")]
            render_every: ii_stopwatch::Every::new(std::time::Duration::from_secs(5)),
        }
    }

    /// Invalidate cached textures for the given buffer IDs.
    /// Call this when buffers are destroyed or replaced.
    pub fn invalidate_textures(&mut self, buffer_ids: &[ObjectId]) {
        for id in buffer_ids {
            if self.texture_cache.remove(id).is_some() {
                tracing::debug!("Invalidated cached texture for buffer {:?}", id);
            }
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

    #[expect(clippy::too_many_lines)]
    pub fn render_scene(
        &mut self,
        widgets: &WidgetTracker,
        buffers: &[(WlBuffer, bmc::compositor::InstanceId)],
    ) -> Result<()> {
        if self.output.is_flip_pending() {
            return Ok(());
        }

        let buffer = self.buffers.back_buffer(&self.output)?;
        let mut dmabuf = buffer.dmabuf.clone();
        let fb = buffer.fb;

        let scene = widgets.active_scene();

        // Collect visible widgets as (buffer_id, placement)
        let to_render: Vec<_> = buffers
            .iter()
            .filter_map(|(client_buffer, instance_id)| {
                let placement = scene
                    .widgets
                    .iter()
                    .find(|w| &w.instance_id == instance_id && w.visible)?;
                Some((client_buffer.id(), placement.clone()))
            })
            .collect();

        // Import textures — cache DMA-BUF, always reimport SHM.
        // SHM clients (Slint) repaint into the same WlBuffer without
        // destroying it, so the ObjectId stays constant while the pixel
        // data changes. Reusing a stale SHM texture would freeze the
        // widget visuals after the first frame.
        let renderer = self.egl.renderer();
        for (client_buffer, _instance_id) in buffers {
            let buffer_id = client_buffer.id();
            if let Ok(dmabuf) = get_dmabuf(client_buffer) {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    self.texture_cache.entry(buffer_id)
                    && let Ok(texture) = renderer.import_dmabuf(dmabuf, None)
                {
                    entry.insert(texture);
                }
            } else if let Ok(texture) = renderer.import_shm_buffer(client_buffer, None, &[]) {
                self.texture_cache.insert(buffer_id, texture);
            }
        }

        ii_stopwatch::stopwatch_start!(self.bind_w);
        let mut framebuffer = renderer
            .bind(&mut dmabuf)
            .context("Failed to bind render target")?;
        ii_stopwatch::stopwatch_stop!(self.bind_w);

        #[expect(clippy::cast_possible_wrap)]
        let output_size = Size::from((self.output.width() as i32, self.output.height() as i32));

        let mut frame = renderer
            .render(&mut framebuffer, output_size, Transform::Normal)
            .context("Failed to begin frame")?;

        ii_stopwatch::stopwatch_start!(self.clear_w);
        frame
            .clear(BACKGROUND_COLOR, &[Rectangle::from_size(output_size)])
            .context("Failed to clear")?;
        ii_stopwatch::stopwatch_stop!(self.clear_w);

        ii_stopwatch::stopwatch_start!(self.compose_w);
        for (buffer_id, placement) in &to_render {
            let Some(texture) = self.texture_cache.get(buffer_id) else {
                tracing::warn!("No cached texture for buffer {:?}", buffer_id);
                continue;
            };

            let tex_size = texture.size();
            let src: Rectangle<f64, BufferCoord> = Rectangle::from_loc_and_size(
                (0.0, 0.0),
                (f64::from(tex_size.w), f64::from(tex_size.h)),
            );

            #[expect(clippy::cast_possible_wrap)]
            let logical_x = placement.position.x as i32;
            #[expect(clippy::cast_possible_wrap)]
            let logical_y = placement.position.y as i32;

            // Physical buffer: 480x1280 (WxH) - portrait orientation
            // Logical space: 1280x480 (WxH) - landscape after rotation
            // Widget texture is logical (e.g., 638x480)
            // After Transform::_270, dst size is (tex_h, tex_w)
            let phys_w = tex_size.h; // 480 (logical height -> physical width)
            let phys_h = tex_size.w; // 638 (logical width -> physical height)

            // Coordinate mapping for 90° CW rotation (Transform::_270):
            // - Physical Y=0 corresponds to RIGHT side of landscape display
            // - Physical Y=max corresponds to LEFT side of landscape display
            // So we invert: logical_x=0 (left) -> high physical_y, logical_x=max (right) -> low physical_y
            #[expect(clippy::cast_possible_wrap)]
            let output_height = self.output.height() as i32;
            let physical_x = logical_y;
            let physical_y = output_height - logical_x - phys_h;

            let dst = Rectangle::from_loc_and_size((physical_x, physical_y), (phys_w, phys_h));

            // Use per-widget damage region for efficient partial updates
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
        ii_stopwatch::stopwatch_stop!(self.compose_w);

        ii_stopwatch::stopwatch_start!(self.finish_w);
        let _sync = frame.finish().context("Failed to finish frame")?;
        drop(framebuffer);
        self.egl.finish_rendering()?;
        ii_stopwatch::stopwatch_stop!(self.finish_w);

        ii_stopwatch::stopwatch_start!(self.flip_w);
        self.output.page_flip(fb)?;
        ii_stopwatch::stopwatch_stop!(self.flip_w);

        self.buffers.swap();

        #[cfg(feature = "profiling")]
        if ii_stopwatch::every_expired!(self.render_every) {
            tracing::info!(
                "render_scene: bind={} clear={} compose={} finish={} flip={}",
                self.bind_w,
                self.clear_w,
                self.compose_w,
                self.finish_w,
                self.flip_w
            );
            ii_stopwatch::stopwatch_reset!(self.bind_w);
            ii_stopwatch::stopwatch_reset!(self.clear_w);
            ii_stopwatch::stopwatch_reset!(self.compose_w);
            ii_stopwatch::stopwatch_reset!(self.finish_w);
            ii_stopwatch::stopwatch_reset!(self.flip_w);
        }

        Ok(())
    }
}
