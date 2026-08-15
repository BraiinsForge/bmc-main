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

//! One previewed device: its widget runtime, the renderer that draws it,
//! and the schedule that decides when it runs.
//!
//! Everything the runtime needs is reached through this type, so the UI never
//! holds a `WasmWidgetRuntime` itself. That is what lets a view sit on a thread
//! of its own: `FemtoVgRenderer` and the runtime are both `!Send`, so they are
//! built in place by whoever will own them, and reached only by message.
//!
//! `ViewCore` is that owned half; `DeviceView` is the UI's handle on it, and
//! holds the same fields whether the core is here or on a worker.

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer as _;
use bmc_wasm_runtime::platform_catalog::DisplayShape;
use bmc_wasm_runtime::{LedEffect, LedRequest, RenderStatus, WasmWidgetRuntime};

mod fence;
mod protocol;
pub(crate) mod worker;

pub(crate) use fence::{GpuWait, gpu_wait_for_version};
pub(crate) use protocol::{Delivery, ViewCommand};

use super::PlacedTile;

/// The host state a view needs to advance one tick.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewTick {
    /// Reading for this pass, from which each view derives its own render gap.
    /// `monotonic_ms` cannot serve: it carries the operator's fast-forward,
    /// which is not time that passed.
    pub(crate) now: std::time::Instant,
    pub(crate) system_time: chrono::DateTime<chrono::FixedOffset>,
    pub(crate) monotonic_ms: u64,
    /// Seal live I/O so refreshes fail, mirroring an offline device.
    pub(crate) offline: bool,
}

/// What one tick did, and when the view wants the next one.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ViewTicked {
    /// Monotonic-ms delay until this view's next render; `None` = idle.
    pub(crate) next_wake_ms: Option<u64>,
    pub(crate) rendered: bool,
}

/// LED strip state mirrored from the widget's requests.
#[derive(Default)]
struct LedState {
    rx: Option<std::sync::mpsc::Receiver<LedRequest>>,
    scene: Option<bmc_led::data::LedScene>,
    enabled: bool,
}

/// Render scheduling for one view, driven by the runtime's own frame requests.
#[derive(Default)]
struct ViewSched {
    ever_rendered: bool,
    /// The widget is beyond driving: it trapped in a delivery, or burned
    /// through its fuel once too often. Nothing further is asked of it.
    dead: bool,
    /// Monotonic-ms deadline for the next WASM render, armed from
    /// `next_frame_delay()` after each render.
    ///
    /// `None` = idle until a delivery or touch.
    /// Absolute, not per-tick relative, so it fires rather than receding.
    next_render_at_ms: Option<u64>,
    /// A touch landed since the last render; forces the next tick to render.
    pending_interaction: bool,
    /// When this view last rendered, or `None` while it is idle.
    /// Animations advance by the gap since then, not by the host's frame time,
    /// so a view due every other pass does not animate at half speed.
    last_render_at: Option<std::time::Instant>,
    /// How late the last deadline-driven render ran against its own deadline.
    /// `None` when the render was not deadline-driven.
    last_slip_ms: Option<u64>,
}

/// Everything one view needs to exist, and nothing that ties it to a thread.
///
/// A renderer and a runtime are both `!Send`, so they can only be built by the
/// thread that will own them. Passing the ingredients instead of the objects is
/// what lets the same code build a view inline or on a thread of its own.
pub(crate) struct ViewSeed {
    pub(crate) wasm: std::sync::Arc<[u8]>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) geometry: super::RuntimeTileGeometry,
    pub(crate) config: bmc_wasm_runtime::RuntimeConfig,
    pub(crate) label: String,
    /// `false` for a viewport the manifest declines. Constructing a runtime
    /// runs the guest's `init` and its discovery, so a declined viewport skips
    /// it and stays wasm-free.
    pub(crate) supported: bool,
    pub(crate) get_proc: super::paint::GlProcAddress,
}

/// A view's GL and wasm state, all of it belonging to one context.
pub(crate) struct ViewParts {
    pub(crate) runtime: Option<WasmWidgetRuntime>,
    pub(crate) renderer: FemtoVgRenderer,
    pub(crate) targets: super::paint::ViewTargets,
}

impl ViewSeed {
    /// Build against the context current on this thread.
    pub(crate) fn build(self, gl: &egui_glow::glow::Context) -> anyhow::Result<ViewParts> {
        use anyhow::Context as _;

        let targets = super::paint::ViewTargets::create(gl, self.width, self.height)?;
        // SAFETY: the caller holds a context current on this thread, and the
        // renderer keeps its own glow context for as long as it lives.
        let renderer = unsafe {
            FemtoVgRenderer::new(
                super::paint::proc_loader(self.get_proc.clone()),
                self.width,
                self.height,
                targets.fbo_id(),
                self.config.mesh_msaa_samples,
            )
        }
        .with_context(|| format!("create renderer for {}", self.label))?;

        Ok(ViewParts {
            runtime: self.build_runtime()?,
            renderer,
            targets,
        })
    }

    /// Build only the widget, for a view whose GPU side already exists.
    ///
    /// This is what a hot reload needs: the renderer and its targets outlive
    /// the runtime, and rebuilding them would cost a font system and a shader
    /// compile per view for nothing.
    pub(crate) fn build_runtime(self) -> anyhow::Result<Option<WasmWidgetRuntime>> {
        use anyhow::Context as _;

        if !self.supported {
            return Ok(None);
        }
        let mut rt = WasmWidgetRuntime::new(
            &self.wasm,
            self.width,
            self.height,
            self.geometry.viewport_shape,
            self.geometry.display,
            chrono::Local::now().fixed_offset(),
            self.config,
        )
        .with_context(|| format!("create runtime for {}", self.label))?;
        rt.set_network_info(super::stub_network());
        Ok(Some(rt))
    }
}

/// The half of a view that a GL context owns: the widget, the renderer that
/// draws it, and the schedule deciding when it runs.
///
/// Every field here is `!Send` or belongs beside one that is, so this whole
/// struct lives on whichever thread built it and never crosses back.
pub(crate) struct ViewCore {
    /// `None` for a viewport the manifest declines. No runtime is built,
    /// so there is no live widget and no discovery, and the UI paints a slab.
    runtime: Option<WasmWidgetRuntime>,
    /// Caller-owned renderer. Every `runtime.render(...)` is bracketed by
    /// `runtime.with_renderer(ptr, ...)`, which parks a pointer to it.
    renderer: FemtoVgRenderer,
    width: u32,
    height: u32,
    label: String,
    sched: ViewSched,
    led: LedState,
}

impl ViewCore {
    fn new(
        parts_runtime: Option<WasmWidgetRuntime>,
        renderer: FemtoVgRenderer,
        width: u32,
        height: u32,
        label: String,
        led_rx: Option<std::sync::mpsc::Receiver<LedRequest>>,
    ) -> Self {
        Self {
            runtime: parts_runtime,
            renderer,
            width,
            height,
            label,
            sched: ViewSched::default(),
            led: LedState {
                rx: led_rx,
                ..LedState::default()
            },
        }
    }

    pub(crate) fn renderer_mut(&mut self) -> &mut FemtoVgRenderer {
        &mut self.renderer
    }

    /// Swap in a freshly built runtime, keeping the renderer and its targets.
    /// The old runtime's assets go with it, so the renderer starts clean.
    pub(crate) fn install_runtime(
        &mut self,
        runtime: Option<WasmWidgetRuntime>,
        led_rx: Option<std::sync::mpsc::Receiver<LedRequest>>,
    ) {
        self.renderer.drop_all();
        self.runtime = runtime;
        self.led.rx = led_rx;
        self.led.scene = None;
        self.led.enabled = false;
        self.sched = ViewSched::default();
    }

    /// Hand the sidebar's bindings to a runtime that has just been built and
    /// so knows nothing of them.
    pub(crate) fn rebind_credentials(
        &mut self,
        credentials: bmc_wasm_runtime::CredentialView,
        secrets: bmc_widget_protocol::CredentialSecrets,
    ) {
        if let Some(rt) = self.runtime.as_mut() {
            rt.deliver_credentials_update(credentials, secrets);
        }
    }

    /// Apply one queued request to the widget.
    pub(crate) fn apply(&mut self, command: ViewCommand) {
        // A dead widget is never driven again — a delivery callback would run
        // on the trapped instance, whose stack the trap left where it stood.
        if self.sched.dead {
            return;
        }
        // A touch has to reach the widget for its reaction to be rendered.
        // The tick applies its commands before deciding whether a render is
        // due, so arming here is what makes that decision see the touch.
        if matches!(command, ViewCommand::Touch(_)) {
            self.sched.pending_interaction = true;
        }
        let Some(rt) = self.runtime.as_mut() else {
            return;
        };
        match command {
            // The deliver_* calls report whether anything changed; the view
            // renders on its own schedule either way.
            ViewCommand::Deliver(Delivery::Params(params)) => {
                let _ = rt.deliver_params_update(params);
            }
            ViewCommand::Deliver(Delivery::System(system)) => {
                let _ = rt.deliver_system_update(*system);
            }
            ViewCommand::Deliver(Delivery::Credentials { view, secrets }) => {
                let _ = rt.deliver_credentials_update(*view, *secrets);
            }
            ViewCommand::Touch(event) => rt.push_touch_event(event),
            ViewCommand::DeliverTouch => {
                let _ = rt.deliver_touch();
            }
        }
    }

    /// What the UI needs to know about a view that it cannot ask across a
    /// thread boundary once the view owns one.
    pub(crate) fn info(&self) -> ViewInfo {
        ViewInfo {
            live: self.runtime.is_some(),
            sdk_version: self.runtime.as_ref().map(WasmWidgetRuntime::sdk_version),
            exports_on_touch: self
                .runtime
                .as_ref()
                .is_some_and(WasmWidgetRuntime::exports_on_touch),
        }
    }

    /// The state the UI mirrors after every tick.
    pub(crate) fn report(&self) -> ViewReport {
        ViewReport {
            timings: self.runtime.as_ref().map(WasmWidgetRuntime::last_timings),
            slip_ms: self.sched.last_slip_ms,
            led_scene: self.led.scene.filter(|_| self.led.enabled),
        }
    }
}

/// Per-instance constants, read once so a threaded view never has to be asked.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ViewInfo {
    pub(crate) live: bool,
    pub(crate) sdk_version: Option<(u16, u16, u16)>,
    pub(crate) exports_on_touch: bool,
}

/// The state a rebuilt widget has to be given back.
///
/// A fresh runtime starts with nothing bound and no LED channel, so a reload
/// that skipped this would drop a credential-fed widget to its unbound state
/// and leave its strip dark.
pub(crate) struct Rebind {
    pub(crate) credentials: Box<bmc_wasm_runtime::CredentialView>,
    pub(crate) secrets: Box<bmc_widget_protocol::CredentialSecrets>,
    pub(crate) led_rx: Option<std::sync::mpsc::Receiver<LedRequest>>,
}

/// What one tick leaves behind for the UI to paint.
#[derive(Debug, Clone, Default)]
pub(crate) struct ViewReport {
    pub(crate) timings: Option<bmc_render::FrameTimings>,
    pub(crate) slip_ms: Option<u64>,
    pub(crate) led_scene: Option<bmc_led::data::LedScene>,
}

pub(crate) struct DeviceView {
    /// Inline, the core sits here and ticks on the UI thread; threaded, it
    /// lives on a worker and this holds the channels instead.
    render: Render,
    /// The painter's handles on this view's colour textures: one inline, one
    /// per present target when threaded. Registered once and never displaced —
    /// the painter deletes whatever a registration displaces, and these
    /// textures belong to the view.
    tex_ids: Vec<egui::TextureId>,
    /// The device this view previews; several platforms can be open at once.
    pub(crate) platform: &'static bmc_wasm_runtime::platform_catalog::Platform,
    pub(crate) shape: DisplayShape,
    label: String,
    /// Names this view's KV store, so a hot reload points the rebuilt runtime
    /// back at the store it was already using.
    kv_key: String,
    width: u32,
    height: u32,
    led_count: Option<usize>,
    info: ViewInfo,
    report: ViewReport,
    /// Requests from the UI, applied at the start of the next tick.
    ///
    /// Queued rather than applied on arrival because the UI runs after
    /// the render pass: a change made while painting the panels belongs
    /// to the next frame either way, and it is the same queue a threaded
    /// view drains from its channel.
    inbox: std::collections::VecDeque<ViewCommand>,
}

enum Render {
    Inline {
        core: Box<ViewCore>,
        targets: super::paint::ViewTargets,
    },
    Threaded(worker::Worker),
}

impl DeviceView {
    /// Build a view that renders on the UI thread.
    pub(crate) fn new_inline(
        placed: &PlacedTile,
        platform: &'static bmc_wasm_runtime::platform_catalog::Platform,
        parts: ViewParts,
        egui_tex_id: egui::TextureId,
        led_rx: Option<std::sync::mpsc::Receiver<LedRequest>>,
    ) -> Self {
        let core = ViewCore::new(
            parts.runtime,
            parts.renderer,
            parts.targets.width,
            parts.targets.height,
            placed.label.clone(),
            led_rx,
        );
        let info = core.info();
        let (width, height) = (parts.targets.width, parts.targets.height);
        Self {
            render: Render::Inline {
                core: Box::new(core),
                targets: parts.targets,
            },
            tex_ids: vec![egui_tex_id],
            platform,
            shape: placed.shape,
            label: placed.label.clone(),
            kv_key: placed.kv_key.clone(),
            width,
            height,
            led_count: placed.led_count,
            info,
            report: ViewReport::default(),
            inbox: std::collections::VecDeque::new(),
        }
    }

    /// Adopt a view already running on its own thread.
    pub(crate) fn new_threaded(
        placed: &PlacedTile,
        platform: &'static bmc_wasm_runtime::platform_catalog::Platform,
        worker: worker::Worker,
        tex_ids: [egui::TextureId; 2],
    ) -> Self {
        let info = worker.info();
        let (width, height) = (placed.w, placed.h);
        Self {
            render: Render::Threaded(worker),
            tex_ids: tex_ids.to_vec(),
            platform,
            shape: placed.shape,
            label: placed.label.clone(),
            kv_key: placed.kv_key.clone(),
            width,
            height,
            led_count: placed.led_count,
            info,
            report: ViewReport::default(),
            inbox: std::collections::VecDeque::new(),
        }
    }

    /// Queue a request. Applied at the start of the next tick, in order.
    pub(crate) fn send(&mut self, command: ViewCommand) {
        self.inbox.push_back(command);
    }

    /// Whether a runtime was built for this viewport.
    pub(crate) fn is_live(&self) -> bool {
        self.info.live
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn kv_key(&self) -> &str {
        &self.kv_key
    }

    pub(crate) fn led_count(&self) -> Option<usize> {
        self.led_count
    }

    /// The texture holding this view's newest frame.
    pub(crate) fn tex_id(&self) -> egui::TextureId {
        let slot = match &self.render {
            Render::Inline { .. } => 0,
            Render::Threaded(worker) => worker.showing(),
        };
        self.tex_ids[slot]
    }

    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    /// The SDK the loaded widget was built against.
    pub(crate) fn sdk_version(&self) -> Option<(u16, u16, u16)> {
        self.info.sdk_version
    }

    /// Timings from the last render, for the status bar and view overlays.
    pub(crate) fn last_timings(&self) -> Option<bmc_render::FrameTimings> {
        self.report.timings
    }

    /// How late the last deadline-driven render ran, in ms.
    /// `None` when a touch or a delivery drove it instead.
    pub(crate) fn last_slip_ms(&self) -> Option<u64> {
        self.report.slip_ms
    }

    /// The scene to draw on the strip, or `None` when it is off.
    pub(crate) fn led_scene(&self) -> Option<&bmc_led::data::LedScene> {
        self.report.led_scene.as_ref()
    }

    /// Whether the widget exports an `on_touch` handler at all.
    pub(crate) fn exports_on_touch(&self) -> bool {
        self.info.exports_on_touch
    }

    // ── Synchronous queries ──────────────────────────────────────────
    //
    // These read the runtime within the gesture the recorder is classifying,
    // so they have no answer that could arrive a frame later. Recording and
    // profiling therefore pin their view inline (`--record`, `--perf-report`),
    // and a threaded view answers as if the widget declined.

    /// The element under a widget-local point, for the recorder's gesture log.
    pub(crate) fn hit_test(&mut self, x: f32, y: f32) -> Option<String> {
        self.core_mut()?.runtime.as_mut()?.hit_test(x, y)
    }

    // ── Recording ────────────────────────────────────────────────────

    pub(crate) fn take_recorded_events(&mut self) -> Vec<bmc_wasm_runtime::FixtureEvent> {
        self.core_mut()
            .and_then(|core| core.runtime.as_mut())
            .map(WasmWidgetRuntime::take_recorded_events)
            .unwrap_or_default()
    }

    // ── Perf ─────────────────────────────────────────────────────────

    /// Timings and fuel sections from the last render.
    pub(crate) fn take_perf_sample(
        &mut self,
    ) -> Option<(
        bmc_render::FrameTimings,
        std::collections::BTreeMap<String, u64>,
    )> {
        let rt = self.core_mut()?.runtime.as_mut()?;
        Some((rt.last_timings(), rt.take_profile_sections()))
    }

    /// The core, for the queries only an inline view can answer.
    fn core_mut(&mut self) -> Option<&mut ViewCore> {
        match &mut self.render {
            Render::Inline { core, .. } => Some(core),
            Render::Threaded(_) => None,
        }
    }

    // ── Lifecycle ────────────────────────────────────────────────────

    /// Rebuild the widget from `seed`, keeping the renderer and its targets.
    ///
    /// The seed is passed rather than a built runtime because a threaded view
    /// must construct its own: the renderer whose assets go with the old one
    /// lives on that thread, and a runtime built here could not reach it.
    pub(crate) fn reload(&mut self, seed: ViewSeed, rebind: Rebind) -> anyhow::Result<()> {
        // Requests aimed at the runtime being replaced would land on a widget
        // that never saw the state they build on.
        self.inbox.clear();
        self.report = ViewReport::default();

        match &mut self.render {
            Render::Inline { core, .. } => {
                let runtime = seed.build_runtime()?;
                core.install_runtime(runtime, rebind.led_rx);
                if let Some(rt) = core.runtime.as_mut() {
                    rt.deliver_credentials_update(*rebind.credentials, *rebind.secrets);
                }
                self.info = core.info();
            }
            Render::Threaded(worker) => worker.reload(seed, rebind),
        }
        Ok(())
    }

    /// Tear the view down, giving its GPU resources back.
    ///
    /// Kept separate from `Drop` because the texture needs the painter in
    /// hand, which is not reachable from there. A threaded view hands back
    /// its departing thread rather than waiting for it: the runtime's own
    /// teardown can hold it for a fetch timeout, which is not the UI's to sit
    /// through.
    #[must_use = "a departing worker must be reaped, or its panic goes unseen"]
    pub(crate) fn release(
        self,
        gl: &egui_glow::glow::Context,
        painter: &mut egui_glow::Painter,
    ) -> Option<worker::Retired> {
        let retired = match self.render {
            Render::Inline { mut core, targets } => {
                core.renderer.drop_all();
                targets.destroy_container_objects(gl);
                None
            }
            Render::Threaded(worker) => Some(worker.shutdown()),
        };
        for tex_id in self.tex_ids {
            painter.free_texture(tex_id);
        }
        retired
    }

    // ── Drive ────────────────────────────────────────────────────────

    /// Advance the view by one tick, wherever it renders.
    ///
    /// The queue is drained here rather than inside the core, because a
    /// threaded view's commands travel to it one message at a time.
    pub(crate) fn tick(&mut self, tick: &ViewTick, gl: &egui_glow::glow::Context) -> ViewTicked {
        let commands: Vec<ViewCommand> = self.inbox.drain(..).collect();
        match &mut self.render {
            Render::Inline { core, .. } => {
                for command in commands {
                    core.apply(command);
                }
                let ticked = core.tick(tick);
                self.report = core.report();
                ticked
            }
            Render::Threaded(worker) => {
                let ticked = worker.drive(tick, commands, gl);
                self.report = worker.report();
                ticked
            }
        }
    }
}

impl ViewCore {
    /// Advance one tick: drain LED requests and pending I/O,
    /// then render if the widget's own scheduler asks for it.
    ///
    /// The clock and the delivery drain run every tick while the WASM render
    /// is gated, so an idle view costs nothing — the contract the device host
    /// honours too.
    pub(crate) fn tick(&mut self, tick: &ViewTick) -> ViewTicked {
        if self.runtime.is_none() || self.sched.dead {
            return ViewTicked::default();
        }
        self.drain_led_commands();

        // `*mut FemtoVgRenderer` → `*mut dyn Renderer` is a coercion, not an
        // `as` cast. Taken through `addr_of_mut!` so the parent `&mut` never
        // enters the borrow stack while the pointer is parked.
        let renderer_raw: *mut dyn bmc_render::renderer::Renderer =
            core::ptr::addr_of_mut!(self.renderer);
        let renderer_ptr =
            std::ptr::NonNull::new(renderer_raw).expect("BUG: addr_of_mut! cannot produce null");

        // No `begin_frame` around the drain — it clears the FBO, so it must
        // bracket a real render. The renderer is parked regardless, because
        // delivery callbacks register bitmaps.
        let outcome = {
            let rt = self.runtime.as_mut().expect("BUG: checked above");
            rt.set_hermetic(tick.offline);
            rt.set_time(tick.system_time, tick.monotonic_ms);
            let polled = rt.poll_deliveries_with_renderer(renderer_ptr);
            delivery_poll_outcome(polled, || rt.next_frame_delay() == Some(0))
        };
        let immediate = match outcome {
            DeliveryPollOutcome::Ready { immediate } => immediate,
            DeliveryPollOutcome::Trapped(error) => {
                self.sched.dead = true;
                tracing::error!("{}: delivery trapped: {error}", self.label);
                return ViewTicked::default();
            }
        };

        let due = !self.sched.ever_rendered
            || self.sched.pending_interaction
            || self
                .sched
                .next_render_at_ms
                .is_some_and(|at| tick.monotonic_ms >= at)
            || immediate;
        if !due {
            return ViewTicked {
                next_wake_ms: self.arm_idle_deadline(tick.monotonic_ms),
                rendered: false,
            };
        }
        self.sched.pending_interaction = false;

        // Must be read before `arm_next_deadline` below overwrites it.
        // Only a render that passed its own deadline counts as slipped;
        // a touch or a delivery arrives whenever it arrives.
        self.sched.last_slip_ms = self
            .sched
            .next_render_at_ms
            .filter(|at| tick.monotonic_ms >= *at)
            .map(|at| tick.monotonic_ms - at);

        let delta_ms = animation_delta_ms(self.sched.last_render_at, tick.now);
        self.sched.last_render_at = Some(tick.now);

        self.renderer.begin_frame(self.width, self.height, 1.0);
        let outcome = self
            .runtime
            .as_mut()
            .expect("BUG: checked above")
            .with_renderer(renderer_ptr, |rt| rt.render(delta_ms));
        self.log_render_outcome(&outcome);
        self.renderer.flush();

        ViewTicked {
            next_wake_ms: self.arm_next_deadline(tick.monotonic_ms),
            rendered: true,
        }
    }

    /// A delivery asked for a future (non-immediate) frame: set the deadline
    /// once, never per idle tick, so it cannot recede.
    fn arm_idle_deadline(&mut self, monotonic_ms: u64) -> Option<u64> {
        if self.sched.next_render_at_ms.is_none() {
            let rt = self.runtime.as_mut().expect("BUG: checked by caller");
            if rt.wants_next_frame() {
                self.sched.next_render_at_ms =
                    Some(monotonic_ms + u64::from(rt.next_frame_delay().unwrap_or(0)));
            }
        }
        self.sched
            .next_render_at_ms
            .map(|at| at.saturating_sub(monotonic_ms))
    }

    /// Arm the next deadline from what the widget just requested; `None` = idle.
    fn arm_next_deadline(&mut self, monotonic_ms: u64) -> Option<u64> {
        let rt = self.runtime.as_mut().expect("BUG: checked by caller");
        self.sched.next_render_at_ms = rt
            .wants_next_frame()
            .then(|| monotonic_ms + u64::from(rt.next_frame_delay().unwrap_or(0)));
        // Idle time is not animation time: whatever eventually wakes this view
        // begins a new animation rather than continuing one.
        if self.sched.next_render_at_ms.is_none() {
            self.sched.last_render_at = None;
        }
        self.sched
            .next_render_at_ms
            .map(|at| at.saturating_sub(monotonic_ms))
    }

    fn log_render_outcome(&mut self, outcome: &anyhow::Result<RenderStatus>) {
        match outcome {
            Ok(RenderStatus::Ok) => {
                if !self.sched.ever_rendered {
                    tracing::info!(
                        label = %self.label,
                        instance_id = %self
                            .runtime
                            .as_ref()
                            .expect("BUG: checked by caller")
                            .asset_namespace(),
                        "view: first render after construction/reload"
                    );
                    self.sched.ever_rendered = true;
                }
            }
            Ok(RenderStatus::FuelExhausted) => tracing::warn!("{}: fuel exhausted", self.label),
            Ok(RenderStatus::Dead) => {
                if !self.sched.dead {
                    tracing::error!("{}: widget killed (repeated fuel overages)", self.label);
                    self.sched.dead = true;
                }
            }
            Err(e) => tracing::error!("{}: render failed: {e}", self.label),
        }
    }

    /// Drain pending LED requests into the mirrored scene state.
    fn drain_led_commands(&mut self) {
        let Some(led_rx) = self.led.rx.as_ref() else {
            return;
        };
        while let Ok(req) = led_rx.try_recv() {
            match req {
                LedRequest::SetEffect {
                    effect,
                    color,
                    period_ms,
                    duration,
                    ..
                } => {
                    let hw_effect = match effect {
                        LedEffect::Chase => bmc_led::data::LedEffect::Chase(color),
                        LedEffect::KnightRider => bmc_led::data::LedEffect::KnightRider(color),
                        LedEffect::Scan => bmc_led::data::LedEffect::Scan(color),
                        LedEffect::Snake => bmc_led::data::LedEffect::Snake(color),
                        LedEffect::Breathe => bmc_led::data::LedEffect::Breathe(color),
                        LedEffect::Solid => bmc_led::data::LedEffect::Solid(color),
                    };
                    self.led.scene = Some(bmc_led::data::LedScene {
                        effect: hw_effect,
                        period: (period_ms > 0)
                            .then(|| std::time::Duration::from_millis(u64::from(period_ms))),
                        duration,
                    });
                    self.led.enabled = true;
                }
                LedRequest::Stop { .. } => {
                    self.led.scene = None;
                    self.led.enabled = false;
                }
            }
        }
    }
}

/// What a delivery drain left behind.
enum DeliveryPollOutcome {
    Ready { immediate: bool },
    Trapped(anyhow::Error),
}

/// Judge a drain without touching a runtime that has already trapped.
///
/// `next_frame_is_immediate` is consulted only on the `Ok` path: a trap skips
/// the guest's epilogues, so its `__stack_pointer` keeps the value it held and
/// every later call would start lower.
fn delivery_poll_outcome(
    result: anyhow::Result<bool>,
    next_frame_is_immediate: impl FnOnce() -> bool,
) -> DeliveryPollOutcome {
    match result {
        Ok(_) => DeliveryPollOutcome::Ready {
            immediate: next_frame_is_immediate(),
        },
        Err(error) => DeliveryPollOutcome::Trapped(error),
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::{DeliveryPollOutcome, delivery_poll_outcome};

    #[test]
    fn a_delivery_trap_does_not_query_the_runtime_again() {
        let outcome = delivery_poll_outcome(Err(anyhow::anyhow!("guest trapped")), || {
            panic!("a trapped runtime must not be driven again")
        });

        assert!(matches!(outcome, DeliveryPollOutcome::Trapped(_)));
    }
}

/// How much animation time to hand the widget for a render at `now`.
///
/// Capped because the host advances transition state by this value:
/// an oversized step would jump a transition most of the way to its end.
/// An animating view is scheduled at most one cap apart, so a wider gap
/// means the view stalled, and is worth exactly one step.
fn animation_delta_ms(last_render_at: Option<std::time::Instant>, now: std::time::Instant) -> u32 {
    last_render_at
        .map_or(0, |last| now.duration_since(last).as_millis() as u32)
        .min(bmc_wasm_runtime::RuntimeConfig::DEFAULT_ANIMATION_FRAME_DELAY_MS)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::animation_delta_ms;

    const CAP: u32 = bmc_wasm_runtime::RuntimeConfig::DEFAULT_ANIMATION_FRAME_DELAY_MS;

    #[test]
    fn idle_view_starts_its_animation_from_zero() {
        assert_eq!(
            animation_delta_ms(None, Instant::now()),
            0,
            "a view with no prior render has no animation time to advance"
        );
    }

    #[test]
    fn gap_within_the_cap_passes_through() {
        let last = Instant::now();
        assert_eq!(
            animation_delta_ms(Some(last), last + Duration::from_millis(16)),
            16
        );
    }

    #[test]
    fn long_stall_is_worth_one_animation_step() {
        let last = Instant::now();
        assert_eq!(
            animation_delta_ms(Some(last), last + Duration::from_secs(30)),
            CAP,
            "an unclamped 30 s delta would finish an in-flight transition in a single frame"
        );
    }
}
