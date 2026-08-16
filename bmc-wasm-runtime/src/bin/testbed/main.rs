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

//! Widget development testbed with hot-reloading.
//!
//! Owns its window, GL context and event loop directly, on winit and glutin.
//! Each view registers a native texture and has to replace and release it
//! as views open, close and resize, which is what eframe would not expose.
//!
//! Renders every viewport of the active platform, plus stats / LED-strip /
//! recording UI overlays.
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
    reason = "UI math on small bounded positive values — intentional in this testbed binary"
)]

mod canvas;
mod credentials_ui;
mod device_window;
mod icon;
mod paint;
mod params_ui;
mod recording;
mod status_bar;
mod system_ui;
mod theme;
mod toolbar;
mod ui_helpers;
mod view;
mod window;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use egui_glow::glow::HasContext as _;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use bmc_render::interaction::TouchEvent;
use bmc_wasm_runtime::fixtures::{
    self, PreparedWidget, find_widget_root, seed_kv_from_widget_root, snapshot_kv_dir,
};
use bmc_wasm_runtime::platform_catalog::{self, DisplayShape, Platform, Viewport};
use bmc_wasm_runtime::{DiskCache, PackageAssetStore, RuntimeConfig, SystemSnapshot};
use clap::Parser;

use paint::{
    GlProcAddress, draw_checkerboard, paint_timing_chart, paint_timing_legend, write_perf_report,
};
use recording::{RecordUnwind, RecordingMode, RecordingState};
use view::{DeviceView, ViewCommand};

/// Height of the LED diffuser strip rendered below a device frame.
pub(crate) const LED_STRIP_H: u32 = 24;

/// Startup window size. The canvas pans, so nothing has to fit;
/// this just opens with the BMC100 frame and the sidebar in view.
const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1680.0, 1000.0);

/// One previewed viewport, in logical pixels.
/// Where it paints is the device frame's business.
pub(crate) struct PlacedTile {
    pub(crate) label: String,
    pub(crate) kv_key: String,
    pub(crate) shape: DisplayShape,
    pub(crate) led_count: Option<usize>,
    pub(crate) w: u32,
    pub(crate) h: u32,
}

impl PlacedTile {
    fn for_viewport(platform: &Platform, viewport: &Viewport) -> Self {
        Self {
            label: viewport.label.to_owned(),
            kv_key: viewport.id.to_owned(),
            shape: viewport.shape,
            led_count: platform.led_count(),
            w: viewport.width,
            h: viewport.height,
        }
    }
}

#[derive(Clone, Copy)]
struct RuntimeTileGeometry {
    viewport_shape: bmc_wasm_protocol::ViewportShape,
    display: bmc_wasm_runtime::RuntimeDisplayInfo,
}

impl RuntimeTileGeometry {
    fn for_viewport_shape(platform: &Platform, viewport_shape: DisplayShape) -> Self {
        Self {
            viewport_shape: platform_catalog::runtime_viewport_shape(viewport_shape),
            display: platform.runtime_display_info(),
        }
    }
}

/// What `--record` asks for: the viewport to pin and the dataset to write.
#[derive(Debug, Clone)]
struct RecordRequest {
    target: platform_catalog::Target,
    dataset: String,
}

/// Resolve `--record`, if given.
///
/// A recording pins one viewport, so the target decides which platform
/// the testbed opens; an explicit `--platform` naming a different one
/// is a contradiction rather than a silent override.
fn resolve_record_request(cli: &CliArgs) -> Result<Option<RecordRequest>> {
    let Some(spec) = cli.record_target.as_deref() else {
        return Ok(None);
    };
    let target: platform_catalog::Target = spec.parse()?;
    if let Some(requested) = cli.platform_id.as_deref()
        && !requested.eq_ignore_ascii_case(target.platform.id)
    {
        anyhow::bail!(
            "--record={spec} records on platform '{}', but --platform={requested} was given",
            target.platform.id,
        );
    }

    let dataset = cli
        .record_name
        .clone()
        .unwrap_or_else(|| default_dataset(target));
    if !bmc_wasm_runtime::capture_config::is_valid_dataset_name(&dataset) {
        anyhow::bail!(
            "--record-name={dataset} must be non-empty and use only letters, digits, '-', '_' or '.'"
        );
    }

    Ok(Some(RecordRequest { target, dataset }))
}

/// The dataset a take writes when the operator names none — the target,
/// which identifies a recording unless a sim scenario distinguishes takes.
fn default_dataset(target: platform_catalog::Target) -> String {
    format!("{}-{}", target.platform.id, target.viewport.id)
}

/// Width of the right-side sidebar housing both the per-widget Params
/// section (when the manifest declares any) and the deck-wide System
/// section (always shown). Added to the window's outer size so the tile
/// area stays at native dimensions instead of getting squeezed.
pub(crate) const PARAM_PANEL_W: u32 = 320;

/// Width of the notice banner: wide enough for a fixture path to fit
/// on one monospaced line.
const NOTICE_W: f32 = 520.0;

// ── CLI ─────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "testbed", about = "WASM widget testbed")]
struct CliArgs {
    /// The widget `.wasm` to load.
    wasm_path: PathBuf,
    /// Package asset root paired with an already stripped widget.
    #[arg(long)]
    asset_root: Option<PathBuf>,
    /// Widget manifest, when it isn't found next to the wasm.
    #[arg(long = "manifest")]
    manifest_path: Option<PathBuf>,
    /// Widget source root, for KV seeding and fixtures.
    #[arg(long)]
    widget_root: Option<PathBuf>,
    /// Write a perf report here after `--perf-frames` frames.
    #[arg(long = "perf-report")]
    perf_report_path: Option<PathBuf>,
    /// Frames to run before the perf report is written.
    #[arg(long, default_value_t = 600)]
    perf_frames: u32,
    /// Record a capture fixture for this target (e.g. `bmc100:small`).
    #[arg(long = "record")]
    record_target: Option<String>,
    /// Dataset name for the recording; defaults to `<platform>-<viewport>`.
    #[arg(long = "record-name", requires = "record_target")]
    record_name: Option<String>,
    /// Platform id to select from the catalog.
    #[arg(long = "platform")]
    platform_id: Option<String>,
    /// Directory backing the widget blob cache, so history survives a rebuild;
    /// the device wires its own, this brings the same to live `wasm::dev`.
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Credential secrets as JSON (`{"<slot>": {"<field>": "…"}}`), so a bound slot
    /// can reach its live API — recording a fetch-backed widget needs one real egress pass.
    /// A slot the widget does not declare is rejected at startup.
    /// Keep the file gitignored (`secrets.local.json`); substitution happens
    /// at the wire hop only, so no fixture, log, or diagnostic ever holds a secret.
    #[arg(long)]
    secrets: Option<PathBuf>,
    /// Rewrite a fetch's origin at the last hop, as `FROM=TO` (repeatable) —
    /// points a widget's hard-coded API base at a simulator:
    /// `--rewrite-url https://api.braiins.com=http://127.0.0.1:20000`.
    /// Both sides name an origin (scheme, host, port), matched whole.
    /// The egress check judges the rewritten destination, so the slot's
    /// secrets entry needs a matching `allow_hosts` pin.
    #[arg(long = "rewrite-url")]
    rewrite_url: Vec<String>,
    /// Report whether this machine can drive views on their own threads, then exit.
    /// Answers the two questions that decide it — a context in the window's share group,
    /// and a pbuffer to make it current on — plus the frame-handoff strategy the GL version allows.
    #[arg(long = "check-shared-gl")]
    check_shared_gl: bool,
    /// Render every view on the UI thread instead of giving each its own.
    ///
    /// Views are threaded by default, falling back on their own when a shared
    /// context cannot be made. This forces the fallback for the whole run —
    /// worth reaching for when a driver misbehaves, or to compare against.
    /// `--check-shared-gl` reports what a machine can do.
    #[arg(long = "inline-views")]
    inline_views: bool,
    /// How a threaded view hands a finished frame to the compositor:
    /// `fence` orders the two GPU-side, `finish` drains the view's queue before
    /// it reports. Defaults to what the GL version supports.
    #[arg(long = "view-handoff")]
    view_handoff: Option<view::GpuWait>,
}

/// Where this run's views render.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ViewPlacement {
    OwnThread,
    UiThread,
}

/// Decide where views render, and say so when a mode takes the choice away.
///
/// Profiling pins every view to the UI thread: the profiler reads fuel
/// sections from the render it just drove, an answer that cannot arrive
/// a frame later. Recording pins the same way but per build,
/// while the mode is active ([`placement_for_build`]) —
/// it comes and goes at runtime, which a launch-time decision would outlive.
fn view_placement(cli: &CliArgs) -> ViewPlacement {
    if cli.inline_views {
        return ViewPlacement::UiThread;
    }
    if cli.perf_report_path.is_some() {
        tracing::info!(
            "profiling: views render on the UI thread, where their queries can be answered"
        );
        return ViewPlacement::UiThread;
    }
    ViewPlacement::OwnThread
}

/// Where a view built right now renders: recording pins it inline,
/// since the recorder's hit-test and event drain are synchronous queries
/// only an inline view answers. Recording also pins the canvas to one
/// platform, so every view that exists during a take is pinned by this —
/// `--record`'s old whole-run pin, scoped to the mode instead.
fn placement_for_build(configured: ViewPlacement, recording: bool) -> ViewPlacement {
    if recording {
        ViewPlacement::UiThread
    } else {
        configured
    }
}

/// The manifest's credential slots, each with the field names its type
/// defines — what a hand-written secrets file is judged against.
fn declared_slots(
    manifest: &bmc_widget_manifest::Manifest,
) -> Vec<bmc_widget_protocol::DeclaredSlot> {
    manifest
        .credentials
        .iter()
        .map(|(key, slot)| {
            let builtin = bmc_widget_manifest::credential::BuiltinType::from_id(&slot.type_id);
            if builtin.is_none() {
                // Its secrets are then refused for naming fields nothing
                // defines, so say which slot is unbindable and why.
                tracing::warn!(
                    slot = key.as_str(),
                    type_id = slot.type_id,
                    "credential type unknown to this firmware; the slot defines no fields"
                );
            }
            bmc_widget_protocol::DeclaredSlot {
                name: key.as_str().to_owned(),
                fields: builtin
                    .map(|builtin| {
                        builtin
                            .schema()
                            .fields
                            .keys()
                            .map(|field| field.as_str().to_owned())
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// One side of a `--rewrite-url` pair as its origin.
///
/// The rewrite matches whole origins, so anything it could not honour
/// is refused here rather than silently never matching:
/// an unparsable URL, a scheme carrying no host to compare,
/// or a path, query or fragment.
fn origin_of(side: &str) -> Result<String> {
    let url = url::Url::parse(side)
        .with_context(|| format!("--rewrite-url {side}: not a parsable URL"))?;
    let origin = url.origin();
    anyhow::ensure!(
        origin.is_tuple(),
        "--rewrite-url {side}: needs a scheme with a host"
    );
    anyhow::ensure!(
        url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
        "--rewrite-url {side}: name an origin only, with no path, query or fragment"
    );
    Ok(origin.ascii_serialization())
}

/// Per-tag bucket cap for the dev blob cache, matching the device's flash cap.
const DEV_CACHE_MAX_BYTES: u64 = 16 * 1_024 * 1_024;

/// A deterministic stand-in for the Deck's network info, so widgets that
/// render bind hints or QR codes show them in the testbed and recordings
/// stay reproducible. Deliberately fake — there is no web app to reach.
fn stub_network() -> bmc_wasm_runtime::NetworkInfo {
    bmc_wasm_runtime::NetworkInfo {
        ssid: "Braiins-Guest".to_owned(),
        ip: "192.168.1.42".to_owned(),
    }
}

impl CliArgs {
    fn resolved_widget_root(&self) -> Option<PathBuf> {
        self.widget_root
            .clone()
            .or_else(|| find_widget_root(&self.wasm_path))
    }

    /// The secrets behind `--secrets`, empty without the flag.
    ///
    /// Reading and parsing are this side's errors; the map's shape is
    /// [`bmc_widget_protocol::CredentialSecrets::from_editable`]'s to judge
    /// against the manifest's slots.
    fn credential_secrets(
        &self,
        declared: &[bmc_widget_protocol::DeclaredSlot],
    ) -> Result<bmc_widget_protocol::CredentialSecrets> {
        let Some(path) = &self.secrets else {
            return Ok(bmc_widget_protocol::CredentialSecrets::default());
        };
        // `just` module recipes run from `bmc-wasm-runtime/`, a directory
        // above where developers keep the file — name the cwd so a wrong
        // relative path diagnoses itself.
        let raw = std::fs::read_to_string(path).with_context(|| {
            let cwd = std::env::current_dir()
                .map_or_else(|_| "?".to_owned(), |dir| dir.display().to_string());
            format!("read secrets file {} (cwd {cwd})", path.display())
        })?;
        let parsed: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&raw)
            .with_context(|| format!("parse secrets file {}", path.display()))?;
        bmc_widget_protocol::CredentialSecrets::from_editable(parsed, declared)
            .with_context(|| format!("secrets file {}", path.display()))
    }

    /// The `--rewrite-url` pairs, split at the first `=`.
    fn url_rewrites(&self) -> Result<Vec<(String, String)>> {
        self.rewrite_url
            .iter()
            .map(|pair| {
                let (from, to) = pair
                    .split_once('=')
                    .with_context(|| format!("--rewrite-url {pair}: expected FROM=TO"))?;
                Ok((origin_of(from)?, origin_of(to)?))
            })
            .collect()
    }

    /// The disk-backed blob cache for `--cache-dir`, creating the directory
    /// first; `None` when unset (the default) or uncreatable, so a hot loop
    /// stays hermetic unless a cache is asked for.
    fn asset_cache(&self) -> Option<DiskCache> {
        let dir = self.cache_dir.clone()?;
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(dir = %dir.display(), %e, "cache dir create failed; blob cache off");
            return None;
        }
        Some(DiskCache::new(dir, DEV_CACHE_MAX_BYTES))
    }
}

#[cfg(test)]
mod platforms_startup_tests {
    use super::*;

    fn parse_test_args(args: &[&str]) -> Result<CliArgs> {
        CliArgs::try_parse_from(args.iter().copied()).map_err(anyhow::Error::from)
    }

    #[test]
    fn views_get_their_own_threads_by_default() {
        let cli = parse_test_args(&["testbed", "widget.wasm"]).expect("BUG: bare args must parse");

        assert_eq!(view_placement(&cli), ViewPlacement::OwnThread);
    }

    #[test]
    fn inline_views_forces_every_view_onto_the_ui_thread() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--inline-views"])
            .expect("BUG: inline-views must parse");

        assert_eq!(view_placement(&cli), ViewPlacement::UiThread);
    }

    #[test]
    fn recording_pins_views_inline() {
        assert_eq!(
            placement_for_build(ViewPlacement::OwnThread, true),
            ViewPlacement::UiThread,
            "the recorder hit-tests inside the gesture it is classifying, \
             which a threaded view cannot answer"
        );
    }

    #[test]
    fn placement_is_the_configured_one_outside_recording() {
        assert_eq!(
            placement_for_build(ViewPlacement::OwnThread, false),
            ViewPlacement::OwnThread,
        );
        assert_eq!(
            placement_for_build(ViewPlacement::UiThread, false),
            ViewPlacement::UiThread,
            "recording must not un-pin what the CLI pinned",
        );
    }

    #[test]
    fn the_default_dataset_matches_the_cli_default() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--record=bmc100:small"])
            .expect("BUG: record args must parse");
        let request = resolve_record_request(&cli)
            .expect("BUG: record request must resolve")
            .expect("BUG: a record target was given");

        assert_eq!(request.dataset, default_dataset(request.target));
        assert_eq!(request.dataset, "bmc100-small");
    }

    #[test]
    fn profiling_pins_views_inline() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--perf-report=out.json"])
            .expect("BUG: perf-report args must parse");

        assert_eq!(
            view_placement(&cli),
            ViewPlacement::UiThread,
            "the profiler reads fuel sections from the render it just drove"
        );
    }

    #[test]
    fn parse_args_accepts_platform_equals_forms() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--platform=BMM100"])
            .expect("BUG: platform equals args must parse");

        assert_eq!(cli.wasm_path, PathBuf::from("widget.wasm"));
        assert_eq!(cli.platform_id.as_deref(), Some("BMM100"));
    }

    #[test]
    fn parse_args_accepts_platform_space_forms() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--platform", "BMM101"])
            .expect("BUG: platform space args must parse");

        assert_eq!(cli.platform_id.as_deref(), Some("BMM101"));
    }

    #[test]
    fn parse_args_accepts_cache_dir_forms() {
        let equals = parse_test_args(&["testbed", "widget.wasm", "--cache-dir=/tmp/hist"])
            .expect("BUG: --cache-dir= must parse");
        assert_eq!(equals.cache_dir, Some(PathBuf::from("/tmp/hist")));

        let split = parse_test_args(&["testbed", "widget.wasm", "--cache-dir", "/tmp/hist"])
            .expect("BUG: --cache-dir <path> must parse");
        assert_eq!(split.cache_dir, Some(PathBuf::from("/tmp/hist")));

        let absent = parse_test_args(&["testbed", "widget.wasm"]).expect("BUG: bare args parse");
        assert_eq!(absent.cache_dir, None, "no cache without the flag");
    }

    #[test]
    fn parse_args_accepts_rewrite_url_forms() {
        let cli = parse_test_args(&[
            "testbed",
            "widget.wasm",
            "--rewrite-url=https://api.example.com=http://127.0.0.1:20000",
            "--rewrite-url",
            "https://api.other.example=http://127.0.0.1:20001",
        ])
        .expect("BUG: rewrite-url args must parse");
        assert_eq!(
            cli.url_rewrites().expect("BUG: well-formed pairs split"),
            vec![
                (
                    "https://api.example.com".to_owned(),
                    "http://127.0.0.1:20000".to_owned()
                ),
                (
                    "https://api.other.example".to_owned(),
                    "http://127.0.0.1:20001".to_owned()
                ),
            ]
        );

        for rejected in [
            "https://api.example.com/pool/v2=http://127.0.0.1:20000",
            "https://api.example.com=http://127.0.0.1:20000/base",
            "api.example.com=http://127.0.0.1:20000",
        ] {
            let cli = parse_test_args(&["testbed", "widget.wasm", "--rewrite-url", rejected])
                .expect("BUG: the flag itself parses");
            assert!(
                cli.url_rewrites().is_err(),
                "{rejected}: a pair the rewrite cannot honour must fail at startup"
            );
        }

        let absent = parse_test_args(&["testbed", "widget.wasm"]).expect("BUG: bare args parse");
        assert_eq!(
            absent.url_rewrites().expect("BUG: no flag, no pairs"),
            Vec::new(),
            "no rewrites without the flag"
        );

        let bad = parse_test_args(&["testbed", "widget.wasm", "--rewrite-url", "no-separator"])
            .expect("BUG: clap accepts the raw string");
        assert!(
            bad.url_rewrites().is_err(),
            "a pair without '=' is rejected at resolution"
        );
    }

    #[test]
    fn parse_args_accepts_legacy_split_forms() {
        let cli = parse_test_args(&[
            "testbed",
            "widget.wasm",
            "--manifest",
            "manifest.json",
            "--perf-report",
            "perf.json",
            "--perf-frames",
            "42",
            "--record",
            "bmc100:small",
        ])
        .expect("BUG: legacy split args must parse");

        assert_eq!(cli.manifest_path, Some(PathBuf::from("manifest.json")));
        assert_eq!(cli.perf_report_path, Some(PathBuf::from("perf.json")));
        assert_eq!(cli.perf_frames, 42);
        assert_eq!(cli.record_target.as_deref(), Some("bmc100:small"));
    }

    #[test]
    fn parse_args_accepts_widget_root_forms() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--widget-root", "widgets/clock"])
            .expect("BUG: widget-root split arg must parse");
        assert_eq!(cli.widget_root, Some(PathBuf::from("widgets/clock")));

        let cli = parse_test_args(&[
            "testbed",
            "widget.wasm",
            "--widget-root=widgets/blockheight",
        ])
        .expect("BUG: widget-root equals arg must parse");
        assert_eq!(cli.widget_root, Some(PathBuf::from("widgets/blockheight")));
    }

    #[test]
    fn load_manifest_uses_widget_root_for_foreign_target_wasm() {
        let temp = tempfile::tempdir().expect("BUG: create manifest fixture directory");
        let widget_root = temp.path().join("hello-widget");
        std::fs::create_dir(&widget_root).expect("BUG: create widget fixture directory");
        std::fs::write(
            widget_root.join("manifest.json"),
            r#"{
                "uid": "550e8400-e29b-41d4-a716-446655440200",
                "version": "0.1.0",
                "name": "Hello",
                "description": "Test fixture",
                "binary": "bin/hello-widget",
                "supported_viewports": [{
                    "type": "rectangular",
                    "min_width": 317,
                    "max_width": 1280,
                    "min_height": 238,
                    "max_height": 480
                }],
                "params": {}
            }"#,
        )
        .expect("BUG: write manifest fixture");
        let wasm_path =
            Path::new("/tmp/foreign-target/wasm32-unknown-unknown/release/hello_widget.wasm");

        let (manifest_path, manifest) = load_manifest(wasm_path, None, Some(widget_root.clone()))
            .expect("BUG: explicit widget root must resolve manifest");

        assert_eq!(manifest_path, widget_root.join("manifest.json"));
        assert_eq!(manifest.name, "Hello");
    }

    #[test]
    fn resolved_widget_root_uses_cli_root_for_foreign_target_wasm() {
        let widget_root = PathBuf::from("fixtures/hello-widget");
        let cli = parse_test_args(&[
            "testbed",
            "/tmp/foreign-target/wasm32-unknown-unknown/release/hello_widget.wasm",
            "--widget-root",
            widget_root
                .to_str()
                .expect("BUG: test widget root path must be UTF-8"),
        ])
        .expect("BUG: widget-root arg must parse");

        assert_eq!(cli.resolved_widget_root(), Some(widget_root));
    }

    #[test]
    fn parse_args_rejects_split_value_that_looks_like_flag() {
        let result = parse_test_args(&[
            "testbed",
            "widget.wasm",
            "--platform",
            "--perf-report=out.json",
        ]);
        let Err(err) = result else {
            panic!("BUG: a following flag token must not be swallowed as the value");
        };

        let err = err.to_string();
        assert!(
            err.contains("--platform"),
            "rejection names the flag: {err}"
        );
    }

    #[test]
    fn startup_without_platform_arg_selects_the_default() {
        let cli =
            parse_test_args(&["testbed", "widget.wasm"]).expect("BUG: minimal args must parse");

        let platform = platform_catalog::select(cli.platform_id.as_deref())
            .expect("BUG: default platform must resolve");

        assert_eq!(platform.id, "bmc100");
    }

    /// A manifest admitting the rectangular range the deck's own viewports
    /// span, which is what the bundled examples declare.
    fn rectangular_manifest() -> bmc_widget_manifest::Manifest {
        let json = serde_json::json!({
            "uid": "550e8400-e29b-41d4-a716-446655440200",
            "version": "0.1.0",
            "name": "Test",
            "description": "Fixture",
            "author": { "name": "Braiins Forge", "url": "https://braiinsforge.com" },
            "binary": "bin/test",
            "icon": "assets/icon.svg",
            "category": "utility",
            "settings": [],
            "supported_viewports": [{
                "type": "rectangular",
                "min_width": 317, "max_width": 1280,
                "min_height": 238, "max_height": 480,
            }],
            "params": {},
        })
        .to_string();
        <bmc_widget_manifest::Manifest as std::str::FromStr>::from_str(&json)
            .expect("BUG: the fixture manifest must parse")
    }

    #[test]
    fn startup_opens_every_platform_the_widget_admits() {
        let manifest = rectangular_manifest();
        let default = platform_catalog::select(None).expect("BUG: default must resolve");

        let open = super::startup_platforms(default, false, &manifest);

        let ids: Vec<&str> = open.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            ["bmc100", "bmm100", "bmm101"],
            "the round platform is not rectangular, so it stays shut"
        );
    }

    #[test]
    fn a_pinned_platform_opens_on_its_own() {
        let manifest = rectangular_manifest();
        let requested = platform_catalog::select(Some("bmm101")).expect("BUG: must resolve");

        let open = super::startup_platforms(requested, true, &manifest);

        let ids: Vec<&str> = open.iter().map(|p| p.id).collect();
        assert_eq!(ids, ["bmm101"], "a pinned run opens what it was pointed at");
    }

    #[test]
    fn startup_selects_the_requested_platform() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--platform", "BFM100"])
            .expect("BUG: platform arg must parse");

        let platform = platform_catalog::select(cli.platform_id.as_deref())
            .expect("BUG: requested platform must resolve");

        assert_eq!(platform.id, "bfm100");
    }

    #[test]
    fn startup_rejects_an_unknown_platform() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--platform", "NOPE"])
            .expect("BUG: platform arg must parse");

        let err = platform_catalog::select(cli.platform_id.as_deref())
            .expect_err("BUG: unknown platform must fail");

        assert!(err.to_string().contains("NOPE"), "{err}");
    }
}

fn load_manifest(
    wasm_path: &Path,
    explicit: Option<PathBuf>,
    widget_root: Option<PathBuf>,
) -> Result<(PathBuf, bmc_widget_manifest::Manifest)> {
    let manifest_path = explicit
        .or_else(|| widget_root.map(|root| root.join("manifest.json")))
        .with_context(|| {
            format!(
                "could not locate manifest.json for {}. Pass --manifest=<path> or \
                 --widget-root=<path> explicitly.",
                wasm_path.display()
            )
        })?;
    let body = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest = <bmc_widget_manifest::Manifest as std::str::FromStr>::from_str(&body)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    Ok((manifest_path, manifest))
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
/// The pre-GL baseline is taken before the event loop starts, so the difference
/// reported here captures GL initialisation + first-runtime construction.
fn log_startup_memory(rss_before_gl_kb: Option<u64>) {
    let now = current_rss_kb();
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return;
    };
    eprintln!("\n=== Memory (startup) ===");
    if let (Some(before), Some(now)) = (rss_before_gl_kb, now) {
        let delta = now.saturating_sub(before);
        eprintln!("Pre-GL RSS:        {before:>6} kB");
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

    let cli = CliArgs::parse();
    let (manifest_path, manifest) = load_manifest(
        &cli.wasm_path,
        cli.manifest_path.clone(),
        cli.resolved_widget_root(),
    )?;
    let params = bmc_wasm_runtime::manifest_default_params(&manifest);
    let record_request = resolve_record_request(&cli)?;
    let selected_platform = match &record_request {
        Some(request) => request.target.platform,
        None => platform_catalog::select(cli.platform_id.as_deref())?,
    };
    let startup_size = DEFAULT_WINDOW_SIZE;

    println!("Loading widget from: {}", cli.wasm_path.display());
    println!("Manifest:            {}", manifest_path.display());
    println!(
        "Params:              {} key(s) from manifest defaults",
        params.len()
    );
    let display = selected_platform.display();
    println!(
        "Platform: {} ({}) — display {}x{} {:?} dpi={}, {} viewport(s)",
        selected_platform.id,
        selected_platform.label,
        display.logical_width,
        display.logical_height,
        display.shape,
        display.dpi,
        selected_platform.viewports.len()
    );
    if let Some(ref path) = cli.perf_report_path {
        println!(
            "Perf report: {} ({} frames)",
            path.display(),
            cli.perf_frames
        );
    }
    if let Some(ref request) = record_request {
        println!(
            "Recording mode: target={} dataset={}",
            request.target, request.dataset
        );
    }

    let rss_before_gl = current_rss_kb();

    let event_loop = winit::event_loop::EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| anyhow::anyhow!("event loop: {e}"))?;
    let mut handler = TestbedHandler {
        proxy: event_loop.create_proxy(),
        seed: Some(AppSeed {
            cli,
            manifest,
            params,
            platform: selected_platform,
            record_request,
            inner_size: winit::dpi::LogicalSize::new(
                f64::from(startup_size.x),
                f64::from(startup_size.y),
            ),
            rss_before_gl,
        }),
        window: None,
        egui_glow: None,
        app: None,
        repaint_after: std::time::Duration::MAX,
        frame_started_at: None,
        fatal_error: None,
    };
    event_loop
        .run_app(&mut handler)
        .map_err(|e| anyhow::anyhow!("event loop: {e}"))?;

    match handler.fatal_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
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
                    tracing::debug!(
                        kind = ?event.kind,
                        paths = ?event.paths,
                        "hot reload: watcher fired"
                    );
                    let _ = tx.send(());
                }
            } else if let Err(e) = res {
                tracing::warn!("hot reload: watcher error: {e}");
            }
        },
        notify::Config::default(),
    )?;
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}

// ── Touch routing ───────────────────────────────────────────────────

/// Translate egui pointer events on a tile rect into `TouchEvent`s sent to the view.
///
/// Click / drag semantics mirror what the prior winit-based testbed forwarded:
/// a quick click fires `Down` then `Up`; a drag fires `Down` on start,
/// `Move` on each frame the pointer moved, and `Up` on release.
///
/// When `recording` is `Some`, also tracks the gesture (start/current pos + start element)
/// so the recording-side gesture classifier can turn it into a Click / Scroll / Drag
/// `UnifiedEvent` on release.
/// `scale` is what the canvas zoom did to `rect`; the guest only ever hears
/// about its own pixels, so every pointer position divides back out of it.
fn dispatch_touch_events(
    response: &egui::Response,
    rect: egui::Rect,
    view: &mut DeviceView,
    recording: Option<&mut RecordingState>,
    scale: f32,
) {
    // Mirror the device host: a widget that doesn't export `on_touch` is
    // non-interactive, so it never receives touch events.
    if !view.exports_on_touch() {
        return;
    }
    // Carry the recording reborrow through each branch by hand instead of `as_deref_mut`
    // (which clippy rejects since the `Option`'s inner type is already a `&mut`).
    let mut rec = recording;
    let mut touched = false;
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (x, y) = ((pos.x - rect.min.x) / scale, (pos.y - rect.min.y) / scale);
        view.send(ViewCommand::Touch(TouchEvent::Down { x, y }));
        view.send(ViewCommand::Touch(TouchEvent::Up));
        touched = true;
        if let Some(r) = rec.as_mut() {
            let start_element = view.hit_test(x, y);
            r.record_tap((x, y), start_element);
        }
    }
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (x, y) = ((pos.x - rect.min.x) / scale, (pos.y - rect.min.y) / scale);
        view.send(ViewCommand::Touch(TouchEvent::Down { x, y }));
        touched = true;
        if let Some(r) = rec.as_mut() {
            let start_element = view.hit_test(x, y);
            r.begin_gesture((x, y), start_element);
        }
    } else if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (x, y) = ((pos.x - rect.min.x) / scale, (pos.y - rect.min.y) / scale);
        view.send(ViewCommand::Touch(TouchEvent::Move { x, y }));
        touched = true;
        if let Some(r) = rec.as_mut() {
            r.update_gesture((x, y));
        }
    }
    if response.drag_stopped() {
        view.send(ViewCommand::Touch(TouchEvent::Up));
        touched = true;
        if let Some(r) = rec.as_mut() {
            r.finish_gesture();
        }
    }
    if touched {
        // Fire `on_touch` once for the gesture, mirroring the host's per-drain
        // delivery. Pushing the events already armed the view to render.
        view.send(ViewCommand::DeliverTouch);
    }
}

// ── App ─────────────────────────────────────────────────────────────

pub(crate) struct TestbedApp {
    cli: CliArgs,
    prepared_widget: PreparedWidget,
    /// The plane the device windows float over. Never persisted.
    canvas: canvas::Canvas,
    /// Chrome theme; Auto follows the system.
    theme: theme::ThemeChoice,
    /// Toolbar icons, rasterized on demand per drawn pixel size.
    icons: icon::Icons,
    /// Whether each view carries its own timings — off by default:
    /// an instrument over the widget, while the status bar covers the whole.
    show_view_timings: bool,
    /// A one-shot window rearrangement, consumed at the next paint.
    arrange: Option<device_window::ArrangeMode>,
    notice: Option<Notice>,
    /// Where views render, once the modes that cannot thread have had their say.
    views: ViewPlacement,
    /// Set once the perf report is sealed, so the host can close the window.
    exit_requested: bool,
    /// What the view pass left for the paint pass that follows it.
    pass: FramePass,
    /// Builds the per-view `FemtoVgRenderer` GL contexts.
    get_proc: GlProcAddress,
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
    /// Current credential view, in the wire JSON shape.
    /// Mutated by the Credentials sidebar section; tile runtimes
    /// are kept in sync via `deliver_credentials_update`.
    pub(crate) credentials: serde_json::Map<String, serde_json::Value>,
    /// Secrets from `--secrets`, handed to the runtime with each credential
    /// delivery; empty by default, so fetches refuse before egress.
    pub(crate) secrets: bmc_widget_protocol::CredentialSecrets,
    /// Base-URL rewrites from `--rewrite-url`, installed on every runtime.
    url_rewrites: Vec<(String, String)>,
    gl: Arc<egui_glow::glow::Context>,
    pub(crate) tiles: Vec<DeviceView>,
    /// Whatever is on its way out, drained a little each frame.
    teardown: Teardown,
    clock: Clock,
    /// Offline toggle: seals every tile's live I/O so refreshes fail.
    offline: bool,
    hot_reload: HotReload,
    perf: PerfState,
    pub(crate) recording_mode: RecordingMode,
    /// The devices currently open on the canvas, one window each.
    /// Recording pins this to the target's platform alone.
    pub(crate) open_platforms: Vec<&'static Platform>,
}

/// Wall-clock instants used to drive per-frame timing.
/// `last_frame` advances on every `ui` call and yields `delta_ms` for the WASM runtime;
/// `start_instant` is the fixed origin for the monotonic clock the runtime sees.
struct Clock {
    last_frame: std::time::Instant,
    start_instant: std::time::Instant,
    /// Fast-forward offset (ms) for the displayed system time; "reset" zeroes it.
    offset_ms: u64,
    /// Fast-forward offset (ms) for the monotonic clock. Ratchets up with each
    /// advance and never rewinds — "reset" leaves it, so pending poll/render
    /// deadlines don't stall behind a rewound clock.
    monotonic_offset_ms: u64,
}

/// Filesystem watcher + manual-reload signal. Drains as a single "rebuild every runtime"
/// signal each frame inside `poll_hot_reload`.
struct HotReload {
    /// Live `notify` watcher. Held to keep the watch thread alive — when dropped, file
    /// events stop arriving. Never read after construction.
    _watcher: RecommendedWatcher,
    /// Channel fed by `setup_watcher` whenever the wasm file on disk changes.
    watcher_rx: std::sync::mpsc::Receiver<()>,
    /// Set by the toolbar's "Reload WASM" button; consumed as a synthetic
    /// watcher event on the next `poll_hot_reload` tick.
    manual_reload: bool,
}

/// What the view pass hands to the paint pass that follows it.
///
/// The two are separate phases because the views' GL work — drawing, and
/// waiting on a threaded view's fence — belongs outside a pass that only
/// builds a draw list.
#[derive(Debug, Clone, Copy)]
struct FramePass {
    /// When the views ran, so the pass animates against the frame they
    /// rendered rather than the moment it happens to paint.
    now: std::time::Instant,
    /// Earliest deadline across the views, or `None` when all of them idle.
    next_wake_ms: Option<u64>,
}

/// The teardown pipeline: whatever was closed, on its way out across frames.
///
/// A closed view first waits for a pass that can free its texture — the
/// painter is only in hand between passes — and a threaded view's worker then
/// winds down in the background until a poll collects it. Spreading that over
/// frames is the point: a runtime's teardown can hold a fetch for its whole
/// I/O timeout, and closing a platform must never stall the UI on it.
#[derive(Default)]
struct Teardown {
    /// Views taken out of service — a closed platform's worth,
    /// or every open one at once when recording mode swaps the whole canvas.
    views: Vec<DeviceView>,
    /// Worker threads asked to stop, polled until they exit.
    workers: Vec<view::worker::Retired>,
}

impl Teardown {
    /// Advance the pipeline one step: free what the painter can, poll the rest.
    fn drain(&mut self, gl: &egui_glow::glow::Context, painter: &mut egui_glow::Painter) {
        for view in std::mem::take(&mut self.views) {
            self.workers.extend(view.release(gl, painter));
        }
        self.workers = std::mem::take(&mut self.workers)
            .into_iter()
            .filter_map(view::worker::Retired::reap)
            .collect();
    }

    /// Run the pipeline to the end, waiting out every worker.
    ///
    /// For process exit only: blocking is fine with no UI left, and a detached
    /// worker would race its GL context against the dying display connection.
    fn finish(&mut self, gl: &egui_glow::glow::Context, painter: &mut egui_glow::Painter) {
        for view in std::mem::take(&mut self.views) {
            self.workers.extend(view.release(gl, painter));
        }
        for retired in std::mem::take(&mut self.workers) {
            retired.reap_blocking();
        }
    }
}

/// Per-frame performance accounting. The rolling window drives the FPS readout
/// in the status bar; the full vector is what `--perf-report=` writes to disk at exit.
/// A transient chrome banner: the outcome of an action whose other traces
/// left the screen — a saved take, mostly. Expires on its own or on click.
struct Notice {
    text: String,
    error: bool,
    shown_at: std::time::Instant,
}

impl Notice {
    /// How long a success notice lingers; enough to read a path, short enough
    /// to never need dismissing by hand.
    const TTL: std::time::Duration = std::time::Duration::from_secs(10);

    fn success(text: String) -> Self {
        Self {
            text,
            error: false,
            shown_at: std::time::Instant::now(),
        }
    }

    fn error(text: String) -> Self {
        Self {
            text,
            error: true,
            shown_at: std::time::Instant::now(),
        }
    }
}

struct PerfState {
    /// Total frames rendered so far. Used to drive the `--perf-frames` exit condition.
    frame_count: u32,
    /// Per-frame timings from FULL tile's runtime;
    /// written to disk by `--perf-report=` at exit.
    samples: Vec<bmc_render::FrameTimings>,
    /// Per-frame fuel per profiling section from the FULL tile's guest.
    section_samples: Vec<std::collections::BTreeMap<String, u64>>,
    /// Set once the report is written, so the frame-count threshold
    /// and an early window close can't double-write it.
    written: bool,
    /// Last frame's wall-clock duration (microseconds);
    /// recent samples averaged for FPS.
    recent_frame_us: std::collections::VecDeque<u32>,
}

impl TestbedApp {
    fn new(
        gl: Arc<egui_glow::glow::Context>,
        get_proc: GlProcAddress,
        cli: CliArgs,
        manifest: bmc_widget_manifest::Manifest,
        params: std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
        active_platform: &'static Platform,
        record_request: Option<RecordRequest>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (watcher, watcher_rx) =
            setup_watcher(&cli.wasm_path).map_err(|e| format!("watcher: {e}"))?;
        let prepared_widget = PreparedWidget::new(&cli.wasm_path, cli.asset_root.as_deref())?;

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
            night_mode: false,
        };

        // Recording no longer pins here: `enter_recording` below saves
        // this set as what Save/Cancel restores, then pins to the take's platform.
        let pinned = cli.platform_id.is_some();
        let open_platforms = startup_platforms(active_platform, pinned, &manifest);

        let now = std::time::Instant::now();
        let declared = declared_slots(&manifest);
        let secrets = cli.credential_secrets(&declared)?;
        let url_rewrites = cli.url_rewrites()?;
        let views = view_placement(&cli);
        let mut app = Self {
            cli,
            prepared_widget,
            secrets,
            url_rewrites,
            canvas: canvas::Canvas::default(),
            theme: theme::ThemeChoice::Auto,
            icons: icon::Icons::new(),
            show_view_timings: false,
            arrange: None,
            notice: None,
            views,
            exit_requested: false,
            pass: FramePass {
                now,
                next_wake_ms: None,
            },
            get_proc,
            manifest,
            params,
            system: pending_system,
            credentials: serde_json::Map::new(),
            gl,
            tiles: Vec::new(),
            teardown: Teardown::default(),
            clock: Clock {
                last_frame: now,
                start_instant: now,
                offset_ms: 0,
                monotonic_offset_ms: 0,
            },
            offline: false,
            hot_reload: HotReload {
                _watcher: watcher,
                watcher_rx,
                manual_reload: false,
            },
            perf: PerfState {
                frame_count: 0,
                samples: Vec::new(),
                section_samples: Vec::new(),
                written: false,
                recent_frame_us: std::collections::VecDeque::with_capacity(60),
            },
            recording_mode: RecordingMode::new(),
            open_platforms,
        };
        if let Some(RecordRequest { target, dataset }) = record_request {
            app.enter_recording(target, dataset);
        }
        Ok(app)
    }

    fn prepare_updated_widget(&self) -> Result<(PreparedWidget, Vec<u8>)> {
        let prepared = PreparedWidget::new(&self.cli.wasm_path, self.cli.asset_root.as_deref())?;
        let wasm = std::fs::read(prepared.wasm_path()).with_context(|| {
            format!(
                "failed to read prepared module {}",
                prepared.wasm_path().display()
            )
        })?;
        Ok((prepared, wasm))
    }

    /// Drain pending watcher events; if any fired, rebuild every live view's
    /// runtime from the (now-updated) wasm bytes on disk.
    fn poll_hot_reload(&mut self) {
        let manual = self.hot_reload.manual_reload;
        self.hot_reload.manual_reload = false;
        let mut watcher_events = 0_usize;
        while self.hot_reload.watcher_rx.try_recv().is_ok() {
            watcher_events += 1;
        }
        let needs_reload = manual || watcher_events > 0;
        if !needs_reload {
            return;
        }
        tracing::debug!(
            manual,
            watcher_events,
            "hot reload: trigger drained, beginning rebuild"
        );
        let (prepared_widget, wasm_bytes) = match self.prepare_updated_widget() {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::warn!("hot reload: package preparation failed: {error:#}");
                return;
            }
        };
        // Shared, because every view's seed carries a handle to the same bytes.
        let wasm_bytes: Arc<[u8]> = wasm_bytes.into();
        tracing::info!(
            wasm_bytes = wasm_bytes.len(),
            tiles = self.tiles.len(),
            "hot reload: rebuilding tile runtime(s)"
        );
        // A rebuilt runtime starts with nothing bound, so the sidebar's bindings
        // are re-delivered below — without that, a hot reload drops
        // a credential-fed widget back to its unbound state.
        let credentials = bmc_wasm_runtime::parse_credentials_json(&self.credentials);
        let secrets = self.secrets.clone();
        let active_record_idx = self
            .recording_mode
            .active()
            .map(RecordingState::active_tile);
        for idx in 0..self.tiles.len() {
            let view = &self.tiles[idx];
            if !view.is_live() {
                continue; // placeholder — no runtime to rebuild
            }
            let (platform, placed_shape) = (view.platform, view.shape);
            let (width, height) = (view.width(), view.height());
            let label = view.label().to_owned();
            let (led_tx, led_rx) = if view.led_count().is_some() {
                let (led_tx, led_rx) = std::sync::mpsc::channel();
                (Some(led_tx), Some(led_rx))
            } else {
                (None, None)
            };
            // The same config the view was built with, so a reload keeps its
            // KV store and — mid-recording — its fetch observer.
            let kv_path = self.kv_dir(platform, view.kv_key());
            let mut config = self.view_runtime_config(kv_path, active_record_idx == Some(idx));
            config.led_request_sender = led_tx;
            let seed = view::ViewSeed {
                wasm: Arc::clone(&wasm_bytes),
                width,
                height,
                geometry: RuntimeTileGeometry::for_viewport_shape(platform, placed_shape),
                config,
                label,
                // Only live views reach here, and a view is live because its
                // viewport was supported when it was built.
                supported: true,
                get_proc: self.get_proc.clone(),
            };
            let rebind = view::Rebind {
                credentials: Box::new(credentials.clone()),
                secrets: Box::new(secrets.clone()),
                led_rx,
            };
            let view = &mut self.tiles[idx];
            if let Err(e) = view.reload(seed, rebind) {
                tracing::warn!("hot reload: {}: {e:#}", view.label());
            }
        }
        self.prepared_widget = prepared_widget;
    }

    /// Open or close a device on the canvas, leaving params and system state
    /// intact. Recording keeps its platform pinned by refusing toggles.
    pub(crate) fn toggle_platform(&mut self, target_id: &str, ctx: &egui::Context) {
        if let Err(reason) = can_switch_platform(self.recording_mode.engaged()) {
            tracing::warn!("toggle: refusing platform toggle of '{target_id}': {reason}");
            return;
        }
        let Some(platform) = platform_catalog::platform(target_id) else {
            tracing::error!("toggle: platform '{target_id}' not found");
            return;
        };
        if toggle_open(&mut self.open_platforms, platform) {
            // Views are built at the next redraw, where the painter is in hand.
        } else {
            // Releasing them needs the painter too, so they only retire here.
            let (closed, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tiles)
                .into_iter()
                .partition(|view| view.platform.id == platform.id);
            self.teardown.views.extend(closed);
            self.tiles = kept;
        }
        ctx.request_repaint();
    }

    /// Open the choosing phase: every supported platform on the canvas,
    /// packed to fit, each viewport wearing its choose overlay. The take
    /// starts when one of them is clicked; Cancel puts the canvas back.
    fn start_choosing(&mut self, ctx: &egui::Context) {
        if !self
            .recording_mode
            .open_choosing(self.recorded_targets(), self.open_platforms.clone())
        {
            return;
        }
        self.open_platforms = platform_catalog::PLATFORMS
            .iter()
            .filter(|p| toolbar::platform_supported(p, &self.manifest))
            .collect();
        // Pack chooses the zoom that fits everything, so every candidate is
        // in view when the overlays appear.
        self.arrange = Some(device_window::ArrangeMode::Pack);
        ctx.request_repaint();
    }

    /// The datasets each target already carries in the widget's capture
    /// config — the overlays badge them, since choosing one again overwrites
    /// its fixture on Save.
    fn recorded_targets(&self) -> std::collections::HashMap<String, Vec<String>> {
        let mut recorded = std::collections::HashMap::<String, Vec<String>>::new();
        let Some(widget_root) = self.cli.resolved_widget_root() else {
            return recorded;
        };
        let capture_dir = widget_root.join("capture");
        let Ok(config) = bmc_wasm_runtime::capture_config::load_from_capture_dir(&capture_dir)
        else {
            return recorded;
        };
        for (dataset, target) in config.capture_matrix() {
            recorded
                .entry(target.to_string())
                .or_default()
                .push(dataset.to_owned());
        }
        recorded
    }

    /// Enter recording mode: pin the canvas to the take's platform and retire
    /// every tile, so the next redraw rebuilds them through `build_views` with
    /// the recording config, inline.
    ///
    /// Ctx-free, so startup (`--record`) can call it before the first frame;
    /// UI callers follow it with `request_repaint`.
    /// The take wipes the recorded viewport's KV store for a deterministic
    /// baseline, so the live dir is stashed aside first and put back on exit.
    pub(crate) fn enter_recording(&mut self, target: platform_catalog::Target, dataset: String) {
        if self.recording_mode.active().is_some() {
            tracing::warn!("record: already recording, ignoring a second entry");
            return;
        }
        let kv_stash = self.stash_kv_dir(target);
        let started = self.recording_mode.begin_take(
            target,
            dataset,
            self.cli.resolved_widget_root(),
            &self.params,
            &self.system,
            &self.credentials,
            kv_stash,
            &self.open_platforms,
        );
        assert!(started, "BUG: begin_take refused after the active() guard");
        self.teardown.views.extend(std::mem::take(&mut self.tiles));
        self.open_platforms = vec![target.platform];
        // Repack once the rebuilt views exist: the recording sidebar just
        // took a slice of the canvas, and the window would sit under it.
        self.arrange = Some(device_window::ArrangeMode::Pack);
    }

    /// Save the take: write the fixture, and put the canvas back only if that
    /// worked. The outcome stays on screen as a notice — a successful unwind
    /// would otherwise say nothing about where the fixture went, and a failed
    /// write nothing at all.
    fn save_recording(&mut self, ctx: &egui::Context) {
        // Drain first: the runtime and the fetch buffer only give their events
        // up once, and they need the view still live to do it.
        self.drain_take_sources();
        let Some(outcome) = self.recording_mode.write_take() else {
            return;
        };
        match outcome {
            Ok(text) => {
                self.notice = Some(Notice::success(text));
                if let Some(unwind) = self.recording_mode.end() {
                    self.apply_record_unwind(unwind, ctx);
                }
            }
            // The take stays open with everything it collected, so pressing
            // Save again after fixing the cause writes the same recording.
            Err(text) => {
                self.notice = Some(Notice::error(format!(
                    "{text}\nThe take is still running — fix the cause and press Save again, \
                     or Cancel to discard it."
                )));
            }
        }
    }

    /// Cancel whichever record phase is on, putting the canvas back.
    fn cancel_record_mode(&mut self, ctx: &egui::Context) {
        let Some(unwind) = self.recording_mode.end() else {
            return;
        };
        self.apply_record_unwind(unwind, ctx);
    }

    /// Undo what the record mode displaced, per what its end handed back.
    fn apply_record_unwind(&mut self, unwind: RecordUnwind, ctx: &egui::Context) {
        match unwind {
            // The extras close; views the restored canvas keeps are plain
            // and stay as they are.
            RecordUnwind::Choosing { restore_platforms } => {
                let (kept, closed): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tiles)
                    .into_iter()
                    .partition(|view| restore_platforms.iter().any(|p| p.id == view.platform.id));
                self.tiles = kept;
                self.teardown.views.extend(closed);
                self.open_platforms = restore_platforms;
            }
            // Every view carries the recording config, so all of them retire
            // and rebuild plain; the stashed KV dir goes back.
            RecordUnwind::Take {
                restore_platforms,
                kv_stash: (live, stash),
            } => {
                let _ = std::fs::remove_dir_all(&live);
                if let Some(stash) = stash
                    && let Err(e) = std::fs::rename(&stash, &live)
                {
                    tracing::warn!("record: cannot restore KV dir {}: {e}", live.display());
                }
                self.teardown.views.extend(std::mem::take(&mut self.tiles));
                self.open_platforms = restore_platforms;
            }
        }
        // The canvas widens by the sidebar's slice on the way out, and the
        // windows were placed against the narrower one — entering already
        // repacked them, so there is no untouched arrangement left to keep.
        self.arrange = Some(device_window::ArrangeMode::Pack);
        ctx.request_repaint();
    }

    /// Move the take's KV dir to a `.pre-record` sibling,
    /// returning `(live, stash)`. A rename rather than a key snapshot,
    /// since the snapshot format is string-typed and this must be exact.
    fn stash_kv_dir(&self, target: platform_catalog::Target) -> (PathBuf, Option<PathBuf>) {
        let live = self.kv_dir(target.platform, target.viewport.id);
        if !live.exists() {
            return (live, None);
        }
        let stash = live.with_extension("pre-record");
        let _ = std::fs::remove_dir_all(&stash);
        match std::fs::rename(&live, &stash) {
            Ok(()) => (live, Some(stash)),
            Err(e) => {
                tracing::warn!("record: cannot stash KV dir {}: {e}", live.display());
                (live, None)
            }
        }
    }

    /// Make sure every open platform has its views, building the missing ones.
    ///
    /// Runs outside the egui pass, because registering a texture and painting
    /// with it both want the painter.
    fn ensure_views(
        &mut self,
        painter: &mut egui_glow::Painter,
        window: &window::GlWindow,
    ) -> Result<()> {
        let missing: Vec<&'static Platform> = self
            .open_platforms
            .iter()
            .copied()
            .filter(|p| !self.tiles.iter().any(|view| view.platform.id == p.id))
            .collect();
        for platform in missing {
            self.build_views(platform, painter, window)?;
        }
        Ok(())
    }

    /// Build and append one view per viewport of `platform`.
    fn build_views(
        &mut self,
        platform: &'static Platform,
        painter: &mut egui_glow::Painter,
        window: &window::GlWindow,
    ) -> Result<()> {
        let get_proc = self.get_proc.clone();
        let wasm_bytes: Arc<[u8]> = std::fs::read(self.prepared_widget.wasm_path())
            .with_context(|| {
                format!(
                    "failed to read {}",
                    self.prepared_widget.wasm_path().display()
                )
            })?
            .into();
        let first_build = self.tiles.is_empty();
        let active_record_idx = self
            .recording_mode
            .active()
            .map(RecordingState::active_tile);

        let mut tiles = Vec::with_capacity(platform.viewports.len());
        for (tile_idx, viewport) in platform.viewports.iter().enumerate() {
            let placed = &PlacedTile::for_viewport(platform, viewport);
            let (w, h) = (placed.w, placed.h);
            let label = placed.label.clone();
            let (led_tx, led_rx) = if placed.led_count.is_some() {
                let (led_tx, led_rx) = std::sync::mpsc::channel();
                (Some(led_tx), Some(led_rx))
            } else {
                (None, None)
            };
            // Active recording tile wipes its KV first so the fixture
            // starts from a known baseline.
            let kv_path = self.kv_dir(platform, &placed.kv_key);
            if active_record_idx == Some(tile_idx) {
                let _ = std::fs::remove_dir_all(&kv_path);
                let _ = std::fs::create_dir_all(&kv_path);
            }
            if let Some(widget_root) = self.cli.resolved_widget_root() {
                seed_kv_from_widget_root(&widget_root, &kv_path);
            }

            let mut rt_config =
                self.view_runtime_config(kv_path, active_record_idx == Some(tile_idx));
            rt_config.led_request_sender = led_tx;
            let seed = view::ViewSeed {
                wasm: Arc::clone(&wasm_bytes),
                width: w,
                height: h,
                geometry: RuntimeTileGeometry::for_viewport_shape(platform, placed.shape),
                config: rt_config,
                label,
                supported: viewport_supported(placed, &self.manifest.supported_viewports),
                get_proc: get_proc.clone(),
            };
            tiles.push(self.build_one_view(placed, platform, seed, led_rx, painter, window)?);
        }
        if first_build
            && let Some((major, minor, patch)) = tiles.iter().find_map(DeviceView::sdk_version)
        {
            println!("Widget SDK version: {major}.{minor}.{patch}");
        }
        // Snapshot the active recording tile's KV directory at start
        // so the fixture's `header.kv` reproduces the initial state on replay.
        let recording_kv = self
            .recording_mode
            .active()
            .filter(|rec| rec.target().platform.id == platform.id)
            .map(|rec| self.kv_dir(platform, rec.target().viewport.id));
        if let Some(kv_path) = recording_kv {
            self.recording_mode
                .set_kv_baseline(snapshot_kv_dir(&kv_path));
        }
        self.tiles.extend(tiles);
        Ok(())
    }

    /// Build one view, on a thread of its own where the driver allows it.
    ///
    /// A view that cannot get a shared context falls back to the UI thread on
    /// its own rather than failing the run: the fallback is only slower, and a
    /// testbed that refuses to open teaches nothing about the widget.
    fn build_one_view(
        &mut self,
        placed: &PlacedTile,
        platform: &'static Platform,
        seed: view::ViewSeed,
        led_rx: Option<std::sync::mpsc::Receiver<bmc_wasm_runtime::LedRequest>>,
        painter: &mut egui_glow::Painter,
        window: &window::GlWindow,
    ) -> Result<DeviceView> {
        let placement = placement_for_build(self.views, self.recording_mode.active().is_some());
        if placement == ViewPlacement::OwnThread {
            // Only the context is tried here. A build that fails on the worker
            // would fail inline too — a bad wasm is not a threading problem —
            // so that one is reported rather than quietly downgraded.
            match window.shared_offscreen() {
                Ok(offscreen) => {
                    let (worker, textures) = view::worker::spawn(view::worker::WorkerSeed {
                        offscreen,
                        seed,
                        label: placed.label.clone(),
                        led_rx,
                        handoff: self.cli.view_handoff,
                    })?;
                    let mut tex_ids = [egui::TextureId::default(); 2];
                    for (tex_id, texture) in tex_ids.iter_mut().zip(textures) {
                        let texture = std::num::NonZeroU32::new(texture)
                            .context("the view thread handed back texture 0")?;
                        *tex_id = painter
                            .register_native_texture(egui_glow::glow::NativeTexture(texture));
                    }
                    return Ok(DeviceView::new_threaded(placed, platform, worker, tex_ids));
                }
                Err(e) => tracing::warn!(
                    label = %placed.label,
                    "view: no context of its own ({e:#}); rendering on the UI thread"
                ),
            }
        }
        let parts = seed.build(&self.gl)?;
        let tex_id = parts.targets.register(painter);
        Ok(DeviceView::new_inline(
            placed, platform, parts, tex_id, led_rx,
        ))
    }

    /// Where one view keeps its KV store.
    ///
    /// Platform-qualified so two open devices sharing a viewport id
    /// (every platform names one "full") don't write into one store.
    fn kv_dir(&self, platform: &Platform, kv_key: &str) -> PathBuf {
        let widget_name = self
            .cli
            .wasm_path
            .file_stem()
            .map_or("widget".into(), |s| s.to_string_lossy().into_owned());
        self.cli
            .wasm_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("widget_data")
            .join(widget_name)
            .join(platform.id)
            .join(kv_key)
    }

    /// Runtime config for one view, carrying the sidebar state it starts from.
    ///
    /// The recording view gets the unified fetch observer, so its traffic
    /// reaches the timeline; every other view takes the plain default.
    /// The observer stamps `at_ms` against the take's own start, so a view
    /// rebuilt mid-recording keeps writing on the timeline earlier events used.
    fn view_runtime_config(&self, kv_path: PathBuf, recording: bool) -> RuntimeConfig {
        let recording_start = self.recording_mode.take_epoch().filter(|_| recording);
        let mut config = if let Some(start) = recording_start {
            fixtures::build_unified_recording_config(
                kv_path,
                self.recording_mode.fetch_buffer(),
                start,
            )
        } else {
            RuntimeConfig {
                kv_store_path: Some(kv_path),
                asset_cache: self.cli.asset_cache(),
                ..RuntimeConfig::default()
            }
        };
        config.mesh_msaa_samples = 4;
        config.package_assets = Some(PackageAssetStore::new(self.prepared_widget.asset_root()));
        config.params = self.params.clone();
        config.system = self.system.clone();
        // The sidebar's bindings as well: a rebuilt runtime starts unbound,
        // and replay installs `initial_credentials` identically,
        // so a recording's first delivery diffs against the operator's real state.
        config.credentials = bmc_wasm_runtime::parse_credentials_json(&self.credentials);
        config.credential_secrets = self.secrets.clone();
        config.url_rewrites.clone_from(&self.url_rewrites);
        config
    }

    /// Write the perf report once, from whatever samples were collected so far.
    /// Idempotent, so the `--perf-frames` threshold and an early window close
    /// can each seal the run without double-writing.
    fn finish_perf_report(&mut self) {
        if self.perf.written {
            return;
        }
        let Some(path) = self.cli.perf_report_path.as_ref() else {
            return;
        };
        write_perf_report(path, &self.perf.samples, &self.perf.section_samples);
        self.perf.written = true;
    }

    /// Advance every view for this frame, ahead of the pass that paints them.
    ///
    /// Separate from the pass because this is where GL happens — widgets draw
    /// into their framebuffers and threaded ones are waited on — while the
    /// pass only describes what to paint.
    fn drive_views(&mut self) {
        let now = std::time::Instant::now();
        let frame_us = now.duration_since(self.clock.last_frame).as_micros() as u32;
        self.clock.last_frame = now;
        self.record_frame_us(frame_us);
        self.pass = FramePass {
            now,
            next_wake_ms: self.render_tiles(now),
        };

        // Seal the report and close once enough real renders are collected.
        if self.cli.perf_report_path.is_some() && self.perf.frame_count >= self.cli.perf_frames {
            self.finish_perf_report();
            self.exit_requested = true;
        }
    }

    /// Drive each tile off its own runtime scheduler, returning the earliest
    /// next render across them (ms) for the host wake.
    ///
    /// Saves the GL framebuffer binding and viewport before an inline view
    /// mutates them, and restores both afterwards, so egui's own draw list runs
    /// against the screen framebuffer the way it expects. Skipping that caused
    /// screen-wide trails, egui's clear landing on a tile FBO instead of the
    /// default one.
    ///
    /// Clock and delivery drain run every tick; the WASM render is gated,
    /// so idle tiles cost nothing — the contract the device host honours.
    fn render_tiles(&mut self, now: std::time::Instant) -> Option<u64> {
        // SAFETY: gl is current on this thread; the queries below only read.
        let (prev_fbo, prev_viewport) = unsafe {
            let prev_fbo = self
                .gl
                .get_parameter_i32(egui_glow::glow::FRAMEBUFFER_BINDING);
            let mut vp = [0_i32; 4];
            self.gl
                .get_parameter_i32_slice(egui_glow::glow::VIEWPORT, &mut vp);
            (prev_fbo, vp)
        };

        // A fast-forward advances both clocks so a due poll fires as its data
        // ages out; "reset" rewinds only the display clock, never the monotonic
        // one (which uses its own ratcheting offset).
        let offset_ms = self.clock.offset_ms;
        let monotonic_ms = now.duration_since(self.clock.start_instant).as_millis() as u64
            + self.clock.monotonic_offset_ms;
        let system_time = (chrono::Local::now()
            + chrono::Duration::milliseconds(offset_ms.cast_signed()))
        .fixed_offset();
        let tick = view::ViewTick {
            now,
            system_time,
            monotonic_ms,
            offline: self.offline,
        };
        // In recording mode only the active view runs; `App::ui` paints the rest
        // as blank slabs. Skipping their render keeps the visual focus clear,
        // and stops idle runtimes spending fuel on frames nobody keeps.
        let active_record_idx = self
            .recording_mode
            .active()
            .map(RecordingState::active_tile);
        // The perf report follows the first live view (placeholders have none).
        let perf_idx = self.tiles.iter().position(DeviceView::is_live);
        let mut next_wake_ms: Option<u64> = None;
        // Captured only on a real render, so `--perf-frames` counts widget
        // renders, not idle ticks.
        let mut perf_capture: Option<(
            bmc_render::FrameTimings,
            std::collections::BTreeMap<String, u64>,
        )> = None;
        for (idx, view) in self.tiles.iter_mut().enumerate() {
            if active_record_idx.is_some_and(|active| active != idx) {
                continue;
            }
            let ticked = view.tick(&tick, &self.gl);
            if let Some(delay) = ticked.next_wake_ms {
                next_wake_ms = Some(next_wake_ms.map_or(delay, |w| w.min(delay)));
            }
            if ticked.rendered && Some(idx) == perf_idx {
                perf_capture = view.take_perf_sample();
            }
        }
        if let Some((timings, sections)) = perf_capture {
            self.perf.samples.push(timings);
            self.perf.section_samples.push(sections);
            self.perf.frame_count += 1;
        }

        // Restore framebuffer + viewport so egui draws onto the screen FBO at the right size.
        // SAFETY: same context invariants as the read above; values came from this very GL.
        unsafe {
            // 0 maps to the default framebuffer; any non-zero prior binding goes back as a
            // `NativeFramebuffer`. The cast through `NonZeroU32` filters the 0 case correctly.
            let target =
                std::num::NonZeroU32::new(prev_fbo as u32).map(egui_glow::glow::NativeFramebuffer);
            self.gl
                .bind_framebuffer(egui_glow::glow::FRAMEBUFFER, target);
            self.gl.viewport(
                prev_viewport[0],
                prev_viewport[1],
                prev_viewport[2],
                prev_viewport[3],
            );
        }

        next_wake_ms
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

fn can_switch_platform(recording_active: bool) -> Result<(), &'static str> {
    if recording_active {
        Err("recording is active")
    } else {
        Ok(())
    }
}

/// The devices a fresh testbed opens.
///
/// Every platform the widget admits, since watching one change land
/// everywhere at once is what the canvas is for.
/// `pinned` — a recording, or an explicit `--platform` — asks for one
/// device and gets only that.
fn startup_platforms(
    active: &'static Platform,
    pinned: bool,
    manifest: &bmc_widget_manifest::Manifest,
) -> Vec<&'static Platform> {
    if pinned {
        return vec![active];
    }
    let supported: Vec<&'static Platform> = platform_catalog::PLATFORMS
        .iter()
        .filter(|platform| toolbar::platform_supported(platform, manifest))
        .collect();
    // A manifest admitting nothing still opens where it was pointed, so the
    // operator sees the placeholder saying so rather than an empty canvas.
    if supported.is_empty() {
        vec![active]
    } else {
        supported
    }
}

/// Flip `platform` in the open set; returns whether it is open afterwards.
fn toggle_open(open: &mut Vec<&'static Platform>, platform: &'static Platform) -> bool {
    if let Some(pos) = open.iter().position(|p| p.id == platform.id) {
        open.remove(pos);
        false
    } else {
        open.push(platform);
        true
    }
}

/// Whether a widget's `supported` viewports admit a tile of this shape and size.
/// An empty list is unconstrained — every tile qualifies.
fn viewport_supported(
    placed: &PlacedTile,
    supported: &[bmc_widget_manifest::WidgetViewportConstraint],
) -> bool {
    use bmc_widget_manifest::ViewportShape;
    if supported.is_empty() {
        return true;
    }
    supported.iter().any(|c| {
        let shape_ok = matches!(
            (placed.shape, c.viewport_shape),
            (DisplayShape::Rectangular, ViewportShape::Rectangular)
                | (DisplayShape::Round, ViewportShape::Round)
        );
        shape_ok
            && c.min_width.is_none_or(|lo| placed.w >= lo)
            && c.max_width.is_none_or(|hi| placed.w <= hi)
            && c.min_height.is_none_or(|lo| placed.h >= lo)
            && c.max_height.is_none_or(|hi| placed.h <= hi)
    })
}

/// Paint a dim "size not supported" slab where a widget declines a tile.
fn paint_placeholder(
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    palette: &theme::Palette,
) {
    painter.rect_filled(rect, 0.0, palette.placeholder_fill);
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, palette.placeholder_outline),
        egui::StrokeKind::Inside,
    );
    let icon = rect.center() - egui::vec2(0.0, 16.0);
    let radius = 13.0;
    let stroke = egui::Stroke::new(2.0_f32, palette.placeholder_glyph);
    painter.circle_stroke(icon, radius, stroke);
    let slash = radius * 0.72;
    painter.line_segment(
        [
            icon + egui::vec2(-slash, -slash),
            icon + egui::vec2(slash, slash),
        ],
        stroke,
    );
    painter.text(
        rect.center() + egui::vec2(0.0, 12.0),
        egui::Align2::CENTER_CENTER,
        "Size not supported",
        egui::FontId::proportional(13.0),
        egui::Color32::from_gray(120),
    );
    painter.text(
        rect.center() + egui::vec2(0.0, 30.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.0),
        egui::Color32::from_gray(75),
    );
}

fn paint_tile_texture(ui: &egui::Ui, tile: &DeviceView, rect: egui::Rect) {
    // FemtoVG renders bottom-up into the FBO; flip V to display top-down.
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0));
    ui.painter()
        .image(tile.tex_id(), rect, uv, egui::Color32::WHITE);
    if paint::is_round(tile.shape) {
        paint::paint_round_overlay(ui.painter(), rect);
    }
}

/// Host wake floor when nothing sooner is scheduled: drains deliveries
/// and animates chrome (the LED strip) without running widget WASM
/// — real renders are gated per tile.
/// ~30 Hz keeps LEDs smooth; it is not a render rate.
const DRAIN_TICK_MS: u64 = 33;

// ── Event loop ──────────────────────────────────────────────────────

/// egui asks for repaints from inside its own pass, so the request is routed
/// back into the loop as an event rather than acted on immediately.
#[derive(Debug)]
enum UserEvent {
    Redraw(std::time::Duration),
}

/// What the app needs to exist, held until the loop hands us a display.
///
/// winit builds windows in `resumed` and not before,
/// so the app cannot be constructed in `main` where these values are produced.
struct AppSeed {
    cli: CliArgs,
    manifest: bmc_widget_manifest::Manifest,
    params:
        std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
    platform: &'static Platform,
    record_request: Option<RecordRequest>,
    inner_size: winit::dpi::LogicalSize<f64>,
    rss_before_gl: Option<u64>,
}

/// Print what this machine can do about per-view threads.
///
/// Creating the context proves little on its own: a share group is only usable
/// if the context can go current on the thread that will own it, and a driver
/// is free to refuse that. So the probe hands it to a real thread and reports
/// what that thread sees.
fn report_shared_gl(window: &window::GlWindow, gl: &egui_glow::glow::Context) {
    use egui_glow::glow::HasContext as _;

    /// Side of the texture the view thread allocates for the compositor to find.
    const PROBE_TEXTURE_SIZE: i32 = 4;
    /// `GL_TEXTURE_WIDTH`, which glow exposes only as a raw enum. It is a
    /// per-level parameter, so it reads back through `glGetTexLevelParameteriv`;
    /// `glGetTexParameteriv` rejects it and answers 0.
    const TEXTURE_WIDTH: u32 = 0x1000;

    // SAFETY: the window context is current on this thread.
    let compositor_version = unsafe { gl.get_parameter_string(egui_glow::glow::VERSION) };
    println!("compositor GL: {compositor_version}");

    let offscreen = match window.shared_offscreen() {
        Ok(offscreen) => offscreen,
        Err(e) => {
            println!("shared context: unavailable — {e:#}");
            println!("per-view threads: no, views would stay inline");
            return;
        }
    };

    let outcome = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                use glutin::context::NotCurrentGlContext as _;
                use glutin::display::{GetGlDisplay as _, GlDisplay as _};

                // Moved in whole, never borrowed — see `OffscreenContext`.
                let window::OffscreenContext { context, surface } = offscreen;
                let context = context
                    .make_current(&surface)
                    .map_err(|e| format!("make current on the view thread: {e}"))?;
                // SAFETY: made current on this thread immediately above, and
                // the loader outlives the scope.
                let view_gl = unsafe {
                    egui_glow::glow::Context::from_loader_function_cstr(|name| {
                        context.display().get_proc_address(name)
                    })
                };
                // SAFETY: current on this thread for every call below.
                unsafe {
                    let version = view_gl.get_parameter_string(egui_glow::glow::VERSION);
                    let texture = view_gl
                        .create_texture()
                        .map_err(|e| format!("create a texture on the view thread: {e}"))?;
                    view_gl.bind_texture(egui_glow::glow::TEXTURE_2D, Some(texture));
                    view_gl.tex_image_2d(
                        egui_glow::glow::TEXTURE_2D,
                        0,
                        egui_glow::glow::RGBA8.cast_signed(),
                        PROBE_TEXTURE_SIZE,
                        PROBE_TEXTURE_SIZE,
                        0,
                        egui_glow::glow::RGBA,
                        egui_glow::glow::UNSIGNED_BYTE,
                        egui_glow::glow::PixelUnpackData::Slice(None),
                    );
                    // The compositor reads this texture back, so the allocation
                    // has to have landed before the probe hands it over.
                    view_gl.finish();
                    Ok((version, texture))
                }
            })
            .join()
            .unwrap_or_else(|_| Err("the view thread panicked".to_owned()))
    });

    let (version, texture) = match outcome {
        Ok(outcome) => outcome,
        Err(e) => {
            println!("shared context: {e}");
            println!("per-view threads: no, views would stay inline");
            return;
        }
    };
    println!("view GL: {version}");

    // The share group is the whole premise: a view renders into its own texture
    // and the compositor samples it by name. Reading the size back proves the
    // name resolves to that allocation here, not merely that the number crossed.
    // SAFETY: the window context is current on this thread.
    let shared_size = unsafe {
        gl.bind_texture(egui_glow::glow::TEXTURE_2D, Some(texture));
        let width = gl.get_tex_level_parameter_i32(egui_glow::glow::TEXTURE_2D, 0, TEXTURE_WIDTH);
        gl.bind_texture(egui_glow::glow::TEXTURE_2D, None);
        gl.delete_texture(texture);
        width
    };
    if shared_size == PROBE_TEXTURE_SIZE {
        println!("texture sharing: ok");
        println!("frame handoff: {:?}", view::gpu_wait_for_version(&version));
        println!("per-view threads: yes");
    } else {
        println!("texture sharing: the view's texture reads back as {shared_size}px here");
        println!("per-view threads: no, views would stay inline");
    }
}

struct TestbedHandler {
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    /// Taken by the first `resumed`; a later one finds the window already built.
    seed: Option<AppSeed>,
    window: Option<window::GlWindow>,
    egui_glow: Option<egui_glow::EguiGlow>,
    app: Option<TestbedApp>,
    /// How long until egui wants the next frame; `MAX` means "only on input".
    repaint_after: std::time::Duration,
    /// When the last frame began, which is what `repaint_after` counts from.
    frame_started_at: Option<std::time::Instant>,
    /// Reported by `main` once the loop returns, since a handler cannot fail outward.
    fatal_error: Option<anyhow::Error>,
}

impl TestbedHandler {
    /// Build the window, GL context, egui integration and app.
    fn start(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) -> Result<()> {
        let seed = self
            .seed
            .take()
            .ok_or_else(|| anyhow::anyhow!("BUG: resumed twice with no window"))?;

        let (window, gl, get_proc) =
            window::GlWindow::new(event_loop, seed.inner_size, "WASM Widget Testbed")?;
        let gl = Arc::new(gl);

        if seed.cli.check_shared_gl {
            report_shared_gl(&window, &gl);
            event_loop.exit();
            return Ok(());
        }

        let egui_glow = egui_glow::EguiGlow::new(
            event_loop,
            Arc::clone(&gl),
            None,
            None,
            /* dithering */ true,
        );
        let proxy = egui::mutex::Mutex::new(self.proxy.clone());
        egui_glow
            .egui_ctx
            .set_request_repaint_callback(move |info| {
                // A send failure means the loop is already gone,
                // so there is nothing left to repaint.
                drop(proxy.lock().send_event(UserEvent::Redraw(info.delay)));
            });

        let app = TestbedApp::new(
            gl,
            get_proc,
            seed.cli,
            seed.manifest,
            seed.params,
            seed.platform,
            seed.record_request,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;

        // The window stays hidden until something paints it,
        // and nothing has asked for a frame yet.
        window.window().request_redraw();

        self.window = Some(window);
        self.egui_glow = Some(egui_glow);
        self.app = Some(app);
        log_startup_memory(seed.rss_before_gl);
        Ok(())
    }

    /// One full frame: widget FBOs, the egui pass, then present.
    fn redraw(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.frame_started_at = Some(std::time::Instant::now());

        let (Some(window), Some(egui_glow), Some(app)) = (
            self.window.as_ref(),
            self.egui_glow.as_mut(),
            self.app.as_mut(),
        ) else {
            return;
        };

        // Registering a texture and painting with it both want the painter,
        // so views are retired and (re)built here rather than in the pass below.
        app.teardown.drain(&app.gl, &mut egui_glow.painter);
        if let Err(e) = app.ensure_views(&mut egui_glow.painter, window) {
            self.fatal_error = Some(e.context("failed to build views"));
            event_loop.exit();
            return;
        }

        app.poll_hot_reload();
        app.drive_views();
        if app.exit_requested {
            event_loop.exit();
            return;
        }

        egui_glow.run(window.window(), |ui| app.ui(ui));

        // SAFETY: the context is current on this thread for the window's lifetime.
        unsafe {
            use egui_glow::glow::HasContext as _;
            app.gl.clear_color(0.0, 0.0, 0.0, 1.0);
            app.gl.clear(egui_glow::glow::COLOR_BUFFER_BIT);
        }
        egui_glow.paint(window.window());

        if let Err(e) = window.swap_buffers() {
            tracing::error!("present failed: {e:#}");
        }
        window.show();
    }
}

impl winit::application::ApplicationHandler<UserEvent> for TestbedHandler {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(e) = self.start(event_loop) {
            self.fatal_error = Some(e);
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        use winit::event::WindowEvent;

        // Closing early still seals the perf report from whatever was collected,
        // so an operator who did not wait for `--perf-frames` keeps the run.
        if matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed) {
            if let Some(app) = self.app.as_mut() {
                app.finish_perf_report();
            }
            event_loop.exit();
            return;
        }
        if matches!(event, WindowEvent::RedrawRequested) {
            self.redraw(event_loop);
            return;
        }
        if let WindowEvent::Resized(size) = &event
            && let Some(window) = self.window.as_ref()
        {
            window.resize(*size);
        }

        // Everything else is input, and egui decides whether it changed
        // anything worth drawing.
        let (Some(window), Some(egui_glow)) = (self.window.as_ref(), self.egui_glow.as_mut())
        else {
            return;
        };
        if egui_glow.on_window_event(window.window(), &event).repaint {
            window.window().request_redraw();
        }
    }

    /// Turn an expired `WaitUntil` deadline into a frame.
    ///
    /// Waking at the deadline is all winit does; without asking for the redraw
    /// here, the only thing left driving frames is input, and every animation
    /// stalls the moment the pointer stops moving.
    fn new_events(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        if matches!(cause, winit::event::StartCause::ResumeTimeReached { .. })
            && let Some(window) = self.window.as_ref()
        {
            window.window().request_redraw();
        }
    }

    fn user_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, event: UserEvent) {
        let UserEvent::Redraw(delay) = event;
        self.repaint_after = delay;
    }

    /// Release GL objects while their context is still current and alive.
    ///
    /// Views own framebuffers and their own femtovg contexts,
    /// and the painter owns egui's textures.
    /// In field order every one of them outlives the window,
    /// so leaving this to `Drop` frees them against a dead context.
    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let (Some(app), Some(egui_glow)) = (self.app.as_mut(), self.egui_glow.as_mut()) {
            let gl = Arc::clone(&app.gl);
            let live = std::mem::take(&mut app.tiles);
            app.teardown.views.extend(live);
            app.teardown.finish(&gl, &mut egui_glow.painter);
        }
        drop(self.app.take());
        if let Some(mut egui_glow) = self.egui_glow.take() {
            egui_glow.destroy();
        }
        drop(self.window.take());
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        // The delay counts from the frame that asked for it, not from now.
        // vsync holds `swap_buffers` for most of a frame interval and this runs
        // after that, so measuring from here would sleep the requested delay
        // on top of the wait already served, landing between two vblanks.
        let deadline = self
            .frame_started_at
            .and_then(|start| start.checked_add(self.repaint_after));
        event_loop.set_control_flow(match deadline {
            // Already due, so the next frame is owed immediately; vsync is what
            // paces it from here.
            Some(deadline) if deadline <= std::time::Instant::now() => {
                window.window().request_redraw();
                winit::event_loop::ControlFlow::Poll
            }
            Some(deadline) => winit::event_loop::ControlFlow::WaitUntil(deadline),
            // Nothing pending: sleep until input arrives.
            None => winit::event_loop::ControlFlow::Wait,
        });
    }
}

impl TestbedApp {
    fn ui(&mut self, root_ui: &mut egui::Ui) {
        let palette = self.theme.palette(root_ui.ctx());
        theme::apply(root_ui.ctx(), palette);

        let ctx = root_ui.ctx().clone();
        // The views already ran for this frame, outside the pass; this paints
        // what they produced.
        let FramePass { now, next_wake_ms } = self.pass;
        let time_s = now.duration_since(self.clock.start_instant).as_secs_f32();

        // Chrome claims its edges before the CentralPanel takes the rest:
        // toolbar first, then the sidebar's 320 px slice off the right.
        // Sidebar changes propagate to all tile runtimes and (when recording)
        // append `ParamDelivery` / `SystemDelivery` events to the timeline.
        self.paint_toolbar(root_ui);
        self.paint_status_bar(root_ui);
        self.paint_right_panel(root_ui);
        self.paint_recording_sidebar(root_ui);

        // The canvas: a pannable backdrop the device windows float over.
        // Dragging any empty spot moves every canvas window in step.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show_inside(root_ui, |ui| {
                self.canvas.rect = ui.max_rect();
                draw_checkerboard(ui.painter(), ui.max_rect(), palette);
                let background = ui.interact(
                    ui.max_rect(),
                    ui.id().with("canvas"),
                    egui::Sense::click_and_drag(),
                );
                if background.dragged() {
                    self.canvas.pan_by(background.drag_delta());
                    background.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if background.hovered() {
                    // Announce that dragging here pans, before it happens.
                    background.ctx.set_cursor_icon(egui::CursorIcon::Grab);
                }
            });

        self.paint_device_windows(&ctx, time_s);
        self.paint_notice(&ctx);
        // The overlay's click, deferred here: it lands inside a borrow of the
        // choosing state, and entering the take swaps the whole canvas.
        if let Some(target) = self.recording_mode.take_choice() {
            self.enter_recording(target, default_dataset(target));
            ctx.request_repaint();
        }

        // Earliest tile deadline, capped at the drain tick for deliveries + chrome.
        let repaint_ms = next_wake_ms.map_or(DRAIN_TICK_MS, |w| w.min(DRAIN_TICK_MS));
        ctx.request_repaint_after(std::time::Duration::from_millis(repaint_ms));
    }

    /// The saved-a-take banner, floating over the top of the canvas.
    ///
    /// A take leaves nothing else behind — the sidebar, the pinned platform and
    /// the log all go with the unwind — so the write's result is stated here:
    /// where the fixture landed, how much it holds, and what to run next.
    fn paint_notice(&mut self, ctx: &egui::Context) {
        const OK: egui::Color32 = egui::Color32::from_rgb(120, 200, 150);

        let Some(notice) = &self.notice else { return };
        // An error names something to go and fix, so it holds until dismissed;
        // only the success banner is on a timer.
        if !notice.error && notice.shown_at.elapsed() > Notice::TTL {
            self.notice = None;
            return;
        }
        let error = notice.error;
        let text = notice.text.clone();

        let palette = self.theme.palette(ctx);
        let accent = if error { palette.record_accent } else { OK };
        let (icon, heading) = if error {
            (&mut self.icons.warning, "Recording failed")
        } else {
            (&mut self.icons.saved, "Recording saved")
        };

        let mut dismissed = false;
        egui::Area::new(egui::Id::new("testbed_notice"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(
                self.canvas.rect.center().x - NOTICE_W / 2.0,
                self.canvas.rect.min.y + 16.0,
            ))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(palette.panel_fill)
                    .stroke(egui::Stroke::new(1.0_f32, accent))
                    .corner_radius(4.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.set_width(NOTICE_W);
                        ui.horizontal(|row| {
                            row.spacing_mut().item_spacing.x = 8.0;
                            let icon_rect = row
                                .allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover())
                                .0;
                            icon.paint(row, icon_rect, accent);
                            row.label(egui::RichText::new(heading).color(accent).strong());
                        });
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(text).font(egui::FontId::monospace(11.0)));
                    });
                dismissed = ui
                    .interact(ui.min_rect(), ui.id().with("dismiss"), egui::Sense::click())
                    .on_hover_text("dismiss")
                    .clicked();
            });
        if dismissed {
            self.notice = None;
        }
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;

    fn platform(id: &str) -> &'static Platform {
        platform_catalog::platform(id)
            .unwrap_or_else(|| panic!("BUG: '{id}' must be in the catalog"))
    }

    fn record_args(target: &str, name: Option<&str>) -> CliArgs {
        let mut args = vec![
            "testbed".to_owned(),
            "widget.wasm".to_owned(),
            format!("--record={target}"),
        ];
        if let Some(name) = name {
            args.push(format!("--record-name={name}"));
        }
        CliArgs::try_parse_from(args).expect("BUG: record args must parse")
    }

    #[test]
    fn every_bmc100_viewport_is_recordable() {
        let p = platform("bmc100");
        for v in p.viewports {
            let spec = format!("bmc100:{}", v.id);
            let cli = record_args(&spec, None);
            let request = resolve_record_request(&cli)
                .unwrap_or_else(|e| panic!("BUG: '{spec}' must resolve: {e:#}"))
                .expect("BUG: a --record target must be Some");
            assert_eq!(request.target.viewport.id, v.id);
            assert_eq!(request.dataset, format!("bmc100-{}", v.id));
        }
    }

    #[test]
    fn an_unknown_record_target_is_rejected_not_defaulted() {
        let err = resolve_record_request(&record_args("bmc100:SIZE=small", None))
            .expect_err("BUG: an unknown viewport must be rejected, not defaulted to full");
        assert!(format!("{err:#}").contains("SIZE=small"), "{err:#}");

        let err = resolve_record_request(&record_args("bmm100:small", None))
            .expect_err("BUG: BMM100 has no Small viewport");
        assert!(format!("{err:#}").contains("small"), "{err:#}");
    }

    #[test]
    fn a_record_target_contradicting_platform_is_rejected() {
        let mut cli = record_args("bfm100:full", None);
        cli.platform_id = Some("bmc100".to_owned());

        let err = resolve_record_request(&cli)
            .expect_err("BUG: a --platform naming another device must be rejected");
        let message = format!("{err:#}");
        assert!(
            message.contains("bfm100") && message.contains("bmc100"),
            "{message}"
        );
    }

    #[test]
    fn an_explicit_dataset_name_overrides_the_default() {
        let request = resolve_record_request(&record_args("bfm100:full", Some("night-shift")))
            .expect("BUG: a named recording must resolve")
            .expect("BUG: a --record target must be Some");

        assert_eq!(request.dataset, "night-shift");
    }

    #[test]
    fn a_path_like_dataset_name_is_rejected() {
        for bad in ["../escape", "a/b", "with space", ""] {
            let err = resolve_record_request(&record_args("bfm100:full", Some(bad)))
                .expect_err("BUG: a dataset name must not be path-like");
            assert!(format!("{err:#}").contains("record-name"), "{err:#}");
        }
    }

    #[test]
    fn bfm100_tile_carries_round_shape() {
        let p = platform("bfm100");
        let tile = PlacedTile::for_viewport(p, &p.viewports[0]);
        assert_eq!((tile.w, tile.h), (480, 480));
        assert!(matches!(tile.shape, DisplayShape::Round));
    }

    #[test]
    fn bmc100_fullscreen_tile_preserves_legacy_kv_key() {
        let p = platform("bmc100");
        let keys: Vec<String> = p
            .viewports
            .iter()
            .map(|v| PlacedTile::for_viewport(p, v).kv_key)
            .collect();
        assert_eq!(keys, ["full", "large", "medium", "small"]);
    }

    #[test]
    fn stripless_platforms_get_stripless_tiles() {
        let p = platform("bmm100");
        let tile = PlacedTile::for_viewport(p, &p.viewports[0]);
        assert_eq!(tile.led_count, None);
    }

    #[test]
    fn toggling_opens_a_platform_and_toggling_again_closes_it() {
        let mut open = vec![platform("bmc100")];

        assert!(toggle_open(&mut open, platform("bmm101")), "opens");
        assert_eq!(open.len(), 2, "both devices stay open together");

        assert!(!toggle_open(&mut open, platform("bmm101")), "closes");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, "bmc100", "the other device is untouched");
    }

    #[test]
    fn recording_blocks_platform_switching() {
        assert_eq!(
            can_switch_platform(false),
            Ok(()),
            "idle testbed should allow platform switches",
        );
        assert_eq!(
            can_switch_platform(true),
            Err("recording is active"),
            "recording must keep active tile indexes and runtimes intact",
        );
    }

    #[test]
    fn a_record_target_names_the_platform_it_opens() {
        let request = resolve_record_request(&record_args("bfm100:full", None))
            .expect("BUG: a round target must resolve")
            .expect("BUG: a --record target must be Some");

        assert_eq!(
            request.target.platform.id, "bfm100",
            "the target decides which platform the testbed opens",
        );
        assert_eq!(request.target.viewport.shape, DisplayShape::Round);
    }

    #[test]
    fn bfm100_runtime_geometry_is_round_for_viewport_and_display() {
        let p = platform("bfm100");

        let geometry = RuntimeTileGeometry::for_viewport_shape(p, p.viewports[0].shape);
        assert_eq!(
            geometry.viewport_shape,
            bmc_wasm_protocol::ViewportShape::Round
        );
        assert_eq!(
            geometry.display.shape,
            bmc_wasm_protocol::DisplayShape::Round
        );
        assert_eq!(
            (geometry.display.width, geometry.display.height),
            (480, 480)
        );
        assert_eq!(geometry.display.dpi, 229);
    }

    #[test]
    fn bmm101_runtime_geometry_reports_selected_display_resolution() {
        let p = platform("bmm101");

        let geometry = RuntimeTileGeometry::for_viewport_shape(p, p.viewports[0].shape);
        assert_eq!(
            geometry.viewport_shape,
            bmc_wasm_protocol::ViewportShape::Rectangular
        );
        assert_eq!(
            geometry.display.shape,
            bmc_wasm_protocol::DisplayShape::Rectangular
        );
        assert_eq!(
            (geometry.display.width, geometry.display.height),
            (480, 320)
        );
    }

    #[test]
    fn bmc100_tile_geometry_keeps_tile_viewport_and_platform_display_separate() {
        let p = platform("bmc100");
        let medium = p
            .viewports
            .iter()
            .find(|v| v.label == "Medium")
            .expect("BUG: BMC100 medium viewport must exist");

        let geometry = RuntimeTileGeometry::for_viewport_shape(p, medium.shape);

        assert_eq!(
            geometry.viewport_shape,
            bmc_wasm_protocol::ViewportShape::Rectangular
        );
        assert_eq!(
            geometry.display.shape,
            bmc_wasm_protocol::DisplayShape::Rectangular
        );
        assert_eq!(
            (geometry.display.width, geometry.display.height),
            (1280, 480)
        );
        assert_eq!((medium.width, medium.height), (638, 238));
    }
}
