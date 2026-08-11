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

// Main application: eframe::App implementation for the console window.

use crate::device_frame::{DEVICE_ASPECT, DeviceFrame};
use crate::fb_texture::FbTexture;
use crate::input::InputHandler;
use crate::log_panel::LogPanel;
use bmc_virt_ipc::protocol::DEFAULT_PORT;
use bmc_virt_ipc::{FeatureState, GuestMessage, HostEndpoint, LED_COUNT, LedState, NotifyLevel};
use eframe::glow;
use std::collections::HashMap;
use std::sync::Arc;

/// LED effect preset names (must match relay's commands::PRESETS order).
const EFFECT_NAMES: &[&str] = &[
    "Knight Rider",
    "Chase",
    "Breathe",
    "Snake",
    "Solid White",
    "Scan",
];
const CONTROL_PANEL_WIDTH: f32 = 240.0;
const CONTROL_PANEL_MARGIN: f32 = 8.0;
const CONTROL_PANEL_GAP: f32 = 16.0;

/// Relay connection state machine.
///
/// Transitions:
///   Disconnected → Linking       (TCP connect succeeds)
///   Linking      → Live          (first frame received)
///   Linking      → Disconnected  (socket dies before first frame)
///   Live         → Disconnected  (connection lost)
///
/// Only `Disconnected` counts toward the exit timeout. A live relay may stay
/// in `Linking` while waiting for capture, but that still counts as connected.
enum RelayState {
    /// No TCP connection. Actively retrying.
    Disconnected {
        since: std::time::Instant,
        last_attempt: std::time::Instant,
    },
    /// TCP connected, waiting for first frame.
    Linking {
        disconnect_since: std::time::Instant,
        ipc: HostEndpoint,
    },
    /// Connected and receiving frames.
    Live { ipc: HostEndpoint },
    /// Connected, but framebuffer capture is unavailable. Keep the app open
    /// and show a placeholder/status instead of timing out.
    Degraded { ipc: HostEndpoint },
}

impl RelayState {
    fn connected(&self) -> bool {
        !matches!(self, Self::Disconnected { .. })
    }

    fn ipc(&self) -> Option<&HostEndpoint> {
        match self {
            Self::Disconnected { .. } => None,
            Self::Linking { ipc, .. } | Self::Live { ipc } | Self::Degraded { ipc } => Some(ipc),
        }
    }

    fn ipc_mut(&mut self) -> Option<&mut HostEndpoint> {
        match self {
            Self::Disconnected { .. } => None,
            Self::Linking { ipc, .. } | Self::Live { ipc } | Self::Degraded { ipc } => Some(ipc),
        }
    }
}

fn should_preserve_degraded_ui(
    capture_status: FeatureState,
    controls_status: FeatureState,
    grpc_error: Option<&str>,
) -> bool {
    capture_status == FeatureState::Unavailable
        || controls_status != FeatureState::Ready
        || grpc_error.is_some()
}

pub struct ConsoleApp {
    gl: Arc<glow::Context>,
    relay: RelayState,
    fb_texture: Option<FbTexture>,
    device_frame: Option<DeviceFrame>,
    input: InputHandler,
    last_frame_seq: u64,
    /// Display backlight brightness from latest frame (0=off, u8::MAX=full).
    backlight: u8,
    led_cache: [LedState; LED_COUNT],
    last_led_seq: u64,
    selected_effect: Option<usize>,
    /// App's configured volume (from gRPC, read-only reference).
    volume_app: u8,
    /// Console override (None = app's value, Some = slider override).
    volume_override: Option<u8>,
    /// Local brightness override (None = use relay's value, Some = slider override).
    brightness_override: Option<u8>,
    log_panel: LogPanel,
    toasts: egui_notify::Toasts,
    /// gRPC/auth error from the relay — `Some` disables all gRPC-dependent controls.
    /// Driven by explicit controls status from the relay.
    grpc_error: Option<String>,
    capture_status: FeatureState,
    controls_status: FeatureState,
    /// Deduplicated warning/error notifications: message → (count, level).
    /// Toasts are rebuilt from this map when a duplicate arrives.
    notify_counts: HashMap<String, (usize, NotifyLevel)>,
    show_logs: bool,
    status_message: Option<String>,
    /// When the last ping was sent (for connection liveness detection).
    last_ping: std::time::Instant,
    icon_volume: crate::icons::SvgIcon,
    icon_brightness: crate::icons::SvgIcon,
    icon_led: crate::icons::SvgIcon,
    icon_terminal: crate::icons::SvgIcon,
    icon_reset: crate::icons::SvgIcon,
    icon_power: crate::icons::SvgIcon,
}

impl ConsoleApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(target_os = "macos")]
        set_macos_srgb_colorspace(cc);

        let gl = cc
            .gl
            .as_ref()
            .unwrap_or_else(|| panic!("glow backend required"))
            .clone();

        install_noto_sans(&cc.egui_ctx);

        // Compact window title bars: no rounding, thinner
        cc.egui_ctx.global_style_mut(|style| {
            style.spacing.window_margin = egui::Margin::same(6);
            style.visuals.window_corner_radius = egui::CornerRadius::ZERO;
            style.visuals.window_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_gray(50));
        });

        // Try to connect to the relay via TCP IPC
        let addr = resolve_addr();
        let now = std::time::Instant::now();
        let (relay, status_message) = match HostEndpoint::connect(&addr) {
            Ok(ep) => {
                tracing::info!("connected to relay at {addr}");
                (
                    RelayState::Linking {
                        disconnect_since: now,
                        ipc: ep,
                    },
                    "Connected, waiting for frames... (if stuck, another console may be open)",
                )
            }
            Err(e) => {
                tracing::warn!("could not connect to relay at {addr}: {e}");
                (
                    RelayState::Disconnected {
                        since: now,
                        last_attempt: now,
                    },
                    "Waiting for relay...",
                )
            }
        };

        Self {
            gl,
            relay,
            fb_texture: None,
            device_frame: None,
            input: InputHandler::new(),
            last_frame_seq: 0,
            backlight: u8::MAX,
            led_cache: [LedState::default(); LED_COUNT],
            last_led_seq: 0,
            selected_effect: None,
            volume_app: 50,
            volume_override: None,
            brightness_override: Some(255),
            log_panel: LogPanel::new(),
            toasts: egui_notify::Toasts::new()
                .with_anchor(egui_notify::Anchor::BottomLeft)
                .with_margin(egui::vec2(10.0, 10.0)),
            grpc_error: None,
            capture_status: FeatureState::Waiting,
            controls_status: FeatureState::Waiting,
            notify_counts: HashMap::new(),
            show_logs: false,
            status_message: Some(status_message.into()),
            last_ping: std::time::Instant::now(),
            icon_volume: crate::icons::svg_icon!("../assets/icons/volume-up.svg"),
            icon_brightness: crate::icons::svg_icon!("../assets/icons/light.svg"),
            icon_led: crate::icons::svg_icon!("../assets/icons/light-filled.svg"),
            icon_terminal: crate::icons::svg_icon!("../assets/icons/terminal.svg"),
            icon_reset: crate::icons::svg_icon!("../assets/icons/reset.svg"),
            icon_power: crate::icons::svg_icon!("../assets/icons/power.svg"),
        }
    }

    /// Try to reconnect to relay if not connected.
    /// Returns `true` if the app should exit (reconnect timeout exceeded).
    fn try_reconnect(&mut self) -> bool {
        if !matches!(self.relay, RelayState::Disconnected { .. }) {
            return false;
        }

        let disconnect_timeout = std::time::Duration::from_secs(30);
        let since = match &self.relay {
            RelayState::Disconnected { since, .. } => *since,
            RelayState::Linking { .. } | RelayState::Live { .. } | RelayState::Degraded { .. } => {
                unreachable!()
            }
        };

        let elapsed = since.elapsed();
        if elapsed >= disconnect_timeout {
            tracing::info!(
                "no connection for {}s, exiting",
                disconnect_timeout.as_secs()
            );
            return true;
        }

        let remaining = disconnect_timeout.saturating_sub(elapsed).as_secs();
        let attempts = elapsed.as_secs();
        if !should_preserve_degraded_ui(
            self.capture_status,
            self.controls_status,
            self.grpc_error.as_deref(),
        ) {
            self.status_message = Some(format!(
                "Relay disconnected \u{2014} reconnecting\nattempt {attempts}, closing in {remaining}s"
            ));
        }

        // Only attempt TCP connect when Disconnected (Linking already has a socket)
        if let RelayState::Disconnected { last_attempt, .. } = &mut self.relay
            && last_attempt.elapsed() >= std::time::Duration::from_secs(1)
        {
            *last_attempt = std::time::Instant::now();
            let addr = resolve_addr();
            if let Ok(ep) = HostEndpoint::connect(&addr) {
                tracing::info!("connected to relay at {addr}");
                self.relay = RelayState::Linking {
                    disconnect_since: since,
                    ipc: ep,
                };
            }
        }
        false
    }

    fn handle_capture_status(&mut self, state: FeatureState, reason: Option<String>) {
        tracing::info!(
            "capture status: {state:?} reason={reason:?} fb={} relay={}",
            if self.fb_texture.is_some() {
                "some"
            } else {
                "none"
            },
            match self.relay {
                RelayState::Disconnected { .. } => "Disconnected",
                RelayState::Linking { .. } => "Linking",
                RelayState::Live { .. } => "Live",
                RelayState::Degraded { .. } => "Degraded",
            },
        );
        self.capture_status = state;
        match state {
            FeatureState::Waiting => {
                self.status_message =
                    Some(reason.unwrap_or_else(|| "Connected, waiting for frames...".to_owned()));
            }
            FeatureState::Ready => {
                if self.fb_texture.is_none() {
                    self.status_message = Some("Connected, waiting for frames...".to_owned());
                }
            }
            FeatureState::Unavailable => {
                let msg = reason.unwrap_or_else(|| {
                    "Guest is connected, but display capture is unavailable.".to_owned()
                });
                match &self.relay {
                    // Already Degraded — just refresh the reason text. The
                    // `mem::replace` dance below would drop the IPC in this
                    // state, so we must not go through it.
                    RelayState::Degraded { .. } => {
                        self.status_message = Some(msg);
                    }
                    RelayState::Linking { .. } | RelayState::Live { .. } => {
                        if let RelayState::Linking { ipc, .. } | RelayState::Live { ipc } =
                            std::mem::replace(
                                &mut self.relay,
                                RelayState::Disconnected {
                                    since: std::time::Instant::now(),
                                    last_attempt: std::time::Instant::now(),
                                },
                            )
                        {
                            self.status_message = Some(msg);
                            self.relay = RelayState::Degraded { ipc };
                        }
                    }
                    RelayState::Disconnected { .. } => {}
                }
            }
        }
    }

    /// Returns true when persistent toasts need rebuilding.
    fn handle_controls_status(&mut self, state: FeatureState, reason: Option<String>) -> bool {
        self.controls_status = state;
        match state {
            FeatureState::Ready => {
                if self.grpc_error.is_some() {
                    self.grpc_error = None;
                    self.notify_counts.clear();
                    return true;
                }
                false
            }
            FeatureState::Waiting | FeatureState::Unavailable => {
                self.grpc_error = reason;
                false
            }
        }
    }

    /// Returns true when persistent toasts need rebuilding.
    fn handle_notify(&mut self, level: NotifyLevel, message: String) -> bool {
        match level {
            NotifyLevel::Info => {
                self.toasts.info(message);
                false
            }
            NotifyLevel::Warning | NotifyLevel::Error => {
                self.notify_counts
                    .entry(message)
                    .and_modify(|(count, _)| *count += 1)
                    .or_insert((1, level));
                true
            }
        }
    }

    /// Drain all pending IPC messages and update local state.
    fn poll_messages(&mut self, frame: &mut eframe::Frame) {
        if self.relay.ipc().is_none() {
            return;
        }

        // Track whether persistent toasts need rebuilding after the drain loop.
        let mut toasts_dirty = false;
        let mut got_frame = false;

        // Drain up to 100 messages per frame to avoid stalling the render loop
        for _ in 0..100 {
            let Some(msg) = self.relay.ipc().and_then(HostEndpoint::try_recv) else {
                break;
            };
            match msg {
                GuestMessage::Frame { header, data } => {
                    self.apply_frame_update(frame, header, data, &mut got_frame);
                }
                GuestMessage::Leds(update) => {
                    self.mark_relay_alive_without_frame();
                    if update.seq != self.last_led_seq {
                        self.last_led_seq = update.seq;
                        self.led_cache = update.leds;
                    }
                }
                GuestMessage::Log { source, line } => {
                    self.mark_relay_alive_without_frame();
                    self.log_panel.push(source, line);
                }
                GuestMessage::ActiveEffect(idx) => {
                    self.mark_relay_alive_without_frame();
                    self.selected_effect = if idx == 0xFF {
                        None
                    } else {
                        Some(idx as usize)
                    };
                }
                GuestMessage::CaptureStatus { state, reason } => {
                    self.mark_relay_alive_without_frame();
                    self.handle_capture_status(state, reason);
                }
                GuestMessage::VolumeLevel { app, override_vol } => {
                    self.mark_relay_alive_without_frame();
                    self.volume_app = app;
                    self.volume_override = override_vol;
                }
                GuestMessage::ControlsStatus { state, reason } => {
                    self.mark_relay_alive_without_frame();
                    if self.handle_controls_status(state, reason) {
                        toasts_dirty = true;
                    }
                }
                GuestMessage::Pong => {
                    self.mark_relay_alive_without_frame();
                } // keepalive reply — received is enough
                GuestMessage::Notify { level, message } => {
                    self.mark_relay_alive_without_frame();
                    tracing::info!("relay notify [{level:?}]: {message}");
                    if self.handle_notify(level, message) {
                        toasts_dirty = true;
                    }
                }
            }
        }

        if let Some((header, data)) = self.relay.ipc().and_then(HostEndpoint::take_latest_frame) {
            self.apply_frame_update(frame, header, data, &mut got_frame);
        }

        if let Some(update) = self.relay.ipc().and_then(HostEndpoint::take_latest_led) {
            self.mark_relay_alive_without_frame();
            if update.seq != self.last_led_seq {
                self.last_led_seq = update.seq;
                self.led_cache = update.leds;
                // Debug: log the first lit LED's color so we can verify
                // animated effects are actually flowing through.
                if let Some((idx, led)) = self
                    .led_cache
                    .iter()
                    .enumerate()
                    .find(|(_, l)| l.brightness > 0 && (l.r > 0 || l.g > 0 || l.b > 0))
                {
                    tracing::debug!(
                        "led update seq={} first_on=LED{}({},{},{}) bright={}",
                        update.seq,
                        idx,
                        led.r,
                        led.g,
                        led.b,
                        led.brightness,
                    );
                } else {
                    tracing::debug!("led update seq={} all_off", update.seq);
                }
            }
        }

        // Rebuild persistent toasts outside the ipc borrow scope
        if toasts_dirty {
            self.rebuild_persistent_toasts();
        }

        self.update_relay_state(got_frame);
    }

    fn apply_frame_update(
        &mut self,
        frame: &mut eframe::Frame,
        header: bmc_virt_ipc::FrameHeader,
        data: impl AsRef<[u8]>,
        got_frame: &mut bool,
    ) {
        let bpp = header.bpp;
        let stride = header.stride;
        let seq = header.seq;
        let format = header.format;

        // Create texture lazily once we know the pixel format.
        if self.fb_texture.is_none() {
            self.fb_texture = Some(FbTexture::new(&self.gl, frame, bpp, format));
            self.status_message = None;
            tracing::info!("created FB texture (bpp={bpp:?} format={format:?})");
        }

        if let Some(ref mut fb_tex) = self.fb_texture {
            fb_tex.update_if_changed(&self.gl, seq, data.as_ref(), stride);
        }

        self.last_frame_seq = seq;
        self.backlight = header.brightness;
        self.capture_status = FeatureState::Ready;
        *got_frame = true;
    }

    /// Update relay state machine after message processing.
    fn update_relay_state(&mut self, got_frame: bool) {
        // First frame means the relay is actually delivering image data.
        // Guard with `matches!` so subsequent frames (when already Live) don't
        // enter the mem::replace dance — the pattern wouldn't match Live and
        // the OLD value (with the IPC) would be silently dropped, killing the
        // reader thread via consumer-dropped.
        if got_frame
            && matches!(
                self.relay,
                RelayState::Linking { .. } | RelayState::Degraded { .. }
            )
            && let RelayState::Linking { ipc, .. } | RelayState::Degraded { ipc } =
                std::mem::replace(
                    &mut self.relay,
                    RelayState::Disconnected {
                        since: std::time::Instant::now(),
                        last_attempt: std::time::Instant::now(),
                    },
                )
        {
            self.relay = RelayState::Live { ipc };
            self.status_message = None;
        }

        // Send periodic pings so the reader thread's read timeout can
        // detect a dead guest even when no frames are flowing.
        if self.last_ping.elapsed() >= std::time::Duration::from_millis(500) {
            self.last_ping = std::time::Instant::now();
            if let Some(ipc) = self.relay.ipc_mut()
                && let Err(e) = ipc.send_ping()
            {
                tracing::warn!("ping failed: {e}");
            }
        }

        // Detect disconnect: Live/Linking → Disconnected
        if let Some(ipc) = self.relay.ipc()
            && ipc.is_disconnected()
        {
            tracing::warn!("relay disconnected, will attempt reconnect");
            self.toasts.warning("Relay disconnected, reconnecting...");
            let since = match &self.relay {
                RelayState::Linking {
                    disconnect_since, ..
                } => *disconnect_since,
                RelayState::Disconnected { .. }
                | RelayState::Live { .. }
                | RelayState::Degraded { .. } => std::time::Instant::now(),
            };
            self.relay = RelayState::Disconnected {
                since,
                last_attempt: std::time::Instant::now(),
            };
            self.fb_texture = None;
            self.last_frame_seq = 0;
            self.led_cache = [LedState::default(); LED_COUNT];
            self.last_led_seq = 0;
            self.capture_status = FeatureState::Waiting;
            self.status_message = Some("Relay disconnected, reconnecting...".to_owned());
        }
    }

    /// Compute the device frame rect that fits in the available space
    /// while maintaining the SVG aspect ratio, centered with some padding.
    fn compute_device_rect(available: egui::Vec2) -> egui::Rect {
        let padding_left = 30.0;
        let padding_right = CONTROL_PANEL_WIDTH + (CONTROL_PANEL_MARGIN * 2.0) + CONTROL_PANEL_GAP;
        let padding_y = 30.0;
        let usable = egui::vec2(
            available.x - padding_left - padding_right,
            available.y - padding_y * 2.0,
        );
        let avail_aspect = usable.x / usable.y;
        let (w, h) = if avail_aspect > DEVICE_ASPECT {
            (usable.y * DEVICE_ASPECT, usable.y)
        } else {
            (usable.x, usable.x / DEVICE_ASPECT)
        };

        // Floor to whole pixels to avoid sub-pixel seams at edges
        let w = w.floor();
        let h = h.floor();
        // Center within the usable area (shifted left to account for asymmetric padding)
        let offset = egui::vec2(
            (padding_left + (usable.x - w) / 2.0).floor(),
            ((available.y - h) / 2.0).floor(),
        );
        egui::Rect::from_min_size(egui::pos2(offset.x, offset.y), egui::vec2(w, h))
    }
}

impl eframe::App for ConsoleApp {
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Try to reconnect if needed — exit if timeout exceeded
        if self.try_reconnect() {
            root_ui.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Drain incoming IPC messages
        self.poll_messages(frame);

        // Initialize device frame texture (once, independent of IPC)
        if self.device_frame.is_none() {
            self.device_frame = Some(DeviceFrame::new(&self.gl, frame, 2000));
        }

        // Draw everything
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(34)))
            .show_inside(root_ui, |ui| {
                let available = ui.available_size();
                let panel_origin = ui.min_rect().left_top().to_vec2();

                // Checkerboard background (upper portion)
                draw_checkerboard(ui.painter(), ui.max_rect());

                let device_rect = Self::compute_device_rect(available).translate(panel_origin);
                let screen_rect = DeviceFrame::screen_rect(device_rect);

                // Table surface below the device
                draw_table_surface(ui.painter(), device_rect, ui.max_rect());

                // LED reflections on the table (rendered before device frame so glow is under the legs)
                crate::led_glow::draw_led_glow(ui.painter(), device_rect, &self.led_cache);

                // Device frame (bevel, inset, screen fill, logo)
                if let Some(ref dev_frame) = self.device_frame {
                    dev_frame.paint(ui.painter(), device_rect);
                }

                // Framebuffer on top of the screen area
                if let Some(ref fb_tex) = self.fb_texture {
                    fb_tex.paint_rotated(ui.painter(), screen_rect);
                }

                // Backlight dimming overlay (0 = fully black, 255 = no overlay)
                let effective_brightness = self.brightness_override.unwrap_or(self.backlight);
                if effective_brightness < u8::MAX {
                    let alpha = u8::MAX - effective_brightness;
                    ui.painter().rect_filled(
                        screen_rect,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, alpha),
                    );
                }

                // Stale-capture overlay: when the last texture is still up but
                // the guest reports capture is unavailable, black it out almost
                // entirely so the frozen image reads as "connection lost".
                let capture_stale =
                    self.fb_texture.is_some() && matches!(self.relay, RelayState::Degraded { .. });
                if capture_stale {
                    ui.painter().rect_filled(
                        screen_rect,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 230),
                    );
                }

                // Layer 4: Touch input (on the screen area)
                if let Some(ipc) = self.relay.ipc_mut() {
                    self.input.process(ui, screen_rect, ipc);
                }

                // Status overlay: shown when there's no live framebuffer, or
                // the capture has gone stale over an existing one.
                if (self.fb_texture.is_none() || capture_stale)
                    && let Some(ref msg) = self.status_message
                {
                    let show_warning = self.capture_status == FeatureState::Unavailable
                        || matches!(self.relay, RelayState::Degraded { .. });
                    let show_spinner =
                        !show_warning && !matches!(self.relay, RelayState::Live { .. });
                    draw_status_overlay(ui, screen_rect, msg, show_warning, show_spinner);
                }
            });

        // Display logs in a separate OS window when enabled
        if self.show_logs {
            self.log_panel.show(root_ui.ctx());
            if self.log_panel.take_close_requested() {
                self.show_logs = false;
            }
        }

        // Show controls whenever the relay is connected, even if framebuffer
        // capture is unavailable. Some actions remain usable in degraded mode.
        if self.relay.connected()
            || should_preserve_degraded_ui(
                self.capture_status,
                self.controls_status,
                self.grpc_error.as_deref(),
            )
        {
            self.show_controls(root_ui.ctx());
        }

        // Toast notifications (rendered last so they overlay everything)
        self.toasts.show(root_ui.ctx());

        // Request repaint at ~60 FPS
        root_ui
            .ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
}

impl ConsoleApp {
    fn show_controls(&mut self, ctx: &egui::Context) {
        let resp = egui::Window::new("Controls")
            .anchor(
                egui::Align2::RIGHT_TOP,
                egui::vec2(-CONTROL_PANEL_MARGIN, CONTROL_PANEL_MARGIN),
            )
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .min_width(CONTROL_PANEL_WIDTH)
            .max_width(CONTROL_PANEL_WIDTH)
            .show(ctx, |ui| {
                compact_header(ui, "Controls");

                if self.controls_status == FeatureState::Ready {
                    self.show_controls_ready(ui);
                } else {
                    let err = self
                        .grpc_error
                        .as_deref()
                        .unwrap_or(match self.controls_status {
                            FeatureState::Waiting => "Waiting for BMC web service...",
                            FeatureState::Unavailable => "BMC web service unavailable.",
                            FeatureState::Ready => "",
                        });
                    grpc_error_banner(ui, err);
                }

                // ── Brightness ──────────────────────────────────────
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "0..=255 mapped to 0..=100 always fits in u8"
                )]
                let brightness_pct_value =
                    (f32::from(self.brightness_override.unwrap_or(self.backlight))
                        * (100.0 / 255.0))
                        .round() as u8;
                let brightness_pct = format!("{brightness_pct_value}%");
                section_header(
                    ui,
                    "Brightness",
                    Some(&mut self.icon_brightness),
                    Some(&brightness_pct),
                );
                self.show_brightness_slider(ui);

                // ── Actions ─────────────────────────────────────────
                section_header(ui, "Actions", None, None);
                let label = if self.show_logs {
                    "Hide Logs"
                } else {
                    "Show Logs"
                };
                if click_button(ui, label, Some(&mut self.icon_terminal)) {
                    self.show_logs = !self.show_logs;
                }
                if let Some(pressed) = gpio_button(ui, "Reset Button", Some(&mut self.icon_reset)) {
                    self.send_gpio_button(pressed);
                }
                if hold_button(ui, "Hold to Power Off", 2.0, Some(&mut self.icon_power)) {
                    self.send_command("poweroff");
                }
            });

        let _ = resp;
    }

    fn show_controls_ready(&mut self, ui: &mut egui::Ui) {
        // ── LED effects ─────────────────────────────────
        section_header(ui, "LED Effect", Some(&mut self.icon_led), None);
        let selected_label = self
            .selected_effect
            .and_then(|i| EFFECT_NAMES.get(i).copied())
            .unwrap_or("Off");

        // Don't mutate selected_effect here — only ActiveEffect
        // messages from the relay confirm the actual state.
        egui::ComboBox::from_id_salt("led_effect")
            .selected_text(selected_label)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(self.selected_effect.is_none(), "Off")
                    .clicked()
                {
                    self.send_button(bmc_virt_ipc::buttons::LED_EFFECT_CLEAR, 0);
                }
                #[expect(clippy::cast_possible_truncation, reason = "preset index fits in u8")]
                for (idx, name) in EFFECT_NAMES.iter().enumerate() {
                    if ui
                        .selectable_label(self.selected_effect == Some(idx), *name)
                        .clicked()
                    {
                        self.send_button(bmc_virt_ipc::buttons::LED_EFFECT_SET, idx as u8);
                    }
                }
            });

        // ── Volume ──────────────────────────────────────
        let volume_pct = format!("{}%", self.volume_override.unwrap_or(self.volume_app));
        section_header(ui, "Volume", Some(&mut self.icon_volume), Some(&volume_pct));
        self.show_volume_slider(ui);
    }

    fn show_volume_slider(&mut self, ui: &mut egui::Ui) {
        let slider_vol = self.volume_override.unwrap_or(self.volume_app);
        let mut vol = f32::from(slider_vol);
        let prev_slider_width = ui.spacing().slider_width;
        ui.spacing_mut().slider_width = ui.available_width();
        let slider_response = ui.add(
            egui::Slider::new(&mut vol, 0.0..=100.0)
                .show_value(false)
                .step_by(5.0)
                .trailing_fill(true),
        );
        ui.spacing_mut().slider_width = prev_slider_width;

        if self.volume_override.is_some() {
            draw_app_marker(ui, &slider_response, f32::from(self.volume_app) / 100.0);
        }

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "volume is clamped 0..=100"
        )]
        if slider_response.changed() {
            let new_vol = vol as u8;
            self.volume_override = Some(new_vol);
            self.send_button(bmc_virt_ipc::buttons::VOLUME_SET, new_vol);
        }
        if self.volume_override.is_some()
            && click_button(ui, "Reset to app volume", Some(&mut self.icon_reset))
        {
            self.volume_override = None;
            self.send_button(bmc_virt_ipc::buttons::VOLUME_RESET, 0);
        }
    }

    fn show_brightness_slider(&mut self, ui: &mut egui::Ui) {
        let slider_br = self.brightness_override.unwrap_or(self.backlight);
        let mut br = f32::from(slider_br);
        let prev_slider_width = ui.spacing().slider_width;
        ui.spacing_mut().slider_width = ui.available_width();
        let slider_response = ui.add(
            egui::Slider::new(&mut br, 0.0..=255.0)
                .show_value(false)
                .step_by(5.0)
                .trailing_fill(true),
        );
        ui.spacing_mut().slider_width = prev_slider_width;

        if self.brightness_override.is_some() {
            draw_app_marker(ui, &slider_response, f32::from(self.backlight) / 255.0);
        }

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "brightness is clamped 0..=255"
        )]
        if slider_response.changed() {
            self.brightness_override = Some(br as u8);
        }
        if self.brightness_override.is_some()
            && click_button(ui, "Reset to app brightness", Some(&mut self.icon_reset))
        {
            self.brightness_override = None;
        }
    }

    fn send_button(&mut self, button: u8, data: u8) {
        if let Some(ipc) = self.relay.ipc_mut() {
            let event = bmc_virt_ipc::InputEvent::ButtonPress { button, data };
            if let Err(e) = ipc.send_input(event) {
                tracing::warn!("failed to send input: {e}");
            }
        }
    }

    /// Send a shell command to be executed on the guest.
    fn send_command(&mut self, cmd: &str) {
        if let Some(ipc) = self.relay.ipc_mut()
            && let Err(e) = ipc.send_command(cmd)
        {
            tracing::warn!("failed to send command: {e}");
        }
    }

    /// Send a GPIO reset button press/release to the guest.
    fn send_gpio_button(&mut self, pressed: bool) {
        if let Some(ipc) = self.relay.ipc_mut()
            && let Err(e) = ipc.send_gpio_button(pressed)
        {
            tracing::warn!("failed to send GPIO button: {e}");
        }
    }

    /// Dismiss all persistent toasts and rebuild from `notify_counts`.
    fn rebuild_persistent_toasts(&mut self) {
        self.toasts.dismiss_all_toasts();
    }

    fn mark_relay_alive_without_frame(&mut self) {
        if self.fb_texture.is_some() {
            return;
        }
        // Only promote Linking → Degraded. Without this guard, a second call
        // would mem::replace a Degraded out, fail the if-let, and DROP the
        // IPC (closing the channel and killing the reader thread).
        if !matches!(self.relay, RelayState::Linking { .. }) {
            return;
        }
        if let RelayState::Linking { ipc, .. } = std::mem::replace(
            &mut self.relay,
            RelayState::Disconnected {
                since: std::time::Instant::now(),
                last_attempt: std::time::Instant::now(),
            },
        ) {
            if self.capture_status == FeatureState::Unavailable {
                self.status_message =
                    Some("Guest is connected, but display capture is unavailable.".to_owned());
            } else if self.status_message.is_none()
                || self
                    .status_message
                    .as_deref()
                    .is_some_and(|msg| msg.starts_with("Relay disconnected"))
            {
                self.status_message = Some("Connected, waiting for frames...".to_owned());
            }
            self.relay = RelayState::Degraded { ipc };
        }
    }
}

/// Draw a checkerboard pattern as the background.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "checkerboard grid math on small positive values"
)]
fn draw_checkerboard(painter: &egui::Painter, rect: egui::Rect) {
    let size = 16.0;
    let color_a = egui::Color32::from_gray(24);
    let color_b = egui::Color32::from_gray(32);

    let cols = (rect.width() / size).ceil() as usize;
    let rows = (rect.height() / size).ceil() as usize;

    for row in 0..rows {
        for col in 0..cols {
            let color = if (row + col) % 2 == 0 {
                color_a
            } else {
                color_b
            };
            let pos = rect.min + egui::vec2(col as f32 * size, row as f32 * size);
            let cell_rect = egui::Rect::from_min_size(pos, egui::vec2(size, size));
            painter.rect_filled(cell_rect, 0.0, color);
        }
    }
}

/// Left-aligned compact header for title-bar-less windows.
fn compact_header(ui: &mut egui::Ui, title: &str) {
    let height = 16.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    ui.painter().text(
        egui::pos2(rect.min.x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(11.0),
        egui::Color32::from_gray(160),
    );
    // Separator line below
    let y = rect.max.y + 1.0;
    ui.painter().line_segment(
        [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(50)),
    );
    ui.add_space(4.0);
}

/// Error banner shown at the top of the controls panel when gRPC is unavailable.
fn grpc_error_banner(ui: &mut egui::Ui, err: &str) {
    egui::Frame::NONE
        .fill(egui::Color32::from_rgba_unmultiplied(180, 80, 20, 40))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(err)
                    .color(egui::Color32::from_rgb(255, 170, 80))
                    .strong(),
            );
            ui.label(
                egui::RichText::new("LED and volume controls need the BMC web service.")
                    .color(egui::Color32::from_gray(140)),
            );
        });
    ui.add_space(4.0);
}

/// Section header with a separator line and optional icon/value — used to group controls.
fn section_header(
    ui: &mut egui::Ui,
    title: &str,
    icon: Option<&mut crate::icons::SvgIcon>,
    trailing: Option<&str>,
) {
    ui.add_space(2.0);
    ui.separator();
    let color = egui::Color32::from_gray(140);
    let icon_size = 11.0;
    ui.horizontal(|ui| {
        if let Some(ico) = icon {
            ui.add(ico.image(ui.ctx(), icon_size, color));
        }
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::hover());
        ui.painter().text(
            egui::pos2(rect.min.x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(11.0),
            color,
        );
        if let Some(text) = trailing {
            ui.painter().text(
                egui::pos2(rect.max.x, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                text,
                egui::FontId::proportional(11.0),
                color,
            );
        }
    });
}

/// Button icon size in pixels.
const BUTTON_ICON_PX: u32 = 14;

/// Resolve an optional `SvgIcon` into a texture handle at the button icon size.
fn resolve_button_icon(
    ui: &egui::Ui,
    icon: Option<&mut crate::icons::SvgIcon>,
) -> Option<egui::TextureHandle> {
    icon.map(|ico| ico.texture(ui.ctx(), BUTTON_ICON_PX))
}

/// Simple click button with left-aligned icon + label, matching the custom button style.
fn click_button(ui: &mut egui::Ui, label: &str, icon: Option<&mut crate::icons::SvgIcon>) -> bool {
    let tex = resolve_button_icon(ui, icon);
    let btn_size = egui::vec2(ui.available_width(), 28.0);
    let (rect, response) = ui.allocate_exact_size(btn_size, egui::Sense::click());

    let bg = if response.hovered() {
        egui::Color32::from_gray(60)
    } else {
        egui::Color32::from_gray(45)
    };
    ui.painter().rect_filled(rect, 3.0, bg);
    paint_button_label(ui.painter(), rect, label, tex.as_ref());

    response.clicked()
}

/// Hold state for hold-to-activate buttons.
#[derive(Clone)]
enum HoldState {
    /// Tracking hold progress since this instant.
    Holding(std::time::Instant),
    /// Already fired — waiting for release before allowing another activation.
    Fired,
}

/// Hold-to-activate button. Returns `true` once per hold when the timer completes.
/// Will not re-fire until the user releases and holds again.
fn hold_button(
    ui: &mut egui::Ui,
    label: &str,
    hold_secs: f32,
    icon: Option<&mut crate::icons::SvgIcon>,
) -> bool {
    let tex = resolve_button_icon(ui, icon);
    let btn_size = egui::vec2(ui.available_width(), 28.0);
    let (rect, response) = ui.allocate_exact_size(btn_size, egui::Sense::click_and_drag());

    let bg = if response.hovered() {
        egui::Color32::from_gray(60)
    } else {
        egui::Color32::from_gray(45)
    };
    ui.painter().rect_filled(rect, 3.0, bg);

    let id = response.id;
    let holding = response.is_pointer_button_down_on();
    let mut fired = false;

    if holding {
        let state = ui.data_mut(|d| {
            d.get_temp_mut_or_insert_with(id, || HoldState::Holding(std::time::Instant::now()))
                .clone()
        });

        let progress = match state {
            HoldState::Holding(start) => {
                let p = (start.elapsed().as_secs_f32() / hold_secs).min(1.0);
                if p >= 1.0 {
                    fired = true;
                    ui.data_mut(|d| d.insert_temp(id, HoldState::Fired));
                }
                p
            }
            HoldState::Fired => 1.0,
        };

        let fill_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * progress, rect.height()));
        ui.painter().rect_filled(
            fill_rect,
            3.0,
            egui::Color32::from_rgba_unmultiplied(255, 60, 60, 80),
        );
    } else {
        ui.data_mut(|d| d.remove::<HoldState>(id));
    }

    paint_button_label(ui.painter(), rect, label, tex.as_ref());

    fired
}

/// GPIO press/release button. Returns `Some(true)` on press, `Some(false)` on release.
///
/// A 1-second arm delay acts as a safety gate (the real button needs a safety pin).
/// The GPIO press is only injected after the arm delay; releasing before that does nothing.
/// After arming, zone labels show what `ButtonManager` will do on release:
///   0–2 s → reboot, 2–5 s → ignored, 5+ s → factory reset.
/// Draw a thin vertical marker on a slider showing the app's value when an override is active.
fn draw_app_marker(ui: &egui::Ui, slider_response: &egui::Response, frac: f32) {
    let r = slider_response.rect;
    let x = r.min.x + frac * r.width();
    ui.painter().line_segment(
        [egui::pos2(x, r.min.y), egui::pos2(x, r.max.y)],
        egui::Stroke::new(
            2.0_f32,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 100),
        ),
    );
}

fn gpio_button(
    ui: &mut egui::Ui,
    label: &str,
    icon: Option<&mut crate::icons::SvgIcon>,
) -> Option<bool> {
    /// Hold time before the GPIO press event is actually injected.
    const ARM_DELAY: f32 = 1.0;
    /// Max visual duration for the progress bar (arm delay + 6s of GPIO hold).
    const VISUAL_MAX: f32 = ARM_DELAY + 7.0;

    let tex = resolve_button_icon(ui, icon);

    let btn_size = egui::vec2(ui.available_width(), 22.0);
    let (rect, response) = ui.allocate_exact_size(btn_size, egui::Sense::click_and_drag());
    let id = response.id;
    let holding = response.is_pointer_button_down_on();

    // Two-phase temp state:
    //   Phase 1 (arming):  Instant stored, but GPIO press not yet sent
    //   Phase 2 (armed):   bool `true` stored alongside — press was sent
    let prev = ui.data(|d| {
        (
            d.get_temp::<std::time::Instant>(id),
            d.get_temp::<bool>(id).unwrap_or(false),
        )
    });
    let (hold_start, was_armed) = prev;

    let mut event: Option<bool> = None;

    if holding {
        let start = hold_start.unwrap_or_else(|| {
            let now = std::time::Instant::now();
            ui.data_mut(|d| d.insert_temp(id, now));
            now
        });
        let elapsed = start.elapsed().as_secs_f32();

        // Arm: inject GPIO press once the arm delay passes
        if !was_armed && elapsed >= ARM_DELAY {
            ui.data_mut(|d| d.insert_temp::<bool>(id, true));
            event = Some(true);
        }
    } else if hold_start.is_some() {
        // Released — clean up state, send GPIO release only if we were armed
        ui.data_mut(|d| {
            d.remove::<std::time::Instant>(id);
            d.remove::<bool>(id);
        });
        if was_armed {
            event = Some(false);
        }
    }

    // Background
    let bg = if response.hovered() {
        egui::Color32::from_gray(60)
    } else {
        egui::Color32::from_gray(45)
    };
    ui.painter().rect_filled(rect, 3.0, bg);

    // Progress fill while held
    let display_label = if let Some(start) = ui.data(|d| d.get_temp::<std::time::Instant>(id)) {
        let elapsed = start.elapsed().as_secs_f32();
        let progress = (elapsed / VISUAL_MAX).min(1.0);

        // Time on the GPIO clock (0 until armed)
        let gpio_secs = (elapsed - ARM_DELAY).max(0.0);

        let fill_color = if elapsed < ARM_DELAY {
            // Arming phase — dim yellow
            egui::Color32::from_rgba_unmultiplied(180, 180, 60, 60)
        } else if gpio_secs < 2.0 {
            // Reboot zone — green
            egui::Color32::from_rgba_unmultiplied(60, 180, 60, 80)
        } else if gpio_secs < 5.0 {
            // Dead zone — gray
            egui::Color32::from_rgba_unmultiplied(128, 128, 128, 60)
        } else {
            // Factory reset zone — red
            egui::Color32::from_rgba_unmultiplied(255, 60, 60, 80)
        };

        let fill_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * progress, rect.height()));
        ui.painter().rect_filled(fill_rect, 3.0, fill_color);
        ui.ctx().request_repaint();

        if elapsed < ARM_DELAY {
            format!("{label} (arming...)")
        } else {
            let zone = if gpio_secs < 2.0 {
                "reboot"
            } else if gpio_secs < 5.0 {
                "\u{2014}"
            } else {
                "factory reset"
            };
            format!("{label} ({gpio_secs:.1}s {zone})")
        }
    } else {
        label.to_owned()
    };

    paint_button_label(ui.painter(), rect, &display_label, tex.as_ref());

    event
}

/// Paint a centered label with an optional icon to the left, used by custom buttons.
fn paint_button_label(
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    icon: Option<&egui::TextureHandle>,
) {
    let color = egui::Color32::from_gray(180);
    let font = egui::FontId::proportional(12.0);
    let icon_size: f32 = 14.0;
    let gap: f32 = 6.0;
    let padding: f32 = 8.0;
    let cy = rect.center().y;
    let start_x = rect.min.x + padding;

    if let Some(tex) = icon {
        // Measure text to align icon + text as a group on the same baseline
        let galley = painter.layout_no_wrap(label.to_owned(), font.clone(), color);
        let text_h = galley.size().y;
        let group_h = icon_size.max(text_h);
        let icon_y = cy - group_h / 2.0 + (group_h - icon_size) / 2.0;
        let text_y = cy - group_h / 2.0 + (group_h - text_h) / 2.0;

        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(start_x, icon_y),
            egui::vec2(icon_size, icon_size),
        );
        painter.image(
            tex.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            color,
        );
        painter.galley(egui::pos2(start_x + icon_size + gap, text_y), galley, color);
    } else {
        painter.text(
            egui::pos2(start_x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font,
            color,
        );
    }
}

/// Draw a dark desk surface below the device, with a subtle edge highlight
/// where the desk meets the "wall" background.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "gradient row math on small positive values"
)]
fn draw_table_surface(painter: &egui::Painter, device_rect: egui::Rect, window_rect: egui::Rect) {
    // The desk surface starts roughly where the device legs meet the table
    // (about 85% down the device frame, which is where the legs touch)
    let desk_top = device_rect.min.y + device_rect.height() * 0.82;
    let desk_rect =
        egui::Rect::from_min_max(egui::pos2(window_rect.min.x, desk_top), window_rect.max);

    if desk_rect.height() <= 0.0 {
        return;
    }

    // Clean dark surface with subtle edge highlight
    let rows = desk_rect.height().ceil() as usize;
    for row in 0..rows {
        let depth = row as f32 / rows as f32;
        let brightness = 0.09 + (-depth * 4.0).exp() * 0.06;
        let red = (brightness * 1.05 * 255.0).clamp(0.0, 255.0) as u8;
        let green = (brightness * 255.0).clamp(0.0, 255.0) as u8;
        let blue = (brightness * 0.95 * 255.0).clamp(0.0, 255.0) as u8;

        let row_y = desk_rect.min.y + row as f32;
        let row_rect = egui::Rect::from_min_size(
            egui::pos2(desk_rect.min.x, row_y),
            egui::vec2(desk_rect.width(), 1.0),
        );
        painter.rect_filled(row_rect, 0.0, egui::Color32::from_rgb(red, green, blue));
    }
}

/// Install the repo's bundled Noto Sans as the highest-priority font for both proportional and monospace families.
/// Egui 0.34's default Ubuntu-Light has gaps which turn into tofu in the UI chrome.
/// Noto Sans Regular covers the BMP well enough that we can use the glyphs we actually want.
fn install_noto_sans(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "noto_sans".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/NotoSans-Regular.ttf"
        ))),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "noto_sans".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// macOS sRGB color space workaround (egui#2712).
/// Without this, colors appear washed out on wide-gamut displays.
#[cfg(target_os = "macos")]
#[expect(unsafe_code, reason = "objc2 FFI needed to set NSWindow color space")]
fn set_macos_srgb_colorspace(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = cc.window_handle() else {
        tracing::warn!("cannot get window handle — skipping sRGB color-space fix");
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };

    unsafe {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};

        let ns_view: &AnyObject = &*appkit.ns_view.as_ptr().cast();
        let ns_window: *mut AnyObject = msg_send![ns_view, window];
        if ns_window.is_null() {
            tracing::warn!("NSView has no window — skipping sRGB color-space fix");
            return;
        }

        let cls =
            AnyClass::get("NSColorSpace").expect("BUG: NSColorSpace class not found on macOS");
        let srgb: *mut AnyObject = msg_send![cls, sRGBColorSpace];
        let _: () = msg_send![&*ns_window, setColorSpace: &*srgb];
        tracing::info!("macOS: set window color space to sRGB (egui#2712 workaround)");
    }
}

/// Draw an animated spinner arc at the given center point.
fn draw_spinner(painter: &egui::Painter, center: egui::Pos2, time: f64, color: egui::Color32) {
    let radius = 12.0;
    let stroke = egui::Stroke::new(2.0_f32, color);
    let segments = 40;
    let arc_len = std::f64::consts::TAU * 0.7;
    let start_angle = time * 4.0;

    let points: Vec<egui::Pos2> = (0..=segments)
        .map(|i| {
            let t = f64::from(i) / f64::from(segments);
            let angle = start_angle + t * arc_len;
            let (sin, cos) = angle.sin_cos();
            #[expect(clippy::cast_possible_truncation, reason = "trig values are small")]
            egui::pos2(
                center.x + cos as f32 * radius,
                center.y + sin as f32 * radius,
            )
        })
        .collect();

    painter.add(egui::Shape::line(points, stroke));
}

/// Draw a simple warning badge used when the relay is alive but framebuffer
/// capture is unavailable.
fn draw_warning_badge(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    painter.circle_stroke(center, 12.0, egui::Stroke::new(2.0_f32, color));
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        "!",
        egui::FontId::proportional(18.0),
        color,
    );
}

/// Lay out and paint the centred status overlay (icon + wrapped text) inside
/// a rect. Text wraps to the rect width so long relay-side error reasons stay
/// inside the device screen instead of bleeding over the desk background.
fn draw_status_overlay(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    message: &str,
    show_warning: bool,
    show_spinner: bool,
) {
    let padding = 16.0;
    let wrap_width = (rect.width() - padding * 2.0).max(32.0);
    let font = egui::FontId::proportional(14.0);
    let text_color = egui::Color32::from_gray(210);
    let warning_color = egui::Color32::from_rgb(255, 170, 80);

    let galley = ui
        .painter()
        .layout(message.to_owned(), font, text_color, wrap_width);
    let text_size = galley.size();

    // Stack: [icon] -> 12px gap -> [text]. Center the stack vertically.
    let icon_size = 24.0;
    let gap = 12.0;
    let total_h = icon_size + gap + text_size.y;
    let start_y = rect.center().y - total_h / 2.0;

    let icon_center = egui::pos2(rect.center().x, start_y + icon_size / 2.0);
    if show_warning {
        draw_warning_badge(ui.painter(), icon_center, warning_color);
    } else if show_spinner {
        let t = ui.input(|i| i.time);
        draw_spinner(ui.painter(), icon_center, t, text_color);
    }

    let text_pos = egui::pos2(
        rect.center().x - text_size.x / 2.0,
        start_y + icon_size + gap,
    );
    ui.painter().galley(text_pos, galley, text_color);
}

/// Resolve the relay address from env or default.
fn resolve_addr() -> String {
    if let Ok(addr) = std::env::var("BMC_VIRT_RELAY_ADDR") {
        return addr;
    }
    format!("127.0.0.1:{DEFAULT_PORT}")
}
