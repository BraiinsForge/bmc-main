// Copyright (C) 2025  Braiins Systems s.r.o.

//! EGL Compositor implementation for bmc-openwrt.

use super::{
    commands::CompositorCommand,
    render_egl::EglRenderState,
    state::{ClientState, CompositorState},
};
use bmc::compositor::{
    Compositor, CompositorError, CompositorEvent, InstanceId, Position, SceneLayout, Size,
    WidgetAction,
};
use bmc_widget_protocol::SettingUpdate;
use smithay::reexports::{
    calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic},
    drm::control::{Device as DrmControlDevice, Event as DrmEvent},
    wayland_server::{Display, ListeningSocket},
};
use std::{
    os::fd::AsFd,
    path::Path,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::mpsc;

const DEFAULT_GPU_PATH: &str = "/dev/dri/renderD128";
const DEFAULT_DISPLAY_PATH: &str = "/dev/dri/card1";

#[derive(Debug)]
struct SharedState {
    wayland_display: Option<String>,
    running: bool,
}

#[derive(Debug)]
pub struct EglCompositor {
    shared: Arc<Mutex<SharedState>>,
    command_tx: flume::Sender<CompositorCommand>,
    command_rx: flume::Receiver<CompositorCommand>,
    action_tx: mpsc::UnboundedSender<WidgetAction>,
    event_tx: mpsc::UnboundedSender<CompositorEvent>,
    action_rx: Mutex<Option<mpsc::UnboundedReceiver<WidgetAction>>>,
    event_rx: Mutex<Option<mpsc::UnboundedReceiver<CompositorEvent>>>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    gpu_path: String,
    display_path: String,
}

impl EglCompositor {
    #[must_use]
    pub fn new() -> Self {
        Self::with_device_paths(DEFAULT_GPU_PATH, DEFAULT_DISPLAY_PATH)
    }

    #[must_use]
    pub fn with_device_paths(gpu_path: &str, display_path: &str) -> Self {
        let (command_tx, command_rx) = flume::unbounded();
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            shared: Arc::new(Mutex::new(SharedState {
                wayland_display: None,
                running: false,
            })),
            command_tx,
            command_rx,
            action_tx,
            event_tx,
            action_rx: Mutex::new(Some(action_rx)),
            event_rx: Mutex::new(Some(event_rx)),
            thread_handle: Mutex::new(None),
            gpu_path: gpu_path.to_owned(),
            display_path: display_path.to_owned(),
        }
    }

    fn run_compositor_loop(
        gpu_path: String,
        display_path: String,
        command_rx: flume::Receiver<CompositorCommand>,
        action_tx: mpsc::UnboundedSender<WidgetAction>,
        event_tx: mpsc::UnboundedSender<CompositorEvent>,
        shared: Arc<Mutex<SharedState>>,
    ) {
        tracing::info!("Compositor thread starting...");

        // Initialize EGL render state with split GPU/display devices
        let render_state = match EglRenderState::new(Path::new(&gpu_path), Path::new(&display_path))
        {
            Ok(state) => state,
            Err(e) => {
                tracing::error!("Failed to initialize EGL render state: {}", e);
                return;
            }
        };

        let (logical_width, logical_height) = render_state.logical_size();
        tracing::info!(
            "Display configured: {}x{} logical (rotated)",
            logical_width,
            logical_height
        );

        // Create calloop event loop for Wayland dispatch
        let mut event_loop: EventLoop<'_, AppState> = match EventLoop::try_new() {
            Ok(el) => el,
            Err(e) => {
                tracing::error!("Failed to create event loop: {}", e);
                return;
            }
        };

        // Create Wayland display and compositor state
        let mut display: Display<CompositorState> = match Display::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to create Wayland display: {}", e);
                return;
            }
        };

        let compositor_state = CompositorState::new(&display, logical_width, logical_height);

        // Bind to Wayland socket
        let listening_socket = match ListeningSocket::bind_auto("wayland", 0..33) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to create Wayland socket: {}", e);
                return;
            }
        };

        let socket_name = match listening_socket.socket_name() {
            Some(name) => name.to_string_lossy().to_string(),
            None => {
                tracing::error!("Failed to get socket name");
                return;
            }
        };

        tracing::info!("Wayland socket created: {}", socket_name);

        // Update shared state so main thread knows we're ready
        {
            let mut shared = shared.lock().expect("BUG: shared state lock poisoned");
            shared.wayland_display = Some(socket_name.clone());
            shared.running = true;
        }

        // Add Wayland display fd to event loop for client dispatch
        let loop_handle = event_loop.handle();
        if let Ok(poll_fd) = display.backend().poll_fd().try_clone_to_owned() {
            if loop_handle
                .insert_source(
                    Generic::new(poll_fd, Interest::READ, Mode::Level),
                    |_, _, state| {
                        state.display.dispatch_clients(&mut state.compositor)?;
                        Ok(PostAction::Continue)
                    },
                )
                .is_err()
            {
                tracing::error!("Failed to add display fd to event loop");
                return;
            }
        }

        let mut app_state = AppState {
            display,
            compositor: compositor_state,
            render_state,
            listening_socket,
            command_rx,
            action_tx,
            event_tx,
            should_exit: false,
        };

        // Add DRM device fd for vblank/page-flip events
        if let Ok(drm_fd) = app_state
            .render_state
            .display_drm()
            .device_fd()
            .as_fd()
            .try_clone_to_owned()
        {
            let _ = loop_handle.insert_source(
                Generic::new(drm_fd, Interest::READ, Mode::Level),
                |_, _, state| {
                    if let Ok(events) = state.render_state.display_drm().receive_events() {
                        for event in events {
                            match event {
                                DrmEvent::Vblank(_) | DrmEvent::PageFlip(_) => {
                                    state.render_state.on_vblank();
                                }
                                DrmEvent::Unknown(_) => {}
                            }
                        }
                    }
                    Ok(PostAction::Continue)
                },
            );
        }

        tracing::info!("Compositor event loop starting");

        // Main event loop: process commands, accept clients, render frames
        loop {
            while let Ok(cmd) = app_state.command_rx.try_recv() {
                handle_command(&mut app_state, cmd);
            }

            if app_state.should_exit {
                tracing::info!("Compositor shutting down");
                break;
            }

            if let Ok(Some(client_stream)) = app_state.listening_socket.accept() {
                if let Err(e) = app_state
                    .display
                    .handle()
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                {
                    tracing::error!("Failed to insert client: {}", e);
                } else {
                    tracing::info!("New Wayland client connected");
                }
            }

            process_protocol_events(&mut app_state);

            let buffer = app_state.compositor.current_buffer.as_ref();
            if let Err(e) = app_state.render_state.render_frame(buffer) {
                tracing::error!("Render error: {}", e);
            }

            #[expect(
                clippy::cast_possible_truncation,
                reason = "wrapping is acceptable for frame time"
            )]
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0);
            app_state.compositor.send_frame_callbacks(time);

            let _ = app_state.display.flush_clients();

            // 16ms dispatch timeout for ~60fps
            if event_loop
                .dispatch(Some(Duration::from_millis(16)), &mut app_state)
                .is_err()
            {
                tracing::error!("Event loop dispatch error");
                break;
            }
        }

        {
            let mut shared = shared.lock().expect("BUG: shared state lock poisoned");
            shared.running = false;
        }

        tracing::info!("Compositor thread exiting");
    }
}

struct AppState {
    display: Display<CompositorState>,
    compositor: CompositorState,
    render_state: EglRenderState,
    listening_socket: ListeningSocket,
    command_rx: flume::Receiver<CompositorCommand>,
    action_tx: mpsc::UnboundedSender<WidgetAction>,
    event_tx: mpsc::UnboundedSender<CompositorEvent>,
    should_exit: bool,
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
            state
                .compositor
                .register_widget(instance_id, position, size, pid);
        }
        CompositorCommand::UnregisterWidget { instance_id } => {
            tracing::debug!("Unregistering widget {}", instance_id);
            state.compositor.unregister_widget(&instance_id);
            state
                .compositor
                .deck_widget_state
                .unregister_widget(&instance_id);
        }
        CompositorCommand::SetActiveScene { layout } => {
            tracing::debug!("Setting active scene with {} widgets", layout.widgets.len());
            state.compositor.active_scene = layout;
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

impl Default for EglCompositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor for EglCompositor {
    fn start(&self) -> Result<String, CompositorError> {
        {
            let shared = self.shared.lock().expect("BUG: shared state lock poisoned");
            if shared.running {
                return Err(CompositorError::AlreadyStarted);
            }
        }

        let gpu_path = self.gpu_path.clone();
        let display_path = self.display_path.clone();
        let command_rx = self.command_rx.clone();
        let action_tx = self.action_tx.clone();
        let event_tx = self.event_tx.clone();
        let shared = self.shared.clone();

        let handle = thread::Builder::new()
            .name("egl-compositor".to_owned())
            .spawn(move || {
                Self::run_compositor_loop(
                    gpu_path,
                    display_path,
                    command_rx,
                    action_tx,
                    event_tx,
                    shared,
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

        let start_time = std::time::Instant::now();
        loop {
            {
                let shared = self.shared.lock().expect("BUG: shared state lock poisoned");
                if let Some(ref name) = shared.wayland_display {
                    return Ok(name.clone());
                }
                if !shared.running && start_time.elapsed() > Duration::from_secs(1) {
                    break;
                }
            }
            if start_time.elapsed() > Duration::from_secs(10) {
                return Err(CompositorError::ThreadError(
                    "Timeout waiting for compositor to start".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }

        Err(CompositorError::ThreadError(
            "Compositor thread exited unexpectedly".to_owned(),
        ))
    }

    fn wayland_display(&self) -> Option<String> {
        let shared = self.shared.lock().expect("BUG: shared state lock poisoned");
        shared.wayland_display.clone()
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
