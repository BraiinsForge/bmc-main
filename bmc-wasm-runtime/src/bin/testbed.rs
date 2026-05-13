// Copyright (C) 2026  Braiins Systems s.r.o.

//! Widget development testbed with hot-reloading.
//!
//! Built on [`eframe`] so the same egui patterns used by `bmc-virt-console`
//! carry over (window + GL context owned by eframe, custom GL via the `glow` backend,
//! native textures registered with the egui frame for painting).
//!
//! Renders all four widget-size variants in a fixed-layout window
//! plus stats / LED-strip / recording UI overlays.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::integer_division,
    clippy::items_after_statements,
    reason = "UI / GL math on small bounded positive values, GL u32 enums cast to GLint, \
              and inline ui-block constants placed next to where they're used \
              — all intentional in this single-file testbed binary"
)]

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use eframe::glow::HasContext as _;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use bmc_render::interaction::TouchEvent;
use bmc_render::renderer::Renderer as _;
use bmc_wasm_runtime::fixtures::{self, find_widget_root, seed_kv_from_secrets, snapshot_kv_dir};
use bmc_wasm_runtime::unified_fixture::{
    FixtureHeader, TimelineEvent, UnifiedEvent, UnifiedFixture,
};
use bmc_wasm_runtime::{RenderStatus, RuntimeConfig, WasmWidgetRuntime};

// ── Layout constants ────────────────────────────────────────────────

const PREVIEW_GAP: u32 = 16;
const PREVIEW_MARGIN: u32 = 16;
/// Height of the LED diffuser strip rendered below each tile.
const LED_STRIP_H: u32 = 24;
/// Number of simulated LEDs across the strip.
const LED_COUNT: usize = 10;
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

fn load_manifest_params(
    wasm_path: &Path,
    explicit: Option<PathBuf>,
) -> Result<(
    PathBuf,
    std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
)> {
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
    let params = manifest
        .params
        .iter()
        .map(|(key, def)| {
            (
                key.clone(),
                bmc_widget_manifest::ParamValue::from_param_kind_default(&def.kind),
            )
        })
        .collect();
    Ok((manifest_path, params))
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
    let (manifest_path, params) = load_manifest_params(&cli.wasm_path, cli.manifest_path.clone())?;

    println!("Loading widget from: {}", cli.wasm_path.display());
    println!("Manifest:            {}", manifest_path.display());
    println!(
        "Params:              {} key(s) from manifest defaults",
        params.len()
    );
    println!("Display size: {PREVIEW_WIDTH}x{PREVIEW_HEIGHT} (4 sizes)");
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([PREVIEW_WIDTH as f32, PREVIEW_HEIGHT as f32])
            .with_min_inner_size([PREVIEW_WIDTH as f32, PREVIEW_HEIGHT as f32])
            .with_resizable(false)
            .with_title("WASM Widget Testbed"),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };

    eframe::run_native(
        "WASM Widget Testbed",
        options,
        Box::new(move |cc| {
            // PHASE 1 stub: no WASM runtime wired yet.
            let app = TestbedApp::new(cc, cli, params)?;
            log_startup_memory(rss_before_gl);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

// ── GL helpers ──────────────────────────────────────────────────────

/// FBO + texture pair allocated against eframe's glow context. Each tile owns one;
/// `WasmWidgetRuntime` renders into the FBO and we paint the texture in egui.
struct TileGpu {
    fbo: eframe::glow::Framebuffer,
    /// Held to keep the GL texture alive for as long as the FBO references it; the egui
    /// painter samples this texture by id at draw time, but Rust doesn't see those reads
    /// through the GL boundary, so without retaining we'd risk the texture being collected.
    #[expect(
        dead_code,
        reason = "ownership marker — keeps the FBO's color attachment alive"
    )]
    texture: eframe::glow::Texture,
    egui_tex_id: egui::TextureId,
    width: u32,
    height: u32,
}

impl TileGpu {
    /// Create an `width × height` RGBA8 colour texture + matching framebuffer.
    /// The texture is registered with egui's frame so callers paint it as an `egui::Image`
    /// after the underlying GL render finishes.
    fn new(
        gl: &eframe::glow::Context,
        frame: &mut eframe::Frame,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        // SAFETY: eframe's glow context is current on the calling thread inside `App::ui`.
        unsafe {
            let texture = gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("create_texture: {e}"))?;
            gl.bind_texture(eframe::glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                eframe::glow::TEXTURE_2D,
                0,
                eframe::glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                eframe::glow::RGBA,
                eframe::glow::UNSIGNED_BYTE,
                eframe::glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                eframe::glow::TEXTURE_2D,
                eframe::glow::TEXTURE_MIN_FILTER,
                eframe::glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                eframe::glow::TEXTURE_2D,
                eframe::glow::TEXTURE_MAG_FILTER,
                eframe::glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                eframe::glow::TEXTURE_2D,
                eframe::glow::TEXTURE_WRAP_S,
                eframe::glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                eframe::glow::TEXTURE_2D,
                eframe::glow::TEXTURE_WRAP_T,
                eframe::glow::CLAMP_TO_EDGE as i32,
            );

            let fbo = gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("create_framebuffer: {e}"))?;
            gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                eframe::glow::FRAMEBUFFER,
                eframe::glow::COLOR_ATTACHMENT0,
                eframe::glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            // Stencil renderbuffer — FemtoVG's stroke shader uses stencil.
            let rbo = gl
                .create_renderbuffer()
                .map_err(|e| anyhow::anyhow!("create_renderbuffer: {e}"))?;
            gl.bind_renderbuffer(eframe::glow::RENDERBUFFER, Some(rbo));
            gl.renderbuffer_storage(
                eframe::glow::RENDERBUFFER,
                eframe::glow::DEPTH24_STENCIL8,
                width as i32,
                height as i32,
            );
            gl.framebuffer_renderbuffer(
                eframe::glow::FRAMEBUFFER,
                eframe::glow::DEPTH_STENCIL_ATTACHMENT,
                eframe::glow::RENDERBUFFER,
                Some(rbo),
            );
            gl.bind_renderbuffer(eframe::glow::RENDERBUFFER, None);

            let status = gl.check_framebuffer_status(eframe::glow::FRAMEBUFFER);
            if status != eframe::glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("FBO incomplete: {status:#x}");
            }
            gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, None);
            gl.bind_texture(eframe::glow::TEXTURE_2D, None);

            let native = eframe::glow::NativeTexture(texture.0);
            let egui_tex_id = frame.register_native_glow_texture(native);

            Ok(Self {
                fbo,
                texture,
                egui_tex_id,
                width,
                height,
            })
        }
    }

    /// Numeric FBO ID for `WasmWidgetRuntime::new(... fbo_id ...)`.
    fn fbo_id(&self) -> u32 {
        self.fbo.0.get()
    }
}

/// Wraps `cc.get_proc_address` into the shape `WasmWidgetRuntime::new` accepts (a `&str`-keyed
/// loader). The eframe-provided callback takes `&CStr`; we allocate the `CString` per call
/// since the runtime constructor only runs once per widget construction.
fn proc_loader(get_proc: GlProcAddress) -> impl FnMut(&str) -> *const std::ffi::c_void {
    move |name: &str| {
        let Ok(cstr) = CString::new(name) else {
            return std::ptr::null();
        };
        get_proc(&cstr)
    }
}

/// Eframe's GL function loader closure shape — `&CStr` → raw function pointer.
/// Aliased so the `dyn Fn` trait object isn't spelled out at every storage site.
type GlProcAddress = Arc<dyn Fn(&std::ffi::CStr) -> *const std::ffi::c_void + Send + Sync>;

// ── Recording mode ──────────────────────────────────────────────────

/// Tracks an in-progress touch gesture for recording mode.
struct GestureTracker {
    start_pos: (f32, f32),
    current_pos: (f32, f32),
    start_element: Option<String>,
}

/// Delay between a user action and its auto-inserted capture event (ms).
const AUTO_CAPTURE_DELAY_MS: u64 = 500;
/// Pixel threshold separating "click" from "drag" / "scroll" gestures.
const GESTURE_THRESHOLD: f32 = 5.0;

/// Recording-mode state. Owned by the `TestbedApp` while a recording is active; replaced with
/// `None` on save/cancel. The recording UI panel reads this to render its event log.
struct RecordingState {
    active_tile: usize,
    size_name: String,
    /// Unified timeline events (user actions + fetch recordings).
    events: Vec<TimelineEvent>,
    gesture: Option<GestureTracker>,
    /// Widget root directory (for output paths).
    widget_root: Option<PathBuf>,
    /// Wall-clock reference for `at_ms` calculation.
    recording_start: std::time::Instant,
    /// Snapshot of KV dir state at recording start.
    kv_snapshot: std::collections::HashMap<String, String>,
    /// Start time (ISO 8601) captured at recording start.
    start_time_iso: String,
    /// When true, a Capture event is auto-inserted after each user action.
    auto_capture: bool,
}

fn record_size_to_idx(s: &str) -> usize {
    match s {
        "large" => 1,
        "medium" => 2,
        "small" => 3,
        _ => 0, // "full" and unknown sizes default to tile 0
    }
}

/// Short label for the event log (the icon already carries the type info).
fn format_event_label(event: &UnifiedEvent) -> String {
    match event {
        UnifiedEvent::Capture { duration_ms, fps } => match (duration_ms, fps) {
            (Some(d), Some(f)) => format!("capture({d}ms, {f}fps)"),
            (Some(d), None) => format!("capture({d}ms)"),
            _ => "capture".to_owned(),
        },
        UnifiedEvent::Click { element } => format!("click #{element}"),
        UnifiedEvent::Scroll { element, delta } => format!("scroll #{element}  Δ{delta}"),
        UnifiedEvent::Drag { element, from, to } => {
            format!("drag #{element}  {from:.0}→{to:.0}")
        }
        UnifiedEvent::Fetch {
            method,
            url,
            status,
            ..
        } => format!("{method} {status} {url}"),
        UnifiedEvent::WsOpen { ws_id } | UnifiedEvent::WsMessage { ws_id, .. } => {
            format!("ws#{ws_id}")
        }
        UnifiedEvent::WsClose { ws_id, code } => format!("ws#{ws_id} code={code}"),
        UnifiedEvent::SocketConnected { socket_id }
        | UnifiedEvent::SocketData { socket_id, .. } => format!("tcp#{socket_id}"),
        UnifiedEvent::SocketClosed { socket_id, code } => format!("tcp#{socket_id} code={code}"),
        UnifiedEvent::SsdpFound { search_id, .. } | UnifiedEvent::SsdpRemoved { search_id, .. } => {
            format!("ssdp#{search_id}")
        }
        UnifiedEvent::MdnsFound { browse_id, .. } | UnifiedEvent::MdnsRemoved { browse_id, .. } => {
            format!("mdns#{browse_id}")
        }
        UnifiedEvent::UdpResponse {
            broadcast_id,
            source,
            ..
        } => format!("udp#{broadcast_id} ← {source}"),
        UnifiedEvent::AudioPlay {
            name,
            volume,
            duration_ms,
            ..
        } => format!("audio {name} vol={volume} {duration_ms}ms"),
        UnifiedEvent::LedSetEffect {
            effect,
            r,
            g,
            b,
            period_ms,
            duration_ms,
        } => format!("LED effect={effect} rgb=({r},{g},{b}) p={period_ms}ms d={duration_ms}ms"),
        UnifiedEvent::LedSetBrightness { brightness } => format!("LED brightness={brightness:.2}"),
        UnifiedEvent::LedEnable => "LED enable".to_owned(),
        UnifiedEvent::LedDisable => "LED disable".to_owned(),
    }
}

/// Classify a finished gesture into a `UnifiedEvent` and append it to `rec.events`.
/// Auto-inserts a `Capture` event 500ms later when `auto_capture` is on.
fn classify_and_record_gesture(rec: &mut RecordingState, gesture: &GestureTracker) {
    let dx = gesture.current_pos.0 - gesture.start_pos.0;
    let dy = gesture.current_pos.1 - gesture.start_pos.1;
    let adx = dx.abs();
    let ady = dy.abs();
    let at_ms = rec.recording_start.elapsed().as_millis() as u64;

    let Some(ref id) = gesture.start_element else {
        if adx < GESTURE_THRESHOLD && ady < GESTURE_THRESHOLD {
            eprintln!("Recording: click on empty area (no element ID)");
        }
        return;
    };

    let event = if adx < GESTURE_THRESHOLD && ady < GESTURE_THRESHOLD {
        eprintln!("Recording: click(#{id})");
        UnifiedEvent::Click {
            element: id.clone(),
        }
    } else if ady >= GESTURE_THRESHOLD && ady > adx {
        let delta = dy.round() as i32;
        eprintln!("Recording: scroll(#{id}, {delta})");
        UnifiedEvent::Scroll {
            element: id.clone(),
            delta,
        }
    } else if adx >= GESTURE_THRESHOLD && adx > ady {
        eprintln!(
            "Recording: drag(#{id}, {:.2}, {:.2})",
            gesture.start_pos.0, gesture.current_pos.0
        );
        UnifiedEvent::Drag {
            element: id.clone(),
            from: gesture.start_pos.0,
            to: gesture.current_pos.0,
        }
    } else {
        return;
    };

    rec.events.push(TimelineEvent { at_ms, event });

    if rec.auto_capture {
        let capture_at = at_ms + AUTO_CAPTURE_DELAY_MS;
        rec.events.push(TimelineEvent {
            at_ms: capture_at,
            event: UnifiedEvent::Capture {
                duration_ms: Some(2_000),
                fps: Some(4),
            },
        });
        eprintln!("Recording: auto-capture at {capture_at}ms");
    }
}

// ── Hot reload ──────────────────────────────────────────────────────

/// Watch the directory containing `path` for relevant changes; send `()` on its receiver
/// whenever the target file is created/modified/removed.
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

// ── Background ──────────────────────────────────────────────────────

/// Paint a two-tone checkerboard over `rect` — same pattern as `bmc-virt-console`'s
/// device backdrop so the tile boundaries read clearly against an otherwise blank window.
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

struct PreviewTile {
    runtime: WasmWidgetRuntime,
    gpu: TileGpu,
    x: u32,
    y: u32,
    label: &'static str,
    logged_dead: bool,
    ever_rendered: bool,
    /// Receiver for LED commands from the widget (drained each frame).
    led_rx: std::sync::mpsc::Receiver<bmc_led::data::LedCommand>,
    /// Current LED scene (from last `SetEffect` command).
    led_scene: Option<bmc_led::data::LedScene>,
    /// Whether LEDs are enabled.
    led_enabled: bool,
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

// ── LED strip rendering (egui painter, gradient approximation) ──────

/// Brightness ∈ [0, 1] for LED `phase` (0..1) at time `anim_t`.
/// Ported from the prior FemtoVG-based strip with identical semantics.
fn led_brightness(effect: bmc_led::data::LedEffect, phase: f32, anim_t: f32) -> f32 {
    use bmc_led::data::LedEffect;
    match &effect {
        LedEffect::Solid(_) => 1.0,
        LedEffect::Breathe(_) => {
            let pulse = f32::midpoint((anim_t * std::f32::consts::TAU).sin(), 1.0);
            0.3 + pulse * 0.7
        }
        LedEffect::Chase(_) => {
            let pos = anim_t.fract();
            let dist = (phase - pos)
                .abs()
                .min((phase - pos + 1.0).abs())
                .min((phase - pos - 1.0).abs());
            (1.0 - dist * LED_COUNT as f32 * 0.5).max(0.02)
        }
        LedEffect::KnightRider(_) | LedEffect::Scan(_) => {
            let pos = (anim_t.fract() * 2.0 - 1.0).abs();
            let dist = (phase - pos).abs();
            (1.0 - dist * LED_COUNT as f32 * 0.5).max(0.02)
        }
        LedEffect::Snake(_) => {
            let tail = (phase - anim_t.fract() + 1.0).fract();
            let tail_len = 0.3;
            if tail < tail_len {
                1.0 - tail / tail_len
            } else {
                0.02
            }
        }
        LedEffect::None => 0.0,
    }
}

/// Paint an LED diffuser strip below a tile.
///
/// Black background always; glow blobs only when the tile's `led_scene` is active.
/// Glow is approximated via 4 stacked alpha-decreasing circles per LED —
/// a cheap gaussian stand-in that reads as soft light without the FBO machinery
/// the prior FemtoVG-based strip needed.
fn paint_led_strip(
    painter: &egui::Painter,
    tile: &PreviewTile,
    tile_origin: egui::Pos2,
    time_s: f32,
) {
    let strip_w = tile.gpu.width as f32;
    let strip_h = LED_STRIP_H as f32;
    let strip_rect = egui::Rect::from_min_size(
        tile_origin + egui::vec2(tile.x as f32, tile.y as f32 + tile.gpu.height as f32),
        egui::vec2(strip_w, strip_h),
    );
    painter.rect_filled(strip_rect, 0.0, egui::Color32::BLACK);

    let Some(scene) = tile.led_scene.as_ref().filter(|_| tile.led_enabled) else {
        return;
    };
    let (cr, cg, cb) = match &scene.effect {
        bmc_led::data::LedEffect::None => return,
        bmc_led::data::LedEffect::Solid(c)
        | bmc_led::data::LedEffect::Breathe(c)
        | bmc_led::data::LedEffect::Chase(c)
        | bmc_led::data::LedEffect::KnightRider(c)
        | bmc_led::data::LedEffect::Scan(c)
        | bmc_led::data::LedEffect::Snake(c) => (c.r, c.g, c.b),
    };
    let period_s = scene.period.map_or(1.0, |d| d.as_secs_f32().max(0.1));
    let anim_t = time_s / period_s;

    // FULL tile: LEDs span centre half. Smaller tiles: full width.
    let is_full = tile.gpu.width >= 1280;
    let led_region_w = if is_full { strip_w * 0.5 } else { strip_w };
    let led_x_offset = (strip_w - led_region_w) / 2.0;
    let led_spacing = led_region_w / LED_COUNT as f32;

    for idx in 0..LED_COUNT {
        let phase = idx as f32 / LED_COUNT as f32;
        let brightness = led_brightness(scene.effect, phase, anim_t);
        if brightness <= 0.0 {
            continue;
        }
        let cx = strip_rect.min.x + led_x_offset + (idx as f32 + 0.5) * led_spacing;
        let cy = strip_rect.min.y;
        // Stacked falloff: 4 circles of increasing radius and decreasing alpha approximate
        // a radial gradient cheaply enough for the testbed UI.
        for ring in 0..4 {
            let t = ring as f32 / 3.0;
            let radius = led_spacing * (0.4 + t * 1.6);
            let alpha = (brightness * (1.0 - t) * (1.0 - t) * 0.8 * 255.0).clamp(0.0, 255.0) as u8;
            if alpha == 0 {
                continue;
            }
            let color = egui::Color32::from_rgba_unmultiplied(cr, cg, cb, alpha);
            painter.circle_filled(egui::pos2(cx, cy), radius, color);
        }
    }
}

// ── App ─────────────────────────────────────────────────────────────

struct TestbedApp {
    cli: CliArgs,
    params:
        std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
    gl: Arc<eframe::glow::Context>,
    tiles: Vec<PreviewTile>,
    clock: Clock,
    hot_reload: HotReload,
    perf: PerfState,
    recording_mode: RecordingMode,
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

/// Per-frame performance accounting. The rolling window drives the FPS readout in the stats
/// panel; the full vector is what `--perf-report=` writes to disk at exit.
struct PerfState {
    /// Total frames rendered so far. Used to drive the `--perf-frames` exit condition.
    frame_count: u32,
    /// Per-frame timings from FULL tile's runtime; written to disk by `--perf-report=` at exit.
    samples: Vec<bmc_render::FrameTimings>,
    /// Last frame's wall-clock duration (microseconds); recent samples averaged for FPS.
    recent_frame_us: std::collections::VecDeque<u32>,
}

/// Recording-mode bundle: the optional in-flight recording state plus the shared fetch
/// buffer the active tile's fetch observer pushes into.
struct RecordingMode {
    /// `Some` only when started via `--record=<size>`; `None` resets it after Save/Cancel.
    state: Option<RecordingState>,
    /// Shared buffer for fetch events captured by the active tile's fetch observer. Held
    /// behind `Arc<Mutex<_>>` because the observer runs on background fetch threads.
    fetch_events: std::sync::Arc<std::sync::Mutex<Vec<TimelineEvent>>>,
}

impl TestbedApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        cli: CliArgs,
        params: std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
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
        // Stash the loader so `init_tiles` can pull it back when the eframe::Frame is in
        // scope (the lazy init path). Cleared on Drop so a fresh app instance doesn't
        // inherit stale handles.
        GET_PROC_ADDRESS.with(|cell| *cell.borrow_mut() = Some(get_proc));

        let (watcher, watcher_rx) =
            setup_watcher(&cli.wasm_path).map_err(|e| format!("watcher: {e}"))?;

        let recording_state = cli.record_size.as_ref().map(|size_name| {
            let active_tile = record_size_to_idx(size_name);
            let widget_root = find_widget_root(&cli.wasm_path);
            let start_time_iso = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
            RecordingState {
                active_tile,
                size_name: size_name.clone(),
                events: Vec::new(),
                gesture: None,
                widget_root,
                recording_start: std::time::Instant::now(),
                kv_snapshot: std::collections::HashMap::new(),
                start_time_iso,
                auto_capture: true,
            }
        });

        let now = std::time::Instant::now();
        Ok(Self {
            cli,
            params,
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
                led_command_sender: Some(led_tx),
                ..RuntimeConfig::default()
            };
            // SAFETY: same context invariants as initial construction in `init_tiles`.
            match unsafe {
                WasmWidgetRuntime::new(
                    &wasm_bytes,
                    proc_loader(get_proc.clone()),
                    tile.gpu.width,
                    tile.gpu.height,
                    tile.gpu.fbo_id(),
                    rt_config,
                )
            } {
                Ok(rt) => {
                    tile.runtime = rt; // drops the old runtime + its renderer
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
            rt_config.led_command_sender = Some(led_tx);
            // SAFETY: `get_proc` returns valid GL function pointers for the current context;
            // eframe keeps that context current for the lifetime of the app.
            let runtime = unsafe {
                WasmWidgetRuntime::new(
                    &wasm_bytes,
                    proc_loader(get_proc.clone()),
                    w,
                    h,
                    gpu.fbo_id(),
                    rt_config,
                )
            }
            .with_context(|| format!("create runtime for {label}"))?;
            tiles.push(PreviewTile {
                runtime,
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
        // Snapshot the active recording tile's KV directory at start so the fixture's
        // `header.kv` reproduces the initial state on replay.
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
    /// Saves the GL framebuffer binding + viewport before mutating them per tile and restores
    /// both at the end so egui's own draw list runs against the screen framebuffer the way it
    /// expects. Skipping this caused screen-wide trails (egui's clear hit a tile FBO instead of
    /// the default framebuffer).
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
        for tile in &mut self.tiles {
            tile.drain_led_commands();
            tile.runtime.set_time(system_time, monotonic_ms);
            tile.runtime
                .renderer()
                .begin_frame(tile.gpu.width, tile.gpu.height, 1.0);
            tile.runtime.deliver_fetch_responses();
            tile.runtime.deliver_ws_messages();
            tile.runtime.deliver_socket_events();
            tile.runtime.deliver_mdns_events();
            tile.runtime.deliver_ssdp_events();
            tile.runtime.deliver_udp_broadcast_events();
            tile.runtime.deliver_http_requests();
            match tile.runtime.render(delta_ms) {
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
            tile.runtime.renderer().flush();
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
        // Right-aligning numbers inside their cells via RichText keeps the digit columns
        // visually anchored even as values change width.
        let mono = egui::FontId::monospace(11.0);
        let val_color = egui::Color32::from_gray(220);
        let lbl_color = egui::Color32::from_gray(160);
        let cell_lbl = |txt: &str| egui::RichText::new(txt).font(mono.clone()).color(lbl_color);
        let cell_val = |txt: String| egui::RichText::new(txt).font(mono.clone()).color(val_color);
        egui::Grid::new("testbed_stats_table")
            .num_columns(4)
            .spacing([12.0, 2.0])
            .min_col_width(0.0)
            .show(&mut child, |g| {
                g.label(cell_lbl("frame avg:"));
                g.label(cell_val(format!("{avg_us} µs")));
                g.label(cell_lbl(""));
                g.label(cell_val(format!("{fps:.1} fps")));
                g.end_row();
                if let Some(t) = self.tiles.first().map(|t| t.runtime.last_timings()) {
                    g.label(cell_lbl("FULL wasm:"));
                    g.label(cell_val(format!("{} µs", t.wasm_us)));
                    g.label(cell_lbl("deser:"));
                    g.label(cell_val(format!("{} µs", t.deserialize_us)));
                    g.end_row();
                    g.label(cell_lbl("layout:"));
                    g.label(cell_val(format!("{} µs", t.layout_us)));
                    g.label(cell_lbl("render:"));
                    g.label(cell_val(format!("{} µs", t.render_us)));
                    g.end_row();
                    g.label(cell_lbl("flush:"));
                    g.label(cell_val(format!("{} µs", t.flush_us)));
                    g.label(cell_lbl(""));
                    g.label(cell_lbl(""));
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
            // Chart on top, legend strip below — keeps the chart visually grouped with the
            // numeric stats above and the legend functions as a key reading downward.
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

// ── Timing chart ────────────────────────────────────────────────────

/// Paint a stacked bar chart of the most recent frame timings into `rect`.
/// One column per sample, stacked top-to-bottom: wasm / deserialize / layout / render / flush.
/// A horizontal reference line marks the 16.6 ms / 60 fps budget.
fn paint_timing_chart(
    painter: &egui::Painter,
    rect: egui::Rect,
    samples: &[bmc_render::FrameTimings],
) {
    // Fixed column width — bars stay the same size and newest samples append at the right edge,
    // older samples scroll off the left. Avoids the "bars resize as the window fills" effect.
    const COL_W: f32 = 2.0;
    let max_cols = (rect.width() / COL_W).floor().max(1.0) as usize;
    let n = samples.len().min(max_cols);
    if n == 0 {
        return;
    }
    let start = samples.len() - n;
    let view = &samples[start..];

    // Peak total across the window establishes the y scale.
    let peak_us = view
        .iter()
        .map(|s| {
            u64::from(s.wasm_us)
                + u64::from(s.deserialize_us)
                + u64::from(s.layout_us)
                + u64::from(s.render_us)
                + u64::from(s.flush_us)
        })
        .max()
        .unwrap_or(1)
        .max(1);
    // y-scale floor is 36,000 µs — slightly above the 30 fps budget (33,333 µs) so the
    // 30 fps reference line sits a bit below the top of the chart and its label has room
    // instead of being clipped at the edge. A genuine spike past 36 ms grows the scale.
    let y_scale_us = peak_us.max(36_000) as f32;
    let col_w = COL_W;

    // Subtle horizontal grid every 5 ms — drawn first so bars overlay on top.
    let grid_color = egui::Color32::from_rgba_unmultiplied(140, 140, 140, 30);
    let grid_step_us = 5_000.0_f32;
    let mut grid_us = grid_step_us;
    while grid_us < y_scale_us {
        let y = rect.max.y - (grid_us / y_scale_us) * rect.height();
        if y > rect.min.y && y < rect.max.y {
            painter.line_segment(
                [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
                egui::Stroke::new(1.0, grid_color),
            );
        }
        grid_us += grid_step_us;
    }
    // Component colours mirror PerfOverlay's legend ordering.
    let colours = [
        (egui::Color32::from_rgb(0x6A, 0x9F, 0xD8), "wasm"),
        (egui::Color32::from_rgb(0xE0, 0x9A, 0x50), "deser"),
        (egui::Color32::from_rgb(0xCC, 0xCC, 0x50), "layout"),
        (egui::Color32::from_rgb(0x50, 0xCC, 0x50), "render"),
        (egui::Color32::from_rgb(0xCC, 0x50, 0xCC), "flush"),
    ];

    // Right-anchored: oldest sample drawn at the leftmost slot used, newest at the right edge.
    let bars_left = rect.max.x - n as f32 * col_w;
    for (i, sample) in view.iter().enumerate() {
        let x = bars_left + i as f32 * col_w;
        let mut y = rect.max.y;
        let parts = [
            sample.wasm_us,
            sample.deserialize_us,
            sample.layout_us,
            sample.render_us,
            sample.flush_us,
        ];
        for (part_us, (color, _)) in parts.into_iter().zip(colours) {
            let h = (part_us as f32 / y_scale_us) * rect.height();
            if h < 0.5 {
                continue;
            }
            let bar =
                egui::Rect::from_min_max(egui::pos2(x, y - h), egui::pos2(x + col_w.max(1.0), y));
            painter.rect_filled(bar, 0.0, color);
            y -= h;
        }
    }

    // Reference lines at 60 fps (16.6 ms) and 30 fps (33.3 ms) — the two budgets the
    // testbed cares about. Each labeled at the right edge.
    for (us, label) in [(16_666.0_f32, "60 fps"), (33_333.0_f32, "30 fps")] {
        let y = rect.max.y - (us / y_scale_us) * rect.height();
        if y < rect.min.y || y > rect.max.y {
            continue;
        }
        painter.line_segment(
            [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(180, 180, 180, 140),
            ),
        );
        // Label sits BELOW the line so it doesn't get clipped against `rect.min.y`
        // when the line itself is near the top of the chart (e.g. 30 fps marker at peak scale).
        painter.text(
            egui::pos2(rect.max.x - 2.0, y + 1.0),
            egui::Align2::RIGHT_TOP,
            label,
            egui::FontId::monospace(9.0),
            egui::Color32::from_gray(200),
        );
    }
}

/// Paint the chart's component legend in its own strip — the colour swatches and labels
/// for wasm / deser / layout / render / flush, in stack order. Lives above the chart so it
/// doesn't overlap the bars.
fn paint_timing_legend(painter: &egui::Painter, rect: egui::Rect) {
    let colours = [
        (egui::Color32::from_rgb(0x6A, 0x9F, 0xD8), "wasm"),
        (egui::Color32::from_rgb(0xE0, 0x9A, 0x50), "deser"),
        (egui::Color32::from_rgb(0xCC, 0xCC, 0x50), "layout"),
        (egui::Color32::from_rgb(0x50, 0xCC, 0x50), "render"),
        (egui::Color32::from_rgb(0xCC, 0x50, 0xCC), "flush"),
    ];
    let mut x_cursor = rect.min.x;
    let cy = rect.center().y;
    for (color, label) in colours {
        let sw = egui::Rect::from_center_size(egui::pos2(x_cursor + 4.0, cy), egui::vec2(8.0, 8.0));
        painter.rect_filled(sw, 0.0, color);
        painter.text(
            egui::pos2(x_cursor + 10.0, cy),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(9.0),
            egui::Color32::from_gray(180),
        );
        x_cursor += 56.0;
    }
}

// ── Recording panel / finish ────────────────────────────────────────

/// Action dispatched by the recording panel each frame.
/// `None` between frames; one of the variants on the frame the operator clicks.
#[derive(Clone, Copy, Debug)]
enum RecordingAction {
    Save,
    Cancel,
    Capture,
}

impl TestbedApp {
    /// Paint the recording panel in `rect` — title, scrollable event log, Save/Cancel/Capture
    /// buttons, and the auto-capture toggle. Returns the operator action for this frame.
    fn paint_recording_panel(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
    ) -> Option<RecordingAction> {
        let rec = self.recording_mode.state.as_mut()?;
        ui.painter()
            .rect_filled(rect, 4.0, egui::Color32::from_gray(18));
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(180, 80, 20)),
            egui::StrokeKind::Inside,
        );

        let pad = 8.0;
        let inner = rect.shrink(pad);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let mut action: Option<RecordingAction> = None;

        child.label(
            egui::RichText::new(format!("RECORDING — {}", rec.size_name))
                .color(egui::Color32::from_rgb(255, 170, 80))
                .strong(),
        );
        child.separator();

        // Bottom button row pinned to inner.max.y; reserve the space first so the scroll
        // area knows how tall it can be.
        const BUTTON_ROW_H: f32 = 28.0;
        let log_max_y = inner.max.y - BUTTON_ROW_H - 6.0;
        let log_rect = egui::Rect::from_min_max(
            egui::pos2(inner.min.x, child.cursor().min.y),
            egui::pos2(inner.max.x, log_max_y),
        );
        if log_rect.height() > 16.0 {
            let mut log_child = child.new_child(egui::UiBuilder::new().max_rect(log_rect));
            let mono = egui::FontId::monospace(10.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .max_height(log_rect.height())
                .show(&mut log_child, |scroll| {
                    if rec.events.is_empty() {
                        scroll.label(
                            egui::RichText::new("(no events yet — click / drag a tile)")
                                .color(egui::Color32::from_gray(120))
                                .font(mono.clone()),
                        );
                    } else {
                        for ev in &rec.events {
                            let secs = ev.at_ms as f32 / 1000.0;
                            let line = format!("{secs:>6.2}s  {}", format_event_label(&ev.event));
                            scroll.label(egui::RichText::new(line).font(mono.clone()));
                        }
                    }
                });
        }

        // Button row at the bottom edge.
        let row_rect = egui::Rect::from_min_max(
            egui::pos2(inner.min.x, inner.max.y - BUTTON_ROW_H),
            inner.max,
        );
        let mut row_child = child.new_child(egui::UiBuilder::new().max_rect(row_rect));
        row_child.horizontal(|row| {
            if row.button("Save").clicked() {
                action = Some(RecordingAction::Save);
            }
            if row.button("Cancel").clicked() {
                action = Some(RecordingAction::Cancel);
            }
            if row.button("Capture").clicked() {
                action = Some(RecordingAction::Capture);
            }
            row.checkbox(&mut rec.auto_capture, "auto");
        });

        action
    }

    /// Append a manual single-frame `Capture` event to the recording timeline.
    fn push_manual_capture(&mut self) {
        if let Some(rec) = self.recording_mode.state.as_mut() {
            let at_ms = rec.recording_start.elapsed().as_millis() as u64;
            rec.events.push(TimelineEvent {
                at_ms,
                event: UnifiedEvent::Capture {
                    duration_ms: None,
                    fps: None,
                },
            });
        }
    }

    /// Take ownership of the active recording, merge all event sources (user actions, network
    /// events from the runtime, fetch events from the shared buffer), validate, and write a
    /// `.jsonl.gz` fixture into the widget's `capture/fixtures/<size>.jsonl.gz`.
    /// Also updates the widget's `capture/config.toml` to point at the new fixture.
    fn finish_recording(&mut self) {
        let Some(rec) = self.recording_mode.state.take() else {
            return;
        };

        // Pull network events out of the active tile's runtime, plus the fetch events the
        // observer pushed into the shared buffer.
        let runtime_events = if let Some(tile) = self.tiles.get_mut(rec.active_tile) {
            tile.runtime.take_recorded_events()
        } else {
            Vec::new()
        };
        let network_timeline = fixtures::fixture_events_to_timeline(&runtime_events);
        let fetch_timeline: Vec<TimelineEvent> = std::mem::take(
            &mut *self
                .recording_mode
                .fetch_events
                .lock()
                .expect("BUG: fetch events poisoned"),
        );

        // Merge: user actions + network + fetch, sorted by at_ms (stable so insertion order
        // breaks ties), then collapse consecutive scrolls on the same element into one event.
        let mut all_events = rec.events;
        all_events.extend(network_timeline);
        all_events.extend(fetch_timeline);
        all_events.sort_by_key(|e| e.at_ms);
        let mut merged: Vec<TimelineEvent> = Vec::with_capacity(all_events.len());
        for event in all_events {
            let should_merge = if let UnifiedEvent::Scroll { ref element, .. } = event.event {
                merged.last().is_some_and(|prev: &TimelineEvent| {
                    matches!(&prev.event, UnifiedEvent::Scroll { element: prev_el, .. } if prev_el == element)
                })
            } else {
                false
            };
            if should_merge {
                if let UnifiedEvent::Scroll { delta, .. } = event.event
                    && let Some(prev) = merged.last_mut()
                    && let UnifiedEvent::Scroll {
                        delta: ref mut prev_delta,
                        ..
                    } = prev.event
                {
                    *prev_delta += delta;
                }
            } else {
                merged.push(event);
            }
        }

        let fixture = UnifiedFixture {
            header: FixtureHeader {
                time: rec.start_time_iso,
                kv: rec.kv_snapshot,
            },
            events: merged,
        };

        let Some(widget_root) = rec.widget_root else {
            eprintln!(
                "error: could not find widget root — fixture not saved ({} event(s))",
                fixture.events.len()
            );
            return;
        };
        let fixture_dir = widget_root.join("capture").join("fixtures");
        let fixture_path = fixture_dir.join(format!("{}.jsonl.gz", rec.size_name));

        if let Err(e) = bmc_wasm_runtime::unified_fixture::validate_fixture(&fixture) {
            eprintln!("warning: fixture validation failed: {e:#} (writing anyway)");
        }
        if let Err(e) = fixtures::write_jsonl_fixture(&fixture_path, &fixture) {
            eprintln!("error: failed to write fixture: {e:#}");
            return;
        }
        eprintln!(
            "wrote: {} event(s) → {}",
            fixture.events.len(),
            fixture_path.display()
        );

        let config_path = widget_root.join("capture").join("config.toml");
        let fixture_rel = format!("fixtures/{}.jsonl.gz", rec.size_name);
        if let Err(e) =
            fixtures::update_config_toml_fixtures(&config_path, &rec.size_name, &fixture_rel)
        {
            eprintln!("warning: failed to update config.toml: {e:#}");
        } else {
            eprintln!("updated: {}", config_path.display());
        }

        let widget_name = widget_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("WIDGET");
        eprintln!("hint: run `just wasm::update-baselines {widget_name}` to set baselines");
    }
}

// ── Perf report ─────────────────────────────────────────────────────

fn write_perf_report(path: &Path, samples: &[bmc_render::FrameTimings]) {
    let n = samples.len();
    if n == 0 {
        return;
    }
    let sum_wasm: u64 = samples.iter().map(|s| u64::from(s.wasm_us)).sum();
    let sum_deser: u64 = samples.iter().map(|s| u64::from(s.deserialize_us)).sum();
    let sum_layout: u64 = samples.iter().map(|s| u64::from(s.layout_us)).sum();
    let sum_render: u64 = samples.iter().map(|s| u64::from(s.render_us)).sum();
    let sum_flush: u64 = samples.iter().map(|s| u64::from(s.flush_us)).sum();
    let n_u64 = n as u64;
    let avg = |s: u64| s / n_u64;
    let report = serde_json::json!({
        "frames": n,
        "avg_us": {
            "wasm": avg(sum_wasm),
            "deserialize": avg(sum_deser),
            "layout": avg(sum_layout),
            "render": avg(sum_render),
            "flush": avg(sum_flush),
        }
    });
    match std::fs::write(
        path,
        serde_json::to_string_pretty(&report).unwrap_or_default(),
    ) {
        Ok(()) => println!("Perf report written to {}", path.display()),
        Err(e) => tracing::warn!("perf report: {e}"),
    }
}

// Process-wide cell holding the eframe-provided GL proc address loader, populated in
// `TestbedApp::new` and read by `init_tiles` / `poll_hot_reload`. A thread-local sidesteps the
// `dyn Fn` capture lifetime question while keeping the closure trivially cloneable.
// `thread_local!` is a macro, so this stays a regular `//` comment — doc comments don't
// attach to macro invocations.
thread_local! {
    static GET_PROC_ADDRESS: std::cell::RefCell<Option<GlProcAddress>>
        = const { std::cell::RefCell::new(None) };
}

impl eframe::App for TestbedApp {
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
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
                    // FemtoVG renders bottom-up into the FBO; flip V to display top-down.
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0));
                    ui.painter()
                        .image(tile.gpu.egui_tex_id, rect, uv, egui::Color32::WHITE);

                    // Touch / mouse routing: allocate the same rect for click+drag so we
                    // can forward pointer events to the runtime in tile-local coordinates.
                    // Recording state is threaded in only for the active recording tile so
                    // gestures on other tiles don't pollute the fixture timeline.
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
