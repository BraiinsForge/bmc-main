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
    calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic},
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

#[derive(Debug)]
pub struct EglCompositor {
    wayland_display: Mutex<Option<String>>,
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
            wayland_display: Mutex::new(None),
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

    #[expect(clippy::too_many_lines)]
    fn run_compositor_loop(
        gpu_path: &str,
        display_path: &str,
        command_rx: flume::Receiver<CompositorCommand>,
        action_tx: mpsc::UnboundedSender<WidgetAction>,
        event_tx: mpsc::UnboundedSender<CompositorEvent>,
        ready_tx: &flume::Sender<Result<String, String>>,
    ) {
        tracing::info!("Compositor thread starting...");

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

        let egl = try_init!(
            EglContext::new(Path::new(&gpu_path)),
            "Failed to initialize EGL context"
        );
        let output = try_init!(
            DrmOutput::new(Path::new(&display_path)),
            "Failed to initialize DRM output"
        );

        let scene_renderer = SceneRenderer::new(egl, output);
        let (logical_width, logical_height) = scene_renderer.logical_size();
        tracing::info!(
            "Display configured: {}x{} logical (rotated)",
            logical_width,
            logical_height
        );

        let mut event_loop: EventLoop<'_, AppState> =
            try_init!(EventLoop::try_new(), "Failed to create event loop");
        let mut display: Display<CompositorState> =
            try_init!(Display::new(), "Failed to create Wayland display");
        let compositor_state = CompositorState::new(&display, logical_width, logical_height);
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
        if let Ok(poll_fd) = display.backend().poll_fd().try_clone_to_owned()
            && loop_handle
                .insert_source(
                    Generic::new(poll_fd, Interest::READ, Mode::Level),
                    |_, _, state| {
                        state.display.dispatch_clients(&mut state.compositor)?;
                        Ok(PostAction::Continue)
                    },
                )
                .is_err()
        {
            let err = "Failed to add display fd to event loop".to_owned();
            tracing::error!("{}", err);
            let _ = ready_tx.send(Err(err));
            return;
        }

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
            command_rx,
            action_tx,
            event_tx,
            should_exit: false,
        };

        // Add DRM device fd for vblank/page-flip events
        if let Ok(drm_fd) = app_state
            .scene_renderer
            .output()
            .drm()
            .device_fd()
            .as_fd()
            .try_clone_to_owned()
        {
            let _ = loop_handle.insert_source(
                Generic::new(drm_fd, Interest::READ, Mode::Level),
                |_, _, state| {
                    if let Ok(events) = state.scene_renderer.output().drm().receive_events() {
                        for event in events {
                            match event {
                                DrmEvent::Vblank(_) | DrmEvent::PageFlip(_) => {
                                    state.scene_renderer.output_mut().on_vblank();
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

            if let Err(e) = app_state.scene_renderer.render_scene(
                &app_state.compositor.widgets,
                &app_state.compositor.widget_buffers,
            ) {
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

        tracing::info!("Compositor thread exiting");
    }
}

struct AppState {
    display: Display<CompositorState>,
    compositor: CompositorState,
    scene_renderer: SceneRenderer,
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
            // Widget registration is informational - actual tracking happens via Wayland protocol
        }
        CompositorCommand::UnregisterWidget { instance_id } => {
            tracing::debug!("Unregistering widget {}", instance_id);
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
        let command_rx = self.command_rx.clone();
        let action_tx = self.action_tx.clone();
        let event_tx = self.event_tx.clone();

        let handle = thread::Builder::new()
            .name("egl-compositor".to_owned())
            .spawn(move || {
                Self::run_compositor_loop(
                    &gpu_path,
                    &display_path,
                    command_rx,
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
