// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL Compositor implementation for bmc-openwrt.

use super::scene_renderer::touch_to_logical;
use super::{
    commands::CompositorCommand,
    device_access::{DeviceAccessConfig, RootLibinputInterface, set_libinput_debug_priority},
    lifecycle_emitter::Emission,
    protocol::{DeckWidgetHandler, DeckWidgetProtocolState},
    render::{DrmOutput, EglContext},
    scene_renderer::SceneRenderer,
    state::{ClientState, CompositorState},
    touch_gesture::{GestureState, TouchGesture},
    widget_tracker::LifecycleState,
};
use bmc::compositor::{
    Compositor, CompositorError, CompositorEvent, InstanceId, Position, SceneLayout, Size,
    WidgetAction,
};
use bmc_platform::TouchTransform;
use bmc_platform::linux_input::discover_touch_node;
use bmc_widget_protocol::{SettingUpdate, WidgetInitialConfig};
use smithay::backend::{
    input::{AbsolutePositionEvent, InputEvent, TouchEvent as TouchEventTrait, TouchSlot},
    libinput::LibinputInputBackend,
};
use smithay::reexports::{
    calloop::{
        EventLoop, Interest, LoopHandle, Mode, PostAction,
        channel::{self as calloop_channel, Event as ChannelEvent},
        generic::Generic,
        timer::{TimeoutAction, Timer},
    },
    drm::control::{Device as DrmControlDevice, Event as DrmEvent},
    input as libinput,
    wayland_server::{Display, ListeningSocket, backend::ClientId},
};
use smithay::utils::{Logical, Point};
use std::{
    collections::HashSet,
    os::fd::AsFd,
    path::{Path, PathBuf},
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

/// Backoff ladder for post-startup touch-discovery retries.
///
/// Sized to cover the udev/sysfs visibility race on mdev-only OpenWrt
/// images — the panel controller can take a few seconds to finish
/// probing after the compositor starts, during which `discover_touch_node`
/// returns nothing. Five attempts totalling ~7.75 s is enough for the
/// observed worst case; past that we give up and wait for a restart.
const TOUCH_DISCOVERY_BACKOFFS: &[Duration] = &[
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

/// Maximum time to wait for the compositor thread to finish its EGL
/// setup and announce its Wayland socket name back to the caller of
/// `start()`. Bounded so a stuck GPU init does not hang the whole
/// control process.
const COMPOSITOR_READY_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum time to wait for the compositor thread to acknowledge a
/// widget-lifecycle command. These acks are processed synchronously
/// inside the compositor event loop, so the deadline only protects
/// against the loop being wedged.
const WIDGET_COMMAND_ACK_TIMEOUT: Duration = Duration::from_secs(2);

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
    device_access: DeviceAccessConfig,
    profile: bmc_platform::HardwareProfile,
    headless: bool,
}

impl std::fmt::Debug for EglCompositor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EglCompositor")
            .field("seat_name", &self.device_access.seat_name())
            .field(
                "render_node",
                &self
                    .device_access
                    .resolved_render_node()
                    .display()
                    .to_string(),
            )
            .field(
                "scanout_node",
                &self
                    .device_access
                    .resolved_scanout_node()
                    .display()
                    .to_string(),
            )
            .finish_non_exhaustive()
    }
}

impl EglCompositor {
    #[must_use]
    pub fn new(profile: bmc_platform::HardwareProfile, headless: bool) -> Self {
        Self::with_device_access_config(DeviceAccessConfig::default(), profile, headless)
    }

    #[must_use]
    pub fn with_device_paths(
        gpu_path: &str,
        display_path: &str,
        profile: bmc_platform::HardwareProfile,
        headless: bool,
    ) -> Self {
        Self::with_device_access_config(
            DeviceAccessConfig::default()
                .with_render_node(gpu_path)
                .with_scanout_node(display_path),
            profile,
            headless,
        )
    }

    #[must_use]
    pub fn with_device_access_config(
        device_access: DeviceAccessConfig,
        profile: bmc_platform::HardwareProfile,
        headless: bool,
    ) -> Self {
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
            device_access,
            profile,
            headless,
        }
    }

    #[expect(clippy::too_many_lines)]
    #[expect(
        clippy::too_many_arguments,
        reason = "compositor bootstrap wiring; argument count stays tied to how \
                  EglCompositor decomposes its fields into the worker thread"
    )]
    fn run_compositor_loop(
        render_node: &Path,
        scanout_node: &Path,
        seat_name: &str,
        input_nodes: &[PathBuf],
        has_explicit_input_nodes: bool,
        profile: &bmc_platform::HardwareProfile,
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

        let display_profile = &profile.display;
        let (
            scene_renderer,
            logical_width,
            logical_height,
            physical_width,
            physical_height,
            refresh_mhz,
        ) = if headless {
            (
                None,
                display_profile.logical_width,
                display_profile.logical_height,
                display_profile.visible_area.width,
                display_profile.visible_area.height,
                HEADLESS_REFRESH_MHZ,
            )
        } else {
            let egl = try_init!(
                EglContext::new(render_node),
                "Failed to initialize EGL context"
            );
            let output = try_init!(
                DrmOutput::new(scanout_node, *display_profile),
                "Failed to initialize DRM output"
            );
            let renderer = try_init!(
                SceneRenderer::new(
                    egl,
                    output,
                    display_profile.scanout_transform,
                    display_profile.seam_overlap_px,
                    display_profile.pixel_format,
                ),
                "Failed to initialize scene renderer"
            );
            let (lw, lh) = renderer.logical_size();
            let pw = renderer.output().width();
            let ph = renderer.output().height();
            let refresh = renderer.output().refresh_mhz();
            (Some(renderer), lw, lh, pw, ph, refresh)
        };

        tracing::info!(
            "Display configured: {}x{} logical, {}x{} physical{}",
            logical_width,
            logical_height,
            physical_width,
            physical_height,
            if headless { " [headless]" } else { "" },
        );

        let mut event_loop: EventLoop<'static, AppState> =
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
            seat_name,
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
            gesture_slot: None,
            active_touch_slots: HashSet::new(),
            scene_drag_active: false,
            touch_frame_dirty: false,
            logical_width,
            logical_height,
            touch_transform: display_profile.touch_transform,
            should_exit: false,
            redraw_state: RedrawState::Idle,
            loop_handle: loop_handle.clone(),
            retry_libinput: None,
            touch_retry_pending: false,
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

        // Wire Smithay's libinput backend using a path-based context.
        //
        // Device open/close goes through RootLibinputInterface (direct open(2)
        // as root, no seatd). The OpenWrt appliance image uses `mdev`, so
        // there is no udev database for `new_with_udev` / `udev_assign_seat`
        // to enumerate — path-based libinput bypasses that entirely and adds
        // each configured evdev node explicitly. `seat_name` is retained for
        // logging / future udev-capable images; it has no runtime effect on
        // the path context itself.
        tracing::info!(
            "Initializing path-based libinput (seat='{seat_name}', {} device(s))",
            input_nodes.len(),
        );
        let libinput_context = libinput::Libinput::new_from_path(RootLibinputInterface);

        // Lower libinput's log priority to DEBUG so its internal
        // rejections (device classification, quirk application, udev
        // lookups) appear on our stderr — which bmc-openwrt's service
        // wrapper tees into bmc.log.
        set_libinput_debug_priority(&libinput_context);

        // Keep a cloned handle so the retry timer can call
        // `path_add_device` after the primary handle is moved into
        // `LibinputInputBackend::new`. `Libinput` is a refcounted
        // wrapper around the same C context; devices added through
        // either handle are visible to the backend's poll loop.
        let retry_context = libinput_context.clone();
        let mut primary_context = libinput_context;
        let initial_added = register_touch_devices(&mut primary_context, input_nodes);

        let backend = LibinputInputBackend::new(primary_context);
        if let Err(e) = loop_handle.insert_source(backend, |event, (), state| {
            state.handle_input_event(event);
        }) {
            tracing::error!("Failed to register libinput backend with event loop: {e}");
        }

        // Make the retry path reachable from `DeviceRemoved` as well —
        // mid-run device loss (kernel module reload, VM hotplug) can leave
        // touch dead just as a startup race can. `None` when explicit
        // input nodes were configured, so retry is disabled.
        app_state.retry_libinput = (!has_explicit_input_nodes).then_some(retry_context);

        if initial_added == 0 {
            tracing::warn!(
                "libinput has no devices registered at startup; touch input is currently disabled. \
                 Check DeviceAccessConfig::with_input_node and the evdev node permissions."
            );
            app_state.schedule_touch_discovery_retry();
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
                    && !app_state.compositor.widget_buffers.is_empty()
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
                if !app_state.compositor.pending_capture_frames.is_empty()
                    && renderer.capture_cache_ready()
                {
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

#[expect(
    clippy::struct_excessive_bools,
    reason = "discrete latches on independent subsystems (touch sequence, redraw, retry); a state-machine refactor would tangle unrelated lifecycles"
)]
struct AppState {
    display: Display<CompositorState>,
    compositor: CompositorState,
    scene_renderer: Option<SceneRenderer>,
    listening_socket: ListeningSocket,
    action_tx: mpsc::UnboundedSender<WidgetAction>,
    event_tx: mpsc::UnboundedSender<CompositorEvent>,
    /// Backend-agnostic gesture state machine, driven by libinput events.
    gesture: GestureState,
    /// Touch slot that owns compositor-level scene arbitration for the
    /// current sequence. Only the first contact in an otherwise idle
    /// sequence can turn into a scene swipe.
    gesture_slot: Option<TouchSlot>,
    /// All currently active libinput touch slots.
    active_touch_slots: HashSet<TouchSlot>,
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
    /// Logical (landscape) width used as the libinput coordinate range; the
    /// GT911 reports its axes already in this orientation.
    logical_width: u32,
    /// Logical (landscape) height used as the libinput coordinate range; the
    /// GT911 reports its axes already in this orientation.
    logical_height: u32,
    /// Residual per-panel rotation applied to libinput samples to produce
    /// logical-screen coordinates; the identity when the touch axes already
    /// match the logical orientation.
    touch_transform: TouchTransform,
    should_exit: bool,
    redraw_state: RedrawState,
    /// Calloop handle, used to (re-)arm the touch-discovery retry timer
    /// from `schedule_touch_discovery_retry`.
    loop_handle: LoopHandle<'static, AppState>,
    /// Cloned libinput handle for `path_add_device` calls from the retry
    /// timer. `None` either before libinput is initialised or when
    /// `DeviceAccessConfig` pinned the input nodes — in pinned mode the
    /// caller chose specific paths and discovery (which the retry runs)
    /// cannot help.
    retry_libinput: Option<libinput::Libinput>,
    /// `true` while a touch-discovery retry timer is scheduled. Prevents
    /// stacking duplicate timers if multiple `DeviceRemoved` events fire
    /// in quick succession.
    touch_retry_pending: bool,
}

impl AppState {
    /// Schedule (or skip) a touch-discovery retry ladder.
    ///
    /// Idempotent: returns early when discovery is disabled (`with_input_node`
    /// override pinned the paths) or a retry is already scheduled. The timer
    /// fires on calloop's main poll loop, walks sysfs, and adds any newly
    /// discovered touch nodes via `path_add_device`. Self-terminates after
    /// success or budget exhaustion (5 attempts, ~6.25 s total).
    fn schedule_touch_discovery_retry(&mut self) {
        if self.touch_retry_pending {
            return;
        }
        // `None` here means retry is disabled — either libinput hasn't been
        // initialised yet, or the caller pinned explicit input nodes (in
        // which case `discover_touch_node` has nothing to add).
        let Some(mut retry_context) = self.retry_libinput.clone() else {
            return;
        };
        let mut attempt = 0_usize;
        let timer = Timer::from_duration(TOUCH_DISCOVERY_BACKOFFS[0]);
        let result = self.loop_handle.insert_source(timer, move |_, (), state| {
            attempt += 1;
            let nodes: Vec<PathBuf> = discover_touch_node().into_iter().collect();
            let added = if nodes.is_empty() {
                tracing::debug!("Touch discovery retry {attempt}: no candidate nodes in sysfs");
                0
            } else {
                register_touch_devices(&mut retry_context, &nodes)
            };
            if added > 0 {
                state.touch_retry_pending = false;
                tracing::info!(
                    "Touch input recovered on discovery retry {attempt} ({added} device(s))"
                );
                return TimeoutAction::Drop;
            }
            if attempt >= TOUCH_DISCOVERY_BACKOFFS.len() {
                state.touch_retry_pending = false;
                tracing::error!(
                    "Touch discovery retry budget exhausted after {attempt} attempts; \
                         touch input remains disabled until restart"
                );
                return TimeoutAction::Drop;
            }
            let next = TOUCH_DISCOVERY_BACKOFFS[attempt];
            tracing::debug!(
                "Touch discovery retry {attempt} produced no devices; retrying in {next:?}"
            );
            TimeoutAction::ToDuration(next)
        });
        match result {
            Ok(_) => self.touch_retry_pending = true,
            Err(e) => tracing::error!("Failed to schedule touch discovery retry timer: {e}"),
        }
    }

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
                // The removed device may be the touch source. If a sequence
                // was in flight, cancel it so `active_touch_slots` and the
                // gesture state machine don't lock out future touches.
                if !self.active_touch_slots.is_empty() {
                    self.on_touch_cancel();
                }
                // Re-arm discovery in case the touch device went away. The
                // retry helper is a no-op when discovery is disabled (explicit
                // nodes) or already pending; if a touch device is still alive,
                // `path_add_device` is idempotent and the timer drops itself.
                self.schedule_touch_discovery_retry();
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
    /// The GT911 reports its `ABS_X` / `ABS_Y` axes already aligned with the
    /// logical landscape orientation the widget tree paints into, so the
    /// sample is scaled directly against the logical dimensions. The profile's
    /// `touch_transform` then applies any residual per-panel rotation.
    fn touch_location(
        &self,
        event: &impl AbsolutePositionEvent<LibinputInputBackend>,
    ) -> Point<f64, Logical> {
        #[expect(
            clippy::cast_possible_wrap,
            reason = "logical dimensions are panel-sized and fit in i32"
        )]
        let lw = self.logical_width as i32;
        #[expect(
            clippy::cast_possible_wrap,
            reason = "logical dimensions are panel-sized and fit in i32"
        )]
        let lh = self.logical_height as i32;
        let raw_x = event.x_transformed(lw);
        let raw_y = event.y_transformed(lh);
        let (lx, ly) = touch_to_logical(
            raw_x,
            raw_y,
            f64::from(self.logical_width),
            f64::from(self.logical_height),
            self.touch_transform,
        );
        Point::<f64, Logical>::from((lx, ly))
    }

    fn on_touch_down(
        &mut self,
        event: &(
             impl AbsolutePositionEvent<LibinputInputBackend> + TouchEventTrait<LibinputInputBackend>
         ),
    ) {
        use smithay::input::touch::DownEvent;
        use smithay::utils::SERIAL_COUNTER;

        let location = self.touch_location(event);
        let time = event.time_msec();
        let slot = event.slot();

        let sequence_was_idle = self.active_touch_slots.is_empty();
        self.active_touch_slots.insert(slot);

        // Single-touch policy: only the first contact in an otherwise
        // idle sequence is forwarded to wl_touch or drives the gesture
        // state machine. The Goodix GT911 can report multiple slots,
        // but the widget set (utility gauges, dashboards) has no pinch,
        // rotate or pan gestures, and the scene-swipe arbitration only
        // drives the primary slot. Forwarding secondary slots would
        // produce wl_touch events with no matching down/cancel
        // lifecycle once the primary promotes to a scene drag. We
        // still track secondary slots in `active_touch_slots` so the
        // next sequence cannot start until every finger is lifted.
        //
        // Rework point for future multi-touch support: introduce
        // per-slot wl_touch lifecycle tracking here rather than
        // papering over the state machine downstream.
        if !sequence_was_idle {
            tracing::debug!(
                ?slot,
                primary_slot = ?self.gesture_slot,
                "ignoring non-primary touch slot: single-touch policy"
            );
            return;
        }

        let _ = self.event_tx.send(CompositorEvent::ScreenActivity);

        self.gesture_slot = Some(slot);
        self.gesture.on_down(location, time);
        self.scene_drag_active = false;

        let touch_handle = self.compositor.touch_handle.clone();
        let focus = self.compositor.touch_focus_at(location.x, location.y);
        touch_handle.down(
            &mut self.compositor,
            focus,
            &DownEvent {
                slot,
                time,
                location,
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

        let location = self.touch_location(event);
        let time = event.time_msec();
        let slot = event.slot();

        // Single-touch policy: drop motion from non-primary slots. See
        // `on_touch_down` for the full rationale.
        if self.gesture_slot != Some(slot) {
            return;
        }

        let drag_activated = self.gesture.on_motion(location, time);

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
                // Scene navigation and widget layout stay in integer logical
                // pixels; round once at the gesture/scene boundary.
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "drag offsets are panel-sized; fractional pixels round to i32"
                )]
                let dx = info.dx as i32;
                self.compositor.widgets.update_drag(dx);
                after_scene_change(self);
            }
        } else {
            let touch_handle = self.compositor.touch_handle.clone();
            let focus = self.compositor.touch_focus_at(location.x, location.y);
            touch_handle.motion(
                &mut self.compositor,
                focus,
                &MotionEvent {
                    slot,
                    time,
                    location,
                },
            );
            self.touch_frame_dirty = true;
        }
    }

    fn on_touch_up(&mut self, event: &impl TouchEventTrait<LibinputInputBackend>) {
        use smithay::input::touch::UpEvent;
        use smithay::utils::SERIAL_COUNTER;

        let time = event.time_msec();
        let slot = event.slot();
        self.active_touch_slots.remove(&slot);

        // Single-touch policy: only the primary slot produced any
        // wl_touch or gesture state on down/motion, so only the
        // primary needs finalization here. See `on_touch_down`.
        if self.gesture_slot != Some(slot) {
            return;
        }

        self.gesture_slot = None;
        let gesture_result = self.gesture.on_up(time);

        if self.scene_drag_active {
            self.scene_drag_active = false;
            if let Some(TouchGesture::DragEnd { dx, velocity_x }) = gesture_result {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "drag offsets are panel-sized; fractional pixels round to i32"
                )]
                let dx_px = dx as i32;
                let committed = self.compositor.widgets.end_drag(dx_px, velocity_x);
                after_scene_change(self);
                if committed {
                    tracing::info!(
                        "Scene transition committed (dx={:.1}, vel={:.0})",
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
                after_scene_change(self);
            }
        } else {
            let touch_handle = self.compositor.touch_handle.clone();
            touch_handle.up(
                &mut self.compositor,
                &UpEvent {
                    slot,
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
        self.gesture_slot = None;
        self.active_touch_slots.clear();

        if self.scene_drag_active {
            self.scene_drag_active = false;
            self.compositor.widgets.end_drag(0, 0.0);
            after_scene_change(self);
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

/// Attempt to register every `node` with `ctx` via `path_add_device`.
///
/// Returns the number of devices that libinput accepted. Callers drive
/// retry scheduling off a zero return — the function itself has no
/// notion of retries or timing.
fn register_touch_devices(ctx: &mut libinput::Libinput, nodes: &[PathBuf]) -> usize {
    let mut added = 0_usize;
    for node in nodes {
        let path_str = node.to_string_lossy();
        match ctx.path_add_device(&path_str) {
            Some(device) => {
                tracing::info!(
                    "libinput registered {} (name='{}', sysname='{}')",
                    node.display(),
                    device.name(),
                    device.sysname(),
                );
                added += 1;
            }
            None => {
                tracing::error!(
                    "libinput failed to register {}; touch input from this node is disabled",
                    node.display(),
                );
            }
        }
    }
    added
}

/// Wayland-side of lifecycle emission. Abstracts `send_lifecycle` +
/// per-client `flush` so [`emit_lifecycle_batches`] is testable
/// without an `AppState` — production wires it through
/// [`AppStateLifecycleSink`], tests through an in-memory recorder.
trait LifecycleSink {
    type ClientId: Clone + PartialEq;
    fn send(&mut self, instance_id: &InstanceId, state: LifecycleState) -> Option<Self::ClientId>;
    fn flush(&mut self, client_id: &Self::ClientId);
}

struct AppStateLifecycleSink<'a> {
    deck_widget_state: &'a mut DeckWidgetProtocolState,
    display: &'a mut Display<CompositorState>,
}

impl LifecycleSink for AppStateLifecycleSink<'_> {
    type ClientId = ClientId;
    fn send(&mut self, instance_id: &InstanceId, state: LifecycleState) -> Option<Self::ClientId> {
        self.deck_widget_state.send_lifecycle(instance_id, state)
    }
    fn flush(&mut self, client_id: &Self::ClientId) {
        if let Err(e) = self.display.backend().flush(Some(client_id.clone())) {
            tracing::warn!("lifecycle: flush for {client_id:?} failed: {e}");
        }
    }
}

/// Re-derive the lifecycle state of every widget from `WidgetTracker`,
/// step the `LifecycleEmitter`, and emit the resulting batches on each
/// widget's surface in the required order: release batch first (flush),
/// acquire batch second (flush). The flushes preserve the host's pool
/// ordering invariant on scene swaps and are scoped to only the clients
/// that actually received an event.
fn emit_lifecycle_transitions(state: &mut AppState) {
    let next = state.compositor.widgets.lifecycle_states();
    let emission = state.compositor.lifecycle.step(&next);

    if emission.is_empty() {
        return;
    }

    let mut sink = AppStateLifecycleSink {
        deck_widget_state: &mut state.compositor.deck_widget_state,
        display: &mut state.display,
    };
    emit_lifecycle_batches(&emission, &mut sink);
}

/// Pure ordering logic, split from [`emit_lifecycle_transitions`] so
/// tests can drive it with an in-memory sink. Sends the release batch,
/// flushes, then sends the acquire batch, flushes — the flush boundary
/// between batches is the contract the protocol XML documents.
fn emit_lifecycle_batches<S: LifecycleSink>(emission: &Emission, sink: &mut S) {
    let release_clients = send_lifecycle_batch(sink, &emission.releases, "release");
    flush_lifecycle_clients(sink, &release_clients);
    let acquire_clients = send_lifecycle_batch(sink, &emission.acquires, "acquire");
    flush_lifecycle_clients(sink, &acquire_clients);
}

fn send_lifecycle_batch<S: LifecycleSink>(
    sink: &mut S,
    batch: &[(InstanceId, LifecycleState)],
    label: &str,
) -> Vec<S::ClientId> {
    let mut clients: Vec<S::ClientId> = Vec::new();
    for (instance_id, lifecycle_state) in batch {
        tracing::debug!("lifecycle: {label} {instance_id} -> {lifecycle_state:?}");
        if let Some(client_id) = sink.send(instance_id, *lifecycle_state)
            && !clients.contains(&client_id)
        {
            clients.push(client_id);
        }
    }
    clients
}

fn flush_lifecycle_clients<S: LifecycleSink>(sink: &mut S, clients: &[S::ClientId]) {
    for client_id in clients {
        sink.flush(client_id);
    }
}

/// Clamp transitional states (Entering/Leaving) to their stable origins:
/// Entering -> Prepared (entered from Prepared), Leaving -> Visible (leaving from Visible).
/// Transitional states are only meaningful as deltas from a prior stable state,
/// which the widget never saw on initial emission.
fn clamp_initial_lifecycle(state: LifecycleState) -> LifecycleState {
    match state {
        LifecycleState::Entering => LifecycleState::Prepared,
        LifecycleState::Leaving => LifecycleState::Visible,
        LifecycleState::Dormant | LifecycleState::Prepared | LifecycleState::Visible => state,
        _ => panic!("BUG: LifecycleState enum only has 5 variants; all are explicitly covered"),
    }
}

/// Common tail of every scene mutation: invalidate the cached output and
/// fan out the resulting lifecycle transitions to widgets.
fn after_scene_change(state: &mut AppState) {
    state.compositor.mark_full_output_damage();
    emit_lifecycle_transitions(state);
}

fn handle_clear_pid_command(state: &mut AppState, instance_id: &InstanceId, expected_pid: u32) {
    tracing::debug!(
        "Clearing pid for widget {} (expected_pid={})",
        instance_id,
        expected_pid
    );
    if state
        .compositor
        .clear_pid_for_instance(instance_id, expected_pid)
        .is_some()
    {
        state.compositor.lifecycle.forget(instance_id);
    }
}

fn handle_command(state: &mut AppState, cmd: CompositorCommand) {
    match cmd {
        CompositorCommand::RegisterWidget {
            instance_id,
            position,
            size,
            initial_config,
            ack,
        } => {
            tracing::debug!(
                "Registering widget {} at ({}, {}) size {}x{} initial={:?}",
                instance_id,
                position.x,
                position.y,
                size.width,
                size.height,
                initial_config
            );
            state
                .compositor
                .deck_widget_state
                .register_widget(instance_id, initial_config);
            let _ = ack.send(());
        }
        CompositorCommand::SetWidgetPid {
            instance_id,
            pid,
            ack,
        } => {
            tracing::debug!("Associating pid {} with widget {}", pid, instance_id);
            state
                .compositor
                .deck_widget_state
                .set_widget_pid(&instance_id, pid);
            let _ = ack.send(());
        }
        CompositorCommand::UnregisterWidget { instance_id } => {
            tracing::debug!("Unregistering widget {}", instance_id);
            state.compositor.unregister_widget(&instance_id);
            state.compositor.lifecycle.forget(&instance_id);
        }
        CompositorCommand::ClearPid {
            instance_id,
            expected_pid,
        } => handle_clear_pid_command(state, &instance_id, expected_pid),
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
            after_scene_change(state);
        }
        CompositorCommand::SetSceneCycling { scenes } => {
            tracing::info!("Setting scene cycling with {} scenes", scenes.len());
            state.compositor.widgets.set_scene_cycling(scenes);
            after_scene_change(state);
        }
        CompositorCommand::SetActiveSceneIndex { index } => {
            tracing::info!("Setting active scene index to {}", index);
            state.compositor.widgets.set_active_scene_index(index);
            after_scene_change(state);
        }
        CompositorCommand::BroadcastSetting { setting } => {
            tracing::debug!("Broadcasting setting: {:?}", setting);
            state
                .compositor
                .deck_widget_state
                .broadcast_setting(&setting);
        }
        CompositorCommand::UpdateWidgetParams {
            instance_id,
            params,
        } => {
            tracing::debug!("Updating widget params: {instance_id}");
            state
                .compositor
                .deck_widget_state
                .update_widget_params(&instance_id, params);
        }
        CompositorCommand::Shutdown => {
            tracing::info!("Shutdown command received");
            state.compositor.deck_widget_state.broadcast_shutdown();
            state.should_exit = true;
        }
    }
}

fn process_protocol_events(state: &mut AppState) {
    let connected = state.compositor.deck_widget_state.drain_connected();
    if !connected.is_empty() {
        let lifecycle_states = state.compositor.widgets.lifecycle_states();
        let mut connect_clients: Vec<ClientId> = Vec::new();

        for instance_id in connected {
            let lifecycle_state = clamp_initial_lifecycle(
                lifecycle_states
                    .get(&instance_id)
                    .copied()
                    .unwrap_or(LifecycleState::Dormant),
            );
            match state
                .compositor
                .send_initial_lifecycle(&instance_id, lifecycle_state)
            {
                Some(client_id) => {
                    if !connect_clients.contains(&client_id) {
                        connect_clients.push(client_id);
                    }
                }
                None => {
                    tracing::warn!(
                        "Widget {instance_id} connected without surface; \
                         deferring lifecycle({lifecycle_state:?}) to next emitter step"
                    );
                }
            }

            tracing::info!("Widget connected: {}", instance_id);
            let _ = state
                .event_tx
                .send(CompositorEvent::WidgetReady { instance_id });
        }
        let mut sink = AppStateLifecycleSink {
            deck_widget_state: &mut state.compositor.deck_widget_state,
            display: &mut state.display,
        };
        flush_lifecycle_clients(&mut sink, &connect_clients);
    }

    for instance_id in state.compositor.deck_widget_state.drain_disconnected() {
        tracing::info!("Widget disconnected: {}", instance_id);
        let _ = state
            .event_tx
            .send(CompositorEvent::WidgetDisconnected { instance_id });
    }

    for (instance_id, payload) in state.compositor.deck_widget_state.drain_actions() {
        tracing::debug!("Widget {} action: {:?}", instance_id, payload);
        let _ = state.action_tx.send(WidgetAction {
            instance_id,
            payload,
        });
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

        let render_node = self
            .device_access
            .render_node()
            .map_or_else(|| self.profile.paths.render_node.clone(), Path::to_path_buf);
        let scanout_node = self.device_access.scanout_node().map_or_else(
            || self.profile.paths.scanout_node.clone(),
            Path::to_path_buf,
        );
        let seat_name = self.device_access.seat_name().to_owned();
        let has_explicit_input_nodes = self.device_access.has_explicit_input_nodes();
        let input_nodes = self.device_access.resolved_input_nodes();
        let profile = self.profile.clone();
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
                    &render_node,
                    &scanout_node,
                    &seat_name,
                    &input_nodes,
                    has_explicit_input_nodes,
                    &profile,
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
            .recv_timeout(COMPOSITOR_READY_TIMEOUT)
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

    fn hardware_capabilities(&self) -> bmc_platform::HardwareCapabilities {
        self.profile.capabilities()
    }

    fn register_widget(
        &self,
        instance_id: InstanceId,
        position: Position,
        size: Size,
        initial_config: WidgetInitialConfig,
    ) -> Result<(), CompositorError> {
        let (ack_tx, ack_rx) = flume::bounded(1);
        self.command_tx
            .send(CompositorCommand::RegisterWidget {
                instance_id,
                position,
                size,
                initial_config,
                ack: ack_tx,
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))?;
        ack_rx
            .recv_timeout(WIDGET_COMMAND_ACK_TIMEOUT)
            .map_err(|e| CompositorError::ThreadError(format!("register_widget ack: {e}")))
    }

    fn set_widget_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError> {
        let (ack_tx, ack_rx) = flume::bounded(1);
        self.command_tx
            .send(CompositorCommand::SetWidgetPid {
                instance_id: instance_id.clone(),
                pid,
                ack: ack_tx,
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))?;
        ack_rx
            .recv_timeout(WIDGET_COMMAND_ACK_TIMEOUT)
            .map_err(|e| CompositorError::ThreadError(format!("set_widget_pid ack: {e}")))
    }

    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::UnregisterWidget {
                instance_id: instance_id.clone(),
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn clear_pid(&self, instance_id: &InstanceId, pid: u32) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::ClearPid {
                instance_id: instance_id.clone(),
                expected_pid: pid,
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

    fn set_active_scene_index(&self, index: usize) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetActiveSceneIndex { index })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_setting(&self, setting: SettingUpdate) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::BroadcastSetting { setting })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn update_widget_params(
        &self,
        instance_id: &InstanceId,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::UpdateWidgetParams {
                instance_id: instance_id.clone(),
                params,
            })
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
    use super::{
        AppState, CompositorState, Emission, GestureState, LifecycleSink, LifecycleState,
        RedrawState, clamp_initial_lifecycle, dispatch_timeout, emit_lifecycle_batches,
        handle_clear_pid_command,
    };
    use bmc::compositor::InstanceId;
    use bmc_widget_protocol::{ViewportShape, WidgetInitialConfig};
    use smithay::reexports::{
        calloop::EventLoop,
        wayland_server::{Display, ListeningSocket},
    };
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::mpsc;

    fn make_widget_config() -> WidgetInitialConfig {
        WidgetInitialConfig {
            width: 100,
            height: 100,
            viewport_shape: ViewportShape::Rectangular,
            display: bmc_widget_protocol::DisplayInfo::BMC100,
            params: serde_json::Map::new(),
        }
    }

    fn make_test_socket_path() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("BUG: system time should be after Unix epoch")
            .as_nanos();
        let dir = PathBuf::from("/tmp/claude-1001/bmc-openwrt-tests");
        std::fs::create_dir_all(&dir).expect("BUG: test socket directory should be creatable");
        dir.join(format!("clear-pid-{timestamp}-{}", std::process::id()))
    }

    fn make_app_state() -> AppState {
        let event_loop: EventLoop<'static, AppState> =
            EventLoop::try_new().expect("BUG: test event loop should initialize");
        let display: Display<CompositorState> =
            Display::new().expect("BUG: test Wayland display should initialize");
        let compositor = CompositorState::new(&display, 480, 1280, 480, 1280, 60_000, "test-seat");
        let listening_socket = ListeningSocket::bind_absolute(make_test_socket_path())
            .expect("BUG: test Wayland socket should bind");
        let (action_tx, _) = mpsc::unbounded_channel();
        let (event_tx, _) = mpsc::unbounded_channel();

        AppState {
            display,
            compositor,
            scene_renderer: None,
            listening_socket,
            action_tx,
            event_tx,
            gesture: GestureState::new(),
            gesture_slot: None,
            active_touch_slots: HashSet::new(),
            scene_drag_active: false,
            touch_frame_dirty: false,
            logical_width: 480,
            logical_height: 1280,
            touch_transform: bmc_platform::TouchTransform::Deg0,
            should_exit: false,
            redraw_state: RedrawState::Idle,
            loop_handle: event_loop.handle(),
            retry_libinput: None,
            touch_retry_pending: false,
        }
    }

    fn lifecycle_map(
        pairs: &[(InstanceId, LifecycleState)],
    ) -> HashMap<InstanceId, LifecycleState> {
        pairs.iter().cloned().collect()
    }

    /// Records `send`/`flush` calls in order. `send` returns a synthetic
    /// per-instance `ClientId` so the test can verify that the flush
    /// boundary between batches refers to the same clients that received
    /// the preceding batch.
    #[derive(Default)]
    struct RecordingSink {
        events: Vec<RecordedEvent>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum RecordedEvent {
        Send(InstanceId, LifecycleState),
        Flush(String),
    }

    impl LifecycleSink for RecordingSink {
        type ClientId = String;
        fn send(
            &mut self,
            instance_id: &InstanceId,
            state: LifecycleState,
        ) -> Option<Self::ClientId> {
            self.events
                .push(RecordedEvent::Send(instance_id.clone(), state));
            Some(format!("client-{instance_id}"))
        }
        fn flush(&mut self, client_id: &Self::ClientId) {
            self.events.push(RecordedEvent::Flush(client_id.clone()));
        }
    }

    #[test]
    fn emit_releases_flushes_then_acquires_flushes() {
        let emission = Emission {
            releases: vec![
                (String::from("a"), LifecycleState::Dormant),
                (String::from("b"), LifecycleState::Dormant),
            ],
            acquires: vec![
                (String::from("c"), LifecycleState::Visible),
                (String::from("d"), LifecycleState::Prepared),
            ],
        };

        let mut sink = RecordingSink::default();
        emit_lifecycle_batches(&emission, &mut sink);

        assert_eq!(
            sink.events,
            vec![
                RecordedEvent::Send(String::from("a"), LifecycleState::Dormant),
                RecordedEvent::Send(String::from("b"), LifecycleState::Dormant),
                RecordedEvent::Flush(String::from("client-a")),
                RecordedEvent::Flush(String::from("client-b")),
                RecordedEvent::Send(String::from("c"), LifecycleState::Visible),
                RecordedEvent::Send(String::from("d"), LifecycleState::Prepared),
                RecordedEvent::Flush(String::from("client-c")),
                RecordedEvent::Flush(String::from("client-d")),
            ],
            "release sends must precede release flushes, which must precede acquire sends",
        );
    }

    #[test]
    fn empty_release_batch_still_flushes_between_zero_releases_and_acquires() {
        // First-step case: nothing to release, but acquires still need
        // a flush after them. Asserts that no flush sneaks in *between*
        // an empty batch and the acquire sends.
        let emission = Emission {
            releases: vec![],
            acquires: vec![(String::from("a"), LifecycleState::Visible)],
        };

        let mut sink = RecordingSink::default();
        emit_lifecycle_batches(&emission, &mut sink);

        assert_eq!(
            sink.events,
            vec![
                RecordedEvent::Send(String::from("a"), LifecycleState::Visible),
                RecordedEvent::Flush(String::from("client-a")),
            ],
        );
    }

    #[test]
    fn same_client_across_release_and_acquire_is_flushed_in_each_batch() {
        // A widget that releases (Visible -> Dormant) and a different
        // widget that acquires (Dormant -> Visible) on the same client
        // connection: the client must be flushed in the release batch
        // before it sees any acquire send, so the pool can be reclaimed.
        struct SingleClientSink {
            events: Vec<RecordedEvent>,
        }
        impl LifecycleSink for SingleClientSink {
            type ClientId = String;
            fn send(
                &mut self,
                instance_id: &InstanceId,
                state: LifecycleState,
            ) -> Option<Self::ClientId> {
                self.events
                    .push(RecordedEvent::Send(instance_id.clone(), state));
                Some(String::from("shared-client"))
            }
            fn flush(&mut self, client_id: &Self::ClientId) {
                self.events.push(RecordedEvent::Flush(client_id.clone()));
            }
        }

        let emission = Emission {
            releases: vec![(String::from("a"), LifecycleState::Dormant)],
            acquires: vec![(String::from("b"), LifecycleState::Visible)],
        };

        let mut sink = SingleClientSink { events: Vec::new() };
        emit_lifecycle_batches(&emission, &mut sink);

        let flush_after_release = sink
            .events
            .iter()
            .position(|e| matches!(e, RecordedEvent::Flush(_)))
            .expect("BUG: release flush should be recorded");
        let acquire_send = sink
            .events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    RecordedEvent::Send(id, LifecycleState::Visible) if id == "b"
                )
            })
            .expect("BUG: acquire send should be recorded");
        assert!(
            flush_after_release < acquire_send,
            "release flush must happen before acquire send for the same client; \
             otherwise the host can't reclaim the pool buffer before the new one is requested",
        );
    }

    #[test]
    fn stale_clear_pid_keeps_lifecycle_history_for_live_respawned_widget() {
        let mut state = make_app_state();
        let instance_id = String::from("alpha");
        state
            .compositor
            .deck_widget_state
            .register_widget(instance_id.clone(), make_widget_config());
        state
            .compositor
            .deck_widget_state
            .set_widget_pid(&instance_id, 200);
        let _ = state.compositor.deck_widget_state.drain_connected();

        let _ = state.compositor.lifecycle.step(&lifecycle_map(&[(
            instance_id.clone(),
            LifecycleState::Visible,
        )]));

        handle_clear_pid_command(&mut state, &instance_id, 100);

        assert!(
            state
                .compositor
                .deck_widget_state
                .drain_disconnected()
                .is_empty(),
            "stale clear must not unregister the live respawned widget",
        );

        let emission = state.compositor.lifecycle.step(&lifecycle_map(&[(
            instance_id.clone(),
            LifecycleState::Dormant,
        )]));
        assert_eq!(
            emission.releases,
            vec![(instance_id, LifecycleState::Dormant)],
            "stale clear must preserve the last emitted lifecycle state so the next Dormant transition releases render targets",
        );
    }

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

    #[test]
    fn clamp_initial_lifecycle_maps_entering_to_prepared_and_leaving_to_visible() {
        use LifecycleState::*;
        assert_eq!(clamp_initial_lifecycle(Entering), Prepared);
        assert_eq!(clamp_initial_lifecycle(Leaving), Visible);
        assert_eq!(clamp_initial_lifecycle(Dormant), Dormant);
        assert_eq!(clamp_initial_lifecycle(Prepared), Prepared);
        assert_eq!(clamp_initial_lifecycle(Visible), Visible);
    }
}
