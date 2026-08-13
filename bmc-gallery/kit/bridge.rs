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

//! Renders SDK trees through bmc-render's femtovg pipeline onto a gallery stage.
//!
//! gallery owns the framebuffer and its chrome; this module draws into a femtovg
//! image of its own and copies that across. Renderer and layout state live in
//! thread-locals, because the SDK's asset registrars are bare `fn` pointers that
//! cannot capture — the single-render-thread contract the SDK has everywhere.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CString;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::interaction::{ActionEvent, InteractionState, TouchEvent};
use bmc_render::renderer::Renderer;
use bmc_render::tree::{NodeContext, TouchHit};
use bmc_render::{
    AnimationState, ModalState, ProcessContext, ScrollState, TransitionState, TransitionStateKey,
};
use bmc_wasm_sdk::tree::Node;
// eframe's glow, not the one femtovg links: only raw GL names cross between the
// two, so the versions never have to agree.
use gallery::eframe::egui::{Id, Ui};
use gallery::eframe::glow::{self, HasContext as _};
use gallery::{ImageInput, Pointer, SceneCtx, Stage, StageTexture};
use taffy::TaffyTree;

use super::size::{AUTO_HEIGHT_MAX, DEVICE_HEIGHT, DeckSize, DivHeight};

/// Callback for frames that draw straight onto the renderer instead of building
/// a tree. Arguments: `(renderer, interaction, width, height, delta_ms)`.
///
/// Returns whether the frame is still moving. A stage that always says it is
/// keeps the window repainting for as long as the scene is open, and never lets
/// a capture settle.
pub type CustomRenderFn =
    Box<dyn FnMut(&mut dyn Renderer, &mut InteractionState, f32, f32, u32) -> bool>;

thread_local! {
    /// Boxed so the pointer the registrars hold stays valid as the option moves.
    static RENDERER: RefCell<Option<Box<FemtoVgRenderer>>> = const { RefCell::new(None) };
    static RENDERER_PTR: Cell<*mut FemtoVgRenderer> = const { Cell::new(std::ptr::null_mut()) };
    /// Ours only to put the framebuffer back the way femtovg found it: it binds
    /// the image it draws into and leaves it bound, and everything egui paints
    /// after a stage would land there instead of on the window.
    static GL: RefCell<Option<glow::Context>> = const { RefCell::new(None) };
    /// Keyed by egui's id for the call site that staged it, so a scene's stages
    /// keep their scroll and animation apart — and keep it across a stage the
    /// scene only sometimes shows, which moves every framebuffer after it.
    static FRAMES: RefCell<HashMap<Id, FrameState>> = RefCell::new(HashMap::new());
    /// Last measured height per call site, read before a target exists.
    /// A first frame has none, starts from the device's, and settles after.
    static HEIGHTS: RefCell<HashMap<Id, usize>> = RefCell::new(HashMap::new());
}

/// A stage's femtovg render target: the image bmc-render draws into, and the
/// texture behind it that the blit reads.
#[derive(Clone, Copy)]
struct Image {
    id: femtovg::ImageId,
    texture: glow::NativeTexture,
    size: [u32; 2],
}

impl Image {
    fn new(renderer: &mut FemtoVgRenderer, size: [u32; 2]) -> Self {
        let (id, name) = renderer.create_render_target(size[0], size[1]);
        Self {
            id,
            texture: glow::NativeTexture(
                std::num::NonZeroU32::new(name).expect("BUG: GL texture name is zero"),
            ),
            size,
        }
    }
}

// ── Asset registrars ────────────────────────────────────────────────
//
// They fire while a scene builds its nodes and re-entrantly from canvas draw
// closures, so the pointer stays set rather than bracketing each render.

macro_rules! registrar {
    ($name:ident, $method:ident -> $id:ty) => {
        fn $name(tag: &str, data: &[u8]) -> Option<$id> {
            let ptr = RENDERER_PTR.with(Cell::get);
            assert!(
                !ptr.is_null(),
                "asset registered before the first Deck stage, or from a spawned thread; \
                 scenes must register assets on the render thread"
            );
            // SAFETY: the pointer is the boxed renderer in this thread's RENDERER,
            // which outlives every scene call; registrars only run on this thread.
            unsafe { &mut *ptr }.$method(tag, data)
        }
    };
}

registrar!(registrar_icon, register_svg -> bmc_wasm_sdk::SvgId);
registrar!(registrar_bitmap, register_bitmap -> bmc_wasm_sdk::BitmapId);
registrar!(registrar_mesh, register_mesh -> bmc_wasm_sdk::MeshId);

fn registrar_bitmap_nearest(
    tag: &str,
    source: bmc_wasm_sdk::StaticAssetSource,
) -> Option<bmc_wasm_sdk::BitmapId> {
    let ptr = RENDERER_PTR.with(Cell::get);
    assert!(
        !ptr.is_null(),
        "asset registered before the first Deck stage, or from a spawned thread; \
         scenes must register assets on the render thread"
    );
    // SAFETY: the pointer is the boxed renderer in this thread's RENDERER,
    // which outlives every scene call; registrars only run on this thread.
    unsafe { &mut *ptr }.register_bitmap_nearest(tag, source.data())
}

/// Uploads already-decoded pixels, so it takes their size
/// where the others take encoded bytes — outside what [`registrar`] shapes.
fn registrar_image_rgba(
    tag: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Option<bmc_wasm_sdk::BitmapId> {
    let ptr = RENDERER_PTR.with(Cell::get);
    assert!(
        !ptr.is_null(),
        "asset registered before the first Deck stage, or from a spawned thread; \
         scenes must register assets on the render thread"
    );
    // SAFETY: as above — this thread's boxed renderer, on this thread only.
    unsafe { &mut *ptr }.register_bitmap_rgba(tag, rgba, width, height)
}

/// The layout and interaction state one staged frame keeps between frames,
/// keyed by stage identity so a scene's stages don't share scroll or animation
/// positions.
struct FrameState {
    interaction: InteractionState,
    modal_states: HashMap<String, ModalState>,
    scroll_states: HashMap<String, ScrollState>,
    animation_states: HashMap<u64, AnimationState>,
    transition_states: HashMap<TransitionStateKey, TransitionState>,
    taffy: TaffyTree<NodeContext>,
    frame_counter: u64,
    elapsed_ms: u64,
    /// egui runs a frame in several passes when a widget asks to be laid out
    /// again, and each pass draws the stage afresh. Only the first carries time:
    /// the rest would advance an animation the viewer never saw a frame of.
    last_frame: Option<u64>,
    /// Also what a browsed-past stage is dropped on: each one holds an image
    /// the GPU keeps until the entry goes.
    last_seen: u64,
    drags: HashMap<String, TouchHit>,
    /// This stage's own render target, so several stages in a scene don't
    /// overwrite each other — femtovg's "screen" can only ever name one FBO.
    image: Option<Image>,
    /// `image`'s texture as egui knows it, registered once and dropped with it.
    texture: Option<gallery::eframe::egui::TextureId>,
}

impl Default for FrameState {
    fn default() -> Self {
        Self {
            interaction: InteractionState::new(),
            modal_states: HashMap::new(),
            scroll_states: HashMap::new(),
            animation_states: HashMap::new(),
            transition_states: HashMap::new(),
            taffy: TaffyTree::new(),
            frame_counter: 0,
            elapsed_ms: 0,
            last_frame: None,
            last_seen: 0,
            drags: HashMap::new(),
            image: None,
            texture: None,
        }
    }
}

/// Build the renderer and wire the SDK registrars to it, once per session.
///
/// The FBO handed to `FemtoVgRenderer::new` is never rendered into: femtovg 0.20.4
/// pins its "screen" target at construction and `Canvas` owns the renderer privately,
/// so every stage draws into an image of its own instead — see [`draw_frame`].
fn init_renderer_with(loader: &gallery::GlLoader, [width, height]: [u32; 2], fbo: u32) {
    let loader = loader.clone();
    let mut renderer = Box::new(
        // SAFETY: `loader` resolves against the context gallery made current for this draw.
        unsafe {
            FemtoVgRenderer::new(
                |name| {
                    let cstr = CString::new(name).expect("BUG: GL function name has a null byte");
                    loader(&cstr)
                },
                width,
                height,
                fbo,
                0,
            )
            .expect("BUG: failed to create the FemtoVG renderer")
        },
    );
    RENDERER_PTR.with(|p| p.set(&raw mut *renderer));
    RENDERER.set(Some(renderer));
    // SAFETY: same loader, same context femtovg just bound itself to.
    GL.set(Some(unsafe {
        glow::Context::from_loader_function_cstr(|symbol| loader(symbol))
    }));

    bmc_wasm_sdk::assets::init_icon_registrar(registrar_icon);
    bmc_wasm_sdk::assets::init_bitmap_registrar(registrar_bitmap);
    bmc_wasm_sdk::assets::init_mesh_registrar(registrar_mesh);
    bmc_wasm_sdk::assets::init_image_registrar(registrar_image_rgba);
    bmc_render_skin::init(registrar_bitmap_nearest);
}

/// Which egui frame a stage is being drawn for, and how much time it carries.
#[derive(Clone, Copy)]
struct Frame {
    number: u64,
    delta_ms: u32,
}

/// Stages a session has browsed past, dropped once this many frames have gone by
/// without drawing them. Each holds a femtovg image the GPU keeps until it goes,
/// and a catalogue is a few hundred stages.
const EVICT_AFTER_FRAMES: u64 = 600;

/// Drop what has not been drawn lately, freeing its image. The `TextureId` goes
/// with it unfreed: egui hands out no way to take one back.
fn evict_stale(renderer: &mut FemtoVgRenderer, frames: &mut HashMap<Id, FrameState>, now: u64) {
    frames.retain(|_, state| {
        if now.saturating_sub(state.last_seen) < EVICT_AFTER_FRAMES {
            return true;
        }
        if let Some(image) = state.image.take() {
            renderer.delete_image(image.id);
        }
        false
    });
}

/// Draw one frame into this stage's own femtovg image and return the content
/// size the tree laid out to. Gallery composites the image itself, so nothing
/// is copied out of it.
#[expect(
    clippy::integer_division,
    clippy::cast_sign_loss,
    reason = "elapsed milliseconds to whole seconds, and a GL framebuffer name \
              that the query returns as a non-negative i32"
)]
fn draw_frame(
    state: &mut FrameState,
    target: [u32; 2],
    frame: Frame,
    draw: impl FnOnce(&mut FemtoVgRenderer, &mut FrameState, u32, i64) -> ((f32, f32), bool),
) -> ((f32, f32), bool) {
    // A later pass of a frame already drawn redraws it as it stands: the tree can
    // differ from the first pass, but no time has gone by for it to move through.
    let repeat = state.last_frame == Some(frame.number);
    state.last_frame = Some(frame.number);
    state.last_seen = frame.number;
    let delta_ms = if repeat { 0 } else { frame.delta_ms };
    // Whatever egui was drawing into, to hand back below.
    let bound = GL.with_borrow(|gl| {
        let gl = gl.as_ref().expect("BUG: GL set beside the renderer");
        // SAFETY: reading a binding from the context gallery made current.
        unsafe { gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING) }
    });
    let content = RENDERER.with_borrow_mut(|slot| {
        let renderer = slot.as_mut().expect("BUG: renderer initialised above");
        // The target only ever grows, so a stage that measures taller than it did
        // reallocates and re-registers; egui frees no texture it was handed, which
        // is why this shrinks for nobody.
        let alloc = state.image.map_or(target, |held| {
            [held.size[0].max(target[0]), held.size[1].max(target[1])]
        });
        if state.image.is_none_or(|held| held.size != alloc) {
            let fresh = Image::new(renderer, alloc);
            if let Some(stale) = state.image.replace(fresh) {
                renderer.delete_image(stale.id);
            }
            state.texture = None;
        }
        let image = state.image.expect("BUG: just ensured present");

        // Cleared transparent, so the stage shows through what the widget does
        // not cover — the base the device's composited overlays get too.
        renderer.begin_frame_to_image(image.id, alloc[0], alloc[1], 1.0);
        state.interaction.begin_frame();

        state.elapsed_ms = state.elapsed_ms.saturating_add(u64::from(delta_ms));
        let now_unix_secs = i64::try_from(state.elapsed_ms / 1_000).unwrap_or(i64::MAX);
        let content = draw(renderer, state, delta_ms, now_unix_secs);

        renderer.flush();
        // Every frame is a new one to bmc-render: the paragraph cache and the
        // animation states key off this, and reusing a number replays the last
        // layout over the new one.
        if !repeat {
            state.frame_counter += 1;
        }
        content
    });
    GL.with_borrow(|gl| {
        let gl = gl.as_ref().expect("BUG: GL set beside the renderer");
        // SAFETY: rebinding what we read above, on the same live context.
        unsafe {
            gl.bind_framebuffer(
                glow::FRAMEBUFFER,
                std::num::NonZeroU32::new(bound as u32).map(glow::NativeFramebuffer),
            );
        }
    });
    content
}

// ── Scene-facing API ────────────────────────────────────────────────

/// What the widgets in a stage reported this frame.
///
/// A scene decides what to do with it: log to gallery's Actions panel with
/// [`gallery::action`], write a knob back with `ctx.set_*`, or ignore it.
#[derive(Debug, Default)]
pub struct Fired {
    /// Taps and scrolls that landed on a keyed element.
    pub actions: Vec<ActionEvent>,
    /// Drags in progress, by element key — `x`/`width` give a slider's fraction.
    pub drags: Vec<(String, TouchHit)>,
}

impl Fired {
    #[must_use]
    pub fn clicked(&self, key: &str) -> bool {
        self.actions
            .iter()
            .any(|event| matches!(event, ActionEvent::Click { key: k, .. } if k == key))
    }

    /// Where `key` is being dragged, as a 0..1 fraction of its width.
    #[must_use]
    pub fn dragged(&self, key: &str) -> Option<f32> {
        self.drags
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, hit)| (hit.x / hit.width).clamp(0.0, 1.0))
    }
}

/// Device-sized femtovg stages — the Deck counterpart of
/// [`stage!`](gallery::stage) for content that renders through bmc-render
/// rather than egui.
pub trait DeckSceneCtx {
    /// Stage an SDK tree at a device size. `build` runs with the asset
    /// registrars live, so `ensure_*_registered` works while building nodes.
    fn node_stage(&mut self, ui: &mut Ui, size: impl Into<DeckSize>, build: impl FnOnce() -> Node);

    /// Stage a frame drawn straight onto the renderer — see [`CustomRenderFn`].
    fn custom_stage(&mut self, ui: &mut Ui, size: impl Into<DeckSize>, render: CustomRenderFn);

    /// [`node_stage`](Self::node_stage) for a widget the viewer can touch:
    /// the frame takes the pointer and the wheel, so neither reaches the canvas
    /// behind it — a scene that only wants to be looked at leaves them alone
    /// and keeps the canvas scrollable.
    fn node_stage_input(
        &mut self,
        ui: &mut Ui,
        size: impl Into<DeckSize>,
        build: impl FnOnce() -> Node,
    ) -> Fired;

    /// [`custom_stage`](Self::custom_stage) taking the pointer and the wheel,
    /// as [`node_stage_input`](Self::node_stage_input) does.
    fn custom_stage_input(
        &mut self,
        ui: &mut Ui,
        size: impl Into<DeckSize>,
        render: CustomRenderFn,
    ) -> Fired;
}

/// Whether a stage claims the pointer and the wheel, or leaves them to the canvas.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    Yes,
    No,
}

impl DeckSceneCtx for SceneCtx<'_> {
    fn node_stage(&mut self, ui: &mut Ui, size: impl Into<DeckSize>, build: impl FnOnce() -> Node) {
        staged_tree(self, ui, size.into(), build, Input::No);
    }

    fn custom_stage(&mut self, ui: &mut Ui, size: impl Into<DeckSize>, render: CustomRenderFn) {
        staged_custom(self, ui, size.into(), render, Input::No);
    }

    fn node_stage_input(
        &mut self,
        ui: &mut Ui,
        size: impl Into<DeckSize>,
        build: impl FnOnce() -> Node,
    ) -> Fired {
        staged_tree(self, ui, size.into(), build, Input::Yes)
    }

    fn custom_stage_input(
        &mut self,
        ui: &mut Ui,
        size: impl Into<DeckSize>,
        render: CustomRenderFn,
    ) -> Fired {
        staged_custom(self, ui, size.into(), render, Input::Yes)
    }
}

fn staged_tree(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    size: DeckSize,
    build: impl FnOnce() -> Node,
    input: Input,
) -> Fired {
    deck_stage(
        ctx,
        ui,
        size,
        input,
        |renderer, state, delta_ms, now_unix_secs| {
            let bytes = bmc_wasm_sdk::tree::serialize_node_to_bytes(&build());
            let mut ctx = ProcessContext {
                interaction: &mut state.interaction,
                modal_states: &mut state.modal_states,
                scroll_states: &mut state.scroll_states,
                animation_states: &mut state.animation_states,
                transition_states: &mut state.transition_states,
                taffy: &mut state.taffy,
                frame_counter: state.frame_counter,
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
                Ok((_tree, result, has_active, _timings)) => {
                    state.drags = result.drags;
                    (result.content_size, has_active)
                }
                Err(e) => {
                    tracing::error!("process_tree failed: {e}");
                    ((size.layout_width(), size.layout_height()), false)
                }
            }
        },
    )
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a device width is a small positive integer"
)]
fn staged_custom(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    size: DeckSize,
    mut render: CustomRenderFn,
    input: Input,
) -> Fired {
    deck_stage(ctx, ui, size, input, |renderer, state, delta_ms, _now| {
        let (width, height) = (size.width() as f32, size.layout_height());
        let animating = render(renderer, &mut state.interaction, width, height, delta_ms);
        ((width, height), animating)
    })
}

/// The shared flow: gallery's staged offscreen owns the target and its chrome;
/// this fills it. Round faces are masked to their inscribed circle over the
/// image gallery drew.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "device sizes and measured heights are small positive values, and \
              each is clamped to its bound before the cast"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one linear pass over a frame: size, target, draw, measure, present"
)]
fn deck_stage(
    ctx: &mut SceneCtx,
    ui: &mut Ui,
    size: DeckSize,
    input: Input,
    draw: impl FnOnce(&mut FemtoVgRenderer, &mut FrameState, u32, i64) -> ((f32, f32), bool),
) -> Fired {
    // egui's own id for this call site: what gallery keys the stage's target on
    // too, and stable where a framebuffer name is a fact about an allocation.
    let key = ui.next_auto_id();
    let frame = Frame {
        // The frame, not the pass: `cumulative_pass_nr` counts several to a frame.
        // The plain call reads the current viewport, unlike `_for`, which panics
        // in a debug build on the perf window's.
        number: ui.ctx().cumulative_frame_nr(),
        // Capped so a stall doesn't jump every animation to its end.
        delta_ms: (ui.input(|i| i.stable_dt) * 1_000.0).clamp(0.0, 100.0) as u32,
    };

    // A content-driven frame is staged at what it last laid out to, starting from
    // a device screen's worth; it settles on the frame after the first.
    let height = match size.div_height() {
        DivHeight::Px(height) => height,
        DivHeight::Auto => HEIGHTS
            .with_borrow(|heights| heights.get(&key).copied())
            .unwrap_or(DEVICE_HEIGHT)
            .clamp(1, AUTO_HEIGHT_MAX),
    };
    let target = [size.width() as u32, height as u32];

    if RENDERER.with_borrow(Option::is_none) {
        let loader = ctx
            .gl_loader()
            .expect("BUG: the Deck kit renders through the glow backend");
        // The FBO is inert here — see `init_renderer`.
        init_renderer_with(&loader, target, 0);
    }
    let mut grew = false;
    let (content, animating, fired, allocated) = FRAMES.with_borrow_mut(|frames| {
        let state = frames.entry(key).or_default();
        let (content, animating) = draw_frame(state, target, frame, draw);
        // What it lays out in, as opposed to what it shows: content past the
        // target is cut, so a stage that measures taller comes back bigger.
        // A custom render states no height and keeps the one it has.
        if matches!(size.div_height(), DivHeight::Auto) && content.1 >= 1.0 {
            let measured = (content.1.ceil() as usize).min(AUTO_HEIGHT_MAX);
            HEIGHTS.with_borrow_mut(|heights| {
                grew = heights.insert(key, measured) != Some(measured);
            });
        }
        let fired = Fired {
            actions: std::mem::take(&mut state.interaction.action_log),
            drags: state
                .drags
                .iter()
                .map(|(key, hit)| (key.clone(), *hit))
                .collect(),
        };
        let allocated = state.image.expect("BUG: drawn just above").size;
        RENDERER.with_borrow_mut(|slot| {
            let renderer = slot.as_mut().expect("BUG: renderer initialised above");
            evict_stale(renderer, frames, frame.number);
        });
        (content, animating, fired, allocated)
    });

    // A content-driven axis is shown at what it just laid out to, in the frame it
    // measured — the target it was drawn into only has to be at least that big.
    // A stage that measures narrower than the device would otherwise present
    // device-wide, its content against the left edge.
    let shown = [
        if size.is_auto_width() {
            (content.0.ceil() as u32).clamp(1, target[0])
        } else {
            target[0]
        },
        match size.div_height() {
            DivHeight::Px(height) => height as u32,
            DivHeight::Auto => (content.1.ceil() as u32).clamp(1, AUTO_HEIGHT_MAX as u32),
        },
    ];
    // Registered the once, outside the borrow above: this needs `ctx`, which also
    // owns the renderer that just drew.
    let texture = FRAMES.with_borrow_mut(|frames| {
        let state = frames.get_mut(&key).expect("BUG: just drawn");
        let name = state.image.expect("BUG: drawn just above").texture.0;
        *state
            .texture
            .get_or_insert_with(|| ctx.register_native_texture(name))
    });

    let adopted = StageTexture::new(texture, allocated).showing(shown);
    // Taking the pointer and the wheel stops the canvas behind the stage
    // scrolling, so only a stage whose widgets hit-test asks for them.
    // A round face fills its box corner to corner, so the breathing room every
    // other stage wants reads as the face being inset in a frame it hasn't got.
    let stage = if size.is_round() {
        Stage::Fit.padding(0)
    } else {
        Stage::Fit.into()
    };
    let staged = ctx.texture_stage(
        ui,
        stage,
        match input {
            Input::Yes => adopted.interactive(),
            Input::No => adopted,
        },
    );

    // Nothing else asks for the next frame: egui repaints on input, and a widget
    // mid-animation has none to offer. Without this a scene advances only while
    // the pointer happens to move over the window.
    if animating || grew {
        ui.ctx().request_repaint();
    }

    // Collapsed: nothing was drawn, so there is nothing to point at or mask.
    let Some(ImageInput { response, pointers }) = staged else {
        return fired;
    };

    // Fed in after the draw, so the widgets under the pointer see them next
    // frame — the same one-frame trip the device's event loop gives them.
    if !pointers.is_empty() {
        FRAMES.with_borrow_mut(|frames| {
            let Some(state) = frames.get_mut(&key) else {
                return;
            };
            for pointer in pointers {
                state.interaction.push_event(match pointer {
                    Pointer::Down { x, y } => TouchEvent::Down { x, y },
                    Pointer::Move { x, y } => TouchEvent::Move { x, y },
                    Pointer::Up { .. } => TouchEvent::Up,
                    Pointer::Wheel { x, y, delta } => TouchEvent::Scroll {
                        x,
                        y,
                        delta_y: -delta.y,
                    },
                });
            }
        });
        ui.ctx().request_repaint();
    }

    if size.is_round() {
        super::round::mask(ui, response.rect);
    }
    fired
}
