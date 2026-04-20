// Copyright (C) 2025  Braiins Systems s.r.o.

//! Scene renderer for compositing multiple widgets.

use std::collections::HashMap;

use anyhow::{Context, Result};
use smithay::{
    backend::renderer::{
        Bind, Color32F, Frame as RendererFrame, ImportDma, ImportMemWl, Renderer, Texture,
        gles::{GlesRenderer, GlesTexture, ffi},
    },
    reexports::wayland_server::{Resource, backend::ObjectId, protocol::wl_buffer::WlBuffer},
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform},
    wayland::{
        dmabuf::get_dmabuf,
        image_copy_capture::{self, CaptureFailureReason},
        shm::with_buffer_contents_mut,
    },
};

use super::render::{BufferPool, DrmOutput, EglContext};
use super::state::OutputDamage;
use super::widget_tracker::WidgetTracker;

const BACKGROUND_COLOR: Color32F = Color32F::new(0.05, 0.05, 0.1, 1.0);
/// Pixel overlap between adjacent scenes during transitions.
/// Compensates for GL texture edge sampling artifacts under Transform::_270
/// on the Vivante GC400. The neighbor scene renders after the active scene,
/// painting over the seam.
const SCENE_SEAM_OVERLAP: i32 = 4;

pub struct SceneRenderer {
    egl: EglContext,
    output: DrmOutput,
    buffers: BufferPool,
    /// Texture cache: maps WlBuffer ObjectId to cached GlesTexture
    texture_cache: HashMap<ObjectId, GlesTexture>,
    /// Cached pixels from the last inline capture readback.
    /// Served to capture clients between renders (avoids re-rendering
    /// just to observe the same frame).
    capture_cache: CaptureCache,
    #[cfg(feature = "profiling")]
    bind_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    compose_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    finish_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    flip_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    render_every: ii_stopwatch::Every,
}

struct CaptureCache {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    flipped: bool,
    valid: bool,
}

impl CaptureCache {
    fn empty() -> Self {
        Self {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            flipped: false,
            valid: false,
        }
    }
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
            capture_cache: CaptureCache::empty(),
            #[cfg(feature = "profiling")]
            bind_w: ii_stopwatch::StopWatch::default(),
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

    /// Import widget textures that were newly committed since the last render.
    ///
    /// Only reimports buffers whose ObjectId appears in `dirty_buffers`
    /// (populated by the commit handler). Unchanged buffers keep their
    /// cached texture. This avoids redundant EGLImage creation on virgl
    /// which can produce subtly different host-side copies and cause flicker.
    ///
    /// SHM buffers are always reimported because clients repaint into the
    /// same WlBuffer without destroying it.
    fn import_textures(
        &mut self,
        buffers: &[(WlBuffer, bmc::compositor::InstanceId)],
        dirty: &[ObjectId],
    ) {
        let renderer = self.egl.renderer();
        for (client_buffer, _instance_id) in buffers {
            let buffer_id = client_buffer.id();
            if let Ok(dmabuf) = get_dmabuf(client_buffer) {
                // DMA-BUF: only reimport if newly committed (dirty)
                if dirty.contains(&buffer_id)
                    && let Ok(texture) = renderer.import_dmabuf(dmabuf, None)
                {
                    self.texture_cache.insert(buffer_id, texture);
                }
            } else if let Ok(texture) = renderer.import_shm_buffer(client_buffer, None, &[]) {
                self.texture_cache.insert(buffer_id, texture);
            }
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "render hot-path with stopwatch instrumentation; splitting hurts readability"
    )]
    pub fn render_scene(
        &mut self,
        widgets: &WidgetTracker,
        buffers: &[(WlBuffer, bmc::compositor::InstanceId)],
        dirty: &[ObjectId],
        capture_frames: Vec<image_copy_capture::Frame>,
        capture_active: bool,
        output_damage: &OutputDamage,
    ) -> Result<(bool, Vec<image_copy_capture::Frame>, bool)> {
        // Flip-pending gating happens in the caller (egl_compositor) so that
        // dirty_buffers aren't consumed when the render would be skipped.
        debug_assert!(
            !self.output.is_flip_pending(),
            "BUG: render_scene entered with flip pending; caller must gate on is_flip_pending"
        );

        self.import_textures(buffers, dirty);

        let buffer = self.buffers.back_buffer(&self.output)?;
        let fb = buffer.fb;

        // Collect render items: (buffer_id, placement, x_offset)
        let mut to_render = Vec::new();
        let drag_offset = widgets.drag_offset().unwrap_or(0);

        // Active scene at drag offset (0 when not dragging)
        collect_scene_widgets(widgets.active_scene(), buffers, drag_offset, &mut to_render);

        // Neighbor scene during drag
        if let Some(dx) = widgets.drag_offset() {
            #[expect(clippy::cast_possible_wrap)]
            let logical_width = self.output.logical_size().0 as i32;
            let (direction, neighbor_offset) = if dx < 0 {
                (1, dx + logical_width - SCENE_SEAM_OVERLAP)
            } else {
                (-1, dx - logical_width + SCENE_SEAM_OVERLAP)
            };
            if let Some(neighbor) = widgets.neighbor_scene(direction) {
                collect_scene_widgets(neighbor, buffers, neighbor_offset, &mut to_render);
            }
        }

        let renderer = self.egl.renderer();
        ii_stopwatch::stopwatch_start!(self.bind_w);
        let mut framebuffer = renderer
            .bind(&mut buffer.dmabuf)
            .context("Failed to bind render target")?;
        ii_stopwatch::stopwatch_stop!(self.bind_w);

        #[expect(clippy::cast_possible_wrap)]
        let output_size = Size::from((self.output.width() as i32, self.output.height() as i32));

        let mut frame = renderer
            .render(&mut framebuffer, output_size, Transform::Normal)
            .context("Failed to begin frame")?;

        let output_rect = Rectangle::from_size(output_size);
        let mut damage_rects = match output_damage {
            OutputDamage::Full => vec![output_rect],
            OutputDamage::Widgets(_) => Vec::new(),
        };

        // The `OutputDamage::Full` branch used to call
        // `frame.clear(BACKGROUND_COLOR, &damage_rects)` here. On Vivante
        // GC400 (Etnaviv Mesa) that clear corrupts sampler coherency
        // against the widget DMA-BUF textures sampled later in this
        // frame, reproducibly dropping first-use atlas texels and
        // producing the missing-glyph rendering captured in
        // docs/devlogs/BDK-389-combined-scene/glyph-damage-bisect.
        // Skipping it costs nothing when widgets cover the full output
        // (current scene model); if a future scene layout leaves
        // non-widget regions and needs hygiene clears, revisit with a
        // scissored, post-composite approach that doesn't precede any
        // texture sampling.
        let _ = BACKGROUND_COLOR;

        ii_stopwatch::stopwatch_start!(self.compose_w);
        for (buffer_id, placement, x_offset) in &to_render {
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
            let logical_x = placement.position.x as i32 + x_offset;
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

            if let OutputDamage::Widgets(dirty_widgets) = output_damage
                && dirty_widgets.contains(&placement.instance_id)
            {
                damage_rects.push(dst);
            }

            // Full-output damage during drag so the GPU doesn't skip the seam overlap region
            let full_damage = Rectangle::from_size(output_size);
            let damage = if widgets.drag_offset().is_some() {
                full_damage
            } else {
                dst
            };
            if let Err(e) = frame.render_texture_from_to(
                texture,
                src,
                dst,
                &[damage],
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

        // Capture readback is only needed when a capture session exists or
        // when frames are already pending. On Deck hardware this path hits
        // unsupported PBO readback, so keep it dormant otherwise.
        if capture_active || !capture_frames.is_empty() {
            let capture_failed = update_capture_cache(
                renderer,
                self.output.width(),
                self.output.height(),
                &mut self.capture_cache,
            );
            if capture_failed {
                return Ok((true, capture_frames, true));
            }
        }

        drop(framebuffer);
        self.egl.finish_rendering()?;

        // Fulfill any pending captures from the fresh cache (after dropping
        // the framebuffer borrow so self is available).
        if !capture_frames.is_empty() {
            self.fulfill_from_cache(capture_frames);
        }
        ii_stopwatch::stopwatch_stop!(self.finish_w);

        let damage_rects = if damage_rects.is_empty() {
            vec![output_rect]
        } else {
            merge_damage_rects(damage_rects)
        };

        ii_stopwatch::stopwatch_start!(self.flip_w);
        self.output.page_flip(fb, &damage_rects)?;
        ii_stopwatch::stopwatch_stop!(self.flip_w);

        self.buffers.swap();

        #[cfg(feature = "profiling")]
        if ii_stopwatch::every_expired!(self.render_every) {
            tracing::info!(
                "render_scene: bind={} compose={} finish={} flip={}",
                self.bind_w,
                self.compose_w,
                self.finish_w,
                self.flip_w
            );
            ii_stopwatch::stopwatch_reset!(self.bind_w);
            ii_stopwatch::stopwatch_reset!(self.compose_w);
            ii_stopwatch::stopwatch_reset!(self.finish_w);
            ii_stopwatch::stopwatch_reset!(self.flip_w);
        }

        Ok((true, Vec::new(), false))
    }

    /// Whether the capture cache holds valid pixel data from at least one
    /// successful `update_capture_cache` call. The compositor's main loop
    /// uses this to force an initial render when capture frames arrive
    /// before any render has happened (otherwise the first frame would
    /// always fail with `Unknown` because the default cache is empty).
    #[must_use]
    pub fn capture_cache_ready(&self) -> bool {
        self.capture_cache.valid
    }

    /// Serve capture frames from the cached pixel readback (no re-render).
    /// Used between renders when the compositor is idle.
    #[expect(
        clippy::cast_sign_loss,
        reason = "buffer dimensions are always positive"
    )]
    pub fn fulfill_from_cache(&self, frames: Vec<image_copy_capture::Frame>) {
        if !self.capture_cache.valid {
            // Should be rare: the main loop in egl_compositor forces a render
            // when capture frames arrive before the cache is populated, so by
            // the time we reach here the cache should always be valid. If we
            // hit this branch anyway something has gone wrong upstream — fail
            // the frames so the relay can report and reconnect.
            for frame in frames {
                frame.fail(CaptureFailureReason::Unknown);
            }
            return;
        }
        let c = &self.capture_cache;
        let src_stride = c.width as usize * 4;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        for frame in frames {
            let buffer = frame.buffer();
            let result = with_buffer_contents_mut(&buffer, |dst_ptr, dst_len, data| {
                let dst_stride = data.stride as usize;
                let copy_w = (data.width as usize).min(c.width as usize) * 4;
                let copy_h = (data.height as usize).min(c.height as usize);
                for row in 0..copy_h {
                    let src_row = if c.flipped { copy_h - 1 - row } else { row };
                    let src_off = src_row * src_stride;
                    let dst_off = row * dst_stride;
                    if src_off + copy_w <= c.pixels.len() && dst_off + copy_w <= dst_len {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                c.pixels.as_ptr().add(src_off),
                                dst_ptr.add(dst_off),
                                copy_w,
                            );
                        }
                    }
                }
            });
            match result {
                Ok(()) => frame.success(Transform::Normal, None, now),
                Err(e) => {
                    tracing::warn!("Capture cache write failed: {e:?}");
                    frame.fail(CaptureFailureReason::BufferConstraints);
                }
            }
        }
    }
}

/// Read back the currently-bound back buffer into the capture cache.
/// Called on every render so the cache always has the latest composited frame.
///
/// Bypasses smithay's `copy_framebuffer` because that path uses a PBO-bound
/// `glReadPixels` (`PIXEL_PACK_BUFFER` + null data ptr) that virgl on macOS
/// HVF rejects with `GL_OUT_OF_MEMORY` for any pixel format. A direct sync
/// `glReadPixels` into a CPU buffer is the most basic GL operation and works
/// on any conformant driver including virgl-on-macOS and the production Mali
/// GPU.
///
/// Pixels arrive in `GL_RGBA` byte order (R, G, B, A). The Wayland SHM buffer
/// is labelled `Xrgb8888`/`Argb8888` (BGRA byte order), so the bytes don't
/// match the label — consumers must honour that mismatch. We do this rather
/// than CPU-swizzling here because downstream consumers (e.g. the console's
/// `FbTexture`) can pass the source format to GPU `glTexSubImage2D` at no
/// cost, while a per-frame CPU swap costs ~90 MB/s of memory bandwidth on
/// the guest.
///
/// Relies on the back-buffer FBO being current from the just-completed
/// `bind`/`render`/`finish` sequence in `render_scene`.
#[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
fn update_capture_cache(
    renderer: &mut GlesRenderer,
    width: u32,
    height: u32,
    cache: &mut CaptureCache,
) -> bool {
    let len = (width as usize) * (height as usize) * 4;
    cache.pixels.resize(len, 0);

    let result = renderer.with_context(|gl| unsafe {
        // Drain pending GL errors so we can attribute the next one to ReadPixels.
        while gl.GetError() != ffi::NO_ERROR {}
        // Ensure no PBO is bound — sync readback writes directly to the CPU buffer.
        gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, 0);
        // FBO color attachment 0 is where the just-finished frame lives.
        gl.ReadBuffer(ffi::COLOR_ATTACHMENT0);
        gl.ReadPixels(
            0,
            0,
            width as i32,
            height as i32,
            ffi::RGBA,
            ffi::UNSIGNED_BYTE,
            cache.pixels.as_mut_ptr().cast(),
        );
        gl.GetError()
    });

    match result {
        Ok(ffi::NO_ERROR) => {
            cache.width = width;
            cache.height = height;
            cache.flipped = true; // glReadPixels origin is bottom-left
            cache.valid = true;
            false
        }
        Ok(err) => {
            tracing::warn!("Capture cache readback failed: GL error 0x{err:04x}");
            cache.valid = false;
            true
        }
        Err(e) => {
            tracing::warn!("Capture cache context error: {e:?}");
            cache.valid = false;
            true
        }
    }
}

fn merge_damage_rects(
    damage_rects: Vec<Rectangle<i32, Physical>>,
) -> Vec<Rectangle<i32, Physical>> {
    let mut merged = Vec::new();

    for rect in damage_rects {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| rectangles_overlap(existing, &rect))
        {
            *existing = rectangle_union(existing, &rect);
        } else {
            merged.push(rect);
        }
    }

    merged
}

fn rectangles_overlap(lhs: &Rectangle<i32, Physical>, rhs: &Rectangle<i32, Physical>) -> bool {
    lhs.loc.x < rhs.loc.x + rhs.size.w
        && rhs.loc.x < lhs.loc.x + lhs.size.w
        && lhs.loc.y < rhs.loc.y + rhs.size.h
        && rhs.loc.y < lhs.loc.y + lhs.size.h
}

fn rectangle_union(
    lhs: &Rectangle<i32, Physical>,
    rhs: &Rectangle<i32, Physical>,
) -> Rectangle<i32, Physical> {
    let x1 = lhs.loc.x.min(rhs.loc.x);
    let y1 = lhs.loc.y.min(rhs.loc.y);
    let x2 = (lhs.loc.x + lhs.size.w).max(rhs.loc.x + rhs.size.w);
    let y2 = (lhs.loc.y + lhs.size.h).max(rhs.loc.y + rhs.size.h);

    Rectangle::from_loc_and_size((x1, y1), (x2 - x1, y2 - y1))
}

/// Collect visible widgets from a scene into the render list with an x offset.
fn collect_scene_widgets(
    scene: &bmc::compositor::SceneLayout,
    buffers: &[(WlBuffer, bmc::compositor::InstanceId)],
    x_offset: i32,
    out: &mut Vec<(ObjectId, bmc::compositor::WidgetPlacement, i32)>,
) {
    for (client_buffer, instance_id) in buffers {
        if let Some(placement) = scene
            .widgets
            .iter()
            .find(|w| &w.instance_id == instance_id && w.visible)
        {
            out.push((client_buffer.id(), placement.clone(), x_offset));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_damage_rects, rectangle_union};
    use smithay::utils::{Physical, Rectangle};

    #[test]
    fn overlapping_damage_rectangles_are_merged() {
        let merged = merge_damage_rects(vec![
            Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (10, 10)),
            Rectangle::<i32, Physical>::from_loc_and_size((5, 5), (10, 10)),
        ]);

        assert_eq!(
            merged,
            vec![Rectangle::<i32, Physical>::from_loc_and_size(
                (0, 0),
                (15, 15)
            )]
        );
    }

    #[test]
    fn disjoint_damage_rectangles_stay_separate() {
        let lhs = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (10, 10));
        let rhs = Rectangle::<i32, Physical>::from_loc_and_size((20, 20), (10, 10));

        assert_eq!(merge_damage_rects(vec![lhs, rhs]), vec![lhs, rhs]);
    }

    #[test]
    fn rectangle_union_covers_both_inputs() {
        let lhs = Rectangle::<i32, Physical>::from_loc_and_size((10, 20), (5, 5));
        let rhs = Rectangle::<i32, Physical>::from_loc_and_size((12, 18), (8, 10));

        assert_eq!(
            rectangle_union(&lhs, &rhs),
            Rectangle::<i32, Physical>::from_loc_and_size((10, 18), (10, 10))
        );
    }
}
