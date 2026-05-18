// Copyright (C) 2026  Braiins Systems s.r.o.

//! Widget development testbed with hot-reloading.
//!
//! Built on [`eframe`] so the same egui patterns used by `bmc-virt-console`
//! carry over (window + GL context owned by eframe, custom GL via the `glow` backend,
//! native textures registered with the egui frame for painting).
//!
//! Renders all four widget-size variants in a fixed-layout window
//! plus stats / LED-strip / recording UI overlays.
//!
//! Split into sibling modules so each concern is reviewable on its own:
//! - [`paint`] — GL plumbing (FBO+texture), checkerboard, LED strip, timing chart, perf report.
//! - [`recording`] — gesture tracking, recording panel, fixture-finishing.
//! - [`params_ui`] — right-side params sidebar + per-row typed inputs + delivery path.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::integer_division,
    clippy::items_after_statements,
    reason = "UI math on small bounded positive values plus inline ui-block constants \
              placed next to where they're used — all intentional in this testbed binary"
)]

mod paint;
mod params_ui;
mod recording;
mod system_ui;
mod ui_helpers;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use eframe::glow::HasContext as _;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::interaction::TouchEvent;
use bmc_render::renderer::Renderer as _;
use bmc_wasm_runtime::fixtures::{self, find_widget_root, seed_kv_from_secrets, snapshot_kv_dir};
use bmc_wasm_runtime::unified_fixture::TimelineEvent;
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, SystemSnapshot, WasmWidgetRuntime};

use paint::{
    GlProcAddress, TileGpu, draw_checkerboard, paint_led_strip, paint_timing_chart,
    paint_timing_legend, proc_loader, write_perf_report,
};
use recording::{
    GestureTracker, RecordingAction, RecordingState, classify_and_record_gesture,
    record_size_to_idx,
};

// ── Layout constants ────────────────────────────────────────────────

const PREVIEW_GAP: u32 = 16;
const PREVIEW_MARGIN: u32 = 16;
/// Height of the LED diffuser strip rendered below each tile.
pub(crate) const LED_STRIP_H: u32 = 24;
/// Number of simulated LEDs across the strip.
pub(crate) const LED_COUNT: usize = 10;
// Widget size presets (logical pixels)
const TILE_FULL_W: u32 = 1280;
const TILE_FULL_H: u32 = 480;
const TILE_LARGE_W: u32 = 638;
const TILE_LARGE_H: u32 = 480;
const TILE_MEDIUM_W: u32 = 638;
const TILE_MEDIUM_H: u32 = 238;
const TILE_SMALL_W: u32 = 317;
const TILE_SMALL_H: u32 = 238;

const INNER_W: u32 = if TILE_FULL_W > TILE_LARGE_W + PREVIEW_GAP + TILE_MEDIUM_W {
    TILE_FULL_W
} else {
    TILE_LARGE_W + PREVIEW_GAP + TILE_MEDIUM_W
};
const PREVIEW_WIDTH: u32 = PREVIEW_MARGIN + INNER_W + PREVIEW_MARGIN;
const RIGHT_COL_H: u32 = TILE_MEDIUM_H + LED_STRIP_H + PREVIEW_GAP + TILE_SMALL_H + LED_STRIP_H;
const LEFT_COL_H: u32 = TILE_LARGE_H + LED_STRIP_H;
const ROW1_H: u32 = if LEFT_COL_H > RIGHT_COL_H {
    LEFT_COL_H
} else {
    RIGHT_COL_H
};
const PREVIEW_HEIGHT: u32 =
    PREVIEW_MARGIN + (TILE_FULL_H + LED_STRIP_H + PREVIEW_GAP) + ROW1_H + PREVIEW_MARGIN;

const M: u32 = PREVIEW_MARGIN;
const G: u32 = PREVIEW_GAP;
const fn row_stride(h: u32) -> u32 {
    h + LED_STRIP_H + G
}
const ROW0_Y: u32 = M;
const ROW1_Y: u32 = ROW0_Y + row_stride(TILE_FULL_H);
const RIGHT_COL_X: u32 = M + TILE_LARGE_W + G;

/// (x, y, w, h, label) — tile positions in logical pixels.
const TILE_DEFS: [(u32, u32, u32, u32, &str); 4] = [
    (M, ROW0_Y, TILE_FULL_W, TILE_FULL_H, "FULL"),
    (M, ROW1_Y, TILE_LARGE_W, TILE_LARGE_H, "LARGE"),
    (RIGHT_COL_X, ROW1_Y, TILE_MEDIUM_W, TILE_MEDIUM_H, "MEDIUM"),
    (
        RIGHT_COL_X,
        ROW1_Y + row_stride(TILE_MEDIUM_H),
        TILE_SMALL_W,
        TILE_SMALL_H,
        "SMALL",
    ),
];

// Stats panel position: empty area right of SMALL tile, below MEDIUM
const STATS_X: u32 = RIGHT_COL_X + TILE_SMALL_W + G;
const STATS_Y: u32 = ROW1_Y + row_stride(TILE_MEDIUM_H);
const STATS_W: u32 = PREVIEW_WIDTH - M - STATS_X;
const STATS_H: u32 = PREVIEW_HEIGHT - M - STATS_Y;

/// Width of the right-side sidebar housing both the per-widget Params
/// section (when the manifest declares any) and the deck-wide System
/// section (always shown). Added to the window's outer size so the tile
/// area stays at native dimensions instead of getting squeezed.
pub(crate) const PARAM_PANEL_W: u32 = 320;

// ── CLI ─────────────────────────────────────────────────────────────

struct CliArgs {
    wasm_path: PathBuf,
    manifest_path: Option<PathBuf>,
    perf_report_path: Option<PathBuf>,
    perf_frames: u32,
    record_size: Option<String>,
}

fn parse_args() -> Result<CliArgs> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        anyhow::bail!(
            "WASM Widget Testbed\n\
             Usage: testbed <wasm_file> [--manifest=<path>] [--perf-report=<path>] \
             [--perf-frames=<N>] [--record=<size>]"
        );
    }

    let wasm_path = PathBuf::from(&args[1]);
    let mut perf_report_path = None;
    let mut perf_frames: u32 = 600;
    let mut record_size = None;
    let mut manifest_path = None;
    for arg in &args[2..] {
        if let Some(path) = arg.strip_prefix("--manifest=") {
            manifest_path = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--perf-report=") {
            perf_report_path = Some(PathBuf::from(path));
        } else if let Some(n) = arg.strip_prefix("--perf-frames=") {
            perf_frames = n.parse().unwrap_or(600);
        } else if let Some(s) = arg.strip_prefix("--record=") {
            record_size = Some(s.to_owned());
        }
    }
    Ok(CliArgs {
        wasm_path,
        manifest_path,
        perf_report_path,
        perf_frames,
        record_size,
    })
}

/// Walk up the wasm path looking for `<package>/manifest.json` at each level.
/// Cargo emits widget binaries with `-` in the package name normalised to `_`.
fn autodetect_manifest(wasm_path: &Path) -> Option<PathBuf> {
    let stem = wasm_path.file_stem()?.to_str()?;
    let package = stem.replace('_', "-");
    let mut dir = wasm_path.parent()?;
    while let Some(parent) = dir.parent() {
        let candidate = parent.join(&package).join("manifest.json");
        if candidate.exists() {
            return Some(candidate);
        }
        dir = parent;
    }
    None
}

fn load_manifest(
    wasm_path: &Path,
    explicit: Option<PathBuf>,
) -> Result<(PathBuf, bmc_widget_manifest::Manifest)> {
    let manifest_path = explicit
        .or_else(|| autodetect_manifest(wasm_path))
        .with_context(|| {
            format!(
                "could not locate manifest.json for {}. Pass --manifest=<path> explicitly.",
                wasm_path.display()
            )
        })?;
    let body = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest = <bmc_widget_manifest::Manifest as std::str::FromStr>::from_str(&body)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    Ok((manifest_path, manifest))
}

/// Initial params snapshot from the manifest: every declared key bound to its
/// `ParamValue::from_param_kind_default`. Mirrors what the compositor delivers on-device
/// when no operator overrides are set.
fn manifest_default_params(
    manifest: &bmc_widget_manifest::Manifest,
) -> std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue> {
    manifest
        .params
        .iter()
        .map(|(key, def)| {
            (
                key.clone(),
                bmc_widget_manifest::ParamValue::from_param_kind_default(&def.kind),
            )
        })
        .collect()
}

// ── Memory stats (Linux only) ───────────────────────────────────────

fn current_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.trim().strip_suffix("kB")?.trim().parse().ok();
        }
    }
    None
}

/// Log RSS deltas at app startup once GL + the WASM runtime are wired up.
/// The pre-GL baseline is taken before `eframe::run_native` is called, so the difference
/// reported here captures GL initialisation + first-runtime construction.
fn log_startup_memory(rss_before_gl_kb: Option<u64>) {
    let now = current_rss_kb();
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return;
    };
    eprintln!("\n=== Memory (startup) ===");
    if let (Some(before), Some(now)) = (rss_before_gl_kb, now) {
        let delta = now.saturating_sub(before);
        eprintln!("Pre-eframe RSS:    {before:>6} kB");
        eprintln!("Post-init RSS:     {now:>6} kB ({delta:+} kB)");
    }
    for line in status.lines() {
        if line.starts_with("VmPeak:") || line.starts_with("VmRSS:") || line.starts_with("VmHWM:") {
            eprintln!("{}", line.trim());
        }
    }
}

// ── main ────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    bmc_render::tree::init_debug_flags();

    let cli = parse_args()?;
    let (manifest_path, manifest) = load_manifest(&cli.wasm_path, cli.manifest_path.clone())?;
    let params = manifest_default_params(&manifest);

    // Window width includes the right-side sidebar that houses both Params
    // (when the manifest declares any) and the always-on System section.
    // Central tile area stays at native dimensions so widgets are never squeezed.
    let outer_w = (PREVIEW_WIDTH + PARAM_PANEL_W) as f32;
    let outer_h = PREVIEW_HEIGHT as f32;

    println!("Loading widget from: {}", cli.wasm_path.display());
    println!("Manifest:            {}", manifest_path.display());
    println!(
        "Params:              {} key(s) from manifest defaults",
        params.len()
    );
    println!(
        "Display size: {PREVIEW_WIDTH}x{PREVIEW_HEIGHT} (4 sizes); \
         requested outer: {outer_w}x{outer_h}"
    );
    if let Some(ref path) = cli.perf_report_path {
        println!(
            "Perf report: {} ({} frames)",
            path.display(),
            cli.perf_frames
        );
    }
    if let Some(ref size) = cli.record_size {
        println!("Recording mode: size={size}");
    }

    let rss_before_gl = current_rss_kb();

    // `inner` + `max` only; deliberately no `with_min_inner_size`. Wayland's `Invalid
    // min/max size` error fires when min > max, so omitting min sidesteps it while max==inner
    // still caps the resize. `with_resizable(false)` is left off because it gets silently
    // honoured-or-not depending on the compositor; the post-frame `Resizable(false)` command
    // takes care of it once the surface exists.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([outer_w, outer_h])
            .with_max_inner_size([outer_w, outer_h])
            .with_title("WASM Widget Testbed"),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        // Persistence defaults to true — eframe saves/restores window size across runs,
        // which silently undoes `with_inner_size` once the user has launched once. The
        // testbed's window dimensions are derived from constants every launch, so saved
        // state is purely harmful.
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        "WASM Widget Testbed",
        options,
        Box::new(move |cc| {
            let app = TestbedApp::new(cc, cli, manifest, params, egui::vec2(outer_w, outer_h))?;
            log_startup_memory(rss_before_gl);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

// ── Hot reload ──────────────────────────────────────────────────────

/// Watch the directory containing `path` for relevant changes; send `()`
/// on its receiver whenever the target file is created/modified/removed.
///
/// Returns both the live watcher (must be kept alive) and the receiver.
fn setup_watcher(path: &Path) -> Result<(RecommendedWatcher, std::sync::mpsc::Receiver<()>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let target = path.canonicalize()?;
    let parent = target.parent().context("no parent directory")?.to_owned();
    let target_file_name = target.file_name().map(ToOwned::to_owned);

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, _>| {
            if let Ok(event) = res {
                let is_relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                );
                if !is_relevant {
                    return;
                }
                let targets_match = event.paths.iter().any(|p| {
                    if p == &target {
                        return true;
                    }
                    if let Some(name) = &target_file_name
                        && p.file_name() != Some(name.as_ref())
                    {
                        return false;
                    }
                    p.canonicalize().ok().as_ref() == Some(&target)
                });
                if targets_match {
                    let _ = tx.send(());
                }
            }
        },
        notify::Config::default(),
    )?;
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}

// ── Touch routing ───────────────────────────────────────────────────

/// Translate egui pointer events on a tile rect into `TouchEvent`s pushed to the runtime.
///
/// Click / drag semantics mirror what the prior winit-based testbed forwarded:
/// a quick click fires `Down` then `Up`; a drag fires `Down` on start,
/// `Move` on each frame the pointer moved, and `Up` on release.
///
/// When `recording` is `Some`, also tracks the gesture (start/current pos + start element)
/// so the recording-side gesture classifier can turn it into a Click / Scroll / Drag
/// `UnifiedEvent` on release.
fn dispatch_touch_events(
    response: &egui::Response,
    rect: egui::Rect,
    runtime: &mut WasmWidgetRuntime,
    recording: Option<&mut RecordingState>,
) {
    // Carry the recording reborrow through each branch by hand instead of `as_deref_mut`
    // (which clippy rejects since the `Option`'s inner type is already a `&mut`).
    let mut rec = recording;
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (x, y) = (pos.x - rect.min.x, pos.y - rect.min.y);
        runtime.push_touch_event(TouchEvent::Down { x, y });
        runtime.push_touch_event(TouchEvent::Up);
        if let Some(r) = rec.as_mut() {
            // A quick click never triggers `drag_started` — synthesise + immediately classify
            // a zero-distance gesture so it's recorded as a click on the hit element.
            let start_element = runtime.hit_test(x, y);
            let gesture = GestureTracker {
                start_pos: (x, y),
                current_pos: (x, y),
                start_element,
            };
            classify_and_record_gesture(r, &gesture);
        }
    }
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (x, y) = (pos.x - rect.min.x, pos.y - rect.min.y);
        runtime.push_touch_event(TouchEvent::Down { x, y });
        if let Some(r) = rec.as_mut() {
            let start_element = runtime.hit_test(x, y);
            r.gesture = Some(GestureTracker {
                start_pos: (x, y),
                current_pos: (x, y),
                start_element,
            });
        }
    } else if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (x, y) = (pos.x - rect.min.x, pos.y - rect.min.y);
        runtime.push_touch_event(TouchEvent::Move { x, y });
        if let Some(r) = rec.as_mut()
            && let Some(g) = r.gesture.as_mut()
        {
            g.current_pos = (x, y);
        }
    }
    if response.drag_stopped() {
        runtime.push_touch_event(TouchEvent::Up);
        if let Some(r) = rec.as_mut()
            && let Some(gesture) = r.gesture.take()
        {
            classify_and_record_gesture(r, &gesture);
        }
    }
}

// ── Tile ────────────────────────────────────────────────────────────

pub(crate) struct PreviewTile {
    pub(crate) runtime: WasmWidgetRuntime,
    /// Caller-owned renderer drawn alongside `runtime`. Bracket each
    /// `runtime.render(...)` call with `runtime.with_renderer(ptr, ...)`.
    pub(crate) renderer: FemtoVgRenderer,
    pub(crate) gpu: TileGpu,
    pub(crate) x: u32,
    pub(crate) y: u32,
    label: &'static str,
    logged_dead: bool,
    ever_rendered: bool,
    /// Receiver for LED commands from the widget (drained each frame).
    led_rx: std::sync::mpsc::Receiver<bmc_led::data::LedCommand>,
    /// Current LED scene (from last `SetEffect` command).
    pub(crate) led_scene: Option<bmc_led::data::LedScene>,
    /// Whether LEDs are enabled.
    pub(crate) led_enabled: bool,
}

impl PreviewTile {
    /// Drain pending LED commands; update `led_scene` / `led_enabled`.
    fn drain_led_commands(&mut self) {
        while let Ok(cmd) = self.led_rx.try_recv() {
            use bmc_led::data::LedCommand;
            match cmd {
                LedCommand::SetEffect(scene) => self.led_scene = Some(scene),
                LedCommand::Enable => self.led_enabled = true,
                LedCommand::Disable => self.led_enabled = false,
                LedCommand::SetBrightness(_) => {}
            }
        }
    }
}

// ── App ─────────────────────────────────────────────────────────────

pub(crate) struct TestbedApp {
    cli: CliArgs,
    /// Requested window size. Sent as `ViewportCommand::InnerSize` repeatedly until the
    /// compositor actually applies it — `with_inner_size` at startup gets silently clamped
    /// on some GNOME/Wayland setups regardless of `persist_window: false`.
    requested_size: egui::Vec2,
    /// Frames remaining in the size-pin retry budget. Counts down from a small cap; we stop
    /// requesting repaints once it hits zero so we never end up in an infinite resize loop
    /// when the compositor refuses the requested size outright.
    size_pin_attempts: u8,
    /// Parsed manifest — read by the param-mutation panel to render type-appropriate inputs
    /// (ComboBox for enums, DragValue for numerics with min/max/step, etc.).
    pub(crate) manifest: bmc_widget_manifest::Manifest,
    /// Current per-instance params snapshot. Mutated by the param-mutation UI; the
    /// underlying runtimes are kept in sync via `deliver_params_update` on each change.
    pub(crate) params:
        std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
    /// Current deck-wide system snapshot. Mutated by the system-mutation UI
    /// on the left sidebar; tile runtimes are kept in sync via `deliver_system_update`
    /// on each change. Pre-recording UI changes are captured into `RecordingState::system_snapshot`;
    /// subsequent changes produce `UnifiedEvent::SystemDelivery` entries in the timeline.
    pub(crate) system: SystemSnapshot,
    gl: Arc<eframe::glow::Context>,
    pub(crate) tiles: Vec<PreviewTile>,
    clock: Clock,
    hot_reload: HotReload,
    perf: PerfState,
    pub(crate) recording_mode: RecordingMode,
}

/// Wall-clock instants used to drive per-frame timing.
/// `last_frame` advances on every `ui` call and yields `delta_ms` for the WASM runtime;
/// `start_instant` is the fixed origin for the monotonic clock the runtime sees.
struct Clock {
    last_frame: std::time::Instant,
    start_instant: std::time::Instant,
}

/// Filesystem watcher + manual-reload signal. Drains as a single "rebuild every runtime"
/// signal each frame inside `poll_hot_reload`.
struct HotReload {
    /// Live `notify` watcher. Held to keep the watch thread alive — when dropped, file
    /// events stop arriving. Never read after construction.
    _watcher: RecommendedWatcher,
    /// Channel fed by `setup_watcher` whenever the wasm file on disk changes.
    watcher_rx: std::sync::mpsc::Receiver<()>,
    /// Set by the "Reload WASM" button in the stats panel; consumed as a synthetic watcher
    /// event on the next `poll_hot_reload` tick.
    manual_reload: bool,
}

/// Per-frame performance accounting. The rolling window drives the FPS readout
/// in the stats panel; the full vector is what `--perf-report=` writes to disk at exit.
struct PerfState {
    /// Total frames rendered so far. Used to drive the `--perf-frames` exit condition.
    frame_count: u32,
    /// Per-frame timings from FULL tile's runtime; written to disk by `--perf-report=` at exit.
    samples: Vec<bmc_render::FrameTimings>,
    /// Last frame's wall-clock duration (microseconds); recent samples averaged for FPS.
    recent_frame_us: std::collections::VecDeque<u32>,
}

/// Recording-mode bundle: the optional in-flight recording state plus the shared
/// fetch buffer the active tile's fetch observer pushes into.
pub(crate) struct RecordingMode {
    /// `Some` only when started via `--record=<size>`; `None` resets it after Save/Cancel.
    pub(crate) state: Option<RecordingState>,
    /// Shared buffer for fetch events captured by the active tile's fetch observer. Held
    /// behind `Arc<Mutex<_>>` because the observer runs on background fetch threads.
    pub(crate) fetch_events: std::sync::Arc<std::sync::Mutex<Vec<TimelineEvent>>>,
}

impl TestbedApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        cli: CliArgs,
        manifest: bmc_widget_manifest::Manifest,
        params: std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
        requested_size: egui::Vec2,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let gl = cc
            .gl
            .as_ref()
            .ok_or("glow backend required (eframe::Renderer::Glow)")?
            .clone();
        let get_proc = cc
            .get_proc_address
            .clone()
            .ok_or("glow backend must expose get_proc_address")?;
        // Stash the loader so `init_tiles` can pull it back when the eframe::Frame
        // is in scope (the lazy init path). Cleared on Drop so a fresh app instance
        // doesn't inherit stale handles.
        GET_PROC_ADDRESS.with(|cell| *cell.borrow_mut() = Some(get_proc));

        let (watcher, watcher_rx) =
            setup_watcher(&cli.wasm_path).map_err(|e| format!("watcher: {e}"))?;

        // Starting system snapshot for the testbed.
        // The real-device path populates this from the wayland `SettingUpdate` stream;
        // the testbed bootstraps with defaults plus a sensible non-empty timezone
        // so the demo cells aren't blank on first paint.
        //
        // Operator changes go through `apply_system_update` and propagate to every tile.
        let pending_system = SystemSnapshot {
            settings: bmc_wasm_runtime::SystemSettings {
                timezone: "Europe/Prague".to_owned(),
                ..bmc_wasm_runtime::SystemSettings::default()
            },
            next_alarm: None,
        };

        let recording_state = cli.record_size.as_ref().map(|size_name| {
            let active_tile = record_size_to_idx(size_name);
            let widget_root = find_widget_root(&cli.wasm_path);
            // Capture's fixture-header parser requires a timezone suffix on the time
            // field (e.g. `2026-05-13T15:48:38+02:00`); a naive datetime is rejected.
            let start_time_iso = chrono::Local::now().to_rfc3339();
            // Initial params snapshot — what the host has staged in `RuntimeConfig::params`
            // at this moment, pre-encoded into the JSON shape `FixtureHeader::initial_params`
            // expects. Captured at recording start so the fixture is self-contained:
            // replay no longer needs to locate the widget's `manifest.json` on disk to
            // reconstruct the starting snapshot.
            let params_snapshot: serde_json::Map<String, serde_json::Value> = params
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), v.to_json_value()))
                .collect();
            RecordingState {
                active_tile,
                size_name: size_name.clone(),
                events: Vec::new(),
                gesture: None,
                widget_root,
                recording_start: std::time::Instant::now(),
                kv_snapshot: std::collections::HashMap::new(),
                params_snapshot,
                // Testbed's starting `SystemSnapshot`, mirroring
                // `params_snapshot`. Replay installs this directly into
                // `RuntimeConfig::system`.
                system_snapshot: pending_system.clone(),
                start_time_iso,
                auto_capture: true,
            }
        });

        let now = std::time::Instant::now();
        Ok(Self {
            cli,
            requested_size,
            // 30 attempts at ~16ms = ~0.5 s of negotiation.
            // More than enough for any compositor to settle,
            // far less than long enough to feel like the UI froze.
            size_pin_attempts: 30,
            manifest,
            params,
            system: pending_system,
            gl,
            tiles: Vec::new(),
            clock: Clock {
                last_frame: now,
                start_instant: now,
            },
            hot_reload: HotReload {
                _watcher: watcher,
                watcher_rx,
                manual_reload: false,
            },
            perf: PerfState {
                frame_count: 0,
                samples: Vec::new(),
                recent_frame_us: std::collections::VecDeque::with_capacity(60),
            },
            recording_mode: RecordingMode {
                state: recording_state,
                fetch_events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            },
        })
    }

    /// Drain pending watcher events; if any fired, rebuild every tile's `WasmWidgetRuntime`
    /// from the (now-updated) wasm bytes on disk.
    /// FBO + texture are kept across reloads since their dimensions don't change.
    fn poll_hot_reload(&mut self, frame: &mut eframe::Frame) {
        let mut needs_reload = self.hot_reload.manual_reload;
        self.hot_reload.manual_reload = false;
        while self.hot_reload.watcher_rx.try_recv().is_ok() {
            needs_reload = true;
        }
        if !needs_reload {
            return;
        }
        let Some(get_proc) = Self::gl_proc_address() else {
            return;
        };
        let wasm_bytes = match std::fs::read(&self.cli.wasm_path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    "hot reload: failed to read {}: {e}",
                    self.cli.wasm_path.display()
                );
                return;
            }
        };
        tracing::info!(
            "hot reload: rebuilding {} tile runtime(s)",
            self.tiles.len()
        );
        for tile in &mut self.tiles {
            let (led_tx, led_rx) = std::sync::mpsc::channel();
            let rt_config = RuntimeConfig {
                mesh_msaa_samples: 4,
                params: self.params.clone(),
                system: self.system.clone(),
                led_command_sender: Some(led_tx),
                ..RuntimeConfig::default()
            };
            // Rebuild the renderer too so atlases/caches don't bleed across reloads.
            //
            // SAFETY: eframe keeps the GL context current for the app's lifetime.
            let new_renderer = match unsafe {
                FemtoVgRenderer::new(
                    proc_loader(get_proc.clone()),
                    tile.gpu.width,
                    tile.gpu.height,
                    tile.gpu.fbo_id(),
                    rt_config.mesh_msaa_samples,
                )
            } {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("hot reload: {} renderer: {e}", tile.label);
                    continue;
                }
            };
            match WasmWidgetRuntime::new(&wasm_bytes, tile.gpu.width, tile.gpu.height, rt_config) {
                Ok(rt) => {
                    tile.renderer = new_renderer;
                    tile.runtime = rt;
                    tile.led_rx = led_rx;
                    tile.led_scene = None;
                    tile.led_enabled = false;
                    tile.logged_dead = false;
                    tile.ever_rendered = false;
                }
                Err(e) => {
                    tracing::warn!("hot reload: {}: {e}", tile.label);
                }
            }
        }
        // Avoid an unused-`frame` warning while we keep the parameter for future texture
        // re-registration in case the reload ever changes tile dimensions.
        let _ = frame;
    }

    /// Build the four widget tiles on first `ui` call (where `eframe::Frame` is available
    /// for `register_native_glow_texture`).
    fn init_tiles(&mut self, frame: &mut eframe::Frame) -> Result<()> {
        let get_proc = Self::gl_proc_address()
            .ok_or_else(|| anyhow::anyhow!("BUG: get_proc_address vanished after construction"))?;
        let wasm_bytes = std::fs::read(&self.cli.wasm_path)
            .with_context(|| format!("failed to read {}", self.cli.wasm_path.display()))?;
        let active_record_idx = self.recording_mode.state.as_ref().map(|r| r.active_tile);
        let widget_name = self
            .cli
            .wasm_path
            .file_stem()
            .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
        let kv_base = self
            .cli
            .wasm_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("widget_data")
            .join(&widget_name);

        let mut tiles = Vec::with_capacity(TILE_DEFS.len());
        for (tile_idx, &(x, y, w, h, label)) in TILE_DEFS.iter().enumerate() {
            let gpu = TileGpu::new(&self.gl, frame, w, h)?;
            let (led_tx, led_rx) = std::sync::mpsc::channel();
            // Per-tile KV storage matches the prior testbed layout
            // (`./widget_data/<widget>/<size>/`). Active recording tile wipes its KV first
            // so the fixture starts from a known baseline.
            let kv_path = kv_base.join(label.to_ascii_lowercase());
            if active_record_idx == Some(tile_idx) {
                let _ = std::fs::remove_dir_all(&kv_path);
                let _ = std::fs::create_dir_all(&kv_path);
            }
            seed_kv_from_secrets(&self.cli.wasm_path, &kv_path);

            // Active recording tile gets the fixture-recording config with the unified
            // fetch observer; non-recording tiles use the simpler default config.
            let mut rt_config = if active_record_idx == Some(tile_idx) {
                fixtures::build_unified_recording_config(
                    kv_path.clone(),
                    self.recording_mode.fetch_events.clone(),
                    std::time::Instant::now(),
                )
            } else {
                RuntimeConfig {
                    kv_store_path: Some(kv_path.clone()),
                    ..RuntimeConfig::default()
                }
            };
            rt_config.mesh_msaa_samples = 4;
            rt_config.params = self.params.clone();
            rt_config.system = self.system.clone();
            rt_config.led_command_sender = Some(led_tx);
            // SAFETY: eframe keeps the GL context current for the app's lifetime.
            let renderer = unsafe {
                FemtoVgRenderer::new(
                    proc_loader(get_proc.clone()),
                    w,
                    h,
                    gpu.fbo_id(),
                    rt_config.mesh_msaa_samples,
                )
            }
            .with_context(|| format!("create renderer for {label}"))?;
            let runtime = WasmWidgetRuntime::new(&wasm_bytes, w, h, rt_config)
                .with_context(|| format!("create runtime for {label}"))?;
            tiles.push(PreviewTile {
                runtime,
                renderer,
                gpu,
                x,
                y,
                label,
                logged_dead: false,
                ever_rendered: false,
                led_rx,
                led_scene: None,
                led_enabled: false,
            });
        }
        let (major, minor, patch) = tiles[0].runtime.sdk_version();
        println!("Widget SDK version: {major}.{minor}.{patch}");
        // Snapshot the active recording tile's KV directory at start
        // so the fixture's `header.kv` reproduces the initial state on replay.
        if let Some(ref mut rec) = self.recording_mode.state {
            let kv_path = kv_base.join(rec.size_name.to_ascii_lowercase());
            rec.kv_snapshot = snapshot_kv_dir(&kv_path);
        }
        self.tiles = tiles;
        Ok(())
    }

    /// Retrieve `get_proc_address` from the process-wide cell populated in `new`.
    /// Associated (no `&self`) because the loader lives in a thread-local, not on the app.
    fn gl_proc_address() -> Option<GlProcAddress> {
        GET_PROC_ADDRESS.with(|cell| cell.borrow().clone())
    }

    /// Drive one frame: each tile's WASM runtime renders into its FBO. Egui paints the
    /// textures afterward via `painter.image`.
    ///
    /// Saves the GL framebuffer binding + viewport before mutating them per tile
    /// and restores both at the end so egui's own draw list runs against the screen
    /// framebuffer the way it expects.
    ///
    /// Skipping this caused screen-wide trails (egui's clear hit
    /// a tile FBO instead of the default framebuffer).
    fn render_tiles(&mut self, delta_ms: u32) {
        // SAFETY: gl is current on this thread inside `App::ui`; the queries below only read.
        let (prev_fbo, prev_viewport) = unsafe {
            let prev_fbo = self.gl.get_parameter_i32(eframe::glow::FRAMEBUFFER_BINDING);
            let mut vp = [0_i32; 4];
            self.gl
                .get_parameter_i32_slice(eframe::glow::VIEWPORT, &mut vp);
            (prev_fbo, vp)
        };

        let monotonic_ms = self.clock.start_instant.elapsed().as_millis() as u64;
        let system_time = chrono::Local::now().fixed_offset();
        // In recording mode, only the active tile renders; the others are painted as blank
        // slabs in `App::ui`. Skipping the WASM render here both clarifies the visual focus
        // and keeps non-active runtimes from spending fuel on frames nobody will keep.
        let active_record_idx = self.recording_mode.state.as_ref().map(|r| r.active_tile);
        for (tile_idx, tile) in self.tiles.iter_mut().enumerate() {
            if active_record_idx.is_some_and(|active| active != tile_idx) {
                continue;
            }
            tile.drain_led_commands();
            tile.runtime.set_time(system_time, monotonic_ms);
            tile.renderer
                .begin_frame(tile.gpu.width, tile.gpu.height, 1.0);

            // `*mut FemtoVgRenderer` → `*mut dyn Renderer` is a coercion, not an `as` cast.
            let renderer_raw: *mut dyn bmc_render::renderer::Renderer =
                core::ptr::addr_of_mut!(tile.renderer);
            let renderer_ptr = std::ptr::NonNull::new(renderer_raw)
                .expect("BUG: addr_of_mut! cannot produce null");
            tile.runtime.poll_deliveries_with_renderer(renderer_ptr);
            let outcome = tile
                .runtime
                .with_renderer(renderer_ptr, |rt| rt.render(delta_ms));
            match outcome {
                Ok(RenderStatus::Ok) => {
                    tile.ever_rendered = true;
                }
                Ok(RenderStatus::FuelExhausted) => {
                    tracing::warn!("{}: fuel exhausted", tile.label);
                }
                Ok(RenderStatus::Dead) => {
                    if !tile.logged_dead {
                        tracing::error!("{}: widget killed (repeated fuel overages)", tile.label);
                        tile.logged_dead = true;
                    }
                }
                Err(e) => {
                    tracing::error!("{}: render failed: {e}", tile.label);
                }
            }
            tile.renderer.flush();
        }
        // Pick the FULL tile (idx 0) as the perf-report sampling source — matches the prior
        // testbed which sampled tile 0 too. The other tiles still render but aren't reported.
        if let Some(tile) = self.tiles.first() {
            self.perf.samples.push(tile.runtime.last_timings());
        }

        // Restore framebuffer + viewport so egui draws onto the screen FBO at the right size.
        // SAFETY: same context invariants as the read above; values came from this very GL.
        unsafe {
            // 0 maps to the default framebuffer; any non-zero prior binding goes back as a
            // `NativeFramebuffer`. The cast through `NonZeroU32` filters the 0 case correctly.
            let target =
                std::num::NonZeroU32::new(prev_fbo as u32).map(eframe::glow::NativeFramebuffer);
            self.gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, target);
            self.gl.viewport(
                prev_viewport[0],
                prev_viewport[1],
                prev_viewport[2],
                prev_viewport[3],
            );
        }
    }

    /// Paint the stats panel inside an explicit rect (the empty slot right of SMALL tile).
    /// Includes the FPS readout, FULL-tile timing breakdown, reload + debug-toggle buttons,
    /// and a stacked-bar chart of recent per-frame timings.
    fn paint_stats_panel(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        // Backing rectangle so the chart + labels read against a flat colour, not the
        // checkerboard underneath.
        ui.painter()
            .rect_filled(rect, 4.0, egui::Color32::from_gray(18));
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(50)),
            egui::StrokeKind::Inside,
        );

        let pad = 8.0;
        let inner = rect.shrink(pad);

        // ── Top row: Reload + Debug-layout buttons ──
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        child.horizontal(|row| {
            if row.button("Reload WASM").clicked() {
                self.hot_reload.manual_reload = true;
            }
            let mut debug_on = bmc_render::tree::debug_layout_enabled();
            if row.checkbox(&mut debug_on, "Debug layout").changed() {
                bmc_render::tree::toggle_debug_layout();
            }
        });
        child.add_space(8.0);

        // ── FPS + last-frame breakdown ──
        let avg_us = if self.perf.recent_frame_us.is_empty() {
            0
        } else {
            let sum: u32 = self.perf.recent_frame_us.iter().sum();
            sum / self.perf.recent_frame_us.len() as u32
        };
        let fps = if avg_us > 0 {
            1_000_000.0 / avg_us as f32
        } else {
            0.0
        };
        // Stats table — egui::Grid gives column alignment without manual width math.
        // Value strings are padded with leading spaces to a fixed width (`{:>5}`)
        // so under a monospace font the digit columns stay anchored even
        // as the number widens from single digit to triple digit between frames.
        // 5 digits covers up to 99 999 µs (~100 ms / frame), well past
        // the realistic budget for any sub-stage.
        let mono = egui::FontId::monospace(11.0);
        let val_color = egui::Color32::from_gray(220);
        let lbl = |txt: &str| ui_helpers::key_label(txt, 160);
        let cell_us = |n: u32| {
            egui::RichText::new(format!("{n:>5} µs"))
                .font(mono.clone())
                .color(val_color)
        };
        let cell_fps = |f: f32| {
            egui::RichText::new(format!("{f:>5.1} fps"))
                .font(mono.clone())
                .color(val_color)
        };
        egui::Grid::new("testbed_stats_table")
            .num_columns(4)
            .spacing([12.0, 2.0])
            .min_col_width(0.0)
            .show(&mut child, |g| {
                g.add(lbl("frame avg:"));
                g.label(cell_us(avg_us));
                g.add(lbl(""));
                g.label(cell_fps(fps));
                g.end_row();
                if let Some(t) = self.tiles.first().map(|t| t.runtime.last_timings()) {
                    g.add(lbl("FULL wasm:"));
                    g.label(cell_us(t.wasm_us));
                    g.add(lbl("deser:"));
                    g.label(cell_us(t.deserialize_us));
                    g.end_row();
                    g.add(lbl("layout:"));
                    g.label(cell_us(t.layout_us));
                    g.add(lbl("render:"));
                    g.label(cell_us(t.render_us));
                    g.end_row();
                    g.add(lbl("flush:"));
                    g.label(cell_us(t.flush_us));
                    g.add(lbl(""));
                    g.add(lbl(""));
                    g.end_row();
                }
            });

        // ── Stacked bar chart + legend pinned to the bottom (fixed heights) ──
        const CHART_H: f32 = 100.0;
        const LEGEND_H: f32 = 14.0;
        let block_h = CHART_H + LEGEND_H;
        let block_h = block_h.min(inner.height() - (child.cursor().min.y - inner.min.y) - 6.0);
        if block_h > LEGEND_H + 12.0 && !self.perf.samples.is_empty() {
            let block_top = inner.max.y - block_h;
            // Chart on top, legend strip below — keeps the chart visually grouped
            // with the numeric stats above and the legend functions as a key reading downward.
            let chart_rect = egui::Rect::from_min_max(
                egui::pos2(inner.min.x, block_top),
                egui::pos2(inner.max.x, inner.max.y - LEGEND_H),
            );
            let legend_rect = egui::Rect::from_min_max(
                egui::pos2(inner.min.x, inner.max.y - LEGEND_H),
                egui::pos2(inner.max.x, inner.max.y),
            );
            // `painter_at` clips chart draws so spikes don't bleed past the rect.
            let chart_painter = ui.painter_at(chart_rect);
            paint_timing_chart(&chart_painter, chart_rect, &self.perf.samples);
            paint_timing_legend(child.painter(), legend_rect);
        }
    }

    /// Trim `recent_frame_us` to a 60-sample sliding window so the FPS readout averages
    /// roughly the last second at 60 fps.
    fn record_frame_us(&mut self, us: u32) {
        if self.perf.recent_frame_us.len() == 60 {
            self.perf.recent_frame_us.pop_front();
        }
        self.perf.recent_frame_us.push_back(us);
    }
}

// Process-wide cell holding the eframe-provided GL proc address loader, populated in
// `TestbedApp::new` and read by `init_tiles` / `poll_hot_reload`.
//
// A thread-local sidesteps the `dyn Fn` capture lifetime question while keeping
// the closure trivially cloneable. `thread_local!` is a macro, so this stays
// a regular `//` comment — doc comments don't attach to macro invocations.
thread_local! {
    static GET_PROC_ADDRESS: std::cell::RefCell<Option<GlProcAddress>>
        = const { std::cell::RefCell::new(None) };
}

impl eframe::App for TestbedApp {
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // One-shot resize lock. We deliberately do NOT re-send `InnerSize` here — on some
        // Wayland compositors it gets clamped, undoing the larger startup hint. The
        // post-frame `Resizable(false)` command pins whatever size the compositor actually
        // gave us. Setting min/max here would re-trigger `wl_surface error 4` on the same
        // compositors that rejected it at startup.
        if self.size_pin_attempts > 0 {
            let ctx = root_ui.ctx();
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(self.requested_size));
            let actual = ctx.input(|i| i.viewport().inner_rect).map(|r| r.size());
            let matches = actual.is_some_and(|s| {
                (s.x - self.requested_size.x).abs() < 1.0
                    && (s.y - self.requested_size.y).abs() < 1.0
            });
            if matches {
                ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
                self.size_pin_attempts = 0;
            } else {
                self.size_pin_attempts -= 1;
                if self.size_pin_attempts == 0 {
                    // Final attempt exhausted; lock down whatever size we have anyway.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
                }
                ctx.request_repaint();
            }
        }
        // Lazy tile construction — `frame.register_native_glow_texture` needs the
        // `eframe::Frame`, which isn't available in `CreationContext`.
        if self.tiles.is_empty()
            && let Err(e) = self.init_tiles(frame)
        {
            root_ui.label(format!("Failed to init tiles: {e:#}"));
            return;
        }
        // Hot reload check — rebuild runtimes if the wasm changed on disk.
        self.poll_hot_reload(frame);

        let now = std::time::Instant::now();
        let delta = now.duration_since(self.clock.last_frame);
        let delta_ms = delta.as_millis() as u32;
        let frame_us = delta.as_micros() as u32;
        self.clock.last_frame = now;
        self.record_frame_us(frame_us);

        let ctx = root_ui.ctx().clone();

        // Render each widget into its FBO before egui submits its own draw list.
        // Must happen before checkerboard / image draws to keep GL state contained.
        self.render_tiles(delta_ms);
        self.perf.frame_count += 1;

        // Perf-report exit condition: write JSON + close the viewport once we've collected
        // enough samples. Matches the prior `--perf-frames` flag behaviour.
        if let Some(ref path) = self.cli.perf_report_path
            && self.perf.frame_count >= self.cli.perf_frames
        {
            write_perf_report(path, &self.perf.samples);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let time_s = self.clock.start_instant.elapsed().as_secs_f32();

        // Right-side sidebar housing Params (top) and System (bottom) —
        // must be added BEFORE the CentralPanel so it claims its 320 px
        // slice from the right edge first. Changes propagate to all tile
        // runtimes and (when recording) append `ParamDelivery` /
        // `SystemDelivery` events to the timeline.
        self.paint_right_panel(root_ui);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show_inside(root_ui, |ui| {
                let origin = ui.min_rect().left_top();
                // Window-wide checkerboard backdrop so tile boundaries read clearly against
                // widget body colours like params-demo's `#14_16_1B`.
                draw_checkerboard(ui.painter(), ui.max_rect());
                let active_record_idx = self.recording_mode.state.as_ref().map(|r| r.active_tile);
                for (tile_idx, tile) in self.tiles.iter_mut().enumerate() {
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(tile.x as f32, tile.y as f32),
                        egui::vec2(tile.gpu.width as f32, tile.gpu.height as f32),
                    );
                    // Recording mode focuses on a single size — non-active tiles get
                    // a flat dark slab instead of the WASM texture (whose FBO contents
                    // are stale since `render_tiles` skipped them), and don't receive
                    // touch events or an LED strip.
                    //
                    // The active tile gets a thin orange border so
                    // the operator can see which one's live.
                    let is_inactive_record =
                        active_record_idx.is_some_and(|active| active != tile_idx);
                    if is_inactive_record {
                        ui.painter()
                            .rect_filled(rect, 0.0, egui::Color32::from_gray(12));
                        continue;
                    }

                    // FemtoVG renders bottom-up into the FBO; flip V to display top-down.
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0));
                    ui.painter()
                        .image(tile.gpu.egui_tex_id, rect, uv, egui::Color32::WHITE);

                    if active_record_idx == Some(tile_idx) {
                        // `Inside` so the bottom edge stays inside the tile rect
                        // — `Outside` would paint one row below, which `paint_led_strip`
                        // then overwrites with the LED strip background.
                        ui.painter().rect_stroke(
                            rect,
                            0.0,
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 170, 80)),
                            egui::StrokeKind::Inside,
                        );
                    }

                    // Touch / mouse routing: allocate the same rect for click+drag
                    // so we can forward pointer events to the runtime in tile-local
                    // coordinates.
                    //
                    // Recording state is threaded in only for the active recording tile
                    // so gestures on other tiles don't pollute the fixture timeline.
                    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                    let rec_for_tile = if active_record_idx == Some(tile_idx) {
                        self.recording_mode.state.as_mut()
                    } else {
                        None
                    };
                    dispatch_touch_events(&response, rect, &mut tile.runtime, rec_for_tile);

                    paint_led_strip(ui.painter(), tile, origin, time_s);
                }
                // Stats panel / recording panel — both anchor in the empty slot right of SMALL.
                // Recording mode displaces the stats view; the chart isn't useful while
                // authoring a fixture and the operator needs the event log there.
                let stats_rect = egui::Rect::from_min_size(
                    origin + egui::vec2(STATS_X as f32, STATS_Y as f32),
                    egui::vec2(STATS_W as f32, STATS_H as f32),
                );
                if self.recording_mode.state.is_some() {
                    if let Some(action) = self.paint_recording_panel(ui, stats_rect) {
                        match action {
                            RecordingAction::Save => self.finish_recording(),
                            RecordingAction::Cancel => self.recording_mode.state = None,
                            RecordingAction::Capture => self.push_manual_capture(),
                        }
                    }
                } else {
                    self.paint_stats_panel(ui, stats_rect);
                }
            });

        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
