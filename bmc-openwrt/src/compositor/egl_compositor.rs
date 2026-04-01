// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL Compositor implementation for bmc-openwrt.

use super::{
    commands::CompositorCommand,
    render::{DrmOutput, EglContext},
    scene_renderer::SceneRenderer,
    state::{ClientState, CompositorState},
};
use bmc::compositor::{
    Compositor, CompositorError, CompositorEvent, InstanceId, Position, SceneLayout, Size,
    WidgetAction,
};
use bmc_widget_protocol::SettingUpdate;
use smithay::reexports::{
    calloop::{
        EventLoop, Interest, Mode, PostAction,
        channel::{self as calloop_channel, Event as ChannelEvent},
        generic::Generic,
    },
    drm::control::{Device as DrmControlDevice, Event as DrmEvent},
    wayland_server::{Display, ListeningSocket},
};
use std::{
    os::fd::AsFd,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::mpsc;

const DEFAULT_GPU_PATH: &str = "/dev/dri/renderD128";
const DEFAULT_DISPLAY_PATH: &str = "/dev/dri/card1";

/// Physical display dimensions (panel reports 600x1280 but only 480x1280 is visible).
const PHYSICAL_WIDTH: u32 = 480;
const PHYSICAL_HEIGHT: u32 = 1_280;

/// Synthetic refresh (mHz) advertised for headless mode; matches the
/// `HEADLESS_FRAME_INTERVAL` timer that paces frame callbacks.
const HEADLESS_REFRESH_MHZ: i32 = 60_000;

/// Headless frame-callback pacing (~60 Hz).
const HEADLESS_FRAME_INTERVAL: Duration = Duration::from_millis(16);

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

        if let Some(renderer) = &app_state.scene_renderer {
            // Add DRM device fd for vblank/page-flip events
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
                                        DrmEvent::Vblank(_) | DrmEvent::PageFlip(_) => {
                                            r.output_mut().on_vblank();
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
        } else {
            // Headless: synthetic ~60 Hz timer to pace frame callbacks
            use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
            let timer = Timer::from_duration(HEADLESS_FRAME_INTERVAL);
            if let Err(e) = loop_handle.insert_source(timer, |_, (), state| {
                state.refresh_redraw_state();
                if matches!(state.redraw_state, RedrawState::Queued) {
                    state.redraw_state = RedrawState::Idle;
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "wrapping is acceptable for frame time"
                    )]
                    let time = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u32)
                        .unwrap_or(0);
                    state
                        .compositor
                        .send_frame_callbacks_for_presented_widgets(time);
                }
                TimeoutAction::ToDuration(HEADLESS_FRAME_INTERVAL)
            }) {
                tracing::error!("Failed to add headless timer to event loop: {e}");
            }
        }

        tracing::info!("Compositor event loop starting");

        #[cfg(feature = "profiling")]
        let mut loop_w = ii_stopwatch::StopWatch::default();
        #[cfg(feature = "profiling")]
        let mut render_w = ii_stopwatch::StopWatch::default();
        #[cfg(feature = "profiling")]
        let mut callbacks_w = ii_stopwatch::StopWatch::default();
        #[cfg(feature = "profiling")]
        let mut dispatch_w = ii_stopwatch::StopWatch::default();
        #[cfg(feature = "profiling")]
        let mut every = ii_stopwatch::Every::new(std::time::Duration::from_secs(5));

        // Main event loop — fully event-driven via calloop.
        //
        // Calloop event sources (registered above):
        //   - Wayland display fd  → client commits, protocol events
        //   - Wayland listener fd → new client connections
        //   - DRM device fd       → vblank / page-flip-complete
        //   - Command channel     → scene changes, shutdown, settings
        //   - Timer (headless)    → synthetic ~60 Hz tick (only without DRM)
        //
        // State machine per iteration:
        //   1. Process protocol events (widget connect/disconnect/actions)
        //   2. If `needs_redraw` — attempt render:
        //      a. `is_flip_pending()` → skip, keep `needs_redraw` for next wake
        //      b. render succeeds → clear `needs_redraw`, send frame callbacks
        //   3. Fulfill any pending capture frames from pixel cache
        //   4. `dispatch(timeout)` — sleep until the next event:
        //      - `needs_redraw && !flip_pending` → ZERO (render ASAP)
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

                    // Only clear redraw state and send frame callbacks
                    // when a frame was actually produced.
                    //
                    // If render was skipped (flip pending), keep redraw queued so
                    // the dispatch timeout (below) blocks on the DRM fd until the flip completes,
                    // then renders on the next iteration.
                    //
                    // Withhold callbacks to pace widget rendering at the actual display rate.
                    if rendered {
                        app_state.compositor.clear_output_damage();
                        app_state.redraw_state = RedrawState::on_frame_submitted();

                        ii_stopwatch::stopwatch_start!(callbacks_w);
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "wrapping is acceptable for frame time"
                        )]
                        let time = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u32)
                            .unwrap_or(0);
                        app_state
                            .compositor
                            .send_frame_callbacks_for_presented_widgets(time);
                        ii_stopwatch::stopwatch_stop!(callbacks_w);
                    } else {
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
                // Headless: discard buffers, frame callbacks fired by the timer
                app_state.compositor.invalidated_buffers.clear();
                app_state.compositor.dirty_buffers.clear();
                app_state.compositor.pending_capture_frames.clear();
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
                    "compositor: loop={} render={} callbacks={} dispatch={}",
                    loop_w,
                    render_w,
                    callbacks_w,
                    dispatch_w
                );
                ii_stopwatch::stopwatch_reset!(loop_w);
                ii_stopwatch::stopwatch_reset!(render_w);
                ii_stopwatch::stopwatch_reset!(callbacks_w);
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

        if self.compositor.needs_redraw {
            self.redraw_state = self.redraw_state.queue();
            self.compositor.needs_redraw = false;
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
            state.redraw_state = state.redraw_state.queue();
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
