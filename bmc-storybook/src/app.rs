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

//! Main eframe application.

// ── Pixel-value casts (always positive, precision loss irrelevant) ───

/// Integer → `f32` for pixel dimensions (precision loss above 2²⁴ is irrelevant for UI).
#[expect(clippy::cast_precision_loss)]
const fn px_f(v: u32) -> f32 {
    v as f32
}

/// `usize` → `f32` (same rationale as `px_f`, for lengths/indices).
#[expect(clippy::cast_precision_loss)]
fn px_len(v: usize) -> f32 {
    v as f32
}

/// `f32` → `u32` for pixel dimensions (truncates toward zero, clamps negatives to 0).
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn px_u(v: f32) -> u32 {
    v as u32
}

// ─────────────────────────────────────────────────────────────────────

use std::cell::Cell;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use egui::{Color32, Frame, Margin, Ui, Vec2};

use bmc_storybook_api::knobs::StoryCtx;
use bmc_storybook_api::{DocBlock, FrameSize, StoryEntry, StoryGroupMeta};

use crate::hot_reload::{
    HotReloader, OwnedStoryEntry, OwnedStoryGroupMeta, ReloadEvent, default_so_path, workspace_root,
};
use crate::preview::{BootstrapFbo, DocumentRenderer};
use crate::sidebar::SidebarState;

use bmc_render::gpu::FemtoVgRenderer;
use bmc_render::interaction::{ActionEvent, TouchEvent};
use bmc_render::renderer::Renderer as _;

// ── Thread-local renderer bridge for asset registrars ────────────────
//
// The SDK's pluggable registrars (`init_icon_registrar`, etc.) and the skin
// system (`bmc_render_skin::init`) take `fn(&str, &[u8]) -> u16` — bare
// function pointers that can't capture. We bridge this by stashing a raw
// pointer to the renderer in a thread-local during story rendering.
//
// Single-thread assumption: story rendering runs on the main UI thread
// only. The thread-local is null on every other thread (thread-locals
// don't propagate to spawned threads), so a story that spawns a thread
// and calls a registrar from it would trigger the null guards below.
// Stories that need background work must register their assets on the
// render thread — there is no thread-safe registrar by design.

thread_local! {
    static RENDERER_PTR: Cell<*mut FemtoVgRenderer> = const { Cell::new(std::ptr::null_mut()) };
}

pub(crate) fn registrar_icon(tag: &str, data: &[u8]) -> Option<bmc_wasm_sdk::SvgId> {
    let ptr = RENDERER_PTR.with(Cell::get);
    assert!(
        !ptr.is_null(),
        "icon registrar called from a non-render thread; \
         RENDERER_PTR is thread-local and stories must not register assets from spawned threads"
    );
    unsafe { &mut *ptr }.register_svg(tag, data)
}

pub(crate) fn registrar_bitmap(tag: &str, data: &[u8]) -> Option<bmc_wasm_sdk::BitmapId> {
    let ptr = RENDERER_PTR.with(Cell::get);
    assert!(
        !ptr.is_null(),
        "bitmap registrar called from a non-render thread; \
         RENDERER_PTR is thread-local and stories must not register assets from spawned threads"
    );
    unsafe { &mut *ptr }.register_bitmap(tag, data)
}

pub(crate) fn registrar_bitmap_nearest(tag: &str, data: &[u8]) -> Option<bmc_wasm_sdk::BitmapId> {
    let ptr = RENDERER_PTR.with(Cell::get);
    assert!(
        !ptr.is_null(),
        "bitmap_nearest registrar called from a non-render thread; \
         RENDERER_PTR is thread-local and stories must not register assets from spawned threads"
    );
    unsafe { &mut *ptr }.register_bitmap_nearest(tag, data)
}

pub(crate) fn registrar_mesh(tag: &str, data: &[u8]) -> Option<bmc_wasm_sdk::MeshId> {
    let ptr = RENDERER_PTR.with(Cell::get);
    assert!(
        !ptr.is_null(),
        "mesh registrar called from a non-render thread; \
         RENDERER_PTR is thread-local and stories must not register assets from spawned threads"
    );
    unsafe { &mut *ptr }.register_mesh(tag, data)
}

// ── Company palette (from bmc-wasm-protocol colors) ─────────────────

use bmc_render::colors;

/// Const-convert a protocol `Color` to an egui `Color32`.
const fn c(c: colors::Color) -> Color32 {
    Color32::from_rgb(c.red(), c.green(), c.blue())
}

const HEADER_BG: Color32 = c(colors::GRAY_90);
const PANEL_BG: Color32 = c(colors::GRAY_100);
/// Center pane: lighter than sidebars but darker than the checkerboard,
/// so frame boundaries remain visible.
const PREVIEW_BG: Color32 = Color32::from_rgb(0x1E, 0x1E, 0x1E);
const GROUP_COLOR: Color32 = c(colors::GRAY_50);

const HEADER_HEIGHT: f32 = 28.0;

const SHORTCUT_COLOR: Color32 = c(colors::GRAY_60);
const BORDER_COLOR: Color32 = c(colors::GRAY_80);
const BUTTON_FG: Color32 = Color32::WHITE;

const CHECKER_SIZE: f32 = 12.0;
const CHECKER_DARK: Color32 = Color32::from_rgb(0x25, 0x25, 0x25);
const CHECKER_LIGHT: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);

const MAX_ACTION_LOG_ENTRIES: usize = 500;

/// Tag for action log entries — determines which icon to show.
#[derive(Debug, Clone, Copy)]
enum ActionKind {
    Click,
}

/// Frame-time ring buffer + smoothed display values for the performance panel.
struct PerfStats {
    frame_times: [f32; 30],
    write_idx: usize,
    display_fps: u32,
    display_ms: f32,
    update_at: Instant,
}

impl PerfStats {
    fn new() -> Self {
        Self {
            frame_times: [0.0; 30],
            write_idx: 0,
            display_fps: 0,
            display_ms: 0.0,
            update_at: Instant::now(),
        }
    }

    /// Record one frame's delta time and refresh the smoothed values ~4×/sec.
    fn record(&mut self, dt: f32) {
        self.frame_times[self.write_idx] = dt;
        self.write_idx = (self.write_idx + 1) % self.frame_times.len();

        if self.update_at.elapsed().as_secs_f32() > 0.25 {
            self.update_at = Instant::now();
            let avg = self.frame_times.iter().sum::<f32>() / px_len(self.frame_times.len());
            self.display_fps = px_u(1.0 / avg);
            self.display_ms = avg * 1_000.0;
        }
    }
}

#[expect(
    missing_debug_implementations,
    clippy::struct_excessive_bools,
    reason = "UI state naturally uses independent toggle bools"
)]
pub struct StorybookApp {
    sidebar: SidebarState,
    ctx: StoryCtx,
    doc_renderer: Option<DocumentRenderer>,
    gl: Arc<eframe::glow::Context>,
    last_entry_name: Option<String>,
    action_log: VecDeque<(Instant, ActionKind, String)>,
    start_time: Instant,
    hot_reloader: Option<HotReloader>,
    /// Error from a panicking story render_fn, shown in the preview area.
    story_error: Option<String>,
    /// Error from a failed hot-reload build, shown in the preview area.
    build_error: Option<String>,
    /// When a hot-reload build is in progress, the instant it started.
    build_started: Option<Instant>,
    /// Deferred story selection — set from persisted storage, applied after first .so load.
    pending_selection: Option<String>,
    /// Last measured preview panel width (for root layout constraint).
    preview_width: f32,
    preview_height: f32,
    icons: crate::icons::Icons,
    perf: PerfStats,
    /// Document blocks from the current story's render — kept alive for the egui layout pass.
    doc_blocks: Vec<DocBlock>,
    /// Show source code tab instead of preview.
    show_source: bool,
    /// Left sidebar visibility (toggled via Ctrl+Shift+L).
    show_sidebar: bool,
    /// Right controls panel visibility (toggled via Ctrl+Shift+R).
    show_right_panel: bool,
    /// Bottom panel visibility (toggled via Ctrl+Shift+B).
    show_bottom_panel: bool,
    /// When true, animations are frozen (delta_ms = 0).
    anim_paused: bool,
    /// Scrub position in 0.0..=1.0 range (maps to 0..10s of animation time).
    anim_scrub: f32,
}

impl StorybookApp {
    /// Configure egui theme: dark CDS-inspired visuals.
    fn setup_theme(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL_BG;
        visuals.window_fill = PANEL_BG;
        visuals.window_stroke = egui::Stroke::NONE;
        visuals.faint_bg_color = HEADER_BG;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, c(colors::GRAY_80));

        let input_bg = c(colors::GRAY_90);
        let input_border = c(colors::GRAY_70);
        visuals.widgets.inactive.bg_fill = input_bg;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, c(colors::GRAY_80));
        visuals.widgets.inactive.corner_radius = egui::CornerRadius::ZERO;
        visuals.widgets.hovered.bg_fill = input_bg;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, input_border);
        visuals.extreme_bg_color = input_bg;
        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        style.interaction.selectable_labels = false;
        ctx.set_style(style);
    }

    /// Create the FemtoVG document renderer from the eframe GL context.
    #[expect(unsafe_code)]
    fn create_renderer(
        cc: &eframe::CreationContext<'_>,
        gl: &Arc<eframe::glow::Context>,
    ) -> DocumentRenderer {
        let bootstrap_fbo = BootstrapFbo::new(gl);
        let get_proc = cc
            .get_proc_address
            .clone()
            .expect("BUG: glow backend must provide get_proc_address");
        let renderer = unsafe {
            FemtoVgRenderer::new(
                |name| {
                    let cstr = std::ffi::CString::new(name)
                        .expect("BUG: GL function name contains null byte");
                    get_proc(&cstr)
                },
                bootstrap_fbo.width,
                bootstrap_fbo.height,
                bootstrap_fbo.fbo_id(),
                0,
            )
            .expect("BUG: failed to create FemtoVG renderer")
        };
        DocumentRenderer::new(renderer)
    }

    /// Load stories and optionally start the hot-reloader.
    fn load_stories(
        hot_reload: bool,
    ) -> (
        Vec<OwnedStoryEntry>,
        Vec<OwnedStoryGroupMeta>,
        Option<HotReloader>,
        Option<Instant>,
    ) {
        if hot_reload {
            let so_path = match default_so_path() {
                Ok(path) => path,
                Err(e) => {
                    tracing::error!(
                        "failed to resolve storybook .so path: {e}, falling back to static"
                    );
                    let (entries, groups) = load_static_stories();
                    return (entries, groups, None, None);
                }
            };
            match HotReloader::new(so_path, &workspace_root()) {
                Ok(mut reloader) => {
                    let started = match reloader.start_build() {
                        Ok(()) => Some(Instant::now()),
                        Err(e) => {
                            tracing::error!("hot-reload: initial build failed to start: {e}");
                            None
                        }
                    };
                    (Vec::new(), Vec::new(), Some(reloader), started)
                }
                Err(e) => {
                    tracing::error!("failed to start hot-reloader: {e}, falling back to static");
                    let (entries, groups) = load_static_stories();
                    (entries, groups, None, None)
                }
            }
        } else {
            let (entries, groups) = load_static_stories();
            (entries, groups, None, None)
        }
    }

    #[must_use]
    pub fn new(cc: &eframe::CreationContext<'_>, hot_reload: bool) -> Self {
        #[cfg(target_os = "macos")]
        set_macos_srgb_colorspace(cc);

        Self::setup_theme(&cc.egui_ctx);

        let icons = crate::icons::Icons::load(&cc.egui_ctx);
        let gl = cc
            .gl
            .clone()
            .expect("BUG: glow backend must provide GL context");
        let doc_renderer = Self::create_renderer(cc, &gl);

        let (entries, groups, hot_reloader, build_started) = Self::load_stories(hot_reload);
        let mut sidebar = SidebarState::new(entries, groups);

        // Restore persisted state — in hot-reload mode, defer selection until first .so load.
        let mut pending_selection = None;
        let mut show_sidebar = true;
        let mut show_right_panel = true;
        let mut show_bottom_panel = true;
        if let Some(storage) = cc.storage {
            if let Some(name) = storage.get_string("selected_story")
                && !sidebar.select_by_module_path(&name)
            {
                pending_selection = Some(name);
            }
            if let Some(filter) = storage.get_string("filter") {
                sidebar.filter = filter;
            }
            if storage.get_string("show_sidebar").as_deref() == Some("false") {
                show_sidebar = false;
            }
            if storage.get_string("show_right_panel").as_deref() == Some("false") {
                show_right_panel = false;
            }
            if storage.get_string("show_bottom_panel").as_deref() == Some("false") {
                show_bottom_panel = false;
            }
        }

        Self {
            sidebar,
            ctx: StoryCtx::new(),
            doc_renderer: Some(doc_renderer),
            gl,
            last_entry_name: None,
            action_log: VecDeque::new(),
            start_time: Instant::now(),
            hot_reloader,
            story_error: None,
            build_error: None,
            build_started,
            pending_selection,
            preview_width: 800.0,
            preview_height: 600.0,
            icons,
            perf: PerfStats::new(),
            doc_blocks: Vec::new(),
            show_source: false,
            show_sidebar,
            show_right_panel,
            show_bottom_panel,
            anim_paused: false,
            anim_scrub: 0.0,
        }
    }

    /// Paint a small keyboard shortcut pill badge (e.g. `Ctrl+F`).
    fn shortcut_pill(ui: &mut Ui, text: &str) {
        let galley = ui.painter().layout_no_wrap(
            text.to_owned(),
            egui::FontId::monospace(10.0),
            SHORTCUT_COLOR,
        );
        let size = galley.size() + egui::vec2(8.0, 4.0);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, c(colors::GRAY_80));
        ui.painter()
            .galley(rect.min + egui::vec2(4.0, 2.0), galley, SHORTCUT_COLOR);
    }

    fn header(ui: &mut Ui, label: &str) {
        Frame::NONE
            .fill(HEADER_BG)
            .inner_margin(Margin::symmetric(8, 0))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), HEADER_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| ui.label(egui::RichText::new(label).size(16.0).color(Color32::WHITE)),
                );
            });
    }

    /// Render toggle buttons at each panel boundary (sidebar, controls, bottom).
    /// Called from the destructured block — takes field references, not `&mut self`.
    #[expect(
        clippy::too_many_arguments,
        reason = "passes destructured UI state for panel toggle rendering"
    )]
    fn render_panel_toggles(
        ctx: &egui::Context,
        screen: egui::Rect,
        sidebar_edge: f32,
        right_edge: f32,
        bottom_edge: f32,
        icons: &crate::icons::Icons,
        show_sidebar: &mut bool,
        show_right_panel: &mut bool,
        show_bottom_panel: &mut bool,
    ) {
        let inset = 16.0;
        let below_header = HEADER_HEIGHT + inset;
        Self::edge_toggle(
            ctx,
            "sidebar_toggle",
            egui::pos2(sidebar_edge - 4.0, screen.top() + below_header),
            if *show_sidebar {
                &icons.caret_left
            } else {
                &icons.caret_right
            },
            if *show_sidebar {
                "Hide sidebar (Ctrl+Shift+L)"
            } else {
                "Show sidebar (Ctrl+Shift+L)"
            },
            show_sidebar,
        );
        Self::edge_toggle(
            ctx,
            "right_panel_toggle",
            egui::pos2(right_edge - 14.0, screen.top() + below_header),
            if *show_right_panel {
                &icons.caret_right
            } else {
                &icons.caret_left
            },
            if *show_right_panel {
                "Hide controls (Ctrl+Shift+R)"
            } else {
                "Show controls (Ctrl+Shift+R)"
            },
            show_right_panel,
        );
        Self::edge_toggle(
            ctx,
            "bottom_panel_toggle",
            egui::pos2(sidebar_edge + inset, bottom_edge - 14.0),
            if *show_bottom_panel {
                &icons.caret_down
            } else {
                &icons.caret_up
            },
            if *show_bottom_panel {
                "Hide bottom panel (Ctrl+Shift+B)"
            } else {
                "Show bottom panel (Ctrl+Shift+B)"
            },
            show_bottom_panel,
        );
    }

    fn edge_toggle(
        ctx: &egui::Context,
        id: &str,
        pos: egui::Pos2,
        icon: &egui::TextureHandle,
        tooltip: &str,
        visible: &mut bool,
    ) {
        egui::Area::new(egui::Id::new(id))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                ui.spacing_mut().button_padding = egui::vec2(4.0, 4.0);
                let btn = ui.add(
                    egui::Button::image(crate::icons::icon_image(icon, 10.0, SHORTCUT_COLOR))
                        .fill(HEADER_BG),
                );
                if btn.on_hover_text(tooltip).clicked() {
                    *visible = !*visible;
                }
            });
    }

    fn filter_id() -> egui::Id {
        egui::Id::new("story_filter")
    }

    fn on_selection_changed(&mut self) {
        if let Some(entry) = self.sidebar.selected()
            && self.last_entry_name.as_deref() != Some(&entry.name)
        {
            self.ctx = StoryCtx::new();
            self.story_error = None;
            self.action_log.clear();
            self.last_entry_name = Some(entry.name.clone());
        }
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        const CMD_SHIFT: egui::Modifiers = egui::Modifiers {
            alt: false,
            ctrl: false,
            shift: true,
            mac_cmd: false,
            command: true,
        };

        // Ctrl+Shift+L → toggle left sidebar
        if ctx.input_mut(|i| i.consume_key(CMD_SHIFT, egui::Key::L)) {
            self.show_sidebar = !self.show_sidebar;
        }

        // Ctrl+Shift+R → toggle right panel
        if ctx.input_mut(|i| i.consume_key(CMD_SHIFT, egui::Key::R)) {
            self.show_right_panel = !self.show_right_panel;
        }

        // Ctrl+Shift+B → toggle bottom panel
        if ctx.input_mut(|i| i.consume_key(CMD_SHIFT, egui::Key::B)) {
            self.show_bottom_panel = !self.show_bottom_panel;
        }

        // Ctrl+F → focus filter input
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            ctx.memory_mut(|m| m.request_focus(Self::filter_id()));
        }

        // Escape → clear filter and unfocus
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.sidebar.filter.clear();
            ctx.memory_mut(|m| m.surrender_focus(Self::filter_id()));
        }

        // Shift+Tab → previous story (must be checked before bare Tab)
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab))
            && self.sidebar.select_previous()
        {
            self.on_selection_changed();
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("__tab_nav__")));
        }

        // Tab → next story
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
            && self.sidebar.select_next()
        {
            self.on_selection_changed();
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("__tab_nav__")));
        }
    }

    /// Handle hot-reload events: poll watcher, process build results, reload .so.
    fn handle_hot_reload(&mut self) {
        let Some(reloader) = &mut self.hot_reloader else {
            return;
        };

        let Some(event) = reloader.poll() else {
            return;
        };

        match event {
            ReloadEvent::BuildStarted => {
                self.build_started = Some(Instant::now());
                self.build_error = None;
                tracing::info!("hot-reload: build started");
            }
            ReloadEvent::BuildSucceeded | ReloadEvent::SoChanged => {
                self.build_started = None;
                match reloader.try_load_so() {
                    Ok((entries, groups)) => {
                        let count = entries.len();
                        self.sidebar.reload(entries, groups);
                        // Apply deferred selection from persisted storage (first load).
                        if let Some(name) = self.pending_selection.take() {
                            self.sidebar.select_by_module_path(&name);
                        }
                        let old_knobs = self.ctx.knobs_mut().clone();
                        self.ctx = StoryCtx::new_with_restore(old_knobs);
                        self.story_error = None;
                        self.build_error = None;
                        tracing::info!("hot-reload: loaded {count} stories");
                    }
                    Err(e) => {
                        tracing::error!("hot-reload: load failed: {e}");
                        self.build_error = Some(e);
                    }
                }
            }
            ReloadEvent::BuildFailed(stderr) => {
                self.build_started = None;
                tracing::warn!("hot-reload: build failed:\n{stderr}");
                self.build_error = Some(stderr);
            }
        }
    }

    /// Render the full document layout: dispatch each block to its renderer.
    fn render_document(ui: &mut Ui, blocks: &[DocBlock], dr: &mut DocumentRenderer) {
        let mut frame_idx = 0;

        egui::ScrollArea::both()
            .id_salt("document_scroll")
            .auto_shrink(false)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                Frame::NONE
                    .inner_margin(Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        Self::render_blocks(ui, blocks, dr, &mut frame_idx);
                    });
            });
    }

    /// Render a list of doc blocks, recursing into Row blocks.
    fn render_blocks(
        ui: &mut Ui,
        blocks: &[DocBlock],
        dr: &mut DocumentRenderer,
        frame_idx: &mut usize,
    ) {
        for (block_idx, block) in blocks.iter().enumerate() {
            match block {
                DocBlock::Frame { size, .. } | DocBlock::CustomRender { size, .. } => {
                    Self::render_doc_frame(ui, dr, frame_idx, *size);
                }
                DocBlock::Header { title, subtitle } => {
                    Self::render_doc_header(ui, title, subtitle.as_deref());
                }
                DocBlock::Code { language, source } => {
                    Self::render_doc_code(ui, source, language);
                }
                DocBlock::Prose { text } => {
                    ui.label(
                        egui::RichText::new(text)
                            .size(14.0)
                            .color(c(colors::GRAY_20)),
                    );
                    ui.add_space(8.0);
                }
                DocBlock::Divider => {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                }
                DocBlock::Grid { cols, gap, cells } => {
                    // Anchor the Grid id to the block index rather than the parent
                    // ui's auto-id counter. The auto-id chain is sensitive to anything
                    // that shifts the parent's allocation count (panel toggles, scroll
                    // state changes), and Grid stores per-id column widths, so an
                    // unstable id wipes that state on every shift.
                    egui::Grid::new(egui::Id::new("doc_grid").with(block_idx))
                        .num_columns(*cols as usize)
                        .spacing([*gap, *gap])
                        .show(ui, |ui| {
                            for (i, cell) in cells.iter().enumerate() {
                                ui.vertical(|ui| {
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                    Self::render_blocks(ui, cell, dr, frame_idx);
                                });
                                #[expect(
                                    clippy::cast_possible_truncation,
                                    reason = "grid cell count is small"
                                )]
                                if ((i + 1) as u32).is_multiple_of(*cols) {
                                    ui.end_row();
                                }
                            }
                        });
                }
            }
        }
    }

    /// Render a rendered component frame: size label, checkerboard bg, GL texture.
    ///
    /// Wrapped in `ui.vertical()` so the size label and frame image stay
    /// stacked vertically even when the parent uses horizontal layout.
    #[expect(clippy::cast_precision_loss, reason = "preview dimensions are small")]
    fn render_doc_frame(
        ui: &mut Ui,
        dr: &mut DocumentRenderer,
        frame_idx: &mut usize,
        size: FrameSize,
    ) {
        let rf = dr.rendered_frames.get(*frame_idx);
        *frame_idx += 1;
        let Some(rf) = rf else { return };
        let target = &mut dr.targets[rf.target_idx];

        let display_w = if size.is_auto_width() {
            rf.content_size.0.max(1.0)
        } else {
            size.width() as f32
        };
        let display_h = match size.div_height() {
            bmc_storybook_api::DivHeight::Auto => rf.content_size.1.max(1.0),
            bmc_storybook_api::DivHeight::Px(h) => h as f32,
        };

        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(format!("{}×{}", px_u(display_w), px_u(display_h)))
                    .size(10.0)
                    .color(SHORTCUT_COLOR),
            );
            ui.add_space(2.0);

            // Always claim drag sense on the story frame so the enclosing
            // ScrollArea doesn't pan when the user drags an inner element
            // (e.g. a slider). Conditioning the sense on `drags.is_empty()`
            // is one frame late: when the drag *starts*, `drags` is still
            // empty (it's populated by `process_tree` after the press event
            // is recognised), so the first-frame drag would leak through to
            // the ScrollArea before the slider could claim it.
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(display_w, display_h),
                egui::Sense::click_and_drag(),
            );

            // Forward pointer events to this frame's InteractionState.
            // `rect` is the full allocated frame; `visible` is its clipped
            // intersection with the surrounding ScrollArea. Containment uses
            // `visible` (a click outside the visible portion shouldn't register),
            // but FBO coord mapping uses `rect.min` — using `visible.min` would
            // shift coordinates by the clip amount whenever part of the frame
            // is scrolled off-screen.
            let visible = rect.intersect(ui.clip_rect());
            for event in Self::collect_pointer_events(
                ui,
                &rect,
                &visible,
                &mut target.state.pointer_captured,
            ) {
                target.state.interaction.push_event(event);
            }

            // Consume mouse-wheel scroll when the pointer is over the story
            // frame so it goes to the inner story's scroll node rather than
            // also panning the enclosing storybook viewport. Without this,
            // a wheel event over a modal's scroll body would scroll both
            // the modal *and* the storybook page (double-scroll).
            let pointer_over_frame = ui
                .input(|i| i.pointer.latest_pos())
                .is_some_and(|p| visible.contains(p));
            if pointer_over_frame {
                ui.input_mut(|i| {
                    i.smooth_scroll_delta = egui::Vec2::ZERO;
                });
            }

            Self::paint_checkerboard(ui, rect);

            // Frame texture (V-flipped for GL origin); round faces mask to the
            // inscribed circle.
            if let Some(tex_id) = target.egui_texture_id {
                let u_right = display_w / target.width as f32;
                let v_bottom = 1.0 - display_h / target.height as f32;
                if size.is_round() {
                    Self::paint_round_frame(ui, rect, tex_id, u_right, v_bottom);
                } else {
                    let uv = egui::Rect::from_min_max(
                        egui::pos2(0.0, 1.0),
                        egui::pos2(u_right, v_bottom),
                    );
                    ui.painter().image(tex_id, rect, uv, Color32::WHITE);
                }
            }

            ui.add_space(12.0);
        });
    }

    /// Paint a checkerboard pattern inside `rect`.
    fn paint_checkerboard(ui: &Ui, rect: egui::Rect) {
        let painter = ui.painter_at(rect);
        let cols = px_u(rect.width() / CHECKER_SIZE + 1.0);
        let rows = px_u(rect.height() / CHECKER_SIZE + 1.0);
        for row in 0..rows {
            for col in 0..cols {
                let x = rect.min.x + px_f(col) * CHECKER_SIZE;
                let y = rect.min.y + px_f(row) * CHECKER_SIZE;
                let tile = egui::Rect::from_min_size(egui::pos2(x, y), Vec2::splat(CHECKER_SIZE));
                let color = if (row + col) % 2 == 0 {
                    CHECKER_DARK
                } else {
                    CHECKER_LIGHT
                };
                painter.rect_filled(tile, 0.0, color);
            }
        }
    }

    /// Draw the frame texture masked to a circle on the panel bg (round face),
    /// with a bezel ring. `u_right`/`v_bottom` are the V-flipped uv extent.
    #[expect(clippy::cast_precision_loss, reason = "segment count is tiny")]
    fn paint_round_frame(
        ui: &Ui,
        rect: egui::Rect,
        tex: egui::TextureId,
        u_right: f32,
        v_bottom: f32,
    ) {
        use egui::epaint::{Mesh, Vertex};
        const SEGMENTS: u32 = 96;

        let painter = ui.painter_at(rect);
        let center = rect.center();
        let radius = rect.width().min(rect.height()) / 2.0;
        let uv_at = |p: egui::Pos2| {
            let fx = (p.x - rect.min.x) / rect.width();
            let fy = (p.y - rect.min.y) / rect.height();
            egui::pos2(fx * u_right, 1.0 - fy * (1.0 - v_bottom))
        };

        let mut content = Mesh::with_texture(tex);
        content.vertices.push(Vertex {
            pos: center,
            uv: uv_at(center),
            color: Color32::WHITE,
        });
        for i in 0..=SEGMENTS {
            let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let p = center + egui::vec2(a.cos(), a.sin()) * radius;
            content.vertices.push(Vertex {
                pos: p,
                uv: uv_at(p),
                color: Color32::WHITE,
            });
        }
        for i in 1..=SEGMENTS {
            content.indices.extend_from_slice(&[0, i, i + 1]);
        }
        painter.add(egui::Shape::mesh(content));

        // Panel-bg annulus from the circle edge out past the corners masks the
        // square checkerboard into a round face.
        let mut mask = Mesh::default();
        for i in 0..=SEGMENTS {
            let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let dir = egui::vec2(a.cos(), a.sin());
            mask.colored_vertex(center + dir * radius, PREVIEW_BG);
            mask.colored_vertex(center + dir * (radius * 2.0), PREVIEW_BG);
        }
        for i in 0..SEGMENTS {
            let b = i * 2;
            mask.add_triangle(b, b + 1, b + 2);
            mask.add_triangle(b + 1, b + 3, b + 2);
        }
        painter.add(egui::Shape::mesh(mask));

        // Inset the bezel so its stroke stays inside the rect (the inscribed
        // circle touches the edges, so a ring at `radius` clips at the tangents).
        painter.circle_stroke(
            center,
            radius - 1.5,
            egui::Stroke::new(1.5_f32, Color32::from_gray(80)),
        );
    }

    /// Render a section header with optional subtitle.
    fn render_doc_header(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(title)
                .size(20.0)
                .strong()
                .color(Color32::WHITE),
        );
        if let Some(sub) = subtitle {
            ui.label(egui::RichText::new(sub).size(13.0).color(SHORTCUT_COLOR));
        }
        ui.add_space(8.0);
    }

    /// Render a syntax-highlighted code block.
    fn render_doc_code(ui: &mut Ui, source: &str, language: &str) {
        let theme = egui_extras::syntax_highlighting::CodeTheme::dark(12.0);
        let layout_job = egui_extras::syntax_highlighting::highlight(
            ui.ctx(),
            ui.style(),
            &theme,
            source,
            language,
        );
        Frame::NONE
            .fill(HEADER_BG)
            .inner_margin(Margin::symmetric(12, 8))
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.add(egui::Label::new(layout_job).wrap_mode(egui::TextWrapMode::Extend));
            });
        ui.add_space(8.0);
    }

    /// Collect raw egui pointer events within the display rect as `TouchEvent`s.
    ///
    /// Uses raw `Event::PointerButton` / `Event::PointerMoved` instead of the
    /// `Response` high-level API to avoid issues with egui's click-vs-drag routing.
    ///
    /// The display shows the FBO at 1:1 pixel scale (clipped, not scaled), so the
    /// coordinate mapping is a direct offset: `fbo_pos = screen_pos - frame.min`.
    ///
    /// `frame` is the full allocated frame rect (for FBO coord mapping);
    /// `visible` is its clipped intersection with the parent ScrollArea (for
    /// containment). Mapping against `visible.min` would shift coords by the
    /// clip amount whenever part of the frame is scrolled off-screen.
    ///
    /// `captured` tracks whether this frame received the active Down — once
    /// captured, subsequent Move/Up events route here regardless of the
    /// pointer's screen position. Without this, dragging past the frame
    /// edge would leave the frame's `InteractionState` stuck in pressed
    /// state because the matching Up never arrived.
    fn collect_pointer_events(
        ui: &Ui,
        frame: &egui::Rect,
        visible: &egui::Rect,
        captured: &mut bool,
    ) -> Vec<TouchEvent> {
        let map_to_fbo =
            |pos: egui::Pos2| -> (f32, f32) { (pos.x - frame.min.x, pos.y - frame.min.y) };

        ui.input(|i| {
            let mut out = Vec::new();

            for event in &i.events {
                if let egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } = *event
                {
                    if pressed {
                        if visible.contains(pos) {
                            let (x, y) = map_to_fbo(pos);
                            out.push(TouchEvent::Down { x, y });
                            *captured = true;
                        }
                    } else if *captured {
                        // wl_touch::up carries no coordinates; InteractionState
                        // re-uses the last Move/Down position for hit testing.
                        // Always deliver Up to whichever frame received the
                        // matching Down, even if the release happened outside
                        // the frame — otherwise the gesture stays "held".
                        out.push(TouchEvent::Up);
                        *captured = false;
                    }
                } else if let egui::Event::PointerMoved(pos) = *event
                    && i.pointer.primary_down()
                    && (*captured || visible.contains(pos))
                {
                    // While captured, forward Moves regardless of position so
                    // sliders/drags keep tracking past the frame edge. The
                    // mapped coords may be negative or exceed the FBO size;
                    // widgets clamp as needed.
                    let (x, y) = map_to_fbo(pos);
                    out.push(TouchEvent::Move { x, y });
                }
            }

            // Mouse wheel → Scroll.
            if i.smooth_scroll_delta.y.abs() > 0.5
                && let Some(pos) = i.pointer.latest_pos()
                && visible.contains(pos)
            {
                let (x, y) = map_to_fbo(pos);
                out.push(TouchEvent::Scroll {
                    x,
                    y,
                    delta_y: -i.smooth_scroll_delta.y,
                });
            }

            out
        })
    }

    /// Paint an inline sparkline (bars right-aligned in the given rect) with
    /// threshold gridlines at 60 fps and 30 fps.
    fn render_sparkline(ui: &Ui, frame_times: &[f32; 30], frame_time_idx: usize, rect: egui::Rect) {
        let n = frame_times.len();
        let bar_h_max = (rect.height() - 2.0).max(4.0);
        let bar_stride = 3.0_f32;
        let bar_fill = 2.0;
        let spark_w = px_len(n) * bar_stride;
        let spark_left = rect.right() - spark_w - 4.0;
        let spark_bottom = rect.bottom() - 1.0;
        let spark_top = spark_bottom - bar_h_max;

        let scale_max = 1.0 / 30.0; // 33.3ms — full height

        // Border around sparkline area.
        let border_rect = egui::Rect::from_min_max(
            egui::pos2(spark_left - 1.0, spark_top - 1.0),
            egui::pos2(rect.right() - 3.0, spark_bottom + 1.0),
        );
        ui.painter().rect_stroke(
            border_rect,
            0.0,
            egui::Stroke::new(1.0_f32, BORDER_COLOR),
            egui::StrokeKind::Outside,
        );

        // Threshold gridlines: 60fps (0.5) and 30fps (1.0).
        let grid_stroke = egui::Stroke::new(
            1.0_f32,
            Color32::from_rgba_premultiplied(0x50, 0x50, 0x50, 0x80),
        );
        let label_font = egui::FontId::monospace(7.0);
        let label_color = Color32::from_rgba_premultiplied(0x70, 0x70, 0x70, 0xA0);
        // Labels show ms thresholds (the Y-axis is frame time, not FPS).
        for (label, frac) in [("17ms", 0.5_f32), ("33ms", 1.0_f32)] {
            let y = (spark_bottom - frac * bar_h_max).floor();
            ui.painter()
                .hline(spark_left..=border_rect.right() - 1.0, y, grid_stroke);
            let galley =
                ui.painter()
                    .layout_no_wrap(label.to_owned(), label_font.clone(), label_color);
            ui.painter().galley(
                egui::pos2(
                    spark_left - galley.size().x - 3.0,
                    y - galley.size().y / 2.0,
                ),
                galley,
                label_color,
            );
        }

        // Bars
        let color_good = c(colors::GREEN_50);
        let color_warn = c(colors::YELLOW_30);
        let color_bad = c(colors::RED_60);

        for i in 0..n {
            let idx = (frame_time_idx + i) % n;
            let t = frame_times[idx];
            if t <= 0.0 {
                continue;
            }
            let frac = (t / scale_max).clamp(0.0, 1.0);
            let x = (spark_left + px_len(i) * bar_stride).floor();
            let bar_h = frac * bar_h_max;
            if bar_h < 0.5 {
                continue;
            }
            let color = if t <= 1.0 / 60.0 {
                color_good
            } else if t <= 1.0 / 30.0 {
                color_warn
            } else {
                color_bad
            };
            let bar_top = (spark_bottom - bar_h).floor();
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(x, bar_top),
                egui::vec2(bar_fill, spark_bottom - bar_top),
            );
            ui.painter().rect_filled(bar_rect, 0.0, color);
        }
    }

    /// Height of sub-panel title bars in the bottom panel.
    const SUB_HEADER_H: f32 = 20.0;

    /// Draw a titled sub-panel inside `rect`: header bar + horizontal content `Ui`.
    ///
    /// All layout math stays here — the returned `Ui` is ready for content code
    /// that only cares about *what* to draw, not *where*.
    fn sub_panel(ui: &mut Ui, title: &str, rect: egui::Rect, align: egui::Align) -> Ui {
        let header_bottom = rect.min.y + Self::SUB_HEADER_H;
        let header = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, header_bottom));

        ui.painter().rect_filled(header, 0.0, HEADER_BG);
        ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header.shrink2(egui::vec2(8.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        )
        .label(egui::RichText::new(title).color(Color32::WHITE).size(11.0));

        let content = egui::Rect::from_min_max(egui::pos2(rect.min.x, header_bottom), rect.max);
        ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content.shrink(8.0))
                .layout(egui::Layout::left_to_right(align)),
        )
    }

    /// Render the bottom panel: two sub-panels side by side.
    fn render_bottom_panel(
        ui: &mut Ui,
        anim_paused: &mut bool,
        anim_scrub: &mut f32,
        doc_renderer: &mut Option<DocumentRenderer>,
        icons: &crate::icons::Icons,
        perf: &PerfStats,
    ) {
        let total = ui.available_rect_before_wrap();
        let mid_x = total.min.x + total.width() * 0.5;
        let border = egui::Stroke::new(1.0_f32, BORDER_COLOR);

        ui.painter().hline(total.x_range(), total.min.y, border);
        ui.painter().vline(mid_x, total.y_range(), border);

        let left = egui::Rect::from_min_max(total.min, egui::pos2(mid_x - 1.0, total.max.y));
        let right = egui::Rect::from_min_max(egui::pos2(mid_x + 1.0, total.min.y), total.max);

        let mut left_ui = Self::sub_panel(ui, "Animation Control", left, egui::Align::TOP);
        Self::render_animation_controls(&mut left_ui, anim_paused, anim_scrub, doc_renderer, icons);

        let mut right_ui = Self::sub_panel(ui, "Performance", right, egui::Align::TOP);
        Self::render_performance(&mut right_ui, perf);

        ui.advance_cursor_after_rect(total);
    }

    /// Animation playback controls: play/pause, reset, scrub slider.
    fn render_animation_controls(
        ui: &mut Ui,
        anim_paused: &mut bool,
        anim_scrub: &mut f32,
        doc_renderer: &mut Option<DocumentRenderer>,
        icons: &crate::icons::Icons,
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;

            let (icon, tooltip) = if *anim_paused {
                (&icons.play, "Play")
            } else {
                (&icons.pause, "Pause")
            };
            if ui
                .add(
                    egui::Button::image(crate::icons::icon_image(icon, 10.0, BUTTON_FG))
                        .corner_radius(3.0),
                )
                .on_hover_text(tooltip)
                .clicked()
            {
                *anim_paused = !*anim_paused;
            }
            if ui
                .add(
                    egui::Button::image(crate::icons::icon_image(&icons.renew, 10.0, BUTTON_FG))
                        .corner_radius(3.0),
                )
                .on_hover_text("Reset")
                .clicked()
            {
                *anim_scrub = 0.0;
                if let Some(dr) = doc_renderer.as_mut() {
                    dr.reset_animation_states();
                }
            }

            ui.add_space(4.0);
            ui.visuals_mut().widgets.inactive.bg_fill = c(colors::GRAY_80);
            let slider = egui::Slider::new(anim_scrub, 0.0..=1.0)
                .trailing_fill(true)
                .show_value(false);
            if *anim_paused {
                ui.add(slider);
            } else {
                ui.add_enabled(false, slider);
            }
        });
    }

    /// Performance stats: labeled fps + frame time, then sparkline.
    fn render_performance(ui: &mut Ui, perf: &PerfStats) {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);

        let font = egui::FontId::monospace(9.0);
        let label_color = SHORTCUT_COLOR;
        let value_color = Color32::WHITE;

        // Two-column layout: right-aligned labels | left-aligned values.
        // Use a single LayoutJob per row for precise column alignment.
        let row = |label: &str, value: String| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                label,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: label_color,
                    ..Default::default()
                },
            );
            job.append(
                &format!("  {value}"),
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: value_color,
                    ..Default::default()
                },
            );
            job
        };

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(row("FPS       ", format!("{:>4}", perf.display_fps)));
            ui.label(row("Frame time", format!("{:>5.1} ms", perf.display_ms)));
        });

        Self::render_sparkline(
            ui,
            &perf.frame_times,
            perf.write_idx,
            ui.available_rect_before_wrap(),
        );
    }

    /// Render an error/status message in the preview area (left-aligned, monospace).
    fn render_status_message(ui: &mut Ui, message: &str, default_color: Color32) {
        Frame::NONE
            .inner_margin(Margin::symmetric(16, 16))
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("status_scroll")
                    .show(ui, |ui| {
                        let job = ansi_to_layout_job(message, default_color);
                        ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
                    });
            });
    }

    // ── Panel rendering methods (called from eframe::App::update) ─────

    /// Left sidebar: filter input, story tree, navigation hint footer.
    /// Returns the panel's right edge x coordinate.
    fn render_sidebar(&mut self, ui: &mut egui::Ui) -> f32 {
        let r = egui::SidePanel::left("sidebar")
            .default_width(200.0)
            .frame(Frame::NONE.fill(PANEL_BG))
            .show_inside(ui, |ui| {
                Self::header(ui, "Widget Catalog");
                self.render_filter_input(ui);

                let footer_height = 28.0;
                let tree_height = ui.available_height() - footer_height;

                Frame::NONE
                    .inner_margin(Margin::symmetric(8, 0))
                    .show(ui, |ui| {
                        if self.sidebar.has_visible_items() {
                            egui::ScrollArea::vertical()
                                .id_salt("sidebar_scroll")
                                .max_height(tree_height)
                                .scroll_bar_visibility(
                                    egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                                )
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    if self.sidebar.render(ui, &self.icons) {
                                        self.on_selection_changed();
                                    }
                                });
                        } else if self.build_started.is_some() {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new("Building stories...")
                                    .color(SHORTCUT_COLOR)
                                    .italics(),
                            );
                        } else {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new("No matching stories").color(SHORTCUT_COLOR),
                            );
                        }
                    });

                // Navigation hint footer.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    Frame::NONE
                        .fill(PANEL_BG)
                        .inner_margin(Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                Self::shortcut_pill(ui, "(Shift+)Tab");
                                ui.label(
                                    egui::RichText::new("navigate")
                                        .size(10.0)
                                        .color(SHORTCUT_COLOR),
                                );
                            });
                        });
                });
            });
        r.response.rect.right()
    }

    /// Filter text input with Ctrl+F pill / clear button overlay.
    fn render_filter_input(&mut self, ui: &mut Ui) {
        Frame::NONE
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                let icon_size = 12.0;
                let gap = 8.0;
                let input_w = ui.available_width() - icon_size - gap;
                let inner = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.add(crate::icons::icon_image(
                        &self.icons.search,
                        icon_size,
                        SHORTCUT_COLOR,
                    ));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.sidebar.filter)
                            .hint_text("Filter...")
                            .id(Self::filter_id())
                            .margin(Margin::symmetric(4, 4))
                            .desired_width(input_w),
                    )
                });
                let response = inner.inner;
                if self.sidebar.filter.is_empty() && !response.has_focus() {
                    // Show Ctrl+F pill inside the input's right edge.
                    let galley = ui.painter().layout_no_wrap(
                        "Ctrl+F".to_owned(),
                        egui::FontId::monospace(10.0),
                        SHORTCUT_COLOR,
                    );
                    let pill_size = galley.size() + egui::vec2(8.0, 4.0);
                    let pill_pos = egui::pos2(
                        response.rect.right() - pill_size.x - 4.0,
                        response.rect.center().y - pill_size.y / 2.0,
                    );
                    let pill_rect = egui::Rect::from_min_size(pill_pos, pill_size);
                    ui.painter().rect_filled(pill_rect, 3.0, c(colors::GRAY_80));
                    ui.painter().galley(
                        pill_rect.min + egui::vec2(2.0, 2.0),
                        galley,
                        SHORTCUT_COLOR,
                    );
                } else if !self.sidebar.filter.is_empty() {
                    // Close icon inside the input's right edge.
                    let btn_size = egui::vec2(16.0, 16.0);
                    let btn_pos = egui::pos2(
                        response.rect.right() - btn_size.x - 4.0,
                        response.rect.center().y - btn_size.y / 2.0,
                    );
                    let btn_rect = egui::Rect::from_min_size(btn_pos, btn_size);
                    let close_btn = ui.put(
                        btn_rect,
                        egui::Button::image(crate::icons::icon_image(
                            &self.icons.close,
                            12.0,
                            SHORTCUT_COLOR,
                        ))
                        .frame(false),
                    );
                    if close_btn.clicked() {
                        self.sidebar.filter.clear();
                        response.request_focus();
                    }
                }
            });
    }

    /// Right panel: Controls (top 70%) + Actions log (bottom 30%).
    /// Returns the panel's left edge x coordinate.
    fn render_right_panel(&mut self, ui: &mut egui::Ui) -> f32 {
        let r = egui::SidePanel::right("controls")
            .default_width(250.0)
            .frame(Frame::NONE.fill(PANEL_BG))
            .show_inside(ui, |ui| {
                let total = ui.available_rect_before_wrap();
                let split_y = total.min.y + total.height() * 0.7;
                let controls_rect =
                    egui::Rect::from_min_max(total.min, egui::pos2(total.max.x, split_y));
                let actions_rect =
                    egui::Rect::from_min_max(egui::pos2(total.min.x, split_y), total.max);

                ui.advance_cursor_after_rect(total);

                // Controls (top 70%)
                ui.scope_builder(egui::UiBuilder::new().max_rect(controls_rect), |ui| {
                    Self::header(ui, "Controls");
                    Frame::NONE
                        .inner_margin(Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("controls_scroll")
                                .show(ui, |ui| {
                                    crate::knobs_ui::render_knobs_ui(&mut self.ctx, ui);
                                });
                        });
                });

                // Actions (bottom 30%)
                ui.scope_builder(egui::UiBuilder::new().max_rect(actions_rect), |ui| {
                    self.render_actions_panel(ui);
                });
            });
        r.response.rect.left()
    }

    /// Actions log panel: header with clear button + scrollable event list.
    fn render_actions_panel(&mut self, ui: &mut Ui) {
        Frame::NONE
            .fill(HEADER_BG)
            .inner_margin(Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Actions")
                            .size(16.0)
                            .color(Color32::WHITE),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Clear").clicked() {
                            self.action_log.clear();
                        }
                    });
                });
            });

        Frame::NONE
            .inner_margin(Margin::symmetric(8, 4))
            .show(ui, |ui| {
                let height = ui.available_height();
                egui::ScrollArea::both()
                    .id_salt("actions_scroll")
                    .stick_to_bottom(true)
                    .min_scrolled_height(height)
                    .show(ui, |ui| {
                        ui.set_min_height(height);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if self.action_log.is_empty() {
                            ui.label(
                                egui::RichText::new("Interaction events will appear here")
                                    .color(SHORTCUT_COLOR)
                                    .italics(),
                            );
                        } else {
                            Self::render_action_log(
                                ui,
                                &self.action_log,
                                self.start_time,
                                &self.icons,
                            );
                        }
                    });
            });
    }

    /// Render the action log entries.
    fn render_action_log(
        ui: &mut Ui,
        log: &VecDeque<(Instant, ActionKind, String)>,
        start: Instant,
        icons: &crate::icons::Icons,
    ) {
        for (i, (ts, kind, msg)) in log.iter().enumerate() {
            let bg = if i % 2 == 0 {
                PANEL_BG
            } else {
                crate::to_egui(colors::GRAY_90)
            };
            let elapsed = ts.duration_since(start).as_secs_f64();
            let stamp = format!("  {elapsed:.2}s");
            let icon_tex = match kind {
                ActionKind::Click => &icons.touch,
            };
            Frame::NONE
                .fill(bg)
                .inner_margin(Margin::symmetric(4, 1))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing = egui::vec2(3.0, 0.0);
                    ui.horizontal(|ui| {
                        ui.add(crate::icons::icon_image(icon_tex, 10.0, SHORTCUT_COLOR));
                        let mut job = egui::text::LayoutJob::default();
                        job.append(
                            msg,
                            4.0,
                            egui::TextFormat {
                                font_id: egui::FontId::monospace(10.0),
                                color: c(colors::GRAY_30),
                                ..Default::default()
                            },
                        );
                        job.append(
                            &stamp,
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::monospace(10.0),
                                color: SHORTCUT_COLOR,
                                ..Default::default()
                            },
                        );
                        job.wrap = egui::text::TextWrapping {
                            max_rows: 1,
                            overflow_character: None,
                            ..Default::default()
                        };
                        ui.label(job);
                    });
                });
        }
    }

    /// Run the selected story's render function and produce FBO frames.
    fn render_story_to_fbo(&mut self, frame: &mut eframe::Frame, pixels_per_point: f32) {
        let Some(entry) = self.sidebar.selected() else {
            return;
        };
        self.ctx.begin_frame();

        // Set up asset registrars so the SDK's ensure_*_registered() and the
        // skin system's ensure_nine_patch_registered() call through to the
        // actual renderer during story rendering.
        if let Some(dr) = &mut self.doc_renderer {
            RENDERER_PTR.with(|p| p.set(&raw mut dr.renderer));
            bmc_wasm_sdk::assets::init_icon_registrar(registrar_icon);
            bmc_wasm_sdk::assets::init_bitmap_registrar(registrar_bitmap);
            bmc_wasm_sdk::assets::init_mesh_registrar(registrar_mesh);
            bmc_render_skin::init(registrar_bitmap_nearest);
        }

        // Wrap story render_fn in catch_unwind — a panic in the .so must
        // not crash the shell. Display the panic message instead.
        let render_fn = entry.render_fn;
        let render_result = catch_unwind(AssertUnwindSafe(|| render_fn(&mut self.ctx)));

        // Clear the renderer bridge after story rendering.
        RENDERER_PTR.with(|p| p.set(std::ptr::null_mut()));

        self.ctx.apply_pending_restore();

        match render_result {
            Ok(()) => {
                self.story_error = None;
                self.doc_blocks = self.ctx.take_doc_blocks();

                if let Some(dr) = &mut self.doc_renderer {
                    let delta_ms = if self.anim_paused {
                        // When paused, reset states and render at the scrub
                        // position (0.0–1.0 mapped to 0–10 000 ms).
                        dr.reset_animation_states();
                        px_u(self.anim_scrub * 10_000.0)
                    } else {
                        16
                    };
                    dr.render_doc_blocks(
                        &mut self.doc_blocks,
                        &self.gl,
                        frame,
                        delta_ms,
                        pixels_per_point,
                    );

                    // Aggregate interactions from all frame targets.
                    let matched: Vec<_> = {
                        let actions = self.ctx.actions();
                        dr.targets
                            .iter_mut()
                            .flat_map(|t| t.state.interaction.action_log.drain(..))
                            .filter_map(|event| {
                                tracing::debug!(?event, "action");
                                format_registered_action(&event, actions)
                            })
                            .collect()
                    };
                    for (display, key, sub) in matched {
                        self.action_log
                            .push_back((Instant::now(), ActionKind::Click, display));
                        if self.action_log.len() > MAX_ACTION_LOG_ENTRIES {
                            self.action_log.pop_front();
                        }
                        self.ctx.record_fired_action(key, sub);
                    }

                    // Aggregate drags from all frame targets.
                    let drags: Vec<_> = dr
                        .targets
                        .iter()
                        .flat_map(|t| {
                            t.state
                                .drags
                                .iter()
                                .map(|(key, hit)| (key.clone(), hit.x, hit.width))
                        })
                        .collect();
                    self.ctx.apply_drags(&drags);
                }
            }
            Err(panic) => {
                let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                    (*s).to_owned()
                } else if let Some(s) = panic.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_owned()
                };
                self.story_error = Some(msg);
            }
        }
    }

    /// Preview heading: story name with group breadcrumb + Preview/Source tabs.
    fn render_preview_heading(
        ui: &mut Ui,
        entry: &OwnedStoryEntry,
        group: Option<&str>,
        show_source: &mut bool,
        icons: &crate::icons::Icons,
    ) {
        Frame::NONE
            .fill(HEADER_BG)
            .inner_margin(Margin::symmetric(12, 0))
            .show(ui, |ui| {
                let w = ui.available_width();
                ui.set_min_width(w);
                ui.allocate_ui_with_layout(
                    egui::vec2(w, HEADER_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        // Story title with group breadcrumb.
                        let mut job = egui::text::LayoutJob::default();
                        if entry.default {
                            job.append(
                                group.unwrap_or(&entry.name),
                                0.0,
                                egui::TextFormat {
                                    font_id: egui::FontId::proportional(16.0),
                                    color: Color32::WHITE,
                                    ..Default::default()
                                },
                            );
                        } else {
                            if let Some(group) = group {
                                job.append(
                                    &format!("{} / ", group.replace('/', " / ")),
                                    0.0,
                                    egui::TextFormat {
                                        font_id: egui::FontId::proportional(16.0),
                                        color: GROUP_COLOR,
                                        ..Default::default()
                                    },
                                );
                            }
                            job.append(
                                &entry.name,
                                0.0,
                                egui::TextFormat {
                                    font_id: egui::FontId::proportional(16.0),
                                    color: Color32::WHITE,
                                    ..Default::default()
                                },
                            );
                        }
                        ui.label(job);

                        // Preview / Source tab toggle (right-aligned)
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if ui
                                .add(
                                    egui::Button::image(crate::icons::icon_image(
                                        &icons.code,
                                        14.0,
                                        BUTTON_FG,
                                    ))
                                    .selected(*show_source)
                                    .corner_radius(3.0),
                                )
                                .on_hover_text("Source")
                                .clicked()
                            {
                                *show_source = true;
                            }
                            if ui
                                .add(
                                    egui::Button::image(crate::icons::icon_image(
                                        &icons.color_palette,
                                        14.0,
                                        BUTTON_FG,
                                    ))
                                    .selected(!*show_source)
                                    .corner_radius(3.0),
                                )
                                .on_hover_text("Preview")
                                .clicked()
                            {
                                *show_source = false;
                            }

                            // Debug-outlines checkbox: same flag the wasm
                            // testbed exposes, useful for inspecting layout
                            // boundaries directly in the storybook.
                            ui.add_space(8.0);
                            let mut debug_on = bmc_render::tree::debug_layout_enabled();
                            if ui
                                .checkbox(&mut debug_on, "Debug")
                                .on_hover_text("Toggle layout debug outlines")
                                .changed()
                            {
                                bmc_render::tree::toggle_debug_layout();
                            }
                        });
                    },
                );
            });
    }

    /// Central preview area: story preview, source view, or status messages.
    ///
    /// Painted with `Frame::NONE.fill(PREVIEW_BG)` rather than `CentralPanel` so the
    /// preview background tone is independent of the egui style's central-panel fill.
    fn render_central_panel(&mut self, ui: &mut egui::Ui) {
        egui::Frame::NONE.fill(PREVIEW_BG).show(ui, |ui| {
            self.preview_width = ui.available_width().max(1.0);
            self.preview_height = ui.available_height().max(1.0);

            if let Some(entry) = self.sidebar.selected() {
                let group = self.sidebar.group_title_for(entry);
                Self::render_preview_heading(ui, entry, group, &mut self.show_source, &self.icons);

                if let Some(err) = &self.build_error {
                    Self::render_status_message(
                        ui,
                        &format!("Build failed:\n\n{err}"),
                        c(colors::RED_60),
                    );
                } else if let Some(err) = &self.story_error {
                    Self::render_status_message(
                        ui,
                        &format!("Story panicked:\n\n{err}"),
                        c(colors::RED_60),
                    );
                } else if self.show_source {
                    Self::render_source_view(ui, &entry.source);
                } else if let Some(dr) = self.doc_renderer.as_mut() {
                    Self::render_document(ui, &self.doc_blocks, dr);
                }
            } else if let Some(started) = &self.build_started {
                let elapsed = started.elapsed().as_secs_f32();
                Self::render_status_message(
                    ui,
                    &format!("Building stories... ({elapsed:.1}s)"),
                    SHORTCUT_COLOR,
                );
            } else if let Some(err) = &self.build_error {
                Self::render_status_message(
                    ui,
                    &format!("Build failed:\n\n{err}"),
                    c(colors::RED_60),
                );
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a story from the sidebar");
                });
            }

            self.perf.record(ui.input(|i| i.stable_dt));
        });
    }

    /// Render source code view for the selected story.
    fn render_source_view(ui: &mut Ui, source: &str) {
        let theme = egui_extras::syntax_highlighting::CodeTheme::dark(13.0);
        let layout_job =
            egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &theme, source, "rs");
        Frame::NONE
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("source_scroll")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(layout_job)
                                .wrap_mode(egui::TextWrapMode::Extend)
                                .selectable(true),
                        );
                    });
            });
    }
}

impl eframe::App for StorybookApp {
    /// Non-painting state mutation. eframe 0.34 splits `App` into `logic` + `ui`,
    /// where `logic` is the documented home for state updates and `ui` is the home
    /// for painting. Both are called once per layout pass (typically once per frame,
    /// but more if any widget calls `ctx.request_discard`); inputs consumed via
    /// `consume_key` are removed from the input queue on first read so re-runs see
    /// no event.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.handle_keyboard(ctx);
        self.handle_hot_reload();
        self.render_story_to_fbo(frame, ctx.pixels_per_point());

        // Keep repainting while animations run or a build is in progress.
        if self.sidebar.selected().is_some() || self.build_started.is_some() {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let screen = ui.ctx().screen_rect();

        // Each `Panel::show_inside` advances the parent ui's auto-id counter by one
        // (`ui.new_child`). When a panel is hidden we skip that slot manually so the
        // counter — and every auto-id derived from it downstream in the central area —
        // stays put across toggles. egui itself uses this idiom in
        // `Panel::show_animated_inside`. Without it, toggling a panel shifts every
        // auto-id by one, the rect-changed-id-between-passes debug check fires for one
        // frame on each toggle, and red debug rects flash around interactive widgets.
        let sidebar_edge = if self.show_sidebar {
            self.render_sidebar(ui)
        } else {
            ui.skip_ahead_auto_ids(1);
            screen.left()
        };
        let right_edge = if self.show_right_panel {
            self.render_right_panel(ui)
        } else {
            ui.skip_ahead_auto_ids(1);
            screen.right()
        };

        // Scoped destructuring: bottom panel + toggles need concurrent field
        // access, then release borrows so render_central_panel can take &mut self.
        {
            let Self {
                show_sidebar,
                show_right_panel,
                show_bottom_panel,
                anim_paused,
                anim_scrub,
                doc_renderer,
                icons,
                perf,
                ..
            } = &mut *self;

            // ── Bottom panel (toggleable) ──
            // See sibling skip_ahead_auto_ids comment above `render_sidebar`.
            let bottom_edge = if *show_bottom_panel {
                let r = egui::TopBottomPanel::bottom("bottom_panel")
                    .frame(Frame::NONE.fill(PANEL_BG))
                    .min_height(Self::SUB_HEADER_H + 36.0)
                    .show_inside(ui, |ui| {
                        Self::render_bottom_panel(
                            ui,
                            anim_paused,
                            anim_scrub,
                            doc_renderer,
                            icons,
                            perf,
                        );
                    });
                r.response.rect.top()
            } else {
                ui.skip_ahead_auto_ids(1);
                screen.bottom()
            };

            Self::render_panel_toggles(
                ui.ctx(),
                screen,
                sidebar_edge,
                right_edge,
                bottom_edge,
                icons,
                show_sidebar,
                show_right_panel,
                show_bottom_panel,
            );
        }

        self.render_central_panel(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(entry) = self.sidebar.selected() {
            storage.set_string("selected_story", entry.module_path.clone());
        }
        storage.set_string("filter", self.sidebar.filter.clone());
        storage.set_string("show_sidebar", self.show_sidebar.to_string());
        storage.set_string("show_right_panel", self.show_right_panel.to_string());
        storage.set_string("show_bottom_panel", self.show_bottom_panel.to_string());
    }
}

/// Load stories from the statically-linked inventory (compiled-in stories).
fn load_static_stories() -> (Vec<OwnedStoryEntry>, Vec<OwnedStoryGroupMeta>) {
    let entries = inventory::iter::<StoryEntry>
        .into_iter()
        .map(OwnedStoryEntry::from_static)
        .collect();
    let groups = inventory::iter::<StoryGroupMeta>
        .into_iter()
        .map(OwnedStoryGroupMeta::from_static)
        .collect();
    (entries, groups)
}

/// Format an `ActionEvent` using registered action names (prefix-matched).
///
/// Composite widgets generate sub-keys from a parent key (e.g. `"{key}_minus"`).
/// Prefix matching ensures sub-element interactions are captured under the
/// parent action name. Returns `(display_string, action_key, sub_suffix)`,
/// or `None` if no registered action matches.
fn format_registered_action(
    event: &ActionEvent,
    actions: &[bmc_storybook_api::knobs::Action],
) -> Option<(String, String, String)> {
    let (event_key, kind) = match event {
        ActionEvent::Click { key, pos } => {
            let detail = pos.map_or(String::new(), |(x, y)| format!("  ({x:.0}, {y:.0})"));
            (key.as_str(), format!("click{detail}"))
        }
        ActionEvent::Scroll { key, delta } => (key.as_str(), format!("scroll  \u{0394}y={delta}")),
    };
    actions
        .iter()
        .find(|a| event_key.starts_with(a.key.as_str()))
        .map(|a| {
            let sub = event_key.get(a.key.len()..).unwrap_or("").to_owned();
            let display = if sub.is_empty() {
                format!("{}  {kind}", a.label)
            } else {
                format!("{}  {kind}  [{sub}]", a.label)
            };
            (display, a.key.clone(), sub)
        })
}

/// Convert a string with ANSI escape codes into an egui `LayoutJob` with colored spans.
fn ansi_to_layout_job(text: &str, default_color: Color32) -> egui::text::LayoutJob {
    let spans = crate::ansi::parse(text);

    let mono = egui::FontId::monospace(13.0);
    let mut job = egui::text::LayoutJob::default();

    let mono_bold = egui::FontId {
        size: mono.size,
        family: egui::FontFamily::Monospace,
    };
    for span in &spans {
        let color = span.color.unwrap_or(default_color);
        let font_id = if span.bold {
            mono_bold.clone()
        } else {
            mono.clone()
        };
        job.append(
            &span.text,
            0.0,
            egui::TextFormat {
                font_id,
                color,
                ..Default::default()
            },
        );
    }

    job
}

// ── macOS color space workaround ────────────────────────────────────
//
// egui's glow backend does not call `[NSWindow setColorSpace:]`, so on
// displays with a wide-gamut native color space (Display P3, common on
// modern Macs) sRGB framebuffer values are misinterpreted and the whole
// window appears washed-out.  Setting the color space to sRGB tells the
// macOS compositor to color-match correctly.
//
// Tracked upstream: <https://github.com/emilk/egui/issues/2712>

#[cfg(target_os = "macos")]
#[expect(unsafe_code)]
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

        // [ns_view window] — may be nil before the view is installed.
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
