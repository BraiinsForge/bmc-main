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

mod credentials_ui;
mod paint;
mod params_ui;
mod platforms;
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
use bmc_wasm_runtime::fixtures::{
    self, PreparedWidget, find_widget_root, seed_kv_from_widget_root, snapshot_kv_dir,
};
use bmc_wasm_runtime::unified_fixture::TimelineEvent;
use bmc_wasm_runtime::{
    DiskCache, LedEffect, LedRequest, PackageAssetStore, RenderStatus, RuntimeConfig,
    SystemSnapshot, WasmWidgetRuntime,
};
use clap::Parser;

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

// Stats panel position: empty area right of SMALL tile, below MEDIUM
const STATS_X: u32 = RIGHT_COL_X + TILE_SMALL_W + G;
const STATS_Y: u32 = ROW1_Y + row_stride(TILE_MEDIUM_H);
const STATS_W: u32 = PREVIEW_WIDTH - M - STATS_X;
const STATS_H: u32 = PREVIEW_HEIGHT - M - STATS_Y;

/// Minimum width of the stats panel anchored below the preview area, so the
/// FPS table and timing chart stay legible on narrow single-viewport platforms.
const STATS_MIN_W: u32 = 360;
/// Fixed height reserved for the stats panel below single-viewport and generic
/// flow tiles. BMC100 keeps its own preserved stats rectangle.
const SINGLE_STATS_H: u32 = 220;

/// One placed preview in the testbed window, in logical pixels.
pub(crate) struct PlacedTile {
    pub(crate) label: String,
    pub(crate) kv_key: String,
    pub(crate) shape: platforms::DisplayShape,
    pub(crate) led_count: Option<u32>,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) w: u32,
    pub(crate) h: u32,
}

#[derive(Clone, Copy)]
struct RuntimeTileGeometry {
    viewport_shape: bmc_wasm_protocol::ViewportShape,
    display: bmc_wasm_runtime::RuntimeDisplayInfo,
}

impl RuntimeTileGeometry {
    fn for_viewport_shape(
        platform: &platforms::Platform,
        viewport_shape: platforms::DisplayShape,
    ) -> Self {
        Self {
            viewport_shape: viewport_shape.to_runtime_viewport_shape(),
            display: platform.display.to_runtime_display_info(),
        }
    }
}

/// Runtime tile layout derived from the active platform.
pub(crate) struct TileLayout {
    pub(crate) tiles: Vec<PlacedTile>,
    pub(crate) preview_w: u32,
    pub(crate) preview_h: u32,
    pub(crate) stats_x: u32,
    pub(crate) stats_y: u32,
    pub(crate) stats_w: u32,
    pub(crate) stats_h: u32,
}

impl TileLayout {
    pub(crate) fn for_platform(platform: &platforms::Platform) -> Self {
        if platform.id == "BMC100" {
            return Self::bmc100(platform);
        }
        match platform.widget_viewports.as_slice() {
            [single] => Self::single_viewport(platform, single),
            _ => Self::generic_flow(platform),
        }
    }

    fn bmc100(platform: &platforms::Platform) -> Self {
        let slots: [(u32, u32, u32, u32); 4] = [
            (M, ROW0_Y, TILE_FULL_W, TILE_FULL_H),
            (M, ROW1_Y, TILE_LARGE_W, TILE_LARGE_H),
            (RIGHT_COL_X, ROW1_Y, TILE_MEDIUM_W, TILE_MEDIUM_H),
            (
                RIGHT_COL_X,
                ROW1_Y + row_stride(TILE_MEDIUM_H),
                TILE_SMALL_W,
                TILE_SMALL_H,
            ),
        ];
        let kv_keys = ["full", "large", "medium", "small"];
        let led_count = platform.led_strip.map(|strip| strip.led_count);
        debug_assert_eq!(led_count, Some(LED_COUNT as u32));
        let tiles = platform
            .widget_viewports
            .iter()
            .zip(slots.iter())
            .zip(kv_keys.iter())
            .map(|((v, &(x, y, w, h)), &kv_key)| PlacedTile {
                label: v.label.clone(),
                kv_key: kv_key.to_owned(),
                shape: v.shape,
                led_count,
                x,
                y,
                w,
                h,
            })
            .collect();
        Self {
            tiles,
            preview_w: PREVIEW_WIDTH,
            preview_h: PREVIEW_HEIGHT,
            stats_x: STATS_X,
            stats_y: STATS_Y,
            stats_w: STATS_W,
            stats_h: STATS_H,
        }
    }

    fn single_viewport(platform: &platforms::Platform, v: &platforms::WidgetViewport) -> Self {
        let led_count = platform.led_strip.map(|strip| strip.led_count);
        let led_strip_h = if led_count.is_some() { LED_STRIP_H } else { 0 };
        let tiles = vec![PlacedTile {
            label: v.label.clone(),
            kv_key: v.label.to_ascii_lowercase(),
            shape: v.shape,
            led_count,
            x: PREVIEW_MARGIN,
            y: PREVIEW_MARGIN,
            w: v.width,
            h: v.height,
        }];
        let tiles_bottom = PREVIEW_MARGIN + v.height + led_strip_h + PREVIEW_GAP;
        let stats_x = PREVIEW_MARGIN;
        let stats_y = tiles_bottom;
        let content_w = v.width;
        let stats_w = content_w.max(STATS_MIN_W);
        let stats_h = SINGLE_STATS_H;
        let preview_w = PREVIEW_MARGIN + content_w.max(stats_w) + PREVIEW_MARGIN;
        let preview_h = stats_y + stats_h + PREVIEW_MARGIN;
        Self {
            tiles,
            preview_w,
            preview_h,
            stats_x,
            stats_y,
            stats_w,
            stats_h,
        }
    }

    fn generic_flow(platform: &platforms::Platform) -> Self {
        let row_cap = platform
            .widget_viewports
            .iter()
            .map(|v| v.width)
            .max()
            .unwrap_or(1);

        let mut tiles = Vec::with_capacity(platform.widget_viewports.len());
        let mut cursor_x = PREVIEW_MARGIN;
        let mut row_y = PREVIEW_MARGIN;
        let mut row_h = 0_u32;
        let mut content_w = 0_u32;
        let led_count = platform.led_strip.map(|strip| strip.led_count);
        let led_strip_h = if led_count.is_some() { LED_STRIP_H } else { 0 };

        for v in &platform.widget_viewports {
            let needs_wrap =
                cursor_x > PREVIEW_MARGIN && cursor_x + v.width > PREVIEW_MARGIN + row_cap;
            if needs_wrap {
                row_y += row_h + led_strip_h + PREVIEW_GAP;
                cursor_x = PREVIEW_MARGIN;
                row_h = 0;
            }
            tiles.push(PlacedTile {
                label: v.label.clone(),
                kv_key: v.label.to_ascii_lowercase(),
                shape: v.shape,
                led_count,
                x: cursor_x,
                y: row_y,
                w: v.width,
                h: v.height,
            });
            cursor_x += v.width + PREVIEW_GAP;
            row_h = row_h.max(v.height);
            content_w = content_w.max(cursor_x - PREVIEW_GAP);
        }

        let preview_w = content_w + PREVIEW_MARGIN;
        let preview_tiles_h = row_y + row_h + led_strip_h + PREVIEW_MARGIN;
        let stats_x = PREVIEW_MARGIN;
        let stats_y = preview_tiles_h;
        let stats_w = preview_w
            .saturating_sub(2 * PREVIEW_MARGIN)
            .max(STATS_MIN_W);
        let stats_h = SINGLE_STATS_H;
        let preview_w = preview_w.max(stats_x + stats_w + PREVIEW_MARGIN);
        let preview_h = stats_y + stats_h + PREVIEW_MARGIN;
        Self {
            tiles,
            preview_w,
            preview_h,
            stats_x,
            stats_y,
            stats_w,
            stats_h,
        }
    }
}

fn requested_window_size(layout: &TileLayout) -> egui::Vec2 {
    egui::vec2(
        (layout.preview_w + PARAM_PANEL_W) as f32,
        layout.preview_h as f32,
    )
}

struct SwitchState {
    active_platform_id: String,
    layout: TileLayout,
    requested_size: egui::Vec2,
    needs_tile_rebuild: bool,
}

impl SwitchState {
    fn new(active_platform_id: &str, platform: &platforms::Platform) -> Self {
        let layout = TileLayout::for_platform(platform);
        let requested_size = requested_window_size(&layout);
        Self {
            active_platform_id: active_platform_id.to_owned(),
            layout,
            requested_size,
            needs_tile_rebuild: false,
        }
    }

    fn switch_to(
        &mut self,
        catalog: &platforms::PlatformCatalog,
        target_id: &str,
    ) -> Result<bool, String> {
        if target_id == self.active_platform_id {
            return Ok(false);
        }
        let platform = catalog
            .platform(target_id)
            .ok_or_else(|| format!("platform '{target_id}' not found"))?;
        target_id.clone_into(&mut self.active_platform_id);
        self.layout = TileLayout::for_platform(platform);
        self.requested_size = requested_window_size(&self.layout);
        self.needs_tile_rebuild = true;
        Ok(true)
    }
}

fn validate_recording_target(
    record_size: Option<&str>,
    active_platform_id: &str,
    layout: &TileLayout,
) -> Result<(), String> {
    let Some(size_name) = record_size else {
        return Ok(());
    };
    let Some(active_tile) = record_size_to_idx(size_name) else {
        return Err(format!(
            "unknown record size '{size_name}'; valid sizes are full, large, medium, small"
        ));
    };
    if active_tile >= layout.tiles.len() {
        return Err(format!(
            "record size '{size_name}' is not available on platform '{active_platform_id}' \
             with {} tile(s)",
            layout.tiles.len()
        ));
    }
    Ok(())
}

/// Width of the right-side sidebar housing both the per-widget Params
/// section (when the manifest declares any) and the deck-wide System
/// section (always shown). Added to the window's outer size so the tile
/// area stays at native dimensions instead of getting squeezed.
pub(crate) const PARAM_PANEL_W: u32 = 320;

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
    /// Record a capture fixture for this viewport size (e.g. `small`).
    #[arg(long = "record")]
    record_size: Option<String>,
    /// Platform catalog JSON; the built-in catalog otherwise.
    #[arg(long = "platform-catalog")]
    platform_catalog_path: Option<PathBuf>,
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

    fn write_test_catalog() -> PathBuf {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!(
            "testbed-platform-catalog-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{
              "default_platform": "TEST",
              "platforms": [
                {
                  "id": "TEST",
                  "label": "Test Platform",
                  "display": { "width": 111, "height": 222, "shape": "rectangular", "dpi": 1 },
                  "slot_grid": null,
                  "led_strip": null,
                  "widget_viewports": [
                    { "label": "Fullscreen", "placement": { "fullscreen": {} }, "shape": "rectangular", "width": 111, "height": 222 }
                  ]
                }
              ]
            }"#,
        )
        .expect("BUG: test catalog must be writable");
        path
    }

    fn write_invalid_test_catalog() -> PathBuf {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!(
            "testbed-invalid-platform-catalog-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"{ "default_platform": "TEST", "platforms": [] }"#)
            .expect("BUG: invalid test catalog must be writable");
        path
    }

    #[test]
    fn parse_args_accepts_platform_equals_forms() {
        let cli = parse_test_args(&[
            "testbed",
            "widget.wasm",
            "--platform-catalog=catalog.json",
            "--platform=BMM100",
        ])
        .expect("BUG: platform equals args must parse");

        assert_eq!(cli.wasm_path, PathBuf::from("widget.wasm"));
        assert_eq!(
            cli.platform_catalog_path,
            Some(PathBuf::from("catalog.json"))
        );
        assert_eq!(cli.platform_id.as_deref(), Some("BMM100"));
    }

    #[test]
    fn parse_args_accepts_platform_space_forms() {
        let cli = parse_test_args(&[
            "testbed",
            "widget.wasm",
            "--platform-catalog",
            "catalog.json",
            "--platform",
            "BMM101",
        ])
        .expect("BUG: platform space args must parse");

        assert_eq!(
            cli.platform_catalog_path,
            Some(PathBuf::from("catalog.json"))
        );
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
            "small",
        ])
        .expect("BUG: legacy split args must parse");

        assert_eq!(cli.manifest_path, Some(PathBuf::from("manifest.json")));
        assert_eq!(cli.perf_report_path, Some(PathBuf::from("perf.json")));
        assert_eq!(cli.perf_frames, 42);
        assert_eq!(cli.record_size.as_deref(), Some("small"));
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
    fn load_catalog_and_platform_uses_bundled_default() {
        let cli =
            parse_test_args(&["testbed", "widget.wasm"]).expect("BUG: minimal args must parse");

        let (_catalog, selected_id) =
            load_catalog_and_platform(&cli).expect("BUG: bundled default platform must load");

        assert_eq!(selected_id, "BMC100");
    }

    #[test]
    fn load_catalog_and_platform_uses_requested_platform() {
        let cli = parse_test_args(&["testbed", "widget.wasm", "--platform", "BFM100"])
            .expect("BUG: platform arg must parse");

        let (_catalog, selected_id) =
            load_catalog_and_platform(&cli).expect("BUG: requested platform must load");

        assert_eq!(selected_id, "BFM100");
    }

    #[test]
    fn load_catalog_and_platform_reads_requested_catalog_path() {
        let catalog_path = write_test_catalog();
        let cli = parse_test_args(&[
            "testbed",
            "widget.wasm",
            "--platform-catalog",
            catalog_path.to_str().expect("BUG: temp path must be UTF-8"),
        ])
        .expect("BUG: catalog path arg must parse");

        let (catalog, selected_id) =
            load_catalog_and_platform(&cli).expect("BUG: catalog file must load");

        assert_eq!(selected_id, "TEST");
        let platform = catalog
            .select(Some(&selected_id))
            .expect("BUG: selected test platform must exist");
        assert_eq!(platform.display.width, 111);
        let _ = std::fs::remove_file(catalog_path);
    }

    #[test]
    fn load_catalog_and_platform_reports_parse_path_context() {
        let catalog_path = write_invalid_test_catalog();
        let cli = parse_test_args(&[
            "testbed",
            "widget.wasm",
            "--platform-catalog",
            catalog_path.to_str().expect("BUG: temp path must be UTF-8"),
        ])
        .expect("BUG: catalog path arg must parse");

        let err = load_catalog_and_platform(&cli)
            .expect_err("BUG: invalid catalog file must fail with context");
        let err = format!("{err:#}");

        assert!(err.contains("failed to parse"), "{err}");
        assert!(err.contains(&catalog_path.display().to_string()), "{err}");
        let _ = std::fs::remove_file(catalog_path);
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

fn load_catalog_and_platform(cli: &CliArgs) -> Result<(platforms::PlatformCatalog, String)> {
    let catalog = if let Some(path) = cli.platform_catalog_path.as_ref() {
        let body = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        platforms::PlatformCatalog::parse(&body)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("failed to parse {}", path.display()))?
    } else {
        platforms::PlatformCatalog::bundled().map_err(anyhow::Error::msg)?
    };
    let selected_id = catalog
        .select(cli.platform_id.as_deref())
        .map_err(anyhow::Error::msg)?
        .id
        .clone();
    Ok((catalog, selected_id))
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

    let cli = CliArgs::parse();
    let (manifest_path, manifest) = load_manifest(
        &cli.wasm_path,
        cli.manifest_path.clone(),
        cli.resolved_widget_root(),
    )?;
    let params = bmc_wasm_runtime::manifest_default_params(&manifest);
    let (catalog, active_platform_id) = load_catalog_and_platform(&cli)?;
    let selected_platform = catalog
        .select(Some(&active_platform_id))
        .map_err(anyhow::Error::msg)?;
    let startup_layout = TileLayout::for_platform(selected_platform);
    let startup_size = requested_window_size(&startup_layout);

    println!("Loading widget from: {}", cli.wasm_path.display());
    println!("Manifest:            {}", manifest_path.display());
    println!(
        "Params:              {} key(s) from manifest defaults",
        params.len()
    );
    println!(
        "Platform: {} ({}) — display {}x{} {:?} dpi={}, {} viewport(s)",
        active_platform_id,
        selected_platform.label,
        selected_platform.display.width,
        selected_platform.display.height,
        selected_platform.display.shape,
        selected_platform.display.dpi,
        selected_platform.widget_viewports.len()
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(startup_size)
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
            let app = TestbedApp::new(cc, cli, manifest, params, catalog, active_platform_id)?;
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
) -> bool {
    // Mirror the device host: a widget that doesn't export `on_touch` is
    // non-interactive, so it never receives touch events.
    if !runtime.exports_on_touch() {
        return false;
    }
    // Carry the recording reborrow through each branch by hand instead of `as_deref_mut`
    // (which clippy rejects since the `Option`'s inner type is already a `&mut`).
    let mut rec = recording;
    let mut touched = false;
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (x, y) = (pos.x - rect.min.x, pos.y - rect.min.y);
        runtime.push_touch_event(TouchEvent::Down { x, y });
        runtime.push_touch_event(TouchEvent::Up);
        touched = true;
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
        touched = true;
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
        touched = true;
        if let Some(r) = rec.as_mut()
            && let Some(g) = r.gesture.as_mut()
        {
            g.current_pos = (x, y);
        }
    }
    if response.drag_stopped() {
        runtime.push_touch_event(TouchEvent::Up);
        touched = true;
        if let Some(r) = rec.as_mut()
            && let Some(gesture) = r.gesture.take()
        {
            classify_and_record_gesture(r, &gesture);
        }
    }
    if touched {
        // Fire `on_touch` once for the gesture, mirroring the host's per-drain
        // delivery; the caller arms `pending_interaction` so the reaction renders.
        runtime.deliver_touch();
    }
    touched
}

// ── Tile ────────────────────────────────────────────────────────────

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent per-tile status flags (dead/rendered/touch/led), not a state machine"
)]
pub(crate) struct PreviewTile {
    /// `None` for a placeholder — a size the manifest declines: no runtime built
    /// (no live widget, no discovery), painted as a "not supported" slab.
    pub(crate) runtime: Option<WasmWidgetRuntime>,
    /// Caller-owned renderer drawn alongside `runtime`. Bracket each
    /// `runtime.render(...)` call with `runtime.with_renderer(ptr, ...)`.
    pub(crate) renderer: FemtoVgRenderer,
    pub(crate) gpu: TileGpu,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) shape: platforms::DisplayShape,
    label: String,
    dead: bool,
    ever_rendered: bool,
    /// Monotonic-ms deadline for this tile's next WASM render,
    /// armed from `next_frame_delay()` at each render.
    ///
    /// `None` = idle until a delivery or touch.
    /// Absolute, not per-tick relative, so it fires rather than receding.
    next_render_at_ms: Option<u64>,
    /// A touch landed since the last render; forces the next tick to render.
    pending_interaction: bool,
    pub(crate) led_count: Option<u32>,
    /// Receiver for LED requests from the widget (drained each frame).
    led_rx: Option<std::sync::mpsc::Receiver<LedRequest>>,
    /// Current LED scene (from last `SetEffect` request).
    pub(crate) led_scene: Option<bmc_led::data::LedScene>,
    /// Whether LEDs are enabled.
    pub(crate) led_enabled: bool,
}

enum DeliveryPollOutcome {
    Ready { immediate: bool },
    Trapped(anyhow::Error),
}

fn delivery_poll_outcome(
    result: Result<()>,
    next_frame_is_immediate: impl FnOnce() -> bool,
) -> DeliveryPollOutcome {
    match result {
        Ok(()) => DeliveryPollOutcome::Ready {
            immediate: next_frame_is_immediate(),
        },
        Err(error) => DeliveryPollOutcome::Trapped(error),
    }
}

impl PreviewTile {
    /// Drain pending LED requests; update `led_scene` / `led_enabled`.
    fn drain_led_commands(&mut self) {
        let Some(led_rx) = self.led_rx.as_ref() else {
            return;
        };
        while let Ok(req) = led_rx.try_recv() {
            match req {
                LedRequest::SetEffect {
                    effect,
                    color,
                    period_ms,
                    duration,
                    ..
                } => {
                    let hw_effect = match effect {
                        LedEffect::Chase => bmc_led::data::LedEffect::Chase(color),
                        LedEffect::KnightRider => bmc_led::data::LedEffect::KnightRider(color),
                        LedEffect::Scan => bmc_led::data::LedEffect::Scan(color),
                        LedEffect::Snake => bmc_led::data::LedEffect::Snake(color),
                        LedEffect::Breathe => bmc_led::data::LedEffect::Breathe(color),
                        LedEffect::Solid => bmc_led::data::LedEffect::Solid(color),
                    };
                    self.led_scene = Some(bmc_led::data::LedScene {
                        effect: hw_effect,
                        period: (period_ms > 0)
                            .then(|| std::time::Duration::from_millis(u64::from(period_ms))),
                        duration,
                    });
                    self.led_enabled = true;
                }
                LedRequest::Stop { .. } => {
                    self.led_scene = None;
                    self.led_enabled = false;
                }
            }
        }
    }

    fn into_pooled_gpu(mut self, gl: &eframe::glow::Context) -> TileGpu {
        self.renderer.drop_all();
        self.gpu.detach_render_target(gl);
        self.gpu
    }
}

// ── App ─────────────────────────────────────────────────────────────

pub(crate) struct TestbedApp {
    cli: CliArgs,
    prepared_widget: PreparedWidget,
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
    /// Current credential view, in the wire JSON shape.
    /// Mutated by the Credentials sidebar section; tile runtimes
    /// are kept in sync via `deliver_credentials_update`.
    pub(crate) credentials: serde_json::Map<String, serde_json::Value>,
    /// Secrets from `--secrets`, handed to the runtime with each credential
    /// delivery; empty by default, so fetches refuse before egress.
    pub(crate) secrets: bmc_widget_protocol::CredentialSecrets,
    /// Base-URL rewrites from `--rewrite-url`, installed on every runtime.
    url_rewrites: Vec<(String, String)>,
    gl: Arc<eframe::glow::Context>,
    pub(crate) tiles: Vec<PreviewTile>,
    gpu_pool: Vec<TileGpu>,
    clock: Clock,
    /// Offline toggle: seals every tile's live I/O so refreshes fail.
    offline: bool,
    hot_reload: HotReload,
    perf: PerfState,
    pub(crate) recording_mode: RecordingMode,
    /// Active platform catalog, kept for the runtime selector.
    pub(crate) catalog: platforms::PlatformCatalog,
    /// Id of the currently previewed platform.
    pub(crate) active_platform_id: String,
    /// Layout derived from the active platform's widget viewports.
    pub(crate) layout: TileLayout,
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
    /// Set by the "Reload WASM" button in the stats panel; consumed as a synthetic watcher
    /// event on the next `poll_hot_reload` tick.
    manual_reload: bool,
}

/// Per-frame performance accounting. The rolling window drives the FPS readout
/// in the stats panel; the full vector is what `--perf-report=` writes to disk at exit.
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

/// Recording-mode bundle: the optional in-flight recording state plus
/// the shared fetch buffer the active tile's fetch observer pushes into.
pub(crate) struct RecordingMode {
    /// `Some` only when started via `--record=<size>`;
    /// `None` resets it after Save/Cancel.
    pub(crate) state: Option<RecordingState>,
    /// Shared buffer for fetch events captured by the active tile's fetch observer.
    /// Held behind `Arc<Mutex<_>>` because the observer runs on background fetch threads.
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
        catalog: platforms::PlatformCatalog,
        active_platform_id: String,
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
        let prepared_widget = PreparedWidget::new(&cli.wasm_path, cli.asset_root.as_deref())?;
        let platform = catalog
            .platform(&active_platform_id)
            .ok_or("BUG: selected platform id must exist in catalog")?;
        let layout = TileLayout::for_platform(platform);
        validate_recording_target(cli.record_size.as_deref(), &active_platform_id, &layout)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        let requested_size = requested_window_size(&layout);

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

        let recording_state = cli.record_size.as_ref().map(|size_name| {
            let active_tile = record_size_to_idx(size_name)
                .expect("BUG: record size already validated by validate_recording_target");
            let widget_root = cli.resolved_widget_root();
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
                // Nothing is bound until the operator binds it in the sidebar.
                credentials_snapshot: serde_json::Map::new(),
                start_time_iso,
                auto_capture: true,
            }
        });

        let now = std::time::Instant::now();
        let declared = declared_slots(&manifest);
        let secrets = cli.credential_secrets(&declared)?;
        let url_rewrites = cli.url_rewrites()?;
        Ok(Self {
            cli,
            prepared_widget,
            secrets,
            url_rewrites,
            requested_size,
            // 30 attempts at ~16ms = ~0.5 s of negotiation.
            // More than enough for any compositor to settle,
            // far less than long enough to feel like the UI froze.
            size_pin_attempts: 30,
            manifest,
            params,
            system: pending_system,
            credentials: serde_json::Map::new(),
            gl,
            tiles: Vec::new(),
            gpu_pool: Vec::new(),
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
            recording_mode: RecordingMode {
                state: recording_state,
                fetch_events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            },
            catalog,
            active_platform_id,
            layout,
        })
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

    /// Drain pending watcher events; if any fired, rebuild every tile's `WasmWidgetRuntime`
    /// (and the host-owned `TileGpu`; see [`Self::reload_one_tile`] for why) from the
    /// (now-updated) wasm bytes on disk.
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
        tracing::info!(
            wasm_bytes = wasm_bytes.len(),
            tiles = self.tiles.len(),
            "hot reload: rebuilding tile runtime(s)"
        );
        let Some(platform) = self.catalog.platform(&self.active_platform_id) else {
            tracing::error!(
                "hot reload: active platform '{}' not in catalog",
                self.active_platform_id
            );
            return;
        };
        let params = self.params.clone();
        let system = self.system.clone();
        // A rebuilt runtime starts with nothing bound, so the sidebar's bindings
        // are re-delivered below — without that, a hot reload drops
        // a credential-fed widget back to its unbound state.
        let credentials = bmc_wasm_runtime::parse_credentials_json(&self.credentials);
        let secrets = self.secrets.clone();
        let mut replacements = Vec::new();
        for idx in 0..self.tiles.len() {
            let placed_shape = self.layout.tiles[idx].shape;
            let tile = &self.tiles[idx];
            if tile.runtime.is_none() {
                continue;
            }
            let (led_tx, led_rx) = if tile.led_count.is_some() {
                let (led_tx, led_rx) = std::sync::mpsc::channel();
                (Some(led_tx), Some(led_rx))
            } else {
                (None, None)
            };
            let rt_config = RuntimeConfig {
                params: params.clone(),
                system: system.clone(),
                led_request_sender: led_tx,
                asset_cache: self.cli.asset_cache(),
                package_assets: Some(PackageAssetStore::new(prepared_widget.asset_root())),
                url_rewrites: self.url_rewrites.clone(),
                ..RuntimeConfig::default()
            };
            let geometry = RuntimeTileGeometry::for_viewport_shape(platform, placed_shape);
            match WasmWidgetRuntime::new(
                &wasm_bytes,
                tile.gpu.width,
                tile.gpu.height,
                geometry.viewport_shape,
                geometry.display,
                chrono::Local::now().fixed_offset(),
                rt_config,
            ) {
                Ok(mut rt) => {
                    rt.set_network_info(stub_network());
                    rt.deliver_credentials_update(credentials.clone(), secrets.clone());
                    replacements.push((idx, rt, led_rx));
                }
                Err(e) => {
                    tracing::warn!("hot reload: {}: {e}", tile.label);
                    return;
                }
            }
        }
        for (idx, runtime, led_rx) in replacements {
            let tile = &mut self.tiles[idx];
            tile.renderer.drop_all();
            tile.runtime = Some(runtime);
            tile.led_rx = led_rx;
            tile.led_scene = None;
            tile.led_enabled = false;
            tile.dead = false;
            tile.ever_rendered = false;
        }
        self.prepared_widget = prepared_widget;
    }

    /// Switch the previewed platform, leaving params and system state intact.
    /// Recording state is preserved by rejecting switches while recording is
    /// active. Tiles are dropped so the lazy init path rebuilds runtime,
    /// renderer, and GPU resources at the new viewport sizes.
    pub(crate) fn switch_platform(&mut self, target_id: &str, ctx: &egui::Context) {
        if target_id == self.active_platform_id {
            return;
        }
        if let Err(reason) = can_switch_platform(self.recording_mode.state.is_some()) {
            tracing::warn!("switch: refusing platform switch to '{target_id}': {reason}");
            return;
        }
        let Some(current_platform) = self.catalog.platform(&self.active_platform_id) else {
            tracing::error!(
                "switch: active platform '{}' not in catalog",
                self.active_platform_id
            );
            return;
        };
        let mut switch = SwitchState::new(&self.active_platform_id, current_platform);
        switch.requested_size = self.requested_size;

        match switch.switch_to(&self.catalog, target_id) {
            Ok(false) => return,
            Ok(true) => {}
            Err(e) => {
                tracing::error!("switch: {e}");
                return;
            }
        }

        self.active_platform_id = switch.active_platform_id;
        self.layout = switch.layout;
        self.requested_size = switch.requested_size;
        if switch.needs_tile_rebuild {
            let expected_pool_len =
                gpu_pool_len_after_detach(self.gpu_pool.len(), self.tiles.len());
            for tile in self.tiles.drain(..) {
                self.gpu_pool.push(tile.into_pooled_gpu(&self.gl));
            }
            debug_assert_eq!(self.gpu_pool.len(), expected_pool_len);
        }
        self.size_pin_attempts = 30;
        ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
        ctx.request_repaint();
    }

    /// Build the four widget tiles on first `ui` call (where `eframe::Frame` is available
    /// for `register_native_glow_texture`).
    #[expect(
        clippy::too_many_lines,
        reason = "single tile-setup pass: read wasm bytes, resolve platform, build runtime + \
                  GPU + renderer per tile, register textures, log SDK version"
    )]
    fn init_tiles(&mut self, frame: &mut eframe::Frame) -> Result<()> {
        let get_proc = Self::gl_proc_address()
            .ok_or_else(|| anyhow::anyhow!("BUG: get_proc_address vanished after construction"))?;
        let wasm_bytes = std::fs::read(self.prepared_widget.wasm_path()).with_context(|| {
            format!(
                "failed to read {}",
                self.prepared_widget.wasm_path().display()
            )
        })?;
        let platform = self
            .catalog
            .platform(&self.active_platform_id)
            .ok_or_else(|| anyhow::anyhow!("BUG: active platform id must exist in catalog"))?;
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

        let initial_pool_len = self.gpu_pool.len();
        let mut tiles = Vec::with_capacity(self.layout.tiles.len());
        for (tile_idx, placed) in self.layout.tiles.iter().enumerate() {
            let (x, y, w, h) = (placed.x, placed.y, placed.w, placed.h);
            let label = placed.label.clone();
            let gpu = if let Some(mut gpu) = self.gpu_pool.pop() {
                gpu.reinitialize(&self.gl, w, h)?;
                gpu
            } else {
                TileGpu::new(&self.gl, frame, w, h)?
            };
            let (led_tx, led_rx) = if placed.led_count.is_some() {
                let (led_tx, led_rx) = std::sync::mpsc::channel();
                (Some(led_tx), Some(led_rx))
            } else {
                (None, None)
            };
            // Per-tile KV storage matches the prior testbed layout
            // (`./widget_data/<widget>/<size>/`). Active recording tile wipes its KV first
            // so the fixture starts from a known baseline.
            let kv_path = kv_base.join(&placed.kv_key);
            if active_record_idx == Some(tile_idx) {
                let _ = std::fs::remove_dir_all(&kv_path);
                let _ = std::fs::create_dir_all(&kv_path);
            }
            if let Some(widget_root) = self.cli.resolved_widget_root() {
                seed_kv_from_widget_root(&widget_root, &kv_path);
            }

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
                    asset_cache: self.cli.asset_cache(),
                    ..RuntimeConfig::default()
                }
            };
            rt_config.mesh_msaa_samples = 4;
            rt_config.package_assets =
                Some(PackageAssetStore::new(self.prepared_widget.asset_root()));
            rt_config.params = self.params.clone();
            rt_config.system = self.system.clone();
            rt_config.led_request_sender = led_tx;
            rt_config.url_rewrites.clone_from(&self.url_rewrites);
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
            let geometry = RuntimeTileGeometry::for_viewport_shape(platform, placed.shape);
            // A runtime only for a supported size: constructing one runs the guest's
            // `init` (and its discovery), so placeholders stay wasm-free.
            let runtime = if viewport_supported(placed, &self.manifest.supported_viewports) {
                let mut rt = WasmWidgetRuntime::new(
                    &wasm_bytes,
                    w,
                    h,
                    geometry.viewport_shape,
                    geometry.display,
                    chrono::Local::now().fixed_offset(),
                    rt_config,
                )
                .with_context(|| format!("create runtime for {label}"))?;
                rt.set_network_info(stub_network());
                Some(rt)
            } else {
                None
            };
            tiles.push(PreviewTile {
                runtime,
                renderer,
                gpu,
                x,
                y,
                shape: placed.shape,
                label,
                dead: false,
                ever_rendered: false,
                next_render_at_ms: None,
                pending_interaction: false,
                led_count: placed.led_count,
                led_rx,
                led_scene: None,
                led_enabled: false,
            });
        }
        debug_assert_eq!(
            self.gpu_pool.len(),
            gpu_pool_len_after_init(initial_pool_len, self.layout.tiles.len())
        );
        if let Some((major, minor, patch)) = tiles
            .iter()
            .find_map(|t| t.runtime.as_ref().map(WasmWidgetRuntime::sdk_version))
        {
            println!("Widget SDK version: {major}.{minor}.{patch}");
        }
        // Snapshot the active recording tile's KV directory at start
        // so the fixture's `header.kv` reproduces the initial state on replay.
        if let Some(ref mut rec) = self.recording_mode.state
            && let Some(placed) = self.layout.tiles.get(rec.active_tile)
        {
            let kv_path = kv_base.join(&placed.kv_key);
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

    /// Drive one frame: each tile's WASM runtime renders into its FBO.
    /// Egui paints the textures afterward via `painter.image`.
    ///
    /// Saves the GL framebuffer binding + viewport before mutating them
    /// per tile and restores both at the end so egui's own draw list runs
    /// against the screen framebuffer the way it expects.
    ///
    /// Skipping this caused screen-wide trails
    /// (egui's clear hit a tile FBO instead of the default framebuffer).
    ///
    /// Drive each tile on-demand off its own runtime scheduler;
    /// returns the earliest next render across tiles (ms) for the host wake.
    ///
    /// Clock and delivery drain run every tick; the WASM render is gated,
    /// so idle tiles cost nothing — the contract the device host honours.
    #[expect(
        clippy::too_many_lines,
        reason = "one on-demand drive pass: per-tile clock, delivery drain, render gate, deadline arming, perf sample"
    )]
    fn render_tiles(&mut self, delta_ms: u32) -> Option<u64> {
        // SAFETY: gl is current on this thread inside `App::ui`; the queries below only read.
        let (prev_fbo, prev_viewport) = unsafe {
            let prev_fbo = self.gl.get_parameter_i32(eframe::glow::FRAMEBUFFER_BINDING);
            let mut vp = [0_i32; 4];
            self.gl
                .get_parameter_i32_slice(eframe::glow::VIEWPORT, &mut vp);
            (prev_fbo, vp)
        };

        // A fast-forward advances both clocks so a due poll fires as its data
        // ages out; "reset" rewinds only the display clock, never the monotonic
        // one (which uses its own ratcheting offset).
        let offset_ms = self.clock.offset_ms;
        let monotonic_ms =
            self.clock.start_instant.elapsed().as_millis() as u64 + self.clock.monotonic_offset_ms;
        let system_time = (chrono::Local::now()
            + chrono::Duration::milliseconds(offset_ms.cast_signed()))
        .fixed_offset();
        let offline = self.offline;
        // In recording mode, only the active tile renders; the others are painted as blank
        // slabs in `App::ui`. Skipping the WASM render here both clarifies the visual focus
        // and keeps non-active runtimes from spending fuel on frames nobody will keep.
        let active_record_idx = self.recording_mode.state.as_ref().map(|r| r.active_tile);
        // Perf report samples the first live tile (placeholders have no runtime).
        let perf_idx = self.tiles.iter().position(|t| t.runtime.is_some());
        let mut next_wake_ms: Option<u64> = None;
        // Captured only on a real render, so `--perf-frames` counts widget
        // renders, not idle ticks.
        let mut perf_capture: Option<(
            bmc_render::FrameTimings,
            std::collections::BTreeMap<String, u64>,
        )> = None;
        for (tile_idx, tile) in self.tiles.iter_mut().enumerate() {
            if active_record_idx.is_some_and(|active| active != tile_idx) {
                continue;
            }
            if tile.runtime.is_none() || tile.dead {
                continue;
            }
            tile.drain_led_commands();

            // `*mut FemtoVgRenderer` → `*mut dyn Renderer` is a coercion, not an `as` cast.
            let renderer_raw: *mut dyn bmc_render::renderer::Renderer =
                core::ptr::addr_of_mut!(tile.renderer);
            let renderer_ptr = std::ptr::NonNull::new(renderer_raw)
                .expect("BUG: addr_of_mut! cannot produce null");

            // Clock + delivery drain every tick (renderer parked
            // so bitmap-registering delivery callbacks work).
            //
            // No `begin_frame` — it clears the FBO,
            // so it must bracket a real render, not a drain.
            let delivery_outcome = {
                let rt = tile
                    .runtime
                    .as_mut()
                    .expect("BUG: placeholder skipped above");
                rt.set_hermetic(offline);
                rt.set_time(system_time, monotonic_ms);
                let result = rt.poll_deliveries_with_renderer(renderer_ptr);
                delivery_poll_outcome(result, || rt.next_frame_delay() == Some(0))
            };
            let immediate = match delivery_outcome {
                DeliveryPollOutcome::Ready { immediate } => immediate,
                DeliveryPollOutcome::Trapped(error) => {
                    tile.dead = true;
                    tracing::error!("{}: delivery trapped: {error}", tile.label);
                    continue;
                }
            };

            // Render only when the tile's scheduler asks: first frame,
            // queued touch, deadline reached, or an immediate (maybe delivery-raised) request.
            let due = !tile.ever_rendered
                || tile.pending_interaction
                || tile.next_render_at_ms.is_some_and(|at| monotonic_ms >= at)
                || immediate;
            if !due {
                // Delivery armed a future (non-immediate) frame: set the deadline once.
                if tile.next_render_at_ms.is_none() {
                    let rt = tile
                        .runtime
                        .as_mut()
                        .expect("BUG: placeholder skipped above");
                    if rt.wants_next_frame() {
                        tile.next_render_at_ms =
                            Some(monotonic_ms + u64::from(rt.next_frame_delay().unwrap_or(0)));
                    }
                }
                if let Some(at) = tile.next_render_at_ms {
                    let delay = at.saturating_sub(monotonic_ms);
                    next_wake_ms = Some(next_wake_ms.map_or(delay, |w| w.min(delay)));
                }
                continue;
            }
            tile.pending_interaction = false;

            tile.renderer
                .begin_frame(tile.gpu.width, tile.gpu.height, 1.0);
            let outcome = tile
                .runtime
                .as_mut()
                .expect("BUG: placeholder skipped above")
                .with_renderer(renderer_ptr, |rt| rt.render(delta_ms));
            match outcome {
                Ok(RenderStatus::Ok) => {
                    if !tile.ever_rendered {
                        tracing::info!(
                            label = %tile.label,
                            instance_id = %tile
                                .runtime
                                .as_ref()
                                .expect("BUG: placeholder skipped above")
                                .asset_namespace(),
                            "tile: first render after construction/reload"
                        );
                        tile.ever_rendered = true;
                    }
                }
                Ok(RenderStatus::FuelExhausted) => {
                    tracing::warn!("{}: fuel exhausted", tile.label);
                }
                Ok(RenderStatus::Dead) => {
                    if !tile.dead {
                        tracing::error!("{}: widget killed (repeated fuel overages)", tile.label);
                        tile.dead = true;
                    }
                }
                Err(e) => {
                    tracing::error!("{}: render failed: {e}", tile.label);
                }
            }
            tile.renderer.flush();

            // Arm the next deadline from what the widget just requested (`None` = idle).
            // Set only here, never per idle tick, so it can't recede.
            let next_at = {
                let rt = tile
                    .runtime
                    .as_mut()
                    .expect("BUG: placeholder skipped above");
                rt.wants_next_frame()
                    .then(|| monotonic_ms + u64::from(rt.next_frame_delay().unwrap_or(0)))
            };
            tile.next_render_at_ms = next_at;
            if let Some(at) = next_at {
                let delay = at.saturating_sub(monotonic_ms);
                next_wake_ms = Some(next_wake_ms.map_or(delay, |w| w.min(delay)));
            }

            if Some(tile_idx) == perf_idx {
                let rt = tile
                    .runtime
                    .as_mut()
                    .expect("BUG: placeholder skipped above");
                perf_capture = Some((rt.last_timings(), rt.take_profile_sections()));
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
                std::num::NonZeroU32::new(prev_fbo as u32).map(eframe::glow::NativeFramebuffer);
            self.gl.bind_framebuffer(eframe::glow::FRAMEBUFFER, target);
            self.gl.viewport(
                prev_viewport[0],
                prev_viewport[1],
                prev_viewport[2],
                prev_viewport[3],
            );
        }

        next_wake_ms
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
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(50)),
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
                if let Some(t) = self
                    .tiles
                    .first()
                    .and_then(|t| t.runtime.as_ref())
                    .map(WasmWidgetRuntime::last_timings)
                {
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

    fn stats_rect(&self, origin: egui::Pos2) -> egui::Rect {
        egui::Rect::from_min_size(
            origin + egui::vec2(self.layout.stats_x as f32, self.layout.stats_y as f32),
            egui::vec2(self.layout.stats_w as f32, self.layout.stats_h as f32),
        )
    }
}

fn can_switch_platform(recording_active: bool) -> Result<(), &'static str> {
    if recording_active {
        Err("recording is active")
    } else {
        Ok(())
    }
}

fn gpu_pool_len_after_detach(pool_len: usize, active_tiles: usize) -> usize {
    pool_len + active_tiles
}

fn gpu_pool_len_after_init(pool_len: usize, needed_tiles: usize) -> usize {
    pool_len.saturating_sub(needed_tiles)
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
            (
                platforms::DisplayShape::Rectangular,
                ViewportShape::Rectangular
            ) | (platforms::DisplayShape::Round, ViewportShape::Round)
        );
        shape_ok
            && c.min_width.is_none_or(|lo| placed.w >= lo)
            && c.max_width.is_none_or(|hi| placed.w <= hi)
            && c.min_height.is_none_or(|lo| placed.h >= lo)
            && c.max_height.is_none_or(|hi| placed.h <= hi)
    })
}

/// Paint a dim "size not supported" slab where a widget declines a tile.
fn paint_placeholder(painter: &egui::Painter, rect: egui::Rect, label: &str) {
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(14));
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(38)),
        egui::StrokeKind::Inside,
    );
    let icon = rect.center() - egui::vec2(0.0, 16.0);
    let radius = 13.0;
    let stroke = egui::Stroke::new(2.0_f32, egui::Color32::from_gray(80));
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

fn paint_tile_texture(ui: &egui::Ui, tile: &PreviewTile, rect: egui::Rect) {
    // FemtoVG renders bottom-up into the FBO; flip V to display top-down.
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0));
    ui.painter()
        .image(tile.gpu.egui_tex_id, rect, uv, egui::Color32::WHITE);
    if paint::is_round(tile.shape) {
        paint::paint_round_overlay(ui.painter(), rect);
    }
}

// Process-wide cell holding the eframe-provided GL proc address loader,
// populated in `TestbedApp::new` and read by `init_tiles` / `poll_hot_reload`.
//
// A thread-local sidesteps the `dyn Fn` capture lifetime question while
// keeping the closure trivially cloneable. `thread_local!` is a macro,
// so this stays a regular `//` comment — doc comments don't attach
// to macro invocations.
thread_local! {
    static GET_PROC_ADDRESS: std::cell::RefCell<Option<GlProcAddress>>
        = const { std::cell::RefCell::new(None) };
}

/// Host wake floor when nothing sooner is scheduled: drains deliveries
/// and animates chrome (the LED strip) without running widget WASM
/// — real renders are gated per tile.
/// ~30 Hz keeps LEDs smooth; it is not a render rate.
const DRAIN_TICK_MS: u64 = 33;

impl eframe::App for TestbedApp {
    #[expect(
        clippy::too_many_lines,
        reason = "one egui frame: resize lock, lazy tile init, render, side panels, per-tile paint + touch"
    )]
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // A manual window close still seals the perf report from whatever
        // was collected, so an operator who closed early instead of waiting
        // for `--perf-frames` doesn't lose the run.
        if root_ui.ctx().input(|i| i.viewport().close_requested()) {
            self.finish_perf_report();
            return;
        }
        // One-shot resize lock. Avoid min/max size commands:
        // on Wayland they can fail protocol validation while switching
        // between narrow and wide platform layouts.
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
        self.poll_hot_reload();

        let now = std::time::Instant::now();
        let delta = now.duration_since(self.clock.last_frame);
        let delta_ms = delta.as_millis() as u32;
        let frame_us = delta.as_micros() as u32;
        self.clock.last_frame = now;
        self.record_frame_us(frame_us);

        let ctx = root_ui.ctx().clone();

        // Render each widget into its FBO before egui submits its own draw list.
        // Must happen before checkerboard / image draws to keep GL state contained.
        // On-demand per tile; `next_wake` = earliest a tile wants a frame (ms).
        let next_wake = self.render_tiles(delta_ms);

        // Seal the report and close once enough real renders are collected.
        if self.cli.perf_report_path.is_some() && self.perf.frame_count >= self.cli.perf_frames {
            self.finish_perf_report();
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
                    // A declined size gets the "not supported" slab, not a texture.
                    if tile.runtime.is_none() {
                        paint_placeholder(ui.painter(), rect, &tile.label);
                        continue;
                    }
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

                    paint_tile_texture(ui, tile, rect);
                    if tile.dead {
                        continue;
                    }

                    if active_record_idx == Some(tile_idx) {
                        // `Inside` so the bottom edge stays inside the tile rect
                        // — `Outside` would paint one row below, which `paint_led_strip`
                        // then overwrites with the LED strip background.
                        ui.painter().rect_stroke(
                            rect,
                            0.0,
                            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(255, 170, 80)),
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
                    let touched = dispatch_touch_events(
                        &response,
                        rect,
                        tile.runtime
                            .as_mut()
                            .expect("BUG: placeholder skipped above"),
                        rec_for_tile,
                    );
                    tile.pending_interaction |= touched;

                    if tile.led_count.is_some() {
                        paint_led_strip(ui.painter(), tile, origin, time_s);
                    }
                }
                // Stats panel / recording panel — both anchor in the empty slot right of SMALL.
                // Recording mode displaces the stats view; the chart isn't useful while
                // authoring a fixture and the operator needs the event log there.
                let stats_rect = self.stats_rect(origin);
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

        // Earliest tile deadline, capped at the drain tick for deliveries + chrome.
        let repaint_ms = next_wake.map_or(DRAIN_TICK_MS, |w| w.min(DRAIN_TICK_MS));
        ctx.request_repaint_after(std::time::Duration::from_millis(repaint_ms));
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn bundled() -> platforms::PlatformCatalog {
        platforms::PlatformCatalog::bundled().expect("BUG: bundled catalog must parse")
    }

    /// Golden BMC100 geometry, copied from the pre-change compile-time layout.
    /// (label, x, y, w, h) in logical pixels.
    const BMC100_GOLDEN_TILES: [(&str, u32, u32, u32, u32); 4] = [
        ("Fullscreen", 16, 16, 1280, 480),
        ("Large", 16, 536, 638, 480),
        ("Medium", 670, 536, 638, 238),
        ("Small", 670, 814, 317, 238),
    ];
    const BMC100_GOLDEN_PREVIEW: (u32, u32) = (1324, 1092);
    const BMC100_GOLDEN_STATS: (u32, u32, u32, u32) = (1003, 814, 305, 262);

    #[test]
    fn bmc100_layout_is_pixel_identical_to_pre_change() {
        let cat = bundled();
        let p = cat.platform("BMC100").expect("BUG: BMC100 must exist");
        let layout = TileLayout::for_platform(p);

        assert_eq!(layout.tiles.len(), 4, "BMC100 must keep four preview tiles");
        for (tile, &(label, x, y, w, h)) in layout.tiles.iter().zip(BMC100_GOLDEN_TILES.iter()) {
            assert_eq!(tile.label, label, "BMC100 tile label drift");
            assert_eq!(
                (tile.x, tile.y, tile.w, tile.h),
                (x, y, w, h),
                "BMC100 tile '{label}' position/size drift",
            );
        }

        assert_eq!(
            (layout.preview_w, layout.preview_h),
            BMC100_GOLDEN_PREVIEW,
            "BMC100 preview/window size drift",
        );
        assert_eq!(
            (
                layout.stats_x,
                layout.stats_y,
                layout.stats_w,
                layout.stats_h,
            ),
            BMC100_GOLDEN_STATS,
            "BMC100 stats-panel rectangle drift",
        );
    }

    #[test]
    fn bmc100_catalog_viewport_sizes_match_golden_arrangement() {
        let cat = bundled();
        let p = cat.platform("BMC100").expect("BUG: BMC100 must exist");
        for (v, &(_label, _x, _y, w, h)) in
            p.widget_viewports.iter().zip(BMC100_GOLDEN_TILES.iter())
        {
            assert_eq!(
                (v.width, v.height),
                (w, h),
                "BMC100 catalog viewport size must match the preserved arrangement",
            );
        }
    }

    #[test]
    fn validate_recording_target_rejects_unknown_size_and_accepts_known() {
        let cat = bundled();
        let p = cat.platform("BMC100").expect("BUG: BMC100 must exist");
        let layout = TileLayout::for_platform(p);
        for s in ["full", "large", "medium", "small"] {
            validate_recording_target(Some(s), "BMC100", &layout)
                .unwrap_or_else(|e| panic!("BUG: known size '{s}' must validate: {e}"));
        }
        let err = validate_recording_target(Some("SIZE=small"), "BMC100", &layout)
            .expect_err("BUG: unknown size must be rejected, not defaulted to full");
        assert!(err.contains("unknown record size 'SIZE=small'"), "{err}");
    }

    #[test]
    fn bmm100_layout_has_single_tile() {
        let cat = bundled();
        let p = cat.platform("BMM100").expect("BUG: BMM100 must exist");
        let layout = TileLayout::for_platform(p);
        assert_eq!(layout.tiles.len(), 1);
        assert_eq!(
            (layout.tiles[0].x, layout.tiles[0].y),
            (PREVIEW_MARGIN, PREVIEW_MARGIN)
        );
        assert_eq!((layout.tiles[0].w, layout.tiles[0].h), (320, 240));
    }

    #[test]
    fn bmm100_stats_width_uses_stats_minimum() {
        let cat = bundled();
        let p = cat.platform("BMM100").expect("BUG: BMM100 must exist");
        let layout = TileLayout::for_platform(p);

        assert_eq!(layout.stats_w, STATS_MIN_W);
        assert_eq!(
            layout.preview_w,
            PREVIEW_MARGIN + STATS_MIN_W + PREVIEW_MARGIN
        );
    }

    #[test]
    fn bmm101_layout_has_single_tile() {
        let cat = bundled();
        let p = cat.platform("BMM101").expect("BUG: BMM101 must exist");
        let layout = TileLayout::for_platform(p);
        assert_eq!(layout.tiles.len(), 1);
        assert_eq!((layout.tiles[0].w, layout.tiles[0].h), (480, 320));
    }

    #[test]
    fn bfm100_tile_carries_round_shape() {
        let cat = bundled();
        let p = cat.platform("BFM100").expect("BUG: BFM100 must exist");
        let layout = TileLayout::for_platform(p);
        assert_eq!(layout.tiles.len(), 1);
        assert_eq!((layout.tiles[0].w, layout.tiles[0].h), (480, 480));
        assert!(matches!(
            layout.tiles[0].shape,
            platforms::DisplayShape::Round
        ));
    }

    #[test]
    fn bfm100_stats_width_uses_viewport_width() {
        let cat = bundled();
        let p = cat.platform("BFM100").expect("BUG: BFM100 must exist");
        let layout = TileLayout::for_platform(p);

        assert_eq!(layout.stats_w, 480);
        assert_eq!(layout.preview_w, 512);
    }

    #[test]
    fn bmc100_fullscreen_tile_preserves_legacy_kv_key() {
        let cat = bundled();
        let p = cat.platform("BMC100").expect("BUG: BMC100 must exist");
        let layout = TileLayout::for_platform(p);

        assert_eq!(layout.tiles[0].label, "Fullscreen");
        assert_eq!(layout.tiles[0].kv_key, "full");
        assert_eq!(layout.tiles[1].kv_key, "large");
        assert_eq!(layout.tiles[2].kv_key, "medium");
        assert_eq!(layout.tiles[3].kv_key, "small");
    }

    #[test]
    fn switching_changes_layout_and_viewport_list() {
        let cat = bundled();
        let mut state = SwitchState::new(
            "BMC100",
            cat.platform("BMC100").expect("BUG: BMC100 must exist"),
        );
        assert_eq!(state.layout.tiles.len(), 4);
        let from_size = state.requested_size;

        let changed = state
            .switch_to(&cat, "BMM101")
            .expect("BUG: BMM101 must exist");

        assert!(changed);
        assert_eq!(state.active_platform_id, "BMM101");
        assert!(state.needs_tile_rebuild);
        assert_eq!(state.layout.tiles.len(), 1);
        assert_eq!(
            (state.layout.tiles[0].w, state.layout.tiles[0].h),
            (480, 320)
        );
        assert!(
            state.requested_size.x < from_size.x,
            "narrow platform must request a smaller window",
        );
        assert_eq!(state.requested_size, requested_window_size(&state.layout));
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
    fn gpu_pool_count_is_bounded_by_max_active_tiles() {
        let mut pooled = 0;

        pooled = gpu_pool_len_after_detach(pooled, 4);
        assert_eq!(pooled, 4, "switching away from BMC100 pools four GPUs");

        pooled = gpu_pool_len_after_init(pooled, 1);
        assert_eq!(
            pooled, 3,
            "switching to one-tile BMM101 reuses one pooled GPU",
        );

        pooled = gpu_pool_len_after_detach(pooled, 1);
        assert_eq!(
            pooled, 4,
            "switching away from BMM101 returns the reused GPU to the pool",
        );

        pooled = gpu_pool_len_after_init(pooled, 4);
        assert_eq!(
            pooled, 0,
            "switching back to BMC100 reuses all four registered GPUs",
        );
    }

    #[test]
    fn invalid_recording_target_reports_platform_and_size() {
        let cat = bundled();
        let p = cat.platform("BMM100").expect("BUG: BMM100 must exist");
        let layout = TileLayout::for_platform(p);
        let err = validate_recording_target(Some("small"), p.id.as_str(), &layout)
            .expect_err("BUG: one-tile platform cannot record BMC100 small tile");

        assert!(err.contains("small"), "{err}");
        assert!(err.contains("BMM100"), "{err}");
    }

    #[test]
    fn preview_area_encloses_every_tile() {
        let cat = bundled();
        for id in ["BMC100", "BMM100", "BMM101", "BFM100"] {
            let p = cat
                .platform(id)
                .expect("BUG: bundled platform id must exist");
            let layout = TileLayout::for_platform(p);
            let led_h = if p.led_strip.is_some() {
                LED_STRIP_H
            } else {
                0
            };
            for t in &layout.tiles {
                assert!(
                    t.x + t.w + PREVIEW_MARGIN <= layout.preview_w,
                    "{id} x overflow"
                );
                assert!(
                    t.y + t.h + led_h + PREVIEW_MARGIN <= layout.preview_h,
                    "{id} y overflow"
                );
            }
        }
    }

    #[test]
    fn stripless_platforms_do_not_reserve_led_strip_height() {
        let cat = bundled();
        let p = cat.platform("BMM100").expect("BUG: BMM100 must exist");
        let layout = TileLayout::for_platform(p);
        assert_eq!(layout.tiles[0].led_count, None);
        assert_eq!(
            layout.stats_y,
            PREVIEW_MARGIN + 240 + PREVIEW_GAP,
            "BMM100 stats should start immediately below the tile without LED_STRIP_H",
        );
    }

    #[test]
    fn bfm100_runtime_geometry_is_round_for_viewport_and_display() {
        let cat = bundled();
        let p = cat.platform("BFM100").expect("BUG: BFM100 must exist");
        let layout = TileLayout::for_platform(p);
        let tile = &layout.tiles[0];

        let geometry = RuntimeTileGeometry::for_viewport_shape(p, tile.shape);
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
        let cat = bundled();
        let p = cat.platform("BMM101").expect("BUG: BMM101 must exist");
        let layout = TileLayout::for_platform(p);
        let tile = &layout.tiles[0];

        let geometry = RuntimeTileGeometry::for_viewport_shape(p, tile.shape);
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
        let cat = bundled();
        let p = cat.platform("BMC100").expect("BUG: BMC100 must exist");
        let layout = TileLayout::for_platform(p);
        let medium = layout
            .tiles
            .iter()
            .find(|tile| tile.label == "Medium")
            .expect("BUG: BMC100 medium tile must exist");

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
        assert_eq!((medium.w, medium.h), (638, 238));
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;

    #[test]
    fn a_delivery_trap_does_not_query_the_runtime_again() {
        let outcome = delivery_poll_outcome(Err(anyhow::anyhow!("guest trapped")), || {
            panic!("a trapped runtime must not be driven again")
        });

        assert!(matches!(outcome, DeliveryPollOutcome::Trapped(_)));
    }
}
