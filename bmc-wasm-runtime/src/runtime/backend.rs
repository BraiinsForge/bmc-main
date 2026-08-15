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

//! WASM runtime wrapper using wasmi.

#![expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ptr::NonNull;
use std::time::Instant;

use anyhow::{Result, bail};
use bmc_wasm_protocol::colors::Color;
use bmc_wasm_protocol::{
    BLACK, BitmapId, ICON_METER, MeshId, RED_60, SDK_INIT_EXPORT, SDK_VERSION, SvgId,
    version_unpack,
};
use chrono::{DateTime, FixedOffset};
use wasmi::{Caller, Extern, Linker};

use bmc_render::renderer::{AssetSuspendResult, AssetTagState, Renderer};
use bmc_render::tree;
use bmc_render::tree::TouchHit;
use bmc_render::{FrameTimings, RendererAssetResolver, layout_and_render_with_asset_resolver};

use crate::host_api::{FixtureEvent, HermeticRun, HostState, Lifecycle, namespaced_tag};
use crate::network::NetworkInfo;
use crate::renderer_assets::{
    AssetBacking, RendererAssetId, RendererAssetKind, RendererAssetRecord, cached_bitmap_dimensions,
};
use crate::system::SystemSnapshot;

use super::{CredentialView, ParamsSnapshot};

/// Logical display geometry handed to [`WasmWidgetRuntime::new`].
///
/// Always supplied by the host from the Wayland handshake; there is no
/// `Default` impl so an accidental fallback can't silently mask a missing
/// handshake plumbing.
#[derive(Debug, Clone, Copy)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub shape: bmc_wasm_protocol::DisplayShape,
    pub dpi: u32,
}

impl From<bmc_widget_protocol::DisplayInfo> for DisplayInfo {
    fn from(info: bmc_widget_protocol::DisplayInfo) -> Self {
        Self {
            width: info.width,
            height: info.height,
            shape: info.shape.into(),
            dpi: info.dpi,
        }
    }
}

impl From<DisplayInfo> for bmc_widget_protocol::DisplayInfo {
    fn from(info: DisplayInfo) -> Self {
        Self {
            width: info.width,
            height: info.height,
            shape: info.shape.into(),
            dpi: info.dpi,
        }
    }
}

/// Write a `TouchHit` (4×f32 LE = 16 bytes) to WASM memory at `out_ptr`.
pub(super) fn write_touch_hit(caller: &mut Caller<'_, HostState>, out_ptr: u32, hit: &TouchHit) {
    let memory = caller.get_export("memory").and_then(Extern::into_memory);
    if let Some(memory) = memory {
        let data = memory.data_mut(caller);
        let start = out_ptr as usize;
        if start + 16 <= data.len() {
            data[start..start + 4].copy_from_slice(&hit.x.to_le_bytes());
            data[start + 4..start + 8].copy_from_slice(&hit.y.to_le_bytes());
            data[start + 8..start + 12].copy_from_slice(&hit.width.to_le_bytes());
            data[start + 12..start + 16].copy_from_slice(&hit.height.to_le_bytes());
        }
    }
}

/// Run a guest call inside the given [`Lifecycle`] phase, restoring the previous phase
/// before returning.
///
/// Save/restore makes the helper re-entrancy-safe by construction:
/// if a host import ever re-enters a guest export,
/// the inner phase doesn't clobber the outer on return.
///
/// Today's lifecycle is flat (`Lifecycle`'s doc spells out the single-threaded,
/// host-serialised guarantee), so `previous` is `Idle` on every outermost call
/// — the save/restore is defence in depth, not currently load-bearing.
///
/// Not panic-safe by design: wasmi reports traps via `Err`, not panic.
/// A Rust-side panic from a guest call indicates a host-import bug
/// we want to surface and abort on, not silently recover from.
fn in_lifecycle<R>(
    store: &mut wasmi::Store<HostState>,
    phase: Lifecycle,
    f: impl FnOnce(&mut wasmi::Store<HostState>) -> R,
) -> R {
    let previous = store.data().current_lifecycle;
    store.data_mut().current_lifecycle = phase;
    let result = f(store);
    store.data_mut().current_lifecycle = previous;
    result
}

/// Call the widget's `__bmc_sdk_init` export and validate against the host.
///
/// Returns the widget's `(major, minor, patch)` version on success.
/// Rejects on missing export or major version mismatch.
fn check_sdk_version(
    instance: wasmi::Instance,
    store: &mut wasmi::Store<HostState>,
) -> Result<(u16, u16, u16)> {
    let (major, minor, patch) = SDK_VERSION;

    let version_func = instance
        .get_typed_func::<(), u64>(&*store, SDK_INIT_EXPORT)
        .map_err(|_| {
            anyhow::anyhow!(
                "widget missing '{SDK_INIT_EXPORT}' export — \
             if using Rust SDK, update bmc-wasm-sdk; \
             otherwise export a `{SDK_INIT_EXPORT}() -> u64` function \
             (packed major|minor<<16|patch<<32, host expects {major}.{minor}.{patch})"
            )
        })?;

    let packed = version_func.call(store, ())?;
    let widget_version = version_unpack(packed);
    let (w_major, w_minor, w_patch) = widget_version;

    if w_major != major {
        bail!(
            "SDK major version mismatch: widget is {w_major}.{w_minor}.{w_patch}, \
             host expects {major}.{minor}.{patch}"
        );
    }

    tracing::info!(
        "widget SDK version {w_major}.{w_minor}.{w_patch} \
         (host {major}.{minor}.{patch})"
    );
    Ok(widget_version)
}

/// Result of a single `render()` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    /// Frame rendered successfully within fuel budget.
    Ok,
    /// Widget exceeded its fuel budget this frame.
    /// The last good frame is shown with a warning indicator.
    FuelExhausted,
    /// Widget exceeded its budget too many times and has been killed.
    /// An error overlay is shown; WASM will not be called again
    /// until [`WasmWidgetRuntime::reset_fuel_state`] is called.
    Dead,
}

/// A callback that can intercept fetch requests before they hit the network.
/// Return `Some((status, body))` to short-circuit, `None` to proceed normally.
pub type FetchInterceptor = Box<dyn Fn(&str, &str) -> Option<(u32, Vec<u8>)>>;

/// A callback invoked when a fetch response is delivered.
/// Called with `(method_and_url, status, body)`.
pub type FetchObserver = Box<dyn Fn(&str, u32, &[u8])>;

/// Host-side limits for resources spawned on behalf of a widget.
pub use crate::runtime_limits::RuntimeResourceLimits;

/// Configuration for creating a [`WasmWidgetRuntime`].
///
/// All optional fields are applied **before** the WASM `init()` export runs,
/// so interceptors and KV are available from the widget's first instruction.
#[expect(missing_debug_implementations)]
pub struct RuntimeConfig {
    /// Instruction budget per frame (default: [`WasmWidgetRuntime::FUEL_PER_FRAME`]).
    pub fuel_per_frame: u64,
    /// Deck-wide system snapshot (timezone, time/date/number/temperature/unit
    /// formats, week start, next-alarm). Staged into the runtime before
    /// `init()` runs so the widget's first frame already sees the operator's
    /// values via `bmc_wasm_sdk::system::current()`. The runtime bumps the
    /// version counter once on install; subsequent deliveries arrive through
    /// [`WasmWidgetRuntime::deliver_system_update`].
    pub system: SystemSnapshot,
    /// Key-value storage directory for this widget.
    pub kv_store_path: Option<std::path::PathBuf>,
    /// Intercept fetch requests before they hit the network.
    /// Return `Some((status, body))` to short-circuit, `None` to proceed normally.
    pub fetch_interceptor: Option<FetchInterceptor>,
    /// Base-URL rewrites `(from_prefix, to_prefix)` applied at the last hop,
    /// ahead of secret substitution and the egress check — dev plumbing
    /// that points a widget's hard-coded API base at a simulator.
    pub url_rewrites: Vec<(String, String)>,
    /// Hermetic-run mode: refuse (and record) any live external I/O a fixture
    /// does not satisfy, instead of hitting the network.
    ///
    /// The capture harness sets this so a stale/missing fixture fails loudly
    /// rather than pulling live data into a visual baseline.
    pub hermetic: bool,
    /// Called when a fetch response is delivered. Use for recording/logging.
    pub fetch_observer: Option<FetchObserver>,
    /// Enable recording of network events (SSDP, mDNS, WebSocket, etc.).
    /// Recorded events are drained via [`WasmWidgetRuntime::take_recorded_events`].
    pub record_events: bool,
    /// Pre-recorded event timeline for deterministic replay.
    pub event_fixtures: Vec<FixtureEvent>,
    /// Per-runtime caps for host-side resources such as fetches and sockets.
    pub resource_limits: RuntimeResourceLimits,
    /// Shared file whose exclusive lock permits one image decode at a time.
    pub image_decode_lock_path: Option<std::path::PathBuf>,
    /// MSAA samples used by the mesh atlas renderer. `0` disables mesh MSAA.
    pub mesh_msaa_samples: u32,
    /// Seed for the host RNG.
    ///
    /// - `None` keeps the default time-derived auto-seeding (the host picks a
    ///   non-zero seed from `monotonic_ms` on first use).
    /// - `Some(s)` honours the seed verbatim, including `Some(0)`. Note that
    ///   `Some(0)` makes the xorshift state stuck at zero (the RNG returns
    ///   `0` indefinitely); pick any non-zero seed for varied deterministic
    ///   output.
    pub rng_seed: Option<u64>,
    /// Sender for widget-perspective LED requests. Widgets call
    /// `led::set_effect()` / `led::stop()`; the runtime translates each call
    /// into a `LedRequest` published through this channel. `None` = LED
    /// control unavailable.
    pub led_request_sender: Option<std::sync::mpsc::Sender<crate::led_request::LedRequest>>,
    /// Frame poll cadence (ms) capping the effective host wake while host-side
    /// animations are active, so a widget's `request_frame_after(longer)`
    /// (e.g. a 1Hz clock tick) does not starve cached-tree animation replays.
    /// Defaults to [`Self::DEFAULT_ANIMATION_FRAME_DELAY_MS`] (~30 fps), which
    /// matches the Deck's hardware ceiling for 3D content. Hosts on faster
    /// targets can lower this toward 16 ms (60 fps) per the BDK-266 NFR.
    pub animation_frame_delay_ms: u32,
    /// Initial widget params, staged into the runtime **before** the guest's `init()`
    /// runs so the widget's first frame already sees operator-configured values
    /// via `bmc_wasm_sdk::params::current()`. The manifest's per-`ParamKind` defaults
    /// are expected to be folded in by the caller (compositor / testbed).
    /// The runtime stores the table verbatim and does not re-apply defaults.
    ///
    /// The host bumps the version counter once when these are installed.
    /// The bump is observed by the guest's lazy `params::current()` fetch on first read,
    /// not as a separate `on_params_update` invocation (that hook only fires for **subsequent**
    /// deliveries through [`WasmWidgetRuntime::deliver_params_update`]).
    pub params:
        std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
    /// Which credential slots are bound, staged before `init`
    /// so the widget's first frame renders live rather than degraded.
    pub credentials: CredentialView,
    pub credential_secrets: bmc_widget_protocol::CredentialSecrets,
    /// Per-instance asset cache, curried to this widget's bucket; `None` disables it.
    pub asset_cache: Option<crate::disk_cache::DiskCache>,
    /// Immutable package source for assets extracted from this widget's WASM.
    pub package_assets: Option<crate::PackageAssetStore>,
    /// Compositor token for asset-tag namespacing; `None` → synthetic `dev-N`.
    pub instance_token: Option<String>,
}

impl RuntimeConfig {
    /// Default animation cadence: ~30 fps (33 ms). Matches the BDK-355 mesh
    /// budget and the observed compositor rate on the Vivante GC400.
    pub const DEFAULT_ANIMATION_FRAME_DELAY_MS: u32 = 33;
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            fuel_per_frame: WasmWidgetRuntime::FUEL_PER_FRAME,
            system: SystemSnapshot::default(),
            kv_store_path: None,
            fetch_interceptor: None,
            url_rewrites: Vec::new(),
            hermetic: false,
            fetch_observer: None,
            record_events: false,
            event_fixtures: Vec::new(),
            resource_limits: RuntimeResourceLimits::default(),
            image_decode_lock_path: None,
            mesh_msaa_samples: 0,
            rng_seed: None,
            led_request_sender: None,
            animation_frame_delay_ms: Self::DEFAULT_ANIMATION_FRAME_DELAY_MS,
            params: std::collections::BTreeMap::new(),
            credentials: CredentialView::default(),
            credential_secrets: bmc_widget_protocol::CredentialSecrets::default(),
            asset_cache: None,
            package_assets: None,
            instance_token: None,
        }
    }
}

/// Coalesced lifecycle work awaiting renderer-backed delivery.
/// Opposite edges preserve both hooks in committed order.
enum PendingHook {
    Wake,
    Sleep,
    SleepThenWake,
    WakeThenSleep,
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RendererAssetSuspensionObservation {
    pub svg_suspended: usize,
    pub bitmap_suspended: usize,
    pub mesh_suspended: usize,
    #[cfg(feature = "profiling")]
    pub svg_heap_bytes_released: u64,
    #[cfg(feature = "profiling")]
    pub svg_path_bytes_released: u64,
    #[cfg(feature = "profiling")]
    pub bitmap_released: u64,
    #[cfg(feature = "profiling")]
    pub mesh_bytes_released: u64,
    #[cfg(feature = "profiling")]
    pub svg_path_bytes_resident_total_before: u64,
    #[cfg(feature = "profiling")]
    pub svg_path_bytes_resident_total_after: u64,
    #[cfg(feature = "profiling")]
    pub bitmap_resident_total_before: u64,
    #[cfg(feature = "profiling")]
    pub bitmap_resident_total_after: u64,
    #[cfg(feature = "profiling")]
    pub mesh_bytes_resident_total_before: u64,
    #[cfg(feature = "profiling")]
    pub mesh_bytes_resident_total_after: u64,
}

impl RendererAssetSuspensionObservation {
    #[cfg(feature = "profiling")]
    fn start(renderer: &dyn Renderer) -> Self {
        Self {
            svg_path_bytes_resident_total_before: renderer.svg_resident_path_bytes(),
            bitmap_resident_total_before: renderer.bitmap_resident_bytes(),
            mesh_bytes_resident_total_before: renderer.mesh_resident_bytes(),
            ..Self::default()
        }
    }

    #[cfg(not(feature = "profiling"))]
    fn start(_renderer: &dyn Renderer) -> Self {
        Self::default()
    }

    #[cfg(feature = "profiling")]
    fn finish(mut self, renderer: &dyn Renderer) -> Self {
        self.svg_path_bytes_resident_total_after = renderer.svg_resident_path_bytes();
        self.bitmap_resident_total_after = renderer.bitmap_resident_bytes();
        self.mesh_bytes_resident_total_after = renderer.mesh_resident_bytes();
        self.svg_path_bytes_released = self
            .svg_path_bytes_resident_total_before
            .saturating_sub(self.svg_path_bytes_resident_total_after);
        self.bitmap_released = self
            .bitmap_resident_total_before
            .saturating_sub(self.bitmap_resident_total_after);
        self.mesh_bytes_released = self
            .mesh_bytes_resident_total_before
            .saturating_sub(self.mesh_bytes_resident_total_after);
        self
    }

    #[cfg(not(feature = "profiling"))]
    fn finish(self, _renderer: &dyn Renderer) -> Self {
        self
    }
}

#[cfg(feature = "profiling")]
fn suspend_svg_profiled(
    renderer: &mut dyn Renderer,
    tag: &str,
    instance_id: &str,
    raw_tag: &str,
    observation: &mut RendererAssetSuspensionObservation,
) -> AssetSuspendResult<bmc_wasm_protocol::SvgId> {
    let deallocation_probe = bmc_render::profile::DeallocationProbe::start();
    let result = renderer.suspend_svg(tag);
    if let Some(probe) = deallocation_probe {
        let svg_heap_bytes_released = probe.finish();
        observation.svg_heap_bytes_released = observation
            .svg_heap_bytes_released
            .saturating_add(svg_heap_bytes_released);
        tracing::info!(
            target: bmc_render::profile::TARGET,
            instance_id,
            tag = raw_tag,
            svg_heap_bytes_released,
            "widget SVG heap released"
        );
    }
    result
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RendererAssetRestorationObservation {
    pub svg_restored: usize,
    pub bitmap_restored: usize,
    pub mesh_restored: usize,
    pub already_resident: usize,
    pub skipped: usize,
}

enum RendererAssetRestore {
    Restored,
    AlreadyResident,
    Skipped,
}

#[derive(Debug, PartialEq, Eq)]
enum DeadOverlayBackground {
    PreserveFrame,
    ReplaceFrame,
}

impl DeadOverlayBackground {
    fn for_stopped_widget(renderer_asset_failure: Option<&str>) -> Self {
        if renderer_asset_failure.is_some() {
            Self::ReplaceFrame
        } else {
            Self::PreserveFrame
        }
    }

    fn scrim(self) -> Color {
        match self {
            Self::PreserveFrame => BLACK.with_alpha(0.69),
            Self::ReplaceFrame => BLACK,
        }
    }
}

pub(super) struct RendererAssetRestorer<'a> {
    instance_id: &'a str,
    asset_cache: Option<&'a crate::disk_cache::DiskCache>,
    package_assets: Option<&'a crate::PackageAssetStore>,
    renderer_assets: &'a mut crate::renderer_assets::RendererAssetLedger,
    profile_sections: &'a mut BTreeMap<String, u64>,
    has_pending: bool,
    seen: HashSet<RendererAssetId>,
    observation: RendererAssetRestorationObservation,
    failure: Option<String>,
    #[cfg(feature = "profiling")]
    restore_us: u64,
}

impl<'a> RendererAssetRestorer<'a> {
    pub(super) fn new(
        instance_id: &'a str,
        asset_cache: Option<&'a crate::disk_cache::DiskCache>,
        package_assets: Option<&'a crate::PackageAssetStore>,
        renderer_assets: &'a mut crate::renderer_assets::RendererAssetLedger,
        profile_sections: &'a mut BTreeMap<String, u64>,
    ) -> Self {
        let has_pending = renderer_assets.has_pending_restorable();
        Self {
            instance_id,
            asset_cache,
            package_assets,
            renderer_assets,
            profile_sections,
            has_pending,
            seen: HashSet::new(),
            observation: RendererAssetRestorationObservation::default(),
            failure: None,
            #[cfg(feature = "profiling")]
            restore_us: 0,
        }
    }

    fn resolve(&mut self, renderer: &mut dyn Renderer, id: RendererAssetId) -> bool {
        if self.failure.is_some() {
            return false;
        }
        if !self.has_pending {
            return true;
        }
        if !self.seen.insert(id) {
            return true;
        }
        let pending = self.renderer_assets.pending_by_id(id);
        if pending.is_empty() {
            return true;
        }
        for (raw_tag, record) in pending {
            let restore_started = Instant::now();
            let restore_result = restore_renderer_asset(
                self.instance_id,
                self.asset_cache,
                self.package_assets,
                renderer,
                &raw_tag,
                &record,
            );
            let restore_us =
                u64::try_from(restore_started.elapsed().as_micros()).unwrap_or(u64::MAX);
            #[cfg(feature = "profiling")]
            {
                self.restore_us = self.restore_us.saturating_add(restore_us);
            }
            match restore_result {
                Ok(RendererAssetRestore::Restored) => {
                    self.renderer_assets.mark_resident(&raw_tag);
                    *self
                        .profile_sections
                        .entry("asset_restore_us".to_owned())
                        .or_default() += restore_us;
                    match record.kind {
                        RendererAssetKind::Svg => self.observation.svg_restored += 1,
                        RendererAssetKind::Bitmap(_) => self.observation.bitmap_restored += 1,
                        RendererAssetKind::Mesh => self.observation.mesh_restored += 1,
                    }
                    #[cfg(feature = "profiling")]
                    tracing::info!(
                        target: bmc_render::profile::TARGET,
                        instance_id = self.instance_id,
                        tag = %raw_tag,
                        asset_kind = record.kind.name(),
                        asset_id = record.id.to_ffi(),
                        "widget renderer asset restored on demand"
                    );
                }
                Ok(RendererAssetRestore::AlreadyResident) => {
                    self.observation.already_resident += 1;
                    self.renderer_assets.mark_resident(&raw_tag);
                }
                Ok(RendererAssetRestore::Skipped) => {
                    self.observation.skipped += 1;
                    self.renderer_assets.disable_restoration(&raw_tag);
                }
                Err(error) => {
                    tracing::error!(
                        instance_id = self.instance_id,
                        %error,
                        "renderer asset restoration failed"
                    );
                    self.failure = Some(error);
                    return false;
                }
            }
        }
        self.has_pending = self.renderer_assets.has_pending_restorable();
        true
    }

    pub(super) fn finish(self) -> Result<Option<RendererAssetRestorationObservation>, String> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let observation = self.observation;
        let demand_work = observation.svg_restored
            + observation.bitmap_restored
            + observation.mesh_restored
            + observation.already_resident
            + observation.skipped;
        if demand_work == 0 {
            return Ok(None);
        }
        #[cfg(feature = "profiling")]
        tracing::info!(
            target: bmc_render::profile::TARGET,
            instance_id = self.instance_id,
            svg_restored = observation.svg_restored,
            bitmap_restored = observation.bitmap_restored,
            mesh_restored = observation.mesh_restored,
            already_resident = observation.already_resident,
            assets_skipped = observation.skipped,
            restore_us = self.restore_us,
            "widget renderer assets restored on demand"
        );
        Ok(Some(observation))
    }
}

impl RendererAssetResolver for RendererAssetRestorer<'_> {
    fn resolve_svg(&mut self, renderer: &mut dyn Renderer, id: SvgId) -> bool {
        self.resolve(renderer, RendererAssetId::Svg(id))
    }

    fn resolve_bitmap(&mut self, renderer: &mut dyn Renderer, id: BitmapId) -> bool {
        self.resolve(renderer, RendererAssetId::Bitmap(id))
    }

    fn resolve_mesh(&mut self, renderer: &mut dyn Renderer, id: MeshId) -> bool {
        self.resolve(renderer, RendererAssetId::Mesh(id))
    }
}

fn restore_renderer_asset(
    instance_id: &str,
    asset_cache: Option<&crate::disk_cache::DiskCache>,
    package_assets: Option<&crate::PackageAssetStore>,
    renderer: &mut dyn Renderer,
    raw_tag: &str,
    record: &RendererAssetRecord,
) -> Result<RendererAssetRestore, String> {
    let tag = namespaced_tag(instance_id, raw_tag);
    let already_resident = match (record.kind, record.id) {
        (RendererAssetKind::Svg, RendererAssetId::Svg(expected)) => {
            renderer.svg_tag_state(&tag) == AssetTagState::Resident(expected)
        }
        (RendererAssetKind::Bitmap(_), RendererAssetId::Bitmap(expected)) => {
            renderer.bitmap_tag_state(&tag) == AssetTagState::Resident(expected)
        }
        (RendererAssetKind::Mesh, RendererAssetId::Mesh(expected)) => {
            renderer.mesh_tag_state(&tag) == AssetTagState::Resident(expected)
        }
        _ => false,
    };
    if already_resident {
        return Ok(RendererAssetRestore::AlreadyResident);
    }
    let restored = match &record.backing {
        AssetBacking::Package(id) => {
            restore_package_asset(package_assets, renderer, raw_tag, &tag, record, *id)?
        }
        AssetBacking::Cache(key) => {
            let Some(cache) = asset_cache else {
                return Ok(RendererAssetRestore::Skipped);
            };
            let Some(blob) = cache.get(key) else {
                return Ok(RendererAssetRestore::Skipped);
            };
            let Some((width, height)) = cached_bitmap_dimensions(&blob) else {
                cache.evict(key);
                return Ok(RendererAssetRestore::Skipped);
            };
            renderer
                .register_bitmap_rgba(&tag, blob.bytes(), width, height)
                .map(RendererAssetId::Bitmap)
        }
        AssetBacking::Volatile => return Ok(RendererAssetRestore::Skipped),
    };
    let Some(restored) = restored else {
        return Err(format!(
            "renderer failed to register asset while restoring {raw_tag}"
        ));
    };
    if restored == record.id {
        Ok(RendererAssetRestore::Restored)
    } else {
        Err(format!(
            "asset reservation changed while restoring {raw_tag}"
        ))
    }
}

fn restore_package_asset(
    package_assets: Option<&crate::PackageAssetStore>,
    renderer: &mut dyn Renderer,
    raw_tag: &str,
    tag: &str,
    record: &RendererAssetRecord,
    id: bmc_wasm_protocol::PackageAssetId,
) -> Result<Option<RendererAssetId>, String> {
    let Some(store) = package_assets else {
        return Err(format!(
            "package store unavailable while restoring {raw_tag}"
        ));
    };
    let kind = match record.kind {
        RendererAssetKind::Svg => bmc_wasm_protocol::PackageAssetKind::Svg,
        RendererAssetKind::Bitmap(_) => bmc_wasm_protocol::PackageAssetKind::Bitmap,
        RendererAssetKind::Mesh => bmc_wasm_protocol::PackageAssetKind::Mesh,
    };
    let payload = store
        .load(kind, id)
        .map_err(|error| format!("load package asset {raw_tag}: {error}"))?;
    let restored = match record.kind {
        RendererAssetKind::Svg => renderer
            .register_svg(tag, &payload)
            .map(RendererAssetId::Svg),
        RendererAssetKind::Bitmap(bmc_wasm_protocol::BitmapSampling::Linear) => renderer
            .register_bitmap(tag, &payload)
            .map(RendererAssetId::Bitmap),
        RendererAssetKind::Bitmap(bmc_wasm_protocol::BitmapSampling::Nearest) => renderer
            .register_bitmap_nearest(tag, &payload)
            .map(RendererAssetId::Bitmap),
        RendererAssetKind::Mesh => renderer
            .register_mesh(tag, &payload)
            .map(RendererAssetId::Mesh),
    };
    Ok(restored)
}

/// A validated WebAssembly widget and its shared engine.
#[expect(missing_debug_implementations)]
pub struct WasmWidgetModule {
    module: wasmi::Module,
}

impl WasmWidgetModule {
    /// Validate a WebAssembly widget using the runtime's process-wide configuration.
    pub fn compile(wasm_bytes: &[u8]) -> Result<Self> {
        let started = Instant::now();
        let span = tracing::trace_span!("wasm_module_compile", bytes = wasm_bytes.len());
        let _entered = span.enter();
        let result = (|| {
            crate::package_assets::reject_embedded_package_assets(wasm_bytes)?;
            let mut engine_config = wasmi::Config::default();
            engine_config.consume_fuel(true);
            // Wasmi 1.0.9's EngineStacks::reuse_or_new allocates on pool exhaustion;
            // execute_root_func and executor/stack reset reused stacks and initialize
            // every newly live frame slot. Re-audit on upgrade.
            engine_config.set_max_cached_stacks(4);
            // Disable Wasm proposals not used by our Rust-compiled widgets.
            // Saves validation/translation overhead.
            engine_config.wasm_tail_call(false);
            engine_config.wasm_multi_memory(false);
            engine_config.wasm_memory64(false);
            engine_config.wasm_extended_const(false);
            engine_config.wasm_custom_page_sizes(false);
            engine_config.wasm_wide_arithmetic(false);
            let engine = wasmi::Engine::new(&engine_config);
            // Wasmi validates eagerly; its default lazy mode defers function translation.
            // First call stores translated code in the shared engine's code map.
            // That call consumes 7 fuel per function-body byte from its Store;
            // later sibling instances do not pay.
            let module = wasmi::Module::new(&engine, wasm_bytes)?;
            Ok(Self { module })
        })();
        match &result {
            Ok(_) => tracing::trace!(
                elapsed_us = started.elapsed().as_micros(),
                "wasm module compiled"
            ),
            Err(error) => {
                tracing::trace!(elapsed_us = started.elapsed().as_micros(), %error, "wasm module compilation failed");
            }
        }
        result
    }
}

/// WebAssembly widget runtime.
///
/// Executes WASM modules in a sandboxed environment with fuel metering.
/// The GPU renderer is owned by the caller and installed for each
/// render scope via [`Self::with_renderer`].
#[expect(missing_debug_implementations)]
pub struct WasmWidgetRuntime {
    pub(super) store: wasmi::Store<HostState>,
    pub(super) instance: wasmi::Instance,
    render_func: wasmi::TypedFunc<u32, ()>,
    /// Optional guest export called best-effort during teardown.
    /// Symmetric counterpart to `init`; gives a widget a chance to release
    /// its own ephemeral state before the host-side safety sweep runs.
    unload_func: Option<wasmi::TypedFunc<(), ()>>,
    /// Optional guest export fired for every params-snapshot delivery
    /// **after** the initial one.
    /// The first delivery is observed through `params::current()` inside `init`.
    /// Subsequent deliveries arrive via [`Self::deliver_params_update`], which calls
    /// this hook under [`Lifecycle::ParamsUpdate`].
    on_params_update_func: Option<wasmi::TypedFunc<(), ()>>,
    /// Optional guest export fired for every system-snapshot delivery
    /// **after** the initial one. Sibling of [`Self::on_params_update_func`]
    /// for the `system` channel; isolating the two hooks lets a widget diff
    /// `system::current()` vs `system::previous()` inside this export without
    /// risking a stale read on the params channel (which doesn't rotate
    /// on a system-only delivery).
    on_system_update_func: Option<wasmi::TypedFunc<(), ()>>,
    /// Sibling of the two above for the credential channel.
    on_credentials_update_func: Option<wasmi::TypedFunc<(), ()>>,
    /// Optional guest export fired once per Wayland drain that delivered touch
    /// activity. A widget that wants to respond to touch must export this and
    /// call `request_frame()` from it — the host no longer force-renders on
    /// touch, so without the hook the widget's touch is dropped.
    on_touch_func: Option<wasmi::TypedFunc<(), ()>>,
    /// Optional guest export fired when the Deck's own SSID or IP changed.
    /// The widget re-reads `network::info()` and calls `request_frame()`
    /// if the change is visible on its current screen —
    /// the host never force-renders on network changes.
    on_network_update_func: Option<wasmi::TypedFunc<(), ()>>,
    /// Optional guest exports fired on the dormancy/wake edge.
    on_wake_func: Option<wasmi::TypedFunc<(), ()>>,
    on_sleep_func: Option<wasmi::TypedFunc<(), ()>>,
    pending_hook: Option<PendingHook>,
    sdk_version: (u16, u16, u16),
    /// Instruction budget reset before each WASM frame execution.
    pub(super) fuel_per_frame: u64,
    /// Consecutive frames that exceeded the fuel budget.
    fuel_strikes: u32,
    /// Trap taken by a guest callback, drained by `poll_deliveries`.
    ///
    /// A trap unwinds the guest's wasm frames without running their epilogues,
    /// so `__stack_pointer` keeps whatever value it had when the trap fired.
    /// Later calls start from there; the instance must not be driven again.
    pub(super) guest_trap: Option<anyhow::Error>,
    #[cfg(feature = "capture")]
    lifecycle_trap: Option<anyhow::Error>,
    /// Widget permanently stopped after exceeding [`Self::max_fuel_strikes`].
    fuel_dead: bool,
    last_asset_suspension: Option<RendererAssetSuspensionObservation>,
    /// How many consecutive fuel-outs before the widget is killed.
    max_fuel_strikes: u32,
    #[cfg(feature = "profiling")]
    wasm_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    wasm_every: ii_stopwatch::Every,
}

impl WasmWidgetRuntime {
    /// Maximum fuel (instructions) per frame.
    pub const FUEL_PER_FRAME: u64 = 10_000_000;

    /// Create a new runtime from WASM bytes.
    ///
    /// See [`Self::from_module`] for the initialization contract.
    pub fn new(
        wasm_bytes: &[u8],
        width: u32,
        height: u32,
        viewport_shape: bmc_wasm_protocol::ViewportShape,
        display: DisplayInfo,
        initial_system_time: DateTime<FixedOffset>,
        config: RuntimeConfig,
    ) -> Result<Self> {
        let module = WasmWidgetModule::compile(wasm_bytes)?;
        Self::from_module(
            &module,
            width,
            height,
            viewport_shape,
            display,
            initial_system_time,
            config,
        )
    }

    /// Create per-instance runtime state from an already validated widget module.
    ///
    /// The renderer is owned by the caller; reach it per-frame via
    /// [`Self::with_renderer`]. `width` / `height` describe widget surface
    /// dimensions and are also installed on `HostState` before `init()` runs.
    ///
    /// All configuration from [`RuntimeConfig`] is applied **before** the WASM
    /// `init()` export runs, so interceptors, KV, and event fixtures are available
    /// from the widget's first instruction.
    pub fn from_module(
        module: &WasmWidgetModule,
        width: u32,
        height: u32,
        viewport_shape: bmc_wasm_protocol::ViewportShape,
        display: DisplayInfo,
        initial_system_time: DateTime<FixedOffset>,
        config: RuntimeConfig,
    ) -> Result<Self> {
        let started = Instant::now();
        let span = tracing::trace_span!("wasm_runtime_instantiate", width, height);
        let _entered = span.enter();
        let result = Self::from_module_inner(
            module,
            width,
            height,
            viewport_shape,
            display,
            initial_system_time,
            config,
        );
        match &result {
            Ok(_) => tracing::trace!(
                elapsed_us = started.elapsed().as_micros(),
                "wasm runtime instantiated"
            ),
            Err(error) => {
                tracing::trace!(elapsed_us = started.elapsed().as_micros(), %error, "wasm runtime instantiation failed");
            }
        }
        result
    }

    #[expect(
        clippy::too_many_lines,
        reason = "construction sets up store, linker, host state, and stages \
                  all RuntimeConfig fields before init() runs — splitting helpers \
                  would scatter the ordering contract"
    )]
    fn from_module_inner(
        module: &WasmWidgetModule,
        width: u32,
        height: u32,
        viewport_shape: bmc_wasm_protocol::ViewportShape,
        display: DisplayInfo,
        initial_system_time: DateTime<FixedOffset>,
        config: RuntimeConfig,
    ) -> Result<Self> {
        // `mesh_msaa_samples` belongs to the caller-owned `FemtoVgRenderer` and is
        // dropped here via `..`. The field stays on `RuntimeConfig` so the capture
        // binary can plumb it through to its `FemtoVgRenderer::new` call.
        let RuntimeConfig {
            fuel_per_frame,
            system,
            kv_store_path,
            fetch_interceptor,
            url_rewrites,
            hermetic,
            fetch_observer,
            record_events,
            event_fixtures,
            resource_limits,
            image_decode_lock_path,
            rng_seed,
            led_request_sender,
            animation_frame_delay_ms,
            params,
            credentials,
            credential_secrets,
            asset_cache,
            package_assets,
            instance_token,
            ..
        } = config;

        let engine = module.module.engine();

        let host_state = HostState::new(resource_limits, initial_system_time);

        let mut store = wasmi::Store::new(engine, host_state);
        store.set_fuel(fuel_per_frame)?;

        let mut linker = Linker::new(engine);
        Self::register_host_functions(&mut linker)?;

        // Stage geometry + all RuntimeConfig before instantiation. wasmi runs
        // the module's `start` section eagerly inside `instantiate_and_start`,
        // so staging here guarantees no guest code — start section or later
        // exports — ever observes the zero defaults from `HostState::new`.
        let state = store.data_mut();
        state.widget_width = width;
        state.widget_height = height;
        state.viewport_shape = viewport_shape;
        state.display_width = display.width;
        state.display_height = display.height;
        state.display_shape = display.shape;
        state.display_dpi = display.dpi;
        state.kv_store_path = kv_store_path;
        state.asset_cache = asset_cache;
        state.package_assets = package_assets;
        state.image_decode_lock_path = image_decode_lock_path;
        if let Some(token) = instance_token {
            state.instance_id = token;
        }
        state.fetch_interceptor = fetch_interceptor;
        state.url_rewrites = url_rewrites;
        state.hermetic = hermetic.then(HermeticRun::default);
        state.fetch_observer = fetch_observer;
        state.record_events = record_events;
        state.rng_state = rng_seed;
        state.led_request_sender = led_request_sender;
        state.frame_schedule.animation_frame_delay_ms = animation_frame_delay_ms;
        // Stage the initial snapshots before `init` runs so the guest's
        // first `params::current()` / `system::current()` calls observe
        // operator-configured values, not defaults.
        //
        // Both channels always replace — including the empty-params
        // and default-system cases — so the post-construction invariant
        // is uniform: version is always ≥ 1, and the SDK's first fetch
        // always hits a real snapshot rather than the v0 placeholder.
        // Subsequent operator changes go through `deliver_*_update`,
        // which bumps again and fires `on_params_update`.
        state.params.replace(ParamsSnapshot::new(params));
        state.system.replace(system);
        state.credentials.replace(credentials);
        state.credential_secrets = credential_secrets;
        if !event_fixtures.is_empty() {
            state.event_fixtures = Some(crate::host_api::FixtureEventState {
                events: event_fixtures,
                cursor: 0,
                ws_event_txs: HashMap::new(),
                socket_event_txs: HashMap::new(),
                mdns_event_txs: HashMap::new(),
                ssdp_event_txs: HashMap::new(),
                udp_event_txs: HashMap::new(),
            });
        }
        let instance_id = state.instance_id.clone();

        // State is staged; instantiate (running `start`, if any) and resolve exports.
        let instance = linker.instantiate_and_start(&mut store, &module.module)?;
        let sdk_version = check_sdk_version(instance, &mut store)?;
        let render_func = instance.get_typed_func::<u32, ()>(&store, "render")?;
        let unload_func = instance.get_typed_func::<(), ()>(&store, "unload").ok();
        let on_params_update_func = instance
            .get_typed_func::<(), ()>(&store, "on_params_update")
            .ok();
        let on_system_update_func = instance
            .get_typed_func::<(), ()>(&store, "on_system_update")
            .ok();
        let on_credentials_update_func = instance
            .get_typed_func::<(), ()>(&store, "on_credentials_update")
            .ok();
        let on_touch_func = instance.get_typed_func::<(), ()>(&store, "on_touch").ok();
        let on_network_update_func = instance
            .get_typed_func::<(), ()>(&store, "on_network_update")
            .ok();
        let on_wake_func = instance.get_typed_func::<(), ()>(&store, "on_wake").ok();
        let on_sleep_func = instance.get_typed_func::<(), ()>(&store, "on_sleep").ok();
        tracing::info!(
            instance_id = %instance_id,
            width,
            height,
            has_unload = unload_func.is_some(),
            has_on_params_update = on_params_update_func.is_some(),
            has_on_system_update = on_system_update_func.is_some(),
            has_on_touch = on_touch_func.is_some(),
            has_on_network_update = on_network_update_func.is_some(),
            "runtime instantiated"
        );

        // Call init — all host config (including widget dimensions) is in place.
        // The guest reads its viewport via `widget_size()` / `host_widget_size`
        // rather than init arguments, so the typed func is `() -> ()`.
        if let Ok(init_func) = instance.get_typed_func::<(), ()>(&store, "init") {
            tracing::trace!(instance_id = %instance_id, "calling widget init");
            in_lifecycle(&mut store, Lifecycle::Init, |s| init_func.call(s, ()))?;
            tracing::trace!(instance_id = %instance_id, "widget init completed");
        }

        Ok(Self {
            store,
            instance,
            render_func,
            unload_func,
            on_params_update_func,
            on_system_update_func,
            on_credentials_update_func,
            on_touch_func,
            on_network_update_func,
            on_wake_func,
            on_sleep_func,
            pending_hook: None,
            sdk_version,
            fuel_per_frame,
            fuel_strikes: 0,
            guest_trap: None,
            #[cfg(feature = "capture")]
            lifecycle_trap: None,
            fuel_dead: false,
            last_asset_suspension: None,
            max_fuel_strikes: 5,
            #[cfg(feature = "profiling")]
            wasm_w: ii_stopwatch::StopWatch::default(),
            #[cfg(feature = "profiling")]
            wasm_every: ii_stopwatch::Every::new(std::time::Duration::from_secs(5)),
        })
    }

    fn register_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
        super::imports::register_host_functions(linker)
    }

    /// Render a frame. Caller calls `begin_frame` on its renderer before and
    /// `flush` after, with this call bracketed by [`Self::with_renderer`].
    ///
    /// On animation-only frames (no pending input, host auto-requested),
    /// skips WASM execution and re-renders from cached tree data.
    ///
    /// Returns [`RenderStatus::FuelExhausted`] if the widget blew its budget
    /// (last good frame is shown with a warning bar). After
    /// [`Self::max_fuel_strikes`] consecutive fuel-outs the widget is killed
    /// and [`RenderStatus::Dead`] is returned on every subsequent call.
    #[expect(
        clippy::too_many_lines,
        reason = "carries trace-level instrumentation for BDK-293 hot-reload freeze investigation; remove the expect when the tracing comes back out"
    )]
    pub fn render(&mut self, delta_ms: u32) -> Result<RenderStatus> {
        let state = self.store.data_mut();
        state.last_asset_restoration = None;
        tracing::trace!(
            instance_id = %state.instance_id,
            delta_ms,
            fuel_dead = self.fuel_dead,
            cached_tree = state.cached_tree.is_some(),
            interaction_pending = state.interaction.has_pending_events(),
            animation_only_candidate = state.frame_schedule.is_animation_only_frame(),
            deferred_wasm_render_at_ms = state.frame_schedule.deferred_wasm_render_at_ms,
            monotonic_ms = state.monotonic_ms,
            "render start"
        );

        if let Some(error) = &state.renderer_asset_failure {
            tracing::error!(instance_id = %state.instance_id, %error, "widget renderer assets failed");
            state.interaction.begin_frame();
            state.begin_render_frame();
            Self::draw_dead_overlay(state, DeadOverlayBackground::ReplaceFrame);
            return Ok(RenderStatus::Dead);
        }

        // Dead widget — show overlay on every frame.
        // Use `reset_fuel_state()` to revive (e.g. from a testbed button).
        if self.fuel_dead {
            state.interaction.begin_frame();
            state.begin_render_frame();
            Self::render_stopped_cached_tree(state, delta_ms);
            let background =
                DeadOverlayBackground::for_stopped_widget(state.renderer_asset_failure.as_deref());
            Self::draw_dead_overlay(state, background);
            tracing::trace!(instance_id = %state.instance_id, "render skipped because widget is dead");
            return Ok(RenderStatus::Dead);
        }

        // Decide frame type BEFORE begin_frame consumes events
        let mut animation_only = state.frame_schedule.is_animation_only_frame()
            && !state.interaction.has_pending_events()
            && state.cached_tree.is_some();

        // Check monotonic deadline for deferred WASM render (request_frame_after).
        // Uses monotonic_ms instead of delta_ms countdown because sub-millisecond
        // frames truncate delta_ms to 0 and stall countdown-based timers.
        if let Some(deadline_ms) = state.frame_schedule.deferred_wasm_render_at_ms
            && state.monotonic_ms >= deadline_ms
        {
            state.frame_schedule.deferred_wasm_render_at_ms = None;
            animation_only = false;
        }

        state.interaction.begin_frame();
        state.begin_render_frame();
        state.delta_ms = delta_ms;

        if animation_only {
            if !Self::render_cached_tree(state, delta_ms) {
                Self::draw_dead_overlay(state, DeadOverlayBackground::ReplaceFrame);
                return Ok(RenderStatus::Dead);
            }
            tracing::trace!(
                instance_id = %state.instance_id,
                delta_ms,
                "render replayed cached tree without wasm"
            );
            return Ok(RenderStatus::Ok);
        }

        // Full WASM frame: compute real elapsed time since last WASM render
        // (not just the animation frame's ~0-16ms delta).
        let wasm_delta = (state.monotonic_ms - state.frame_schedule.last_wasm_render_at_ms) as u32;
        state.frame_schedule.last_wasm_render_at_ms = state.monotonic_ms;

        // Full frame: run WASM with per-frame fuel budget.
        self.store.set_fuel(self.fuel_per_frame)?;
        let wasm_t0 = Instant::now();
        ii_stopwatch::stopwatch_start!(self.wasm_w);
        let render_func = self.render_func;
        tracing::trace!(
            instance_id = %self.store.data().instance_id,
            wasm_delta,
            fuel_per_frame = self.fuel_per_frame,
            "calling widget render"
        );
        let call_result = in_lifecycle(&mut self.store, Lifecycle::Render, |s| {
            render_func.call(s, wasm_delta)
        });
        ii_stopwatch::stopwatch_stop!(self.wasm_w);

        #[cfg(feature = "profiling")]
        if ii_stopwatch::every_expired!(self.wasm_every) {
            let rss = bmc_render::proc_mem::read_self_rss();
            let vm_rss_kb = rss.map_or(0, |s| s.vm_rss_kb);
            let rss_shmem_kb = rss.map_or(0, |s| s.rss_shmem_kb);
            tracing::info!(
                target: bmc_render::profile::TARGET,
                "wasm_tick {wasm} vm_rss_kb={vm_rss_kb} rss_shmem_kb={rss_shmem_kb}",
                wasm = self.wasm_w,
            );
            // Each profiling span's share of cumulative guest fuel
            // — which spans dominate the wasm render on this hardware.
            //
            // Read-only: the host never drains `profile_sections`,
            // and draining here would starve the testbed's per-frame capture.
            let sections = &self.store.data().profile_sections;
            let total: u64 = sections.values().sum();
            if total > 0 {
                let mut ranked: Vec<(&str, u64)> = sections
                    .iter()
                    .map(|(name, fuel)| (name.as_str(), *fuel))
                    .collect();
                ranked.sort_unstable_by_key(|(_, fuel)| std::cmp::Reverse(*fuel));
                #[expect(
                    clippy::integer_division,
                    reason = "an integer percent is the intended log format"
                )]
                let shares = ranked
                    .iter()
                    .take(6)
                    .map(|(name, fuel)| format!("{name}={}%", fuel * 100 / total))
                    .collect::<Vec<_>>()
                    .join(" ");
                tracing::info!(target: bmc_render::profile::TARGET, "wasm_sections {shares}");
            }
            ii_stopwatch::stopwatch_reset!(self.wasm_w);
        }

        match call_result {
            Ok(()) => {
                let wasm_us = wasm_t0.elapsed().as_micros() as u32;
                let state = self.store.data_mut();
                state.last_timings.wasm_us = wasm_us;
                if state.renderer_asset_failure.is_some() {
                    Self::draw_dead_overlay(state, DeadOverlayBackground::ReplaceFrame);
                    return Ok(RenderStatus::Dead);
                }
                self.fuel_strikes = 0;
                tracing::trace!(
                    instance_id = %self.store.data().instance_id,
                    wasm_delta,
                    wasm_us,
                    wants_next_frame = self.wants_next_frame(),
                    next_frame_delay_ms = self.next_frame_delay(),
                    has_deferred_render = self.has_deferred_render(),
                    "widget render completed"
                );
                Ok(RenderStatus::Ok)
            }
            Err(e) if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) => {
                self.fuel_strikes += 1;
                tracing::warn!(
                    "widget exceeded fuel budget (strike {}/{})",
                    self.fuel_strikes,
                    self.max_fuel_strikes,
                );
                if self.fuel_strikes >= self.max_fuel_strikes {
                    self.fuel_dead = true;
                    let state = self.store.data_mut();
                    Self::render_stopped_cached_tree(state, delta_ms);
                    let background = DeadOverlayBackground::for_stopped_widget(
                        state.renderer_asset_failure.as_deref(),
                    );
                    Self::draw_dead_overlay(state, background);
                    return Ok(RenderStatus::Dead);
                }
                // Show last good frame + warning bar, and request a
                // retry so the widget can run again with any state
                // changes that happened before the fuel trap.
                let state = self.store.data_mut();
                if !Self::render_cached_tree(state, delta_ms) {
                    Self::draw_dead_overlay(state, DeadOverlayBackground::ReplaceFrame);
                    return Ok(RenderStatus::Dead);
                }
                Self::draw_fuel_warning(state, self.fuel_strikes, self.max_fuel_strikes);
                // Force a retry next frame so the widget can run again with
                // any state changes that happened before the fuel trap.
                state.frame_schedule.widget_delay_ms = Some(0);
                Ok(RenderStatus::FuelExhausted)
            }
            Err(e) => {
                tracing::warn!(
                    instance_id = %self.store.data().instance_id,
                    trap = %e,
                    "widget render trapped"
                );
                Err(e.into())
            }
        }
    }

    /// Re-render the last successfully submitted tree (no WASM execution).
    ///
    /// Runs the cached `TreeNode` through the layout-and-render pipeline
    /// without deserializing it again.
    ///
    /// Invariant: callers must be inside a [`Self::with_renderer`] scope so
    /// `HostState::renderer_ptr` is installed; the `expect` below asserts the
    /// host's own invariant, not the guest's.
    fn render_cached_tree(state: &mut HostState, delta_ms: u32) -> bool {
        state.last_asset_restoration = None;
        if state.cached_tree.is_none() {
            return true;
        }
        let mut ptr = state
            .renderer_ptr
            .expect("BUG: render_cached_tree called outside `with_renderer` scope");
        // SAFETY: `ptr` was installed by `WasmWidgetRuntime::with_renderer` on this
        // thread; single-threaded wasmi dispatch keeps it unique for this borrow.
        let renderer: &mut dyn Renderer = unsafe { ptr.as_mut() };
        Self::layout_cached_tree(state, renderer, delta_ms)
    }

    fn layout_cached_tree(
        state: &mut HostState,
        renderer: &mut dyn Renderer,
        delta_ms: u32,
    ) -> bool {
        let Some((ref tree_node, width, height)) = state.cached_tree else {
            return true;
        };
        let frame_counter = state.frame_counter;
        state.frame_counter += 1;
        let now_unix_secs = state.system_time.timestamp();
        let mut timings = FrameTimings::default();

        let mut ctx = bmc_render::ProcessContext {
            interaction: &mut state.interaction,
            modal_states: &mut state.modal_states,
            scroll_states: &mut state.scroll_states,
            animation_states: &mut state.animation_states,
            transition_states: &mut state.transition_states,
            taffy: &mut state.taffy,
            frame_counter,
            delta_ms,
            now_unix_secs,
        };
        let mut resolver = RendererAssetRestorer::new(
            &state.instance_id,
            state.asset_cache.as_ref(),
            state.package_assets.as_ref(),
            &mut state.renderer_assets,
            &mut state.profile_sections,
        );
        let render_result = layout_and_render_with_asset_resolver(
            tree_node,
            width,
            height,
            renderer,
            &mut resolver,
            &mut timings,
            &mut ctx,
        );
        match resolver.finish() {
            Ok(observation) => state.last_asset_restoration = observation,
            Err(error) => {
                state.renderer_asset_failure = Some(error);
                return false;
            }
        }
        match render_result {
            Ok((result, has_active)) => {
                state.last_timings = timings;
                let had_interaction = !result.clicks.is_empty() || !result.drags.is_empty();
                // No WASM execution, no deserialization on cached frames
                state.tree_clicks = result.clicks;
                state.tree_drags = result.drags;
                state.frame_schedule.has_active_animations = has_active;
                state.frame_schedule.interaction_pending = had_interaction;
                state.frame_schedule.host_frame_delay_ms = result.next_frame_delay_ms;
                true
            }
            Err(e) => {
                tracing::error!("cached tree render failed: {e}");
                true
            }
        }
    }

    fn render_stopped_cached_tree(state: &mut HostState, delta_ms: u32) {
        let _ = Self::render_cached_tree(state, delta_ms);
        state.begin_render_frame();
    }

    /// Subtle red bar at the top edge indicating fuel exhaustion.
    fn draw_fuel_warning(state: &mut HostState, strikes: u32, max_strikes: u32) {
        let mut ptr = state
            .renderer_ptr
            .expect("BUG: draw_fuel_warning called outside `with_renderer` scope");
        // SAFETY: same as `render_cached_tree`.
        let renderer: &mut dyn Renderer = unsafe { ptr.as_mut() };
        let w = renderer.width();
        let fraction = strikes as f32 / max_strikes as f32;
        let bar_w = w * fraction;
        // Red bar, increasingly opaque as strikes accumulate
        #[expect(clippy::cast_sign_loss)] // fraction is always 0..=1
        let alpha = (100.0 + 155.0 * fraction) as u8;
        let color = Color::from_rgba(0xFF, 0x00, 0x00, alpha);
        renderer.fill_rect(0.0, 0.0, bar_w, 3.0, color);
    }

    /// Full error overlay for a dead widget — CDS notification banner.
    fn draw_dead_overlay(state: &mut HostState, background: DeadOverlayBackground) {
        let mut ptr = state
            .renderer_ptr
            .expect("BUG: draw_dead_overlay called outside `with_renderer` scope");
        // SAFETY: same as `render_cached_tree`.
        let renderer: &mut dyn Renderer = unsafe { ptr.as_mut() };
        let canvas_w = renderer.width();
        let canvas_h = renderer.height();

        let title = "This widget has been stopped";
        let subtitle = "It used too many resources and was suspended.";
        let banner_w = f32::clamp(canvas_w * 0.6, 250.0, 400.0);
        let banner_h = tree::measure_notification_banner(title, subtitle, banner_w, renderer);

        renderer.fill_rect(0.0, 0.0, canvas_w, canvas_h, background.scrim());

        tree::render_notification_banner(
            title,
            subtitle,
            RED_60,
            ICON_METER,
            (canvas_w - banner_w) / 2.0,
            (canvas_h - banner_h) / 2.0,
            banner_w,
            banner_h,
            renderer,
        );
    }

    /// Reset the fuel strike counter and dead state.
    ///
    /// Call this after hot-reloading a widget or when the host wants to
    /// give the widget another chance.
    pub fn reset_fuel_state(&mut self) {
        self.fuel_strikes = 0;
        self.fuel_dead = false;
        let state = self.store.data_mut();
        let now = state.monotonic_ms;
        state.frame_schedule.request_frame_after(0, now);
    }

    /// Set the wall-clock time and monotonic clock for the next render.
    ///
    /// Must be called before each `render()`. The testbed sets these from real
    /// clocks; the capture binary increments by fixed 16ms steps.
    pub fn set_time(&mut self, system_time: DateTime<FixedOffset>, monotonic_ms: u64) {
        let state = self.store.data_mut();
        state.system_time = system_time;
        state.monotonic_ms = monotonic_ms;
        // Clamp so that wasm_delta doesn't underflow when time rewinds
        // (e.g. after a capture span resets the timeline cursor).
        state.frame_schedule.last_wasm_render_at_ms = state
            .frame_schedule
            .last_wasm_render_at_ms
            .min(monotonic_ms);
    }

    /// Replace the host-side widget params snapshot **without** firing the guest's
    /// `on_params_update` hook.
    ///
    /// The version counter is bumped via `wrapping_add(1)` so guests observing it
    /// re-fetch the snapshot on the next read of `params::current()`. Intended for
    /// initial-delivery test scenarios and the rare host code path that wants to
    /// stage a snapshot before the first render without notifying the guest.
    /// Production deliveries should use [`Self::deliver_params_update`].
    pub fn set_params(
        &mut self,
        params: std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
    ) {
        self.store
            .data_mut()
            .params
            .replace(ParamsSnapshot::new(params));
    }

    /// Deliver an operator-driven params update to the running widget.
    ///
    /// Stages the snapshot via [`Self::set_params`] (bumping the version) and, if the
    /// widget exported `on_params_update`, calls it under [`Lifecycle::ParamsUpdate`]
    /// with a fresh fuel budget. The hook is best-effort: a trap is logged and swallowed
    /// — the new snapshot is already staged, so the next `render` will pick it up via
    /// `params::current()` regardless of whether the hook completed cleanly.
    ///
    /// No rollback on trap (by design).
    ///
    /// A trapped hook does not revert the snapshot.
    /// The contract is that the compositor schema-validates against the widget's manifest
    /// (see `validate_widget_params` in `bmc/src/web/grpc/scene_management.rs`) before
    /// invoking delivery, so by the time the runtime sees the update, manifest invariants
    /// — required keys present, enum/range constraints, typed kinds — already hold.
    ///
    /// A deterministic trap here (e.g. `ParamRead::read_required` panicking on a missing
    /// required key) therefore implies a widget or SDK bug, not bad operator input;
    /// reverting would mask the bug behind a stale snapshot while the operator-visible
    /// change silently no-ops. Fuel exhaustion is similarly the widget's responsibility.
    ///
    /// Use this for every params delivery **after** the initial one (initial deliveries
    /// belong in [`RuntimeConfig::params`] so they're observable from `init`).
    ///
    /// Returns whether the guest's hook actually ran (`true` = widget exports
    /// `on_params_update` and the call completed without a trap; `false` = no hook or
    /// the hook trapped). Most callers ignore the return; tests use it to assert wiring.
    pub fn deliver_params_update(
        &mut self,
        params: std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
    ) -> bool {
        self.set_params(params);
        self.fire_update_hook(
            self.on_params_update_func,
            "on_params_update",
            Lifecycle::ParamsUpdate,
        )
    }

    /// Deliver an updated deck-wide [`SystemSnapshot`] (timezone, formatting
    /// preferences, next-alarm, night-mode flag) to the running widget.
    ///
    /// Parallel to [`Self::deliver_params_update`] for the `system` channel:
    /// stages the snapshot, bumps the version counter, invalidates the
    /// encoded-cache, and — if the widget exported `on_system_update` — calls
    /// it under [`Lifecycle::SystemUpdate`] with a fresh fuel budget.
    ///
    /// The two channels have **separate** hooks (`on_params_update`
    /// for the params channel, `on_system_update` for the system channel)
    /// so a widget diffing snapshots can rely on `*::previous()`
    /// being fresh — a unified hook would re-fire on the *other*
    /// channel's deliveries and surface the previous-channel's
    /// stale rotation as a spurious diff.
    ///
    /// No rollback on trap (by design); see [`Self::deliver_params_update`].
    ///
    /// Use this for every system delivery **after** the initial one (initial
    /// deliveries belong in [`RuntimeConfig::system`] so they're observable
    /// from `init`). Returns whether the guest's hook actually ran — most
    /// callers ignore it.
    pub fn deliver_system_update(&mut self, snapshot: SystemSnapshot) -> bool {
        self.store.data_mut().system.replace(snapshot);
        self.fire_update_hook(
            self.on_system_update_func,
            "on_system_update",
            Lifecycle::SystemUpdate,
        )
    }

    /// Set the Deck's network info for the `host_network_info` getter.
    /// Fires no hook by itself; the caller follows up with
    /// [`Self::deliver_network_update`] when the value changed.
    pub fn set_network_info(&mut self, info: NetworkInfo) {
        self.store.data_mut().network_info = info;
    }

    /// The hook observes the view only, so a rotated secret fires it
    /// without the guest being able to tell what changed.
    ///
    /// No rollback on trap (by design); see [`Self::deliver_params_update`].
    pub fn deliver_credentials_update(
        &mut self,
        view: CredentialView,
        secrets: bmc_widget_protocol::CredentialSecrets,
    ) -> bool {
        let data = self.store.data_mut();
        data.credentials.replace(view);
        data.credential_secrets = secrets;
        self.fire_update_hook(
            self.on_credentials_update_func,
            "on_credentials_update",
            Lifecycle::CredentialsUpdate,
        )
    }

    /// Whether the widget exported `on_touch`.
    ///
    /// The host uses this to gate touch delivery: a widget without the hook is
    /// non-interactive, so its touch events are dropped rather than queued for a
    /// render that will never be requested.
    #[must_use]
    pub fn exports_on_touch(&self) -> bool {
        self.on_touch_func.is_some()
    }

    /// Notify the widget that touch activity occurred this drain.
    ///
    /// Unlike [`Self::deliver_params_update`] / [`Self::deliver_system_update`]
    /// there is no snapshot to stage — the touch events themselves are queued
    /// separately via [`Self::push_touch_event`] and consumed at the next
    /// `render`. This hook is purely the "a touch happened, decide whether to
    /// re-render" notification; the widget responds by calling `request_frame()`
    /// under [`Lifecycle::Touch`].
    ///
    /// Returns whether the guest's hook actually ran (`false` = no hook or a
    /// trap), matching the other deliver methods.
    pub fn deliver_touch(&mut self) -> bool {
        self.fire_update_hook(self.on_touch_func, "on_touch", Lifecycle::Touch)
    }

    /// Notify the widget that the Deck's SSID or IP changed.
    ///
    /// Like [`Self::deliver_touch`] there is no snapshot to stage —
    /// the caller already stored the new value via [`Self::set_network_info`],
    /// and the guest re-reads it through `host_network_info`.
    /// The hook is purely the "decide whether to re-render" notification;
    /// the widget responds by calling `request_frame()` under
    /// [`Lifecycle::NetworkUpdate`].
    ///
    /// Returns whether the guest's hook actually ran
    /// (`false` = no hook or a trap), matching the other deliver methods.
    pub fn deliver_network_update(&mut self) -> bool {
        self.fire_update_hook(
            self.on_network_update_func,
            "on_network_update",
            Lifecycle::NetworkUpdate,
        )
    }

    /// Mark a newly attached runtime dormant without delivering a lifecycle hook.
    pub fn initialize_dormant(&mut self) {
        debug_assert!(
            self.pending_hook.is_none(),
            "initial dormancy must be established before lifecycle edges are queued"
        );
        self.store.data_mut().mark_renderer_assets_dormant();
    }

    /// Queue the dormant edge; the hook fires later, in `poll_deliveries` scope.
    pub fn notify_dormant(&mut self) -> bool {
        self.store.data_mut().mark_renderer_assets_dormant();
        self.pending_hook = match self.pending_hook.take() {
            Some(PendingHook::Wake | PendingHook::WakeThenSleep) => {
                Some(PendingHook::WakeThenSleep)
            }
            Some(PendingHook::Sleep | PendingHook::SleepThenWake) | None => {
                Some(PendingHook::Sleep)
            }
        };
        self.on_sleep_func.is_some()
    }

    /// Queue the wake edge; the hook fires later, in `poll_deliveries` scope.
    pub fn notify_wake(&mut self) -> bool {
        self.pending_hook = match self.pending_hook.take() {
            Some(PendingHook::Sleep | PendingHook::SleepThenWake) => {
                Some(PendingHook::SleepThenWake)
            }
            Some(PendingHook::Wake | PendingHook::WakeThenSleep) | None => Some(PendingHook::Wake),
        };
        let state = self.store.data_mut();
        state.mark_renderer_assets_active();
        let now = state.monotonic_ms;
        state.frame_schedule.request_frame_after(0, now);
        self.on_wake_func.is_some()
    }

    #[must_use]
    pub fn has_pending_lifecycle(&self) -> bool {
        self.pending_hook.is_some()
    }

    /// Deliver queued hooks with renderer access for suspension and guest imports.
    pub(super) fn flush_pending_lifecycle(&mut self) {
        if self.pending_hook.is_some() && self.store.data().renderer_ptr.is_none() {
            return;
        }
        match self.pending_hook.take() {
            Some(PendingHook::Wake) => {
                self.fire_update_hook(self.on_wake_func, "on_wake", Lifecycle::Wake);
            }
            Some(PendingHook::Sleep) => {
                self.fire_sleep();
            }
            Some(PendingHook::SleepThenWake) => {
                self.store.data_mut().mark_renderer_assets_dormant();
                self.fire_update_hook(self.on_sleep_func, "on_sleep", Lifecycle::Sleep);
                self.store.data_mut().mark_renderer_assets_active();
                self.fire_update_hook(self.on_wake_func, "on_wake", Lifecycle::Wake);
            }
            Some(PendingHook::WakeThenSleep) => {
                self.fire_update_hook(self.on_wake_func, "on_wake", Lifecycle::Wake);
                self.fire_update_hook(self.on_sleep_func, "on_sleep", Lifecycle::Sleep);
            }
            None => {}
        }
    }

    fn fire_sleep(&mut self) {
        self.fire_update_hook(self.on_sleep_func, "on_sleep", Lifecycle::Sleep);
        match self.suspend_renderer_assets() {
            Ok(observation) => {
                self.last_asset_suspension = Some(observation);
                #[cfg(feature = "profiling")]
                tracing::info!(
                    target: bmc_render::profile::TARGET,
                    instance_id = %self.store.data().instance_id,
                    svg_suspended = observation.svg_suspended,
                    bitmap_suspended = observation.bitmap_suspended,
                    mesh_suspended = observation.mesh_suspended,
                    svg_heap_bytes_released = observation.svg_heap_bytes_released,
                    svg_path_bytes_released = observation.svg_path_bytes_released,
                    bitmap_released = observation.bitmap_released,
                    mesh_bytes_released = observation.mesh_bytes_released,
                    svg_path_bytes_resident_total_before = observation.svg_path_bytes_resident_total_before,
                    svg_path_bytes_resident_total_after = observation.svg_path_bytes_resident_total_after,
                    bitmap_resident_total_before = observation.bitmap_resident_total_before,
                    bitmap_resident_total_after = observation.bitmap_resident_total_after,
                    mesh_bytes_resident_total_before = observation.mesh_bytes_resident_total_before,
                    mesh_bytes_resident_total_after = observation.mesh_bytes_resident_total_after,
                    "widget renderer assets suspended"
                );
            }
            Err(error) => {
                tracing::error!(
                    instance_id = %self.store.data().instance_id,
                    %error,
                    "renderer asset suspension failed"
                );
                self.store.data_mut().renderer_asset_failure = Some(error);
            }
        }
    }

    fn suspend_renderer_assets(&mut self) -> Result<RendererAssetSuspensionObservation, String> {
        let records = self.store.data().renderer_assets.restorable();
        let Some(mut pointer) = self.store.data().renderer_ptr else {
            return Err("renderer unavailable during asset suspension".to_owned());
        };
        // SAFETY: lifecycle delivery runs inside `with_renderer`; no guest import is
        // active while this exact-tag pass owns the renderer reborrow.
        let renderer: &mut dyn Renderer = unsafe { pointer.as_mut() };
        let mut observation = RendererAssetSuspensionObservation::start(renderer);
        for (raw_tag, record) in records {
            let tag = self.store.data().namespaced_tag(&raw_tag);
            let mut newly_suspended = false;
            let result_matches = match (record.kind, record.id) {
                (RendererAssetKind::Svg, RendererAssetId::Svg(expected)) => {
                    #[cfg(feature = "profiling")]
                    let result = suspend_svg_profiled(
                        renderer,
                        &tag,
                        &self.store.data().instance_id,
                        &raw_tag,
                        &mut observation,
                    );
                    #[cfg(not(feature = "profiling"))]
                    let result = renderer.suspend_svg(&tag);
                    match result {
                        AssetSuspendResult::Suspended(id) if id == expected => {
                            newly_suspended = true;
                            true
                        }
                        AssetSuspendResult::AlreadySuspended(id) if id == expected => true,
                        AssetSuspendResult::Suspended(_)
                        | AssetSuspendResult::AlreadySuspended(_)
                        | AssetSuspendResult::Unknown => false,
                    }
                }
                (RendererAssetKind::Bitmap(_), RendererAssetId::Bitmap(expected)) => {
                    if matches!(renderer.bitmap_tag_state(&tag), AssetTagState::Resident(_)) {
                        self.store
                            .data_mut()
                            .require_renderer_gpu_access()
                            .map_err(|error| error.to_string())?;
                    }
                    let result = renderer.suspend_bitmap(&tag);
                    match result {
                        AssetSuspendResult::Suspended(id) if id == expected => {
                            newly_suspended = true;
                            true
                        }
                        AssetSuspendResult::AlreadySuspended(id) if id == expected => true,
                        AssetSuspendResult::Suspended(_)
                        | AssetSuspendResult::AlreadySuspended(_)
                        | AssetSuspendResult::Unknown => false,
                    }
                }
                (RendererAssetKind::Mesh, RendererAssetId::Mesh(expected)) => {
                    if matches!(renderer.mesh_tag_state(&tag), AssetTagState::Resident(_)) {
                        self.store
                            .data_mut()
                            .require_renderer_gpu_access()
                            .map_err(|error| error.to_string())?;
                    }
                    let result = renderer.suspend_mesh(&tag);
                    match result {
                        AssetSuspendResult::Suspended(id) if id == expected => {
                            newly_suspended = true;
                            true
                        }
                        AssetSuspendResult::AlreadySuspended(id) if id == expected => true,
                        AssetSuspendResult::Suspended(_)
                        | AssetSuspendResult::AlreadySuspended(_)
                        | AssetSuspendResult::Unknown => false,
                    }
                }
                _ => false,
            };
            if !result_matches {
                return Err(format!(
                    "asset reservation changed while suspending {raw_tag}"
                ));
            }
            self.store.data_mut().renderer_assets.mark_pending(&raw_tag);
            if newly_suspended {
                match record.kind {
                    RendererAssetKind::Svg => observation.svg_suspended += 1,
                    RendererAssetKind::Bitmap(_) => observation.bitmap_suspended += 1,
                    RendererAssetKind::Mesh => observation.mesh_suspended += 1,
                }
            }
            #[cfg(feature = "profiling")]
            tracing::info!(
                target: bmc_render::profile::TARGET,
                instance_id = %self.store.data().instance_id,
                tag = %raw_tag,
                asset_kind = record.kind.name(),
                asset_id = record.id.to_ffi(),
                newly_suspended,
                "widget renderer asset suspension observed"
            );
        }
        Ok(observation.finish(renderer))
    }

    /// Common tail of `deliver_params_update` / `deliver_system_update`:
    /// invoke the named guest export with a fresh fuel budget under
    /// the matching lifecycle phase, swallowing traps (the snapshot
    /// is already staged so the next `render` picks it up).
    fn fire_update_hook(
        &mut self,
        hook: Option<wasmi::TypedFunc<(), ()>>,
        hook_name: &'static str,
        phase: Lifecycle,
    ) -> bool {
        let Some(hook) = hook else {
            return false;
        };
        if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
            tracing::warn!("{hook_name}: could not set fuel: {e}");
            if self.guest_trap.is_none() {
                self.guest_trap = Some(anyhow::anyhow!("{hook_name}: could not set fuel: {e}"));
            }
            #[cfg(feature = "capture")]
            if self.lifecycle_trap.is_none() {
                self.lifecycle_trap = Some(anyhow::anyhow!("{hook_name}: could not set fuel: {e}"));
            }
            return false;
        }
        let call_result = in_lifecycle(&mut self.store, phase, |s| hook.call(s, ()));
        match call_result {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("{hook_name} trapped: {e}");
                if self.guest_trap.is_none() {
                    self.guest_trap = Some(anyhow::anyhow!("{hook_name} trapped: {e}"));
                }
                #[cfg(feature = "capture")]
                if self.lifecycle_trap.is_none() {
                    self.lifecycle_trap = Some(anyhow::anyhow!("{hook_name} trapped: {e}"));
                }
                false
            }
        }
    }

    /// Reject a capture replay after a lifecycle hook trapped.
    ///
    /// # Errors
    ///
    /// Returns the first hook trap since the previous call.
    #[cfg(feature = "capture")]
    pub fn take_lifecycle_trap(&mut self) -> Result<()> {
        match self.lifecycle_trap.take() {
            Some(trap) => Err(trap),
            None => Ok(()),
        }
    }

    /// Install `renderer` on `HostState::renderer_ptr` for the duration of `f`,
    /// then clear it on normal exit. Host imports inside `f` reach the renderer
    /// through `runtime::imports::with_renderer`, which traps the guest (does
    /// **not** panic the host) if called outside this scope.
    ///
    /// # Soundness
    /// Parking a `NonNull<dyn Renderer>` on `HostState::renderer_ptr` is itself
    /// harmless: the pointer is never dereferenced from this function's body.
    /// The actual deref happens inside `imports::with_renderer` (and the helper
    /// `imports::with_renderer_and_state`), and that is the operation whose
    /// soundness depends on the parked pointer being valid and exclusively
    /// borrowed for the duration of `f`.
    ///
    /// Callers must derive the pointer with `ptr::addr_of_mut!` (not
    /// `NonNull::from(&mut renderer)`) so the parent `&mut Renderer` reborrow
    /// does not enter the Tree-Borrows stack while parked. If a caller hands a
    /// stale or aliasing `NonNull` here, the eventual deref in
    /// `imports::with_renderer` is UB; this safe signature is the single
    /// documented contact point for that obligation.
    ///
    /// # Panic safety
    /// The parked pointer is cleared before a panic resumes unwinding.
    pub fn with_renderer<R>(
        &mut self,
        renderer: NonNull<dyn Renderer>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.store.data_mut().renderer_ptr = Some(renderer);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        self.store.data_mut().renderer_ptr = None;
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Per-component timing breakdown from the last rendered frame.
    #[must_use]
    pub fn last_timings(&self) -> FrameTimings {
        self.store.data().last_timings
    }

    /// Drains the fuel charged to each profiling section this frame.
    pub fn take_profile_sections(&mut self) -> std::collections::BTreeMap<String, u64> {
        std::mem::take(&mut self.store.data_mut().profile_sections)
    }

    /// Seal or unseal live I/O at runtime; while sealed every egress is refused
    /// (a fetch returns `FetchOutcome::Network`), simulating offline.
    pub fn set_hermetic(&mut self, sealed: bool) {
        let state = self.store.data_mut();
        match (sealed, state.hermetic.is_some()) {
            (true, false) => state.hermetic = Some(HermeticRun::default()),
            (false, true) => state.hermetic = None,
            _ => {}
        }
    }

    /// Breaches recorded during a hermetic run (empty otherwise).
    #[must_use]
    pub fn hermetic_breaches(&self) -> &[String] {
        self.store
            .data()
            .hermetic
            .as_ref()
            .map_or(&[], |run| run.breaches.as_slice())
    }

    /// Prefix used to namespace every host-managed asset tag for this widget.
    ///
    /// Every tag the widget registers (icons, bitmaps, meshes) is stored on
    /// the renderer under `<asset_namespace()>:<tag>`. Hosts that share a
    /// single renderer across multiple widgets call
    /// `renderer.evict_prefix(&runtime.asset_namespace())` when a widget
    /// goes away, so the renderer-side entries are reclaimed without
    /// dropping the renderer itself.
    #[must_use]
    pub fn asset_namespace(&self) -> String {
        self.store.data().instance_id.clone()
    }

    /// Whether the widget needs another frame rendered.
    ///
    /// Returns `true` after the widget calls `request_frame()` or
    /// `request_frame_after(ms)`, and also while cached-tree animations still
    /// require host-side replay frames. The host **must not** call
    /// [`Self::render`] when this returns `false` — doing so wastes CPU and GPU
    /// for an identical frame.
    ///
    /// When this returns `true`, check [`Self::next_frame_delay`] to see if
    /// the frame should be rendered immediately or after a delay.
    #[must_use]
    pub fn wants_next_frame(&self) -> bool {
        self.store.data().frame_schedule.wants_next_frame()
    }

    /// Delay before the next host wake, if another frame was requested.
    ///
    /// Returns `None` for immediate frames (`request_frame()`), or `Some(ms)`
    /// for delayed wakes. This may be shorter than the widget's original
    /// `request_frame_after(ms)` delay while cached-tree animations are active;
    /// the widget's semantic full-WASM deadline remains tracked separately.
    ///
    /// The host should **sleep or schedule one timer** for the delay — not
    /// busy-wait or render immediately.
    #[must_use]
    pub fn next_frame_delay(&self) -> Option<u32> {
        self.store.data().frame_schedule.effective_delay_ms()
    }

    /// Push a touch event to be processed next frame.
    pub fn push_touch_event(&mut self, event: bmc_render::interaction::TouchEvent) {
        self.store.data_mut().interaction.push_event(event);
    }

    /// The SDK version the widget was compiled with (major, minor, patch).
    #[must_use]
    pub fn sdk_version(&self) -> (u16, u16, u16) {
        self.sdk_version
    }

    /// The SDK version the host expects (major, minor, patch).
    #[must_use]
    pub fn host_sdk_version() -> (u16, u16, u16) {
        SDK_VERSION
    }

    /// Look up the screen-space bounds of a registered UI element by string ID.
    ///
    /// Delegates to [`InteractionState::element_bounds`]. Must be called after
    /// a render pass (hit regions are rebuilt each frame).
    #[must_use]
    pub fn element_bounds(&self, id: &str) -> Option<bmc_render::interaction::Rect> {
        self.store.data().interaction.element_bounds(id)
    }

    /// Return all registered hit region element IDs (sorted).
    #[must_use]
    pub fn element_ids(&self) -> Vec<&str> {
        self.store.data().interaction.element_ids()
    }

    /// Hit test against registered UI regions.
    ///
    /// Delegates to [`InteractionState::hit_test`]. Must be called after
    /// a render pass (hit regions are rebuilt each frame).
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> Option<String> {
        self.store.data().interaction.hit_test(x, y)
    }

    /// Whether a deferred render is pending (widget called `request_frame_after`
    /// and the deadline hasn't been reached yet).
    #[must_use]
    pub fn has_deferred_render(&self) -> bool {
        let state = self.store.data();
        state
            .frame_schedule
            .deferred_wasm_render_at_ms
            .is_some_and(|deadline| state.monotonic_ms < deadline)
    }

    /// Get the instance for additional exports.
    #[must_use]
    pub fn instance(&self) -> &wasmi::Instance {
        &self.instance
    }

    #[cfg(feature = "capture")]
    #[must_use]
    pub fn exported_global_i32(&self, name: &str) -> Option<i32> {
        self.instance
            .get_global(&self.store, name)?
            .get(&self.store)
            .i32()
    }

    /// Observe the committed renderer-asset renderability state.
    ///
    /// Hidden from the supported runtime API; exposed for integration testing.
    #[doc(hidden)]
    #[must_use]
    pub fn renderer_assets_are_dormant_for_test(&self) -> bool {
        self.store.data().renderer_assets_are_dormant()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn last_asset_suspension_for_test(&self) -> Option<RendererAssetSuspensionObservation> {
        self.last_asset_suspension
    }

    #[doc(hidden)]
    #[must_use]
    pub fn last_asset_restoration_for_test(&self) -> Option<RendererAssetRestorationObservation> {
        self.store.data().last_asset_restoration
    }

    #[doc(hidden)]
    #[must_use]
    pub fn renderer_asset_failure_for_test(&self) -> Option<&str> {
        self.store.data().renderer_asset_failure.as_deref()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn frame_counter_for_test(&self) -> u64 {
        self.store.data().frame_counter
    }

    #[doc(hidden)]
    pub fn replay_cached_tree_for_test(
        &mut self,
        renderer: NonNull<dyn Renderer>,
        delta_ms: u32,
    ) -> bool {
        self.with_renderer(renderer, |runtime| {
            Self::render_cached_tree(runtime.store.data_mut(), delta_ms)
        })
    }

    /// Test-only escape hatch: call an arbitrary `() -> i32` export by name.
    ///
    /// Used by `tests/lifecycle.rs` to read observation counters out of hand-rolled
    /// WAT probe widgets without exposing the full `Store<HostState>` to test code.
    /// Returns `None` if the widget doesn't export `name` or the call traps —
    /// the test then fails with a clear "missing export X" assertion at the call site.
    ///
    /// Marked `#[doc(hidden)]` because it is not part of the supported runtime API.
    /// `#[cfg(test)]` is not viable here because integration tests in `tests/` see
    /// the crate as a regular dependency, not with the `test` cfg set.
    #[doc(hidden)]
    pub fn call_export_i32(&mut self, name: &str) -> Option<i32> {
        let func = self
            .instance
            .get_typed_func::<(), i32>(&self.store, name)
            .ok()?;
        func.call(&mut self.store, ()).ok()
    }

    /// Test-only escape hatch: call an arbitrary `() -> i64` export by name.
    /// Used for `last_version` observations that thread the `u64` version counter
    /// through `i64` because wasmi typed funcs round-trip i64 cleanly.
    ///
    /// Same scoping notes as [`Self::call_export_i32`].
    #[doc(hidden)]
    pub fn call_export_i64(&mut self, name: &str) -> Option<i64> {
        let func = self
            .instance
            .get_typed_func::<(), i64>(&self.store, name)
            .ok()?;
        func.call(&mut self.store, ()).ok()
    }

    fn run_unload_with_fresh_fuel(&mut self) {
        if let Some(unload) = self.unload_func {
            if let Err(e) = self.store.set_fuel(self.fuel_per_frame) {
                tracing::warn!("unload: could not set fuel: {e}");
            } else if let Err(e) =
                in_lifecycle(&mut self.store, Lifecycle::Unload, |s| unload.call(s, ()))
            {
                tracing::warn!("unload trapped: {e}");
            }
        }
        #[cfg(feature = "testing")]
        {
            self.store.data_mut().unload_ran = true;
        }
    }

    #[cfg(feature = "testing")]
    #[must_use]
    pub fn test_progress_counter(&self) -> u64 {
        self.store.data().delivered_events
    }

    #[cfg(test)]
    #[must_use]
    pub fn test_geometry(
        &self,
    ) -> (
        bmc_wasm_protocol::ViewportShape,
        u32,
        u32,
        bmc_wasm_protocol::DisplayShape,
        u32,
    ) {
        let s = self.store.data();
        (
            s.viewport_shape,
            s.display_width,
            s.display_height,
            s.display_shape,
            s.display_dpi,
        )
    }

    #[cfg(feature = "testing")]
    #[must_use]
    pub fn test_unload_ran(&self) -> bool {
        self.store.data().unload_ran
    }
}

#[cfg(feature = "testing")]
impl WasmWidgetRuntime {
    pub fn test_kick_fetch(&mut self) -> std::thread::JoinHandle<()> {
        let state = self.store.data_mut();
        let fetch_tx = state.fetches.test_settle_sender();
        state.fetches.queue_delayed(crate::host_api::DelayedFetch {
            fire_at_ms: 0,
            method: String::new(),
            url: String::new(),
            headers: Vec::new(),
            body: None,
            timeout: std::time::Duration::from_secs(10),
            request_id: bmc_wasm_protocol::FetchRequestId::alloc(&mut state.next_request_id),
        });
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(2));
                if fetch_tx
                    .send(crate::host_api::CompletedFetch::test_sentinel())
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    pub fn test_kick_ws_connect(&mut self) -> std::thread::JoinHandle<()> {
        let state = self.store.data_mut();
        let (msg_tx, msg_rx) = std::sync::mpsc::channel::<crate::host_api::WsOutbound>();
        let (_evt_tx, event_rx) = std::sync::mpsc::channel();
        let id = bmc_wasm_protocol::WebsocketId::alloc(&mut state.next_ws_id);
        state
            .websockets
            .insert(id, crate::host_api::ActiveWebSocket { msg_tx, event_rx });
        std::thread::spawn(move || while msg_rx.recv().is_ok() {})
    }

    pub fn test_kick_mdns_browse(&mut self) -> std::thread::JoinHandle<()> {
        let state = self.store.data_mut();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (_evt_tx, event_rx) = std::sync::mpsc::channel();
        let id = bmc_wasm_protocol::MdnsBrowseId::alloc(&mut state.next_mdns_browse_id);
        state
            .mdns_browses
            .insert(id, crate::host_api::ActiveMdnsBrowse { event_rx, stop_tx });
        std::thread::spawn(move || {
            let _ = stop_rx.recv();
        })
    }

    pub fn test_start_mdns_browse(&mut self, service_type: &str) {
        use super::background::mdns_browse_thread;
        use crate::host_api::{ActiveMdnsBrowse, MdnsEvent};
        use bmc_wasm_protocol::MdnsBrowseId;

        let svc = if service_type.ends_with(".local.") {
            service_type.to_owned()
        } else {
            format!("{service_type}.local.")
        };

        let state = self.store.data_mut();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<MdnsEvent>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let browse_id = MdnsBrowseId::alloc(&mut state.next_mdns_browse_id);
        state
            .mdns_browses
            .insert(browse_id, ActiveMdnsBrowse { event_rx, stop_tx });
        std::thread::spawn(move || {
            mdns_browse_thread(vec![svc], event_tx, stop_rx);
        });
    }

    pub fn test_register_mdns(
        &mut self,
        service_type: &str,
        instance_name: &str,
        host: &str,
        addr: &str,
        port: u16,
    ) {
        use crate::host_api::ActiveMdnsRegistration;
        use bmc_wasm_protocol::MdnsRegId;
        use std::collections::HashMap;

        let svc_type = if service_type.ends_with(".local.") {
            service_type.to_owned()
        } else {
            format!("{service_type}.local.")
        };

        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("test_register_mdns: daemon creation failed: {e}");
                return;
            }
        };

        let info = match mdns_sd::ServiceInfo::new(
            &svc_type,
            instance_name,
            host,
            addr,
            port,
            HashMap::<String, String>::new(),
        ) {
            Ok(info) => info,
            Err(e) => {
                tracing::error!("test_register_mdns: ServiceInfo creation failed: {e}");
                return;
            }
        };
        let fullname = info.get_fullname().to_owned();

        if let Err(e) = daemon.register(info) {
            tracing::error!("test_register_mdns: register failed: {e}");
            return;
        }

        let state = self.store.data_mut();
        let reg_id = MdnsRegId::alloc(&mut state.next_mdns_reg_id);
        state
            .mdns_registrations
            .insert(reg_id, ActiveMdnsRegistration { daemon, fullname });
    }

    pub fn test_take_mdns_events(&mut self) -> Vec<crate::host_api::CapturedMdnsEvent> {
        std::mem::take(&mut self.store.data_mut().mdns_captured_events)
    }
}

impl Drop for WasmWidgetRuntime {
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.run_unload_with_fresh_fuel();
        }));
        self.store.data_mut().shutdown_workers();
        let evicted = self.store.data_mut().evict_widget();
        if evicted > 0 {
            tracing::debug!("widget teardown evicted {evicted} asset(s)");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        DeadOverlayBackground, DisplayInfo, RuntimeConfig, WasmWidgetModule, WasmWidgetRuntime,
    };
    use bmc_wasm_protocol::{DisplayShape, ViewportShape};

    /// Minimal SDK-version-shaped widget so `WasmWidgetRuntime::new` finishes
    /// instantiation without needing a renderer or GL context.
    fn minimal_wat() -> String {
        format!(
            r#"
        (module
          (memory (export "memory") 1)
          (func (export "__bmc_sdk_init") (result i64)
            i64.const {})
          (func (export "render") (param i32)))
        "#,
            bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION)
        )
    }

    fn shared_state_wat() -> String {
        format!(
            r#"
        (module
          (memory (export "memory") 1)
          (global $value (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64)
            i64.const {})
          (func (export "render") (param i32))
          (func $recurse (param $depth i32)
            local.get $depth
            i32.const 0
            i32.gt_s
            if
              local.get $depth
              i32.const 1
              i32.sub
              call $recurse
            end)
          (func (export "exercise_stack") (result i32)
            i32.const 8
            call $recurse
            global.get $value
            i32.const 1
            i32.add
            global.set $value
            i32.const 0
            i32.const 0
            i32.load8_u
            i32.const 1
            i32.add
            i32.store8
            global.get $value)
          (func (export "memory_value") (result i32)
            i32.const 0
            i32.load8_u)
          (func (export "global_value") (result i32)
            global.get $value))
        "#,
            bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
        )
    }

    fn fuel_probe_wat() -> String {
        format!(
            r#"
        (module
          (memory (export "memory") 1)
          (global $completed (mut i32) (i32.const 0))
          (func (export "__bmc_sdk_init") (result i64)
            i64.const {})
          (func (export "render") (param i32))
          (func (export "on_params_update") (local $i i32)
            (block $done
              (loop $again
                local.get $i
                i32.const 10000
                i32.ge_u
                br_if $done
                local.get $i
                i32.const 1
                i32.add
                local.set $i
                br $again))
            i32.const 1
            global.set $completed)
          (func (export "completed") (result i32)
            global.get $completed))
        "#,
            bmc_wasm_protocol::version_pack(bmc_wasm_protocol::SDK_VERSION),
        )
    }

    fn runtime_from_module(module: &WasmWidgetModule, config: RuntimeConfig) -> WasmWidgetRuntime {
        WasmWidgetRuntime::from_module(
            module,
            320,
            240,
            ViewportShape::Rectangular,
            DisplayInfo {
                width: 320,
                height: 240,
                shape: DisplayShape::Rectangular,
                dpi: 1,
            },
            chrono::Local::now().fixed_offset(),
            config,
        )
        .expect("BUG: shared-module test runtime must construct")
    }

    fn minimal_runtime() -> WasmWidgetRuntime {
        let wasm = wat::parse_str(minimal_wat()).expect("BUG: minimal WAT must parse");
        WasmWidgetRuntime::new(
            &wasm,
            480,
            480,
            ViewportShape::Round,
            DisplayInfo {
                width: 480,
                height: 480,
                shape: DisplayShape::Rectangular,
                dpi: 42,
            },
            chrono::Local::now().fixed_offset(),
            RuntimeConfig::default(),
        )
        .expect("BUG: runtime should construct from the minimal fixture")
    }

    #[test]
    fn new_installs_geometry_on_host_state() {
        let rt = minimal_runtime();
        assert_eq!(
            rt.test_geometry(),
            (
                ViewportShape::Round,
                480,
                480,
                DisplayShape::Rectangular,
                42
            )
        );
    }

    #[test]
    fn panic_clears_renderer_gpu_access_callback() {
        let mut runtime = minimal_runtime();
        let mut require_gpu_access = || Ok(());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.with_renderer_gpu_access(&mut require_gpu_access, |_| {
                panic!("fixture delivery panic")
            });
        }));

        assert!(result.is_err(), "the fixture panic must propagate");
        assert!(
            !runtime.store.data().renderer_gpu_access_is_installed(),
            "unwinding must not leave a callback pointer into the dead delivery frame"
        );
    }

    #[test]
    fn module_instances_keep_store_memory_and_host_state_independent() {
        let wasm = wat::parse_str(shared_state_wat()).expect("BUG: shared-state WAT must parse");
        let module =
            WasmWidgetModule::compile(&wasm).expect("BUG: shared-state module must compile");
        let mut first = runtime_from_module(
            &module,
            RuntimeConfig {
                instance_token: Some("first".to_owned()),
                ..RuntimeConfig::default()
            },
        );
        let mut second = runtime_from_module(
            &module,
            RuntimeConfig {
                instance_token: Some("second".to_owned()),
                ..RuntimeConfig::default()
            },
        );

        assert_eq!(first.call_export_i32("exercise_stack"), Some(1));
        assert_eq!(first.call_export_i32("memory_value"), Some(1));
        assert_eq!(
            second.call_export_i32("memory_value"),
            Some(0),
            "one Store's memory must not affect another"
        );
        assert_eq!(
            second.call_export_i32("global_value"),
            Some(0),
            "one Store's globals must not affect another"
        );
        assert_ne!(
            first.asset_namespace(),
            second.asset_namespace(),
            "instance identity must remain per Store"
        );
    }

    #[test]
    fn renderer_asset_failure_overlay_replaces_the_partial_frame() {
        let renderer_failure = DeadOverlayBackground::for_stopped_widget(Some("missing asset"));
        let fuel_failure = DeadOverlayBackground::for_stopped_widget(None);

        assert_eq!(renderer_failure, DeadOverlayBackground::ReplaceFrame);
        assert_eq!(renderer_failure.scrim().alpha(), 255);
        assert_eq!(fuel_failure, DeadOverlayBackground::PreserveFrame);
        assert!(fuel_failure.scrim().alpha() < 255);
    }

    #[test]
    fn recycled_engine_stack_preserves_store_isolation() {
        let wasm = wat::parse_str(shared_state_wat()).expect("BUG: shared-state WAT must parse");
        let module =
            WasmWidgetModule::compile(&wasm).expect("BUG: shared-state module must compile");
        let mut runtimes: Vec<_> = (0..5)
            .map(|_| runtime_from_module(&module, RuntimeConfig::default()))
            .collect();

        for runtime in &mut runtimes {
            assert_eq!(runtime.call_export_i32("exercise_stack"), Some(1));
        }
        assert_eq!(runtimes[0].call_export_i32("exercise_stack"), Some(2));

        for (index, runtime) in runtimes.iter_mut().enumerate() {
            let expected = if index == 0 { 2 } else { 1 };
            assert_eq!(runtime.call_export_i32("memory_value"), Some(expected));
            assert_eq!(runtime.call_export_i32("global_value"), Some(expected));
        }
    }

    #[test]
    fn shared_module_keeps_each_stores_fuel_budget_independent() {
        let wasm = wat::parse_str(fuel_probe_wat()).expect("BUG: fuel-probe WAT must parse");
        let module = WasmWidgetModule::compile(&wasm).expect("BUG: fuel-probe module must compile");
        let mut low = runtime_from_module(
            &module,
            RuntimeConfig {
                fuel_per_frame: 1_000,
                ..RuntimeConfig::default()
            },
        );
        let mut normal = runtime_from_module(&module, RuntimeConfig::default());

        assert_eq!(low.call_export_i32("completed"), Some(0));
        assert_eq!(normal.call_export_i32("completed"), Some(0));
        assert!(!low.deliver_params_update(BTreeMap::new()));
        let error = low
            .poll_deliveries()
            .expect_err("fuel exhaustion must reject the trapped runtime");
        assert!(error.to_string().contains("all fuel consumed"));
        assert!(normal.deliver_params_update(BTreeMap::new()));
        assert_eq!(normal.call_export_i32("completed"), Some(1));
    }
}
