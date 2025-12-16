// Copyright (C) 2025  Braiins Systems s.r.o.

//! BMC Compositor - Wayland compositor for widget display
//!
//! This is a minimal Wayland compositor that:
//! 1. Takes control of the display via DRM/KMS
//! 2. Creates a Wayland socket for widget clients
//! 3. Composites widget surfaces to the framebuffer

use anyhow::{Context, Result};
use bmc_compositor::{
    drm_backend::{DISPLAY_HEIGHT, DISPLAY_WIDTH, DrmBackendState},
    render::RenderState,
    state::{ClientState, Compositor},
};
use smithay::reexports::{
    calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic},
    drm::control::{Device as DrmControlDevice, Event as DrmEvent},
    wayland_server::{Display, ListeningSocket},
};
use std::{os::fd::AsFd, sync::Arc, time::Duration};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Shared state passed to event loop callbacks
struct AppState {
    display: Display<Compositor>,
    compositor: Compositor,
    drm_state: DrmBackendState,
    render_state: RenderState,
    listening_socket: ListeningSocket,
}

#[expect(
    clippy::too_many_lines,
    reason = "main function orchestrates initialization"
)]
fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("BMC Compositor starting...");

    // Create event loop with our app state type
    let mut event_loop: EventLoop<'_, AppState> =
        EventLoop::try_new().context("Failed to create event loop")?;

    // Initialize DRM backend
    let mut drm_state = DrmBackendState::new_without_event_loop();

    // Find primary GPU
    let primary_gpu = drm_state
        .find_primary_gpu()
        .context("Failed to find primary GPU")?;

    tracing::info!("Using primary GPU: {:?}", primary_gpu);

    // Get the device path and initialize
    let device_path = primary_gpu
        .dev_path()
        .context("Failed to get device path")?;
    drm_state
        .init_device(&device_path)
        .context("Failed to initialize DRM device")?;

    // Configure display mode
    let display_info = drm_state
        .configure_display()
        .context("Failed to configure display")?;

    tracing::info!(
        "Display configured: {}x{} @ {}Hz",
        display_info.width,
        display_info.height,
        display_info.refresh_mhz
    );

    // Create render state
    let render_state = RenderState::new(drm_state.device_mut().context("Device not initialized")?)
        .context("Failed to create render state")?;

    tracing::info!("Render state initialized");

    // Create Wayland display
    let mut display: Display<Compositor> =
        Display::new().context("Failed to create Wayland display")?;

    // Create compositor state
    let compositor = Compositor::new(&display, DISPLAY_WIDTH, DISPLAY_HEIGHT);

    tracing::info!(
        "Compositor state created for {}x{} display",
        DISPLAY_WIDTH,
        DISPLAY_HEIGHT
    );

    // Create Wayland socket
    let listening_socket =
        ListeningSocket::bind_auto("wayland", 0..33).context("Failed to create Wayland socket")?;

    let socket_name = listening_socket
        .socket_name()
        .context("Failed to get socket name")?
        .to_string_lossy()
        .to_string();

    tracing::info!("Wayland socket created: {}", socket_name);

    // Add Wayland display fd to event loop for dispatching events
    let loop_handle = event_loop.handle();
    loop_handle
        .insert_source(
            Generic::new(
                display.backend().poll_fd().try_clone_to_owned()?,
                Interest::READ,
                Mode::Level,
            ),
            |_, _, state| {
                state.display.dispatch_clients(&mut state.compositor)?;
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("Failed to add display fd to event loop: {}", e))?;

    // Add DRM device fd to event loop for vblank events
    if let Some(device) = drm_state.device() {
        let drm_fd = device.drm.device_fd().as_fd().try_clone_to_owned()?;
        loop_handle
            .insert_source(
                Generic::new(drm_fd, Interest::READ, Mode::Level),
                |_, _, state| {
                    if let Some(device) = state.drm_state.device() {
                        // Process DRM events (vblank, page flip complete, etc.)
                        match device.drm.receive_events() {
                            Ok(events) => {
                                for event in events {
                                    match event {
                                        DrmEvent::Vblank(_) | DrmEvent::PageFlip(_) => {
                                            state.render_state.on_vblank();
                                        }
                                        DrmEvent::Unknown(_) => {}
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("DRM event error: {}", e);
                            }
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|e| anyhow::anyhow!("Failed to add DRM fd to event loop: {}", e))?;
    }

    tracing::info!("BMC Compositor initialized successfully");
    tracing::info!("Waiting for Wayland clients...");
    tracing::info!("Connect widgets with: WAYLAND_DISPLAY={}", socket_name);

    // Create shared state for event loop
    let mut app_state = AppState {
        display,
        compositor,
        drm_state,
        render_state,
        listening_socket,
    };

    // Run event loop
    loop {
        // Accept new client connections
        if let Some(client_stream) = app_state.listening_socket.accept()? {
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

        // Render a frame with current buffer
        if let Some(device) = app_state.drm_state.device() {
            let buffer = app_state.compositor.current_buffer.as_ref();
            if let Err(e) = app_state.render_state.render_frame(&device.drm, buffer) {
                tracing::error!("Render error: {}", e);
            }
        }

        // Send frame callbacks to clients so they can render next frame
        // Use monotonic time in milliseconds (wrapping is fine for frame callbacks)
        #[expect(
            clippy::cast_possible_truncation,
            reason = "wrapping is acceptable for frame time"
        )]
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u32)
            .unwrap_or(0);
        app_state.compositor.send_frame_callbacks(time);

        // Flush client buffers
        app_state.display.flush_clients()?;

        // Run event loop iteration (16ms ~ 60fps)
        event_loop
            .dispatch(Some(Duration::from_millis(16)), &mut app_state)
            .context("Event loop error")?;
    }
}
