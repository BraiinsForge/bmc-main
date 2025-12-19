// Copyright (C) 2025  Braiins Systems s.r.o.

//! BMC Compositor with EGL/OpenGL ES rendering
//!
//! This is a Wayland compositor that uses GPU-accelerated rendering via EGL.
//! It supports split GPU/display architectures common on embedded SoCs.
//!
//! Device paths:
//! - GPU: /dev/dri/renderD128 or /dev/dri/card0 (etnaviv)
//! - Display: /dev/dri/card1 (stm32-ltdc)

use anyhow::{Context, Result};
use bmc_compositor::{
    render_egl::EglRenderState,
    state::{ClientState, Compositor},
};
use smithay::reexports::{
    calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic},
    drm::control::{Device as DrmControlDevice, Event as DrmEvent},
    wayland_server::{Display, ListeningSocket},
};
use std::{os::fd::AsFd, path::Path, sync::Arc, time::Duration};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Default GPU device path (etnaviv render node)
const DEFAULT_GPU_PATH: &str = "/dev/dri/renderD128";

/// Default display device path (stm32-ltdc)
const DEFAULT_DISPLAY_PATH: &str = "/dev/dri/card1";

/// Shared state passed to event loop callbacks
struct AppState {
    display: Display<Compositor>,
    compositor: Compositor,
    render_state: EglRenderState,
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

    tracing::info!("BMC Compositor (EGL) starting...");

    // Get device paths from environment or use defaults
    let gpu_path = std::env::var("BMC_GPU_DEVICE").unwrap_or_else(|_| DEFAULT_GPU_PATH.to_owned());
    let display_path =
        std::env::var("BMC_DISPLAY_DEVICE").unwrap_or_else(|_| DEFAULT_DISPLAY_PATH.to_owned());

    tracing::info!("GPU device: {}", gpu_path);
    tracing::info!("Display device: {}", display_path);

    // Create event loop
    let mut event_loop: EventLoop<'_, AppState> =
        EventLoop::try_new().context("Failed to create event loop")?;

    // Initialize EGL render state with split GPU/display
    let render_state = EglRenderState::new(Path::new(&gpu_path), Path::new(&display_path))
        .context("Failed to initialize EGL render state")?;

    let (physical_width, physical_height) = render_state.physical_size();
    let (logical_width, logical_height) = render_state.logical_size();
    tracing::info!(
        "Display configured: {}x{} physical, {}x{} logical (rotated)",
        physical_width,
        physical_height,
        logical_width,
        logical_height
    );

    // Create Wayland display
    let mut display: Display<Compositor> =
        Display::new().context("Failed to create Wayland display")?;

    // Create compositor state with LOGICAL dimensions (what widgets see)
    // Widgets render to 1280x480 landscape, compositor rotates to 480x1280 physical
    let compositor = Compositor::new(&display, logical_width, logical_height);

    tracing::info!(
        "Compositor advertising {}x{} to widgets",
        logical_width,
        logical_height
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

    // Create shared state for event loop
    let mut app_state = AppState {
        display,
        compositor,
        render_state,
        listening_socket,
    };

    // Add display DRM device fd to event loop for vblank events
    let drm_fd = app_state
        .render_state
        .display_drm()
        .device_fd()
        .as_fd()
        .try_clone_to_owned()?;

    loop_handle
        .insert_source(
            Generic::new(drm_fd, Interest::READ, Mode::Level),
            |_, _, state| {
                // Process DRM events (vblank, page flip complete, etc.)
                match state.render_state.display_drm().receive_events() {
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
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("Failed to add DRM fd to event loop: {}", e))?;

    tracing::info!("BMC Compositor (EGL) initialized successfully");
    tracing::info!("Waiting for Wayland clients...");
    tracing::info!("Connect widgets with: WAYLAND_DISPLAY={}", socket_name);

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
        let buffer = app_state.compositor.current_buffer.as_ref();
        if let Err(e) = app_state.render_state.render_frame(buffer) {
            tracing::error!("Render error: {}", e);
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
