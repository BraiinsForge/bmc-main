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
//! holds a `WasmWidgetRuntime` itself. That keeps the interaction surface small
//! enough to become a message protocol, and later to sit on its own thread —
//! `FemtoVgRenderer` and the runtime are both `!Send`, so whoever owns them
//! must construct them in place and keep them for life.

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer as _;
use bmc_wasm_runtime::platform_catalog::DisplayShape;
use bmc_wasm_runtime::{LedEffect, LedRequest, RenderStatus, WasmWidgetRuntime};

mod protocol;

pub(crate) use protocol::{Delivery, ViewCommand};

use super::PlacedTile;
use super::paint::TileGpu;

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
    /// `None` on a platform without a strip.
    count: Option<usize>,
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

pub(crate) struct DeviceView {
    /// `None` for a viewport the manifest declines. No runtime is built,
    /// so there is no live widget and no discovery, and the UI paints a slab.
    runtime: Option<WasmWidgetRuntime>,
    /// Caller-owned renderer. Every `runtime.render(...)` is bracketed by
    /// `runtime.with_renderer(ptr, ...)`, which parks a pointer to it.
    renderer: FemtoVgRenderer,
    pub(crate) gpu: TileGpu,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) shape: DisplayShape,
    label: String,
    sched: ViewSched,
    led: LedState,
    /// Requests from the UI, applied at the start of the next tick.
    ///
    /// Queued rather than applied on arrival because the UI runs after
    /// the render pass: a change made while painting the panels belongs
    /// to the next frame either way, and holding it as data is what lets
    /// the same request cross a channel once views move to their own threads.
    inbox: std::collections::VecDeque<ViewCommand>,
}

impl DeviceView {
    /// Build a view for one placed viewport. The placement carries where it
    /// sits and what hardware it stands for; the rest is what drives it.
    pub(crate) fn new(
        placed: &PlacedTile,
        runtime: Option<WasmWidgetRuntime>,
        renderer: FemtoVgRenderer,
        gpu: TileGpu,
        led_rx: Option<std::sync::mpsc::Receiver<LedRequest>>,
    ) -> Self {
        Self {
            runtime,
            renderer,
            gpu,
            x: placed.x,
            y: placed.y,
            shape: placed.shape,
            label: placed.label.clone(),
            sched: ViewSched::default(),
            led: LedState {
                count: placed.led_count,
                rx: led_rx,
                ..LedState::default()
            },
            inbox: std::collections::VecDeque::new(),
        }
    }

    /// Queue a request. Applied at the start of the next tick, in order.
    pub(crate) fn send(&mut self, command: ViewCommand) {
        // A touch has to reach the widget for its reaction to be rendered.
        // Arm the schedule on arrival rather than at drain time: the drain
        // runs inside the tick that then decides whether a render is due.
        if matches!(command, ViewCommand::Touch(_)) {
            self.sched.pending_interaction = true;
        }
        self.inbox.push_back(command);
    }

    /// Apply everything queued since the last tick.
    fn drain_inbox(&mut self) {
        while let Some(command) = self.inbox.pop_front() {
            let Some(rt) = self.runtime.as_mut() else {
                continue;
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
    }

    /// Whether a runtime was built for this viewport.
    pub(crate) fn is_live(&self) -> bool {
        self.runtime.is_some()
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn led_count(&self) -> Option<usize> {
        self.led.count
    }

    /// The SDK the loaded widget was built against.
    pub(crate) fn sdk_version(&self) -> Option<(u16, u16, u16)> {
        self.runtime.as_ref().map(WasmWidgetRuntime::sdk_version)
    }

    /// Timings from the last render, for the stats panel.
    pub(crate) fn last_timings(&self) -> Option<bmc_render::FrameTimings> {
        self.runtime.as_ref().map(WasmWidgetRuntime::last_timings)
    }

    /// How late the last deadline-driven render ran, in ms.
    /// `None` when a touch or a delivery drove it instead.
    pub(crate) fn last_slip_ms(&self) -> Option<u64> {
        self.sched.last_slip_ms
    }

    /// The scene to draw on the strip, or `None` when it is off.
    pub(crate) fn led_scene(&self) -> Option<&bmc_led::data::LedScene> {
        self.led.scene.as_ref().filter(|_| self.led.enabled)
    }

    // ── Synchronous queries ──────────────────────────────────────────
    //
    // These read the runtime rather than mutate it, and the recorder needs
    // an answer within the gesture it is classifying, so they stay direct.

    /// Whether the widget exports an `on_touch` handler at all.
    pub(crate) fn exports_on_touch(&self) -> bool {
        self.runtime
            .as_ref()
            .is_some_and(WasmWidgetRuntime::exports_on_touch)
    }

    /// The element under a widget-local point, for the recorder's gesture log.
    pub(crate) fn hit_test(&mut self, x: f32, y: f32) -> Option<String> {
        self.runtime.as_mut().and_then(|rt| rt.hit_test(x, y))
    }

    // ── Recording ────────────────────────────────────────────────────

    pub(crate) fn take_recorded_events(&mut self) -> Vec<bmc_wasm_runtime::FixtureEvent> {
        self.runtime
            .as_mut()
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
        self.runtime
            .as_mut()
            .map(|rt| (rt.last_timings(), rt.take_profile_sections()))
    }

    // ── Lifecycle ────────────────────────────────────────────────────

    /// Swap in a freshly built runtime, keeping the renderer and GPU target.
    /// The old runtime's assets go with it, so the renderer starts clean.
    pub(crate) fn replace_runtime(
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
        // Requests aimed at the runtime being replaced would land on a widget
        // that never saw the state they build on.
        self.inbox.clear();
    }

    pub(crate) fn into_pooled_gpu(mut self, gl: &eframe::glow::Context) -> TileGpu {
        self.renderer.drop_all();
        self.gpu.detach_render_target(gl);
        self.gpu
    }

    // ── Drive ────────────────────────────────────────────────────────

    /// Advance one tick: drain LED requests and pending I/O,
    /// then render if the widget's own scheduler asks for it.
    ///
    /// The clock and the delivery drain run every tick while the WASM render
    /// is gated, so an idle view costs nothing — the contract the device host
    /// honours too.
    pub(crate) fn tick(&mut self, tick: &ViewTick) -> ViewTicked {
        if self.runtime.is_none() || self.sched.dead {
            self.inbox.clear();
            return ViewTicked::default();
        }
        self.drain_inbox();
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

        self.renderer
            .begin_frame(self.gpu.width, self.gpu.height, 1.0);
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
