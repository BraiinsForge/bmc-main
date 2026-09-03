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

//! Run subcommand — headless capture of a single widget at a given size.

use chrono::{DateTime, FixedOffset};
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::io::IsTerminal;
#[cfg(target_os = "linux")]
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use glow::HasContext;
#[cfg(target_os = "linux")]
use glutin::config::ConfigTemplateBuilder;
#[cfg(target_os = "linux")]
use glutin::context::{ContextApi, ContextAttributesBuilder};
#[cfg(target_os = "linux")]
use glutin::display::{Display, GetGlDisplay};
#[cfg(target_os = "linux")]
use glutin::prelude::*;
#[cfg(target_os = "linux")]
use glutin::surface::{PbufferSurface, SurfaceAttributesBuilder};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::interaction::TouchEvent;
use bmc_render::renderer::Renderer;
use bmc_wasm_protocol::{MdnsBrowseId, SocketId, SsdpSearchId, UdpBroadcastId, WebsocketId};
use bmc_wasm_runtime::capture_config::CaptureConfig;
use bmc_wasm_runtime::platform_catalog::{DisplayShape, Target, manifest_viewport_shape};
use bmc_wasm_runtime::unified_fixture::{
    FixtureHeader, TimelineEvent, UnifiedEvent, UnifiedFixture, load_unified_fixture,
    validate_fixture,
};
use bmc_wasm_runtime::{
    DiskCache, FixtureEvent, FixtureEventKind, InterceptedReply, PackageAssetStore, RenderStatus,
    RuntimeConfig, SystemSnapshot, WasmWidgetRuntime,
};

/// Fixed timestep per frame (ms).
const DELTA_MS: u32 = 16;

/// Number of frames for a synthetic drag gesture.
const DRAG_FRAMES: u32 = 10;

const ONLINE_IO_DRAIN_TIMEOUT: Duration = Duration::from_secs(15);
const OFFLINE_IO_DRAIN_TIMEOUT: Duration = Duration::from_mins(1);

// ── Public interface ────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub enum StackProfiling {
    Disabled,
    Enabled { expected_origin: i32 },
}

impl StackProfiling {
    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled { .. })
    }

    fn expected_origin(self) -> Option<i32> {
        match self {
            Self::Disabled => None,
            Self::Enabled { expected_origin } => Some(expected_origin),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub enum LayoutCacheProfiling {
    Enabled,
    #[default]
    Disabled,
}

impl LayoutCacheProfiling {
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

impl From<bool> for LayoutCacheProfiling {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

impl From<Option<i32>> for StackProfiling {
    fn from(expected_origin: Option<i32>) -> Self {
        expected_origin.map_or(Self::Disabled, |expected_origin| Self::Enabled {
            expected_origin,
        })
    }
}

/// Arguments passed from the CLI `run` subcommand.
pub struct RunArgs {
    pub wasm_path: PathBuf,
    pub asset_root: Option<PathBuf>,
    /// `<platform>:<viewport>` to render.
    pub target: Option<String>,
    /// Which dataset from `[fixtures]` to replay. Defaults to the only one
    /// bound to the target.
    pub dataset: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub fixture: Option<PathBuf>,
    /// Path to the `capture/` directory containing `config.toml` and fixtures.
    /// Optional in `--online` mode.
    pub capture_dir: Option<PathBuf>,
    /// Preview against live data: run non-hermetic so the widget fetches its
    /// own data source, and wait for the response before the shot.
    pub online: bool,
    /// Render every configured (dataset, target) pair the manifest supports,
    /// or with `--online`, every target the catalog offers. Ignores `--target`.
    pub all_targets: bool,
    pub stack_profiling: StackProfiling,
    pub layout_cache_profiling: LayoutCacheProfiling,
}

/// One (dataset, target) capture.
struct CaptureCtx {
    wasm_path: PathBuf,
    runtime_wasm_path: PathBuf,
    asset_root: PathBuf,
    target: Target,
    /// Dataset name, and the last component of the frame directory.
    dataset: String,
    /// The target viewport's dimensions, carried alongside it
    /// because the render path reaches for them on nearly every line.
    width: u32,
    height: u32,
    output_dir: PathBuf,
    fixture: Option<PathBuf>,
    online: bool,
    stack_profiling: StackProfiling,
    layout_cache_profiling: LayoutCacheProfiling,
}

impl CaptureCtx {
    #[expect(
        clippy::too_many_arguments,
        reason = "one capture's whole identity, each part from a different source"
    )]
    fn new(
        wasm_path: PathBuf,
        prepared: &bmc_wasm_runtime::fixtures::PreparedWidget,
        target: Target,
        dataset: String,
        output_dir: PathBuf,
        fixture: Option<PathBuf>,
        online: bool,
        stack_profiling: StackProfiling,
        layout_cache_profiling: LayoutCacheProfiling,
    ) -> Self {
        Self {
            wasm_path,
            runtime_wasm_path: prepared.wasm_path().to_owned(),
            asset_root: prepared.asset_root().to_owned(),
            target,
            dataset,
            width: target.viewport.width,
            height: target.viewport.height,
            output_dir,
            fixture,
            online,
            stack_profiling,
            layout_cache_profiling,
        }
    }
}

pub fn execute(args: RunArgs) -> Result<()> {
    let config = match &args.capture_dir {
        Some(dir) => bmc_wasm_runtime::capture_config::load_from_capture_dir(dir)?,
        None => CaptureConfig::default(),
    };

    let prepared = bmc_wasm_runtime::fixtures::PreparedWidget::new(
        &args.wasm_path,
        args.asset_root.as_deref(),
    )?;

    if args.all_targets {
        return run_all_supported_targets(&args, &config, &prepared);
    }

    let target: Target = args
        .target
        .context("--target=<platform>:<viewport> is required")?
        .parse()?;
    let output_dir = args.output_dir.context("--output=<dir> is required")?;
    let dataset = match args.dataset {
        Some(name) => name,
        None => sole_dataset_for(&config, target, args.online || args.fixture.is_some())?,
    };

    let manifest = load_widget_manifest(&args.wasm_path, args.capture_dir.as_deref())?;
    let ctx = CaptureCtx::new(
        args.wasm_path,
        &prepared,
        target,
        dataset,
        output_dir,
        args.fixture,
        args.online,
        args.stack_profiling,
        args.layout_cache_profiling,
    );

    run_capture(&ctx, &config, &manifest)
}

/// The one dataset bound to a target, when `--dataset` is omitted.
///
/// `--online` and an explicit `--fixture` bring their own data,
/// so they need no configured dataset — only a name to file frames under.
fn sole_dataset_for(
    config: &CaptureConfig,
    target: Target,
    brings_own_data: bool,
) -> Result<String> {
    let bound: Vec<&str> = config
        .capture_matrix()
        .into_iter()
        .filter(|(_, t)| t.platform.id == target.platform.id && t.viewport.id == target.viewport.id)
        .map(|(dataset, _)| dataset)
        .collect();
    match bound.as_slice() {
        [] if brings_own_data => Ok(target.viewport.id.to_owned()),
        [] => bail!(
            "target '{target}' has no dataset — add a [fixtures.<name>] entry \
             for it, or pass --fixture or --online"
        ),
        [only] => Ok((*only).to_owned()),
        many => bail!(
            "target '{target}' has {} datasets ({}) — pass --dataset to pick one",
            many.len(),
            many.join(", "),
        ),
    }
}

// ── Capture entry point ─────────────────────────────────────────────

/// Resolve the offline replay fixture: an explicit `--fixture` wins,
/// else the dataset's `config.toml` entry. `None` = no such dataset.
fn offline_fixture_path(ctx: &CaptureCtx, config: &CaptureConfig) -> Option<PathBuf> {
    ctx.fixture
        .clone()
        .or_else(|| config.fixtures.get(&ctx.dataset).map(|e| e.path.clone()))
}

fn run_capture(
    ctx: &CaptureCtx,
    config: &CaptureConfig,
    manifest: &bmc_widget_manifest::Manifest,
) -> Result<()> {
    // Checked here so every route is admitted alike: no baseline for geometry
    // the device refuses and the testbed paints as a declined slab.
    if let Err(declined) = target_admitted(manifest, ctx.target) {
        bail!("{}: {declined}", ctx.target);
    }

    if ctx.online {
        return run_unified_capture(
            ctx,
            config,
            manifest,
            &synth_online_fixture(),
            "online (live data)",
        );
    }

    let fixture_path = offline_fixture_path(ctx, config).with_context(|| {
        // Try to extract the example name from the WASM path for a helpful hint.
        let example_name = bmc_wasm_runtime::fixtures::find_widget_root(&ctx.wasm_path)
            .and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "<name>".into());
        format!(
            "no fixture '{}' — record one with: just wasm::record {example_name} {} {}",
            ctx.dataset, ctx.target, ctx.dataset
        )
    })?;
    let fixture = load_unified_fixture(&fixture_path)
        .with_context(|| format!("failed to load fixture {}", fixture_path.display()))?;
    run_unified_capture(
        ctx,
        config,
        manifest,
        &fixture,
        &fixture_path.display().to_string(),
    )
    .with_context(|| {
        format!(
            "replay failed — frames captured so far are in {}",
            ctx.output_dir.display()
        )
    })
}

/// A fixture with no recorded I/O: the widget starts at the current instant
/// and a single capture fires at 500 ms — enough for its poll to dispatch
/// and (in `--online` mode) the live fetch to be drained before the shot.
fn synth_online_fixture() -> UnifiedFixture {
    // Online previews reflect the host: seed the deck timezone
    // from the machine's IANA zone so date/time captions render
    // in the local zone rather than the empty default.
    let mut initial_system = SystemSnapshot::default();
    if let Ok(tz) = iana_time_zone::get_timezone() {
        initial_system.settings.timezone = tz;
    }
    UnifiedFixture {
        header: FixtureHeader {
            time: chrono::Utc::now().to_rfc3339(),
            kv: HashMap::new(),
            initial_params: serde_json::Map::new(),
            initial_system,
            initial_credentials: serde_json::Map::new(),
        },
        events: vec![TimelineEvent {
            at_ms: 500,
            event: UnifiedEvent::Capture {
                duration_ms: None,
                fps: None,
            },
        }],
    }
}

/// Load the widget's `manifest.json` sitting next to its wasm binary.
/// The widget's manifest, from wherever this run can reach its root.
///
/// A nix build hands over a store path, not the wasm in its own source tree,
/// while `--capture-dir` still points inside the widget.
fn load_widget_manifest(
    wasm_path: &Path,
    capture_dir: Option<&Path>,
) -> Result<bmc_widget_manifest::Manifest> {
    let manifest_path = [
        bmc_wasm_runtime::fixtures::find_widget_root(wasm_path),
        capture_dir.and_then(|dir| dir.parent().map(Path::to_owned)),
    ]
    .into_iter()
    .flatten()
    .map(|root| root.join("manifest.json"))
    .find(|candidate| candidate.is_file())
    .context("cannot locate the widget's manifest.json from the wasm path or --capture-dir")?;
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    text.parse()
        .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", manifest_path.display()))
}

/// Manifest default params for an `--online` preview.
/// Render every (dataset, target) pair into `<output>/<platform>/<viewport>/<dataset>/`,
/// skipping targets the widget's manifest declines.
///
/// Offline that is the configured matrix. `--online` replays no dataset,
/// so it sweeps the catalog and names each shot after its viewport,
/// as `sole_dataset_for` does for a single target.
fn run_all_supported_targets(
    args: &RunArgs,
    config: &CaptureConfig,
    prepared: &bmc_wasm_runtime::fixtures::PreparedWidget,
) -> Result<()> {
    let output_base = args
        .output_dir
        .clone()
        .context("--output=<dir> is required")?;
    let manifest = load_widget_manifest(&args.wasm_path, args.capture_dir.as_deref())?;

    let matrix = if args.online {
        catalog_matrix()
    } else {
        config.capture_matrix()
    };
    if matrix.is_empty() {
        bail!("no datasets configured — add a [fixtures.<name>] entry or use --online");
    }

    let mut rendered = 0_usize;
    for (dataset, target) in matrix {
        if let Err(declined) = target_admitted(&manifest, target) {
            eprintln!("→ {target} {dataset}: {declined}, skipping");
            continue;
        }
        eprintln!("→ {target} {dataset}");
        run_capture(
            &CaptureCtx::new(
                args.wasm_path.clone(),
                prepared,
                target,
                dataset.to_owned(),
                CaptureConfig::frame_dir(&output_base, dataset, target),
                args.fixture.clone(),
                args.online,
                args.stack_profiling,
                args.layout_cache_profiling,
            ),
            config,
            &manifest,
        )?;
        rendered += 1;
    }
    if rendered == 0 {
        bail!("the widget's manifest declines every target on offer");
    }
    Ok(())
}

/// Every target the catalog offers, each named after its viewport.
fn catalog_matrix() -> Vec<(&'static str, Target)> {
    bmc_wasm_runtime::platform_catalog::PLATFORMS
        .iter()
        .flat_map(|platform| {
            platform
                .viewports
                .iter()
                .map(move |viewport| (viewport.id, Target { platform, viewport }))
        })
        .collect()
}

/// Whether the widget's manifest admits a target, DPI included.
///
/// The verdict is returned rather than acted on: iterating the whole catalog
/// skips what a widget declines, where a named target is a mistake to report.
fn target_admitted(
    manifest: &bmc_widget_manifest::Manifest,
    target: Target,
) -> Result<(), bmc_widget_manifest::ViewportDeclined> {
    manifest.admits_viewport_at_dpi(
        manifest_viewport_shape(target.viewport.shape),
        target.viewport.width,
        target.viewport.height,
        target.platform.display().dpi,
    )
}

// ── Unified fixture replay ──────────────────────────────────────────

/// Run capture using a unified fixture file — the new replay path.
///
/// Loads and validates the fixture, extracts fetch interceptors and network
/// events, seeds KV from the fixture header, then advances virtual time
/// frame-by-frame dispatching events at their `at_ms` timestamps.
#[expect(
    clippy::too_many_lines,
    clippy::integer_division,
    clippy::cast_precision_loss,
    reason = "single-flow replay routine; splitting purely for line count would obscure the \
              event-cursor / time-cursor coupling, and the precision-loss casts are \
              capture-step math on small bounded integers"
)]
fn run_unified_capture(
    ctx: &CaptureCtx,
    config: &CaptureConfig,
    manifest: &bmc_widget_manifest::Manifest,
    fixture: &UnifiedFixture,
    source_label: &str,
) -> Result<()> {
    validate_fixture(fixture)?;

    let widget_name = ctx
        .wasm_path
        .file_stem()
        .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
    eprintln!(
        "Unified replay: {source_label} ({} events) for {widget_name} at {}x{}",
        fixture.events.len(),
        ctx.width,
        ctx.height
    );

    // Parse start time from fixture header (overrides config/CLI).
    // Accepts any RFC 3339 timestamp — must include timezone offset;
    // fractional seconds are optional (the testbed's `Local::now().to_rfc3339()`
    // emits them, hand-authored fixtures usually don't).
    let mut system_time =
        chrono::DateTime::parse_from_rfc3339(&fixture.header.time).with_context(|| {
            format!(
                "invalid time '{}' in fixture header — must be RFC 3339 with timezone (e.g. 2026-03-10T18:00:00+02:00)",
                fixture.header.time
            )
        })?;

    // Prepare KV directory — seed from fixture header KV, not secrets.ini
    let kv_dir = prepare_unified_kv_dir(ctx, config, &widget_name, fixture);
    let blob_dir = prepare_unified_blob_dir(ctx, &widget_name);

    // Extract fetch interceptors and network events from the unified timeline
    let (fetch_interceptor, network_events) = split_unified_events(fixture);

    // Initial params snapshot — baked into the fixture header so replay
    // is fully  self-contained (no `manifest.json` lookup at replay time).
    // Online previews have no fixture params, so seed the manifest defaults.
    let initial_params = if ctx.online {
        bmc_wasm_runtime::manifest_default_params(manifest)
    } else {
        bmc_wasm_runtime::parse_params_json(&fixture.header.initial_params)
            .expect("BUG: capture fixture initial_params must be valid")
    };

    // Initial system snapshot — serde-deserialised at fixture load;
    // `#[serde(default)]` on each field lets older fixtures fall back
    // to typed defaults instead of failing load.
    let initial_system = fixture.header.initial_system.clone();

    // Build runtime config
    let mut rt_config = RuntimeConfig {
        kv_store_path: Some(kv_dir),
        asset_cache: Some(DiskCache::new(blob_dir, CAPTURE_CACHE_MAX_BYTES)),
        package_assets: Some(PackageAssetStore::new(&ctx.asset_root)),
        mesh_msaa_samples: 4,
        rng_seed: Some(42),
        // Captures are hermetic (unmatched live I/O fails the run) except in
        // `--online` preview mode, where the widget fetches its own data.
        hermetic: !ctx.online,
        params: initial_params,
        system: initial_system,
        credentials: bmc_wasm_runtime::parse_credentials_json(&fixture.header.initial_credentials),
        ..RuntimeConfig::default()
    };
    if !fetch_interceptor.is_empty() {
        let fetches = std::sync::Arc::new(fetch_interceptor);
        let counters = std::sync::Arc::new(std::sync::Mutex::new(HashMap::<String, usize>::new()));
        rt_config.fetch_interceptor = Some(Box::new(move |method, url| {
            let key = format!("{method} {url}");
            let queue = fetches.get(&key)?;
            let mut counters = counters.lock().expect("BUG: fetch counter mutex poisoned");
            let counter = counters.entry(key).or_default();
            let idx = (*counter).min(queue.len().saturating_sub(1));
            *counter = counter.saturating_add(1);
            let fix = &queue[idx];
            Some(InterceptedReply {
                status: fix.status,
                headers: fix.headers.clone(),
                body: fix.body.clone(),
            })
        }));
    }
    rt_config.event_fixtures = network_events;

    let (gl, fbo, _keep_alive, mut renderer, mut runtime) =
        setup_gl_and_runtime(ctx, system_time, rt_config)?;
    // A device slot is born dormant and woken before its first frame.
    // Both calls only queue: the hooks reach the guest in `deliver_all_io`,
    // which runs before every render below.
    runtime.initialize_dormant();
    runtime.notify_wake();
    if ctx.layout_cache_profiling.is_enabled() {
        renderer.enable_text_layout_profiling();
    }

    let (major, minor, patch) = runtime.sdk_version();
    eprintln!(
        "Capturing {widget_name} at {}x{} (SDK {major}.{minor}.{patch})",
        ctx.width, ctx.height
    );

    // ── Main replay loop ────────────────────────────────────────────
    //
    // Collect user-action events (Capture, Click, Scroll, Drag) with their
    // timestamps.  Network events and fetches are handled by the runtime's
    // inject_fixture_events and fetch_interceptor respectively.
    //
    // Exhaustive rather than a `matches!` whitelist: a variant missing
    // from one is dropped before the dispatch loop ever sees it, which
    // reads as the event doing nothing rather than as a mistake.
    let user_events: Vec<&TimelineEvent> = fixture
        .events
        .iter()
        .filter(|e| match e.event {
            UnifiedEvent::Capture { .. }
            | UnifiedEvent::Click { .. }
            | UnifiedEvent::Scroll { .. }
            | UnifiedEvent::Drag { .. }
            | UnifiedEvent::ParamDelivery { .. }
            | UnifiedEvent::SystemDelivery { .. }
            | UnifiedEvent::CredentialDelivery { .. }
            | UnifiedEvent::ClockSet { .. } => true,
            // Served by the fetch interceptor, or injected on their own clock.
            UnifiedEvent::Fetch { .. }
            | UnifiedEvent::SsdpFound { .. }
            | UnifiedEvent::SsdpRemoved { .. }
            | UnifiedEvent::MdnsFound { .. }
            | UnifiedEvent::MdnsRemoved { .. }
            | UnifiedEvent::WsOpen { .. }
            | UnifiedEvent::WsMessage { .. }
            | UnifiedEvent::WsClose { .. }
            | UnifiedEvent::SocketConnected { .. }
            | UnifiedEvent::SocketData { .. }
            | UnifiedEvent::SocketClosed { .. }
            | UnifiedEvent::UdpResponse { .. }
            | UnifiedEvent::AudioPlay { .. }
            | UnifiedEvent::LedSetEndless { .. }
            | UnifiedEvent::LedSetTemporary { .. }
            | UnifiedEvent::LedStop => false,
        })
        .collect();

    let is_tty = std::io::stderr().is_terminal();
    let mut monotonic_ms: u64 = 0;
    // Time in the original recording timeline — advances with monotonic_ms
    // but pauses during capture events (which consume real frames but shouldn't
    // shift the fixture's network event schedule).
    let mut fixture_ms: u64 = 0;
    let mut frame_count: u32 = 0;
    let mut captured_count: u32 = 0;
    let mut event_cursor: usize = 0;
    let mut gate = FrameGate::default();

    // Process all user events by advancing time to each one
    while event_cursor < user_events.len() {
        // User events fire at their recorded timestamp in the fixture timeline.
        // Convert to monotonic time: monotonic = recorded + (monotonic - fixture).
        let target_ms = user_events[event_cursor].at_ms + (monotonic_ms - fixture_ms);

        // Advance to the event's timestamp. I/O is delivered every tick,
        // but the render is gated on the widget's own cadence.
        while monotonic_ms < target_ms {
            runtime.set_time(system_time, monotonic_ms);
            runtime.inject_fixture_events(fixture_ms);
            deliver_all_io(&mut runtime, &mut renderer)?;
            if gate.due(&runtime, monotonic_ms) {
                gate.render(
                    &mut runtime,
                    &mut renderer,
                    ctx,
                    &gl,
                    frame_count,
                    monotonic_ms,
                )?;
            }

            monotonic_ms += u64::from(DELTA_MS);
            fixture_ms += u64::from(DELTA_MS);
            system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
            frame_count += 1;
        }

        // Fire all events at this timestamp (may be multiple).
        // Compare in fixture-timeline space: the event's recorded at_ms
        // against the current fixture_ms.
        while event_cursor < user_events.len() && user_events[event_cursor].at_ms <= fixture_ms {
            let event = &user_events[event_cursor].event;
            match event {
                UnifiedEvent::Capture { duration_ms, fps } => {
                    // Run settle_delay extra frames so the widget can
                    // process network events and animate before capture.
                    // Both monotonic and fixture time advance so events
                    // continue to be delivered during the settle period.
                    for _ in 0..config.settle_delay_for(&ctx.dataset) {
                        runtime.set_time(system_time, monotonic_ms);
                        runtime.inject_fixture_events(fixture_ms);
                        deliver_all_io(&mut runtime, &mut renderer)?;
                        gate.render(
                            &mut runtime,
                            &mut renderer,
                            ctx,
                            &gl,
                            frame_count,
                            monotonic_ms,
                        )
                        .context("while settling before the capture")?;
                        monotonic_ms += u64::from(DELTA_MS);
                        fixture_ms += u64::from(DELTA_MS);
                        system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
                        frame_count += 1;
                    }

                    // Drain active fetch → decode chains without advancing fixture time.
                    // Delayed polls are excluded so scheduled refreshes do not block capture.
                    let io_drain_started = Instant::now();
                    loop {
                        let pending = if ctx.online {
                            runtime.has_pending_io()
                        } else {
                            runtime.has_in_flight_fetches() || runtime.has_pending_image_decodes()
                        };
                        if !pending
                            || io_drain_deadline_reached(ctx.online, io_drain_started.elapsed())?
                        {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                        deliver_all_io(&mut runtime, &mut renderer)?;
                        gate.render(
                            &mut runtime,
                            &mut renderer,
                            ctx,
                            &gl,
                            frame_count,
                            monotonic_ms,
                        )
                        .context("while draining I/O")?;
                    }

                    // How many frames to capture and at what interval
                    let (total_frames, capture_interval_ms) = match (*duration_ms, *fps) {
                        (Some(dur), Some(f)) if f > 0 => {
                            #[expect(
                                clippy::cast_sign_loss,
                                clippy::cast_precision_loss,
                                reason = "duration_ms is small, result is always positive"
                            )]
                            let n = (dur as f64 * f64::from(f) / 1_000.0).ceil() as u32;
                            (n.max(1), 1_000 / u64::from(f))
                        }
                        (Some(dur), _) => {
                            // Duration but no fps: capture at render rate (DELTA_MS)
                            let n = (dur / u64::from(DELTA_MS)).max(1) as u32;
                            (n, u64::from(DELTA_MS))
                        }
                        _ => (1, 0), // single frame
                    };

                    tracing::debug!(
                        event = event_cursor,
                        frames = total_frames,
                        monotonic_ms,
                        fixture_ms,
                        recorded_ms = user_events[event_cursor].at_ms,
                        "capture"
                    );

                    let mut next_capture_at = monotonic_ms;
                    let mut frames_left = total_frames;

                    // During capture, only monotonic_ms advances — fixture_ms
                    // is frozen so network events stay aligned with the original
                    // recording timeline.
                    while frames_left > 0 {
                        // Advance to next capture point
                        while monotonic_ms < next_capture_at {
                            runtime.set_time(system_time, monotonic_ms);
                            runtime.inject_fixture_events(fixture_ms);
                            deliver_all_io(&mut runtime, &mut renderer)?;
                            gate.render(
                                &mut runtime,
                                &mut renderer,
                                ctx,
                                &gl,
                                frame_count,
                                monotonic_ms,
                            )?;
                            monotonic_ms += u64::from(DELTA_MS);
                            system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
                            frame_count += 1;
                        }

                        // Render and capture
                        runtime.set_time(system_time, monotonic_ms);
                        runtime.inject_fixture_events(fixture_ms);
                        deliver_all_io(&mut runtime, &mut renderer)?;
                        gate.render(
                            &mut runtime,
                            &mut renderer,
                            ctx,
                            &gl,
                            frame_count,
                            monotonic_ms,
                        )?;

                        let path = ctx
                            .output_dir
                            .join(format!("frame_{captured_count:04}.png"));
                        let pixels = read_fbo_pixels(&gl, fbo, ctx.width, ctx.height);
                        // Mask the round disc only for previews.
                        // Regression captures stay square:
                        // the corners outside the disc are free regression surface.
                        let round = ctx.online && ctx.target.viewport.shape == DisplayShape::Round;
                        save_screenshot(&pixels, ctx.width, ctx.height, round, &path)?;
                        if !is_tty {
                            eprintln!("Captured frame {captured_count} → {}", path.display());
                        }
                        captured_count += 1;
                        frames_left -= 1;

                        monotonic_ms += u64::from(DELTA_MS);
                        system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
                        frame_count += 1;

                        if frames_left > 0 {
                            next_capture_at =
                                monotonic_ms + capture_interval_ms - u64::from(DELTA_MS);
                        }
                    }
                }
                UnifiedEvent::Click { element } => {
                    eprintln!("  [{event_cursor}] click(#{element})");
                    tracing::debug!(
                        event = event_cursor,
                        element,
                        monotonic_ms,
                        fixture_ms,
                        recorded_ms = user_events[event_cursor].at_ms,
                        frame_count,
                        "click"
                    );
                    let b = runtime.element_bounds(element).with_context(|| {
                        let available = runtime.element_ids().join(", ");
                        format!(
                            "element '#{element}' not found in hit regions at frame {frame_count} \
                             (monotonic={monotonic_ms}ms, fixture={fixture_ms}ms)\n\
                             available elements: [{available}]"
                        )
                    })?;
                    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                    runtime.push_touch_event(TouchEvent::Down { x: cx, y: cy });
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut gate,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                    runtime.push_touch_event(TouchEvent::Up);
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut gate,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                }
                UnifiedEvent::Scroll { element, delta } => {
                    eprintln!("  [{event_cursor}] scroll(#{element}, {delta})");
                    let b = runtime.element_bounds(element).with_context(|| {
                        let available = runtime.element_ids().join(", ");
                        format!(
                            "element '#{element}' not found in hit regions at frame {frame_count}\n\
                             available elements: [{available}]"
                        )
                    })?;
                    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                    let steps = 5;
                    let step_delta = *delta as f32 / 5.0;
                    runtime.push_touch_event(TouchEvent::Down { x: cx, y: cy });
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut gate,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                    let mut current_y = cy;
                    for _ in 0..steps {
                        current_y += step_delta;
                        runtime.push_touch_event(TouchEvent::Move {
                            x: cx,
                            y: current_y,
                        });
                        tick_one_frame(
                            &mut runtime,
                            &mut renderer,
                            ctx,
                            &gl,
                            &mut gate,
                            &mut monotonic_ms,
                            &mut fixture_ms,
                            &mut system_time,
                            &mut frame_count,
                        )?;
                    }
                    runtime.push_touch_event(TouchEvent::Up);
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut gate,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                }
                UnifiedEvent::Drag { element, from, to } => {
                    eprintln!("  [{event_cursor}] drag(#{element}, {from}, {to})");
                    let b = runtime.element_bounds(element).with_context(|| {
                        let available = runtime.element_ids().join(", ");
                        format!(
                            "element '#{element}' not found in hit regions at frame {frame_count}\n\
                             available elements: [{available}]"
                        )
                    })?;
                    let cy = b.y + b.h / 2.0;
                    let start_x = b.x + from * b.w;
                    let end_x = b.x + to * b.w;
                    runtime.push_touch_event(TouchEvent::Down { x: start_x, y: cy });
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut gate,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                    for i in 1..=DRAG_FRAMES {
                        let t = i as f32 / DRAG_FRAMES as f32;
                        let x = start_x + (end_x - start_x) * t;
                        runtime.push_touch_event(TouchEvent::Move { x, y: cy });
                        tick_one_frame(
                            &mut runtime,
                            &mut renderer,
                            ctx,
                            &gl,
                            &mut gate,
                            &mut monotonic_ms,
                            &mut fixture_ms,
                            &mut system_time,
                            &mut frame_count,
                        )?;
                    }
                    runtime.push_touch_event(TouchEvent::Up);
                    tick_one_frame(
                        &mut runtime,
                        &mut renderer,
                        ctx,
                        &gl,
                        &mut gate,
                        &mut monotonic_ms,
                        &mut fixture_ms,
                        &mut system_time,
                        &mut frame_count,
                    )?;
                }
                // Operator-driven params update — call `deliver_params_update`
                // on the runtime and let the widget's `on_params_update` hook fire.
                // The version counter is bumped by the runtime;
                // we don't need to advance any timeline-side state.
                UnifiedEvent::ParamDelivery { params } => {
                    let table = bmc_wasm_runtime::parse_params_json(params)
                        .expect("BUG: capture ParamDelivery params must be valid");
                    runtime.deliver_params_update(table);
                }
                // System-snapshot delivery — same shape as ParamDelivery
                // but for deck-wide settings. Fires the widget's
                // `on_system_update` hook (channel-isolated sibling
                // of `on_params_update`).
                UnifiedEvent::SystemDelivery { system } => {
                    runtime.deliver_system_update(system.clone());
                }
                // Operator bound or unbound an account.
                // Paired with empty secrets: a fixture never carries them,
                // and no rendering can read them anyway.
                UnifiedEvent::CredentialDelivery { credentials } => {
                    runtime.deliver_credentials_update(
                        bmc_wasm_runtime::parse_credentials_json(credentials),
                        bmc_widget_protocol::CredentialSecrets::default(),
                    );
                }
                // The wall clock alone: the operator moved the calendar,
                // so no time elapsed and `monotonic_ms` must not follow.
                // Walking the span instead would render every frame of it.
                UnifiedEvent::ClockSet { time } => {
                    system_time =
                        chrono::DateTime::parse_from_rfc3339(time).with_context(|| {
                            format!("clock_set at {event_cursor} carries an unreadable time")
                        })?;
                }
                // Network events are handled by inject_fixture_events/fetch_interceptor
                UnifiedEvent::Fetch { .. }
                | UnifiedEvent::SsdpFound { .. }
                | UnifiedEvent::SsdpRemoved { .. }
                | UnifiedEvent::MdnsFound { .. }
                | UnifiedEvent::MdnsRemoved { .. }
                | UnifiedEvent::WsOpen { .. }
                | UnifiedEvent::WsMessage { .. }
                | UnifiedEvent::WsClose { .. }
                | UnifiedEvent::SocketConnected { .. }
                | UnifiedEvent::SocketData { .. }
                | UnifiedEvent::SocketClosed { .. }
                | UnifiedEvent::UdpResponse { .. }
                | UnifiedEvent::AudioPlay { .. }
                | UnifiedEvent::LedSetEndless { .. }
                | UnifiedEvent::LedSetTemporary { .. }
                | UnifiedEvent::LedStop => {}
            }

            runtime.take_lifecycle_trap().with_context(|| {
                if ctx.stack_profiling.is_enabled() {
                    "stack profile invalid: lifecycle callback trapped"
                } else {
                    "widget lifecycle callback trapped"
                }
            })?;

            if is_tty {
                eprint!(
                    "\r  frame {frame_count}  [{event_cursor}/{}]  ({captured_count} captured)   ",
                    user_events.len()
                );
            }
            event_cursor += 1;
        }
    }

    if is_tty {
        eprintln!();
    }

    // Fail the run if it breached hermeticity (unmatched live I/O).
    let breaches = runtime.hermetic_breaches();
    if !breaches.is_empty() {
        anyhow::bail!(
            "hermetic capture breach in {widget_name} ({}x{}): widget attempted live I/O with no fixture:\n  {}",
            ctx.width,
            ctx.height,
            breaches.join("\n  ")
        );
    }

    if ctx.stack_profiling.is_enabled() {
        let high_water = runtime
            .exported_global_i32(bmc_wasm_runtime::stack_profile::STACK_HIGH_WATER_EXPORT)
            .context("instrumented widget did not expose its stack measurement")?;
        println!("BMC_STACK_HIGH_WATER={high_water}");
    }
    if ctx.layout_cache_profiling.is_enabled() {
        let layout = renderer.text_layout_counters();
        println!("BMC_LAYOUT_CACHE_HITS={}", layout.layout_cache_hits);
        println!("BMC_LAYOUT_CACHE_SHAPES={}", layout.layout_cache_shapes);
        println!(
            "BMC_LAYOUT_CACHE_SINGLE_LINE_HITS={}",
            layout.layout_cache_single_line_hits
        );
        println!(
            "BMC_LAYOUT_CACHE_SINGLE_LINE_SHAPES={}",
            layout.layout_cache_single_line_shapes
        );
        println!(
            "BMC_LAYOUT_CACHE_PARAGRAPH_HITS={}",
            layout.layout_cache_paragraph_hits
        );
        println!(
            "BMC_LAYOUT_CACHE_PARAGRAPH_SHAPES={}",
            layout.layout_cache_paragraph_shapes
        );
        println!(
            "BMC_LAYOUT_CACHE_SINGLE_LINE_ENTRIES={}",
            layout.layout_cache_single_line_entries
        );
        println!(
            "BMC_LAYOUT_CACHE_SINGLE_LINE_PEAK_ENTRIES={}",
            layout.layout_cache_single_line_peak_entries
        );
        println!(
            "BMC_LAYOUT_CACHE_PARAGRAPH_ENTRIES={}",
            layout.layout_cache_paragraph_entries
        );
        println!(
            "BMC_LAYOUT_CACHE_PARAGRAPH_PEAK_ENTRIES={}",
            layout.layout_cache_paragraph_peak_entries
        );
        println!(
            "BMC_LAYOUT_CACHE_RESIDENT_GLYPHS={}",
            layout.layout_cache_resident_glyphs
        );
        println!(
            "BMC_LAYOUT_CACHE_PEAK_RESIDENT_GLYPHS={}",
            layout.layout_cache_peak_resident_glyphs
        );
        println!(
            "BMC_LAYOUT_CACHE_SINGLE_LINE_RESIDENT_GLYPHS={}",
            layout.layout_cache_single_line_resident_glyphs
        );
        println!(
            "BMC_LAYOUT_CACHE_SINGLE_LINE_PEAK_RESIDENT_GLYPHS={}",
            layout.layout_cache_single_line_peak_resident_glyphs
        );
        println!(
            "BMC_LAYOUT_CACHE_PARAGRAPH_RESIDENT_GLYPHS={}",
            layout.layout_cache_paragraph_resident_glyphs
        );
        println!(
            "BMC_LAYOUT_CACHE_PARAGRAPH_PEAK_RESIDENT_GLYPHS={}",
            layout.layout_cache_paragraph_peak_resident_glyphs
        );
        println!(
            "BMC_LAYOUT_CACHE_PEAK_FRAME_GLYPH_INSTANCES={}",
            layout.layout_cache_peak_frame_glyph_instances
        );
        println!(
            "BMC_LAYOUT_CACHE_PEAK_FRAME_DISTINCT_GLYPHS={}",
            layout.layout_cache_peak_frame_distinct_glyphs
        );
        println!(
            "BMC_LAYOUT_CACHE_CAPACITY_EVICTIONS={}",
            layout.layout_cache_capacity_evictions
        );
        println!(
            "BMC_LAYOUT_CACHE_PEAK_ENTRIES={}",
            layout.layout_cache_peak_entries
        );
        println!(
            "BMC_LAYOUT_CACHE_PEAK_FRAME_KEYS={}",
            layout.layout_cache_peak_frame_keys
        );
        println!(
            "BMC_LAYOUT_CACHE_REPEAT_SHAPES_SAME_FRAME={}",
            layout.layout_cache_repeat_shapes_same_frame
        );
        println!(
            "BMC_LAYOUT_CACHE_DRAW_MISSES_AFTER_MEASURE={}",
            layout.layout_cache_draw_misses_after_measure
        );
    }

    eprintln!(
        "Done: {captured_count} frame(s) captured to {}",
        ctx.output_dir.display()
    );
    Ok(())
}

fn io_drain_deadline_reached(online: bool, elapsed: Duration) -> Result<bool> {
    let timeout = if online {
        ONLINE_IO_DRAIN_TIMEOUT
    } else {
        OFFLINE_IO_DRAIN_TIMEOUT
    };
    if elapsed < timeout {
        return Ok(false);
    }
    if online {
        Ok(true)
    } else {
        bail!(
            "offline capture I/O did not settle within {} s",
            timeout.as_secs()
        )
    }
}

// ── Frame helpers ───────────────────────────────────────────────────

/// Tick a single frame: set time, inject fixtures, deliver I/O, render, advance.
///
/// Both `monotonic_ms` and `fixture_ms` are advanced by `DELTA_MS`.
#[expect(
    clippy::too_many_arguments,
    reason = "single-flow per-frame helper threading the replay loop's interlocked clocks; \
              splitting hurts readability"
)]
fn tick_one_frame(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
    ctx: &CaptureCtx,
    gl: &glow::Context,
    gate: &mut FrameGate,
    monotonic_ms: &mut u64,
    fixture_ms: &mut u64,
    system_time: &mut chrono::DateTime<chrono::FixedOffset>,
    frame_count: &mut u32,
) -> Result<()> {
    runtime.set_time(*system_time, *monotonic_ms);
    runtime.inject_fixture_events(*fixture_ms);
    deliver_all_io(runtime, renderer)?;
    gate.render(runtime, renderer, ctx, gl, *frame_count, *monotonic_ms)?;
    *monotonic_ms += u64::from(DELTA_MS);
    *fixture_ms += u64::from(DELTA_MS);
    *system_time += chrono::Duration::milliseconds(i64::from(DELTA_MS));
    *frame_count += 1;
    Ok(())
}

/// Faithful-cadence render gate, and the only door a frame leaves through:
/// [`FrameGate::render`] arms the next deadline at every render,
/// so a settle or capture burst cannot leave the schedule
/// phased off the last gated frame.
///
/// The device host coalesces renders onto the cadence the widget requests
/// with `request_frame_after`, so a fold decoupled from the render loop
/// runs only at those coalesced wakes.
///
/// Rendering every virtual frame would fold on a different schedule and
/// sample a different device set: a group transiently present on hardware
/// vanishes on replay, so an interaction recorded against it misses its target.
///
/// Mirrors the testbed's on-demand drive (`render_tiles`):
/// device, testbed and replay all redraw on the same signal.
#[derive(Default)]
struct FrameGate {
    ever_rendered: bool,
    next_render_at_ms: Option<u64>,
}

impl FrameGate {
    /// Whether a render is due at `monotonic_ms`. Call it once the tick has
    /// delivered its I/O, so a just-made frame request is seen.
    /// Arms the next deadline when a want was raised but nothing is due yet.
    fn due(&mut self, runtime: &WasmWidgetRuntime, monotonic_ms: u64) -> bool {
        let due = !self.ever_rendered
            || self.next_render_at_ms.is_some_and(|at| monotonic_ms >= at)
            || runtime.next_frame_delay() == Some(0);
        if !due && self.next_render_at_ms.is_none() && runtime.wants_next_frame() {
            self.next_render_at_ms =
                Some(monotonic_ms + u64::from(runtime.next_frame_delay().unwrap_or(0)));
        }
        due
    }

    /// Render one frame, flush it, and arm the next deadline
    /// from what the widget asked for during the draw
    /// (`None` = idle until the next delivery).
    fn render(
        &mut self,
        runtime: &mut WasmWidgetRuntime,
        renderer: &mut FemtoVgRenderer,
        ctx: &CaptureCtx,
        gl: &glow::Context,
        frame_count: u32,
        monotonic_ms: u64,
    ) -> Result<()> {
        if !render_frame(runtime, renderer, ctx, frame_count) {
            bail!("widget died at frame {frame_count}");
        }
        unsafe { gl.flush() };
        self.ever_rendered = true;
        self.next_render_at_ms = runtime
            .wants_next_frame()
            .then(|| monotonic_ms + u64::from(runtime.next_frame_delay().unwrap_or(0)));
        Ok(())
    }
}

/// Render one frame. Returns false if the widget died or errored (caller should break).
fn render_frame(
    runtime: &mut WasmWidgetRuntime,
    renderer: &mut FemtoVgRenderer,
    ctx: &CaptureCtx,
    frame_count: u32,
) -> bool {
    renderer.begin_frame(ctx.width, ctx.height, 1.0);
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    let ptr = std::ptr::NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    match runtime.with_renderer(ptr, |rt| rt.render(DELTA_MS)) {
        Ok(RenderStatus::Dead) => {
            eprintln!("Widget died at frame {frame_count}");
            false
        }
        Ok(RenderStatus::FuelExhausted) if ctx.stack_profiling.is_enabled() => {
            eprintln!("Stack profile invalid: widget exhausted its production fuel budget");
            false
        }
        Ok(_) => {
            renderer.flush();
            true
        }
        Err(e) => {
            eprintln!("Render error at frame {frame_count}: {e}");
            false
        }
    }
}

/// Deliver all async I/O to the runtime.
fn deliver_all_io(runtime: &mut WasmWidgetRuntime, renderer: &mut FemtoVgRenderer) -> Result<()> {
    let raw: *mut dyn Renderer = core::ptr::addr_of_mut!(*renderer);
    let ptr = std::ptr::NonNull::new(raw).expect("BUG: addr_of_mut! cannot produce null");
    runtime.poll_deliveries_with_renderer(ptr)?;
    runtime.take_lifecycle_trap()
}

// ── KV directory setup ──────────────────────────────────────────────

/// Prepare a fresh KV directory for unified fixture replay.
///
/// Seeds from the fixture header's KV map (not from secrets.ini — the fixture
/// is self-contained), with the dataset's `kv` overlay on top.
fn prepare_unified_kv_dir(
    ctx: &CaptureCtx,
    config: &CaptureConfig,
    widget_name: &str,
    fixture: &UnifiedFixture,
) -> PathBuf {
    let kv_dir = std::env::temp_dir()
        .join("bmc-wasm-capture")
        .join(widget_name)
        .join(ctx.target.platform.id)
        .join(ctx.target.viewport.id)
        .join(&ctx.dataset);
    let _ = std::fs::remove_dir_all(&kv_dir);
    let _ = std::fs::create_dir_all(&kv_dir);
    // Fixture header KV (self-contained baseline)
    for (key, value) in &fixture.header.kv {
        let _ = std::fs::write(kv_dir.join(key), value.as_bytes());
    }
    if let Some(entry) = config.fixtures.get(&ctx.dataset) {
        for (key, value) in &entry.kv {
            let _ = std::fs::write(kv_dir.join(key), value.as_bytes());
        }
    }
    kv_dir
}

/// Per-tag bucket cap for the replay blob cache, matching the device's flash cap.
const CAPTURE_CACHE_MAX_BYTES: u64 = 16 * 1_024 * 1_024;

/// A fresh blob cache for replay: without one an image stays `Volatile`,
/// so a dormant view drops it and the frame draws fallback, not artwork.
fn prepare_unified_blob_dir(ctx: &CaptureCtx, widget_name: &str) -> PathBuf {
    let blob_dir = std::env::temp_dir()
        .join("bmc-wasm-capture")
        .join(widget_name)
        .join(ctx.target.platform.id)
        .join(ctx.target.viewport.id)
        .join(format!("{}-blobs", ctx.dataset));
    let _ = std::fs::remove_dir_all(&blob_dir);
    let _ = std::fs::create_dir_all(&blob_dir);
    blob_dir
}

// ── Event splitting ─────────────────────────────────────────────────

/// A pre-recorded fetch response for the interceptor.
struct FetchEntry {
    status: u32,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Split a unified fixture timeline into fetch interceptors and network events.
///
/// - `Fetch` events → `HashMap<String, Vec<FetchEntry>>` keyed by `"METHOD URL"`,
///   preserving fixture order so the interceptor can replay them in sequence
/// - Network events (SSDP, mDNS, WS, Socket, UDP) → `Vec<FixtureEvent>`
/// - User actions (Capture, Click, Scroll, Drag) are skipped (handled in the
///   main replay loop)
#[expect(clippy::too_many_lines)]
fn split_unified_events(
    fixture: &UnifiedFixture,
) -> (HashMap<String, Vec<FetchEntry>>, Vec<FixtureEvent>) {
    let mut fetches: HashMap<String, Vec<FetchEntry>> = HashMap::new();
    let mut network_events = Vec::new();

    for te in &fixture.events {
        match &te.event {
            UnifiedEvent::Fetch {
                method,
                url,
                status,
                headers,
                body,
            } => {
                let key = format!("{method} {url}");
                fetches.entry(key).or_default().push(FetchEntry {
                    status: *status,
                    headers: headers.clone(),
                    body: body.to_bytes(),
                });
            }

            // Convert network events to the runtime's FixtureEvent format
            UnifiedEvent::SsdpFound { search_id, data } => {
                let Some(search_id) = SsdpSearchId::from_wire(*search_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SsdpFound {
                        search_id,
                        data: data.clone(),
                    },
                });
            }
            UnifiedEvent::SsdpRemoved { search_id, data } => {
                let Some(search_id) = SsdpSearchId::from_wire(*search_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SsdpRemoved {
                        search_id,
                        data: data.clone(),
                    },
                });
            }
            UnifiedEvent::MdnsFound { browse_id, data } => {
                let Some(browse_id) = MdnsBrowseId::from_wire(*browse_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::MdnsFound {
                        browse_id,
                        data: data.clone(),
                    },
                });
            }
            UnifiedEvent::MdnsRemoved { browse_id, data } => {
                let Some(browse_id) = MdnsBrowseId::from_wire(*browse_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::MdnsRemoved {
                        browse_id,
                        data: data.clone(),
                    },
                });
            }
            UnifiedEvent::WsOpen { ws_id } => {
                let Some(ws_id) = WebsocketId::from_wire(*ws_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::WsOpen { ws_id },
                });
            }
            UnifiedEvent::WsMessage { ws_id, data } => {
                let Some(ws_id) = WebsocketId::from_wire(*ws_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::WsMessage {
                        ws_id,
                        data: data.to_bytes(),
                    },
                });
            }
            UnifiedEvent::WsClose { ws_id, code } => {
                let Some(ws_id) = WebsocketId::from_wire(*ws_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::WsClose { ws_id, code: *code },
                });
            }
            UnifiedEvent::SocketConnected { socket_id } => {
                let Some(socket_id) = SocketId::from_wire(*socket_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SocketConnected { socket_id },
                });
            }
            UnifiedEvent::SocketData { socket_id, data } => {
                let Some(socket_id) = SocketId::from_wire(*socket_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SocketData {
                        socket_id,
                        data: data.to_bytes(),
                    },
                });
            }
            UnifiedEvent::SocketClosed { socket_id, code } => {
                let Some(socket_id) = SocketId::from_wire(*socket_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::SocketClosed {
                        socket_id,
                        code: *code,
                    },
                });
            }
            UnifiedEvent::UdpResponse {
                broadcast_id,
                data,
                source,
            } => {
                let Some(broadcast_id) = UdpBroadcastId::from_wire(*broadcast_id) else {
                    continue;
                };
                network_events.push(FixtureEvent {
                    at_ms: te.at_ms,
                    kind: FixtureEventKind::UdpResponse {
                        broadcast_id,
                        data: data.clone(),
                        source: source.clone(),
                    },
                });
            }

            // User actions and informational events are handled in the main replay loop
            UnifiedEvent::Capture { .. }
            | UnifiedEvent::Click { .. }
            | UnifiedEvent::Scroll { .. }
            | UnifiedEvent::Drag { .. }
            | UnifiedEvent::ParamDelivery { .. }
            | UnifiedEvent::SystemDelivery { .. }
            | UnifiedEvent::CredentialDelivery { .. }
            | UnifiedEvent::ClockSet { .. }
            | UnifiedEvent::AudioPlay { .. }
            | UnifiedEvent::LedSetEndless { .. }
            | UnifiedEvent::LedSetTemporary { .. }
            | UnifiedEvent::LedStop => {}
        }
    }

    (fetches, network_events)
}

// ── GL helpers ──────────────────────────────────────────────────────

#[expect(clippy::cast_possible_wrap)]
fn create_fbo(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<(glow::Framebuffer, glow::Texture)> {
    unsafe {
        let texture = gl
            .create_texture()
            .map_err(|e| anyhow::anyhow!("create texture: {e}"))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );

        let fbo = gl
            .create_framebuffer()
            .map_err(|e| anyhow::anyhow!("create framebuffer: {e}"))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );

        // Stencil buffer required by FemtoVG
        let rbo = gl
            .create_renderbuffer()
            .map_err(|e| anyhow::anyhow!("create renderbuffer: {e}"))?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rbo));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            glow::DEPTH24_STENCIL8,
            width as i32,
            height as i32,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_STENCIL_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(rbo),
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);

        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        assert_eq!(
            status,
            glow::FRAMEBUFFER_COMPLETE,
            "FBO incomplete: {status:#x}"
        );

        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D, None);

        Ok((fbo, texture))
    }
}

#[expect(clippy::cast_possible_wrap)]
fn read_fbo_pixels(gl: &glow::Context, fbo: glow::Framebuffer, w: u32, h: u32) -> Vec<u8> {
    unsafe {
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
        let mut pixels = vec![0_u8; (w * h * 4) as usize];
        gl.read_pixels(
            0,
            0,
            w as i32,
            h as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        pixels
    }
}

fn save_screenshot(pixels: &[u8], w: u32, h: u32, round: bool, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // OpenGL gives us bottom-up rows — flip vertically
    let row_bytes = (w * 4) as usize;
    let mut flipped = vec![0_u8; pixels.len()];
    for y in 0..h as usize {
        let src_row = (h as usize - 1 - y) * row_bytes;
        let dst_row = y * row_bytes;
        flipped[dst_row..dst_row + row_bytes]
            .copy_from_slice(&pixels[src_row..src_row + row_bytes]);
    }
    if round {
        mask_round(&mut flipped, w, h);
    }
    image::save_buffer(path, &flipped, w, h, image::ColorType::Rgba8)?;
    Ok(())
}

/// Fade pixels outside the inscribed circle to transparent, so a round
/// display's screenshot shows its visible disc rather than the square
/// framebuffer. A 1px feather at the rim keeps the edge from aliasing.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "viewport coords are <= 1280 (exact in f32); coverage and alpha are \
              non-negative, so the u8 cast cannot lose a sign"
)]
fn mask_round(rgba: &mut [u8], w: u32, h: u32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let radius = w.min(h) as f32 / 2.0;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let coverage = (radius + 0.5 - dist).clamp(0.0, 1.0);
            if coverage < 1.0 {
                let alpha = &mut rgba[(y * w as usize + x) * 4 + 3];
                *alpha = (f32::from(*alpha) * coverage) as u8;
            }
        }
    }
}

// ── Headless GL setup ────────────────────────────────────────────────
//
// Platform-specific EGL context creation.  Everything after this point
// goes through `glow::Context` and is fully cross-platform.
//
// Linux  — glutin + Mesa EGL (llvmpipe for deterministic CI rendering)
// macOS  — khronos-egl + ANGLE (Metal backend, loaded at runtime)

/// The rasteriser this process captured through, as GL reports it.
///
/// Pixels compare only against baselines drawn by the same rasteriser.
/// The platform picks it — llvmpipe on Linux, ANGLE's Metal backend on macOS.
/// Naming it separates a cross-renderer difference from a real regression.
static RENDERER: OnceLock<String> = OnceLock::new();

/// Marks the rasteriser on a capture's stderr.
pub(super) const RENDERER_PREFIX: &str = "renderer: ";

/// The recorded rasteriser, once a capture has built a GL context.
pub(super) fn renderer() -> Option<&'static str> {
    RENDERER.get().map(String::as_str)
}

/// Adopt a rasteriser name a capture reported on its stderr.
pub(super) fn note_renderer(name: &str) {
    let _ = RENDERER.set(name.to_owned());
}

/// Record the rasteriser from a context that is current on this thread.
fn record_renderer(gl: &glow::Context) {
    if RENDERER.get().is_some() {
        return;
    }
    // SAFETY: callers hold the GL context current for the duration.
    let name = unsafe { gl.get_parameter_string(glow::RENDERER) };
    // Printed too: a capture is its own process, so stderr carries it out.
    eprintln!("{RENDERER_PREFIX}{name}");
    let _ = RENDERER.set(name);
}

// ── Linux: glutin + Mesa EGL ────────────────────────────────────────

#[cfg(target_os = "linux")]
fn create_headless_egl_display() -> Result<Display> {
    let devices: Vec<_> = glutin::api::egl::device::Device::query_devices()
        .context("EGL device enumeration not supported (missing EGL_EXT_device_query?)")?
        .collect();

    let device = devices
        .iter()
        .find(|d| d.extensions().contains("EGL_MESA_device_software"))
        .or_else(|| devices.first())
        .context("no EGL devices found (is Mesa/libGL in LD_LIBRARY_PATH?)")?;

    let display = unsafe { glutin::api::egl::display::Display::with_device(device, None) }
        .context("failed to create EGL display from device")?;

    Ok(Display::Egl(display))
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::type_complexity,
    reason = "headless EGL setup returns several owned handles whose lifetimes must \
              outlive the caller; pulling them out via a struct would force a public \
              `pub(super)` wrapper for a single call site"
)]
fn setup_gl_and_runtime(
    ctx: &CaptureCtx,
    initial_system_time: DateTime<FixedOffset>,
    rt_config: RuntimeConfig,
) -> Result<(
    glow::Context,
    glow::Framebuffer,
    Box<dyn std::any::Any>,
    FemtoVgRenderer,
    WasmWidgetRuntime,
)> {
    let egl_display =
        create_headless_egl_display().context("failed to create headless EGL display")?;

    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_stencil_size(8)
        .with_surface_type(glutin::config::ConfigSurfaceTypes::PBUFFER);
    let gl_config = unsafe { egl_display.find_configs(template.build()) }
        .map_err(|e| anyhow::anyhow!("failed to find GL configs: {e}"))?
        .reduce(|a, b| {
            if a.num_samples() > b.num_samples() {
                a
            } else {
                b
            }
        })
        .context("no suitable GL config found")?;

    let gl_display = gl_config.display();
    let context_attrs = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(glutin::context::Version::new(2, 0))))
        .build(None);
    let gl_context = unsafe {
        gl_display
            .create_context(&gl_config, &context_attrs)
            .context("failed to create GL context")?
    };

    // Pbuffer surface — fully offscreen, no window needed.
    let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
        NonZeroU32::new(ctx.width).expect("BUG: zero width"),
        NonZeroU32::new(ctx.height).expect("BUG: zero height"),
    );
    let surface = unsafe {
        gl_display
            .create_pbuffer_surface(&gl_config, &surface_attrs)
            .context("failed to create pbuffer surface")?
    };
    let gl_context = gl_context
        .make_current(&surface)
        .context("failed to make GL context current")?;

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            gl_display.get_proc_address(&CString::new(s).unwrap_or_default())
        })
    };
    record_renderer(&gl);
    let (fbo, texture) = create_fbo(&gl, ctx.width, ctx.height)?;
    let fbo_id = fbo.0.get();

    let wasm_bytes =
        std::fs::read(&ctx.runtime_wasm_path).context("failed to read prepared WASM file")?;
    let wasm_bytes = match ctx.stack_profiling.expected_origin() {
        Some(expected_origin) => {
            bmc_wasm_runtime::stack_profile::instrument(&wasm_bytes, expected_origin)?
        }
        None => wasm_bytes,
    };
    // SAFETY: GL context is current on this thread for the lifetime
    // of the returned `keep_alive` bundle, which holds `gl_context`.
    let renderer = unsafe {
        FemtoVgRenderer::new(
            |s| gl_display.get_proc_address(&CString::new(s).unwrap_or_default()),
            ctx.width,
            ctx.height,
            fbo_id,
            rt_config.mesh_msaa_samples,
        )
    }
    .context("failed to create renderer")?;
    let runtime = WasmWidgetRuntime::new(
        &wasm_bytes,
        ctx.width,
        ctx.height,
        ctx.target.viewport.runtime_viewport_shape(),
        ctx.target.platform.runtime_display_info(),
        initial_system_time,
        rt_config,
    )
    .context("failed to create WASM runtime")?;

    let keep_alive: Box<dyn std::any::Any> = Box::new((texture, surface, gl_context));
    Ok((gl, fbo, keep_alive, renderer, runtime))
}

// ── macOS: khronos-egl + ANGLE ──────────────────────────────────────

#[cfg(target_os = "macos")]
fn load_angle_egl() -> Result<khronos_egl::DynamicInstance<khronos_egl::EGL1_4>> {
    // Try default search path first (works when DYLD_LIBRARY_PATH is set,
    // e.g. inside `nix develop`), then fall back to common Homebrew prefixes.
    let candidates = [
        "libEGL.dylib",
        "/opt/homebrew/lib/libEGL.dylib", // Homebrew on Apple Silicon
        "/usr/local/lib/libEGL.dylib",    // Homebrew on Intel
    ];

    let mut last_err = None;
    for path in candidates {
        // SAFETY: loading a well-known library path.
        match unsafe {
            khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required_from_filename(path)
        } {
            Ok(instance) => return Ok(instance),
            Err(e) => last_err = Some(e),
        }
    }

    Err(anyhow::anyhow!(
        "failed to load libEGL.dylib: {}\n\n\
         ANGLE is required on macOS for headless GL rendering.\n\
         Install via one of:\n  \
         - nix develop\n  \
         - brew tap startergo/angle && brew install angle",
        last_err.expect("BUG: candidates list is empty"),
    ))
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::type_complexity,
    reason = "headless EGL setup returns several owned handles whose lifetimes must \
              outlive the caller; pulling them out via a struct would force a public \
              `pub(super)` wrapper for a single call site"
)]
fn setup_gl_and_runtime(
    ctx: &CaptureCtx,
    initial_system_time: DateTime<FixedOffset>,
    rt_config: RuntimeConfig,
) -> Result<(
    glow::Context,
    glow::Framebuffer,
    Box<dyn std::any::Any>,
    FemtoVgRenderer,
    WasmWidgetRuntime,
)> {
    use khronos_egl as egl;

    let instance = load_angle_egl()?;

    // SAFETY: DEFAULT_DISPLAY is a well-known constant — ANGLE handles it.
    let display =
        unsafe { instance.get_display(egl::DEFAULT_DISPLAY) }.context("eglGetDisplay failed")?;
    instance
        .initialize(display)
        .context("eglInitialize failed")?;

    let config = instance
        .choose_first_config(
            display,
            &[
                egl::RED_SIZE,
                8,
                egl::GREEN_SIZE,
                8,
                egl::BLUE_SIZE,
                8,
                egl::ALPHA_SIZE,
                8,
                egl::STENCIL_SIZE,
                8,
                egl::SURFACE_TYPE,
                egl::PBUFFER_BIT,
                egl::RENDERABLE_TYPE,
                egl::OPENGL_ES2_BIT,
                egl::NONE,
            ],
        )?
        .context("no suitable EGL config")?;

    let context = instance
        .create_context(
            display,
            config,
            None,
            &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
        )
        .context("eglCreateContext failed")?;

    let surface = instance
        .create_pbuffer_surface(
            display,
            config,
            &[
                egl::WIDTH,
                ctx.width.cast_signed(),
                egl::HEIGHT,
                ctx.height.cast_signed(),
                egl::NONE,
            ],
        )
        .context("eglCreatePbufferSurface failed")?;

    instance
        .make_current(display, Some(surface), Some(surface), Some(context))
        .context("eglMakeCurrent failed")?;

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            instance
                .get_proc_address(s)
                .map_or(std::ptr::null(), |f| f as *const _)
        })
    };
    record_renderer(&gl);

    let (fbo, texture) = create_fbo(&gl, ctx.width, ctx.height)?;
    let fbo_id = fbo.0.get();

    let wasm_bytes =
        std::fs::read(&ctx.runtime_wasm_path).context("failed to read prepared WASM file")?;
    // SAFETY: ANGLE EGL context is current on this thread
    // for the lifetime of the returned `keep_alive` bundle.
    let renderer = unsafe {
        FemtoVgRenderer::new(
            |s| {
                instance
                    .get_proc_address(s)
                    .map_or(std::ptr::null(), |f| f as *const _)
            },
            ctx.width,
            ctx.height,
            fbo_id,
            rt_config.mesh_msaa_samples,
        )
    }
    .context("failed to create renderer")?;
    let runtime = WasmWidgetRuntime::new(
        &wasm_bytes,
        ctx.width,
        ctx.height,
        ctx.target.viewport.runtime_viewport_shape(),
        ctx.target.platform.runtime_display_info(),
        initial_system_time,
        rt_config,
    )
    .context("failed to create WASM runtime")?;

    // Keep EGL state alive — dropping tears down the GL context.
    let keep_alive: Box<dyn std::any::Any> =
        Box::new((instance, display, context, surface, texture));

    Ok((gl, fbo, keep_alive, renderer, runtime))
}

// ── Init subcommand ─────────────────────────────────────────────────

/// Write a default `capture/config.toml` template with commented-out options.
pub fn write_default_capture_config(dir: &Path) -> Result<()> {
    let capture_dir = dir.join("capture");
    let _ = std::fs::create_dir_all(&capture_dir);
    let path = capture_dir.join("config.toml");
    if path.exists() {
        bail!(
            "{} already exists — remove it first to regenerate",
            path.display()
        );
    }

    let template = r#"# Capture configuration for visual regression testing.
#
# This file controls how the headless capture binary renders and screenshots
# your widget. All fields are optional — sensible defaults are used when omitted.

# ── Timing ───────────────────────────────────────────────────────────
#
# The start time comes from the fixture header, not from here.

# Extra frames to render before every capture, ahead of the I/O drain (default: 0).
# Each one advances the replay clock by 16 ms and delivers timeline events.
# A widget whose visuals follow the clock captures a later state than at 0.
# settle_delay = 30

# ── Fixtures ─────────────────────────────────────────────────────────

# A fixture is a named dataset replayed against one or more targets, written
# `<platform>:<viewport>`. Frames land in
# `<output>/<platform>/<viewport>/<dataset>/`.
#
# Record one with: just wasm::record <widget> <platform>:<viewport> <dataset>
#
# [fixtures.bmc100-full]
# path = "fixtures/bmc100-full.jsonl.gz"
# targets = ["bmc100:full"]

# One dataset can drive several targets:
#
# [fixtures.common]
# path = "fixtures/common.jsonl.gz"
# targets = ["bmc100:full", "bmc100:large", "bmc100:medium", "bmc100:small"]

# And a dataset may override the settle delay or seed its own KV:
#
# [fixtures.slow-feed]
# path = "fixtures/slow-feed.jsonl.gz"
# targets = ["bfm100:full"]
# settle_delay = 40
# kv = { theme = "dark" }
"#;

    std::fs::write(&path, template)
        .with_context(|| format!("failed to write {}", path.display()))?;
    eprintln!("Created {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_io_drain_fails_at_its_ceiling() {
        let before_ceiling = OFFLINE_IO_DRAIN_TIMEOUT
            .checked_sub(Duration::from_millis(1))
            .expect("BUG: offline I/O drain timeout exceeds one millisecond");
        assert!(
            !io_drain_deadline_reached(false, before_ceiling)
                .expect("BUG: an offline drain below its ceiling must continue")
        );

        let error = io_drain_deadline_reached(false, OFFLINE_IO_DRAIN_TIMEOUT)
            .expect_err("BUG: an offline drain at its ceiling must fail");
        assert_eq!(
            error.to_string(),
            "offline capture I/O did not settle within 60 s"
        );
    }

    #[test]
    fn online_io_drain_stops_at_its_ceiling() {
        let before_ceiling = ONLINE_IO_DRAIN_TIMEOUT
            .checked_sub(Duration::from_millis(1))
            .expect("BUG: online I/O drain timeout exceeds one millisecond");
        assert!(
            !io_drain_deadline_reached(true, before_ceiling)
                .expect("BUG: an online drain below its ceiling must continue")
        );
        assert!(
            io_drain_deadline_reached(true, ONLINE_IO_DRAIN_TIMEOUT)
                .expect("BUG: an online drain at its ceiling must stop cleanly")
        );
    }

    #[test]
    fn a_round_target_carries_round_through_to_the_guest() {
        let round: Target = "bfm100:full".parse().expect("BUG: target must parse");
        assert_eq!(
            round.viewport.runtime_viewport_shape(),
            bmc_wasm_protocol::ViewportShape::Round
        );
        assert_eq!(
            round.platform.runtime_display_info().shape,
            bmc_wasm_protocol::DisplayShape::Round
        );

        let slot: Target = "bmc100:small".parse().expect("BUG: target must parse");
        assert_eq!(
            slot.viewport.runtime_viewport_shape(),
            bmc_wasm_protocol::ViewportShape::Rectangular
        );
    }

    #[test]
    fn an_online_sweep_names_datasets_as_a_single_target_run_would() {
        let swept = catalog_matrix();
        let offered: usize = bmc_wasm_runtime::platform_catalog::PLATFORMS
            .iter()
            .map(|platform| platform.viewports.len())
            .sum();
        assert_eq!(
            swept.len(),
            offered,
            "an online sweep must cover every target the catalog offers",
        );

        for (dataset, target) in swept {
            let single = sole_dataset_for(&CaptureConfig::default(), target, true)
                .expect("BUG: an unconfigured target must fall back to one name");
            assert_eq!(
                dataset, single,
                "swept and single-target runs must write the same frame directory",
            );
        }
    }

    /// Without one, the run would capture whatever the widget draws
    /// with no data at all, and file it as if it were a fixture.
    #[test]
    fn an_offline_run_needs_a_dataset_to_replay() {
        let target: Target = "bmc100:full".parse().expect("BUG: target must parse");
        let refused = sole_dataset_for(&CaptureConfig::default(), target, false)
            .expect_err("an offline run has nothing to replay");
        assert!(
            refused.to_string().contains("--online"),
            "the refusal names the way out, got: {refused}",
        );
    }

    #[test]
    fn a_run_carrying_its_own_fixture_needs_no_configured_dataset() {
        let target: Target = "bmc100:full".parse().expect("BUG: target must parse");
        assert_eq!(
            sole_dataset_for(&CaptureConfig::default(), target, true)
                .expect("a run bringing its own data needs only a name"),
            "full",
        );
    }

    #[test]
    fn a_slot_target_reports_the_whole_display_not_the_slot() {
        let slot: Target = "bmc100:small".parse().expect("BUG: target must parse");
        let display = slot.platform.runtime_display_info();

        assert_eq!((slot.viewport.width, slot.viewport.height), (317, 238));
        assert_eq!(
            (display.width, display.height, display.dpi),
            (1_280, 480, 217),
            "a slot capture must tell the widget about the whole Deck panel",
        );
    }

    /// The shape a nix build hands `capture run` — the wasm nowhere near
    /// the manifest that admits it.
    #[test]
    fn a_manifest_is_reached_through_the_capture_dir() {
        let widget = tempfile::tempdir().expect("BUG: widget root");
        std::fs::write(
            widget.path().join("manifest.json"),
            r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440202",
            "version": "0.1.0",
            "name": "out-of-tree",
            "description": "built somewhere its source is not",
            "author": { "name": "Braiins Forge", "url": "https://braiinsforge.com" },
            "binary": "bin/out-of-tree",
            "icon": "assets/icon.svg",
            "category": "utility",
            "settings": [],
            "supported_viewports": [{ "type": "rectangular" }],
            "params": {}
        }"#,
        )
        .expect("BUG: write manifest");
        let capture_dir = widget.path().join("capture");
        std::fs::create_dir(&capture_dir).expect("BUG: capture dir");

        let elsewhere = tempfile::tempdir().expect("BUG: build output");
        let wasm = elsewhere.path().join("out_of_tree.wasm");
        std::fs::write(&wasm, b"").expect("BUG: write wasm");

        assert!(
            bmc_wasm_runtime::fixtures::find_widget_root(&wasm).is_none(),
            "the wasm must be out of reach, or this proves nothing"
        );
        let manifest = load_widget_manifest(&wasm, Some(&capture_dir))
            .expect("the capture dir's parent is the widget root");
        assert_eq!(manifest.name, "out-of-tree");
    }

    #[test]
    fn manifest_dpi_bounds_gate_a_target() {
        let manifest: bmc_widget_manifest::Manifest = r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440200",
            "version": "0.1.0",
            "name": "dpi-picky",
            "description": "declares a density floor",
            "author": { "name": "Braiins Forge", "url": "https://braiinsforge.com" },
            "binary": "bin/dpi-picky",
            "icon": "assets/icon.svg",
            "category": "utility",
            "settings": [],
            "supported_viewports": [{ "type": "rectangular", "min_dpi": 200 }],
            "params": {}
        }"#
        .parse()
        .expect("BUG: test manifest must parse");

        assert_eq!(
            target_admitted(&manifest, "bmc100:full".parse().expect("BUG: parse")),
            Ok(()),
            "BMC100 is 217 dpi, inside the declared range",
        );
        assert_eq!(
            target_admitted(&manifest, "bmm100:full".parse().expect("BUG: parse")),
            Err(bmc_widget_manifest::ViewportDeclined::Dpi),
            "BMM100 is 141 dpi, below the declared minimum",
        );
        assert_eq!(
            target_admitted(&manifest, "bfm100:full".parse().expect("BUG: parse")),
            Err(bmc_widget_manifest::ViewportDeclined::Geometry),
            "BFM100 is round, and only a rectangular viewport is declared",
        );
    }

    #[test]
    fn split_unified_events_preserves_repeated_fetch_order() {
        use bmc_wasm_runtime::system::SystemSnapshot;
        use bmc_wasm_runtime::unified_fixture::{
            FixtureBody, FixtureHeader, TimelineEvent, UnifiedEvent, UnifiedFixture,
        };

        let fixture = UnifiedFixture {
            header: FixtureHeader {
                time: "2026-05-30T12:00:00Z".to_owned(),
                kv: std::collections::HashMap::new(),
                initial_params: serde_json::Map::new(),
                initial_system: SystemSnapshot::default(),
                initial_credentials: serde_json::Map::new(),
            },
            events: vec![
                TimelineEvent {
                    at_ms: 0,
                    event: UnifiedEvent::Fetch {
                        method: "GET".to_owned(),
                        url: "http://miner/api/v1/miner/stats".to_owned(),
                        status: 200,
                        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                        body: FixtureBody::Text("first".to_owned()),
                    },
                },
                // No headers, as a fixture recorded before they were carried.
                TimelineEvent {
                    at_ms: 1,
                    event: UnifiedEvent::Fetch {
                        method: "GET".to_owned(),
                        url: "http://miner/api/v1/miner/stats".to_owned(),
                        status: 200,
                        headers: Vec::new(),
                        body: FixtureBody::Text("second".to_owned()),
                    },
                },
            ],
        };

        let (fetches, network_events) = split_unified_events(&fixture);
        assert!(network_events.is_empty());
        let entries = fetches
            .get("GET http://miner/api/v1/miner/stats")
            .expect("BUG: stats fetch should be captured");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].body, b"first");
        assert_eq!(entries[1].body, b"second");
        assert_eq!(
            entries[0].headers,
            vec![("content-type".to_owned(), "application/json".to_owned())],
            "each reply keeps its own headers, not the queue's first"
        );
        assert!(
            entries[1].headers.is_empty(),
            "a fixture recorded without headers replays as an origin that sent none"
        );
    }
}
