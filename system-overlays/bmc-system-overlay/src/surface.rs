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

//! wlr-layer-shell Wayland client for system overlays.
//!
//! Mirrors `bmc_widget`'s `deck_widget` surface client, swapping the
//! `deck_widget_manager_v2` surface for a `zwlr_layer_shell_v1` layer
//! surface. Overlays self-pace their redraws off the framework's
//! tick/`next_wake` schedule, so this client never requests a
//! `wl_surface.frame` callback for redraw pacing. The one exception is a
//! non-blocking presentation fence a hosted overlay requests just before its
//! unmap, so the compositor's last repaint is confirmed shown before the
//! NULL attach (see `request_presentation_fence`).

use std::time::{Duration, Instant};

use ::deck_alarm_v1::client::deck_alarm_v1::{self, DeckAlarmV1, Snooze};
use ::deck_device_info_v1::client::deck_device_info_v1::{
    self, DeckDeviceInfoV1, DeviceState as WireDeviceState, SetupState as WireSetupState,
};
use ::deck_upgrade_v1::UpgradeDecoder;
use ::deck_upgrade_v1::client::deck_upgrade_v1::{self, DeckUpgradeV1};
use anyhow::Context;
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_region, wl_registry, wl_seat, wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::overlay::{AlarmEvent, InputRegion, LayerConfig, ScreenEdge};
use ::deck_settings_v1::client::deck_settings_v1::{self, Capability, DeckSettingsV1};
use bmc_widget::egl::DmaBufInfo;
use bmc_widget::surface::{
    BufferSlotMap, PollOutcome, ReleasedBuffer, ReleasedBufferSet, create_buffer_from_dmabuf,
    drain_released_buffers, poll_dispatch, record_released_buffer, submit_buffer_to_surface,
    unregister_wl_buffer_slot,
};
use deck_screen_edge_v1::client::deck_auto_hide_screen_edge_v1::{self, DeckAutoHideScreenEdgeV1};
use deck_screen_edge_v1::client::deck_screen_edge_manager_v1::{self, DeckScreenEdgeManagerV1};

/// Wayland protocol state for a layer-shell overlay surface.
///
/// Holds the bound globals, surface objects, configure handshake state, and
/// the dmabuf buffer-release bookkeeping that mirrors the `deck_widget`
/// client. Lives behind [`LayerSurfaceClient`], which owns the connection and
/// event queue.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Wayland client state stores independent protocol latches"
)]
struct State {
    /// Whether the event loop should keep running. Cleared on `Closed`.
    running: bool,

    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    seat: Option<wl_seat::WlSeat>,
    touch: Option<wl_touch::WlTouch>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    screen_edge_manager: Option<DeckScreenEdgeManagerV1>,
    screen_edge: Option<DeckAutoHideScreenEdgeV1>,
    /// Set when the compositor reveals an armed screen edge.
    pending_reveal: bool,
    /// Set when the compositor hides the revealed screen edge.
    pending_hidden: bool,

    /// Whether this overlay opted into `deck_settings_v1` (its
    /// `SystemOverlay::uses_settings`). Set at construction; gates the registry
    /// bind so non-settings overlays neither bind it nor receive its events.
    wants_settings: bool,
    settings: Option<DeckSettingsV1>,
    /// Set on a `brightness` event; drained by the framework into on_brightness.
    pending_brightness: Option<u8>,
    /// Set on a `volume` event; drained by the framework into on_volume.
    pending_volume: Option<u8>,
    /// Set on a `capabilities` event (first event after a v2 bind).
    pending_capabilities: Option<crate::overlay::SettingsCaps>,
    /// Set on a `night_mode` event.
    pending_night_mode: Option<(bool, Option<String>)>,
    /// Set on a one-shot `restart_declined` event.
    pending_restart_declined: Option<String>,
    /// Set on a `preempted` event; `true` when a modal full-screen overlay is
    /// covering the scene, drained by the framework into `on_preempted`.
    pending_preempted: Option<bool>,
    /// Set on a `wifi_ap` event; `Some(Some(ssid))`/`Some(None)` distinguishes a
    /// fresh event (active/inactive) from "no event this pass".
    #[expect(
        clippy::option_option,
        reason = "outer Option latches event-arrived; inner Option carries active/inactive SSID"
    )]
    pending_wifi_ap: Option<Option<String>>,

    /// Whether this overlay opted into `deck_alarm_v1` (its
    /// `SystemOverlay::uses_alarm`).
    wants_alarm: bool,
    alarm: Option<DeckAlarmV1>,
    /// Latest `deck_alarm_v1` event seen this dispatch round. A single slot
    /// (not separate ring/stop flags) so a `stop` then `ring` arriving in one
    /// round keeps the ring — the trailing event wins, preserving wire order.
    pending_alarm_event: Option<AlarmEvent>,

    /// Whether this overlay opted into `deck_upgrade_v1` (its
    /// `SystemOverlay::uses_upgrade`).
    wants_upgrade: bool,
    upgrade: Option<DeckUpgradeV1>,
    upgrade_decoder: UpgradeDecoder,
    /// Latest coherent `deck_upgrade_v1` snapshot, drained by the framework
    /// before its next tick.
    pending_upgrade_snapshot: Option<crate::overlay::UpgradeSnapshot>,

    /// Whether this overlay opted into `deck_device_info_v1` (its
    /// `SystemOverlay::uses_device_info`).
    wants_device_info: bool,
    device_info: Option<DeckDeviceInfoV1>,
    /// Latest `deck_device_info_v1` events, one latest-wins slot per event
    /// kind: the on-bind replay delivers all three back-to-back in a single
    /// dispatch round, so a shared slot would drop two of them.
    pending_device_lifecycle: Option<(crate::overlay::DeviceState, bool)>,
    pending_setup_progress: Option<(crate::overlay::SetupStep, String)>,
    #[expect(
        clippy::option_option,
        reason = "outer Option latches event-arrived; inner Option carries AP up/down"
    )]
    pending_access_point: Option<Option<crate::overlay::AccessPoint>>,

    /// Set true on the first layer-surface Configure (after which we may map).
    configured: bool,
    /// Compositor-suggested size from the latest Configure.
    configured_size: (u32, u32),
    pending_touch: Vec<crate::overlay::TouchEvent>,
    /// Surface-dirty from a Configure/resize only. Redraw pacing is the
    /// framework's tick/next_wake job and never uses frame callbacks; the
    /// only `wl_surface.frame` this client ever requests is the pre-unmap
    /// presentation fence, tracked separately below.
    needs_render: bool,
    /// Latest post-connect compositor-suggested size, when it changed.
    pending_size_change: Option<(u32, u32)>,

    /// `ObjectId` of the `wl_callback` from the most recent
    /// `request_presentation_fence`, cleared when it resolves or is
    /// cancelled. Matching on this id (rather than latching on any `Done`)
    /// keeps a late callback from an abandoned fence from completing a
    /// newer one early.
    pending_fence: Option<ObjectId>,
    /// Set on a `wl_callback::Event::Done` whose id matched `pending_fence`;
    /// drained by `take_frame_presented`.
    fence_done: bool,

    buffer_slots: BufferSlotMap,
    released_buffers: ReleasedBufferSet,
}

impl Default for State {
    fn default() -> Self {
        Self {
            running: true,
            compositor: None,
            layer_shell: None,
            linux_dmabuf: None,
            seat: None,
            touch: None,
            surface: None,
            layer_surface: None,
            screen_edge_manager: None,
            screen_edge: None,
            pending_reveal: false,
            pending_hidden: false,
            wants_settings: false,
            settings: None,
            pending_brightness: None,
            pending_volume: None,
            pending_capabilities: None,
            pending_night_mode: None,
            pending_restart_declined: None,
            pending_preempted: None,
            pending_wifi_ap: None,
            wants_alarm: false,
            alarm: None,
            pending_alarm_event: None,
            wants_upgrade: false,
            upgrade: None,
            upgrade_decoder: UpgradeDecoder::default(),
            pending_upgrade_snapshot: None,
            wants_device_info: false,
            device_info: None,
            pending_device_lifecycle: None,
            pending_setup_progress: None,
            pending_access_point: None,
            configured: false,
            configured_size: (0, 0),
            pending_touch: Vec::new(),
            needs_render: false,
            pending_size_change: None,
            pending_fence: None,
            fence_done: false,
            buffer_slots: BufferSlotMap::new(),
            released_buffers: ReleasedBufferSet::new(),
        }
    }
}

impl State {
    fn mark_screen_edge_revealed(&mut self) {
        self.pending_reveal = true;
        self.pending_hidden = false;
    }

    fn mark_screen_edge_hidden(&mut self) {
        self.pending_reveal = false;
        self.pending_hidden = true;
    }

    /// Record an `alarm_ringing` into the single latest-wins alarm slot.
    fn note_alarm_ring(
        &mut self,
        time: String,
        period: String,
        label: String,
        snooze_allowed: bool,
    ) {
        self.pending_alarm_event = Some(AlarmEvent::Ring {
            time,
            period,
            label,
            snooze_allowed,
        });
    }

    /// Record an `alarm_stopped` into the single latest-wins alarm slot.
    fn note_alarm_stop(&mut self) {
        self.pending_alarm_event = Some(AlarmEvent::Stop);
    }

    fn on_upgrade_event(&mut self, event: &deck_upgrade_v1::Event) {
        if let Some(snapshot) = self.upgrade_decoder.decode(event) {
            self.pending_upgrade_snapshot = Some(snapshot);
        }
    }

    fn discard_unmap_configure(&mut self, previous_size: (u32, u32)) {
        self.configured_size = previous_size;
        self.pending_size_change = None;
        self.needs_render = false;
    }

    fn discard_resize_unmap_configure(&mut self, requested_size: (u32, u32)) {
        self.discard_unmap_configure(requested_size);
    }

    fn mark_fence_done(&mut self, callback_id: &ObjectId) {
        if self.pending_fence.as_ref() == Some(callback_id) {
            self.fence_done = true;
            self.pending_fence = None;
        }
    }
}

/// How long [`LayerSurfaceClient::connect`] waits for the compositor to send
/// the first Configure before giving up.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(10);

fn apply_layer_config(
    compositor: &wl_compositor::WlCompositor,
    surface: &wl_surface::WlSurface,
    layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    config: &LayerConfig,
    qh: &QueueHandle<State>,
) {
    layer_surface.set_layer(config.layer);
    layer_surface.set_anchor(config.anchor);
    layer_surface.set_size(config.size.0, config.size.1);
    layer_surface.set_margin(
        config.margin_top,
        config.margin_right,
        config.margin_bottom,
        config.margin_left,
    );
    layer_surface.set_exclusive_zone(config.exclusive_zone);
    match config.input {
        InputRegion::Full => surface.set_input_region(None),
        InputRegion::None => {
            let region = compositor.create_region(qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();
        }
    }
}

fn wait_for_configure(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    context: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + CONFIGURE_TIMEOUT;
    while !state.configured {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let remaining_ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
        match poll_dispatch(conn, queue, state, remaining_ms)
            .with_context(|| format!("{context}: dispatch awaiting configure"))?
        {
            PollOutcome::Events => {}
            PollOutcome::Timeout => {
                anyhow::bail!(
                    "{context}: timed out after {:?} waiting for layer-surface configure",
                    CONFIGURE_TIMEOUT
                );
            }
        }
    }
    Ok(())
}

/// Single-connection Wayland client for a wlr-layer-shell overlay surface.
///
/// Connects to the compositor, binds the layer-shell global, creates and
/// configures a layer surface, and mints/attaches DMA-BUF buffers through the
/// shared `bmc-widget` helpers.
pub struct LayerSurfaceClient {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    config: LayerConfig,
    needs_remap_configure: bool,
}

impl std::fmt::Debug for LayerSurfaceClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerSurfaceClient")
            .field("running", &self.state.running)
            .field("configured", &self.state.configured)
            .field("configured_size", &self.state.configured_size)
            .field("pending_touch", &self.state.pending_touch.len())
            .field("buffer_slots", &self.state.buffer_slots.len())
            .field("released_buffers", &self.state.released_buffers.len())
            .finish_non_exhaustive()
    }
}

/// Which optional control protocols the overlay opted into (its `uses_*()`
/// answers), gating the registry binds.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one independent opt-in flag per control protocol"
)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ProtocolOptIns {
    pub settings: bool,
    pub alarm: bool,
    pub upgrade: bool,
    pub device_info: bool,
}

impl ProtocolOptIns {
    pub(crate) fn from_overlay(overlay: &dyn crate::overlay::SystemOverlay) -> Self {
        Self {
            settings: overlay.uses_settings(),
            alarm: overlay.uses_alarm(),
            upgrade: overlay.uses_upgrade(),
            device_info: overlay.uses_device_info(),
        }
    }
}

impl LayerSurfaceClient {
    /// Connect to the Wayland display, create a layer surface from `config`,
    /// and block until the compositor emits its first Configure.
    pub fn connect(
        config: &crate::overlay::LayerConfig,
        opt_ins: ProtocolOptIns,
    ) -> anyhow::Result<Self> {
        let conn =
            Connection::connect_to_env().map_err(|e| anyhow::anyhow!("wayland connect: {e}"))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());

        let mut state = State {
            wants_settings: opt_ins.settings,
            wants_alarm: opt_ins.alarm,
            wants_upgrade: opt_ins.upgrade,
            wants_device_info: opt_ins.device_info,
            ..State::default()
        };
        queue
            .roundtrip(&mut state)
            .map_err(|e| anyhow::anyhow!("roundtrip: {e}"))?;

        let compositor = state.compositor.clone().context("wl_compositor missing")?;
        let layer_shell = state
            .layer_shell
            .clone()
            .context("zwlr_layer_shell_v1 missing")?;
        anyhow::ensure!(state.linux_dmabuf.is_some(), "zwp_linux_dmabuf_v1 missing");

        let surface = compositor.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            config.layer,
            config.namespace.clone(),
            &qh,
            (),
        );
        apply_layer_config(&compositor, &surface, &layer_surface, config, &qh);
        surface.commit();

        state.surface = Some(surface);
        state.layer_surface = Some(layer_surface);

        wait_for_configure(
            &conn,
            &mut queue,
            &mut state,
            "initial layer-surface configure",
        )?;

        tracing::info!(
            "Layer surface ready: {}x{} namespace={}",
            state.configured_size.0,
            state.configured_size.1,
            config.namespace,
        );
        state.pending_size_change = None;

        Ok(Self {
            conn,
            queue,
            state,
            config: config.clone(),
            needs_remap_configure: false,
        })
    }

    /// Mint a `wl_buffer` from DMA-BUF info and register it for the given slot.
    pub fn mint_wl_buffer(
        &mut self,
        info: &DmaBufInfo,
        slot: usize,
    ) -> anyhow::Result<wl_buffer::WlBuffer> {
        let qh = self.queue.handle();
        let linux_dmabuf = self
            .state
            .linux_dmabuf
            .as_ref()
            .context("zwp_linux_dmabuf_v1 missing")?;
        let buffer = create_buffer_from_dmabuf(linux_dmabuf, info, &qh);
        self.state.buffer_slots.insert(buffer.id(), slot);
        Ok(buffer)
    }

    /// Attach `buffer`, damage the surface, and commit. Never requests a frame
    /// callback: overlays self-pace via the framework tick.
    pub fn submit_buffer_with_wl_buffer(
        &mut self,
        info: &DmaBufInfo,
        buffer: &wl_buffer::WlBuffer,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.needs_remap_configure,
            "layer surface needs remap configure before buffer attach"
        );
        let qh = self.queue.handle();
        let surface = self.state.surface.as_ref().context("surface not created")?;
        submit_buffer_to_surface(surface, &qh, buffer, info, false);
        Ok(())
    }

    /// Request a `wl_surface.frame` callback as a non-blocking presentation
    /// fence and commit with no attach/damage, replacing any previously
    /// pending fence. A layer-shell commit drains queued frame callbacks
    /// regardless of buffer assignment and marks full output damage when any
    /// were present, so this commit both queues the callback and forces the
    /// redraw whose completion fires it.
    pub fn request_presentation_fence(&mut self) -> anyhow::Result<()> {
        let qh = self.queue.handle();
        let surface = self.state.surface.as_ref().context("surface not created")?;
        let callback = surface.frame(&qh, ());
        self.state.pending_fence = Some(callback.id());
        self.state.fence_done = false;
        surface.commit();
        self.conn
            .flush()
            .map_err(|e| anyhow::anyhow!("wl flush on presentation fence: {e}"))?;
        Ok(())
    }

    /// Drain whether the pending presentation fence has resolved.
    pub fn take_frame_presented(&mut self) -> bool {
        std::mem::take(&mut self.state.fence_done)
    }

    /// Abandon a pending presentation fence without waiting for its
    /// callback. A late `Done` for it is then ignored, since it no longer
    /// matches `pending_fence`.
    pub fn cancel_presentation_fence(&mut self) {
        self.state.pending_fence = None;
        self.state.fence_done = false;
    }

    /// Unmap the surface: attach a NULL buffer and commit. The compositor
    /// releases the previously-attached buffer and evicts its texture (handled
    /// compositor-side on the `Removed` assignment).
    pub fn attach_null_buffer(&mut self) -> anyhow::Result<()> {
        let surface = self.state.surface.as_ref().context("surface not created")?;
        surface.attach(None, 0, 0);
        surface.commit();
        self.conn
            .flush()
            .map_err(|e| anyhow::anyhow!("wl flush on unmap: {e}"))?;
        self.needs_remap_configure = true;
        Ok(())
    }

    /// Drain the compositor response to an unmap commit before local buffer
    /// proxies are destroyed. The compositor sends `wl_buffer.release` when it
    /// observes the NULL attach; dispatching that before destruction avoids
    /// later reads for already-dead client-side proxies.
    pub fn roundtrip_after_unmap(&mut self) -> anyhow::Result<()> {
        self.queue
            .roundtrip(&mut self.state)
            .map_err(|e| anyhow::anyhow!("wl roundtrip after unmap: {e}"))?;
        Ok(())
    }

    /// Drain the compositor response to an unmap commit, but keep the last
    /// usable mapped size. Some compositors send a placeholder configure while
    /// the surface is unmapped; that size is not actionable for the reusable
    /// render target and would shrink stretch-axis overlays before remap.
    pub fn roundtrip_after_hide_unmap(&mut self) -> anyhow::Result<()> {
        let previous_size = self.state.configured_size;
        self.roundtrip_after_unmap()?;
        self.state.discard_unmap_configure(previous_size);
        Ok(())
    }

    /// Drain the compositor response to an unmap commit made during mapped
    /// resize, preserving the configured size that triggered the resize.
    pub fn roundtrip_after_resize_unmap(
        &mut self,
        requested_size: (u32, u32),
    ) -> anyhow::Result<()> {
        self.roundtrip_after_unmap()?;
        self.state.discard_resize_unmap_configure(requested_size);
        Ok(())
    }

    /// Re-apply layer-surface state after a NULL-buffer unmap, just before the
    /// next real buffer commit.
    ///
    /// wlr-layer-shell resets layer/anchor/size/margins on unmap. The unmap
    /// roundtrip already drains the compositor's initial configure; for the
    /// custom screen-edge flow, waiting for another configure after `revealed`
    /// can block indefinitely. The next buffer attach commits these restored
    /// pending values together with the buffer.
    pub fn ensure_ready_for_buffer_attach(&mut self) -> anyhow::Result<bool> {
        if !self.needs_remap_configure {
            return Ok(false);
        }

        {
            let qh = self.queue.handle();
            let compositor = self
                .state
                .compositor
                .as_ref()
                .context("wl_compositor missing")?;
            let surface = self.state.surface.as_ref().context("surface not created")?;
            let layer_surface = self
                .state
                .layer_surface
                .as_ref()
                .context("layer surface not created")?;
            apply_layer_config(compositor, surface, layer_surface, &self.config, &qh);
        }
        self.needs_remap_configure = false;
        Ok(true)
    }

    /// Create and arm the compositor-managed auto-hide object for `edge`.
    pub fn create_screen_edge(&mut self, edge: ScreenEdge) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.state.screen_edge.is_none(),
            "screen edge already created for this layer surface"
        );
        let qh = self.queue.handle();
        let manager = self
            .state
            .screen_edge_manager
            .as_ref()
            .context("deck_screen_edge_manager_v1 missing")?;
        let surface = self.state.surface.as_ref().context("surface not created")?;
        let border = match edge {
            ScreenEdge::Top => deck_screen_edge_manager_v1::Border::Top,
            ScreenEdge::Bottom => deck_screen_edge_manager_v1::Border::Bottom,
        };
        let edge = manager.get_auto_hide_screen_edge(border, surface, &qh, ());
        tracing::info!(?border, "Activating screen edge");
        edge.activate();
        self.state.screen_edge = Some(edge);
        self.flush()
    }

    /// Re-arm an existing screen edge after the compositor hides it.
    pub fn rearm_screen_edge(&mut self) -> anyhow::Result<()> {
        let edge = self.state.screen_edge.as_ref().context("no screen edge")?;
        tracing::info!("Re-arming screen edge after hide");
        edge.activate();
        self.flush()
    }

    /// Drain whether the compositor revealed the armed screen edge.
    pub fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.state.pending_reveal)
    }

    /// Drain whether the compositor hid the revealed screen edge.
    pub fn take_hidden(&mut self) -> bool {
        std::mem::take(&mut self.state.pending_hidden)
    }

    pub fn take_brightness(&mut self) -> Option<u8> {
        self.state.pending_brightness.take()
    }

    pub fn take_volume(&mut self) -> Option<u8> {
        self.state.pending_volume.take()
    }

    pub fn take_capabilities(&mut self) -> Option<crate::overlay::SettingsCaps> {
        self.state.pending_capabilities.take()
    }

    pub fn take_night_mode(&mut self) -> Option<(bool, Option<String>)> {
        self.state.pending_night_mode.take()
    }

    pub fn take_restart_declined(&mut self) -> Option<String> {
        self.state.pending_restart_declined.take()
    }

    pub fn take_preempted(&mut self) -> Option<bool> {
        self.state.pending_preempted.take()
    }

    pub fn take_wifi_ap(&mut self) -> Option<Option<String>> {
        self.state.pending_wifi_ap.take()
    }

    pub fn take_alarm_event(&mut self) -> Option<AlarmEvent> {
        self.state.pending_alarm_event.take()
    }

    pub fn take_upgrade_snapshot(&mut self) -> Option<crate::overlay::UpgradeSnapshot> {
        self.state.pending_upgrade_snapshot.take()
    }

    /// The lifecycle state, and whether this session's operational boot sequence
    /// has already been delivered.
    pub fn take_device_state(&mut self) -> Option<(crate::overlay::DeviceState, bool)> {
        self.state.pending_device_lifecycle.take()
    }

    pub fn take_setup_progress(&mut self) -> Option<(crate::overlay::SetupStep, String)> {
        self.state.pending_setup_progress.take()
    }

    /// Outer `Some` means an `access_point` event arrived;
    /// the inner value is the AP while it is up, `None` when it is down.
    pub fn take_access_point(&mut self) -> Option<Option<crate::overlay::AccessPoint>> {
        self.state.pending_access_point.take()
    }

    pub fn send_settings_request(
        &self,
        req: crate::overlay::SettingsRequest,
    ) -> anyhow::Result<()> {
        use crate::overlay::SettingsRequest;
        let settings = self
            .state
            .settings
            .as_ref()
            .context("deck_settings_v1 not bound")?;
        let v2 = settings.version() >= 2;
        match req {
            SettingsRequest::SetBrightness(v) => settings.set_brightness(u32::from(v)),
            SettingsRequest::SetVolume(v) if v2 => settings.set_volume(u32::from(v)),
            SettingsRequest::ToggleNightMode if v2 => settings.toggle_night_mode(),
            SettingsRequest::Restart if v2 => settings.restart(),
            SettingsRequest::SetVolume(_)
            | SettingsRequest::ToggleNightMode
            | SettingsRequest::Restart => {
                tracing::warn!(?req, "dropping v2 settings request on a v1 compositor");
            }
            SettingsRequest::ReconfigureWifi => settings.reconfigure_wifi(),
        }
        self.flush()
    }

    pub fn send_alarm_request(&self, req: crate::overlay::AlarmRequest) -> anyhow::Result<()> {
        use crate::overlay::AlarmRequest;
        let alarm = self
            .state
            .alarm
            .as_ref()
            .context("deck_alarm_v1 not bound")?;
        match req {
            AlarmRequest::Dismiss => alarm.dismiss_alarm(),
            AlarmRequest::Snooze => alarm.snooze_alarm(),
        }
        self.flush()
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.state.configured_size
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.state.running
    }

    pub fn take_needs_render(&mut self) -> bool {
        std::mem::take(&mut self.state.needs_render)
    }

    pub fn take_configured_size_change(&mut self) -> Option<(u32, u32)> {
        self.state.pending_size_change.take()
    }

    pub fn drain_touch(&mut self) -> Vec<crate::overlay::TouchEvent> {
        std::mem::take(&mut self.state.pending_touch)
    }

    pub fn drain_released_buffers(&mut self) -> Vec<ReleasedBuffer> {
        drain_released_buffers(&self.state.buffer_slots, &mut self.state.released_buffers)
    }

    pub fn poll_dispatch(&mut self, timeout_ms: i32) -> anyhow::Result<()> {
        poll_dispatch(&self.conn, &mut self.queue, &mut self.state, timeout_ms)
            .map(|_outcome| ())
            .context("poll_dispatch")
    }

    #[must_use]
    pub fn connection_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::{AsFd, AsRawFd};
        self.conn.as_fd().as_raw_fd()
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.conn
            .flush()
            .map_err(|e| anyhow::anyhow!("wl flush: {e}"))
    }

    pub fn destroy_minted_wl_buffer(&mut self, buffer: wl_buffer::WlBuffer) {
        let id = buffer.id();
        unregister_wl_buffer_slot(
            &mut self.state.buffer_slots,
            &mut self.state.released_buffers,
            &id,
        );
        buffer.destroy();
        drop(buffer);
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(
                        name,
                        version.min(6),
                        qh,
                        (),
                    );
                    state.compositor = Some(compositor);
                }
                "zwlr_layer_shell_v1" => {
                    let layer_shell = registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    state.layer_shell = Some(layer_shell);
                }
                "zwp_linux_dmabuf_v1" => {
                    let dmabuf = registry.bind::<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    state.linux_dmabuf = Some(dmabuf);
                }
                "wl_seat" if state.seat.is_none() => {
                    let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(9), qh, ());
                    state.seat = Some(seat);
                }
                "deck_screen_edge_manager_v1" => {
                    let manager = registry.bind::<DeckScreenEdgeManagerV1, _, _>(
                        name,
                        version.min(1),
                        qh,
                        (),
                    );
                    state.screen_edge_manager = Some(manager);
                }
                "deck_settings_v1" if state.wants_settings => {
                    let settings =
                        registry.bind::<DeckSettingsV1, _, _>(name, version.min(3), qh, ());
                    state.settings = Some(settings);
                }
                "deck_alarm_v1" if state.wants_alarm => {
                    let alarm = registry.bind::<DeckAlarmV1, _, _>(name, version.min(1), qh, ());
                    state.alarm = Some(alarm);
                }
                "deck_upgrade_v1" if state.wants_upgrade => {
                    let upgrade =
                        registry.bind::<DeckUpgradeV1, _, _>(name, version.min(1), qh, ());
                    state.upgrade = Some(upgrade);
                }
                "deck_device_info_v1" if state.wants_device_info => {
                    let device_info =
                        registry.bind::<DeckDeviceInfoV1, _, _>(name, version.min(1), qh, ());
                    state.device_info = Some(device_info);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                let size = (width, height);
                if state.configured && state.configured_size != size {
                    state.pending_size_change = Some(size);
                }
                state.configured_size = size;
                state.configured = true;
                state.needs_render = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.running = false;
            }
            _ => {}
        }
    }
}

impl Dispatch<DeckScreenEdgeManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &DeckScreenEdgeManagerV1,
        _: deck_screen_edge_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<DeckAutoHideScreenEdgeV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &DeckAutoHideScreenEdgeV1,
        event: deck_auto_hide_screen_edge_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            deck_auto_hide_screen_edge_v1::Event::Revealed => {
                tracing::info!("Screen edge revealed");
                state.mark_screen_edge_revealed();
            }
            deck_auto_hide_screen_edge_v1::Event::Hidden => {
                tracing::info!("Screen edge hidden");
                state.mark_screen_edge_hidden();
            }
            other => tracing::debug!(?other, "unhandled deck_auto_hide_screen_edge_v1 event"),
        }
    }
}

impl Dispatch<DeckSettingsV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &DeckSettingsV1,
        event: deck_settings_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            deck_settings_v1::Event::Brightness { value } => {
                state.pending_brightness = Some(u8::try_from(value.min(100)).unwrap_or(100));
            }
            deck_settings_v1::Event::WifiAp { ssid } => {
                let v = if ssid.is_empty() { None } else { Some(ssid) };
                state.pending_wifi_ap = Some(v);
            }
            deck_settings_v1::Event::Volume { value } => {
                state.pending_volume = Some(u8::try_from(value.min(100)).unwrap_or(100));
            }
            deck_settings_v1::Event::Capabilities { capabilities } => {
                let caps = match capabilities {
                    WEnum::Value(c) => c,
                    // An unknown bit from a newer compositor must not drop the
                    // known bits with it.
                    WEnum::Unknown(raw) => Capability::from_bits_truncate(raw),
                };
                state.pending_capabilities = Some(crate::overlay::SettingsCaps {
                    brightness: caps.contains(Capability::Brightness),
                    sound: caps.contains(Capability::Sound),
                    wifi_setup: caps.contains(Capability::WifiSetup),
                });
            }
            deck_settings_v1::Event::NightMode { active, until } => {
                state.pending_night_mode = Some((active != 0, until));
            }
            deck_settings_v1::Event::RestartDeclined { reason } => {
                state.pending_restart_declined = Some(reason);
            }
            deck_settings_v1::Event::Preempted { active } => {
                state.pending_preempted = Some(active != 0);
            }
            other => tracing::debug!(?other, "unhandled deck_settings_v1 event"),
        }
    }
}

impl Dispatch<DeckAlarmV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &DeckAlarmV1,
        event: deck_alarm_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            deck_alarm_v1::Event::AlarmRinging {
                time,
                period,
                label,
                snooze_allowed,
            } => {
                // Unknown enum values default to no-snooze, the safe fallback.
                let snooze_allowed = matches!(snooze_allowed, WEnum::Value(Snooze::Allowed));
                state.note_alarm_ring(time, period, label, snooze_allowed);
            }
            deck_alarm_v1::Event::AlarmStopped => {
                state.note_alarm_stop();
            }
            other => tracing::debug!(?other, "unhandled deck_alarm_v1 event"),
        }
    }
}

impl Dispatch<DeckUpgradeV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &DeckUpgradeV1,
        event: deck_upgrade_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.on_upgrade_event(&event);
    }
}

fn device_state_from_wire(state: WireDeviceState) -> Option<crate::overlay::DeviceState> {
    match state {
        WireDeviceState::FactoryDefault => Some(crate::overlay::DeviceState::FactoryDefault),
        WireDeviceState::SetupPending => Some(crate::overlay::DeviceState::SetupPending),
        WireDeviceState::Operational => Some(crate::overlay::DeviceState::Operational),
        WireDeviceState::WifiReconfiguration => {
            Some(crate::overlay::DeviceState::WifiReconfiguration)
        }
        other => {
            tracing::warn!(?other, "unknown device_state");
            None
        }
    }
}

fn setup_step_from_wire(state: WireSetupState) -> Option<crate::overlay::SetupStep> {
    match state {
        WireSetupState::Idle => Some(crate::overlay::SetupStep::Idle),
        WireSetupState::ConnectingToWifi => Some(crate::overlay::SetupStep::ConnectingToWifi),
        WireSetupState::WifiConnectionSuccess => {
            Some(crate::overlay::SetupStep::WifiConnectionSuccess)
        }
        WireSetupState::WifiConnectionFailed => {
            Some(crate::overlay::SetupStep::WifiConnectionFailed)
        }
        WireSetupState::WifiReconfigSuccess => Some(crate::overlay::SetupStep::WifiReconfigSuccess),
        WireSetupState::DeviceSetupSuccess => Some(crate::overlay::SetupStep::DeviceSetupSuccess),
        WireSetupState::UnexpectedError => {
            Some(crate::overlay::SetupStep::UnexpectedError { restarting: false })
        }
        WireSetupState::UnexpectedErrorRestarting => {
            Some(crate::overlay::SetupStep::UnexpectedError { restarting: true })
        }
        other => {
            tracing::warn!(?other, "unknown setup_progress state");
            None
        }
    }
}

impl Dispatch<DeckDeviceInfoV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &DeckDeviceInfoV1,
        event: deck_device_info_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Unknown enum values are dropped, keeping the last coherent value —
        // a newer compositor must not push this client into a guessed state.
        match event {
            deck_device_info_v1::Event::DeviceState {
                state: wire,
                boot_flow_delivered,
            } => {
                if let WEnum::Value(v) = wire
                    && let Some(v) = device_state_from_wire(v)
                {
                    state.pending_device_lifecycle = Some((v, boot_flow_delivered != 0));
                }
            }
            deck_device_info_v1::Event::SetupProgress {
                state: wire,
                wifi_ssid,
            } => {
                if let WEnum::Value(v) = wire
                    && let Some(step) = setup_step_from_wire(v)
                {
                    state.pending_setup_progress = Some((step, wifi_ssid));
                }
            }
            deck_device_info_v1::Event::AccessPoint { ssid, setup_url } => {
                let ap =
                    (!ssid.is_empty()).then_some(crate::overlay::AccessPoint { ssid, setup_url });
                state.pending_access_point = Some(ap);
            }
            other => tracing::debug!(?other, "unhandled deck_device_info_v1 event"),
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            record_released_buffer(
                &state.buffer_slots,
                &mut state.released_buffers,
                buffer.id(),
            );
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        callback: &wl_callback::WlCallback,
        event: wl_callback::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.mark_fence_done(&callback.id());
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_region::WlRegion, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        _: zwp_linux_buffer_params_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            let has_touch = caps.contains(wl_seat::Capability::Touch);
            if has_touch && state.touch.is_none() {
                state.touch = Some(seat.get_touch(qh, ()));
            } else if !has_touch && let Some(touch) = state.touch.take() {
                touch.release();
            }
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: wl_touch::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_touch::Event::Down { id, x, y, .. } => {
                state
                    .pending_touch
                    .push(crate::overlay::TouchEvent::Down { id, x, y });
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                state
                    .pending_touch
                    .push(crate::overlay::TouchEvent::Motion { id, x, y });
            }
            wl_touch::Event::Up { id, .. } => {
                state
                    .pending_touch
                    .push(crate::overlay::TouchEvent::Up { id });
            }
            wl_touch::Event::Cancel => {
                state.pending_touch.push(crate::overlay::TouchEvent::Cancel);
            }
            wl_touch::Event::Frame => tracing::trace!("wl_touch::Frame"),
            wl_touch::Event::Shape { .. } => tracing::trace!("wl_touch::Shape (ignored)"),
            wl_touch::Event::Orientation { .. } => {
                tracing::trace!("wl_touch::Orientation (ignored)");
            }
            other => tracing::debug!(?other, "unhandled wl_touch event"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::deck_upgrade_v1::client::deck_upgrade_v1::{Event, Kind, Phase};

    fn running_snapshot(
        kind: crate::overlay::UpgradeKind,
        phase: Option<crate::overlay::UpgradePhase>,
        progress: Option<crate::overlay::DownloadProgress>,
    ) -> crate::overlay::UpgradeSnapshot {
        crate::overlay::UpgradeSnapshot {
            kind,
            state: crate::overlay::UpgradeState::Running { phase, progress },
        }
    }

    fn started(kind: Kind) -> Event {
        Event::Started {
            kind: WEnum::Value(kind),
        }
    }

    fn phase(phase: Phase) -> Event {
        Event::Phase {
            phase: WEnum::Value(phase),
        }
    }

    #[test]
    fn invalid_upgrade_candidate_preserves_pending_valid_snapshot() {
        let valid = running_snapshot(
            crate::overlay::UpgradeKind::Packages,
            Some(crate::overlay::UpgradePhase::PackageRealizing),
            None,
        );
        let mut state = State::default();
        state.on_upgrade_event(&started(Kind::Packages));
        state.on_upgrade_event(&phase(Phase::PackageRealizing));
        state.on_upgrade_event(&Event::SnapshotDone);

        state.on_upgrade_event(&started(Kind::Firmware));
        state.on_upgrade_event(&Event::Phase {
            phase: WEnum::Unknown(99),
        });
        state.on_upgrade_event(&Event::SnapshotDone);

        assert_eq!(state.pending_upgrade_snapshot, Some(valid));
    }

    #[derive(Default)]
    struct UpgradeObserver {
        snapshot: Option<crate::overlay::UpgradeSnapshot>,
        calls: Vec<&'static str>,
    }

    impl crate::overlay::SystemOverlay for UpgradeObserver {
        fn layer_config(&self) -> crate::overlay::LayerConfig {
            crate::overlay::LayerConfig::fullscreen("upgrade-observer")
        }

        fn tick(&mut self, _now: Instant) -> crate::overlay::TickOutcome {
            self.calls.push("tick");
            crate::overlay::TickOutcome::default()
        }

        fn render(
            &mut self,
            _renderer: &mut dyn bmc_render::renderer::Renderer,
            _size: (u32, u32),
        ) {
        }

        fn uses_upgrade(&self) -> bool {
            true
        }

        fn on_upgrade_state(&mut self, snapshot: crate::overlay::UpgradeSnapshot) {
            self.calls.push("upgrade");
            self.snapshot = Some(snapshot);
        }
    }

    #[test]
    fn malformed_sequences_preserve_the_snapshot_delivered_before_tick() {
        let valid = running_snapshot(
            crate::overlay::UpgradeKind::Packages,
            Some(crate::overlay::UpgradePhase::PackageRealizing),
            None,
        );
        let mut state = State::default();
        state.on_upgrade_event(&started(Kind::Packages));
        state.on_upgrade_event(&phase(Phase::PackageRealizing));
        state.on_upgrade_event(&Event::SnapshotDone);

        state.on_upgrade_event(&Event::Started {
            kind: WEnum::Unknown(99),
        });
        state.on_upgrade_event(&Event::SnapshotDone);

        state.on_upgrade_event(&started(Kind::Packages));
        state.on_upgrade_event(&Event::DownloadProgress {
            downloaded_bytes_hi: 0,
            downloaded_bytes_lo: 1,
        });
        state.on_upgrade_event(&phase(Phase::PackageVerifying));
        state.on_upgrade_event(&Event::SnapshotDone);

        state.on_upgrade_event(&started(Kind::Firmware));
        state.on_upgrade_event(&Event::Phase {
            phase: WEnum::Unknown(99),
        });
        state.on_upgrade_event(&Event::SnapshotDone);

        let mut observer = UpgradeObserver::default();
        let _ = crate::overlay::deliver_upgrade_snapshot_and_tick(
            &mut observer,
            state.pending_upgrade_snapshot.take(),
            Instant::now(),
        );

        assert_eq!(observer.snapshot, Some(valid));
        assert_eq!(observer.calls, vec!["upgrade", "tick"]);
    }

    #[test]
    fn latest_completed_upgrade_snapshot_wins_before_take() {
        let mut state = State::default();
        state.on_upgrade_event(&started(Kind::Packages));
        state.on_upgrade_event(&Event::SnapshotDone);
        state.on_upgrade_event(&started(Kind::Firmware));
        state.on_upgrade_event(&Event::Succeeded { remaining_ms: 500 });
        state.on_upgrade_event(&Event::SnapshotDone);

        assert_eq!(
            state.pending_upgrade_snapshot,
            Some(crate::overlay::UpgradeSnapshot {
                kind: crate::overlay::UpgradeKind::Firmware,
                state: crate::overlay::UpgradeState::Succeeded {
                    remaining: Duration::from_millis(500),
                },
            })
        );
    }

    #[test]
    fn screen_edge_hidden_clears_pending_reveal() {
        let mut state = State::default();

        state.mark_screen_edge_revealed();
        state.mark_screen_edge_hidden();

        assert!(!state.pending_reveal);
        assert!(state.pending_hidden);
    }

    #[test]
    fn unmap_configure_does_not_resize_reusable_surface() {
        let mut state = State {
            configured: true,
            configured_size: (1, 200),
            pending_size_change: Some((1, 200)),
            needs_render: true,
            ..State::default()
        };

        state.discard_unmap_configure((1280, 200));

        assert_eq!(state.configured_size, (1280, 200));
        assert_eq!(state.pending_size_change, None);
        assert!(!state.needs_render);
    }

    #[test]
    fn resize_unmap_configure_preserves_requested_surface_size() {
        let mut state = State {
            configured: true,
            configured_size: (1, 200),
            pending_size_change: Some((1, 200)),
            needs_render: true,
            ..State::default()
        };

        state.discard_resize_unmap_configure((1280, 240));

        assert_eq!(state.configured_size, (1280, 240));
        assert_eq!(state.pending_size_change, None);
        assert!(!state.needs_render);
    }

    #[test]
    fn stop_then_ring_in_one_round_keeps_the_ring() {
        let mut state = State::default();

        state.note_alarm_stop();
        state.note_alarm_ring(
            "07:30".to_owned(),
            String::new(),
            "Wake up".to_owned(),
            true,
        );

        assert_eq!(
            state.pending_alarm_event,
            Some(AlarmEvent::Ring {
                time: "07:30".to_owned(),
                period: String::new(),
                label: "Wake up".to_owned(),
                snooze_allowed: true,
            })
        );
    }

    #[test]
    fn ring_then_stop_in_one_round_keeps_the_stop() {
        let mut state = State::default();

        state.note_alarm_ring(
            "07:30".to_owned(),
            String::new(),
            "Wake up".to_owned(),
            true,
        );
        state.note_alarm_stop();

        assert_eq!(state.pending_alarm_event, Some(AlarmEvent::Stop));
    }
}
