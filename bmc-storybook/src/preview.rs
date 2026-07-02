// Copyright (C) 2026  Braiins Systems s.r.o.

//! Preview pane: renders SDK tree nodes through bmc-render's FemtoVG pipeline.
//!
//! Each `DocBlock::Frame` gets its own femtovg offscreen render target via
//! `create_render_target` / `begin_frame_to_image`. A bootstrap FBO is needed
//! only for `FemtoVgRenderer` initialization.

use std::collections::HashMap;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::interaction::InteractionState;
use bmc_render::renderer::Renderer;
use bmc_render::tree::{NodeContext, TouchHit};
use bmc_render::{AnimationState, ModalState, ScrollState, TransitionState, TransitionStateKey};
use bmc_storybook_api::{DocBlock, FrameSize};
use taffy::TaffyTree;

/// Bootstrap FBO dimensions — only used for `FemtoVgRenderer::new()`.
const BOOTSTRAP_W: u32 = 64;
const BOOTSTRAP_H: u32 = 64;

// ── Bootstrap FBO ───────────────────────────────────────────────────

/// Minimal FBO for `FemtoVgRenderer` initialization.
/// Not used for actual rendering — each frame gets its own offscreen image.
pub struct BootstrapFbo {
    pub width: u32,
    pub height: u32,
    fbo: eframe::glow::Framebuffer,
}

impl BootstrapFbo {
    #[expect(unsafe_code, clippy::cast_possible_wrap)]
    pub fn new(gl: &eframe::glow::Context) -> Self {
        use eframe::glow::HasContext;
        unsafe {
            let texture = gl
                .create_texture()
                .expect("BUG: failed to create GL texture");
            gl.bind_texture(eframe::glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                eframe::glow::TEXTURE_2D,
                0,
                eframe::glow::SRGB8_ALPHA8 as i32,
                BOOTSTRAP_W as i32,
                BOOTSTRAP_H as i32,
                0,
                eframe::glow::RGBA,
                eframe::glow::UNSIGNED_BYTE,
                eframe::glow::PixelUnpackData::Slice(None),
            );
            let fbo = gl
                .create_framebuffer()
                .expect("BUG: failed to create GL framebuffer");
            gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                eframe::glow::FRAMEBUFFER,
                eframe::glow::COLOR_ATTACHMENT0,
                eframe::glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            let rbo = gl
                .create_renderbuffer()
                .expect("BUG: failed to create GL renderbuffer");
            gl.bind_renderbuffer(eframe::glow::RENDERBUFFER, Some(rbo));
            gl.renderbuffer_storage(
                eframe::glow::RENDERBUFFER,
                eframe::glow::DEPTH24_STENCIL8,
                BOOTSTRAP_W as i32,
                BOOTSTRAP_H as i32,
            );
            gl.framebuffer_renderbuffer(
                eframe::glow::FRAMEBUFFER,
                eframe::glow::DEPTH_STENCIL_ATTACHMENT,
                eframe::glow::RENDERBUFFER,
                Some(rbo),
            );
            gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, None);
            gl.bind_texture(eframe::glow::TEXTURE_2D, None);
            gl.bind_renderbuffer(eframe::glow::RENDERBUFFER, None);

            Self {
                width: BOOTSTRAP_W,
                height: BOOTSTRAP_H,
                fbo,
            }
        }
    }

    pub fn fbo_id(&self) -> u32 {
        self.fbo.0.get()
    }
}

// ── Per-frame rendering state ───────────────────────────────────────

/// State for a single rendered frame.
pub struct FrameState {
    pub interaction: InteractionState,
    pub modal_states: HashMap<String, ModalState>,
    pub scroll_states: HashMap<String, ScrollState>,
    pub animation_states: HashMap<u64, AnimationState>,
    pub transition_states: HashMap<TransitionStateKey, TransitionState>,
    pub taffy: TaffyTree<NodeContext>,
    pub frame_counter: u64,
    pub content_size: (f32, f32),
    pub drags: HashMap<String, TouchHit>,
    /// `true` between `Down` and the matching `Up` — the frame "owns" the
    /// gesture and must receive subsequent `Move`/`Up` events even when the
    /// pointer leaves its display rect (otherwise sliders/drags get stuck
    /// in pressed state when the user drags past the frame edge).
    pub pointer_captured: bool,
}

impl FrameState {
    fn new() -> Self {
        Self {
            interaction: InteractionState::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            taffy: TaffyTree::new(),
            frame_counter: 0,
            content_size: (0.0, 0.0),
            drags: HashMap::new(),
            pointer_captured: false,
        }
    }
}

/// A femtovg offscreen render target for one frame.
pub struct FrameRenderTarget {
    pub width: u32,
    pub height: u32,
    pub image_id: femtovg::ImageId,
    pub egui_texture_id: Option<egui::TextureId>,
    pub state: FrameState,
    pub frame_size: FrameSize,
}

/// Rendered frame output.
pub struct RenderedFrame {
    pub target_idx: usize,
    pub content_size: (f32, f32),
}

/// Document-level renderer with per-frame offscreen images.
pub struct DocumentRenderer {
    pub renderer: FemtoVgRenderer,
    /// femtovg offscreen render targets, one per unique `(width, height)`
    /// ever rendered in this process — reused on subsequent renders of
    /// the same size, never evicted.
    ///
    /// Eviction is deliberately not implemented: `eframe::Frame` (0.34)
    /// exposes `register_native_glow_texture` but no companion
    /// `free_native_glow_texture`, so calling femtovg's `delete_image`
    /// would leave a dangling `egui::TextureId` registered with the
    /// painter — strictly worse than retaining both. Bound in practice
    /// by the small set of frame sizes a storybook session navigates
    /// through.
    pub targets: Vec<FrameRenderTarget>,
    pub rendered_frames: Vec<RenderedFrame>,
    /// Monotonic elapsed ms (sum of `delta_ms`) — the storybook's advancing clock.
    elapsed_ms: u64,
}

impl DocumentRenderer {
    pub fn new(renderer: FemtoVgRenderer) -> Self {
        Self {
            renderer,
            targets: Vec::new(),
            rendered_frames: Vec::new(),
            elapsed_ms: 0,
        }
    }

    /// Reset all animation and transition states across all frame targets.
    pub fn reset_animation_states(&mut self) {
        for target in &mut self.targets {
            target.state.animation_states.clear();
            target.state.transition_states.clear();
            target.state.frame_counter = 0;
        }
    }

    /// Render all Frame/CustomRender blocks into per-frame offscreen images.
    ///
    /// Walks the block tree recursively (into `Grid` blocks) to find all
    /// renderable frames. Each frame gets its own FBO and `InteractionState`.
    #[expect(unsafe_code)]
    pub fn render_doc_blocks(
        &mut self,
        blocks: &mut [DocBlock],
        gl: &eframe::glow::Context,
        egui_frame: &mut eframe::Frame,
        delta_ms: u32,
    ) {
        use eframe::glow::HasContext;

        self.rendered_frames.clear();

        // Advance the storybook clock by this frame's delta (frozen while paused,
        // when delta_ms is 0) so `RelativeTimeLive` labels tick like on-device.
        self.elapsed_ms = self.elapsed_ms.saturating_add(u64::from(delta_ms));
        #[expect(clippy::integer_division, reason = "ms to whole seconds")]
        let now_unix_secs = i64::try_from(self.elapsed_ms / 1_000).unwrap_or(i64::MAX);

        // Collect frame sizes for FBO allocation (no mutable access needed).
        let frame_sizes: Vec<FrameSize> = collect_frame_sizes(blocks);

        // Match/allocate per-frame offscreen images.
        let mut used = vec![false; self.targets.len()];
        let mut target_assignments: Vec<usize> = Vec::with_capacity(frame_sizes.len());

        for size in &frame_sizes {
            let w = size.width();
            let h = size.fbo_height();
            let target_idx = self
                .targets
                .iter()
                .enumerate()
                .position(|(i, t)| !used[i] && t.width == w && t.height == h);
            let target_idx = if let Some(idx) = target_idx {
                used[idx] = true;
                self.targets[idx].frame_size = *size;
                idx
            } else {
                let (image_id, gl_name) = self.renderer.create_render_target(w, h);
                let native = eframe::glow::NativeTexture(
                    std::num::NonZeroU32::new(gl_name).expect("BUG: GL texture name is zero"),
                );
                let tex_id = egui_frame.register_native_glow_texture(native);
                let idx = self.targets.len();
                self.targets.push(FrameRenderTarget {
                    width: w,
                    height: h,
                    image_id,
                    egui_texture_id: Some(tex_id),
                    state: FrameState::new(),
                    frame_size: *size,
                });
                used.push(true);
                idx
            };
            target_assignments.push(target_idx);
        }

        let srgb_was_enabled = unsafe { gl.is_enabled(eframe::glow::FRAMEBUFFER_SRGB) };
        unsafe { gl.disable(eframe::glow::FRAMEBUFFER_SRGB) };

        // Render each frame. We walk the block tree again in the same order
        // as collect_frame_sizes so frame_idx stays in sync.
        let mut frame_idx = 0;
        render_frames_recursive(
            blocks,
            &target_assignments,
            &mut frame_idx,
            &mut self.renderer,
            &mut self.targets,
            &mut self.rendered_frames,
            delta_ms,
            now_unix_secs,
        );
        // Catch tree-mutation drift between the immutable `collect_frame_sizes`
        // walk and the `&mut` render walk: every assignment must have been
        // consumed exactly once.
        debug_assert_eq!(
            frame_idx,
            target_assignments.len(),
            "frame_idx desynced from target_assignments — block tree mutated mid-render?"
        );

        // Switch to screen and flush — the flush resolves the offscreen
        // image's render target to its GL texture (needed for egui display).
        self.renderer.set_render_target_screen();
        self.renderer.flush();
        unsafe {
            gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, None);
            if srgb_was_enabled {
                gl.enable(eframe::glow::FRAMEBUFFER_SRGB);
            }
        }
    }
}

/// Collect frame sizes in tree order (recursive into Grid blocks).
fn collect_frame_sizes(blocks: &[DocBlock]) -> Vec<FrameSize> {
    let mut out = Vec::new();
    collect_frame_sizes_rec(blocks, &mut out);
    out
}

fn collect_frame_sizes_rec(blocks: &[DocBlock], out: &mut Vec<FrameSize>) {
    for block in blocks {
        match block {
            DocBlock::Frame { size, .. } | DocBlock::CustomRender { size, .. } => {
                out.push(*size);
            }
            DocBlock::Grid { cells, .. } => {
                for cell in cells {
                    collect_frame_sizes_rec(cell, out);
                }
            }
            DocBlock::Header { .. }
            | DocBlock::Code { .. }
            | DocBlock::Prose { .. }
            | DocBlock::Divider => {}
        }
    }
}

/// Render frames in tree order, matching the order of `collect_frame_sizes`.
#[expect(clippy::too_many_arguments, reason = "recursive frame walk threads render state + clock")]
fn render_frames_recursive(
    blocks: &mut [DocBlock],
    target_assignments: &[usize],
    frame_idx: &mut usize,
    renderer: &mut FemtoVgRenderer,
    targets: &mut [FrameRenderTarget],
    rendered_frames: &mut Vec<RenderedFrame>,
    delta_ms: u32,
    now_unix_secs: i64,
) {
    for block in blocks {
        match block {
            DocBlock::Frame { node, .. } => {
                let target_idx = target_assignments[*frame_idx];
                let target = &mut targets[target_idx];
                let size = target.frame_size;

                renderer.begin_frame_to_image(target.image_id, target.width, target.height, 1.0);
                target.state.interaction.begin_frame();

                let bytes = bmc_wasm_sdk::tree::serialize_node_to_bytes(node);
                let mut ctx = bmc_render::ProcessContext {
                    interaction: &mut target.state.interaction,
                    modal_states: &mut target.state.modal_states,
                    scroll_states: &mut target.state.scroll_states,
                    animation_states: &mut target.state.animation_states,
                    transition_states: &mut target.state.transition_states,
                    taffy: &mut target.state.taffy,
                    frame_counter: target.state.frame_counter,
                    delta_ms,
                    now_unix_secs,
                };
                match bmc_render::process_tree(
                    &bytes,
                    size.layout_width(),
                    size.layout_height(),
                    renderer,
                    &mut ctx,
                ) {
                    Ok((_tree, result, _has_active, _timings)) => {
                        target.state.content_size = result.content_size;
                        target.state.drags = result.drags;
                    }
                    Err(e) => tracing::error!("process_tree failed: {e}"),
                }

                renderer.flush();
                if delta_ms > 0 {
                    target.state.frame_counter += 1;
                }
                rendered_frames.push(RenderedFrame {
                    target_idx,
                    content_size: target.state.content_size,
                });
                *frame_idx += 1;
            }
            DocBlock::CustomRender { render_fn, .. } => {
                let target_idx = target_assignments[*frame_idx];
                let target = &mut targets[target_idx];
                let size = target.frame_size;

                renderer.begin_frame_to_image(target.image_id, target.width, target.height, 1.0);
                target.state.interaction.begin_frame();

                #[expect(clippy::cast_precision_loss)]
                let (w, h) = (size.width() as f32, size.layout_height());
                render_fn(renderer, &mut target.state.interaction, w, h, delta_ms);
                target.state.content_size = (w, h);

                renderer.flush();
                if delta_ms > 0 {
                    target.state.frame_counter += 1;
                }
                rendered_frames.push(RenderedFrame {
                    target_idx,
                    content_size: target.state.content_size,
                });
                *frame_idx += 1;
            }
            DocBlock::Grid { cells, .. } => {
                for cell in cells {
                    render_frames_recursive(
                        cell,
                        target_assignments,
                        frame_idx,
                        renderer,
                        targets,
                        rendered_frames,
                        delta_ms,
                        now_unix_secs,
                    );
                }
            }
            DocBlock::Header { .. }
            | DocBlock::Code { .. }
            | DocBlock::Prose { .. }
            | DocBlock::Divider => {}
        }
    }
}
