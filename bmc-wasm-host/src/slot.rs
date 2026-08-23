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

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::renderer::Renderer;
use bmc_wasm_runtime::{
    DiskCache, LedEffect, LedRequest, LedScope, NetworkInfo, NextAlarm as RuntimeNextAlarm,
    RenderStatus, RuntimeConfig, SystemSnapshot, WasmWidgetRuntime,
};
use bmc_wasm_thin_protocol::{WIDGET_CACHE_BUCKET_MAX_BYTES, WIDGET_CACHE_DIR};
use bmc_widget::surface::{DeckWidgetSurfaceClient, ReleasedBuffer, WidgetEvent, WidgetSurface};
use bmc_widget_protocol::{
    ActionPayload, LedEffect as ProtoEffect, LedScope as ProtoScope, NextAlarm as WireNextAlarm,
    RgbColor, SettingUpdate,
};
use serde_json::{Map, Value};

// The wasm runtime allocates `LedRequestId`s on the guest's behalf and
// forwards them as `deck_widget_v1` actions over Wayland. The two crates
// don't share a type definition; this assert guarantees the all-stop
// sentinel value stays equal across the boundary, so a runtime stop
// emitted as `LedRequest::Stop { request_id: 0 }` lands on the protocol
// side as `stop_led { request_id: 0 }` and is interpreted as "cancel
// every outstanding request from this widget."
const _: () = assert!(
    bmc_wasm_runtime::LED_REQUEST_ID_ALL == bmc_widget_protocol::LED_REQUEST_ID_ALL,
    "bmc_wasm_runtime::LED_REQUEST_ID_ALL must equal bmc_widget_protocol::LED_REQUEST_ID_ALL",
);

use crate::host::SharedHost;
use crate::lifecycle::{
    LifecycleEgl, LifecycleHook, LifecycleState, LifecycleStateMachine, LifecycleSurface,
    SlotApplyCtx, frame_callback_enabled, lifecycle_hook, should_render,
};
use crate::module_cache::ModuleLease;
use crate::render_target::{EglRenderTarget, RenderTarget, RenderTargetFactory};

/// Per-slot inter-frame floor — caps a misbehaving widget that returns
/// `wants_next_frame() == true` every iteration at ~120 fps.
pub const MIN_INTER_FRAME: std::time::Duration = std::time::Duration::from_millis(8);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RenderGate {
    #[default]
    NotRenderable,
    Blocked,
    Renderable,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SlotRenderInputs {
    pub gate: RenderGate,
    pub surface_needs_render: bool,
    pub runtime_frame_due: bool,
    pub min_inter_frame_remaining: Option<Duration>,
}

#[must_use]
pub fn slot_needs_render_from_inputs(inputs: SlotRenderInputs) -> bool {
    if inputs.gate != RenderGate::Renderable {
        return false;
    }
    if !inputs.surface_needs_render && !inputs.runtime_frame_due {
        return false;
    }
    inputs
        .min_inter_frame_remaining
        .is_none_or(|remaining| remaining.is_zero())
}

/// Whether a dirty surface may drive a render in `state`.
///
/// Off-screen states (Prepared neighbours, Entering/Leaving during a swipe)
/// keep presenting their last committed buffer; the dirty flag holds
/// and the deferred render happens on the transition to Visible.
/// Two exceptions: no frame was committed since the render target was acquired
/// (the warm-up frame must paint), and the compositor explicitly demanded
/// a pre-transition frame via `transition_incoming`.
#[must_use]
pub fn dirty_render_allowed(
    state: LifecycleState,
    rendered_since_acquire: bool,
    forced_render_pending: bool,
) -> bool {
    matches!(state, LifecycleState::Visible) || !rendered_since_acquire || forced_render_pending
}

/// Project the connectivity snapshot onto the widget-visible `NetworkInfo`.
/// The snapshot also carries the Wi-Fi signal level, which widgets cannot observe;
/// comparing this projection instead of the snapshot version
/// keeps signal-only bumps from waking widgets at all.
#[must_use]
pub fn widget_network_info(snapshot: &bmc_system_overlay::Snapshot) -> NetworkInfo {
    NetworkInfo {
        ssid: snapshot.station_ssid.clone().unwrap_or_default(),
        ip: snapshot
            .ipv4
            .as_ref()
            .map(std::net::Ipv4Addr::to_string)
            .unwrap_or_default(),
    }
}

#[must_use]
pub fn duration_to_timeout_millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

#[must_use]
pub fn refresh_runtime_frame_due_at(
    existing_due_at: Option<Instant>,
    wants_next_frame: bool,
    next_frame_delay: Option<u32>,
    anchor: Instant,
) -> Option<Instant> {
    if !wants_next_frame {
        return None;
    }

    let candidate = match next_frame_delay {
        None => anchor,
        Some(delay_ms) => anchor + Duration::from_millis(delay_ms.into()),
    };
    Some(existing_due_at.map_or(candidate, |existing| existing.min(candidate)))
}

trait RendererAssetEvictor {
    fn evict_renderer_prefix(&mut self, prefix: &str) -> usize;
}

impl<T: Renderer + ?Sized> RendererAssetEvictor for T {
    fn evict_renderer_prefix(&mut self, prefix: &str) -> usize {
        Renderer::evict_prefix(self, prefix)
    }
}

fn evict_renderer_assets(renderer: &mut impl RendererAssetEvictor, asset_namespace: &str) -> usize {
    renderer.evict_renderer_prefix(asset_namespace)
}

#[derive(Debug)]
pub enum ControlSocketStatus {
    WouldBlock,
    PeerClosed,
    UnsolicitedByte(u8),
    Error(std::io::Error),
}

#[must_use]
pub fn classify_control_socket_read(
    result: std::io::Result<usize>,
    byte: u8,
) -> ControlSocketStatus {
    match result {
        Ok(0) => ControlSocketStatus::PeerClosed,
        Ok(_) => ControlSocketStatus::UnsolicitedByte(byte),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => ControlSocketStatus::WouldBlock,
        Err(e) => ControlSocketStatus::Error(e),
    }
}

/// The surface operations `WidgetSlot` uses beyond [`WidgetSurface`] —
/// the seam that lets tests drive slot logic over a stub surface.
pub trait SlotSurface: WidgetSurface + LifecycleSurface {
    fn request_action(&self, action: &ActionPayload) -> Result<()>;
    fn submit_buffer_with_wl_buffer(
        &self,
        info: &bmc_widget::egl::DmaBufInfo,
        buffer: &wayland_client::protocol::wl_buffer::WlBuffer,
        request_frame: bool,
    ) -> Result<()>;
    fn flush(&self) -> Result<()>;
    fn drain_released_buffers(&mut self) -> Vec<ReleasedBuffer>;
    fn fd(&self) -> std::os::fd::BorrowedFd<'_>;
    fn shutdown_connection(&self) -> Result<()> {
        use std::os::fd::AsRawFd as _;

        nix::sys::socket::shutdown(self.fd().as_raw_fd(), nix::sys::socket::Shutdown::Both)
            .context("shut down transferred Wayland connection")?;
        Ok(())
    }
}

impl SlotSurface for DeckWidgetSurfaceClient {
    fn request_action(&self, action: &ActionPayload) -> Result<()> {
        DeckWidgetSurfaceClient::request_action(self, action)
    }

    fn submit_buffer_with_wl_buffer(
        &self,
        info: &bmc_widget::egl::DmaBufInfo,
        buffer: &wayland_client::protocol::wl_buffer::WlBuffer,
        request_frame: bool,
    ) -> Result<()> {
        DeckWidgetSurfaceClient::submit_buffer_with_wl_buffer(self, info, buffer, request_frame)
    }

    fn flush(&self) -> Result<()> {
        DeckWidgetSurfaceClient::flush(self)
    }

    fn drain_released_buffers(&mut self) -> Vec<ReleasedBuffer> {
        DeckWidgetSurfaceClient::drain_released_buffers(self)
    }

    fn fd(&self) -> std::os::fd::BorrowedFd<'_> {
        DeckWidgetSurfaceClient::fd(self)
    }
}

#[expect(missing_debug_implementations)]
pub struct WidgetSlot<S = DeckWidgetSurfaceClient> {
    pub surface: S,
    pub runtime: WasmWidgetRuntime,
    // Declaration order drops the Store before its compiled-module lease.
    module_lease: Option<ModuleLease>,
    pub lifecycle: LifecycleStateMachine,
    pub render_target: Option<RenderTarget>,
    pub retired_render_targets: Vec<RenderTarget>,
    pub factory: Rc<dyn RenderTargetFactory>,
    pub last_render_at: Option<Instant>,
    pub monotonic_origin: Instant,
    pub frame_count: u64,
    pub initial_params: Map<String, Value>,
    /// Deck-wide system snapshot accumulated from `SettingUpdate` wayland
    /// events. Seeded from the compositor's initial configure batch,
    /// then mutated per-field by each subsequent setting delivery.
    ///
    /// After a drain that touched any setting, the latest snapshot is pushed
    /// to the runtime via `deliver_system_update`.
    pub pending_system: SystemSnapshot,
    /// Last connectivity snapshot version pushed to the runtime as `NetworkInfo`;
    /// version-gated so an unchanged network re-polls free.
    pub snapshot_version: Option<bmc_system_overlay::SnapshotVersion>,
    /// Last `NetworkInfo` pushed to the runtime.
    /// The snapshot version also bumps on fields widgets cannot observe
    /// (Wi-Fi signal level), so the delivery hook fires only when this projection changes.
    pub last_network_info: NetworkInfo,
    pub credentials: bmc_wasm_runtime::CredentialView,
    pub credential_secrets: bmc_widget_protocol::CredentialSecrets,
    pub peer_pid: Option<libc::pid_t>,
    pub wasm_basename: String,
    /// Asset-cache bucket token, published to the host's GC-root file so the
    /// cross-host cache GC keeps this bucket alive. `None` if no cache identity.
    pub cache_token: Option<String>,
    pub control_socket: UnixStream,
    pub next_frame_due_at: Option<Instant>,
    /// A frame was committed since the current render target was acquired.
    /// While set, a dirty surface drives a render only under [`dirty_render_allowed`];
    /// a fresh target's warm-up frame is always allowed.
    pub rendered_since_acquire: bool,
    /// The compositor demanded a pre-transition frame (`transition_incoming`);
    /// lets the next dirty render through regardless of lifecycle state.
    pub forced_render_pending: bool,
    /// One-time diagnostic latch: set after logging that touch events were
    /// dropped because the widget does not export `on_touch`.
    pub touch_drop_logged: bool,
    /// Receiver for LED requests the runtime emits on the guest's behalf;
    /// drained each loop iteration and forwarded as `deck_widget_v1` actions.
    led_rx: mpsc::Receiver<LedRequest>,
}

#[derive(Default)]
struct SlotIdentity {
    initial_params: Map<String, Value>,
    pending_system: SystemSnapshot,
    credentials: bmc_wasm_runtime::CredentialView,
    credential_secrets: bmc_widget_protocol::CredentialSecrets,
    peer_pid: Option<libc::pid_t>,
    wasm_basename: String,
    cache_token: Option<String>,
}

fn instantiate_cached_runtime(
    lease: ModuleLease,
    width: u32,
    height: u32,
    viewport_shape: bmc_wasm_protocol::ViewportShape,
    display: bmc_wasm_runtime::RuntimeDisplayInfo,
    initial_system_time: chrono::DateTime<chrono::FixedOffset>,
    config: RuntimeConfig,
) -> Result<(WasmWidgetRuntime, ModuleLease)> {
    let runtime = WasmWidgetRuntime::from_module(
        lease.module(),
        width,
        height,
        viewport_shape,
        display,
        initial_system_time,
        config,
    )?;
    Ok((runtime, lease))
}

fn release_runtime_and_module(runtime: WasmWidgetRuntime, module_lease: &mut Option<ModuleLease>) {
    drop(runtime);
    drop(module_lease.take());
}

impl WidgetSlot {
    pub(crate) fn from_handshake(
        wasm_path: &Path,
        asset_root: Option<&Path>,
        shared: &SharedHost,
        wayland_fd: std::os::fd::OwnedFd,
        control_socket: UnixStream,
        peer_pid: Option<libc::pid_t>,
        factory: Rc<dyn RenderTargetFactory>,
    ) -> Result<Self> {
        tracing::info!(
            ?peer_pid,
            wasm = %wasm_path.display(),
            "connecting widget Wayland fd"
        );
        let (surface, initial) = DeckWidgetSurfaceClient::connect_with_fd(wayland_fd)
            .context("DeckWidgetSurfaceClient::connect_with_fd")?;
        tracing::info!(
            ?peer_pid,
            wasm = %wasm_path.display(),
            w = initial.width,
            h = initial.height,
            params = initial.params.len(),
            settings = initial.settings.len(),
            "widget Wayland configure received"
        );
        let module_load = shared.module_cache.load(wasm_path)?;
        let digest = module_load.lease.digest();
        tracing::info!(
            ?peer_pid,
            wasm = %wasm_path.display(),
            bytes = module_load.byte_len,
            digest = %format_args!(
                "{:02x}{:02x}{:02x}{:02x}",
                digest[0], digest[1], digest[2], digest[3]
            ),
            outcome = ?module_load.outcome,
            "wasm module loaded"
        );
        // Seed the system snapshot from any per-field setting events
        // the compositor  included in the initial configure batch.
        //
        // The runtime sees the same bytes via `RuntimeConfig::system`
        // before `init` runs, so the widget's first frame observes
        // operator values.
        let mut pending_system = SystemSnapshot::default();
        for setting in &initial.settings {
            apply_setting_update(&mut pending_system, setting);
        }
        let credentials = bmc_wasm_runtime::parse_credentials_json(&initial.credentials);
        let credential_secrets = initial.credential_secrets.clone();
        let viewport_shape = bmc_wasm_protocol::ViewportShape::from(initial.viewport_shape);
        let display = bmc_wasm_runtime::RuntimeDisplayInfo::from(initial.display);
        let token = initial.token.clone();
        let (led_tx, led_rx) = mpsc::channel();
        let (runtime, module_lease) = instantiate_cached_runtime(
            module_load.lease,
            initial.width,
            initial.height,
            viewport_shape,
            display,
            chrono::Local::now().fixed_offset(),
            RuntimeConfig {
                params: bmc_wasm_runtime::parse_params_json(&initial.params).unwrap_or_else(
                    |err| {
                        tracing::warn!(?err, "invalid initial params — empty map");
                        BTreeMap::default()
                    },
                ),
                system: pending_system.clone(),
                credentials: credentials.clone(),
                credential_secrets: credential_secrets.clone(),
                led_request_sender: Some(led_tx),
                image_decode_lock_path: Some(shared.image_decode_lock_path.clone()),
                asset_cache: Some(DiskCache::new(
                    PathBuf::from(WIDGET_CACHE_DIR).join(&token),
                    WIDGET_CACHE_BUCKET_MAX_BYTES,
                )),
                package_assets: asset_root.map(bmc_wasm_runtime::PackageAssetStore::new),
                instance_token: Some(token.clone()),
                ..RuntimeConfig::default()
            },
        )?;
        tracing::info!(
            ?peer_pid,
            wasm = %wasm_path.display(),
            w = initial.width,
            h = initial.height,
            "wasm runtime initialized; waiting for lifecycle event"
        );

        let identity = SlotIdentity {
            initial_params: initial.params,
            pending_system,
            credentials,
            credential_secrets,
            peer_pid,
            wasm_basename: wasm_path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            cache_token: Some(token),
        };
        Ok(WidgetSlot::from_parts_with_identity(
            surface,
            runtime,
            factory,
            control_socket,
            led_rx,
            identity,
            Some(module_lease),
        ))
    }
}

impl<S: SlotSurface> WidgetSlot<S> {
    /// A slot over an injected surface and runtime,
    /// everything else at its initial state.
    pub fn from_parts(
        surface: S,
        runtime: WasmWidgetRuntime,
        factory: Rc<dyn RenderTargetFactory>,
        control_socket: UnixStream,
        led_rx: mpsc::Receiver<LedRequest>,
    ) -> Self {
        Self::from_parts_with_identity(
            surface,
            runtime,
            factory,
            control_socket,
            led_rx,
            SlotIdentity::default(),
            None,
        )
    }

    fn from_parts_with_identity(
        surface: S,
        mut runtime: WasmWidgetRuntime,
        factory: Rc<dyn RenderTargetFactory>,
        control_socket: UnixStream,
        led_rx: mpsc::Receiver<LedRequest>,
        identity: SlotIdentity,
        module_lease: Option<ModuleLease>,
    ) -> Self {
        runtime.initialize_dormant();
        Self {
            surface,
            runtime,
            module_lease,
            lifecycle: LifecycleStateMachine::new(),
            render_target: None,
            retired_render_targets: Vec::new(),
            factory,
            last_render_at: None,
            monotonic_origin: Instant::now(),
            frame_count: 0,
            initial_params: identity.initial_params,
            pending_system: identity.pending_system,
            snapshot_version: None,
            last_network_info: NetworkInfo::default(),
            credentials: identity.credentials,
            credential_secrets: identity.credential_secrets,
            peer_pid: identity.peer_pid,
            wasm_basename: identity.wasm_basename,
            cache_token: identity.cache_token,
            control_socket,
            next_frame_due_at: None,
            rendered_since_acquire: false,
            forced_render_pending: false,
            touch_drop_logged: false,
            led_rx,
        }
    }

    /// Drain LED requests from the runtime and forward each one as a
    /// `deck_widget_v1` action on this slot's surface.
    pub fn flush_led_requests(&mut self) -> Result<()> {
        while let Ok(req) = self.led_rx.try_recv() {
            let action = led_request_to_action(&req);
            self.surface.request_action(&action)?;
        }
        Ok(())
    }

    /// Poll the shared OS connectivity prober, then push the Deck's SSID + IP
    /// to the runtime as `NetworkInfo` so a widget can show how to reach the Deck.
    /// Version-gated like the overlays' `refresh_network`, with the overlays' inner
    /// comparison: the version also bumps on signal-level jitter, which widgets
    /// cannot observe, so only a changed projection reaches the runtime.
    /// The widget decides whether to re-render by calling `request_frame()`
    /// from `on_network_update` — the host never marks the surface dirty
    /// for network changes.
    pub fn refresh_network(&mut self) {
        self.refresh_network_from(bmc_system_overlay::snapshot_if_changed);
    }

    /// [`refresh_network`](Self::refresh_network) with the prober injected,
    /// so tests can drive this path with fake snapshots.
    pub fn refresh_network_from(
        &mut self,
        snapshot_if_changed: impl FnOnce(
            Option<bmc_system_overlay::SnapshotVersion>,
        ) -> Option<bmc_system_overlay::VersionedSnapshot>,
    ) {
        let Some(bmc_system_overlay::VersionedSnapshot { version, snapshot }) =
            snapshot_if_changed(self.snapshot_version)
        else {
            return;
        };
        self.snapshot_version = Some(version);
        let info = widget_network_info(&snapshot);
        if info == self.last_network_info {
            return;
        }
        self.last_network_info = info.clone();
        self.runtime.set_network_info(info);
        self.runtime.deliver_network_update();
    }

    pub fn dispatch_wayland_events(&mut self) -> Result<()> {
        self.surface.poll_dispatch(0)?;
        // The compositor fans a single localization save into one per-field setting
        // event per setting; per-event delivery would invoke `on_system_update`
        // N times for one operator action.
        //
        // Collect Setting events into `pending_system` during the drain
        // and push a single `deliver_system_update` after the loop.
        // Similarly, every `ParamUpdate` carries the full params set per the
        // compositor↔widget contract; multiple updates in one drain collapse
        // to the latest event — never merged.
        let mut system_dirty = false;
        let mut latest_params: Option<serde_json::Map<String, Value>> = None;
        let mut latest_credentials: Option<serde_json::Map<String, Value>> = None;
        let mut latest_secrets: Option<bmc_widget_protocol::CredentialSecrets> = None;
        // Touch is coalesced like Setting/ParamUpdate: every event is queued for
        // the next render during the drain, and `on_touch` fires once afterwards.
        // A widget without an `on_touch` export is non-interactive — the host no
        // longer force-renders on touch, so its events would queue for a render
        // that is never requested. Drop them at the source.
        let mut touch_dirty = false;
        let mut touch_dropped = false;
        let accepts_touch = self.runtime.exports_on_touch();
        for event in WidgetSurface::drain_events(&mut self.surface) {
            let is_touch = matches!(
                event,
                WidgetEvent::TouchDown { .. }
                    | WidgetEvent::TouchMotion { .. }
                    | WidgetEvent::TouchUp { .. }
                    | WidgetEvent::TouchCancel
            );
            if is_touch && !accepts_touch {
                touch_dropped = true;
                continue;
            }
            match event {
                WidgetEvent::Setting(setting) => {
                    apply_setting_update(&mut self.pending_system, &setting);
                    system_dirty = true;
                }
                WidgetEvent::ParamUpdate(params) => latest_params = Some(params),
                WidgetEvent::TouchDown { x, y, .. } => {
                    let (x, y) = wayland_touch_xy(x, y);
                    self.runtime
                        .push_touch_event(bmc_render::interaction::TouchEvent::Down { x, y });
                    touch_dirty = true;
                }
                WidgetEvent::TouchMotion { x, y, .. } => {
                    let (x, y) = wayland_touch_xy(x, y);
                    self.runtime
                        .push_touch_event(bmc_render::interaction::TouchEvent::Move { x, y });
                    touch_dirty = true;
                }
                WidgetEvent::TouchUp { .. } => {
                    self.runtime
                        .push_touch_event(bmc_render::interaction::TouchEvent::Up);
                    touch_dirty = true;
                }
                WidgetEvent::TouchCancel => {
                    self.runtime
                        .push_touch_event(bmc_render::interaction::TouchEvent::Cancel);
                    touch_dirty = true;
                }
                WidgetEvent::CredentialsUpdate(view) => latest_credentials = Some(view),
                WidgetEvent::SecretsUpdate(secrets) => latest_secrets = Some(secrets),
                event @ (WidgetEvent::Lifecycle(_)
                | WidgetEvent::TransitionIncoming
                | WidgetEvent::Shutdown) => {
                    self.on_wayland_event(&event);
                }
            }
        }
        self.deliver_coalesced_snapshots(system_dirty, latest_params);
        self.deliver_credentials(latest_credentials, latest_secrets);
        if touch_dirty {
            // No `mark_needs_render` here: the widget decides whether to re-render
            // by calling `request_frame()` from `on_touch`, which the main loop's
            // `refresh_next_runtime_frame_after_delivery` picks up.
            self.runtime.deliver_touch();
        }
        if touch_dropped {
            self.log_dropped_touch_once();
        }
        let released_buffers = self.surface.drain_released_buffers();
        if let Some(egl_target) = self
            .render_target
            .as_mut()
            .and_then(RenderTarget::as_egl_mut)
        {
            for released in &released_buffers {
                egl_target.mark_released_buffer(released);
            }
        }
        for target in &mut self.retired_render_targets {
            let Some(egl_target) = target.as_egl_mut() else {
                continue;
            };
            for released in &released_buffers {
                egl_target.mark_released_buffer(released);
            }
        }
        Ok(())
    }

    fn log_dropped_touch_once(&mut self) {
        if self.touch_drop_logged {
            return;
        }
        self.touch_drop_logged = true;
        tracing::debug!(
            peer_pid = ?self.peer_pid,
            wasm = %self.wasm_basename,
            "dropping touch events: widget does not export on_touch"
        );
    }

    pub fn reclaim_retired_render_targets(&mut self, shared: &SharedHost) {
        let mut pending = Vec::new();
        for mut target in self.retired_render_targets.drain(..) {
            match self
                .factory
                .destroy_released_slots(&mut target, &shared.egl, &mut self.surface)
            {
                crate::render_target::RenderTargetCleanup::Complete => {}
                crate::render_target::RenderTargetCleanup::PendingRelease => {
                    pending.push(target);
                }
            }
        }
        self.retired_render_targets = pending;
    }

    #[must_use]
    pub fn has_retired_render_target_cleanup(&self) -> bool {
        self.retired_render_targets.iter().any(|target| {
            target
                .as_egl()
                .is_some_and(EglRenderTarget::has_released_slot_cleanup)
        })
    }

    #[must_use]
    pub fn has_lifecycle_gpu_work(&self, now: Instant) -> bool {
        self.lifecycle.render_target_change_ready(now)
            || ((self.lifecycle.current() == LifecycleState::Prepared
                || self.lifecycle.target() == LifecycleState::Prepared)
                && self.render_target.as_ref().is_some_and(|target| {
                    target
                        .as_egl()
                        .is_some_and(EglRenderTarget::has_prepared_compaction_work)
                }))
    }

    pub fn dispatch_control_socket(&mut self) -> Result<()> {
        use std::io::Read;
        let mut buf = [0_u8; 1];
        let result = (&self.control_socket).read(&mut buf);
        match classify_control_socket_read(result, buf[0]) {
            ControlSocketStatus::WouldBlock => Ok(()),
            ControlSocketStatus::PeerClosed => Err(anyhow::anyhow!("control socket EOF")),
            ControlSocketStatus::UnsolicitedByte(b) => Err(anyhow::anyhow!(
                "unsolicited byte on control socket (protocol violation): {b:#04x}"
            )),
            ControlSocketStatus::Error(e) => Err(anyhow::Error::from(e)),
        }
    }

    pub fn shutdown_wayland_connection(&self) -> Result<()> {
        self.surface.shutdown_connection()
    }

    #[must_use]
    pub fn is_renderable(&self) -> bool {
        should_render(self.lifecycle.current())
    }

    #[must_use]
    pub fn is_blocked(&self) -> bool {
        self.lifecycle.blocked()
    }

    #[must_use]
    pub fn render_buffer_available(&self) -> bool {
        self.render_target
            .as_ref()
            .and_then(RenderTarget::as_egl)
            .is_none_or(EglRenderTarget::current_slot_available)
    }

    #[must_use]
    pub fn surface_needs_render(&self) -> bool {
        self.surface.needs_render()
    }

    #[must_use]
    pub fn runtime_frame_due(&self, now: Instant) -> bool {
        self.next_frame_due_at.is_some_and(|due| now >= due)
    }

    #[must_use]
    pub fn runtime_frame_due_in(&self, now: Instant) -> Option<Duration> {
        self.next_frame_due_at
            .map(|due| due.saturating_duration_since(now))
    }

    #[must_use]
    pub fn frame_callback_enabled(&self) -> bool {
        frame_callback_enabled(self.lifecycle.current())
    }

    /// Spec § 7 / BDK-437 § States table: a fresh render target paints its
    /// warm-up frame **once**; after that a dirty surface renders only under
    /// [`dirty_render_allowed`] (Visible, or a compositor-demanded pre-transition
    /// frame), so off-screen slots keep presenting their last buffer.
    /// Visible widgets run the full animation loop;
    /// gate animation-driven renders on `frame_callback_enabled` so inactive slots
    /// whose runtime returns `wants_next_frame() == true` do NOT spin a render loop.
    #[must_use]
    pub fn needs_render(&self, now: Instant) -> bool {
        let gate = if !self.is_renderable() {
            RenderGate::NotRenderable
        } else if self.is_blocked() || !self.render_buffer_available() {
            RenderGate::Blocked
        } else {
            RenderGate::Renderable
        };
        let runtime_frame_due = self.frame_callback_enabled() && self.runtime_frame_due(now);
        slot_needs_render_from_inputs(SlotRenderInputs {
            gate,
            surface_needs_render: self.effective_surface_needs_render(),
            runtime_frame_due,
            min_inter_frame_remaining: self.min_inter_frame_remaining(now),
        })
    }

    /// The surface dirty flag, masked by [`dirty_render_allowed`].
    /// A held-back flag is not cleared — the deferred render happens
    /// once the state allows it again (typically on the transition to Visible).
    #[must_use]
    fn effective_surface_needs_render(&self) -> bool {
        self.surface_needs_render()
            && dirty_render_allowed(
                self.lifecycle.current(),
                self.rendered_since_acquire,
                self.forced_render_pending,
            )
    }

    #[must_use]
    pub fn has_min_inter_frame_elapsed(&self, now: Instant) -> bool {
        match self.last_render_at {
            None => true,
            Some(t) => now.duration_since(t) >= MIN_INTER_FRAME,
        }
    }

    #[must_use]
    pub fn min_inter_frame_remaining(&self, now: Instant) -> Option<std::time::Duration> {
        let t = self.last_render_at?;
        let elapsed = now.duration_since(t);
        MIN_INTER_FRAME.checked_sub(elapsed)
    }

    #[must_use]
    pub fn retry_in(&self, now: Instant) -> Option<std::time::Duration> {
        let t = self.lifecycle.retry_at()?;
        Some(t.saturating_duration_since(now))
    }

    #[must_use]
    pub fn poll_inputs(&self, now: Instant) -> crate::main_loop::SlotPollInputs {
        crate::main_loop::SlotPollInputs {
            retry_in: self.retry_in(now),
            is_renderable: self.is_renderable(),
            is_blocked: self.is_blocked() || !self.render_buffer_available(),
            frame_callback_enabled: self.frame_callback_enabled(),
            animation_wants_immediate: self.runtime_frame_due(now),
            // The masked flag, not the raw one — a held-back dirty surface
            // must not wake the poll loop for renders `needs_render` will refuse.
            surface_needs_render: self.effective_surface_needs_render(),
            min_inter_frame_remaining: self.min_inter_frame_remaining(now),
            next_frame_delay: self
                .runtime_frame_due_in(now)
                .map(duration_to_timeout_millis),
            has_pending_io: self.runtime.has_pending_io(),
        }
    }

    /// Schedule the next runtime frame relative to `anchor` — the instant
    /// the just-rendered frame *started*, not the instant it finished.
    ///
    /// Anchoring to the start makes the widget's requested delay
    /// the true frame period; render time is absorbed into
    /// the interval instead of stacking on top.
    pub fn schedule_next_runtime_frame(&mut self, anchor: Instant) {
        self.next_frame_due_at = if self.runtime.wants_next_frame() {
            Some(match self.runtime.next_frame_delay() {
                None => anchor,
                Some(delay_ms) => anchor + Duration::from_millis(delay_ms.into()),
            })
        } else {
            None
        };
    }

    pub fn refresh_next_runtime_frame_after_delivery(&mut self, anchor: Instant) {
        self.next_frame_due_at = refresh_runtime_frame_due_at(
            self.next_frame_due_at,
            self.runtime.wants_next_frame(),
            self.runtime.next_frame_delay(),
            anchor,
        );
    }

    pub fn tick_delta(&mut self, now: Instant) -> u32 {
        let delta = self.last_render_at.map_or(0, |t| {
            u32::try_from(now.duration_since(t).as_millis()).unwrap_or(u32::MAX)
        });
        self.last_render_at = Some(now);
        delta
    }

    #[must_use]
    pub fn monotonic_ms(&self, now: Instant) -> u64 {
        u64::try_from(now.duration_since(self.monotonic_origin).as_millis()).unwrap_or(u64::MAX)
    }

    pub fn advance_runtime_time(
        &mut self,
        system_time: chrono::DateTime<chrono::FixedOffset>,
        now: Instant,
    ) {
        self.runtime.set_time(system_time, self.monotonic_ms(now));
    }

    pub fn apply_lifecycle(&mut self, now: Instant, egl: &dyn LifecycleEgl) {
        let previous = self.lifecycle.current();
        let had_render_target = self.render_target.is_some();
        let w = self.surface.width();
        let h = self.surface.height();
        let mut ctx = SlotApplyCtx {
            factory: &self.factory,
            egl,
            surface: &mut self.surface,
            render_target: &mut self.render_target,
            retired_render_targets: &mut self.retired_render_targets,
            width: w,
            height: h,
        };
        self.lifecycle.apply(&mut ctx, now);
        if !had_render_target && self.render_target.is_some() {
            self.rendered_since_acquire = false;
        }
        let current = self.lifecycle.current();
        if previous != current {
            tracing::debug!(
                peer_pid = ?self.peer_pid,
                wasm = %self.wasm_basename,
                ?previous,
                ?current,
                blocked = self.lifecycle.blocked(),
                render_target = self.render_target.is_some(),
                "slot lifecycle applied"
            );
            match lifecycle_hook(previous, current) {
                Some(LifecycleHook::Wake) => {
                    self.runtime.notify_wake();
                }
                Some(LifecycleHook::Sleep) => {
                    self.runtime.notify_dormant();
                }
                None => {}
            }
        }
    }

    pub fn render(
        &mut self,
        ptr: NonNull<dyn Renderer>,
        delta_ms: u32,
        shared: &mut SharedHost,
    ) -> Result<RenderStatus> {
        let _ = self.surface.take_render_requested();
        let frame_start_instant = Instant::now();
        let frame_start = HostRenderProfiling::start_phase();
        let (dmabuf, slot_idx, status, target_width, target_height) = {
            let frame_setup_phase = HostRenderProfiling::start_phase();
            let target = self.render_target.as_mut().expect(
                "BUG: render() called on a slot without a render target — \
                 needs_render() should have gated this off when lifecycle ∉ {Prepared, Entering, Visible, Leaving}",
            );
            let target_width = target.width;
            let target_height = target.height;
            let egl_target = target.as_egl_mut().expect(
                "BUG: EglRenderTargetFactory allocated all WidgetSlot render targets in Task 8",
            );
            let wasm_basename = self.wasm_basename.as_str();
            let runtime = &mut self.runtime;
            let gl_wait_phase = std::cell::Cell::new(HostRenderProfiling::start_phase());

            let status = stage_frame_under_gpu_lock(
                shared,
                "host_widget_render",
                target_width,
                target_height,
                |shared| {
                    egl_target.buffers.ensure_current(&shared.egl)?;

                    unsafe { ptr.as_ptr().as_mut() }
                        .expect(
                            "BUG: renderer pointer was NonNull when stored, \
                             raw-pointer reborrow must produce a non-null reference",
                        )
                        .begin_frame(target_width, target_height, 1.0);
                    HostRenderProfiling::log_phase(wasm_basename, "frame_setup", frame_setup_phase);

                    let phase_start = HostRenderProfiling::start_phase();
                    let status = runtime.with_renderer(ptr, |rt| rt.render(delta_ms))?;
                    HostRenderProfiling::log_phase(wasm_basename, "runtime_render", phase_start);

                    let phase_start = HostRenderProfiling::start_phase();
                    unsafe { ptr.as_ptr().as_mut() }
                        .expect(
                            "BUG: renderer pointer was NonNull when stored, \
                             raw-pointer reborrow must produce a non-null reference",
                        )
                        .flush();
                    HostRenderProfiling::log_phase(wasm_basename, "femtovg_flush", phase_start);

                    let current_export = egl_target.buffers.current_ref().expect(
                        "BUG: ensure_current succeeded above, so DoubleBufferState::current_ref \
                         must return Some; an internal invariant of DoubleBufferState was violated",
                    );
                    let phase_start = HostRenderProfiling::start_phase();
                    shared.blit_staging_to(current_export.fbo, target_width, target_height);
                    HostRenderProfiling::log_phase(wasm_basename, "staging_blit", phase_start);

                    gl_wait_phase.set(HostRenderProfiling::start_phase());
                    Ok(status)
                },
                || HostRenderProfiling::log_phase(wasm_basename, "gl_wait", gl_wait_phase.get()),
            )?;

            let phase_start = HostRenderProfiling::start_phase();
            let (dmabuf, slot_idx) = egl_target.buffers.export_and_swap()?;
            HostRenderProfiling::log_phase(wasm_basename, "export_and_swap", phase_start);
            (dmabuf, slot_idx, status, target_width, target_height)
        };

        let wants_immediate = self.frame_callback_enabled()
            && self.runtime.wants_next_frame()
            && self.runtime.next_frame_delay().is_none();
        self.submit_exported_buffer(&dmabuf, slot_idx, wants_immediate)?;
        HostRenderProfiling::log_frame(
            &self.wasm_basename,
            self.frame_count + 1,
            delta_ms,
            frame_start,
            HostRenderFrameContext::new(target_width, target_height, wants_immediate, status),
        );
        self.schedule_next_runtime_frame(frame_start_instant);
        self.rendered_since_acquire = true;
        self.forced_render_pending = false;
        self.frame_count += 1;
        tracing::debug!(
            peer_pid = ?self.peer_pid,
            wasm = %self.wasm_basename,
            frame = self.frame_count,
            delta_ms,
            ?status,
            wants_immediate,
            "widget frame submitted"
        );
        Ok(status)
    }

    fn submit_exported_buffer(
        &mut self,
        dmabuf: &bmc_widget::egl::DmaBufInfo,
        slot_idx: usize,
        wants_immediate: bool,
    ) -> Result<()> {
        let target = self
            .render_target
            .as_mut()
            .expect("BUG: render target must still be present after export_and_swap");
        let egl_target = target.as_egl_mut().expect(
            "BUG: EglRenderTargetFactory allocated all WidgetSlot render targets in Task 8",
        );
        let phase_start = HostRenderProfiling::start_phase();
        let wl_buffer = egl_target
            .wl_buffer_for_slot(&mut self.surface, dmabuf, slot_idx)
            .map_err(anyhow::Error::msg)?;
        self.surface
            .submit_buffer_with_wl_buffer(dmabuf, &wl_buffer, wants_immediate)?;
        HostRenderProfiling::log_phase(&self.wasm_basename, "wayland_attach_commit", phase_start);

        let phase_start = HostRenderProfiling::start_phase();
        self.surface.flush()?;
        HostRenderProfiling::log_phase(&self.wasm_basename, "wayland_flush", phase_start);
        egl_target.mark_presented(slot_idx);
        Ok(())
    }

    pub fn shutdown(
        self,
        shared: &mut SharedHost,
        renderer: &mut FemtoVgRenderer,
    ) -> anyhow::Result<()> {
        let gpu_render_lock = shared.acquire_gpu_render_lock("host_widget_shutdown")?;
        self.shutdown_with_gpu_access(shared, renderer);
        shared.flush_and_wait_gl();
        drop(gpu_render_lock);
        Ok(())
    }

    pub(crate) fn shutdown_with_gpu_access(
        mut self,
        shared: &mut SharedHost,
        renderer: &mut FemtoVgRenderer,
    ) {
        let asset_namespace = self.runtime.asset_namespace();
        release_runtime_and_module(self.runtime, &mut self.module_lease);

        let evicted_renderer_assets = evict_renderer_assets(renderer, &asset_namespace);
        if evicted_renderer_assets > 0 {
            tracing::debug!(
                peer_pid = ?self.peer_pid,
                wasm = %self.wasm_basename,
                asset_namespace = %asset_namespace,
                evicted_renderer_assets,
                "widget renderer assets evicted"
            );
        }

        if let Some(target) = self.render_target.take() {
            self.factory.destroy(target, &shared.egl, &mut self.surface);
        }
        for target in self.retired_render_targets.drain(..) {
            self.factory.destroy(target, &shared.egl, &mut self.surface);
        }
        drop(self.surface);
        drop(self.control_socket);
    }

    fn deliver_coalesced_snapshots(
        &mut self,
        system_dirty: bool,
        latest_params: Option<serde_json::Map<String, Value>>,
    ) {
        if system_dirty {
            tracing::info!(
                peer_pid = ?self.peer_pid,
                wasm = %self.wasm_basename,
                "settings drained from wayland — delivering system snapshot to runtime"
            );
            self.runtime
                .deliver_system_update(self.pending_system.clone());
        }
        if let Some(params) = latest_params
            && let Ok(table) = bmc_wasm_runtime::parse_params_json(&params)
        {
            tracing::info!(
                peer_pid = ?self.peer_pid,
                wasm = %self.wasm_basename,
                params = table.len(),
                "param update received"
            );
            self.runtime.deliver_params_update(table);
        }
    }

    /// Deliver a re-resolved credential set,
    /// keeping whichever half this drain did not carry.
    ///
    /// Nothing is logged but the slot count:
    /// the secret values stop here and must not reach a log line.
    fn deliver_credentials(
        &mut self,
        view: Option<serde_json::Map<String, Value>>,
        secrets: Option<bmc_widget_protocol::CredentialSecrets>,
    ) {
        if view.is_none() && secrets.is_none() {
            return;
        }
        if let Some(view) = view {
            self.credentials = bmc_wasm_runtime::parse_credentials_json(&view);
        }
        if let Some(secrets) = secrets {
            self.credential_secrets = secrets;
        }

        tracing::info!(
            peer_pid = ?self.peer_pid,
            wasm = %self.wasm_basename,
            slots = self.credentials.slot_count(),
            "credential update received"
        );
        self.runtime
            .deliver_credentials_update(self.credentials.clone(), self.credential_secrets.clone());
    }

    fn on_wayland_event(&mut self, event: &WidgetEvent) {
        match event {
            WidgetEvent::Lifecycle(state) => {
                let target = map_protocol_lifecycle(*state);
                let previous_target = self.lifecycle.target();
                let effect = self.lifecycle.on_event(target);
                if effect.request_render {
                    self.surface.mark_needs_render();
                }
                tracing::debug!(
                    peer_pid = ?self.peer_pid,
                    wasm = %self.wasm_basename,
                    ?previous_target,
                    ?target,
                    request_render = effect.request_render,
                    "lifecycle event received"
                );
            }
            WidgetEvent::TransitionIncoming => {
                let current = self.lifecycle.current();
                let request_render = should_render(current);
                if request_render {
                    self.forced_render_pending = true;
                    self.surface.mark_needs_render();
                }
                tracing::debug!(
                    peer_pid = ?self.peer_pid,
                    wasm = %self.wasm_basename,
                    ?current,
                    request_render,
                    "transition incoming event received"
                );
            }
            WidgetEvent::Shutdown => {
                tracing::info!(
                    peer_pid = ?self.peer_pid,
                    wasm = %self.wasm_basename,
                    "shutdown event received"
                );
            }
            WidgetEvent::Setting(_)
            | WidgetEvent::ParamUpdate(_)
            | WidgetEvent::CredentialsUpdate(_)
            | WidgetEvent::SecretsUpdate(_)
            | WidgetEvent::TouchDown { .. }
            | WidgetEvent::TouchMotion { .. }
            | WidgetEvent::TouchUp { .. }
            | WidgetEvent::TouchCancel => {
                // The drain loop in `dispatch_wayland_events` filters Setting,
                // ParamUpdate, credential and touch events into their coalescing
                // paths and never dispatches them here.
                unreachable!("coalesced events are handled in dispatch_wayland_events drain");
            }
        }
    }
}

/// Fold a per-field [`SettingUpdate`] (wayland-wire) into
/// the [`SystemSnapshot`] the host maintains per widget slot.
///
/// The host bridges the wayland-wire enums (`bmc_shared_time::*`,
/// `bmc_shared_utils::*`, `bmc_widget_protocol::*`) and the wasmi-wire
/// enums (`bmc_wasm_protocol::system::*`).
///
/// The wasm runtime intentionally does not depend on the bmc-shared crates,
/// so the translation lives here.
/// Narrow Wayland touch pixel coordinates to the `f32` the render interaction
/// layer uses. The values are screen-pixel positions, so the precision loss is
/// inconsequential.
#[expect(
    clippy::cast_possible_truncation,
    reason = "touch coordinates from Wayland are pixel positions; f64→f32 precision loss is acceptable"
)]
fn wayland_touch_xy(x: f64, y: f64) -> (f32, f32) {
    (x as f32, y as f32)
}

/// Map a `LedRequest` from the wasm runtime to the `deck_widget_v1`
/// action shape used by `DeckWidgetSurfaceClient::request_action`.
///
/// Six-variant `LedEffect` (runtime side) and `bmc_widget_protocol::
/// LedEffect` are both protocol-aligned, so the effect transform is
/// an exhaustive variant identity. Duration is `Option<Duration>` on
/// the runtime side, which picks `LedEndless` (None) vs `LedTemporary`
/// (Some). `LedRequest::Stop` carries the widget's request id; the
/// runtime emits `0` for stop-all and any non-zero for targeted, but
/// the current SDK only emits stop-all.
fn led_request_to_action(req: &LedRequest) -> ActionPayload {
    match req {
        LedRequest::SetEffect {
            request_id,
            effect,
            color,
            period_ms,
            duration,
            scope,
        } => {
            let effect = match effect {
                LedEffect::Chase => ProtoEffect::Chase,
                LedEffect::KnightRider => ProtoEffect::KnightRider,
                LedEffect::Scan => ProtoEffect::Scan,
                LedEffect::Snake => ProtoEffect::Snake,
                LedEffect::Breathe => ProtoEffect::Breathe,
                LedEffect::Solid => ProtoEffect::Solid,
            };
            let color = RgbColor {
                r: color.r,
                g: color.g,
                b: color.b,
            };
            let scope = match scope {
                LedScope::Local => ProtoScope::Local,
                LedScope::Global => ProtoScope::Global,
            };
            match duration {
                None => ActionPayload::LedEndless {
                    request_id: *request_id,
                    effect,
                    color,
                    period_ms: *period_ms,
                    scope,
                },
                Some(d) => ActionPayload::LedTemporary {
                    request_id: *request_id,
                    effect,
                    color,
                    period_ms: *period_ms,
                    duration_ms: u32::try_from(d.as_millis()).unwrap_or(u32::MAX),
                    scope,
                },
            }
        }
        LedRequest::Stop { request_id } => ActionPayload::StopLed {
            request_id: *request_id,
        },
    }
}

fn apply_setting_update(snap: &mut SystemSnapshot, update: &SettingUpdate) {
    match update {
        SettingUpdate::Timezone(tz) => snap.settings.timezone.clone_from(tz),
        SettingUpdate::TimeFormat(t) => snap.settings.time_format = time_format_to_wasmi(*t),
        SettingUpdate::DateFormat(d) => snap.settings.date_format = date_format_to_wasmi(*d),
        SettingUpdate::NumberFormat(n) => snap.settings.number_format = number_format_to_wasmi(*n),
        SettingUpdate::TemperatureUnit(u) => {
            snap.settings.temperature_unit = temperature_unit_to_wasmi(*u);
        }
        SettingUpdate::FirstDayOfWeek(w) => snap.settings.first_day_of_week = weekday_to_wasmi(*w),
        SettingUpdate::UnitSystem(u) => snap.settings.unit_system = unit_system_to_wasmi(*u),
        SettingUpdate::NextAlarm(na) => {
            snap.next_alarm = na.as_ref().map(next_alarm_to_runtime);
        }
        SettingUpdate::NightMode(active) => snap.night_mode = *active,
    }
}

fn time_format_to_wasmi(
    t: bmc_widget_protocol::TimeSystem,
) -> bmc_wasm_protocol::system::TimeFormat {
    use bmc_wasm_protocol::system::TimeFormat as W;
    use bmc_widget_protocol::TimeSystem as H;
    match t {
        H::Hour12 => W::Hour12,
        H::Hour24 => W::Hour24,
    }
}

fn date_format_to_wasmi(
    d: bmc_widget_protocol::DateFormat,
) -> bmc_wasm_protocol::system::DateFormat {
    use bmc_wasm_protocol::system::DateFormat as W;
    use bmc_widget_protocol::DateFormat as H;
    match d {
        H::DdMmYyyyDot => W::DdMmYyyyDot,
        H::DdMmYyyySlash => W::DdMmYyyySlash,
        H::DMYyyySlash => W::DMYyyySlash,
        H::MDYyyySlash => W::MDYyyySlash,
        H::DdMmYyyyDash => W::DdMmYyyyDash,
        H::YyyyMDSlash => W::YyyyMDSlash,
        H::YyyyMmDdDot => W::YyyyMmDdDot,
        H::YyyyMmDdDash => W::YyyyMmDdDash,
    }
}

fn number_format_to_wasmi(
    n: bmc_widget_protocol::NumberFormat,
) -> bmc_wasm_protocol::system::NumberFormat {
    use bmc_wasm_protocol::system::NumberFormat as W;
    use bmc_widget_protocol::NumberFormat as H;
    match n {
        H::SpaceGroupCommaDecimal => W::SpaceGroupCommaDecimal,
        H::CommaGroupDotDecimal => W::CommaGroupDotDecimal,
        H::DotGroupCommaDecimal => W::DotGroupCommaDecimal,
        H::SpaceGroupDotDecimal => W::SpaceGroupDotDecimal,
    }
}

fn temperature_unit_to_wasmi(
    u: bmc_widget_protocol::TemperatureUnit,
) -> bmc_wasm_protocol::system::TemperatureUnit {
    use bmc_wasm_protocol::system::TemperatureUnit as W;
    use bmc_widget_protocol::TemperatureUnit as H;
    match u {
        H::Celsius => W::Celsius,
        H::Fahrenheit => W::Fahrenheit,
    }
}

fn weekday_to_wasmi(w: bmc_widget_protocol::WeekDay) -> bmc_wasm_protocol::system::Weekday {
    use bmc_wasm_protocol::system::Weekday as W;
    use bmc_widget_protocol::WeekDay as H;
    match w {
        H::Monday => W::Monday,
        H::Tuesday => W::Tuesday,
        H::Wednesday => W::Wednesday,
        H::Thursday => W::Thursday,
        H::Friday => W::Friday,
        H::Saturday => W::Saturday,
        H::Sunday => W::Sunday,
    }
}

fn unit_system_to_wasmi(
    u: bmc_widget_protocol::UnitSystem,
) -> bmc_wasm_protocol::system::UnitSystem {
    use bmc_wasm_protocol::system::UnitSystem as W;
    use bmc_widget_protocol::UnitSystem as H;
    match u {
        H::Metric => W::Metric,
        H::Imperial => W::Imperial,
    }
}

fn next_alarm_to_runtime(na: &WireNextAlarm) -> RuntimeNextAlarm {
    RuntimeNextAlarm {
        fire_at_utc_ms: na.fire_at_utc_ms,
        name: na.name.clone(),
    }
}

fn map_protocol_lifecycle(s: bmc_widget_protocol::LifecycleState) -> LifecycleState {
    use bmc_widget_protocol::LifecycleState as P;
    match s {
        P::Dormant => LifecycleState::Dormant,
        P::Prepared => LifecycleState::Prepared,
        P::Entering => LifecycleState::Entering,
        P::Visible => LifecycleState::Visible,
        P::Leaving => LifecycleState::Leaving,
        _ => {
            tracing::warn!("unknown protocol lifecycle state, defaulting to Dormant");
            LifecycleState::Dormant
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostRenderProfiling;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostRenderFrameContext {
    target_width: u32,
    target_height: u32,
    wants_immediate: bool,
    status: RenderStatus,
}

impl HostRenderFrameContext {
    fn new(
        target_width: u32,
        target_height: u32,
        wants_immediate: bool,
        status: RenderStatus,
    ) -> Self {
        Self {
            target_width,
            target_height,
            wants_immediate,
            status,
        }
    }
}

#[cfg(feature = "profiling")]
type HostRenderPhaseStart = Instant;

#[cfg(not(feature = "profiling"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HostRenderPhaseStart;

impl HostRenderProfiling {
    #[cfg(feature = "profiling")]
    const DEFAULT_FRAME_SUMMARY_INTERVAL: u64 = 120;

    #[cfg(feature = "profiling")]
    fn start_phase() -> HostRenderPhaseStart {
        Instant::now()
    }

    #[cfg(not(feature = "profiling"))]
    fn start_phase() -> HostRenderPhaseStart {
        HostRenderPhaseStart
    }

    #[cfg(feature = "profiling")]
    fn log_phase(wasm: &str, phase: &str, started: HostRenderPhaseStart) {
        tracing::debug!(
            wasm,
            phase,
            elapsed_us = started.elapsed().as_micros(),
            "wasm host render phase"
        );
    }

    #[cfg(not(feature = "profiling"))]
    fn log_phase(_wasm: &str, _phase: &str, _started: HostRenderPhaseStart) {}

    #[cfg(feature = "profiling")]
    fn log_frame(
        wasm: &str,
        frame: u64,
        delta_ms: u32,
        started: HostRenderPhaseStart,
        context: HostRenderFrameContext,
    ) {
        let elapsed_us = started.elapsed().as_micros();
        #[expect(
            clippy::cast_precision_loss,
            reason = "profiling output only needs approximate FPS"
        )]
        let render_fps = if elapsed_us == 0 {
            f64::INFINITY
        } else {
            1_000_000.0 / elapsed_us as f64
        };

        tracing::debug!(
            wasm,
            frame,
            delta_ms,
            total_us = elapsed_us,
            render_fps,
            target_width = context.target_width,
            target_height = context.target_height,
            wants_immediate = context.wants_immediate,
            status = ?context.status,
            "wasm host render frame"
        );

        if frame.is_multiple_of(Self::DEFAULT_FRAME_SUMMARY_INTERVAL) {
            tracing::info!(
                wasm,
                frame,
                delta_ms,
                total_us = elapsed_us,
                render_fps,
                target_width = context.target_width,
                target_height = context.target_height,
                wants_immediate = context.wants_immediate,
                status = ?context.status,
                "wasm host render frame summary"
            );
        }
    }

    #[cfg(not(feature = "profiling"))]
    fn log_frame(
        _wasm: &str,
        _frame: u64,
        _delta_ms: u32,
        _started: HostRenderPhaseStart,
        _context: HostRenderFrameContext,
    ) {
    }
}

/// Acquire the GPU render lock, prepare the shared scratch FBO, normalise GL
/// state, execute `render_fn` under the lock, then fence-wait and drop the
/// lock before returning. `on_after_fence` fires immediately after the fence
/// completes and before the lock is released (callers use it to record timing
/// phases that span only the fence wait).
///
/// Lock invariant: the GPU render lock is held from acquisition until after
/// [`SharedHost::flush_and_wait_gl`] completes, then dropped. Any
/// `export_and_swap` call must happen AFTER this function returns.
///
/// `render_fn` receives `&mut SharedHost` so it can call
/// [`SharedHost::blit_staging_to`] and access `shared.egl`. It must NOT
/// call [`SharedHost::flush_and_wait_gl`] itself — the helper owns that step.
pub(crate) fn stage_frame_under_gpu_lock<F, T, G>(
    shared: &mut SharedHost,
    lock_label: &'static str,
    w: u32,
    h: u32,
    render_fn: F,
    on_after_fence: G,
) -> anyhow::Result<T>
where
    F: FnOnce(&mut SharedHost) -> anyhow::Result<T>,
    G: FnOnce(),
{
    let gpu_render_lock = shared.acquire_gpu_render_lock(lock_label)?;
    let _ = shared.scratch.begin_frame(&shared.egl, w, h);
    normalize_gl_state(&shared.egl, w, h);
    let result = render_fn(shared);
    shared.flush_and_wait_gl();
    on_after_fence();
    drop(gpu_render_lock);
    result
}

#[expect(
    clippy::cast_possible_wrap,
    reason = "GL viewport dimensions fit in i32"
)]
pub(crate) fn normalize_gl_state(egl: &bmc_widget::egl::EglContext, w: u32, h: u32) {
    use glow::HasContext;
    let gl = egl.gl();
    unsafe {
        gl.viewport(0, 0, w as i32, h as i32);
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::STENCIL_TEST);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.disable(glow::BLEND);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        gl.stencil_mask(0xFF);
        gl.active_texture(glow::TEXTURE0);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HostRenderFrameContext, RendererAssetEvictor, SystemSnapshot, apply_setting_update,
        evict_renderer_assets, instantiate_cached_runtime, led_request_to_action,
        release_runtime_and_module,
    };
    use crate::module_cache::ModuleCache;
    use bmc_wasm_runtime::{
        LedEffect, LedRequest, LedScope, Rgb, RuntimeConfig, RuntimeDisplayInfo,
    };
    use bmc_widget_protocol::{
        ActionPayload, LedEffect as ProtoEffect, LedScope as ProtoScope,
        NextAlarm as WireNextAlarm, RgbColor, SettingUpdate,
    };
    use std::path::Path;
    use std::time::Duration;

    fn cached_runtime_wat(render: bool, trapping_init: bool) -> Vec<u8> {
        let render_export = if render {
            r#"(func (export "render") (param i32))"#
        } else {
            ""
        };
        let init_export = if trapping_init {
            r#"(func (export "init") unreachable)"#
        } else {
            r#"(func (export "init"))"#
        };
        wat::parse_str(format!(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "__bmc_sdk_init") (result i64)
                i64.const {})
              {render_export}
              {init_export}
              (func (export "probe") (result i32) i32.const 7))
            "#,
            bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
        ))
        .expect("BUG: cached-runtime test WAT must parse")
    }

    fn write_cached_runtime(directory: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, bytes).expect("BUG: cached-runtime fixture must be writable");
        path
    }

    fn instantiate_test_runtime(
        cache: &ModuleCache,
        path: &Path,
    ) -> anyhow::Result<(
        bmc_wasm_runtime::WasmWidgetRuntime,
        crate::module_cache::ModuleLease,
    )> {
        let loaded = cache.load(path)?;
        instantiate_cached_runtime(
            loaded.lease,
            320,
            240,
            bmc_wasm_protocol::ViewportShape::Rectangular,
            RuntimeDisplayInfo {
                width: 320,
                height: 240,
                shape: bmc_wasm_protocol::DisplayShape::Rectangular,
                dpi: 1,
            },
            chrono::Local::now().fixed_offset(),
            RuntimeConfig::default(),
        )
    }

    #[test]
    fn cached_runtime_handoff_retains_one_compiled_module() {
        let directory = tempfile::tempdir().expect("BUG: slot test needs a temporary directory");
        let path = write_cached_runtime(
            directory.path(),
            "widget.wasm",
            &cached_runtime_wat(true, false),
        );
        let cache = ModuleCache::new();
        let (first_runtime, first_lease) =
            instantiate_test_runtime(&cache, &path).expect("first runtime must instantiate");
        let (mut second_runtime, second_lease) =
            instantiate_test_runtime(&cache, &path).expect("second runtime must instantiate");

        assert_eq!(
            cache.compile_count(),
            1,
            "two successful runtimes must share one compilation"
        );
        assert!(
            first_lease.ptr_eq(&second_lease),
            "both slots must retain the same cached module"
        );
        assert_eq!(second_runtime.call_export_i32("probe"), Some(7));
        drop((first_runtime, first_lease, second_runtime, second_lease));
        assert_eq!(
            cache.entry_count(),
            0,
            "dropping both runtime leases must evict the module"
        );
    }

    #[test]
    fn cached_runtime_failure_releases_a_new_entry() {
        let directory = tempfile::tempdir().expect("BUG: slot test needs a temporary directory");
        let cache = ModuleCache::new();
        let missing_render = write_cached_runtime(
            directory.path(),
            "missing-render.wasm",
            &cached_runtime_wat(false, false),
        );

        assert!(instantiate_test_runtime(&cache, &missing_render).is_err());
        assert_eq!(
            cache.entry_count(),
            0,
            "export lookup failure must release the miss lease"
        );

        let trapping_init = write_cached_runtime(
            directory.path(),
            "trapping-init.wasm",
            &cached_runtime_wat(true, true),
        );
        assert!(instantiate_test_runtime(&cache, &trapping_init).is_err());
        assert_eq!(
            cache.entry_count(),
            0,
            "init failure must release the miss lease"
        );
    }

    #[test]
    fn runtime_release_evicts_the_final_module_lease() {
        let directory = tempfile::tempdir().expect("BUG: slot test needs a temporary directory");
        let path = write_cached_runtime(
            directory.path(),
            "widget.wasm",
            &cached_runtime_wat(true, false),
        );
        let cache = ModuleCache::new();
        let (runtime, lease) =
            instantiate_test_runtime(&cache, &path).expect("runtime must instantiate");
        let mut lease = Some(lease);

        release_runtime_and_module(runtime, &mut lease);
        assert!(
            lease.is_none(),
            "runtime release must consume its module lease"
        );
        assert_eq!(
            cache.entry_count(),
            0,
            "runtime release must evict the final module lease"
        );
    }

    #[test]
    fn apply_setting_update_timezone_folds_into_snapshot() {
        let mut snap = SystemSnapshot::default();
        apply_setting_update(
            &mut snap,
            &SettingUpdate::Timezone("Europe/Prague".to_owned()),
        );
        assert_eq!(snap.settings.timezone, "Europe/Prague");
    }

    #[test]
    fn apply_setting_update_next_alarm_some_translates_payload() {
        let mut snap = SystemSnapshot::default();
        apply_setting_update(
            &mut snap,
            &SettingUpdate::NextAlarm(Some(WireNextAlarm {
                fire_at_utc_ms: 1_700_000_000_000,
                name: "Wake up".to_owned(),
            })),
        );
        let na = snap
            .next_alarm
            .expect("BUG: NextAlarm(Some) must populate the snapshot");
        assert_eq!(na.fire_at_utc_ms, 1_700_000_000_000);
        assert_eq!(na.name, "Wake up");
    }

    #[test]
    fn apply_setting_update_next_alarm_none_clears_snapshot_field() {
        let mut snap = SystemSnapshot {
            next_alarm: Some(bmc_wasm_runtime::NextAlarm {
                fire_at_utc_ms: 1,
                name: "stale".to_owned(),
            }),
            ..SystemSnapshot::default()
        };
        apply_setting_update(&mut snap, &SettingUpdate::NextAlarm(None));
        assert_eq!(snap.next_alarm, None);
    }

    #[test]
    fn apply_setting_update_night_mode_active_sets_snapshot_field() {
        let mut snap = SystemSnapshot::default();
        apply_setting_update(&mut snap, &SettingUpdate::NightMode(true));
        assert!(snap.night_mode);
    }

    #[test]
    fn apply_setting_update_night_mode_inactive_clears_snapshot_field() {
        let mut snap = SystemSnapshot {
            night_mode: true,
            ..SystemSnapshot::default()
        };
        apply_setting_update(&mut snap, &SettingUpdate::NightMode(false));
        assert!(!snap.night_mode);
    }

    #[derive(Debug)]
    struct RecordingEvictor {
        result: usize,
        prefixes: Vec<String>,
    }

    impl RendererAssetEvictor for RecordingEvictor {
        fn evict_renderer_prefix(&mut self, prefix: &str) -> usize {
            self.prefixes.push(prefix.to_owned());
            self.result
        }
    }

    #[test]
    fn evict_renderer_assets_uses_exact_runtime_namespace() {
        let mut evictor = RecordingEvictor {
            result: 3,
            prefixes: Vec::new(),
        };

        assert_eq!(evict_renderer_assets(&mut evictor, "42"), 3);
        assert_eq!(evictor.prefixes, vec!["42".to_owned()]);
    }

    #[test]
    fn host_render_frame_context_carries_summary_dimensions_and_status() {
        let context =
            HostRenderFrameContext::new(638, 480, true, bmc_wasm_runtime::RenderStatus::Ok);

        assert_eq!(context.target_width, 638);
        assert_eq!(context.target_height, 480);
        assert!(context.wants_immediate);
        assert_eq!(context.status, bmc_wasm_runtime::RenderStatus::Ok);
    }

    fn red() -> Rgb {
        Rgb::new(255, 0, 0)
    }

    #[test]
    fn endless_maps_to_led_endless() {
        let req = LedRequest::SetEffect {
            request_id: 7,
            effect: LedEffect::Breathe,
            color: red(),
            period_ms: 750,
            duration: None,
            scope: LedScope::Local,
        };
        assert_eq!(
            led_request_to_action(&req),
            ActionPayload::LedEndless {
                request_id: 7,
                effect: ProtoEffect::Breathe,
                color: RgbColor { r: 255, g: 0, b: 0 },
                period_ms: 750,
                scope: ProtoScope::Local,
            }
        );
    }

    #[test]
    fn temporary_maps_to_led_temporary() {
        let req = LedRequest::SetEffect {
            request_id: 9,
            effect: LedEffect::Solid,
            color: red(),
            period_ms: 0,
            duration: Some(Duration::from_secs(5)),
            scope: LedScope::Local,
        };
        assert_eq!(
            led_request_to_action(&req),
            ActionPayload::LedTemporary {
                request_id: 9,
                effect: ProtoEffect::Solid,
                color: RgbColor { r: 255, g: 0, b: 0 },
                period_ms: 0,
                duration_ms: 5_000,
                scope: ProtoScope::Local,
            }
        );
    }

    #[test]
    fn temporary_zero_duration_is_preserved() {
        let req = LedRequest::SetEffect {
            request_id: 1,
            effect: LedEffect::Snake,
            color: red(),
            period_ms: 0,
            duration: Some(Duration::ZERO),
            scope: LedScope::Local,
        };
        let action = led_request_to_action(&req);
        let ActionPayload::LedTemporary { duration_ms, .. } = action else {
            panic!("BUG: temporary mapping must produce LedTemporary");
        };
        assert_eq!(duration_ms, 0);
    }

    #[test]
    fn stop_maps_to_stop_led() {
        let req = LedRequest::Stop { request_id: 0 };
        assert_eq!(
            led_request_to_action(&req),
            ActionPayload::StopLed { request_id: 0 }
        );
    }

    #[test]
    fn endless_global_maps_to_led_endless_with_global_scope() {
        let req = LedRequest::SetEffect {
            request_id: 3,
            effect: LedEffect::Chase,
            color: red(),
            period_ms: 500,
            duration: None,
            scope: LedScope::Global,
        };
        assert_eq!(
            led_request_to_action(&req),
            ActionPayload::LedEndless {
                request_id: 3,
                effect: ProtoEffect::Chase,
                color: RgbColor { r: 255, g: 0, b: 0 },
                period_ms: 500,
                scope: ProtoScope::Global,
            }
        );
    }

    #[test]
    fn temporary_global_maps_to_led_temporary_with_global_scope() {
        let req = LedRequest::SetEffect {
            request_id: 5,
            effect: LedEffect::Solid,
            color: red(),
            period_ms: 0,
            duration: Some(Duration::from_secs(5)),
            scope: LedScope::Global,
        };
        assert_eq!(
            led_request_to_action(&req),
            ActionPayload::LedTemporary {
                request_id: 5,
                effect: ProtoEffect::Solid,
                color: RgbColor { r: 255, g: 0, b: 0 },
                period_ms: 0,
                duration_ms: 5_000,
                scope: ProtoScope::Global,
            }
        );
    }
}
