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

//! EGL Compositor implementation for bmc-openwrt.

use super::scene_renderer::{logo_for_product, touch_to_logical};
use super::{
    commands::CompositorCommand,
    device_access::{DeviceAccessConfig, RootLibinputInterface, set_libinput_debug_priority},
    lifecycle_emitter::Emission,
    protocol::{DeckWidgetHandler, DeckWidgetProtocolState},
    render::{DrmOutput, EglContext},
    scene_cycling::{
        AutomaticCycling, AutomaticCyclingAction, AutomaticCyclingPhase, SceneCyclingRuntimeConfig,
    },
    scene_renderer::SceneRenderer,
    state::{ClientState, CompositorState, release_widget_buffers},
    touch_gesture::{GestureBorder, GestureConfig, GestureState, MotionActivation, TouchGesture},
    widget_tracker::{LifecycleState, SceneTransitionTarget},
};
use bmc::compositor::{
    ActiveScene, AlarmCommand, Compositor, CompositorError, CompositorEvent, InstanceId,
    LedRequestStatusEvent, Position, SceneLayout, SettingsCommand, Size,
    WIDGET_COMMAND_ACK_TIMEOUT, WidgetAction, WidgetGeneration, WidgetInstanceKey,
    WidgetRegistration,
};
use bmc_platform::TouchTransform;
use bmc_platform::backlight::ScreenVisibility;
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
    wayland_server::{
        Display, ListeningSocket,
        backend::{ClientId, ObjectId},
        protocol::wl_buffer::WlBuffer,
    },
};
use smithay::utils::{Logical, Point};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    num::NonZeroU64,
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
use tokio::sync::{broadcast, mpsc, watch};

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
const TRANSITION_WARM_UP_RETRY: Duration = Duration::from_millis(16);
// Device profiling observed valid incoming renders taking up to 615 ms.
const TRANSITION_WARM_UP_TIMEOUT: Duration = Duration::from_secs(1);

/// How often, while an alarm is ringing, the loop probes for a live overlay so
/// a mid-ring overlay crash is noticed promptly (see `on_alarm_fallback_tick`).
const ALARM_FALLBACK_POLL: Duration = Duration::from_secs(1);
/// How long an alarm may ring with no live overlay before the compositor
/// auto-dismisses it — the no-overlay fallback grace. Long enough for a
/// crashed overlay to re-bind and replay before we give up on the UI.
const ALARM_FALLBACK_GRACE: Duration = Duration::from_secs(10);

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

/// Frame-callback timestamp for the current pass. Prefers the kernel-
/// delivered vblank timestamp (`CLOCK_MONOTONIC`, the actual presentation
/// time); falls back to a process-local monotonic reference for headless
/// mode and the pre-first-vblank window on hardware — Wayland only requires
/// monotonic ms from an unspecified epoch, and these two sources combined
/// satisfy that without sampling `SystemTime` (not monotonic; can jump
/// backwards on NTP adjustments).
#[expect(
    clippy::cast_possible_truncation,
    reason = "wrapping at ~49.7 days is acceptable for frame-callback time"
)]
fn frame_callback_time_ms(vblank_ms: Option<u32>) -> u32 {
    vblank_ms.unwrap_or_else(|| COMPOSITOR_BOOT.elapsed().as_millis() as u32)
}

pub struct EglCompositor {
    wayland_display: Mutex<Option<String>>,
    command_tx: calloop_channel::Sender<CompositorCommand>,
    command_channel: Mutex<Option<calloop_channel::Channel<CompositorCommand>>>,
    action_tx: mpsc::UnboundedSender<WidgetAction>,
    event_tx: broadcast::Sender<CompositorEvent>,
    /// `status_rx` is taken into the compositor thread on
    /// [`Self::start`]; `status_tx` is cloned out via
    /// [`Self::request_status_sender`].
    status_tx: mpsc::UnboundedSender<LedRequestStatusEvent>,
    status_rx: Mutex<Option<mpsc::UnboundedReceiver<LedRequestStatusEvent>>>,
    /// Latest active scene; consumers subscribe via [`Self::active_scene_watch`].
    active_scene_tx: watch::Sender<Option<ActiveScene>>,
    /// Currently connected widgets; consumers subscribe via
    /// [`Self::connected_widgets_watch`].
    connected_widgets_tx: watch::Sender<BTreeSet<InstanceId>>,
    action_rx: Mutex<Option<mpsc::UnboundedReceiver<WidgetAction>>>,
    settings_tx: mpsc::UnboundedSender<SettingsCommand>,
    settings_rx: Mutex<Option<mpsc::UnboundedReceiver<SettingsCommand>>>,
    alarm_tx: mpsc::UnboundedSender<AlarmCommand>,
    alarm_rx: Mutex<Option<mpsc::UnboundedReceiver<AlarmCommand>>>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    device_access: DeviceAccessConfig,
    profile: bmc_platform::HardwareProfile,
    headless: bool,
    screen_visibility: Option<Arc<dyn ScreenVisibility>>,
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
        let (event_tx, _) = broadcast::channel(64);
        let (status_tx, status_rx) = mpsc::unbounded_channel();
        let (active_scene_tx, _) = watch::channel(None);
        let (connected_widgets_tx, _) = watch::channel(BTreeSet::new());
        let (settings_tx, settings_rx) = mpsc::unbounded_channel();
        let (alarm_tx, alarm_rx) = mpsc::unbounded_channel();

        Self {
            wayland_display: Mutex::new(None),
            command_tx,
            command_channel: Mutex::new(Some(command_channel)),
            action_tx,
            event_tx,
            status_tx,
            status_rx: Mutex::new(Some(status_rx)),
            active_scene_tx,
            connected_widgets_tx,
            action_rx: Mutex::new(Some(action_rx)),
            settings_tx,
            settings_rx: Mutex::new(Some(settings_rx)),
            alarm_tx,
            alarm_rx: Mutex::new(Some(alarm_rx)),
            thread_handle: Mutex::new(None),
            device_access,
            profile,
            headless,
            screen_visibility: None,
        }
    }

    /// Attach the panel-visibility source consulted before a touch is
    /// delivered. Leave it unset — as the tests do — and every touch counts as
    /// landing on a lit screen.
    #[must_use]
    pub fn with_screen_visibility(mut self, screen_visibility: Arc<dyn ScreenVisibility>) -> Self {
        self.screen_visibility = Some(screen_visibility);
        self
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
        screen_visibility: Option<Arc<dyn ScreenVisibility>>,
        command_channel: calloop_channel::Channel<CompositorCommand>,
        action_tx: mpsc::UnboundedSender<WidgetAction>,
        event_tx: broadcast::Sender<CompositorEvent>,
        settings_tx: mpsc::UnboundedSender<SettingsCommand>,
        alarm_tx: mpsc::UnboundedSender<AlarmCommand>,
        status_rx: mpsc::UnboundedReceiver<LedRequestStatusEvent>,
        active_scene_tx: watch::Sender<Option<ActiveScene>>,
        connected_widgets_tx: watch::Sender<BTreeSet<InstanceId>>,
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
                    logo_for_product(profile.product),
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
            super::settings::caps_for_product(profile.product),
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
            settings_tx,
            alarm_tx,
            status_rx,
            active_scene_tx,
            connected_widgets_tx,
            connected_widgets: BTreeSet::new(),
            gesture: GestureState::with_config(GestureConfig {
                screen_height: Some(f64::from(logical_height)),
                ..GestureConfig::default()
            }),
            gesture_slot: None,
            active_touch_slots: HashSet::new(),
            screen_visibility,
            scene_drag_active: false,
            edge_reveal_active: false,
            touch_frame_dirty: false,
            logical_width,
            logical_height,
            touch_transform: display_profile.touch_transform,
            should_exit: false,
            redraw_state: RedrawState::Idle,
            pending_lifecycle_emission: false,
            loop_handle: loop_handle.clone(),
            retry_libinput: None,
            touch_retry_pending: false,
            automatic_cycling: AutomaticCycling::new(
                Instant::now(),
                SceneCyclingRuntimeConfig::default(),
            ),
            pending_transition_warm_up: None,
            scene_cycling_timer_generation: 0,
            alarm_fallback_generation: 0,
            alarm_no_overlay_since: None,
            last_neighbors_suppressed: false,
            last_modal_overlay_active: false,
        };
        app_state.reevaluate_automatic_cycling(Instant::now());

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

        // Always-on tick that evaluates pending widget frame callbacks and,
        // in headless mode, also drives the redraw state machine in place
        // of real DRM vblank events. Keeping a single chokepoint here for
        // the widget callback path — rather than one timer per mode plus
        // post-render firing — avoids compounding call sites competing to
        // fire the same callbacks. Layer-shell callbacks fire from the
        // render path instead (see the `if rendered` block and the headless
        // branch below), so they mean "consumed by a repaint", not "a timer
        // tick elapsed".
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

            let time = frame_callback_time_ms(
                state
                    .scene_renderer
                    .as_ref()
                    .and_then(|r| r.output().last_vblank_ms()),
            );
            if !state.widget_frame_callbacks_suppressed() {
                state
                    .compositor
                    .send_frame_callbacks_for_presented_widgets(time);
            }

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

            // Layer-shell map/unmap and screen-edge reveal happen during the
            // previous iteration's dispatch, not on a scene command, so
            // lifecycle does not re-emit on its own. Detect the combined
            // suppression predicate flipping and re-emit so scene-swipe
            // neighbors are demoted to `Dormant` when a blocker maps or an
            // edge reveals, then restored to `Prepared` when it goes away.
            let suppressed = app_state.compositor.neighbors_suppressed();
            if suppressed != app_state.last_neighbors_suppressed {
                app_state.last_neighbors_suppressed = suppressed;
                emit_lifecycle_transitions(&mut app_state);
            }

            // A modal full-screen overlay (alarm, startup) mapping or unmapping
            // is not a scene command either, so — like the suppression check
            // above — detect the edge here and tell the settings-tray to retract
            // via `deck_settings_v1.preempted`. Generic: any full-screen
            // preempting overlay drives this, so new modal overlays need no
            // wiring, and the tray no longer binds `deck_alarm_v1`.
            let modal_active = app_state.compositor.modal_overlay_active();
            if modal_active != app_state.last_modal_overlay_active {
                app_state.last_modal_overlay_active = modal_active;
                app_state.compositor.settings.set_preempted(modal_active);
            }

            app_state.refresh_redraw_state();

            // Whether the committed scene's frame was handled this iteration —
            // rendered on the GPU, or damage-processed in headless mode. Gates
            // the deferred lifecycle flush below so transitions go out only
            // after the frame is handled.
            let mut frame_handled = false;

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
                    let logical_width = renderer.logical_size().0;
                    let transition_frame = app_state
                        .automatic_cycling
                        .transition_frame(logical_width, Instant::now());
                    let layer_items = app_state.compositor.layer_render_items();
                    let (rendered, unconsumed_captures, capture_failed) = renderer
                        .render_scene(
                            &app_state.compositor.widgets,
                            transition_frame,
                            &app_state.compositor.widget_buffers,
                            &layer_items,
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

                    if rendered {
                        frame_handled = true;
                        // A layer commit marks output damage, so a queued
                        // callback always has a render coming; firing it
                        // here means "this render sampled and submitted the
                        // surface's buffer", matching core-protocol
                        // `wl_surface.frame` semantics instead of an
                        // unrelated timer tick.
                        let time = frame_callback_time_ms(renderer.output().last_vblank_ms());
                        app_state.compositor.send_layer_frame_callbacks(time);
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
                // No GPU handoff to wait for; the damage clear is the headless
                // equivalent of handling the committed scene's frame, so
                // layer callbacks fire here too — otherwise they would
                // starve with no render path to drain them.
                app_state
                    .compositor
                    .send_layer_frame_callbacks(frame_callback_time_ms(None));
                frame_handled = true;
            }

            // Flush lifecycle transitions armed by a scene change — but only
            // after a render this iteration, so the host starts re-rendering
            // once the compositor's frame for the committed scene is done.
            if frame_handled {
                emit_pending_lifecycle(&mut app_state);
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
    event_tx: broadcast::Sender<CompositorEvent>,
    settings_tx: mpsc::UnboundedSender<SettingsCommand>,
    alarm_tx: mpsc::UnboundedSender<AlarmCommand>,
    status_rx: mpsc::UnboundedReceiver<LedRequestStatusEvent>,
    /// Latest active scene, published to consumers via a `watch` channel.
    active_scene_tx: watch::Sender<Option<ActiveScene>>,
    /// Connected-widget set, published to consumers via a `watch` channel.
    connected_widgets_tx: watch::Sender<BTreeSet<InstanceId>>,
    /// Mirror of the set last published on `connected_widgets_tx`, updated
    /// as widgets connect and disconnect.
    connected_widgets: BTreeSet<InstanceId>,
    /// Backend-agnostic gesture state machine, driven by libinput events.
    gesture: GestureState,
    /// Touch slot that owns compositor-level scene arbitration for the
    /// current sequence. Only the first contact in an otherwise idle
    /// sequence can turn into a scene swipe.
    gesture_slot: Option<TouchSlot>,
    /// All currently active libinput touch slots.
    active_touch_slots: HashSet<TouchSlot>,
    /// Panel visibility, read from the backlight driver on demand. While the
    /// panel is dark the primary touch-down is consumed right after it emits
    /// `ScreenActivity`: the touch wakes the screen but must not reach gesture
    /// arbitration or `wl_touch` delivery.
    ///
    /// Deliberately not a `bool` mirrored from a screen-power command to keep
    /// the backlight driver a single source of the truth.
    screen_visibility: Option<Arc<dyn ScreenVisibility>>,
    /// `true` once the current touch has been arbitrated to scene-drag
    /// mode; cleared on [`GestureState::on_up`] / cancel. Separate from
    /// `gesture.drag_active()` so the cancel-on-first-drag-sample and
    /// end-drag-on-release transitions are unambiguous even when the
    /// gesture state machine resets `drag_active` mid-handler.
    scene_drag_active: bool,
    /// `true` once the current sequence is claimed by edge reveal;
    /// subsequent motion/up are owned by reveal until lift.
    edge_reveal_active: bool,
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
    /// Set by `after_scene_change` when a scene mutation needs its lifecycle
    /// transitions sent to widgets. The emission is deferred until the
    /// compositor has rendered one frame of the committed scene, so the host
    /// only starts re-rendering after the compositor's GPU work for that frame
    /// is done — avoiding a host/compositor render overlap across the handoff.
    pending_lifecycle_emission: bool,
    automatic_cycling: AutomaticCycling,
    pending_transition_warm_up: Option<TransitionWarmUp>,
    scene_cycling_timer_generation: u64,
    /// Generation guard for the alarm no-overlay fallback watchdog timer; a
    /// bump invalidates any in-flight timer (mirrors `scene_cycling_timer_generation`).
    alarm_fallback_generation: u64,
    /// When the ringing alarm was first observed with no live overlay; `None`
    /// while an overlay is present. Drives the `ALARM_FALLBACK_GRACE` deadline.
    alarm_no_overlay_since: Option<Instant>,
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
    /// Last observed value of [`CompositorState::neighbors_suppressed`]. A layer
    /// map/unmap or screen-edge reveal happens during dispatch, not on a scene
    /// command, so lifecycle would not re-emit on its own. Comparing against
    /// this each loop iteration lets us re-emit when the predicate flips,
    /// demoting scene-swipe neighbors to `Dormant` when an overlay maps or a
    /// screen edge reveals, and restoring them when it goes away.
    last_neighbors_suppressed: bool,
    /// Last observed value of [`CompositorState::modal_overlay_active`]. A modal
    /// full-screen overlay maps/unmaps during dispatch, not on a scene command,
    /// so — like `last_neighbors_suppressed` — comparing against this each loop
    /// iteration lets us emit `deck_settings_v1.preempted` only on the edge, so
    /// the settings-tray retracts when an alarm (or other modal overlay) covers
    /// the screen and is told when it clears.
    last_modal_overlay_active: bool,
}

impl AppState {
    /// Keep outgoing widgets animated during warm-up.
    /// Suppress callbacks during scene motion to avoid contention with compositor work.
    /// Suppressed callbacks remain queued.
    fn widget_frame_callbacks_suppressed(&self) -> bool {
        self.compositor.widgets.drag_offset().is_some()
            || matches!(
                self.automatic_cycling.phase(),
                AutomaticCyclingPhase::Transition { .. }
            )
    }

    fn active_scene_cycle_duration(&self) -> Duration {
        self.compositor
            .widgets
            .active_scene()
            .cycle_duration
            .unwrap_or(self.automatic_cycling.default_duration())
    }

    fn reevaluate_automatic_cycling(&mut self, now: Instant) {
        let transition_was_active = matches!(
            self.automatic_cycling.phase(),
            AutomaticCyclingPhase::PreTransition { .. } | AutomaticCyclingPhase::Transition { .. }
        );
        let touch_active = !self.active_touch_slots.is_empty();
        if touch_active {
            self.cancel_automatic_transition_for_interruption();
        }
        self.automatic_cycling.reevaluate(
            now,
            self.compositor.widgets.can_drag(),
            self.compositor.widgets.scene_count(),
            touch_active,
        );
        if transition_was_active
            && !matches!(
                self.automatic_cycling.phase(),
                AutomaticCyclingPhase::PreTransition { .. }
                    | AutomaticCyclingPhase::Transition { .. }
            )
        {
            if self.compositor.widgets.automatic_transition_active() {
                self.cancel_automatic_transition_for_interruption();
            } else {
                self.finish_automatic_transition_interruption();
            }
        }
        self.schedule_scene_cycling_timer(now);
    }

    fn reset_automatic_waiting(&mut self, now: Instant) {
        self.cancel_automatic_transition_for_interruption();
        self.automatic_cycling.reset_waiting(
            now,
            self.compositor.widgets.scene_count(),
            !self.active_touch_slots.is_empty(),
        );
        self.schedule_scene_cycling_timer(now);
    }

    fn pause_automatic_cycling_for_touch(&mut self, now: Instant) {
        self.cancel_automatic_transition_for_interruption();
        self.automatic_cycling.reevaluate(
            now,
            self.compositor.widgets.can_drag(),
            self.compositor.widgets.scene_count(),
            true,
        );
        self.schedule_scene_cycling_timer(now);
    }

    fn cancel_automatic_transition_for_interruption(&mut self) {
        if !self.compositor.widgets.automatic_transition_active() {
            return;
        }

        self.compositor.widgets.cancel_automatic_transition();
        self.finish_automatic_transition_interruption();
        self.compositor.mark_full_output_damage();
        if self.pending_lifecycle_emission {
            return;
        }

        let mut next = self.compositor.widgets.lifecycle_states();
        if self.compositor.neighbors_suppressed() {
            // Match emit_lifecycle_transitions so the release peek uses the
            // same Prepared -> Dormant neighbor suppression as the real emit.
            crate::compositor::layer_surface::suppress_prepared(&mut next);
        }
        let mut lifecycle = self.compositor.lifecycle.clone();
        let emission = lifecycle.step(&next);
        if !emission.releases.is_empty() {
            self.pending_lifecycle_emission = true;
            return;
        }

        let emission = emit_lifecycle_transitions(self);
        debug_assert!(
            emission.releases.is_empty(),
            "BUG: reverting an automatic transition restores widgets to \
             Visible/Prepared and must not release dormant buffers",
        );
    }

    fn discard_automatic_transition_for_scene_replacement(&mut self) {
        self.compositor.widgets.cancel_automatic_transition();
        self.finish_automatic_transition_interruption();
    }

    fn finish_automatic_transition_interruption(&mut self) {
        self.pending_transition_warm_up = None;
    }

    fn schedule_scene_cycling_timer(&mut self, now: Instant) {
        let delay = self
            .automatic_cycling
            .next_delay(now, self.active_scene_cycle_duration());
        self.rearm_scene_cycling_timer(delay);
    }

    /// Invalidate any pending scene-cycling timer (by bumping the
    /// generation) and, when `delay` is `Some`, arm a fresh one. A `None`
    /// delay leaves cycling idle — the generation bump alone drops the old
    /// timer so a paused machine stops firing.
    fn rearm_scene_cycling_timer(&mut self, delay: Option<Duration>) {
        debug_assert_eq!(
            self.compositor.widgets.automatic_transition_active(),
            matches!(
                self.automatic_cycling.phase(),
                AutomaticCyclingPhase::PreTransition { .. }
                    | AutomaticCyclingPhase::Transition { .. }
            ),
            "BUG: tracker transition and cycling phase disagree"
        );
        self.scene_cycling_timer_generation = self.scene_cycling_timer_generation.saturating_add(1);
        let Some(delay) = delay else {
            return;
        };
        let generation = self.scene_cycling_timer_generation;
        let timer = Timer::from_duration(delay);
        let result = self.loop_handle.insert_source(timer, move |_, (), state| {
            if state.scene_cycling_timer_generation != generation {
                return TimeoutAction::Drop;
            }
            state.on_scene_cycling_timer(Instant::now());
            TimeoutAction::Drop
        });
        if let Err(e) = result {
            tracing::error!("failed to schedule scene cycling timer: {e}");
        }
    }

    /// Arm the no-overlay fallback watchdog for a freshly-ringing alarm. Seeds
    /// `alarm_no_overlay_since` when no overlay is bound at fire time so that
    /// case dismisses exactly `ALARM_FALLBACK_GRACE` later, then polls every
    /// `ALARM_FALLBACK_POLL` to catch a mid-ring overlay crash. Bumping the
    /// generation drops any previously-armed watchdog.
    fn arm_alarm_fallback(&mut self) {
        self.alarm_no_overlay_since =
            (!self.compositor.alarm.has_live_overlay()).then(Instant::now);
        self.alarm_fallback_generation = self.alarm_fallback_generation.saturating_add(1);
        let generation = self.alarm_fallback_generation;
        let timer = Timer::from_duration(ALARM_FALLBACK_POLL);
        let result = self.loop_handle.insert_source(timer, move |_, (), state| {
            if generation != state.alarm_fallback_generation || !state.compositor.alarm.is_ringing()
            {
                return TimeoutAction::Drop;
            }
            if state.on_alarm_fallback_tick(Instant::now()) {
                TimeoutAction::ToDuration(ALARM_FALLBACK_POLL)
            } else {
                TimeoutAction::Drop
            }
        });
        if let Err(e) = result {
            tracing::error!("failed to schedule alarm fallback timer: {e}");
        }
    }

    /// Invalidate the fallback watchdog (alarm stopped / dismissed elsewhere).
    fn cancel_alarm_fallback(&mut self) {
        self.alarm_fallback_generation = self.alarm_fallback_generation.saturating_add(1);
        self.alarm_no_overlay_since = None;
    }

    /// One watchdog poll. Returns whether to keep polling. With a live overlay,
    /// the overlay owns the alarm — reset the no-overlay clock and keep watching
    /// in case it later crashes. With no overlay for `ALARM_FALLBACK_GRACE`,
    /// auto-dismiss and stop polling (bmc's `stop` follows and clears `ringing`).
    fn on_alarm_fallback_tick(&mut self, now: Instant) -> bool {
        if self.compositor.alarm.has_live_overlay() {
            self.alarm_no_overlay_since = None;
            return true;
        }
        let since = *self.alarm_no_overlay_since.get_or_insert(now);
        if now.duration_since(since) < ALARM_FALLBACK_GRACE {
            return true;
        }
        tracing::warn!("alarm ringing with no live overlay; auto-dismissing");
        self.compositor.alarm.request_dismiss();
        self.alarm_no_overlay_since = None;
        false
    }

    fn on_scene_cycling_timer(&mut self, now: Instant) {
        let action = self
            .automatic_cycling
            .on_timer(now, self.active_scene_cycle_duration());
        match action {
            AutomaticCyclingAction::None => {
                if matches!(
                    self.automatic_cycling.phase(),
                    AutomaticCyclingPhase::Transition { .. }
                ) {
                    self.compositor.mark_full_output_damage();
                }
                self.schedule_scene_cycling_timer(now);
            }
            AutomaticCyclingAction::BeginPreTransition => {
                if let Some(target) = self.compositor.widgets.begin_automatic_transition_to_next() {
                    self.automatic_cycling.enter_pre_transition(now);
                    let emission = emit_lifecycle_transitions(self);
                    debug_assert!(
                        emission.releases.is_empty(),
                        "automatic pre-transition must not release dormant buffers before slide"
                    );
                    self.pending_transition_warm_up =
                        emit_transition_incoming_for_target(self, target, now);
                    self.schedule_scene_cycling_timer(now);
                } else {
                    self.reset_automatic_waiting(now);
                }
            }
            AutomaticCyclingAction::BeginTransition => {
                debug_assert!(
                    matches!(
                        self.automatic_cycling.phase(),
                        AutomaticCyclingPhase::PreTransition { .. }
                    ),
                    "BUG: BeginTransition is only produced by the PreTransition phase",
                );
                if !self.take_transition_warm_up(now) {
                    return;
                }
                self.automatic_cycling.enter_transition(now);
                self.compositor.mark_full_output_damage();
                self.schedule_scene_cycling_timer(now);
            }
            AutomaticCyclingAction::FinishTransition => {
                // A zero-length (None) transition finishes straight from
                // PreTransition, so the warm-up gate applies here too; after
                // an animated transition the warm-up is already taken and
                // the gate passes vacuously.
                if !self.take_transition_warm_up(now) {
                    return;
                }
                self.commit_automatic_transition(now);
            }
        }
    }

    /// Take the pending transition warm-up if the incoming widgets have
    /// rendered (or the warm-up timed out); otherwise rearm the cycling
    /// timer for a retry and return `false`.
    fn take_transition_warm_up(&mut self, now: Instant) -> bool {
        if !transition_warm_up_ready(
            self.pending_transition_warm_up.as_ref(),
            now,
            |instance_id| self.compositor.latest_widget_generation(instance_id),
        ) {
            tracing::debug!("scene cycling transition waiting for warm-up render");
            self.rearm_scene_cycling_timer(Some(TRANSITION_WARM_UP_RETRY));
            return false;
        }
        self.pending_transition_warm_up = None;
        true
    }

    /// Commit the in-flight automatic transition: the target scene becomes
    /// current, and the cycler returns to waiting for the next period.
    fn commit_automatic_transition(&mut self, now: Instant) {
        let active_scene_before = (
            self.compositor.widgets.active_scene_id(),
            self.compositor.widgets.active_visible_widget_ids(),
        );
        self.pending_transition_warm_up = None;
        self.compositor.widgets.finish_automatic_transition();
        after_scene_change(self);
        emit_active_scene_changed_if_changed(self, &active_scene_before);
        self.reset_automatic_waiting(now);
    }

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

        // No-overlay fallback: while an alarm rings with no overlay to show its
        // Stop/Snooze buttons, any touch dismisses it immediately. Consume the
        // touch so it does not also drive gestures/scene drags on the widget
        // behind. With a live overlay this is skipped — the overlay owns input.
        if self.compositor.alarm.is_ringing() && !self.compositor.alarm.has_live_overlay() {
            tracing::info!("touch dismissed alarm ringing with no live overlay");
            self.compositor.alarm.request_dismiss();
            return;
        }

        let location = self.touch_location(event);
        let time = event.time_msec();
        let slot = event.slot();

        let sequence_was_idle = self.active_touch_slots.is_empty();
        self.active_touch_slots.insert(slot);
        if sequence_was_idle {
            self.pause_automatic_cycling_for_touch(Instant::now());
        }

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
            tracing::warn!(
                ?slot,
                primary_slot = ?self.gesture_slot,
                active_slots = ?self.active_touch_slots,
                "ignoring non-primary touch slot: single-touch policy"
            );
            return;
        }

        // Check whether the panel is currently showing the user anything
        // before sending a screen activity event to decide whether
        // to swallow a touch.
        //
        // If the backlight state cannot be read, assume that the screen
        // is on, as an unreadable backlight must not silently eat every touch.
        let screen_was_visible = self.screen_visibility.as_ref().is_none_or(|visibility| {
            visibility.is_visible().unwrap_or_else(|err| {
                tracing::warn!(error = %err, "backlight state unreadable; assuming screen is visible");
                true
            })
        });

        let _ = self.event_tx.send(CompositorEvent::ScreenActivity);

        // A touch on a dark panel only wakes the screen.
        //
        // The user cannot see what is under the finger, so it must not
        // reach gesture arbitration or `wl_touch` delivery. Leaving
        // `gesture_slot` unset makes the single-touch-policy checks swallow
        // the sequence's motion/up too.
        if !screen_was_visible {
            tracing::info!("consumed touch on dark screen: wake only");
            return;
        }

        self.gesture_slot = Some(slot);
        self.gesture.on_down(location, time);
        self.scene_drag_active = false;
        self.edge_reveal_active = false;

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

        let activation = self.gesture.on_motion(location, time);

        if !self.edge_reveal_active
            && let MotionActivation::RevealEdge(gesture_border) = activation
        {
            use deck_screen_edge_v1::server::deck_screen_edge_manager_v1::Border;

            let border = match gesture_border {
                GestureBorder::Top => Border::Top,
                GestureBorder::Bottom => Border::Bottom,
            };
            if self.compositor.trigger_screen_edge(border) {
                tracing::info!(
                    slot = ?slot,
                    x = location.x,
                    y = location.y,
                    time_ms = time,
                    ?border,
                    "screen-edge swipe consumed"
                );
                self.gesture.confirm_edge_reveal();
                self.edge_reveal_active = true;
                let touch_handle = self.compositor.touch_handle.clone();
                touch_handle.cancel(&mut self.compositor);
                self.touch_frame_dirty = true;
                return;
            }
            self.gesture.reject_edge_reveal();
            tracing::info!(
                ?border,
                "reveal gesture activated, but no armed edge consumed it"
            );
        }
        if self.edge_reveal_active {
            return;
        }

        let drag_activated = matches!(activation, MotionActivation::SceneDrag);

        if drag_activated
            && !self.scene_drag_active
            && self.compositor.widgets.can_drag()
            && !self.compositor.neighbors_suppressed()
        {
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
                after_drag_scene_update(self);
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
        let touch_sequence_finished = self.active_touch_slots.is_empty();

        // Single-touch policy: only the primary slot produced any
        // wl_touch or gesture state on down/motion, so only the
        // primary needs finalization here. See `on_touch_down`.
        if self.gesture_slot != Some(slot) {
            if touch_sequence_finished {
                self.reset_automatic_waiting(Instant::now());
            }
            return;
        }

        self.gesture_slot = None;
        if !touch_sequence_finished {
            // Legitimate for a multi-finger hold, but also the moment a
            // leaked slot (up/cancel never delivered) becomes observable.
            tracing::info!(
                remaining_slots = ?self.active_touch_slots,
                "primary touch lifted with other slots still active"
            );
        }
        let gesture_result = self.gesture.on_up(time);

        if self.edge_reveal_active {
            self.edge_reveal_active = false;
            if touch_sequence_finished {
                self.reset_automatic_waiting(Instant::now());
            }
            return;
        }

        if self.scene_drag_active {
            self.scene_drag_active = false;
            if let Some(TouchGesture::DragEnd { dx, velocity_x }) = gesture_result {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "drag offsets are panel-sized; fractional pixels round to i32"
                )]
                let dx_px = dx as i32;
                let active_scene_before = (
                    self.compositor.widgets.active_scene_id(),
                    self.compositor.widgets.active_visible_widget_ids(),
                );
                let committed = self.compositor.widgets.end_drag(dx_px, velocity_x);
                after_scene_change(self);
                if committed {
                    emit_active_scene_changed_if_changed(self, &active_scene_before);
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
        if touch_sequence_finished {
            self.reset_automatic_waiting(Instant::now());
        }
    }

    fn on_touch_cancel(&mut self) {
        self.gesture.on_cancel();
        let edge_reveal_active = self.edge_reveal_active;
        self.edge_reveal_active = false;
        self.gesture_slot = None;
        self.active_touch_slots.clear();

        if self.scene_drag_active {
            self.scene_drag_active = false;
            self.compositor.widgets.end_drag(0, 0.0);
            after_scene_change(self);
        } else if edge_reveal_active {
            self.reset_automatic_waiting(Instant::now());
            return;
        } else {
            let touch_handle = self.compositor.touch_handle.clone();
            touch_handle.cancel(&mut self.compositor);
            self.touch_frame_dirty = true;
        }
        self.reset_automatic_waiting(Instant::now());
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

/// Wayland-side of lifecycle emission. Abstracts `send_lifecycle`,
/// `wl_buffer.release`, and per-client `flush` so
/// [`emit_lifecycle_batches`] is testable without an `AppState` —
/// production wires it through [`AppStateLifecycleSink`], tests through
/// an in-memory recorder.
trait LifecycleSink {
    type ClientId: Clone + PartialEq;
    fn send(&mut self, instance_id: &InstanceId, state: LifecycleState) -> Option<Self::ClientId>;
    fn release_buffers(&mut self, instance_id: &InstanceId) -> Vec<Self::ClientId>;
    fn send_transition_incoming(&mut self, instance_id: &InstanceId) -> Option<Self::ClientId>;
    fn flush(&mut self, client_id: &Self::ClientId);
}

struct AppStateLifecycleSink<'a> {
    deck_widget_state: &'a mut DeckWidgetProtocolState,
    display: &'a mut Display<CompositorState>,
    widget_buffers: &'a mut Vec<(WlBuffer, InstanceId)>,
    invalidated_buffers: &'a mut Vec<ObjectId>,
}

impl LifecycleSink for AppStateLifecycleSink<'_> {
    type ClientId = ClientId;
    fn send(&mut self, instance_id: &InstanceId, state: LifecycleState) -> Option<Self::ClientId> {
        self.deck_widget_state.send_lifecycle(instance_id, state)
    }
    fn release_buffers(&mut self, instance_id: &InstanceId) -> Vec<Self::ClientId> {
        release_widget_buffers(self.widget_buffers, self.invalidated_buffers, instance_id)
    }
    fn send_transition_incoming(&mut self, instance_id: &InstanceId) -> Option<Self::ClientId> {
        self.deck_widget_state.send_transition_incoming(instance_id)
    }
    fn flush(&mut self, client_id: &Self::ClientId) {
        if let Err(e) = self.display.backend().flush(Some(client_id.clone())) {
            tracing::warn!("lifecycle: flush for {client_id:?} failed: {e}");
        }
    }
}

/// Re-derive the lifecycle state of every widget from `WidgetTracker`,
/// step the `LifecycleEmitter`, and emit the resulting batches on each
/// widget's surface in the required order: release batch and the released
/// widgets' `wl_buffer.release` first (flush), acquire batch second
/// (flush). The flushes preserve the host's pool ordering invariant on
/// scene swaps and are scoped to only the clients that actually received
/// an event.
fn emit_lifecycle_transitions(state: &mut AppState) -> Emission {
    let mut next = state.compositor.widgets.lifecycle_states();
    if state.compositor.neighbors_suppressed() {
        crate::compositor::layer_surface::suppress_prepared(&mut next);
    }
    let emission = state.compositor.lifecycle.step(&next);

    if emission.is_empty() {
        return emission;
    }

    let mut sink = AppStateLifecycleSink {
        deck_widget_state: &mut state.compositor.deck_widget_state,
        display: &mut state.display,
        widget_buffers: &mut state.compositor.widget_buffers,
        invalidated_buffers: &mut state.compositor.invalidated_buffers,
    };
    emit_lifecycle_batches(&emission, &mut sink);
    emission
}

/// Pure ordering logic, split from [`emit_lifecycle_transitions`] so
/// tests can drive it with an in-memory sink. Sends the release batch
/// and the released widgets' `wl_buffer.release`, flushes, then sends
/// the acquire batch, flushes. The flush boundary between batches is
/// the contract the protocol XML documents: a host must see the buffer
/// release before any acquire lifecycle, so an acquiring slot cannot
/// allocate while a dormant slot's buffer is still held.
fn emit_lifecycle_batches<S: LifecycleSink>(emission: &Emission, sink: &mut S) {
    let mut release_clients = send_lifecycle_batch(sink, &emission.releases, "release");
    for (instance_id, _) in &emission.releases {
        for client_id in sink.release_buffers(instance_id) {
            if !release_clients.contains(&client_id) {
                release_clients.push(client_id);
            }
        }
    }
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

fn transition_incoming_widget_ids(scene: &SceneLayout) -> Vec<InstanceId> {
    scene
        .widgets
        .iter()
        .filter(|widget| widget.visible)
        .map(|widget| widget.instance_id.clone())
        .collect()
}

#[derive(Debug, Clone)]
struct TransitionWarmUp {
    started_at: Instant,
    widgets: HashMap<InstanceId, Option<NonZeroU64>>,
}

/// Whether the slide may start: every incoming widget has committed a
/// fresh frame since `transition_incoming` was sent.
///
/// Readiness requires the generation to *strictly* advance.
/// Valid restoration and decoding can take hundreds of milliseconds;
/// device profiling observed 615 ms even for responsive widgets.
/// `TRANSITION_WARM_UP_TIMEOUT` caps the warm-up.
/// This prevents indefinite stalls when a widget is crashed, respawning, or unresponsive.
/// The timeout may instead slide it in with a stale frame.
fn transition_warm_up_ready(
    warm_up: Option<&TransitionWarmUp>,
    now: Instant,
    latest_generation: impl Fn(&InstanceId) -> Option<NonZeroU64>,
) -> bool {
    let Some(warm_up) = warm_up else {
        return true;
    };
    if now.saturating_duration_since(warm_up.started_at) >= TRANSITION_WARM_UP_TIMEOUT {
        return true;
    }
    warm_up.widgets.iter().all(|(instance_id, before)| {
        latest_generation(instance_id).is_some_and(|current| Some(current) > *before)
    })
}

fn emit_transition_incoming_for_target(
    state: &mut AppState,
    target: SceneTransitionTarget,
    now: Instant,
) -> Option<TransitionWarmUp> {
    let instance_ids = state
        .compositor
        .widgets
        .scene_at(target.to_index)
        .map(transition_incoming_widget_ids)
        .unwrap_or_default();
    if instance_ids.is_empty() {
        return None;
    }

    let widgets = instance_ids
        .iter()
        .map(|instance_id| {
            (
                instance_id.clone(),
                state.compositor.latest_widget_generation(instance_id),
            )
        })
        .collect();

    let mut sink = AppStateLifecycleSink {
        deck_widget_state: &mut state.compositor.deck_widget_state,
        display: &mut state.display,
        widget_buffers: &mut state.compositor.widget_buffers,
        invalidated_buffers: &mut state.compositor.invalidated_buffers,
    };
    emit_transition_incoming_batch(&instance_ids, &mut sink);
    Some(TransitionWarmUp {
        started_at: now,
        widgets,
    })
}

fn emit_transition_incoming_batch<S: LifecycleSink>(instance_ids: &[InstanceId], sink: &mut S) {
    let mut clients: Vec<S::ClientId> = Vec::new();
    for instance_id in instance_ids {
        tracing::debug!("transition_incoming: {instance_id}");
        if let Some(client_id) = sink.send_transition_incoming(instance_id)
            && !clients.contains(&client_id)
        {
            clients.push(client_id);
        }
    }
    for client_id in &clients {
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

/// Common tail of every scene mutation: invalidate the cached output and arm
/// the deferred lifecycle emission.
///
/// The lifecycle transitions are not sent here. Damaging the output queues a
/// compositor render of the committed scene; the transitions (and the dormant
/// buffer release) are flushed by [`emit_pending_lifecycle`] only after that
/// render finishes, so the host starts re-rendering after the compositor's GPU
/// work for the frame is done rather than racing it across the handoff.
fn after_scene_change(state: &mut AppState) {
    state.compositor.mark_full_output_damage();
    state.pending_lifecycle_emission = true;
}

/// Drag-in-progress variant of [`after_scene_change`]: damage the output and
/// emit the lifecycle transitions immediately rather than deferring them.
///
/// A drag moves the active widget to `Leaving` and the drag-direction
/// neighbour to `Entering`; neither is a transition into `Dormant`, so the
/// emission carries no buffer release and the after-render ordering that
/// [`after_scene_change`] exists to enforce is not engaged. Emitting straight
/// away lets a `Visible` widget learn it is `Leaving` before the first drag
/// frame, so its animation loop stops driving renders that would otherwise
/// contend with the drag for the GPU render lock.
///
/// If a deferred emission armed by a prior scene change has not flushed
/// yet, it may carry transitions into `Dormant`; folding those into this
/// inline path would send a buffer release before the after-render
/// ordering. Leave the emission armed instead — the deferred flush reads
/// the tracker at emission time, so it coalesces this drag's transitions.
fn after_drag_scene_update(state: &mut AppState) {
    state.compositor.mark_full_output_damage();
    if state.pending_lifecycle_emission {
        return;
    }
    let emission = emit_lifecycle_transitions(state);
    debug_assert!(
        emission.releases.is_empty(),
        "drag-in-progress lifecycle emission must not release buffers; \
         buffer releases are deferred until after render via after_scene_change",
    );
}

/// Flush the lifecycle transitions armed by [`after_scene_change`], once the
/// compositor has rendered a frame of the committed scene. No-op unless an
/// emission is pending.
fn emit_pending_lifecycle(state: &mut AppState) {
    if !state.pending_lifecycle_emission {
        return;
    }
    state.pending_lifecycle_emission = false;
    emit_lifecycle_transitions(state);
}

fn handle_clear_pid_command(
    state: &mut AppState,
    instance_id: &InstanceId,
    generation: WidgetGeneration,
    expected_pid: u32,
) {
    tracing::debug!(
        "Clearing pid for widget {} (expected_pid={})",
        instance_id,
        expected_pid
    );
    if state
        .compositor
        .clear_pid_for_instance(instance_id, generation, expected_pid)
        .is_some()
    {
        state.compositor.lifecycle.forget(instance_id);
    }
}

fn handle_set_scene_cycling_suspended_command(state: &mut AppState, suspended: bool) {
    tracing::debug!(suspended, "scene cycling suspend gate changed");
    state.automatic_cycling.set_suspended(suspended);
    // Suspending has to undo a slide that is already running. Pausing alone
    // would leave the cycler saying "nothing is running" while the scene is
    // still half slid, with no timer left to finish the move.
    //
    // Resuming must not undo it. The night-mode watch also fires when the value
    // did not change, so a daytime schedule edit re-sends `false` — that must
    // not cut off a slide the user is watching.
    if suspended {
        state.cancel_automatic_transition_for_interruption();
    }
    // Work out the new phase from the flag. Resuming lifts only this gate, so
    // cycling the user switched off in settings stays off, and a slide or wait
    // already in progress keeps running.
    state.reevaluate_automatic_cycling(Instant::now());
}

fn handle_reset_to_first_scene_command(state: &mut AppState) {
    tracing::debug!("resetting to the first scene");
    let active_scene_before = (
        state.compositor.widgets.active_scene_id(),
        state.compositor.widgets.active_visible_widget_ids(),
    );
    state.discard_automatic_transition_for_scene_replacement();
    state.compositor.widgets.reset_to_first_scene();
    after_scene_change(state);
    emit_active_scene_changed_if_changed(state, &active_scene_before);
    state.reset_automatic_waiting(Instant::now());
}

fn handle_set_scene_cycling_config_command(
    state: &mut AppState,
    config: bmc::compositor::SceneCycling,
) {
    tracing::debug!(?config, "updating scene cycling config");
    state
        .automatic_cycling
        .set_config(SceneCyclingRuntimeConfig::from(config));
    state.reevaluate_automatic_cycling(Instant::now());
}

#[expect(clippy::too_many_lines, reason = "command dispatch")]
fn handle_command(state: &mut AppState, cmd: CompositorCommand) {
    match cmd {
        CompositorCommand::RegisterRetainedWidget {
            registration,
            applied,
        } => {
            state
                .compositor
                .deck_widget_state
                .register_retained_widget(registration);
            let _ = applied.send(());
        }
        CompositorCommand::ActivateWidget { key, applied } => {
            state.compositor.deck_widget_state.activate_widget(key);
            let _ = applied.send(());
        }
        CompositorCommand::DeactivateWidget { key, applied } => {
            let instance_id = key.to_string();
            if state.compositor.deactivate_retained_widget(key) {
                reconcile_widget_cutoff(state, &instance_id);
            }
            let _ = applied.send(());
        }
        CompositorCommand::UnregisterRetainedWidget { key, applied } => {
            let instance_id = key.to_string();
            if state.compositor.unregister_retained_widget(key) {
                reconcile_widget_cutoff(state, &instance_id);
            }
            let _ = applied.send(());
        }
        CompositorCommand::RegisterWidget {
            instance_id,
            generation,
            position,
            size,
            initial_config,
            ack,
        } => {
            tracing::debug!(
                "Registering widget {} (generation {generation}) at ({}, {}) size {}x{} initial={:?}",
                instance_id,
                position.x,
                position.y,
                size.width,
                size.height,
                initial_config
            );
            state.compositor.deck_widget_state.register_widget(
                instance_id,
                generation,
                initial_config,
            );
            let _ = ack.send(());
        }
        CompositorCommand::SetWidgetPid {
            instance_id,
            generation,
            pid,
            ack,
        } => {
            tracing::debug!("Associating pid {} with widget {}", pid, instance_id);
            state
                .compositor
                .deck_widget_state
                .set_widget_pid(&instance_id, generation, pid);
            let _ = ack.send(());
        }
        CompositorCommand::BindRespawnedPid {
            instance_id,
            generation,
            pid,
            ack,
        } => {
            tracing::debug!("Binding respawned pid {} to widget {}", pid, instance_id);
            state
                .compositor
                .deck_widget_state
                .bind_respawned_pid(&instance_id, generation, pid);
            let _ = ack.send(());
        }
        CompositorCommand::UnregisterWidget { instance_id } => {
            tracing::debug!("Unregistering widget {}", instance_id);
            state.compositor.unregister_widget(&instance_id);
            state.compositor.lifecycle.forget(&instance_id);
        }
        CompositorCommand::UnregisterAbandoned {
            instance_id,
            generation,
        } => {
            tracing::debug!("Unregistering abandoned widget {}", instance_id);
            if state
                .compositor
                .unregister_abandoned(&instance_id, generation)
            {
                state.compositor.lifecycle.forget(&instance_id);
            }
        }
        CompositorCommand::ClearPid {
            instance_id,
            generation,
            expected_pid,
        } => handle_clear_pid_command(state, &instance_id, generation, expected_pid),
        CompositorCommand::SetActiveScene { layout } => {
            let active_scene_before = (
                state.compositor.widgets.active_scene_id(),
                state.compositor.widgets.active_visible_widget_ids(),
            );
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
            state.discard_automatic_transition_for_scene_replacement();
            state.compositor.widgets.set_active_scene(layout);
            after_scene_change(state);
            emit_active_scene_changed_if_changed(state, &active_scene_before);
            state.reset_automatic_waiting(Instant::now());
        }
        CompositorCommand::SetSceneCycling { scenes } => {
            let active_scene_before = (
                state.compositor.widgets.active_scene_id(),
                state.compositor.widgets.active_visible_widget_ids(),
            );
            tracing::info!("Setting scene cycling with {} scenes", scenes.len());
            state.discard_automatic_transition_for_scene_replacement();
            state.compositor.widgets.set_scene_cycling(scenes);
            after_scene_change(state);
            emit_active_scene_changed_if_changed(state, &active_scene_before);
            state.reset_automatic_waiting(Instant::now());
        }
        CompositorCommand::SetSceneCyclingConfig { config } => {
            handle_set_scene_cycling_config_command(state, config);
        }
        CompositorCommand::SetSceneCyclingSuspended { suspended } => {
            handle_set_scene_cycling_suspended_command(state, suspended);
        }
        CompositorCommand::ResetToFirstScene => handle_reset_to_first_scene_command(state),
        CompositorCommand::BroadcastSetting { setting } => {
            tracing::debug!("Broadcasting setting: {:?}", setting);
            state
                .compositor
                .deck_widget_state
                .broadcast_setting(&setting);
        }
        CompositorCommand::SetBrightness { value } => {
            state.compositor.settings.set_brightness(value);
        }
        CompositorCommand::SetWifiAp { ssid } => {
            state.compositor.settings.set_wifi_ap(ssid);
        }
        CompositorCommand::SetVolume { value } => {
            state.compositor.settings.set_volume(value);
        }
        CompositorCommand::SetNightMode { active, until } => {
            state.compositor.settings.set_night_mode(active, until);
        }
        CompositorCommand::SetUpgradeState { state: upgrade } => {
            state.compositor.upgrade.set(upgrade, Instant::now());
        }
        CompositorCommand::RestartDeclined { reason } => {
            state.compositor.settings.restart_declined(&reason);
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
        CompositorCommand::UpdateWidgetCredentials {
            instance_id,
            credentials,
            secrets,
            ack,
        } => {
            tracing::debug!("Updating widget credentials: {instance_id}");
            let changed = state
                .compositor
                .deck_widget_state
                .update_widget_credentials(&instance_id, credentials, secrets);
            let _ = ack.send(changed);
        }
        CompositorCommand::Shutdown => {
            tracing::info!("Shutdown command received");
            state.compositor.deck_widget_state.broadcast_shutdown();
            state.should_exit = true;
        }
        CompositorCommand::RingAlarm {
            time,
            period,
            label,
            snooze_allowed,
        } => {
            state
                .compositor
                .alarm
                .ring(&time, &period, &label, snooze_allowed);
            state.arm_alarm_fallback();
        }
        CompositorCommand::StopAlarm => {
            state.compositor.alarm.stop();
            state.cancel_alarm_fallback();
        }
    }
}

fn reconcile_widget_cutoff(state: &mut AppState, instance_id: &InstanceId) {
    if state.connected_widgets.remove(instance_id) {
        let _ = state
            .connected_widgets_tx
            .send(state.connected_widgets.clone());
    }
}

fn emit_active_scene_changed_if_changed(
    state: &AppState,
    active_scene_before: &(Option<bmc::scene::SceneId>, Vec<InstanceId>),
) {
    let active_scene_after = (
        state.compositor.widgets.active_scene_id(),
        state.compositor.widgets.active_visible_widget_ids(),
    );
    if active_scene_before == &active_scene_after {
        return;
    }
    // `None` scene_id publishes `None` so a cleared active scene reaches
    // consumers too, not just transitions between scenes.
    let (scene_id, widget_ids) = active_scene_after;
    let value = scene_id.map(|scene_id| ActiveScene {
        scene_id,
        widget_ids,
    });
    let _ = state.active_scene_tx.send(value);
}

#[expect(
    clippy::too_many_lines,
    reason = "linear drain-and-forward dispatcher; one independent block per \
              compositor subsystem (widgets, actions, settings, alarm, status)"
)]
fn process_protocol_events(state: &mut AppState) {
    let mut connected_set_changed = false;
    let connected = state.compositor.deck_widget_state.drain_connected();
    let disconnected = state.compositor.deck_widget_state.drain_disconnected();
    for instance_id in &disconnected {
        tracing::info!("Widget disconnected: {}", instance_id);
    }
    if !connected.is_empty() {
        let mut lifecycle_states = state.compositor.widgets.lifecycle_states();
        if state.compositor.neighbors_suppressed() {
            crate::compositor::layer_surface::suppress_prepared(&mut lifecycle_states);
        }
        let mut connect_clients: Vec<ClientId> = Vec::new();

        for instance_id in &connected {
            let lifecycle_state = clamp_initial_lifecycle(
                lifecycle_states
                    .get(instance_id)
                    .copied()
                    .unwrap_or(LifecycleState::Dormant),
            );
            match state
                .compositor
                .send_initial_lifecycle(instance_id, lifecycle_state)
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
            let _ = state.event_tx.send(CompositorEvent::WidgetReady {
                instance_id: instance_id.clone(),
            });
        }
        let mut sink = AppStateLifecycleSink {
            deck_widget_state: &mut state.compositor.deck_widget_state,
            display: &mut state.display,
            widget_buffers: &mut state.compositor.widget_buffers,
            invalidated_buffers: &mut state.compositor.invalidated_buffers,
        };
        flush_lifecycle_clients(&mut sink, &connect_clients);
    }

    let affected = connected
        .into_iter()
        .chain(disconnected)
        .collect::<BTreeSet<_>>();
    for instance_id in affected {
        let changed = if state
            .compositor
            .deck_widget_state
            .has_attachment(&instance_id)
        {
            state.connected_widgets.insert(instance_id)
        } else {
            state.connected_widgets.remove(&instance_id)
        };
        if changed {
            connected_set_changed = true;
        }
    }

    if connected_set_changed {
        let _ = state
            .connected_widgets_tx
            .send(state.connected_widgets.clone());
    }

    for (instance_id, payload) in state.compositor.deck_widget_state.drain_actions() {
        tracing::debug!("Widget {} action: {:?}", instance_id, payload);
        let _ = state.action_tx.send(WidgetAction {
            instance_id,
            payload,
        });
    }

    for action in state.compositor.settings.drain_actions() {
        use crate::compositor::settings::SettingsAction;
        let cmd = match action {
            SettingsAction::SetBrightness(value) => SettingsCommand::SetBrightness(value),
            SettingsAction::SetVolume(value) => SettingsCommand::SetVolume(value),
            SettingsAction::ToggleNightMode => SettingsCommand::ToggleNightMode,
            SettingsAction::Restart => SettingsCommand::Restart,
            SettingsAction::ReconfigureWifi => SettingsCommand::ReconfigureWifi,
        };
        if let Err(e) = state.settings_tx.send(cmd) {
            tracing::error!(
                ?e,
                "dropped settings command: action handler receiver closed"
            );
        }
    }

    for action in state.compositor.alarm.drain_actions() {
        use crate::compositor::alarm::AlarmAction;
        let cmd = match action {
            AlarmAction::Dismiss => AlarmCommand::Dismiss,
            AlarmAction::Snooze => AlarmCommand::Snooze,
        };
        if let Err(e) = state.alarm_tx.send(cmd) {
            tracing::error!(?e, "dropped alarm command: action handler receiver closed");
        }
    }

    // Status acks come from the tokio side, so they have no calloop wake
    // source of their own — they are drained here every loop iteration. The
    // loop iterates at least every `FRAME_CALLBACK_TICK` (the tick is
    // unconditional), so delivery is bounded to one tick; the channel is
    // unbounded, so nothing is dropped while queued. If the tick ever becomes
    // conditional, give this channel its own wake source instead.
    while let Ok(status) = state.status_rx.try_recv() {
        tracing::debug!(
            "Widget {} led_request_status: req={} status={:?}",
            status.instance_id,
            status.request_id,
            status.status
        );
        state.compositor.deck_widget_state.emit_led_request_status(
            &status.instance_id,
            status.request_id,
            status.status,
        );
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
        let screen_visibility = self.screen_visibility.clone();
        let command_channel = self
            .command_channel
            .lock()
            .expect("BUG: command_channel lock poisoned")
            .take()
            .expect("BUG: command_channel already taken");
        let action_tx = self.action_tx.clone();
        let event_tx = self.event_tx.clone();
        let settings_tx = self.settings_tx.clone();
        let alarm_tx = self.alarm_tx.clone();
        let status_rx = self
            .status_rx
            .lock()
            .expect("BUG: status_rx lock poisoned")
            .take()
            .expect("BUG: status_rx already taken");
        let active_scene_tx = self.active_scene_tx.clone();
        let connected_widgets_tx = self.connected_widgets_tx.clone();

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
                    screen_visibility,
                    command_channel,
                    action_tx,
                    event_tx,
                    settings_tx,
                    alarm_tx,
                    status_rx,
                    active_scene_tx,
                    connected_widgets_tx,
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

    fn set_upgrade_state(
        &self,
        state: bmc::compositor::UpgradeDisplaySnapshot,
    ) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetUpgradeState { state })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn enqueue_register_widget(
        &self,
        registration: WidgetRegistration,
    ) -> Result<bmc::compositor::CompositorReceipt, CompositorError> {
        let (applied, receipt) = bmc::compositor::CompositorReceipt::pending("register widget");
        self.command_tx
            .send(CompositorCommand::RegisterRetainedWidget {
                registration,
                applied,
            })
            .map_err(|error| CompositorError::SendError(error.to_string()))?;
        Ok(receipt)
    }

    fn enqueue_activate_widget(
        &self,
        key: WidgetInstanceKey,
    ) -> Result<bmc::compositor::CompositorReceipt, CompositorError> {
        let (applied, receipt) = bmc::compositor::CompositorReceipt::pending("activate widget");
        self.command_tx
            .send(CompositorCommand::ActivateWidget { key, applied })
            .map_err(|error| CompositorError::SendError(error.to_string()))?;
        Ok(receipt)
    }

    fn enqueue_deactivate_widget(
        &self,
        key: WidgetInstanceKey,
    ) -> Result<bmc::compositor::CompositorReceipt, CompositorError> {
        let (applied, receipt) = bmc::compositor::CompositorReceipt::pending("deactivate widget");
        self.command_tx
            .send(CompositorCommand::DeactivateWidget { key, applied })
            .map_err(|error| CompositorError::SendError(error.to_string()))?;
        Ok(receipt)
    }

    fn enqueue_unregister_widget(
        &self,
        key: WidgetInstanceKey,
    ) -> Result<bmc::compositor::CompositorReceipt, CompositorError> {
        let (applied, receipt) = bmc::compositor::CompositorReceipt::pending("unregister widget");
        self.command_tx
            .send(CompositorCommand::UnregisterRetainedWidget { key, applied })
            .map_err(|error| CompositorError::SendError(error.to_string()))?;
        Ok(receipt)
    }

    fn register_widget(
        &self,
        instance_id: InstanceId,
        generation: WidgetGeneration,
        position: Position,
        size: Size,
        initial_config: WidgetInitialConfig,
    ) -> Result<(), CompositorError> {
        let (ack_tx, ack_rx) = flume::bounded(1);
        self.command_tx
            .send(CompositorCommand::RegisterWidget {
                instance_id,
                generation,
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

    fn set_widget_pid(
        &self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
        pid: u32,
    ) -> Result<(), CompositorError> {
        let (ack_tx, ack_rx) = flume::bounded(1);
        self.command_tx
            .send(CompositorCommand::SetWidgetPid {
                instance_id: instance_id.clone(),
                generation,
                pid,
                ack: ack_tx,
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))?;
        ack_rx
            .recv_timeout(WIDGET_COMMAND_ACK_TIMEOUT)
            .map_err(|e| CompositorError::ThreadError(format!("set_widget_pid ack: {e}")))
    }

    fn bind_respawned_pid(
        &self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
        pid: u32,
    ) -> Result<(), CompositorError> {
        let (ack_tx, ack_rx) = flume::bounded(1);
        self.command_tx
            .send(CompositorCommand::BindRespawnedPid {
                instance_id: instance_id.clone(),
                generation,
                pid,
                ack: ack_tx,
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))?;
        ack_rx
            .recv_timeout(WIDGET_COMMAND_ACK_TIMEOUT)
            .map_err(|e| CompositorError::ThreadError(format!("bind_respawned_pid ack: {e}")))
    }

    fn unregister_widget(&self, instance_id: &InstanceId) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::UnregisterWidget {
                instance_id: instance_id.clone(),
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn unregister_abandoned(
        &self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
    ) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::UnregisterAbandoned {
                instance_id: instance_id.clone(),
                generation,
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn clear_pid(
        &self,
        instance_id: &InstanceId,
        generation: WidgetGeneration,
        pid: u32,
    ) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::ClearPid {
                instance_id: instance_id.clone(),
                generation,
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

    fn set_scene_cycling_config(
        &self,
        config: bmc::compositor::SceneCycling,
    ) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetSceneCyclingConfig { config })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn set_scene_cycling_suspended(&self, suspended: bool) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetSceneCyclingSuspended { suspended })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn reset_to_first_scene(&self) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::ResetToFirstScene)
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_setting(&self, setting: SettingUpdate) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::BroadcastSetting { setting })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_brightness(&self, value: u8) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetBrightness { value })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_volume(&self, value: u8) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetVolume { value })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_night_mode(
        &self,
        active: bool,
        until: Option<&str>,
    ) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetNightMode {
                active,
                until: until.map(str::to_owned),
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_restart_declined(&self, reason: &str) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::RestartDeclined {
                reason: reason.to_owned(),
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_wifi_ap(&self, ssid: Option<String>) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::SetWifiAp { ssid })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_alarm_ring(
        &self,
        time: String,
        period: String,
        label: String,
        snooze_allowed: bool,
    ) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::RingAlarm {
                time,
                period,
                label,
                snooze_allowed,
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))
    }

    fn broadcast_alarm_stop(&self) -> Result<(), CompositorError> {
        self.command_tx
            .send(CompositorCommand::StopAlarm)
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

    fn update_widget_credentials(
        &self,
        instance_id: &InstanceId,
        credentials: serde_json::Map<String, serde_json::Value>,
        secrets: bmc_widget_protocol::CredentialSecrets,
    ) -> Result<bool, CompositorError> {
        let (ack_tx, ack_rx) = flume::bounded(1);
        self.command_tx
            .send(CompositorCommand::UpdateWidgetCredentials {
                instance_id: instance_id.clone(),
                credentials,
                secrets,
                ack: ack_tx,
            })
            .map_err(|e| CompositorError::SendError(e.to_string()))?;
        ack_rx
            .recv_timeout(WIDGET_COMMAND_ACK_TIMEOUT)
            .map_err(|e| {
                CompositorError::ThreadError(format!("update_widget_credentials ack: {e}"))
            })
    }

    fn action_receiver(&self) -> mpsc::UnboundedReceiver<WidgetAction> {
        self.action_rx
            .lock()
            .expect("BUG: action_rx lock poisoned")
            .take()
            .expect("BUG: action_receiver already taken")
    }

    fn settings_receiver(&self) -> mpsc::UnboundedReceiver<SettingsCommand> {
        self.settings_rx
            .lock()
            .expect("BUG: settings_rx lock poisoned")
            .take()
            .expect("BUG: settings_receiver already taken")
    }

    fn alarm_receiver(&self) -> mpsc::UnboundedReceiver<AlarmCommand> {
        self.alarm_rx
            .lock()
            .expect("BUG: alarm_rx lock poisoned")
            .take()
            .expect("BUG: alarm_receiver already taken")
    }

    fn request_status_sender(&self) -> mpsc::UnboundedSender<LedRequestStatusEvent> {
        self.status_tx.clone()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<CompositorEvent> {
        self.event_tx.subscribe()
    }

    fn active_scene_watch(&self) -> watch::Receiver<Option<ActiveScene>> {
        self.active_scene_tx.subscribe()
    }

    fn connected_widgets_watch(&self) -> watch::Receiver<BTreeSet<InstanceId>> {
        self.connected_widgets_tx.subscribe()
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
        ALARM_FALLBACK_GRACE, AppState, CompositorState, EglCompositor, Emission, GestureConfig,
        GestureState, LibinputInputBackend, LifecycleSink, LifecycleState, RedrawState, TouchSlot,
        TransitionWarmUp, clamp_initial_lifecycle, dispatch_timeout, emit_lifecycle_batches,
        emit_lifecycle_transitions, emit_transition_incoming_batch, handle_clear_pid_command,
        handle_command, process_protocol_events, transition_incoming_widget_ids,
        transition_warm_up_ready,
    };
    use crate::compositor::scene_cycling::{
        AUTOMATIC_TRANSITION_DURATION, AutomaticCycling, AutomaticCyclingPhase,
        PRE_TRANSITION_DURATION, SceneCyclingRuntimeConfig,
    };
    use crate::compositor::{
        CompositorCommand, protocol::WidgetSurfaceUserData, state::ClientState,
    };
    use bmc::compositor::{
        Compositor, CompositorError, CompositorEvent, InstanceId, Position, SceneCycling,
        SceneCyclingTransition, SceneLayout, Size, UpgradeDisplaySnapshot, UpgradeDisplayState,
        UpgradeGeneration, UpgradeKind, WidgetConnectionMode, WidgetGeneration, WidgetInstanceKey,
        WidgetPlacement, WidgetRegistration,
    };

    const GEN: WidgetGeneration = WidgetGeneration(1);
    use bmc_platform::backlight::ScreenVisibility;
    use bmc_widget_protocol::{
        ActionPayload, ViewportShape, WidgetInitialConfig,
        server::deck_widget_surface_v1::DeckWidgetSurfaceV1,
    };
    use smithay::reexports::{
        calloop::EventLoop,
        input as libinput,
        wayland_server::{Display, ListeningSocket, Resource},
    };
    use std::{
        collections::{HashMap, HashSet},
        os::unix::net::UnixStream,
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::mpsc;

    /// Panel visibility a test can flip *without* dispatching a command, which
    /// is the whole point: the touch guard must track the hardware, not the
    /// compositor's command queue.
    #[derive(Debug)]
    struct FakeScreenVisibility {
        visible: AtomicBool,
        readable: AtomicBool,
    }

    impl FakeScreenVisibility {
        fn new(visible: bool) -> Arc<Self> {
            Arc::new(Self {
                visible: AtomicBool::new(visible),
                readable: AtomicBool::new(true),
            })
        }

        fn set_visible(&self, visible: bool) {
            self.visible.store(visible, Ordering::Relaxed);
        }

        fn fail_reads(&self) {
            self.readable.store(false, Ordering::Relaxed);
        }
    }

    impl ScreenVisibility for FakeScreenVisibility {
        fn is_visible(&self) -> anyhow::Result<bool> {
            if self.readable.load(Ordering::Relaxed) {
                Ok(self.visible.load(Ordering::Relaxed))
            } else {
                anyhow::bail!("simulated backlight read failure")
            }
        }
    }

    fn make_app_state_with_screen(screen: &Arc<FakeScreenVisibility>) -> AppState {
        let mut state = make_app_state();
        state.screen_visibility = Some(Arc::clone(screen) as Arc<dyn ScreenVisibility>);
        state
    }

    fn make_widget_config() -> WidgetInitialConfig {
        WidgetInitialConfig {
            width: 100,
            height: 100,
            viewport_shape: ViewportShape::Rectangular,
            display: bmc_widget_protocol::DisplayInfo::BMC100,
            params: serde_json::Map::new(),
            credentials: serde_json::Map::new(),
            credential_secrets: bmc_widget_protocol::CredentialSecrets::default(),
            token: "test-instance-2x1".to_owned(),
        }
    }

    fn make_test_socket_path() -> PathBuf {
        static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(0);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("BUG: system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join("bmc-openwrt-tests");
        std::fs::create_dir_all(&dir).expect("BUG: test socket directory should be creatable");
        let socket_id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        dir.join(format!(
            "clear-pid-{timestamp}-{}-{socket_id}",
            std::process::id()
        ))
    }

    fn make_app_state() -> AppState {
        let event_loop: EventLoop<'static, AppState> =
            EventLoop::try_new().expect("BUG: test event loop should initialize");
        let display: Display<CompositorState> =
            Display::new().expect("BUG: test Wayland display should initialize");
        let compositor = CompositorState::new(
            &display,
            480,
            1280,
            480,
            1280,
            60_000,
            "test-seat",
            crate::compositor::settings::caps_for_product(bmc_platform::Product::Bmc100),
        );
        let listening_socket = ListeningSocket::bind_absolute(make_test_socket_path())
            .expect("BUG: test Wayland socket should bind");
        let (action_tx, _) = mpsc::unbounded_channel();
        let (event_tx, _) = tokio::sync::broadcast::channel(64);
        let (settings_tx, _) = mpsc::unbounded_channel();
        let (alarm_tx, _) = mpsc::unbounded_channel();
        let (_status_tx, status_rx) = mpsc::unbounded_channel();
        let (active_scene_tx, _) = tokio::sync::watch::channel(None);
        let (connected_widgets_tx, _) =
            tokio::sync::watch::channel(std::collections::BTreeSet::new());

        AppState {
            display,
            compositor,
            scene_renderer: None,
            listening_socket,
            action_tx,
            event_tx,
            settings_tx,
            alarm_tx,
            status_rx,
            active_scene_tx,
            connected_widgets_tx,
            connected_widgets: std::collections::BTreeSet::new(),
            gesture: GestureState::with_config(GestureConfig {
                screen_height: Some(1280.0),
                ..GestureConfig::default()
            }),
            gesture_slot: None,
            active_touch_slots: HashSet::new(),
            screen_visibility: None,
            scene_drag_active: false,
            edge_reveal_active: false,
            touch_frame_dirty: false,
            logical_width: 480,
            logical_height: 1280,
            touch_transform: bmc_platform::TouchTransform::Deg0,
            should_exit: false,
            redraw_state: RedrawState::Idle,
            pending_lifecycle_emission: false,
            loop_handle: event_loop.handle(),
            retry_libinput: None,
            touch_retry_pending: false,
            automatic_cycling: AutomaticCycling::new(
                Instant::now(),
                SceneCyclingRuntimeConfig::default(),
            ),
            pending_transition_warm_up: None,
            scene_cycling_timer_generation: 0,
            alarm_fallback_generation: 0,
            alarm_no_overlay_since: None,
            last_neighbors_suppressed: false,
            last_modal_overlay_active: false,
        }
    }

    fn retained_registration() -> WidgetRegistration {
        let key = WidgetInstanceKey::from(bmc::scene::WidgetId::generate());
        WidgetRegistration {
            key,
            connection_mode: WidgetConnectionMode::Accepting,
            placement: WidgetPlacement {
                instance_id: key.to_string(),
                position: Position { x: 0, y: 0 },
                size: Size {
                    width: 100,
                    height: 100,
                },
                visible: true,
            },
            initial_config: make_widget_config(),
        }
    }

    #[test]
    fn lifecycle_receipt_follows_queued_event_and_connected_watch_cutoff() {
        for unregister in [false, true] {
            let mut state = make_app_state();
            let registration = retained_registration();
            let key = registration.key;
            let instance_id = key.to_string();
            state
                .compositor
                .deck_widget_state
                .register_retained_widget(registration);
            let mut handle = state.display.handle();
            let (socket, _peer) =
                UnixStream::pair().expect("BUG: test Wayland socket pair should initialize");
            let client = handle
                .insert_client(socket, Arc::new(ClientState::default()))
                .expect("BUG: test Wayland client should register");
            let protocol_surface = client
                .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(
                    &handle,
                    2,
                    WidgetSurfaceUserData {
                        instance_id: Arc::new(Mutex::new(instance_id.clone())),
                    },
                )
                .expect("BUG: test protocol surface should initialize");
            state
                .compositor
                .deck_widget_state
                .attach_protocol_surface_for_test(&instance_id, protocol_surface.clone());
            state
                .compositor
                .lifecycle
                .record_initial(&instance_id, LifecycleState::Visible);
            state
                .compositor
                .deck_widget_state
                .queue_connected_for_test(instance_id.clone());
            state
                .compositor
                .deck_widget_state
                .add_action(instance_id.clone(), ActionPayload::StopSound {});
            state.connected_widgets.insert(instance_id.clone());
            let _ = state
                .connected_widgets_tx
                .send(state.connected_widgets.clone());
            let connected_rx = state.connected_widgets_tx.subscribe();
            let (action_tx, mut action_rx) = mpsc::unbounded_channel();
            state.action_tx = action_tx;
            let mut event_rx = state.event_tx.subscribe();
            let (applied, mut receipt) = tokio::sync::oneshot::channel();

            let command = if unregister {
                CompositorCommand::UnregisterRetainedWidget { key, applied }
            } else {
                CompositorCommand::DeactivateWidget { key, applied }
            };
            handle_command(&mut state, command);

            assert_eq!(receipt.try_recv(), Ok(()));
            assert!(!protocol_surface.is_alive());
            assert_eq!(state.compositor.lifecycle.last_state(&instance_id), None);
            assert!(!connected_rx.borrow().contains(&instance_id));
            process_protocol_events(&mut state);
            assert!(action_rx.try_recv().is_err());
            assert!(event_rx.try_recv().is_err());
        }
    }

    #[test]
    fn replacement_pass_keeps_connected_watch_and_emits_successor_lifecycle() {
        let mut state = make_app_state();
        let registration = retained_registration();
        let key = registration.key;
        let instance_id = key.to_string();
        state
            .compositor
            .deck_widget_state
            .register_retained_widget(registration.clone());
        let mut handle = state.display.handle();
        let make_surface = |handle: &mut smithay::reexports::wayland_server::DisplayHandle| {
            let (socket, peer) =
                UnixStream::pair().expect("BUG: test Wayland socket pair should initialize");
            let client = handle
                .insert_client(socket, Arc::new(ClientState::default()))
                .expect("BUG: test Wayland client should register");
            let surface = client
                .create_resource::<DeckWidgetSurfaceV1, _, CompositorState>(
                    handle,
                    2,
                    WidgetSurfaceUserData {
                        instance_id: Arc::new(Mutex::new(instance_id.clone())),
                    },
                )
                .expect("BUG: test protocol surface should initialize");
            (surface, peer)
        };
        let (predecessor, _predecessor_peer) = make_surface(&mut handle);
        state
            .compositor
            .deck_widget_state
            .attach_protocol_surface_for_test(&instance_id, predecessor);
        let connected_rx = state.connected_widgets_tx.subscribe();
        state
            .compositor
            .deck_widget_state
            .queue_connected_for_test(instance_id.clone());
        process_protocol_events(&mut state);

        state.compositor.deck_widget_state.deactivate_widget(key);
        state
            .compositor
            .deck_widget_state
            .register_retained_widget(registration);
        let (successor, _successor_peer) = make_surface(&mut handle);
        state
            .compositor
            .deck_widget_state
            .attach_protocol_surface_for_test(&instance_id, successor);
        state
            .compositor
            .deck_widget_state
            .queue_connected_for_test(instance_id.clone());
        process_protocol_events(&mut state);

        assert!(connected_rx.borrow().contains(&instance_id));
        assert_eq!(
            state.compositor.lifecycle.last_state(&instance_id),
            Some(LifecycleState::Dormant),
            "replacement must receive its initial lifecycle in the same pass"
        );
    }

    #[test]
    fn closed_command_channel_reports_enqueue_failure_before_a_receipt_exists() {
        let compositor = EglCompositor::new(
            bmc_platform::HardwareProfile::for_product(bmc_platform::Product::Bmc100),
            true,
        );
        drop(
            compositor
                .command_channel
                .lock()
                .expect("BUG: command channel lock poisoned")
                .take(),
        );

        assert!(matches!(
            compositor
                .enqueue_activate_widget(WidgetInstanceKey::from(bmc::scene::WidgetId::generate())),
            Err(CompositorError::SendError(_))
        ));
    }

    fn gen_nz(n: u64) -> std::num::NonZeroU64 {
        std::num::NonZeroU64::new(n).expect("BUG: test generation must be non-zero")
    }

    fn lifecycle_map(
        pairs: &[(InstanceId, LifecycleState)],
    ) -> HashMap<InstanceId, LifecycleState> {
        pairs.iter().cloned().collect()
    }

    fn test_scene(instance_id: &str) -> SceneLayout {
        SceneLayout {
            scene_id: None,
            cycle_duration: None,
            combined: false,
            widgets: vec![WidgetPlacement {
                instance_id: instance_id.to_owned(),
                position: Position { x: 0, y: 0 },
                size: Size {
                    width: 480,
                    height: 1280,
                },
                visible: true,
            }],
        }
    }

    fn test_scene_with_cycle_duration(instance_id: &str, duration: Duration) -> SceneLayout {
        SceneLayout {
            cycle_duration: Some(duration),
            ..test_scene(instance_id)
        }
    }

    fn cycling_config(enabled: bool) -> SceneCyclingRuntimeConfig {
        SceneCyclingRuntimeConfig {
            enabled,
            default_duration: Duration::from_secs(30),
            transition: SceneCyclingTransition::Slide,
        }
    }

    struct FakeTouchEvent {
        slot: TouchSlot,
        time: u64,
        x: f64,
        y: f64,
    }

    impl FakeTouchEvent {
        fn new(slot: u32, time_msec: u32) -> Self {
            Self {
                slot: TouchSlot::from(Some(slot)),
                time: u64::from(time_msec) * 1000,
                x: 1.0,
                y: 1.0,
            }
        }

        fn at_x(slot: u32, time_msec: u32, x: f64) -> Self {
            Self {
                x,
                ..Self::new(slot, time_msec)
            }
        }
    }

    impl smithay::backend::input::Event<LibinputInputBackend> for FakeTouchEvent {
        fn time(&self) -> u64 {
            self.time
        }

        fn device(&self) -> libinput::Device {
            panic!("BUG: touch pause tests must not inspect the libinput device")
        }
    }

    impl smithay::backend::input::TouchEvent<LibinputInputBackend> for FakeTouchEvent {
        fn slot(&self) -> TouchSlot {
            self.slot
        }
    }

    impl smithay::backend::input::AbsolutePositionEvent<LibinputInputBackend> for FakeTouchEvent {
        fn x(&self) -> f64 {
            self.x
        }

        fn y(&self) -> f64 {
            self.y
        }

        fn x_transformed(&self, _width: i32) -> f64 {
            self.x
        }

        fn y_transformed(&self, _height: i32) -> f64 {
            self.y
        }
    }

    #[test]
    fn alarm_fallback_auto_dismisses_when_no_overlay() {
        let now = Instant::now();
        let mut state = make_app_state();
        // No overlay resources are bound in the test harness.
        state.compositor.alarm.ring("07:30", "", "", false);

        // First poll only records the no-overlay start; still within grace.
        assert!(state.on_alarm_fallback_tick(now));
        assert!(state.compositor.alarm.is_ringing());

        // Once grace has elapsed, the alarm auto-dismisses and polling stops.
        assert!(!state.on_alarm_fallback_tick(now + ALARM_FALLBACK_GRACE));
        assert!(!state.compositor.alarm.is_ringing());
        assert_eq!(
            state.compositor.alarm.drain_actions(),
            vec![crate::compositor::alarm::AlarmAction::Dismiss]
        );
    }

    #[test]
    fn touch_dismisses_alarm_when_no_overlay() {
        let mut state = make_app_state();
        state.compositor.alarm.ring("07:30", "", "", false);

        state.on_touch_down(&FakeTouchEvent::new(0, 1));

        assert!(!state.compositor.alarm.is_ringing());
        assert_eq!(
            state.compositor.alarm.drain_actions(),
            vec![crate::compositor::alarm::AlarmAction::Dismiss]
        );
        // The touch was consumed, not routed into gesture arbitration.
        assert!(state.active_touch_slots.is_empty());
    }

    #[test]
    fn touch_down_pauses_automatic_cycling() {
        let now = Instant::now();
        let mut state = make_app_state();
        state.automatic_cycling.set_config(cycling_config(true));
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state.reset_automatic_waiting(now);

        state.on_touch_down(&FakeTouchEvent::new(0, 1));

        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
    }

    #[test]
    fn touch_release_resets_waiting_deadline() {
        let mut state = make_app_state();
        state.automatic_cycling.set_config(cycling_config(true));
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);

        state.on_touch_down(&FakeTouchEvent::new(0, 1));
        state.on_touch_up(&FakeTouchEvent::new(0, 2));

        let AutomaticCyclingPhase::WaitingForTimer { started_at } = state.automatic_cycling.phase()
        else {
            panic!("BUG: final touch release should reset automatic cycling to waiting");
        };
        assert_eq!(
            state
                .automatic_cycling
                .next_delay(started_at, Duration::from_secs(30)),
            Some(Duration::from_secs(30)),
        );
    }

    #[test]
    fn touch_release_keeps_cycling_paused_until_secondary_slot_releases() {
        let now = Instant::now();
        let mut state = make_app_state();
        state.automatic_cycling.set_config(cycling_config(true));
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state.reset_automatic_waiting(now);

        state.on_touch_down(&FakeTouchEvent::new(0, 1));
        state.on_touch_down(&FakeTouchEvent::new(1, 2));
        state.on_touch_up(&FakeTouchEvent::new(0, 3));

        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));

        state.on_touch_up(&FakeTouchEvent::new(1, 4));

        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::WaitingForTimer { .. }
        ));
    }

    #[test]
    fn touch_cancel_clears_slots_and_resets_waiting_deadline() {
        let now = Instant::now();
        let mut state = make_app_state();
        state.automatic_cycling.set_config(cycling_config(true));
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state.active_touch_slots.insert(TouchSlot::from(Some(1)));
        state.automatic_cycling.reset_waiting(now, 2, true);

        state.on_touch_cancel();

        assert!(state.active_touch_slots.is_empty());
        let AutomaticCyclingPhase::WaitingForTimer { started_at } = state.automatic_cycling.phase()
        else {
            panic!("BUG: touch cancel should reset automatic cycling to waiting");
        };
        assert_eq!(
            state
                .automatic_cycling
                .next_delay(started_at, Duration::from_secs(30)),
            Some(Duration::from_secs(30)),
        );
    }

    #[test]
    fn edge_reveal_touch_cancel_does_not_forward_duplicate_cancel() {
        let mut state = make_app_state();
        state.edge_reveal_active = true;
        state.touch_frame_dirty = false;
        state.active_touch_slots.insert(TouchSlot::from(Some(1)));

        state.on_touch_cancel();

        assert!(
            !state.touch_frame_dirty,
            "edge-owned cancel must not emit a second wl_touch.cancel"
        );
        assert!(!state.edge_reveal_active);
        assert!(state.active_touch_slots.is_empty());
    }

    #[test]
    fn touch_pause_cancels_tracker_transition() {
        let now = Instant::now();
        let mut state = make_app_state();
        state.automatic_cycling.set_config(cycling_config(true));
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next();

        state.pause_automatic_cycling_for_touch(now);

        assert!(!state.compositor.widgets.automatic_transition_active());
        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
    }

    #[test]
    fn automatic_transition_timer_tick_marks_output_damage() {
        let mut state = make_app_state();
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state.compositor.clear_output_damage();
        let now = Instant::now();
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_transition(now);

        state.on_scene_cycling_timer(now + Duration::from_millis(16));

        assert!(state.compositor.needs_redraw());
    }

    #[test]
    fn pre_transition_keeps_widget_animation_running_until_motion_starts() {
        let mut state = make_app_state();
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        let now = Instant::now();
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(now);

        assert!(!state.widget_frame_callbacks_suppressed());

        state.automatic_cycling.enter_transition(now);

        assert!(state.widget_frame_callbacks_suppressed());
    }

    #[test]
    fn manual_drag_suppresses_widget_animation() {
        let mut state = make_app_state();
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);

        state.compositor.widgets.start_drag();

        assert!(state.widget_frame_callbacks_suppressed());

        state.compositor.widgets.end_drag(0, 0.0);

        assert!(!state.widget_frame_callbacks_suppressed());
    }

    #[test]
    fn automatic_transition_finish_defers_lifecycle_until_rendered_frame() {
        let mut state = make_app_state();
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        let now = Instant::now();
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_transition(now);
        state.compositor.clear_output_damage();

        state.on_scene_cycling_timer(now + AUTOMATIC_TRANSITION_DURATION);

        assert!(state.pending_lifecycle_emission);
        assert!(state.compositor.needs_redraw());
    }

    #[test]
    fn none_transition_commits_scene_straight_from_pre_transition() {
        let mut state = make_app_state();
        state
            .automatic_cycling
            .set_config(SceneCyclingRuntimeConfig {
                transition: SceneCyclingTransition::None,
                ..cycling_config(true)
            });
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        let now = Instant::now();
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(now);
        state.compositor.clear_output_damage();

        state.on_scene_cycling_timer(now + PRE_TRANSITION_DURATION);

        assert!(
            !state.compositor.widgets.automatic_transition_active(),
            "None transition must commit without an animated Transition phase"
        );
        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::WaitingForTimer { .. }
        ));
        assert!(state.compositor.needs_redraw());
        assert!(state.pending_lifecycle_emission);
    }

    #[test]
    fn cancelling_automatic_transition_restores_lifecycle_and_damages_output() {
        let mut state = make_app_state();
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(Instant::now());
        let _ = emit_lifecycle_transitions(&mut state);
        state.compositor.clear_output_damage();

        state.pause_automatic_cycling_for_touch(Instant::now());

        assert!(!state.compositor.widgets.automatic_transition_active());
        assert!(state.compositor.needs_redraw());
        let emission = state
            .compositor
            .lifecycle
            .step(&state.compositor.widgets.lifecycle_states());
        assert!(
            emission.is_empty(),
            "cancellation should emit restored lifecycle before returning"
        );
    }

    #[test]
    fn cancelling_automatic_transition_with_four_scenes_restores_lifecycle_inline() {
        let mut state = make_app_state();
        state.compositor.widgets.set_scene_cycling(vec![
            test_scene("a"),
            test_scene("b"),
            test_scene("c"),
            test_scene("d"),
        ]);
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: four scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(Instant::now());
        let _ = emit_lifecycle_transitions(&mut state);
        state.compositor.clear_output_damage();

        state.pause_automatic_cycling_for_touch(Instant::now());

        assert!(!state.compositor.widgets.automatic_transition_active());
        assert!(state.compositor.needs_redraw());
        assert!(!state.pending_lifecycle_emission);
        let emission = state
            .compositor
            .lifecycle
            .step(&state.compositor.widgets.lifecycle_states());
        assert!(
            emission.is_empty(),
            "cancellation should emit restored lifecycle before returning"
        );
    }

    #[test]
    fn cancelling_automatic_transition_defers_when_emission_already_pending() {
        let mut state = make_app_state();
        state.compositor.widgets.set_scene_cycling(vec![
            test_scene("a"),
            test_scene("b"),
            test_scene("c"),
            test_scene("d"),
        ]);
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: four scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(Instant::now());
        let _ = emit_lifecycle_transitions(&mut state);
        state.compositor.clear_output_damage();

        // A prior scene change armed a deferred emission that has not flushed yet.
        state.pending_lifecycle_emission = true;

        state.pause_automatic_cycling_for_touch(Instant::now());

        assert!(!state.compositor.widgets.automatic_transition_active());
        assert!(state.compositor.needs_redraw());
        assert!(
            state.pending_lifecycle_emission,
            "a cancel must leave a prior deferred emission armed, not clear it",
        );
        let emission = state
            .compositor
            .lifecycle
            .step(&state.compositor.widgets.lifecycle_states());
        assert!(
            !emission.is_empty(),
            "restored lifecycle must remain deferred for the post-render flush, not be emitted inline",
        );
    }

    #[test]
    fn set_active_scene_discards_stale_transition_without_emitting_old_lifecycle() {
        let mut state = make_app_state();
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        let _ = emit_lifecycle_transitions(&mut state);
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(Instant::now());
        let _ = emit_lifecycle_transitions(&mut state);
        state.pending_transition_warm_up = Some(TransitionWarmUp {
            started_at: Instant::now(),
            widgets: HashMap::new(),
        });

        handle_command(
            &mut state,
            CompositorCommand::SetActiveScene {
                layout: test_scene("preview"),
            },
        );

        assert!(!state.compositor.widgets.automatic_transition_active());
        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
        assert!(state.pending_transition_warm_up.is_none());
        assert_eq!(
            state.compositor.lifecycle.last_state(&"a".to_owned()),
            Some(LifecycleState::Leaving),
            "scene replacement must not emit a lifecycle rollback for the outgoing scene"
        );
        assert_eq!(
            state.compositor.lifecycle.last_state(&"b".to_owned()),
            Some(LifecycleState::Entering),
            "scene replacement must not wake the old transition target before removing it"
        );
    }

    #[test]
    fn reset_to_first_scene_clears_stale_automatic_transition() {
        let mut state = make_app_state();
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(Instant::now());
        state.pending_transition_warm_up = Some(TransitionWarmUp {
            started_at: Instant::now(),
            widgets: HashMap::new(),
        });

        handle_command(&mut state, CompositorCommand::ResetToFirstScene);

        assert!(!state.compositor.widgets.automatic_transition_active());
        assert!(state.pending_transition_warm_up.is_none());
    }

    #[test]
    fn set_scene_cycling_resets_stale_automatic_transition_phase() {
        let mut state = make_app_state();
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(Instant::now());
        state.pending_transition_warm_up = Some(TransitionWarmUp {
            started_at: Instant::now(),
            widgets: HashMap::new(),
        });

        handle_command(
            &mut state,
            CompositorCommand::SetSceneCycling {
                scenes: vec![test_scene("a"), test_scene("b")],
            },
        );

        assert!(!state.compositor.widgets.automatic_transition_active());
        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::WaitingForTimer { .. }
        ));
        assert!(
            state.pending_transition_warm_up.is_none(),
            "replacing scenes must complete interruption cleanup before clearing the widget transition"
        );
    }

    #[test]
    fn reevaluation_cleans_up_a_transition_when_cycling_becomes_invalid() {
        let mut state = make_cycling_app_state();
        begin_automatic_slide(&mut state);
        state.pending_transition_warm_up = Some(TransitionWarmUp {
            started_at: Instant::now(),
            widgets: HashMap::new(),
        });
        state.automatic_cycling.set_suspended(true);

        state.reevaluate_automatic_cycling(Instant::now());

        assert!(
            !state.compositor.widgets.automatic_transition_active(),
            "cycling phase and tracker transition must leave motion together"
        );
        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
        assert!(state.pending_transition_warm_up.is_none());
    }

    #[test]
    fn set_scene_cycling_config_command_updates_automatic_duration() {
        let mut state = make_app_state();
        let duration = Duration::from_secs(7);

        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingConfig {
                config: SceneCycling {
                    automatic_cycling_default_duration: duration,
                    ..SceneCycling::default()
                },
            },
        );

        assert_eq!(state.active_scene_cycle_duration(), duration);
    }

    #[test]
    fn set_upgrade_state_command_installs_the_authoritative_snapshot() {
        let mut state = make_app_state();
        let snapshot = UpgradeDisplaySnapshot {
            generation: UpgradeGeneration::new(9),
            state: UpgradeDisplayState::Succeeded {
                kind: UpgradeKind::Firmware,
            },
        };

        handle_command(
            &mut state,
            CompositorCommand::SetUpgradeState {
                state: snapshot.clone(),
            },
        );

        assert_eq!(state.compositor.upgrade.current_snapshot(), Some(&snapshot));
    }

    #[test]
    fn active_scene_cycle_duration_prefers_scene_duration() {
        let mut state = make_app_state();
        let duration = Duration::from_secs(5);
        state.compositor.widgets.set_scene_cycling(vec![
            test_scene_with_cycle_duration("a", duration),
            test_scene("b"),
        ]);

        assert_eq!(state.active_scene_cycle_duration(), duration);
    }

    /// Records `send`/`release_buffers`/`flush` calls in order. `send`
    /// and `release_buffers` return a synthetic per-instance `ClientId`
    /// so the test can verify that the flush boundary between batches
    /// refers to the same clients that received the preceding batch.
    #[derive(Default)]
    struct RecordingSink {
        events: Vec<RecordedEvent>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum RecordedEvent {
        Send(InstanceId, LifecycleState),
        ReleaseBuffers(InstanceId),
        TransitionIncoming(InstanceId),
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
        fn release_buffers(&mut self, instance_id: &InstanceId) -> Vec<Self::ClientId> {
            self.events
                .push(RecordedEvent::ReleaseBuffers(instance_id.clone()));
            vec![format!("client-{instance_id}")]
        }
        fn send_transition_incoming(&mut self, instance_id: &InstanceId) -> Option<Self::ClientId> {
            self.events
                .push(RecordedEvent::TransitionIncoming(instance_id.clone()));
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
                RecordedEvent::ReleaseBuffers(String::from("a")),
                RecordedEvent::ReleaseBuffers(String::from("b")),
                RecordedEvent::Flush(String::from("client-a")),
                RecordedEvent::Flush(String::from("client-b")),
                RecordedEvent::Send(String::from("c"), LifecycleState::Visible),
                RecordedEvent::Send(String::from("d"), LifecycleState::Prepared),
                RecordedEvent::Flush(String::from("client-c")),
                RecordedEvent::Flush(String::from("client-d")),
            ],
            "release sends and buffer releases must precede release flushes, \
             which must precede acquire sends — a host must observe \
             wl_buffer.release before any acquire lifecycle",
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
    fn transition_incoming_batch_sends_then_flushes_affected_clients() {
        let mut sink = RecordingSink::default();
        emit_transition_incoming_batch(&[String::from("a"), String::from("b")], &mut sink);

        assert_eq!(
            sink.events,
            vec![
                RecordedEvent::TransitionIncoming(String::from("a")),
                RecordedEvent::TransitionIncoming(String::from("b")),
                RecordedEvent::Flush(String::from("client-a")),
                RecordedEvent::Flush(String::from("client-b")),
            ],
        );
    }

    #[test]
    fn transition_incoming_widget_ids_include_only_visible_widgets() {
        let scene = SceneLayout {
            scene_id: None,
            cycle_duration: None,
            combined: false,
            widgets: vec![
                WidgetPlacement {
                    instance_id: String::from("visible"),
                    position: Position { x: 0, y: 0 },
                    size: Size {
                        width: 480,
                        height: 1280,
                    },
                    visible: true,
                },
                WidgetPlacement {
                    instance_id: String::from("hidden"),
                    position: Position { x: 0, y: 0 },
                    size: Size {
                        width: 480,
                        height: 1280,
                    },
                    visible: false,
                },
            ],
        };

        assert_eq!(
            transition_incoming_widget_ids(&scene),
            vec![String::from("visible")]
        );
    }

    #[test]
    fn transition_warm_up_waits_for_target_widget_commit() {
        let started_at = Instant::now();
        let warm_up = TransitionWarmUp {
            started_at,
            widgets: HashMap::from([(String::from("incoming"), Some(gen_nz(7)))]),
        };

        assert!(!transition_warm_up_ready(
            Some(&warm_up),
            started_at + Duration::from_millis(100),
            |id| {
                assert_eq!(id, "incoming");
                Some(gen_nz(7))
            },
        ));
        assert!(transition_warm_up_ready(
            Some(&warm_up),
            started_at + Duration::from_millis(100),
            |_| Some(gen_nz(8)),
        ));
    }

    #[test]
    fn transition_warm_up_allows_one_second_for_a_slow_widget() {
        let started_at = Instant::now();
        let warm_up = TransitionWarmUp {
            started_at,
            widgets: HashMap::from([(String::from("incoming"), Some(gen_nz(7)))]),
        };

        assert!(!transition_warm_up_ready(
            Some(&warm_up),
            started_at + Duration::from_millis(999),
            |_| Some(gen_nz(7)),
        ));
        assert!(transition_warm_up_ready(
            Some(&warm_up),
            started_at + Duration::from_secs(1),
            |_| Some(gen_nz(7)),
        ));
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
            fn release_buffers(&mut self, instance_id: &InstanceId) -> Vec<Self::ClientId> {
                self.events
                    .push(RecordedEvent::ReleaseBuffers(instance_id.clone()));
                vec![String::from("shared-client")]
            }
            fn send_transition_incoming(
                &mut self,
                instance_id: &InstanceId,
            ) -> Option<Self::ClientId> {
                self.events
                    .push(RecordedEvent::TransitionIncoming(instance_id.clone()));
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
    fn buffer_release_for_unreachable_widget_is_flushed_before_acquires() {
        // A widget removed from the scene-cycling list has no surface to
        // send lifecycle to (`send` returns None), but the compositor may
        // still hold its buffers. The wl_buffer.release client must be
        // flushed before any acquire send, or the acquiring slot can
        // allocate while the dormant slot's buffer is still held.
        struct NoSendSink {
            events: Vec<RecordedEvent>,
        }
        impl LifecycleSink for NoSendSink {
            type ClientId = String;
            fn send(
                &mut self,
                instance_id: &InstanceId,
                state: LifecycleState,
            ) -> Option<Self::ClientId> {
                self.events
                    .push(RecordedEvent::Send(instance_id.clone(), state));
                None
            }
            fn release_buffers(&mut self, instance_id: &InstanceId) -> Vec<Self::ClientId> {
                self.events
                    .push(RecordedEvent::ReleaseBuffers(instance_id.clone()));
                vec![String::from("buffer-client")]
            }
            fn send_transition_incoming(
                &mut self,
                instance_id: &InstanceId,
            ) -> Option<Self::ClientId> {
                self.events
                    .push(RecordedEvent::TransitionIncoming(instance_id.clone()));
                None
            }
            fn flush(&mut self, client_id: &Self::ClientId) {
                self.events.push(RecordedEvent::Flush(client_id.clone()));
            }
        }

        let emission = Emission {
            releases: vec![(String::from("a"), LifecycleState::Dormant)],
            acquires: vec![(String::from("b"), LifecycleState::Visible)],
        };

        let mut sink = NoSendSink { events: Vec::new() };
        emit_lifecycle_batches(&emission, &mut sink);

        assert_eq!(
            sink.events,
            vec![
                RecordedEvent::Send(String::from("a"), LifecycleState::Dormant),
                RecordedEvent::ReleaseBuffers(String::from("a")),
                RecordedEvent::Flush(String::from("buffer-client")),
                RecordedEvent::Send(String::from("b"), LifecycleState::Visible),
            ],
        );
    }

    #[test]
    fn stale_clear_pid_keeps_lifecycle_history_for_live_respawned_widget() {
        let mut state = make_app_state();
        let instance_id = String::from("alpha");
        state.compositor.deck_widget_state.register_widget(
            instance_id.clone(),
            GEN,
            make_widget_config(),
        );
        state
            .compositor
            .deck_widget_state
            .set_widget_pid(&instance_id, GEN, 200);
        let _ = state.compositor.deck_widget_state.drain_connected();

        let _ = state.compositor.lifecycle.step(&lifecycle_map(&[(
            instance_id.clone(),
            LifecycleState::Visible,
        )]));

        handle_clear_pid_command(&mut state, &instance_id, GEN, 100);

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

    #[test]
    fn settings_receiver_receives_compositor_settings_commands() {
        use bmc::compositor::{Compositor, SettingsCommand};
        use bmc_platform::{HardwareProfile, Product};

        let compositor =
            super::EglCompositor::new(HardwareProfile::for_product(Product::Bmc100), true);
        let mut rx = compositor.settings_receiver();

        compositor
            .settings_tx
            .send(SettingsCommand::SetBrightness(42))
            .expect("BUG: settings receiver should be alive in this test");

        assert!(matches!(
            rx.try_recv(),
            Ok(SettingsCommand::SetBrightness(42))
        ));
    }

    /// Cycling enabled, two scenes, sitting on scene 1 and waiting to cycle.
    fn make_cycling_app_state() -> AppState {
        let mut state = make_app_state();
        state.automatic_cycling.set_config(cycling_config(true));
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);
        state.compositor.widgets.set_active_scene_index(1);
        state.reevaluate_automatic_cycling(Instant::now());
        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::WaitingForTimer { .. }
        ));
        state.compositor.clear_output_damage();
        state
    }

    #[test]
    fn suspend_pauses_cycling_without_moving_the_scene() {
        // The night-mode fix: cycling stops as soon as night mode starts, while
        // the panel is still lit, so it must not yank the visible scene around.
        let mut state = make_cycling_app_state();

        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingSuspended { suspended: true },
        );

        assert!(
            matches!(
                state.automatic_cycling.phase(),
                AutomaticCyclingPhase::PausedDisabled { .. }
            ),
            "night mode must stop cycling immediately"
        );
        assert_eq!(
            state.compositor.widgets.active_visible_widget_ids(),
            vec!["b".to_owned()],
            "suspending must leave the lit panel on the scene it was showing"
        );
    }

    /// Put `state` into a running slide, the way the cycling timer would.
    fn begin_automatic_slide(state: &mut AppState) {
        state
            .compositor
            .widgets
            .begin_automatic_transition_to_next()
            .expect("BUG: two scenes should have an automatic transition target");
        state.automatic_cycling.enter_pre_transition(Instant::now());
        let _ = emit_lifecycle_transitions(state);
    }

    #[test]
    fn suspending_mid_slide_reverts_the_slide() {
        // Night mode can start while a slide is running. Without the undo the
        // scene is left half slid, with nothing left to finish the move.
        let mut state = make_cycling_app_state();
        begin_automatic_slide(&mut state);

        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingSuspended { suspended: true },
        );

        assert!(
            !state.compositor.widgets.automatic_transition_active(),
            "suspending must revert an in-flight slide, not strand it"
        );
        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::PausedDisabled { .. }
        ));
        assert_eq!(
            state.compositor.widgets.active_visible_widget_ids(),
            vec!["b".to_owned()],
            "reverting a slide must restore the scene it started from"
        );
    }

    #[test]
    fn resuming_mid_slide_lets_the_slide_finish() {
        // The night-mode watch also fires when the value did not change, so a
        // daytime schedule edit re-sends `suspended: false` while cycling runs.
        // That must not cut off a slide the user is watching.
        let mut state = make_cycling_app_state();
        begin_automatic_slide(&mut state);

        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingSuspended { suspended: false },
        );

        assert!(
            state.compositor.widgets.automatic_transition_active(),
            "a redundant resume must leave a running slide alone"
        );
        assert!(matches!(
            state.automatic_cycling.phase(),
            AutomaticCyclingPhase::PreTransition { .. }
        ));
    }

    #[test]
    fn reset_to_first_scene_lands_on_scene_zero_and_stays_paused() {
        let mut state = make_cycling_app_state();
        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingSuspended { suspended: true },
        );
        state.compositor.clear_output_damage();

        handle_command(&mut state, CompositorCommand::ResetToFirstScene);

        assert_eq!(
            state.compositor.widgets.active_visible_widget_ids(),
            vec!["a".to_owned()],
            "screen-off must reset to the first scene"
        );
        assert!(
            state.compositor.needs_redraw(),
            "the scene-0 frame must be scheduled while the panel is dark"
        );
        assert!(
            matches!(
                state.automatic_cycling.phase(),
                AutomaticCyclingPhase::PausedDisabled { .. }
            ),
            "the reset must not re-arm cycling while still suspended"
        );
    }

    #[test]
    fn reset_landing_mid_drag_survives_the_lift() {
        // The reset only stops tracking the drag; the gesture machine keeps its
        // accumulated dx either way, so the lift decides on its own numbers.
        let mut state = make_cycling_app_state();

        state.on_touch_down(&FakeTouchEvent::at_x(0, 1, 400.0));
        state.on_touch_motion(&FakeTouchEvent::at_x(0, 20, 100.0));
        assert!(
            state.scene_drag_active,
            "a 300 px swipe on a lit panel must activate the scene drag"
        );

        handle_command(&mut state, CompositorCommand::ResetToFirstScene);
        state.on_touch_up(&FakeTouchEvent::at_x(0, 40, 100.0));

        assert_eq!(
            state.compositor.widgets.active_visible_widget_ids(),
            vec!["a".to_owned()],
            "a lift past the commit threshold must not advance off the scene \
             the reset chose"
        );
    }

    #[test]
    fn resume_re_arms_cycling_with_a_fresh_timer() {
        let mut state = make_cycling_app_state();
        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingSuspended { suspended: true },
        );
        handle_command(&mut state, CompositorCommand::ResetToFirstScene);

        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingSuspended { suspended: false },
        );

        assert_eq!(
            state.compositor.widgets.active_visible_widget_ids(),
            vec!["a".to_owned()],
            "leaving night mode must not move the scene"
        );
        assert!(
            matches!(
                state.automatic_cycling.phase(),
                AutomaticCyclingPhase::WaitingForTimer { .. }
            ),
            "leaving night mode must resume cycling with a fresh timer"
        );
    }

    #[test]
    fn touch_on_dark_screen_wakes_but_is_not_delivered() {
        let screen = FakeScreenVisibility::new(false);
        let mut state = make_app_state_with_screen(&screen);
        let mut event_rx = state.event_tx.subscribe();

        state.on_touch_down(&FakeTouchEvent::new(0, 1));

        assert!(
            matches!(event_rx.try_recv(), Ok(CompositorEvent::ScreenActivity)),
            "the consumed touch must still wake the screen"
        );
        assert!(
            state.gesture_slot.is_none(),
            "a dark-screen touch must not enter gesture arbitration"
        );
        assert!(
            !state.touch_frame_dirty,
            "a dark-screen touch must not emit wl_touch events"
        );
        assert!(
            !state.active_touch_slots.is_empty(),
            "the consumed slot still counts toward the touch sequence"
        );

        state.on_touch_up(&FakeTouchEvent::new(0, 2));
        assert!(state.active_touch_slots.is_empty());

        screen.set_visible(true);
        state.on_touch_down(&FakeTouchEvent::new(0, 3));
        assert!(
            state.gesture_slot.is_some(),
            "a touch after wake must be delivered normally"
        );
    }

    #[test]
    fn touch_guard_follows_the_panel_not_the_command_queue() {
        // Both directions of the race a mirrored screen-power flag lost: the
        // hardware transition happens first, any command about it arrives
        // later, and touches in between must follow the panel.
        let screen = FakeScreenVisibility::new(true);
        let mut state = make_app_state_with_screen(&screen);

        screen.set_visible(false);
        state.on_touch_down(&FakeTouchEvent::new(0, 1));
        assert!(
            state.gesture_slot.is_none(),
            "a touch after power-off must be consumed before the command lands"
        );
        state.on_touch_up(&FakeTouchEvent::new(0, 2));

        screen.set_visible(true);
        state.on_touch_down(&FakeTouchEvent::new(0, 3));
        assert!(
            state.gesture_slot.is_some(),
            "a touch on a lit panel must be delivered before the command lands"
        );
    }

    #[test]
    fn unreadable_backlight_delivers_touches() {
        let screen = FakeScreenVisibility::new(false);
        let mut state = make_app_state_with_screen(&screen);
        screen.fail_reads();

        state.on_touch_down(&FakeTouchEvent::new(0, 1));

        assert!(
            state.gesture_slot.is_some(),
            "an unreadable backlight must fail open, never swallow every touch"
        );
    }

    #[test]
    fn alarm_dismiss_wins_over_dark_screen_touch_inhibit() {
        let screen = FakeScreenVisibility::new(false);
        let mut state = make_app_state_with_screen(&screen);
        state.compositor.alarm.ring("07:30", "", "", false);

        state.on_touch_down(&FakeTouchEvent::new(0, 1));

        assert!(
            !state.compositor.alarm.is_ringing(),
            "a touch on a dark ringing screen must still dismiss the alarm"
        );
    }

    #[test]
    fn resume_does_not_override_disabled_cycling_config() {
        let mut state = make_app_state();
        state.automatic_cycling.set_config(cycling_config(false));
        state
            .compositor
            .widgets
            .set_scene_cycling(vec![test_scene("a"), test_scene("b")]);

        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingSuspended { suspended: true },
        );
        handle_command(
            &mut state,
            CompositorCommand::SetSceneCyclingSuspended { suspended: false },
        );

        assert!(
            matches!(
                state.automatic_cycling.phase(),
                AutomaticCyclingPhase::PausedDisabled { .. }
            ),
            "resuming lifts only the suspend gate, never the user's disabled config"
        );
    }
}
