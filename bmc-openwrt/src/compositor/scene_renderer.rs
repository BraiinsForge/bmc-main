// Copyright (C) 2025  Braiins Systems s.r.o.

//! Scene renderer for compositing multiple widgets.

use std::collections::HashMap;

use anyhow::{Context, Result};
use bmc_platform::{DisplayPixelFormat, DisplayTransform};
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

use super::gpu_render_lock::GpuRenderLock;
use super::render::{BufferPool, DrmOutput, EglContext, ScanoutFormat, ScanoutSwizzler};
use super::state::OutputDamage;
use super::widget_tracker::WidgetTracker;

const BACKGROUND_COLOR: Color32F = Color32F::new(0.05, 0.05, 0.1, 1.0);

#[must_use]
pub fn scanout_transform(profile: DisplayTransform) -> Transform {
    match profile {
        DisplayTransform::Deg0 => Transform::Normal,
        DisplayTransform::Deg90 => Transform::_90,
        DisplayTransform::Deg270 => Transform::_270,
    }
}

/// Map a widget's logical placement to its physical destination rectangle on the rotated panel.
/// `output_w`/`output_h` are the **physical** panel dimensions (post-crop, pre-rotation) — the
/// same values returned by `DrmOutput::width()` / `DrmOutput::height()`. `tex_w`/`tex_h` are
/// texture dimensions in logical (un-rotated) space; the helper applies the axis swap when the
/// scanout transform rotates by 90° or 270°.
#[must_use]
pub fn place_widget(
    logical_x: i32,
    logical_y: i32,
    tex_w: i32,
    tex_h: i32,
    output_w: i32,
    output_h: i32,
    transform: DisplayTransform,
) -> Rectangle<i32, Physical> {
    let (phys_w, phys_h, physical_x, physical_y) = match transform {
        DisplayTransform::Deg0 => (tex_w, tex_h, logical_x, logical_y),
        DisplayTransform::Deg270 => (tex_h, tex_w, logical_y, output_h - logical_x - tex_w),
        DisplayTransform::Deg90 => (tex_h, tex_w, output_w - logical_y - tex_h, logical_x),
    };
    Rectangle::from_loc_and_size((physical_x, physical_y), (phys_w, phys_h))
}

#[must_use]
pub fn touch_to_logical(
    x: f64,
    y: f64,
    logical_width: f64,
    logical_height: f64,
    transform: bmc_platform::TouchTransform,
) -> (f64, f64) {
    match transform {
        bmc_platform::TouchTransform::Deg0 => (x, y),
        bmc_platform::TouchTransform::Deg90 => (y, logical_width - x),
        bmc_platform::TouchTransform::Deg270 => (logical_height - y, x),
    }
}

pub struct SceneRenderer {
    egl: EglContext,
    output: DrmOutput,
    buffers: BufferPool,
    /// Present `XRGB8888` directly (`None`) or run a BGR565 swizzle output pass
    /// over a separate `RG16` scanout buffer before page-flip (`Some`).
    swizzler: Option<ScanoutSwizzler>,
    /// Texture cache: maps WlBuffer ObjectId to cached GlesTexture
    texture_cache: HashMap<ObjectId, GlesTexture>,
    /// Cached pixels from the last inline capture readback.
    /// Served to capture clients between renders (avoids re-rendering
    /// just to observe the same frame).
    capture_cache: CaptureCache,
    gpu_render_lock: GpuRenderLock,
    scanout_transform: DisplayTransform,
    seam_overlap_px: i32,
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
    pub fn new(
        mut egl: EglContext,
        output: DrmOutput,
        scanout_transform: DisplayTransform,
        seam_overlap_px: i32,
        pixel_format: DisplayPixelFormat,
    ) -> Result<Self> {
        let (width, height) = (output.width(), output.height());
        let (logical_w, logical_h) = output.logical_size();
        tracing::info!(
            "SceneRenderer: physical {}x{}, logical {}x{}",
            width,
            height,
            logical_w,
            logical_h
        );
        let swizzler = match pixel_format {
            DisplayPixelFormat::Xrgb8888 => None,
            DisplayPixelFormat::Bgr565 => Some(
                ScanoutSwizzler::new(egl.renderer(), width, height)
                    .context("Failed to set up BGR565 swizzle output pass")?,
            ),
        };
        Ok(Self {
            egl,
            output,
            buffers: BufferPool::new(width, height, ScanoutFormat::Xrgb8888),
            swizzler,
            texture_cache: HashMap::new(),
            capture_cache: CaptureCache::empty(),
            gpu_render_lock: GpuRenderLock::from_env()?,
            scanout_transform,
            seam_overlap_px,
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
        })
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
                if dirty.contains(&buffer_id) {
                    match renderer.import_dmabuf(dmabuf, None) {
                        Ok(texture) => {
                            self.texture_cache.insert(buffer_id, texture);
                        }
                        Err(e) => {
                            tracing::warn!("import_dmabuf failed for buffer {:?}: {e}", buffer_id);
                        }
                    }
                }
            } else {
                match renderer.import_shm_buffer(client_buffer, None, &[]) {
                    Ok(texture) => {
                        self.texture_cache.insert(buffer_id, texture);
                    }
                    Err(e) => {
                        tracing::warn!("import_shm_buffer failed for buffer {:?}: {e}", buffer_id);
                    }
                }
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

        let _gpu_render_lock = self.gpu_render_lock.lock("compositor_render_scene")?;
        self.import_textures(buffers, dirty);

        // Collect render items: (buffer_id, placement, x_offset)
        let mut to_render = Vec::new();
        let drag_offset = widgets.drag_offset().unwrap_or(0);

        // Active scene at drag offset (0 when not dragging)
        collect_scene_widgets(widgets.active_scene(), buffers, drag_offset, &mut to_render);

        // Neighbor scene during drag
        if let Some(dx) = widgets.drag_offset() {
            #[expect(clippy::cast_possible_wrap)]
            let logical_width = self.output.logical_size().0 as i32;
            let neighbor_offset = if dx <= 0 {
                dx + logical_width - self.seam_overlap_px
            } else {
                dx - logical_width + self.seam_overlap_px
            };
            if let Some(neighbor) = widgets.drag_neighbor_scene() {
                collect_scene_widgets(neighbor, buffers, neighbor_offset, &mut to_render);
            }
        }

        let buffer = self.buffers.back_buffer(&self.output)?;
        let fb = buffer.fb;

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

        let mut renderable_items = Vec::new();

        ii_stopwatch::stopwatch_start!(self.compose_w);
        for (buffer_id, placement, x_offset) in &to_render {
            let Some(texture) = self.texture_cache.get(buffer_id) else {
                tracing::warn!("No cached texture for buffer {:?}", buffer_id);
                continue;
            };

            let tex_size = texture.size();

            #[expect(clippy::cast_possible_wrap)]
            let logical_x = placement.position.x as i32 + x_offset;
            #[expect(clippy::cast_possible_wrap)]
            let logical_y = placement.position.y as i32;

            #[expect(clippy::cast_possible_wrap)]
            let output_w = self.output.width() as i32;
            #[expect(clippy::cast_possible_wrap)]
            let output_h = self.output.height() as i32;
            let dst = place_widget(
                logical_x,
                logical_y,
                tex_size.w,
                tex_size.h,
                output_w,
                output_h,
                self.scanout_transform,
            );

            if let OutputDamage::Widgets(dirty_widgets) = output_damage
                && dirty_widgets.contains(&placement.instance_id)
            {
                damage_rects.push(dst);
            }

            renderable_items.push((buffer_id.clone(), placement.instance_id.clone(), dst));
        }

        let drawn_regions: Vec<_> = renderable_items.iter().map(|(_, _, dst)| *dst).collect();

        // Clear regions not covered by any widget before drawing widgets.
        // Clearing after widget draws can overpaint widget content on the
        // target hardware when the clear path and rotated texture path mix.
        let clear_regions = uncovered_output_regions(output_rect, drawn_regions);
        if !clear_regions.is_empty() {
            frame
                .clear(BACKGROUND_COLOR, &clear_regions)
                .context("Failed to clear uncovered output regions")?;
        }

        for (buffer_id, instance_id, dst) in &renderable_items {
            let Some(texture) = self.texture_cache.get(buffer_id) else {
                tracing::warn!("No cached texture for buffer {:?}", buffer_id);
                continue;
            };
            let tex_size = texture.size();
            let src: Rectangle<f64, BufferCoord> = Rectangle::from_loc_and_size(
                (0.0, 0.0),
                (f64::from(tex_size.w), f64::from(tex_size.h)),
            );
            let damage = texture_damage_rect(*dst);
            if let Err(e) = frame.render_texture_from_to(
                texture,
                src,
                *dst,
                &[damage],
                &[],
                scanout_transform(self.scanout_transform),
                1.0,
                None,
                &[],
            ) {
                tracing::warn!("Failed to render widget {}: {:?}", instance_id, e);
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
        // Clone the rendered buffer for the swizzler before `buffer`'s pool
        // borrow ends; only the BGR565 path needs it, so skip the clone
        // otherwise. Paired with the swizzler in the match below.
        let intermediate = self.swizzler.is_some().then(|| buffer.dmabuf.clone());
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

        // For BGR565 panels the page-flipped buffer is the swizzler's RG16
        // scanout, produced from the natural-RGB intermediate. Otherwise the
        // intermediate is itself the scanout buffer.
        let scanout_fb = match (self.swizzler.as_mut(), intermediate) {
            (Some(swizzler), Some(intermediate)) => {
                swizzler.present(self.egl.renderer(), &self.output, &intermediate)?
            }
            _ => fb,
        };

        ii_stopwatch::stopwatch_start!(self.flip_w);
        self.output.page_flip(scanout_fb, &damage_rects)?;
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

fn texture_damage_rect(dst: Rectangle<i32, Physical>) -> Rectangle<i32, Physical> {
    Rectangle::from_size(dst.size)
}

fn uncovered_output_regions(
    output_rect: Rectangle<i32, Physical>,
    drawn_regions: Vec<Rectangle<i32, Physical>>,
) -> Vec<Rectangle<i32, Physical>> {
    Rectangle::subtract_rects_many_in_place(vec![output_rect], drawn_regions)
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
    use super::{
        merge_damage_rects, rectangle_union, texture_damage_rect, uncovered_output_regions,
    };
    use crate::compositor::scene_renderer::{place_widget, scanout_transform, touch_to_logical};
    use bmc_platform::{DisplayTransform, TouchTransform};
    use smithay::utils::{Physical, Rectangle, Transform};

    #[test]
    fn scanout_transform_maps_each_profile_degree() {
        assert_eq!(scanout_transform(DisplayTransform::Deg0), Transform::Normal);
        assert_eq!(scanout_transform(DisplayTransform::Deg90), Transform::_90);
        assert_eq!(scanout_transform(DisplayTransform::Deg270), Transform::_270);
    }

    #[test]
    fn place_widget_deg0_is_identity() {
        let dst = place_widget(50, 30, 200, 100, 320, 240, DisplayTransform::Deg0);
        assert_eq!(
            dst,
            Rectangle::<i32, Physical>::from_loc_and_size((50, 30), (200, 100)),
        );
        let full = place_widget(0, 0, 320, 240, 320, 240, DisplayTransform::Deg0);
        assert_eq!(
            full,
            Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (320, 240)),
        );
    }

    #[test]
    fn place_widget_deg270_matches_current_bmc100_math() {
        let dst = place_widget(0, 0, 638, 480, 480, 1280, DisplayTransform::Deg270);
        assert_eq!(
            dst,
            Rectangle::<i32, Physical>::from_loc_and_size((0, 642), (480, 638)),
        );
        let full = place_widget(0, 0, 1280, 480, 480, 1280, DisplayTransform::Deg270);
        assert_eq!(
            full,
            Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (480, 1280)),
        );
        let right = place_widget(642, 0, 638, 480, 480, 1280, DisplayTransform::Deg270);
        assert_eq!(
            right,
            Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (480, 638)),
        );
    }

    #[test]
    fn place_widget_deg90_mirrors_deg270_on_the_opposite_axis() {
        let full = place_widget(0, 0, 480, 480, 480, 480, DisplayTransform::Deg90);
        assert_eq!(
            full,
            Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (480, 480)),
        );
        let dst = place_widget(40, 20, 200, 100, 480, 480, DisplayTransform::Deg90);
        assert_eq!(
            dst,
            Rectangle::<i32, Physical>::from_loc_and_size((360, 40), (100, 200)),
        );
    }

    #[test]
    fn place_widget_keeps_widget_within_output_bounds() {
        let cases = [
            (DisplayTransform::Deg0, 320, 240, 200, 100, 50, 30),
            (DisplayTransform::Deg270, 480, 1280, 638, 480, 0, 0),
            (DisplayTransform::Deg90, 480, 480, 200, 100, 40, 20),
        ];
        for (transform, output_w, output_h, tex_w, tex_h, logical_x, logical_y) in cases {
            let dst = place_widget(
                logical_x, logical_y, tex_w, tex_h, output_w, output_h, transform,
            );
            assert!(
                dst.loc.x >= 0 && dst.loc.x + dst.size.w <= output_w,
                "{transform:?}: x out of bounds: {dst:?}"
            );
            assert!(
                dst.loc.y >= 0 && dst.loc.y + dst.size.h <= output_h,
                "{transform:?}: y out of bounds: {dst:?}"
            );
        }
    }

    #[test]
    fn touch_to_logical_maps_profile_transforms() {
        assert_eq!(
            touch_to_logical(10.0, 20.0, 320.0, 240.0, TouchTransform::Deg0),
            (10.0, 20.0)
        );
        assert_eq!(
            touch_to_logical(10.0, 20.0, 320.0, 240.0, TouchTransform::Deg90),
            (20.0, 310.0)
        );
        assert_eq!(
            touch_to_logical(10.0, 20.0, 320.0, 240.0, TouchTransform::Deg270),
            (220.0, 10.0)
        );
    }

    #[test]
    fn touch_to_logical_pins_bmc100_panel_mapping() {
        // The GT911 reports its axes already in the logical landscape
        // orientation, so BMC100 maps touch with the identity transform.
        let w = 1280.0_f64;
        let h = 480.0_f64;
        assert_eq!(
            touch_to_logical(0.0, 0.0, w, h, TouchTransform::Deg0),
            (0.0, 0.0),
        );
        assert_eq!(
            touch_to_logical(1280.0, 0.0, w, h, TouchTransform::Deg0),
            (1280.0, 0.0),
        );
        assert_eq!(
            touch_to_logical(1280.0, 480.0, w, h, TouchTransform::Deg0),
            (1280.0, 480.0),
        );
        assert_eq!(
            touch_to_logical(0.0, 480.0, w, h, TouchTransform::Deg0),
            (0.0, 480.0),
        );
        assert_eq!(
            touch_to_logical(640.0, 240.0, w, h, TouchTransform::Deg0),
            (640.0, 240.0),
        );
    }

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

    #[test]
    fn texture_damage_is_local_to_destination_rect() {
        let dst = Rectangle::<i32, Physical>::from_loc_and_size((240, 642), (480, 638));

        assert_eq!(
            texture_damage_rect(dst),
            Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (480, 638))
        );
    }

    #[test]
    fn uncovered_output_regions_detects_gap_and_edge_strip() {
        let output = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (10, 2));
        let drawn = vec![
            Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (3, 2)),
            Rectangle::<i32, Physical>::from_loc_and_size((5, 0), (3, 2)),
        ];

        let mut clear = uncovered_output_regions(output, drawn);
        clear.sort_by_key(|r| (r.loc.x, r.loc.y, r.size.w, r.size.h));

        assert_eq!(
            clear,
            vec![
                Rectangle::<i32, Physical>::from_loc_and_size((3, 0), (2, 2)),
                Rectangle::<i32, Physical>::from_loc_and_size((8, 0), (2, 2)),
            ]
        );
    }

    #[test]
    fn uncovered_output_regions_is_empty_when_fully_covered() {
        let output = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), (10, 2));
        let drawn = vec![Rectangle::<i32, Physical>::from_loc_and_size(
            (0, 0),
            (10, 2),
        )];

        assert!(uncovered_output_regions(output, drawn).is_empty());
    }
}
