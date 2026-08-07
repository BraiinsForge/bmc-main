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

//! Scene renderer for compositing multiple widgets.

use std::collections::HashMap;

use anyhow::{Context, Result};
use bmc::compositor::ScenePlaceholder;
use bmc::scene::WidgetPosition;
use bmc_gpu_render_lock::GpuRenderLock;
use bmc_platform::{DisplayPixelFormat, DisplayTransform, Product};
use bmc_shared_utils::include_png;
use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        Bind, Color32F, Frame as RendererFrame, ImportDma, ImportMem, ImportMemWl, Renderer,
        Texture,
        gles::GlesFrame,
        gles::{GlesRenderer, GlesTexture, ffi},
    },
    reexports::wayland_server::{Resource, backend::ObjectId, protocol::wl_buffer::WlBuffer},
    utils::{Buffer as BufferCoord, Logical, Physical, Rectangle, Size, Transform},
    wayland::{
        dmabuf::get_dmabuf,
        image_copy_capture::{self, CaptureFailureReason},
        shm::with_buffer_contents_mut,
    },
};

use super::render::{BufferPool, DrmOutput, EglContext, ScanoutFormat, ScanoutSwizzler};
use super::scene_cycling::TransitionFrame;
use super::state::OutputDamage;
use super::widget_tracker::WidgetTracker;

const BACKGROUND_COLOR: Color32F = Color32F::new(0.0, 0.0, 0.0, 1.0);
const SEPARATOR_COLOR: Color32F = Color32F::new(0.15, 0.15, 0.15, 1.0);

/// DECK logo shown as the screen background when a scene has no rendered
/// content. Embedded so there is no runtime file dependency.
const DECK_LOGO_PNG: &[u8] = include_png!("../assets/deck_logo.png");

/// Branding logo a product shows in scenes with no rendered content, or
/// `None` to keep the plain cleared background (which also drops the
/// "Loading scene…" caption). The DECK logo is Deck branding and is wider
/// (527px) than the other products' panels, so only the Deck shows it.
#[must_use]
pub fn logo_for_product(product: Product) -> Option<&'static [u8]> {
    match product {
        Product::Bmc100 => Some(DECK_LOGO_PNG),
        Product::Bmm100 | Product::Bmm101 | Product::Bfm100 => None,
    }
}

/// "Loading scene…" caption drawn below the logo while a configured scene has
/// not painted its first frame yet.
const LOADING_SCENE_TEXT_PNG: &[u8] = include_png!("../assets/loading_scene_text.png");

/// Logical-pixel gap between the logo and the caption below it.
const LOGO_TEXT_GAP_PX: i32 = 42;

#[must_use]
pub fn scanout_transform(profile: DisplayTransform) -> Transform {
    match profile {
        DisplayTransform::Deg0 => Transform::Normal,
        DisplayTransform::Deg90 => Transform::_90,
        DisplayTransform::Deg270 => Transform::_270,
    }
}

/// Controls when an SHM buffer's texture is reimported relative to the dirty set.
#[derive(Clone, Copy)]
enum ShmImport {
    /// Always reimport — widgets repaint into the same `WlBuffer` without destroying it,
    /// so a new commit does not produce a new buffer ID.
    Always,
    /// Only reimport when the buffer ID appears in the dirty set — layer surfaces replace
    /// their buffer on each commit, so the dirty check is sufficient.
    WhenDirty,
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

fn draw_rect_on_frame(
    frame: &mut GlesFrame<'_, '_>,
    logical: Rectangle<i32, Logical>,
    output_w: i32,
    output_h: i32,
    transform: DisplayTransform,
    color: Color32F,
) {
    let dst = place_widget(
        logical.loc.x,
        logical.loc.y,
        logical.size.w,
        logical.size.h,
        output_w,
        output_h,
        transform,
    );
    if let Err(e) = frame.draw_solid(dst, &[texture_damage_rect(dst)], color) {
        tracing::warn!("Failed to draw separator rect {:?}: {:?}", dst, e);
    }
}

// `draw_separator_grids` and `draw_logo_scenes` repaint without reporting into
// the frame's `damage_rects`, which only holds up while `DrmOutput::page_flip`
// discards damage clips. Fail the build rather than the panel if that flips
// before the two draws are damage-tracked.
const _: () = assert!(
    !DrmOutput::DAMAGE_CLIPS_ENABLED,
    "BUG: the logo, caption and separator draws do not report damage - damage-track them before enabling damage clips"
);

/// A combined scene showing the separator grid this frame: the x-offset the
/// grid slides with during swipes and transitions, and the opacity it fades
/// with during cross-fades.
struct CombinedSceneGrid {
    x_offset: i32,
    alpha: f32,
}

/// Draw the combined-scene separator grid once per entry in `grids`,
/// so it slides with its scene during swipes and transitions
/// and fades with it during cross-fades. Widgets snap to
/// `WidgetPosition::{COL,ROW}_PITCH` (viewport + a uniform 4px gap), so a strip
/// drawn in the gap just before each internal boundary shows as the separator,
/// and is covered or trimmed by the widgets blitted on top: a spanning widget
/// hides its internal boundary line, an occupied cell trims its strips to the
/// 4px gap, and an empty cell keeps a black interior framed by the lines.
/// During a cross-fade the widgets go translucent and covered strips can
/// bleed through faintly — verified invisible on device at the fade's pace.
/// Geometry is sourced from `WidgetPosition` + `DrmOutput::logical_size`.
fn draw_separator_grids(
    frame: &mut GlesFrame<'_, '_>,
    output: &DrmOutput,
    transform: DisplayTransform,
    grids: &[CombinedSceneGrid],
) {
    if grids.is_empty() {
        return;
    }
    let (lw, lh) = output.logical_size();
    // All panel/grid geometry is small and non-negative; narrow the
    // u32/usize sources to the i32 space the signed x-offsets live in.
    let to_i32 = |v: u32| i32::try_from(v).expect("BUG: panel/grid geometry fits i32");
    let output_w = to_i32(output.width());
    let output_h = to_i32(output.height());
    let logical_w = to_i32(lw);
    let logical_h = to_i32(lh);
    let gap = to_i32(WidgetPosition::SEPARATOR_PX);
    let col_pitch = to_i32(WidgetPosition::col_pitch(lw));
    let row_pitch = to_i32(WidgetPosition::row_pitch(lh));
    let cols = i32::try_from(WidgetPosition::MAX_COLS).expect("BUG: grid dimension fits i32");
    let rows = i32::try_from(WidgetPosition::MAX_ROWS).expect("BUG: grid dimension fits i32");
    for grid in grids {
        // `Color32F` is premultiplied, so scaling all components fades it.
        let color = SEPARATOR_COLOR * grid.alpha;
        for col in 1..cols {
            let x = col * col_pitch - gap + grid.x_offset;
            draw_rect_on_frame(
                frame,
                Rectangle::from_loc_and_size((x, 0), (gap, logical_h)),
                output_w,
                output_h,
                transform,
                color,
            );
        }
        for row in 1..rows {
            let y = row * row_pitch - gap;
            draw_rect_on_frame(
                frame,
                Rectangle::from_loc_and_size((grid.x_offset, y), (logical_w, gap)),
                output_w,
                output_h,
                transform,
                color,
            );
        }
    }
}

/// Decode an embedded PNG and upload it as a texture. Returns `None` (with a
/// warning) on failure so the compositor still starts — these overlays are
/// cosmetic. `label` names the asset in log messages.
fn load_texture_from_png(
    renderer: &mut GlesRenderer,
    png: &[u8],
    label: &str,
) -> Option<GlesTexture> {
    let rgba = match image::load_from_memory(png) {
        Ok(image) => image.to_rgba8(),
        Err(e) => {
            tracing::warn!("Failed to decode {label}: {e:?}");
            return None;
        }
    };
    let (width, height) = rgba.dimensions();
    let (Ok(w), Ok(h)) = (i32::try_from(width), i32::try_from(height)) else {
        tracing::warn!("{label} dimensions {width}x{height} do not fit i32");
        return None;
    };
    let size = Size::<i32, BufferCoord>::from((w, h));
    // `to_rgba8` yields bytes in R,G,B,A order, i.e. little-endian ABGR8888.
    match renderer.import_memory(rgba.as_raw(), Fourcc::Abgr8888, size, false) {
        Ok(texture) => Some(texture),
        Err(e) => {
            tracing::warn!("Failed to upload {label} texture: {e:?}");
            None
        }
    }
}

/// An overlay texture held only while an overlay scene is on screen. The
/// `Failed` state keeps a failed load from re-decoding the PNG on every render
/// for as long as that scene stays up; [`OverlayTexture::unload`] clears it
/// along with the texture, so the next entry tries once more — a GL upload can
/// fail transiently, and one attempt per entry is what a successful load costs
/// anyway.
enum OverlayTexture {
    Unloaded,
    Failed,
    Ready(GlesTexture),
}

impl OverlayTexture {
    /// The texture to draw, or `None` while unloaded or failed.
    fn texture(&self) -> Option<&GlesTexture> {
        match self {
            Self::Ready(texture) => Some(texture),
            Self::Unloaded | Self::Failed => None,
        }
    }

    /// Decode and upload `png`, unless that already succeeded or already
    /// failed since the last [`Self::unload`].
    fn ensure_loaded(&mut self, renderer: &mut GlesRenderer, png: &[u8], label: &str) {
        match self {
            Self::Failed | Self::Ready(_) => (),
            Self::Unloaded => {
                *self = match load_texture_from_png(renderer, png, label) {
                    Some(texture) => Self::Ready(texture),
                    None => Self::Failed,
                };
            }
        }
    }

    /// Drop the texture and forget a failed load.
    fn unload(&mut self) {
        *self = Self::Unloaded;
    }
}

/// Blit a texture at a logical top-left placement, applying the panel's scanout
/// transform. The destination is `size` when given — the texture scales into
/// it — and texture-sized (no scaling) when `None`. Layer surfaces pass their
/// configured geometry so a mismatched buffer keeps agreeing with the
/// geometry-sized touch hit-box (the mismatch itself is warned at the commit
/// boundary). Logs a warning on failure without aborting the frame.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors place_widget's explicit geometry inputs"
)]
fn blit_texture_on_frame(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    logical_x: i32,
    logical_y: i32,
    size: Option<Size<i32, Logical>>,
    output_w: i32,
    output_h: i32,
    transform: DisplayTransform,
    alpha: f32,
    label: &str,
) {
    let tex_size = texture.size();
    let dst_size = size.map_or((tex_size.w, tex_size.h), |s| (s.w, s.h));
    let dst = place_widget(
        logical_x, logical_y, dst_size.0, dst_size.1, output_w, output_h, transform,
    );
    let src: Rectangle<f64, BufferCoord> =
        Rectangle::from_loc_and_size((0.0, 0.0), (f64::from(tex_size.w), f64::from(tex_size.h)));
    if let Err(e) = frame.render_texture_from_to(
        texture,
        src,
        dst,
        &[texture_damage_rect(dst)],
        &[],
        scanout_transform(transform),
        alpha,
        None,
        &[],
    ) {
        tracing::warn!("Failed to render {label}: {e:?}");
    }
}

/// Start coordinate (left/top edge along one axis) at which `content` sits
/// centered inside `container`; the 1px bias from integer division is fine
/// for the overlays this positions.
#[expect(
    clippy::integer_division,
    reason = "1px rounding when centering is fine"
)]
fn centered_start(container: i32, content: i32) -> i32 {
    (container - content) / 2
}

/// A scene showing the branding logo this frame: the x-offset it slides with
/// during swipes and transitions, the opacity it fades with during cross-fades,
/// and whether the "Loading scene…" caption is drawn below the logo
/// (see [`ScenePlaceholder`]).
struct LogoScene {
    x_offset: i32,
    alpha: f32,
    with_caption: bool,
}

/// Draw the DECK logo centered on every scene in `scenes` — the on-screen
/// scenes that have no rendered widget yet (the empty sentinel, or a
/// configured scene whose widgets have not painted — fullscreen, combined, or
/// preview). A scene with a caption also gets the "Loading scene…" `caption`
/// below the logo. Drawn per scene at its x-offset so it slides with
/// swipes/transitions; these scenes are excluded from the separator grid, so
/// the grid never overlaps the logo. Products without a logo (see
/// [`logo_for_product`]) pass `logo: None` and keep the cleared background.
fn draw_logo_scenes(
    frame: &mut GlesFrame<'_, '_>,
    output: &DrmOutput,
    transform: DisplayTransform,
    scenes: &[LogoScene],
    logo: Option<&GlesTexture>,
    caption: Option<&GlesTexture>,
) {
    let Some(logo) = logo else {
        return;
    };
    let (lw, lh) = output.logical_size();
    // Panel dimensions are small and non-negative; narrow them to the i32
    // space the signed x-offsets live in.
    let to_i32 = |v: u32| i32::try_from(v).expect("BUG: panel geometry fits i32");
    let output_w = to_i32(output.width());
    let output_h = to_i32(output.height());
    let logical_w = to_i32(lw);
    let logical_h = to_i32(lh);
    let logo_size = logo.size();
    for scene in scenes {
        // Draw the logo centered.
        let logo_x = scene.x_offset + centered_start(logical_w, logo_size.w);
        let logo_y = centered_start(logical_h, logo_size.h);
        blit_texture_on_frame(
            frame,
            logo,
            logo_x,
            logo_y,
            None,
            output_w,
            output_h,
            transform,
            scene.alpha,
            "DECK logo",
        );

        // Draw optional caption under the logo.
        if scene.with_caption
            && let Some(caption) = caption
        {
            let caption_size = caption.size();
            let caption_x = scene.x_offset + centered_start(logical_w, caption_size.w);
            let caption_y = logo_y + logo_size.h + LOGO_TEXT_GAP_PX;
            blit_texture_on_frame(
                frame,
                caption,
                caption_x,
                caption_y,
                None,
                output_w,
                output_h,
                transform,
                scene.alpha,
                "loading text",
            );
        }
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
    /// Branding logo drawn when a scene has no rendered content, from
    /// [`logo_for_product`]; `None` keeps the plain cleared background.
    logo_png: Option<&'static [u8]>,
    /// Logo texture decoded from [`Self::logo_png`]. Loaded on demand and
    /// dropped when unused.
    logo_texture: OverlayTexture,
    /// "Loading scene…" caption texture. Loaded on demand and dropped when
    /// unused.
    loading_text_texture: OverlayTexture,
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
        logo_png: Option<&'static [u8]>,
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
            logo_png,
            // Overlay textures are loaded lazily on first use (see render_scene).
            logo_texture: OverlayTexture::Unloaded,
            loading_text_texture: OverlayTexture::Unloaded,
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

    /// Import a single buffer's texture into the cache.
    ///
    /// DMA-BUF is always dirty-gated: a new EGLImage is created only when the buffer ID
    /// appears in `dirty`. SHM behaviour is controlled by `shm_import`:
    /// - `ShmImport::Always` — reimport on every call (widget path: clients repaint into
    ///   the same `WlBuffer` without destroying it, so the buffer ID never changes).
    /// - `ShmImport::WhenDirty` — reimport only when the buffer ID is in `dirty`
    ///   (layer path: each commit replaces the buffer, so the dirty set is sufficient).
    fn import_buffer_texture(
        &mut self,
        buffer: &WlBuffer,
        dirty: &[ObjectId],
        shm_import: ShmImport,
        label: &str,
    ) {
        let buffer_id = buffer.id();
        if let Ok(dmabuf) = get_dmabuf(buffer) {
            if dirty.contains(&buffer_id) {
                match self.egl.renderer().import_dmabuf(dmabuf, None) {
                    Ok(texture) => {
                        self.texture_cache.insert(buffer_id, texture);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "import_dmabuf failed for {label} buffer {:?}: {e}",
                            buffer_id
                        );
                    }
                }
            }
        } else {
            let do_import = match shm_import {
                ShmImport::Always => true,
                ShmImport::WhenDirty => dirty.contains(&buffer_id),
            };
            if do_import {
                match self.egl.renderer().import_shm_buffer(buffer, None, &[]) {
                    Ok(texture) => {
                        self.texture_cache.insert(buffer_id, texture);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "import_shm_buffer failed for {label} buffer {:?}: {e}",
                            buffer_id
                        );
                    }
                }
            }
        }
    }

    /// Import widget textures that were newly committed since the last render.
    ///
    /// Only reimports DMA-BUF buffers whose ObjectId appears in `dirty_buffers`
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
        for (client_buffer, _instance_id) in buffers {
            self.import_buffer_texture(client_buffer, dirty, ShmImport::Always, "widget");
        }
    }

    /// Whether any visible widget of `scene` has already committed a buffer
    /// that was imported into a texture — i.e. the scene has real content to
    /// show this frame rather than a blank/loading placeholder.
    fn scene_has_rendered_widget(
        &self,
        scene: &bmc::compositor::SceneLayout,
        buffers: &[(WlBuffer, bmc::compositor::InstanceId)],
    ) -> bool {
        buffers.iter().any(|(buffer, instance_id)| {
            self.texture_cache.contains_key(&buffer.id())
                && scene
                    .widgets
                    .iter()
                    .any(|w| &w.instance_id == instance_id && w.visible)
        })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "render hot-path with stopwatch instrumentation; splitting hurts readability"
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "render hot-path inputs are owned by compositor state and renderer-local grouping would obscure call sites"
    )]
    pub fn render_scene(
        &mut self,
        widgets: &WidgetTracker,
        transition_frame: Option<TransitionFrame>,
        buffers: &[(WlBuffer, bmc::compositor::InstanceId)],
        layers: &[(WlBuffer, Rectangle<i32, Logical>)],
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
        // This lock serializes completed GPU jobs across the compositor and
        // WASM host contexts. Hold it until all GL work submitted from this
        // path has finished, so a handoff cannot leave overlapping in-flight
        // jobs on etnaviv.
        self.import_textures(buffers, dirty);

        for (buffer, _) in layers {
            self.import_buffer_texture(buffer, dirty, ShmImport::WhenDirty, "layer");
        }

        // Collect render items: (buffer_id, placement, x_offset, alpha)
        let mut to_render = Vec::new();
        // Rendered combined scenes in view; each gets a separator grid,
        // sliding and fading with its scene. Scenes that show the logo
        // instead are excluded so the grid never sits on it — except mid
        // cross-fade, where the other scene's fading grid can overlap a
        // logo parked at the same offset.
        let mut combined_scene_grids: Vec<CombinedSceneGrid> = Vec::new();
        // On-screen scenes showing the branding logo, per their
        // `ScenePlaceholder`. Products without a logo (see `logo_for_product`)
        // keep the plain cleared background.
        let mut logo_scenes: Vec<LogoScene> = Vec::new();

        for rendered in widgets.rendered_scenes(transition_frame, self.seam_overlap_px) {
            collect_scene_widgets(
                rendered.scene,
                buffers,
                rendered.x_offset,
                rendered.alpha,
                &mut to_render,
            );
            if self.scene_has_rendered_widget(rendered.scene, buffers) {
                if rendered.scene.combined {
                    combined_scene_grids.push(CombinedSceneGrid {
                        x_offset: rendered.x_offset,
                        alpha: rendered.alpha,
                    });
                }
            } else {
                // The grid-vs-logo-vs-caption policy lives on SceneLayout;
                // see `ScenePlaceholder` for the policy and its rationale.
                match rendered.scene.placeholder() {
                    ScenePlaceholder::Grid => {
                        combined_scene_grids.push(CombinedSceneGrid {
                            x_offset: rendered.x_offset,
                            alpha: rendered.alpha,
                        });
                    }
                    ScenePlaceholder::Logo => {
                        if self.logo_png.is_some() {
                            logo_scenes.push(LogoScene {
                                x_offset: rendered.x_offset,
                                alpha: rendered.alpha,
                                with_caption: false,
                            });
                        }
                    }
                    ScenePlaceholder::LogoWithCaption => {
                        if self.logo_png.is_some() {
                            logo_scenes.push(LogoScene {
                                x_offset: rendered.x_offset,
                                alpha: rendered.alpha,
                                with_caption: true,
                            });
                        }
                    }
                }
            }
        }

        // Load the overlay textures on demand and drop them when unused, so the
        // GPU only holds them while a "no content yet" scene is on screen.
        let needs_logo = !logo_scenes.is_empty();
        let needs_text = logo_scenes.iter().any(|s| s.with_caption);
        if needs_logo && let Some(png) = self.logo_png {
            self.logo_texture
                .ensure_loaded(self.egl.renderer(), png, "DECK logo");
        } else if !needs_logo {
            self.logo_texture.unload();
        }
        if needs_text {
            self.loading_text_texture.ensure_loaded(
                self.egl.renderer(),
                LOADING_SCENE_TEXT_PNG,
                "loading text",
            );
        } else {
            self.loading_text_texture.unload();
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
        for (buffer_id, placement, x_offset, alpha) in &to_render {
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

            renderable_items.push((
                buffer_id.clone(),
                placement.instance_id.clone(),
                dst,
                *alpha,
            ));
        }

        // Only opaque widgets hide the stale back buffer beneath them, so
        // translucent ones (mid cross-fade) don't count as coverage — the
        // clear must reach under them or the previous frame ghosts through.
        let drawn_regions: Vec<_> = renderable_items
            .iter()
            .filter(|(_, _, _, alpha)| *alpha >= 1.0)
            .map(|(_, _, dst, _)| *dst)
            .collect();

        // Clear regions not covered by any opaque widget before drawing widgets.
        // Clearing after widget draws can overpaint widget content on the
        // target hardware when the clear path and rotated texture path mix.
        let clear_regions = uncovered_output_regions(output_rect, drawn_regions);

        if !clear_regions.is_empty() {
            frame
                .clear(BACKGROUND_COLOR, &clear_regions)
                .context("Failed to clear uncovered output regions")?;
        }

        // Neither the logo/caption nor the separator draws below report into
        // `damage_rects`; see the assert next to `draw_separator_grids`.
        draw_logo_scenes(
            &mut frame,
            &self.output,
            self.scanout_transform,
            &logo_scenes,
            self.logo_texture.texture(),
            self.loading_text_texture.texture(),
        );

        // Drawn over the cleared background and before the widgets, for the
        // same reason as the clear above.
        draw_separator_grids(
            &mut frame,
            &self.output,
            self.scanout_transform,
            &combined_scene_grids,
        );

        for (buffer_id, instance_id, dst, alpha) in &renderable_items {
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
                *alpha,
                None,
                &[],
            ) {
                tracing::warn!("Failed to render widget {}: {:?}", instance_id, e);
            }
        }

        #[expect(clippy::cast_possible_wrap, reason = "output dims are within i32")]
        let (output_w, output_h) = (self.output.width() as i32, self.output.height() as i32);
        for (buffer, geo) in layers {
            let Some(texture) = self.texture_cache.get(&buffer.id()) else {
                continue;
            };
            blit_texture_on_frame(
                &mut frame,
                texture,
                geo.loc.x,
                geo.loc.y,
                Some(geo.size),
                output_w,
                output_h,
                self.scanout_transform,
                1.0,
                "layer surface",
            );
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
        self.egl.wait_for_rendering_completion()?;

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

/// Collect visible widgets from a scene into the render list
/// with the scene's x offset and opacity.
fn collect_scene_widgets(
    scene: &bmc::compositor::SceneLayout,
    buffers: &[(WlBuffer, bmc::compositor::InstanceId)],
    x_offset: i32,
    alpha: f32,
    out: &mut Vec<(ObjectId, bmc::compositor::WidgetPlacement, i32, f32)>,
) {
    for (client_buffer, instance_id) in buffers {
        if let Some(placement) = scene
            .widgets
            .iter()
            .find(|w| &w.instance_id == instance_id && w.visible)
        {
            out.push((client_buffer.id(), placement.clone(), x_offset, alpha));
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
