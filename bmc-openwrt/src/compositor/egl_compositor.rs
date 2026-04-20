// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL Compositor implementation for bmc-openwrt.

use super::{
    commands::CompositorCommand,
    device_access::{DEFAULT_SEAT_NAME, RootLibinputInterface},
    render::{DrmOutput, EglContext},
    scene_renderer::SceneRenderer,
    state::{ClientState, CompositorState},
    touch_gesture::{GestureState, TouchGesture},
};
use bmc::compositor::{
    Compositor, CompositorError, CompositorEvent, InstanceId, Position, SceneLayout, Size,
    WidgetAction,
};
use bmc_widget_protocol::SettingUpdate;
use smithay::backend::{
    input::{AbsolutePositionEvent, InputEvent, TouchEvent as TouchEventTrait},
    libinput::LibinputInputBackend,
};
use smithay::reexports::{
    calloop::{
        EventLoop, Interest, Mode, PostAction,
        channel::{self as calloop_channel, Event as ChannelEvent},
        generic::Generic,
        timer::{TimeoutAction, Timer},
    },
    drm::control::{Device as DrmControlDevice, Event as DrmEvent},
    input as libinput,
    wayland_server::{Display, ListeningSocket},
};
use std::{
    os::fd::AsFd,
    path::Path,
    sync::{Arc, LazyLock, Mutex},
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Monotonic reference used as a frame-callback time source when no DRM
/// vblank timestamp is available (headless mode, and the pre-first-vblank
/// window on hardware). Wayland only requires monotonic ms from an
/// unspecified epoch — `Instant::elapsed` from a lazy-initialised reference
/// satisfies that without the per-call overhead of sampling a realtime
/// clock.
static COMPOSITOR_BOOT: LazyLock<Instant> = LazyLock::new(Instant::now);
use tokio::sync::mpsc;

const DEFAULT_GPU_PATH: &str = "/dev/dri/renderD128";
const DEFAULT_DISPLAY_PATH: &str = "/dev/dri/card1";

/// Physical display dimensions (panel reports 600x1280 but only 480x1280 is visible).
const PHYSICAL_WIDTH: u32 = 480;
const PHYSICAL_HEIGHT: u32 = 1_280;

/// Synthetic refresh (mHz) advertised to clients in headless mode.
/// Picked to be close to the 16 ms `FRAME_CALLBACK_TICK` cadence so
/// clients that rely on the advertised rate (e.g. timing-sensitive
/// animations) don't skew against the actual callback delivery rate.
const HEADLESS_REFRESH_MHZ: i32 = 60_000;

/// Cadence at which the compositor evaluates pending frame callbacks.
///
/// The widget rate cap in `CompositorState::send_frame_callbacks_for_presented_widgets`
/// decides which callbacks are eligible to fire based on the per-widget
/// minimum interval; this timer just gives the evaluator regular turns.
/// 16 ms ≈ one vblank on a 63 Hz panel — callbacks land within at most
/// one vblank of the moment the rate cap would allow them.
///
/// In headless mode the same timer also synthesises vblank-like state
/// transitions, since there is no DRM device to emit real ones.
const FRAME_CALLBACK_TICK: Duration = Duration::from_millis(16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RedrawState {
    Idle,
    Queued,
    WaitingForVblank { redraw_queued: bool },
}

impl RedrawState {
    fn queue(self) -> Self {
        match self {
            Self::Idle | Self::Queued => Self::Queued,
            Self::WaitingForVblank { .. } => Self::WaitingForVblank {
                redraw_queued: true,
            },
        }
    }

    fn on_vblank(self, flip_pending: bool) -> Self {
        if flip_pending {
            return self;
        }

        match self {
            Self::WaitingForVblank {
                redraw_queued: true,
            }
            | Self::Queued => Self::Queued,
            Self::WaitingForVblank {
                redraw_queued: false,
            }
            | Self::Idle => Self::Idle,
        }
    }

    fn on_frame_submitted() -> Self {
        Self::WaitingForVblank {
            redraw_queued: false,
        }
    }
}

fn dispatch_timeout(redraw_state: RedrawState) -> Option<Duration> {
    match redraw_state {
        RedrawState::Queued => Some(Duration::ZERO),
        RedrawState::Idle | RedrawState::WaitingForVblank { .. } => None,
    }
}

pub struct EglCompositor {
    wayland_display: Mutex<Option<String>>,
    command_tx: calloop_channel::Sender<CompositorCommand>,
    command_channel: Mutex<Option<calloop_channel::Channel<CompositorCommand>>>,
    action_tx: mpsc::UnboundedSender<WidgetAction>,
    event_tx: mpsc::UnboundedSender<CompositorEvent>,
    action_rx: Mutex<Option<mpsc::UnboundedReceiver<WidgetAction>>>,
    event_rx: Mutex<Option<mpsc::UnboundedReceiver<CompositorEvent>>>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    gpu_path: String,
    display_path: String,
    headless: bool,
}

impl std::fmt::Debug for EglCompositor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EglCompositor")
            .field("gpu_path", &self.gpu_path)
            .field("display_path", &self.display_path)
            .finish_non_exhaustive()
    }
}

impl EglCompositor {
    #[must_use]
    pub fn new(headless: bool) -> Self {
        Self::with_device_paths(DEFAULT_GPU_PATH, DEFAULT_DISPLAY_PATH, headless)
    }

    #[must_use]
    pub fn with_device_paths(gpu_path: &str, display_path: &str, headless: bool) -> Self {
        let (command_tx, command_channel) = calloop_channel::channel();
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            wayland_display: Mutex::new(None),
            command_tx,
            command_channel: Mutex::new(Some(command_channel)),
            action_tx,
            event_tx,
            action_rx: Mutex::new(Some(action_rx)),
            event_rx: Mutex::new(Some(event_rx)),
            thread_handle: Mutex::new(None),
            gpu_path: gpu_path.to_owned(),
            display_path: display_path.to_owned(),
            headless,
        }
    }

    #[expect(clippy::too_many_lines)]
    fn run_compositor_loop(
        gpu_path: &str,
        display_path: &str,
        headless: bool,
        command_channel: calloop_channel::Channel<CompositorCommand>,
        action_tx: mpsc::UnboundedSender<WidgetAction>,
        event_tx: mpsc::UnboundedSender<CompositorEvent>,
        ready_tx: &flume::Sender<Result<String, String>>,
    ) {
        tracing::info!("Compositor thread starting (headless={})...", headless,);

        macro_rules! try_init {
            ($expr:expr, $msg:literal) => {
                match $expr {
                    Ok(v) => v,
                    Err(e) => {
                        let err = format!("{}: {}", $msg, e);
                        tracing::error!("{}", err);
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                }
            };
        }

        let (
            scene_renderer,
            logical_width,
            logical_height,
            physical_width,
            physical_height,
            refresh_mhz,
        ) = if headless {
            // Headless: skip EGL/DRM, use hardcoded panel dimensions.
            // 90° rotation: logical = height x width.
            // No real display mode exists, fall back to 60 Hz pacing to match
            // the synthetic frame-callback timer below.
            (
                None,
                PHYSICAL_HEIGHT,
                PHYSICAL_WIDTH,
                PHYSICAL_WIDTH,
                PHYSICAL_HEIGHT,
                HEADLESS_REFRESH_MHZ,
            )
        } else {
            let egl = try_init!(
                EglContext::new(Path::new(&gpu_path)),
                "Failed to initialize EGL context"
            );
            let output = try_init!(
                DrmOutput::new(Path::new(&display_path)),
                "Failed to initialize DRM output"
            );
            let renderer = SceneRenderer::new(egl, output);
            let (lw, lh) = renderer.logical_size();
            let pw = renderer.output().width();
            let ph = renderer.output().height();
            let refresh = renderer.output().refresh_mhz();
            (Some(renderer), lw, lh, pw, ph, refresh)
        };

        tracing::info!(
            "Display configured: {}x{} logical (rotated), {}x{} physical{}",
            logical_width,
            logical_height,
            physical_width,
            physical_height,
            if headless { " [headless]" } else { "" },
        );

        let mut event_loop: EventLoop<'_, AppState> =
            try_init!(EventLoop::try_new(), "Failed to create event loop");
        let mut display: Display<CompositorState> =
            try_init!(Display::new(), "Failed to create Wayland display");
        let compositor_state = CompositorState::new(
            &display,
            logical_width,
            logical_height,
            physical_width,
            physical_height,
            refresh_mhz,
        );
        let listening_socket = try_init!(
            ListeningSocket::bind_auto("wayland", 0..33),
            "Failed to create Wayland socket"
        );

        let Some(socket_name_os) = listening_socket.socket_name() else {
            let err = "Failed to get socket name".to_owned();
            tracing::error!("{}", err);
            let _ = ready_tx.send(Err(err));
            return;
        };
        let socket_name = socket_name_os.to_string_lossy().to_string();

        tracing::info!("Wayland socket created: {}", socket_name);

        let loop_handle = event_loop.handle();
        let poll_fd = try_init!(
            display.backend().poll_fd().try_clone_to_owned(),
            "Failed to clone display poll fd"
        );
        try_init!(
            loop_handle.insert_source(
                Generic::new(poll_fd, Interest::READ, Mode::Level),
                |_, _, state| {
                    state.display.dispatch_clients(&mut state.compositor)?;
                    Ok(PostAction::Continue)
                },
            ),
            "Failed to add display fd to event loop"
        );

        // Register command channel with calloop for event-driven command dispatch
        try_init!(
            loop_handle.insert_source(command_channel, |event, (), state| match event {
                ChannelEvent::Msg(cmd) => handle_command(state, cmd),
                ChannelEvent::Closed => {
                    tracing::info!("Command channel closed");
                    state.should_exit = true;
                }
            }),
            "Failed to add command channel to event loop"
        );

        // Signal ready to main thread
        if ready_tx.send(Ok(socket_name.clone())).is_err() {
            tracing::error!("Failed to signal ready - receiver dropped");
            return;
        }

        let mut app_state = AppState {
            display,
            compositor: compositor_state,
            scene_renderer,
            listening_socket,
            action_tx,
            event_tx,
            gesture: GestureState::new(),
            scene_drag_active: false,
            touch_frame_dirty: false,
            logical_width,
            logical_height,
            should_exit: false,
            redraw_state: RedrawState::Idle,
        };

        // Add listening socket fd for new Wayland client connections
        match app_state.listening_socket.as_fd().try_clone_to_owned() {
            Ok(listener_fd) => {
                if let Err(e) = loop_handle.insert_source(
                    Generic::new(listener_fd, Interest::READ, Mode::Level),
                    |_, _, state| {
                        if let Ok(Some(client_stream)) = state.listening_socket.accept() {
                            if let Err(e) = state
                                .display
                                .handle()
                                .insert_client(client_stream, Arc::new(ClientState::default()))
                            {
                                tracing::error!("Failed to insert client: {e}");
                            } else {
                                tracing::info!("New Wayland client connected");
                            }
                        }
                        Ok(PostAction::Continue)
                    },
                ) {
                    tracing::error!("Failed to add listener fd to event loop: {e}");
                }
            }
            Err(e) => tracing::error!("Failed to clone listener fd: {e}"),
        }

        // Add DRM device fd for real vblank/page-flip events — HW only.
        // In headless mode the callback tick below also synthesises vblank-
        // like redraw-state transitions.
        if let Some(renderer) = &app_state.scene_renderer {
            match renderer
                .output()
                .drm()
                .device_fd()
                .as_fd()
                .try_clone_to_owned()
            {
                Ok(drm_fd) => {
                    if let Err(e) = loop_handle.insert_source(
                        Generic::new(drm_fd, Interest::READ, Mode::Level),
                        |_, _, state| {
                            if let Some(r) = &mut state.scene_renderer
                                && let Ok(events) = r.output().drm().receive_events()
                            {
                                for event in events {
                                    match event {
                                        DrmEvent::Vblank(v) => {
                                            r.output_mut().on_vblank(v.time);
                                        }
                                        DrmEvent::PageFlip(p) => {
                                            r.output_mut().on_vblank(p.duration);
                                        }
                                        DrmEvent::Unknown(_) => {}
                                    }
                                }
                            }
                            Ok(PostAction::Continue)
                        },
                    ) {
                        tracing::error!("Failed to add DRM fd to event loop: {e}");
                    }
                }
                Err(e) => tracing::error!("Failed to clone DRM device fd: {e}"),
            }
        }

        // Single always-on tick that evaluates pending frame callbacks and,
        // in headless mode, also drives the redraw state machine in place
        // of real DRM vblank events. Keeping a single chokepoint here —
        // rather than one timer per mode plus post-render firing — avoids
        // compounding call sites competing to fire the same callbacks.
        let callback_tick = Timer::from_duration(FRAME_CALLBACK_TICK);
        if let Err(e) = loop_handle.insert_source(callback_tick, |_, (), state| {
            if state.scene_renderer.is_none() {
                // Headless: synthesise a vblank by advancing the redraw
                // machine so Queued commits transition to Idle.
                state.refresh_redraw_state();
                if matches!(state.redraw_state, RedrawState::Queued) {
                    state.redraw_state = RedrawState::Idle;
                }
            }

            // Prefer the kernel-delivered vblank timestamp (CLOCK_MONOTONIC,
            // the actual presentation time). Fall back to a process-local
            // monotonic reference for headless mode and for the pre-first-
            // vblank window on hardware — Wayland only requires monotonic
            // ms from an unspecified epoch, and these two fallbacks combined
            // satisfy that without sampling SystemTime (not monotonic; can
            // jump backwards on NTP adjustments).
            #[expect(
                clippy::cast_possible_truncation,
                reason = "wrapping at ~49.7 days is acceptable for frame-callback time"
            )]
            let time = state
                .scene_renderer
                .as_ref()
                .and_then(|r| r.output().last_vblank_ms())
                .unwrap_or_else(|| COMPOSITOR_BOOT.elapsed().as_millis() as u32);
            state
                .compositor
                .send_frame_callbacks_for_presented_widgets(time);

            TimeoutAction::ToDuration(FRAME_CALLBACK_TICK)
        }) {
            tracing::error!("Failed to add frame-callback tick timer to event loop: {e}");
        }

        // Wire Smithay's libinput backend. Device open/close goes through
        // RootLibinputInterface (direct open(2) as root, no seatd), and udev
        // enumerates devices tagged with DEFAULT_SEAT_NAME. A failed
        // assign_seat is non-fatal — the compositor continues without touch,
        // matching the old behaviour when /dev/input/event0 was absent.
        let mut libinput_context = libinput::Libinput::new_with_udev(RootLibinputInterface);
        match libinput_context.udev_assign_seat(DEFAULT_SEAT_NAME) {
            Ok(()) => {
                tracing::info!("libinput seat '{}' assigned", DEFAULT_SEAT_NAME);
                let backend = LibinputInputBackend::new(libinput_context);
                if let Err(e) = loop_handle.insert_source(backend, |event, (), state| {
                    state.handle_input_event(event);
                }) {
                    tracing::error!("Failed to register libinput backend: {e}");
                }
            }
            Err(()) => {
                tracing::warn!(
                    "Failed to assign udev seat '{}' for libinput; touch input disabled",
                    DEFAULT_SEAT_NAME
                );
            }
        }

        tracing::info!("Compositor event loop starting");

        #[cfg(feature = "profiling")]
        let mut loop_w = ii_stopwatch::StopWatch::default();
        #[cfg(feature = "profiling")]
        let mut render_w = ii_stopwatch::StopWatch::default();
        #[cfg(feature = "profiling")]
        let mut dispatch_w = ii_stopwatch::StopWatch::default();
        #[cfg(feature = "profiling")]
        let mut every = ii_stopwatch::Every::new(std::time::Duration::from_secs(5));

        // Main event loop — fully event-driven via calloop.
        //
        // Calloop event sources (registered above):
        //   - Wayland display fd  → client commits, protocol events
        //   - Wayland listener fd → new client connections
        //   - DRM device fd       → vblank / page-flip-complete (HW only)
        //   - Command channel     → scene changes, shutdown, settings
        //   - Callback tick timer → fire pending frame callbacks at 16 ms
        //                           cadence; in headless mode also
        //                           synthesises vblank-like state transitions
        //
        // State machine per iteration:
        //   1. Process protocol events (widget connect/disconnect/actions)
        //   2. If `needs_redraw()` (damage non-empty) — attempt render:
        //      a. `is_flip_pending()` → skip, damage stays for next wake
        //      b. render succeeds → `clear_output_damage()` clears the derived
        //         `needs_redraw()`, send frame callbacks
        //   3. Fulfill any pending capture frames from pixel cache
        //   4. `dispatch(timeout)` — sleep until the next event:
        //      - `needs_redraw() && !flip_pending` → ZERO (render ASAP)
        //      - otherwise → None (block; DRM/Wayland events will wake us)
        loop {
            ii_stopwatch::stopwatch_start!(loop_w);

            if app_state.should_exit {
                tracing::info!("Compositor shutting down");
                break;
            }

            process_protocol_events(&mut app_state);
            app_state.refresh_redraw_state();

            if let Some(renderer) = &mut app_state.scene_renderer {
                // Invalidate texture cache for destroyed buffers — do this
                // unconditionally so GPU resources are freed promptly even
                // when the compositor is idle.
                if !app_state.compositor.invalidated_buffers.is_empty() {
                    let invalidated: Vec<_> =
                        app_state.compositor.invalidated_buffers.drain(..).collect();
                    renderer.invalidate_textures(&invalidated);
                }

                // Bootstrap: if a capture frame request arrived before any
                // render has happened, force a render so `update_capture_cache`
                // populates the cache. Otherwise the first capture would always
                // be failed by `fulfill_from_cache` with `Unknown` (default
                // empty cache), which the relay treats as fatal.
                if !app_state.compositor.pending_capture_frames.is_empty()
                    && !renderer.capture_cache_ready()
                {
                    app_state.redraw_state = app_state.redraw_state.queue();
                }

                // Gate rendering on flip_pending: if a prior page-flip has not
                // completed yet, skip rendering this iteration. We must avoid
                // draining `dirty_buffers` on the flip-pending path — otherwise
                // the dirty-set for DMA-BUFs committed during the flip is lost
                // and `import_textures` never reimports them, causing permanent
                // "No cached texture" spam until the next client commit.
                // The DRM fd wake will re-enter this block once the flip lands.
                let should_render = matches!(app_state.redraw_state, RedrawState::Queued)
                    && !renderer.output().is_flip_pending();

                if should_render {
                    ii_stopwatch::stopwatch_start!(render_w);
                    let dirty: Vec<_> = app_state.compositor.dirty_buffers.drain(..).collect();
                    let capture_frames: Vec<_> = app_state
                        .compositor
                        .pending_capture_frames
                        .drain(..)
                        .collect();
                    let capture_active = !app_state.compositor.capture_sessions.is_empty();
                    let output_damage = app_state.compositor.current_output_damage();
                    let (rendered, unconsumed_captures, capture_failed) = renderer
                        .render_scene(
                            &app_state.compositor.widgets,
                            &app_state.compositor.widget_buffers,
                            &dirty,
                            capture_frames,
                            capture_active,
                            &output_damage,
                        )
                        .unwrap_or_else(|e| {
                            tracing::error!("Render error: {}", e);
                            (false, Vec::new(), false)
                        });
                    if capture_failed {
                        tracing::warn!(
                            "Disabling Wayland image-copy capture after fatal readback failure"
                        );
                        app_state.compositor.disable_capture();
                    }
                    // Return unconsumed captures (flip was pending) for the fallback path
                    app_state
                        .compositor
                        .pending_capture_frames
                        .extend(unconsumed_captures);
                    ii_stopwatch::stopwatch_stop!(render_w);

                    // Frame callbacks are fired from the always-on callback
                    // tick — not from here — so pending callbacks are paced
                    // by a single code path regardless of whether a render
                    // just happened. Here we only advance the redraw state.
                    if rendered {
                        app_state.compositor.clear_output_damage();
                        app_state.redraw_state = RedrawState::on_frame_submitted();
                    } else {
                        // import_textures runs before any bubbled render error
                        // today, so the cache is up to date even when we land
                        // here. Restore the drained IDs anyway so the invariant
                        // survives a future refactor that moves the import or
                        // introduces an earlier `?` — a stale texture cache
                        // manifests as silent rendering glitches that are a
                        // pain to bisect.
                        app_state.compositor.dirty_buffers.extend(dirty);
                        app_state.redraw_state = RedrawState::WaitingForVblank {
                            redraw_queued: true,
                        };
                    }
                }

                // Fulfill pending capture frames from the pixel cache (no re-render).
                // The cache is updated by the inline capture during each render pass.
                if !app_state.compositor.pending_capture_frames.is_empty() {
                    let frames: Vec<_> = app_state
                        .compositor
                        .pending_capture_frames
                        .drain(..)
                        .collect();
                    renderer.fulfill_from_cache(frames);
                }
            } else {
                // Headless: no GPU pipeline, so handling damage means dropping
                // the pending buffers/captures and parking at Idle. The damage
                // clear is load-bearing: `needs_redraw()` is derived from
                // `output_damage`, and without clearing it `refresh_redraw_state`
                // re-queues every iteration and `dispatch_timeout(Queued)` keeps
                // returning `Duration::ZERO`, spinning the loop until the 16 ms
                // tick fires.
                app_state.compositor.invalidated_buffers.clear();
                app_state.compositor.dirty_buffers.clear();
                app_state.compositor.pending_capture_frames.clear();
                app_state.compositor.clear_output_damage();
                app_state.redraw_state = RedrawState::Idle;
            }

            let _ = app_state.display.flush_clients();

            // Dispatch timeout: sleep until the next event unless we can render right now.
            //
            // IMPORTANT: `RedrawState` already encodes whether a flip is pending.
            // `dispatch_timeout` returns `Some(ZERO)` only for `Queued` — never
            // while waiting for vblank — so we avoid the ~1600 Hz busy-spin that
            // used to waste a full CPU core when polling with ZERO during flip.
            let timeout = dispatch_timeout(app_state.redraw_state);
            ii_stopwatch::stopwatch_start!(dispatch_w);
            if event_loop.dispatch(timeout, &mut app_state).is_err() {
                tracing::error!("Event loop dispatch error");
                break;
            }
            ii_stopwatch::stopwatch_stop!(dispatch_w);

            ii_stopwatch::stopwatch_stop!(loop_w);

            #[cfg(feature = "profiling")]
            if ii_stopwatch::every_expired!(every) {
                tracing::info!(
                    "compositor: loop={} render={} dispatch={}",
                    loop_w,
                    render_w,
                    dispatch_w
                );
                ii_stopwatch::stopwatch_reset!(loop_w);
                ii_stopwatch::stopwatch_reset!(render_w);
                ii_stopwatch::stopwatch_reset!(dispatch_w);
            }
        }

        tracing::info!("Compositor thread exiting");
    }
}

struct AppState {
    display: Display<CompositorState>,
    compositor: CompositorState,
    scene_renderer: Option<SceneRenderer>,
    listening_socket: ListeningSocket,
    action_tx: mpsc::UnboundedSender<WidgetAction>,
    event_tx: mpsc::UnboundedSender<CompositorEvent>,
    /// Backend-agnostic gesture state machine, driven by libinput events.
    gesture: GestureState,
    /// `true` once the current touch has been arbitrated to scene-drag
    /// mode; cleared on [`GestureState::on_up`] / cancel. Separate from
    /// `gesture.drag_active()` so the cancel-on-first-drag-sample and
    /// end-drag-on-release transitions are unambiguous even when the
    /// gesture state machine resets `drag_active` mid-handler.
    scene_drag_active: bool,
    /// `true` when at least one `wl_touch` event has been emitted since
    /// the last `wl_touch.frame`. Ensures `wl_touch.frame` is sent exactly
    /// once per libinput `TouchFrame` that produced forwarding.
    touch_frame_dirty: bool,
    /// Logical display width in pixels (for libinput coordinate transforms).
    logical_width: u32,
    /// Logical display height in pixels (for libinput coordinate transforms).
    logical_height: u32,
    should_exit: bool,
    redraw_state: RedrawState,
}

impl AppState {
    fn refresh_redraw_state(&mut self) {
        let flip_pending = self
            .scene_renderer
            .as_ref()
            .is_some_and(|r| r.output().is_flip_pending());
        self.redraw_state = self.redraw_state.on_vblank(flip_pending);

        if self.compositor.needs_redraw() {
            self.redraw_state = self.redraw_state.queue();
        }
    }

    fn handle_input_event(&mut self, event: InputEvent<LibinputInputBackend>) {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "keyboard/pointer/gesture/tablet events are not used by this product"
        )]
        match event {
            InputEvent::DeviceAdded { device } => {
                tracing::info!("libinput device added: {}", device.name());
            }
            InputEvent::DeviceRemoved { device } => {
                tracing::info!("libinput device removed: {}", device.name());
            }
            InputEvent::TouchDown { event } => self.on_touch_down(&event),
            InputEvent::TouchMotion { event } => self.on_touch_motion(&event),
            InputEvent::TouchUp { event } => self.on_touch_up(&event),
            InputEvent::TouchCancel { .. } => self.on_touch_cancel(),
            InputEvent::TouchFrame { .. } => self.on_touch_frame(),
            _ => {}
        }
    }

    /// Map a libinput absolute touch event into logical-screen space.
    ///
    /// `x_transformed` / `y_transformed` rely on libinput already seeing
    /// the panel in logical landscape orientation. On the shipping
    /// Goodix GT911 the kernel driver's native `ABS_X` / `ABS_Y` axes
    /// already align with landscape, so the identity calibration is
    /// correct and no `LIBINPUT_CALIBRATION_MATRIX` udev tag or
    /// `config_calibration_set_matrix()` call is set on the device.
    ///
    /// This invariant is specific to the current panel + kernel driver
    /// combination. A panel swap (hardware rev, controller firmware
    /// change) can silently break it — touches will land rotated 90°,
    /// 180° or 270° with no runtime diagnostic. The inverse rotation on
    /// the render side (`scene_renderer.rs`, `Transform::_270`) must
    /// stay consistent with this assumption.
    fn touch_location(
        &self,
        event: &impl AbsolutePositionEvent<LibinputInputBackend>,
    ) -> (f64, f64) {
        #[expect(
            clippy::cast_possible_wrap,
            reason = "logical dimensions are panel-sized and fit in i32"
        )]
        let w = self.logical_width as i32;
        #[expect(
            clippy::cast_possible_wrap,
            reason = "logical dimensions are panel-sized and fit in i32"
        )]
        let h = self.logical_height as i32;
        (event.x_transformed(w), event.y_transformed(h))
    }

    fn on_touch_down(
        &mut self,
        event: &(
             impl AbsolutePositionEvent<LibinputInputBackend> + TouchEventTrait<LibinputInputBackend>
         ),
    ) {
        use smithay::input::touch::DownEvent;
        use smithay::utils::{Logical, Point, SERIAL_COUNTER};

        let (x, y) = self.touch_location(event);
        let time = event.time_msec();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "touch coordinates are panel-sized; fractional pixels round to i32"
        )]
        let ix = x as i32;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "touch coordinates are panel-sized; fractional pixels round to i32"
        )]
        let iy = y as i32;

        self.gesture.on_down(ix, iy, time);
        self.scene_drag_active = false;

        let touch_handle = self.compositor.touch_handle.clone();
        let focus = self.compositor.touch_focus_at(x, y);
        touch_handle.down(
            &mut self.compositor,
            focus,
            &DownEvent {
                slot: event.slot(),
                time,
                location: Point::<f64, Logical>::from((x, y)),
                serial: SERIAL_COUNTER.next_serial(),
            },
        );
        self.touch_frame_dirty = true;
    }

    fn on_touch_motion(
        &mut self,
        event: &(
             impl AbsolutePositionEvent<LibinputInputBackend> + TouchEventTrait<LibinputInputBackend>
         ),
    ) {
        use smithay::input::touch::MotionEvent;
        use smithay::utils::{Logical, Point};

        let (x, y) = self.touch_location(event);
        let time = event.time_msec();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "touch coordinates are panel-sized; fractional pixels round to i32"
        )]
        let ix = x as i32;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "touch coordinates are panel-sized; fractional pixels round to i32"
        )]
        let iy = y as i32;

        let drag_activated = self.gesture.on_motion(ix, iy, time);

        if drag_activated && !self.scene_drag_active && self.compositor.widgets.can_drag() {
            // Mid-touch transition: arbitrate to scene drag and cancel the
            // wl_touch sequence the widget is currently seeing. Skipped when
            // the layout has only one scene — there is nothing to swipe to,
            // so the widget keeps owning the touch sequence.
            self.scene_drag_active = true;
            let touch_handle = self.compositor.touch_handle.clone();
            touch_handle.cancel(&mut self.compositor);
            self.touch_frame_dirty = true;
            self.compositor.widgets.start_drag();
        }

        if self.scene_drag_active {
            if let Some(info) = self.gesture.drag_info() {
                self.compositor.widgets.update_drag(info.dx);
                self.compositor.mark_full_output_damage();
            }
        } else {
            let touch_handle = self.compositor.touch_handle.clone();
            let focus = self.compositor.touch_focus_at(x, y);
            touch_handle.motion(
                &mut self.compositor,
                focus,
                &MotionEvent {
                    slot: event.slot(),
                    time,
                    location: Point::<f64, Logical>::from((x, y)),
                },
            );
            self.touch_frame_dirty = true;
        }
    }

    fn on_touch_up(&mut self, event: &impl TouchEventTrait<LibinputInputBackend>) {
        use smithay::input::touch::UpEvent;
        use smithay::utils::SERIAL_COUNTER;

        let time = event.time_msec();
        let gesture_result = self.gesture.on_up(time);

        if self.scene_drag_active {
            self.scene_drag_active = false;
            if let Some(TouchGesture::DragEnd { dx, velocity_x }) = gesture_result {
                let committed = self.compositor.widgets.end_drag(dx, velocity_x);
                self.compositor.mark_full_output_damage();
                if committed {
                    tracing::info!(
                        "Scene transition committed (dx={}, vel={:.0})",
                        dx,
                        velocity_x
                    );
                } else {
                    tracing::info!("Scene transition snapped back");
                }
            } else {
                // Drag started but gesture classification didn't finalize —
                // snap back to the origin rather than leaving the scene offset.
                self.compositor.widgets.end_drag(0, 0.0);
                self.compositor.mark_full_output_damage();
            }
        } else {
            let touch_handle = self.compositor.touch_handle.clone();
            touch_handle.up(
                &mut self.compositor,
                &UpEvent {
                    slot: event.slot(),
                    time,
                    serial: SERIAL_COUNTER.next_serial(),
                },
            );
            self.touch_frame_dirty = true;
            if matches!(gesture_result, Some(TouchGesture::Tap)) {
                tracing::debug!("Tap detected");
            }
        }
    }

    fn on_touch_cancel(&mut self) {
        self.gesture.on_cancel();

        if self.scene_drag_active {
            self.scene_drag_active = false;
            self.compositor.widgets.end_drag(0, 0.0);
            self.compositor.mark_full_output_damage();
        } else {
            let touch_handle = self.compositor.touch_handle.clone();
            touch_handle.cancel(&mut self.compositor);
            self.touch_frame_dirty = true;
        }
    }

    fn on_touch_frame(&mut self) {
        if self.touch_frame_dirty {
            let touch_handle = self.compositor.touch_handle.clone();
            touch_handle.frame(&mut self.compositor);
            self.touch_frame_dirty = false;
        }
    }
}

fn handle_command(state: &mut AppState, cmd: CompositorCommand) {
    match cmd {
        CompositorCommand::RegisterWidget {
            instance_id,
            position,
            size,
            pid,
        } => {
            tracing::debug!(
                "Registering widget {} at ({}, {}) size {}x{} pid={:?}",
                instance_id,
                position.x,
                position.y,
                size.width,
                size.height,
                pid
            );
            // Widget registration is informational - actual tracking happens via Wayland protocol
        }
        CompositorCommand::UnregisterWidget { instance_id } => {
            tracing::debug!("Unregistering widget {}", instance_id);
            state.compositor.mark_full_output_damage();
            state
                .compositor
                .deck_widget_state
                .unregister_widget(&instance_id);
            // Remove stale touch routing surface so a reconnecting widget
            // gets a fresh entry via the surface commit path.
            state.compositor.render_surfaces.remove(&instance_id);
        }
        CompositorCommand::SetActiveScene { layout } => {
            tracing::info!("Setting active scene with {} widgets", layout.widgets.len());
            for w in &layout.widgets {
                tracing::info!(
                    "  Scene widget: {} at ({}, {}) size {}x{} visible={}",
                    w.instance_id,
                    w.position.x,
                    w.position.y,
                    w.size.width,
                    w.size.height,
                    w.visible
                );
            }
            state.compositor.widgets.set_active_scene(layout);
            state.compositor.mark_full_output_damage();
        }
        CompositorCommand::SetSceneCycling { scenes } => {
            tracing::info!("Setting scene cycling with {} scenes", scenes.len());
            state.compositor.widgets.set_scene_cycling(scenes);
            state.compositor.mark_full_output_damage();
        }
        CompositorCommand::BroadcastSetting { setting } => {
            tracing::debug!("Broadcasting setting: {:?}", setting);
            state
                .compositor
                .deck_widget_state
                .broadcast_setting(&setting);
        }
        CompositorCommand::Shutdown => {
            tracing::info!("Shutdown command received");
            state.compositor.deck_widget_state.broadcast_shutdown();
            state.should_exit = true;
        }
    }
}

fn process_protocol_events(state: &mut AppState) {
    for instance_id in state.compositor.deck_widget_state.drain_connected() {
        tracing::info!("Widget connected: {}", instance_id);
        let _ = state
            .event_tx
            .send(CompositorEvent::WidgetReady { instance_id });
    }

    for disconnected in state.compositor.deck_widget_state.drain_disconnected() {
        state.compositor.mark_full_output_damage();
        state
            .compositor
            .drop_widget_callback_state(&disconnected.instance_id, disconnected.pid);
        tracing::info!("Widget disconnected: {}", disconnected.instance_id);
        let _ = state.event_tx.send(CompositorEvent::WidgetDisconnected {
            instance_id: disconnected.instance_id,
        });
    }

    for (instance_id, payload) in state.compositor.deck_widget_state.drain_actions() {
        tracing::debug!("Widget {} action: {:?}", instance_id, payload);
        let _ = state.action_tx.send(WidgetAction {
            instance_id,
            payload,
        });
    }
}

impl Default for EglCompositor {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Compositor for EglCompositor {
    fn start(&self) -> Result<String, CompositorError> {
        {
            let display = self
                .wayland_display
                .lock()
                .expect("BUG: wayland_display lock poisoned");
            if display.is_some() {
                return Err(CompositorError::AlreadyStarted);
            }
        }

        let (ready_tx, ready_rx) = flume::bounded(1);

        let gpu_path = self.gpu_path.clone();
        let display_path = self.display_path.clone();
        let headless = self.headless;
        let command_channel = self
            .command_channel
            .lock()
            .expect("BUG: command_channel lock poisoned")
            .take()
            .expect("BUG: command_channel already taken");
        let action_tx = self.action_tx.clone();
        let event_tx = self.event_tx.clone();

        let handle = thread::Builder::new()
            .name("egl-compositor".to_owned())
            .spawn(move || {
                Self::run_compositor_loop(
                    &gpu_path,
                    &display_path,
                    headless,
                    command_channel,
                    action_tx,
                    event_tx,
                    &ready_tx,
                );
            })
            .map_err(|e| CompositorError::ThreadError(e.to_string()))?;

        {
            let mut thread_handle = self
                .thread_handle
                .lock()
                .expect("BUG: thread handle lock poisoned");
            *thread_handle = Some(handle);
        }

        let socket_name = ready_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| {
                CompositorError::ThreadError("Timeout waiting for compositor to start".to_owned())
            })?
            .map_err(CompositorError::ThreadError)?;

        {
            let mut display = self
                .wayland_display
                .lock()
                .expect("BUG: wayland_display lock poisoned");
            *display = Some(socket_name.clone());
        }

        Ok(socket_name)
    }

    fn wayland_display(&self) -> Option<String> {
        let display = self
            .wayland_display
            .lock()
            .expect("BUG: wayland_display lock poisoned");
        display.clone()
    }

    fn register_widget(
        &self,
        instance_id: InstanceId,
        position: Position,
        size: Size,
        pid: Option<u32>,
    ) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::RegisterWidget {
                instance_id,
                position,
                size,
                pid,
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::UnregisterWidget {
                instance_id: instance_id.clone(),
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn set_active_scene(&self, layout: SceneLayout) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetActiveScene { layout })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn set_scene_cycling(&self, scenes: Vec<SceneLayout>) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetSceneCycling { scenes })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_setting(&self, setting: SettingUpdate) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::BroadcastSetting { setting })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn action_receiver(&self) -> mpsc::UnboundedReceiver<WidgetAction> {
        self.action_rx
            .lock()
            .expect("BUG: action_rx lock poisoned")
            .take()
            .expect("BUG: action_receiver already taken")
    }

    fn event_receiver(&self) -> mpsc::UnboundedReceiver<CompositorEvent> {
        self.event_rx
            .lock()
            .expect("BUG: event_rx lock poisoned")
            .take()
            .expect("BUG: event_receiver already taken")
    }

    fn shutdown(&self) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::Shutdown)
            .map_err(|e| CompositorError::SendError(e.to_string()))?;

        let handle = {
            let mut thread_handle = self
                .thread_handle
                .lock()
                .expect("BUG: thread handle lock poisoned");
            thread_handle.take()
        };

        if let Some(handle) = handle {
            let _ = handle.join();
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{RedrawState, dispatch_timeout};
    use std::time::Duration;

    #[test]
    fn queued_redraw_without_pending_flip_retries_immediately() {
        assert_eq!(dispatch_timeout(RedrawState::Queued), Some(Duration::ZERO));
    }

    #[test]
    fn queued_redraw_with_pending_flip_waits_for_events() {
        assert_eq!(
            dispatch_timeout(RedrawState::WaitingForVblank {
                redraw_queued: true,
            }),
            None
        );
    }

    #[test]
    fn idle_state_waits_for_events() {
        assert_eq!(dispatch_timeout(RedrawState::Idle), None);
    }

    #[test]
    fn queued_redraw_during_vblank_wait_is_retained() {
        assert_eq!(
            RedrawState::WaitingForVblank {
                redraw_queued: false,
            }
            .queue(),
            RedrawState::WaitingForVblank {
                redraw_queued: true
            }
        );
    }

    #[test]
    fn vblank_promotes_waiting_redraw_to_queued() {
        assert_eq!(
            RedrawState::WaitingForVblank {
                redraw_queued: true
            }
            .on_vblank(false),
            RedrawState::Queued
        );
    }

    #[test]
    fn vblank_without_new_work_returns_to_idle() {
        assert_eq!(
            RedrawState::WaitingForVblank {
                redraw_queued: false,
            }
            .on_vblank(false),
            RedrawState::Idle
        );
    }
}
