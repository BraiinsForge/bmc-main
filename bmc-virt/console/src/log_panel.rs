// Copyright (C) 2026  Braiins Systems s.r.o.

// Log viewer panel: displays log entries received via IPC with ANSI color
// support, zebra striping, and line numbers. Uses VirtualList for efficient
// rendering of large log buffers with stable scroll position.
//
// Rendered as a separate OS window via egui's deferred viewport API.

use bmc_virt_ipc::{ALL_LOG_SOURCES, LogSource};
use egui_virtual_list::VirtualList;
use std::sync::{Arc, Mutex};

const ZEBRA_EVEN: egui::Color32 = egui::Color32::from_rgb(30, 30, 34);
const ZEBRA_ODD: egui::Color32 = egui::Color32::from_rgb(36, 36, 40);
const LINE_NUM_COLOR: egui::Color32 = egui::Color32::from_rgb(90, 90, 100);
const DEFAULT_TEXT_COLOR: egui::Color32 = egui::Color32::from_rgb(200, 200, 200);
const FONT_SIZE: f32 = 11.0;

/// Per-tab ring buffer cap: oldest lines get dropped past this.
const MAX_LINES_PER_TAB: usize = 20_000;
/// How many lines to drop at once when the cap is exceeded. Amortizes the
/// cost of the shift so we don't pay O(n) per push.
const DROP_CHUNK: usize = 1_000;

fn log_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("log_viewport")
}

/// Shared state between the main app (writer) and the log viewport (reader).
struct LogState {
    buffers: [Vec<String>; 4],
    /// First line number represented by `buffers[i][0]` — bumps when we drop
    /// old lines so the displayed numbers keep increasing monotonically.
    line_offsets: [usize; 4],
    active_tab: usize,
    auto_scroll: bool,
    /// Set by the viewport callback when the user clicks the close button.
    close_requested: bool,
    /// One VirtualList per source tab — each tracks its own scroll/row state.
    virtual_lists: [VirtualList; 4],
    /// Captured on the first viewport frame so writers can wake it on push().
    viewport_ctx: Option<egui::Context>,
}

pub struct LogPanel {
    state: Arc<Mutex<LogState>>,
}

impl LogPanel {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(LogState {
                buffers: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
                line_offsets: [0; 4],
                active_tab: 0,
                auto_scroll: true,
                close_requested: false,
                virtual_lists: [
                    VirtualList::new(),
                    VirtualList::new(),
                    VirtualList::new(),
                    VirtualList::new(),
                ],
                viewport_ctx: None,
            })),
        }
    }

    /// Push a log line received from the IPC channel.
    pub fn push(&mut self, source: LogSource, line: String) {
        let mut state = self.state.lock().expect("BUG: log state mutex poisoned");
        let idx = source as usize;
        if idx >= state.buffers.len() {
            return;
        }
        state.buffers[idx].push(line);
        if state.buffers[idx].len() > MAX_LINES_PER_TAB {
            let drop_n = DROP_CHUNK.min(state.buffers[idx].len());
            state.buffers[idx].drain(..drop_n);
            state.line_offsets[idx] = state.line_offsets[idx].saturating_add(drop_n);
            // Draining shifts all indices — the VirtualList's row-height cache
            // is now misaligned; reset it so heights get re-measured cleanly.
            state.virtual_lists[idx] = VirtualList::new();
        }
        if let Some(ctx) = &state.viewport_ctx {
            // request_repaint() targets the *current* viewport, which is the
            // main one when push() is called from the main app's frame.
            // We need to specifically address the log viewport.
            ctx.request_repaint_of(log_viewport_id());
        }
    }

    /// Show the log panel as a separate OS window (deferred viewport).
    pub fn show(&self, ctx: &egui::Context) {
        let state = Arc::clone(&self.state);
        ctx.show_viewport_deferred(
            log_viewport_id(),
            egui::ViewportBuilder::default()
                .with_title("Logs")
                .with_inner_size([900.0, 500.0])
                .with_minimize_button(false)
                .with_maximize_button(false),
            move |ui, _class| {
                // Detect window close button
                if ui.input(|i| i.viewport().close_requested()) {
                    state
                        .lock()
                        .expect("BUG: log state mutex poisoned")
                        .close_requested = true;
                    return;
                }

                // Solid background — without this the viewport is transparent
                // during drag and compositing.
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::NONE
                            .fill(ui.style().visuals.panel_fill)
                            .inner_margin(egui::Margin::symmetric(8, 0)),
                    )
                    .show_inside(ui, |ui| {
                        let mut state = state.lock().expect("BUG: log state mutex poisoned");
                        // Cache the viewport's context so push() can wake it.
                        if state.viewport_ctx.is_none() {
                            state.viewport_ctx = Some(ui.ctx().clone());
                        }
                        render_log_panel(ui, &mut state);
                    });
            },
        );
    }

    /// Returns true and resets the flag if the user closed the log viewport.
    pub fn take_close_requested(&self) -> bool {
        let mut state = self.state.lock().expect("BUG: log state mutex poisoned");
        std::mem::replace(&mut state.close_requested, false)
    }
}

fn render_log_panel(ui: &mut egui::Ui, state: &mut LogState) {
    let mono = egui::FontId::monospace(FONT_SIZE);

    ui.spacing_mut().item_spacing.y = 0.0;

    // Tab bar
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (idx, source) in ALL_LOG_SOURCES.iter().enumerate() {
            let count = state.buffers[idx].len();
            let name = source.name();
            let label = if count > 0 {
                format!("{name} ({count})")
            } else {
                name.to_owned()
            };
            if ui
                .selectable_label(state.active_tab == idx, label)
                .clicked()
            {
                state.active_tab = idx;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut state.auto_scroll, "Auto-scroll");
        });
    });
    ui.add_space(2.0);

    let tab = state.active_tab;
    let buf = &state.buffers[tab];
    let line_count = buf.len();
    let line_offset = state.line_offsets[tab];
    let max_line_num = line_offset + line_count;
    let line_num_width = format!("{max_line_num}").len().max(3);

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .stick_to_bottom(state.auto_scroll)
        .drag_to_scroll(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.set_width(ui.available_width());

            state.virtual_lists[tab].ui_custom_layout(ui, line_count, |ui, index| {
                let line = &buf[index];
                let line_num = line_offset + index + 1;
                let bg = if index % 2 == 0 {
                    ZEBRA_EVEN
                } else {
                    ZEBRA_ODD
                };

                let mut job = egui::text::LayoutJob::default();

                // Line number
                #[expect(
                    clippy::uninlined_format_args,
                    reason = "dynamic width$ specifier can't be inlined"
                )]
                let num_str = format!("{line_num:>width$} ", width = line_num_width);
                job.append(
                    &num_str,
                    0.0,
                    egui::text::TextFormat {
                        font_id: mono.clone(),
                        color: LINE_NUM_COLOR,
                        ..Default::default()
                    },
                );

                // ANSI-parsed text spans
                let spans = crate::ansi::parse(line);
                for span in &spans {
                    let mut color = span.color.unwrap_or(DEFAULT_TEXT_COLOR);
                    if span.bold {
                        color = egui::Color32::from_rgb(
                            color.r().saturating_add(40),
                            color.g().saturating_add(40),
                            color.b().saturating_add(40),
                        );
                    }
                    job.append(
                        &span.text,
                        0.0,
                        egui::text::TextFormat {
                            font_id: mono.clone(),
                            color,
                            ..Default::default()
                        },
                    );
                }

                // Reserve a shape slot so the zebra rect paints *under* the
                // label. We need the label's rect to know the row height, and
                // we need row-level (not glyph-level) fill to span full width.
                let row_left = ui.cursor().left();
                let available = ui.available_width();
                let bg_slot = ui.painter().add(egui::Shape::Noop);
                let label_resp = ui.label(job);
                let row_width = available.max(label_resp.rect.width());
                let row_rect = egui::Rect::from_min_size(
                    egui::pos2(row_left, label_resp.rect.top()),
                    egui::vec2(row_width, label_resp.rect.height()),
                );
                ui.painter()
                    .set(bg_slot, egui::epaint::RectShape::filled(row_rect, 0.0, bg));
                1
            });
        });
}
