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
/// Dim olive — every match in the visible buffer gets this so the eye picks
/// up the constellation of hits without one stealing the show.
const SEARCH_HIGHLIGHT_BG: egui::Color32 = egui::Color32::from_rgb(70, 60, 20);
/// Saturated orange — only the row the search cursor currently sits on.
/// Picked for a deliberate hue + brightness delta against the dim olive
/// so the active match is unmistakable even on small monitors.
const SEARCH_CURRENT_BG: egui::Color32 = egui::Color32::from_rgb(255, 160, 30);
const FONT_SIZE: f32 = 11.0;
/// Per-row line height used to translate a target match index into a `ScrollArea::vertical_scroll_offset`.
/// Mono font at `FONT_SIZE` with zero item spacing renders one log line at roughly this height;
/// if the row layout above changes (font size, padding), bump this in lockstep.
const ROW_HEIGHT: f32 = FONT_SIZE * 1.3;

/// Per-tab ring buffer cap: oldest lines get dropped past this.
const MAX_LINES_PER_TAB: usize = 20_000;
/// How many lines to drop at once when the cap is exceeded. Amortizes the
/// cost of the shift so we don't pay O(n) per push.
const DROP_CHUNK: usize = 1_000;

fn log_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("log_viewport")
}

/// Find-in-page state for the log viewport. Lives inside [`LogState`]
/// but stays self-contained so the search UI can be reasoned about independently
/// of buffers and tabs. The input is always visible; Ctrl/Cmd+F just grabs
/// focus and Esc clears the query.
#[derive(Default)]
struct SearchState {
    /// Current query; empty means "no highlights, no navigation".
    query: String,
    /// Bumped to ask the input to grab focus on the next frame, e.g. right
    /// after Ctrl/Cmd+F was pressed.
    focus_request: bool,
    /// Row index inside the active tab to jump to via
    /// `ScrollArea::vertical_scroll_offset` on the next frame.
    pending_scroll: Option<usize>,
    /// Cursor into the per-frame match list (0-based). Wraps on next/prev.
    cursor: usize,
    /// Step requested by either Enter/Shift+Enter or the prev/next buttons.
    /// Consumed at the end of the frame so the source (key vs click) doesn't
    /// matter to the navigation logic.
    nav: Option<NavDir>,
}

#[derive(Clone, Copy)]
enum NavDir {
    Next,
    Prev,
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
    search: SearchState,
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
                search: SearchState::default(),
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

                // Ctrl/Cmd+F focuses the always-visible search input.
                // `command` is Cmd on macOS and Ctrl elsewhere;
                // consuming the event keeps the TextEdit from seeing a stray `f` keystroke.
                let focus_search =
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
                if focus_search {
                    state
                        .lock()
                        .expect("BUG: log state mutex poisoned")
                        .search
                        .focus_request = true;
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

    let tab = state.active_tab;

    // Scan the active tab's buffer for case-insensitive literal substring
    // matches up-front so the search bar can show the n/m counter and the row
    // renderer can highlight matched chars without re-scanning per row.
    // ANSI escapes are stripped so positions align with visible characters.
    let matches = if state.search.query.is_empty() {
        Vec::new()
    } else {
        scan_literal_matches(&state.buffers[tab], &state.search.query)
    };
    state.search.cursor = matches
        .len()
        .checked_sub(1)
        .map_or(0, |last| state.search.cursor.min(last));
    let current_match_row = matches.get(state.search.cursor).map(|m| m.line);

    // Tab bar (with optional search bar on the right)
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
        // Right-to-left layout: widgets are added right-edge first, so auto-scroll lands
        // on the far right and the search bar pieces (input + counter + clear button)
        // fill the space to its left.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut state.auto_scroll, "Auto-scroll");
            ui.separator();
            render_search_bar(ui, state, matches.len());
        });
    });
    ui.add_space(2.0);

    let buf = &state.buffers[tab];
    let line_count = buf.len();
    let line_offset = state.line_offsets[tab];
    let max_line_num = line_offset + line_count;
    let line_num_width = format!("{max_line_num}").len().max(3);

    // Build a sparse "line idx → matched char positions" map so
    // the row closure can look up highlights without a linear scan.
    // Vec lookups by searching `matches` for the row would be O(matches) per render call.
    let match_lookup: std::collections::HashMap<usize, &[usize]> = matches
        .iter()
        .map(|m| (m.line, m.chars.as_slice()))
        .collect();

    let scroll_target = state.search.pending_scroll.take();
    let mut scroll_area = egui::ScrollArea::both()
        .auto_shrink([false, false])
        .stick_to_bottom(state.auto_scroll && scroll_target.is_none())
        .drag_to_scroll(false);
    if let Some(idx) = scroll_target {
        // Match indices map to absolute pixel offsets because rows don't wrap
        // (`TextWrapMode::Extend` below). Centring is approximate but cheap.
        // The MAX_LINES_PER_TAB cap (20k) sits far below the f32 mantissa
        // range, so the cast is exact for any in-buffer index.
        #[expect(
            clippy::cast_precision_loss,
            reason = "idx is bounded by MAX_LINES_PER_TAB (20k) ≪ 2^23"
        )]
        let offset = (idx as f32 * ROW_HEIGHT - 80.0).max(0.0);
        scroll_area = scroll_area.vertical_scroll_offset(offset);
    }
    scroll_area.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ui.set_width(ui.available_width());

        state.virtual_lists[tab].ui_custom_layout(ui, line_count, |ui, index| {
            render_log_row(
                ui,
                &buf[index],
                line_offset + index + 1,
                line_num_width,
                index % 2 == 0,
                &mono,
                &RowHighlight {
                    chars: match_lookup.get(&index).copied().unwrap_or(&[]),
                    is_cursor: current_match_row == Some(index),
                },
            );
            1
        });
    });

    // Stash navigation requests for the *next* frame so the freshly computed
    // matches above already reflect the user's just-typed query. Sources:
    // Enter / Shift+Enter from the keyboard, or the prev/next buttons
    // that wrote into `search.nav` during `render_search_bar`.
    if !matches.is_empty() {
        let total = matches.len();
        let key_nav = ui.input(|i| {
            let dir = if i.modifiers.shift {
                NavDir::Prev
            } else {
                NavDir::Next
            };
            i.key_pressed(egui::Key::Enter).then_some(dir)
        });
        if let Some(dir) = state.search.nav.take().or(key_nav) {
            let cur = state.search.cursor;
            state.search.cursor = match dir {
                NavDir::Next => (cur + 1) % total,
                NavDir::Prev => (cur + total - 1) % total,
            };
            state.search.pending_scroll = Some(matches[state.search.cursor].line);
        }
    }
}

/// One literal match against a single log line: the row index,
/// plus the character positions (0-based, in the ANSI-stripped
/// visible text) that the substring(s) covered. A line containing
/// two occurrences contributes `2 × needle_chars` entries here.
struct LineMatch {
    line: usize,
    chars: Vec<usize>,
}

/// Scan a tab's buffer for case-insensitive substring matches against `query`.
/// Returns matches in buffer order so n/m navigation walks top-to-bottom.
/// Works on char positions rather than byte indices so non-ASCII visible text
/// doesn't trip clippy's `string_slice` lint and so queries with multi-byte chars
/// stay aligned with the per-span highlight renderer.
fn scan_literal_matches(buf: &[String], query: &str) -> Vec<LineMatch> {
    let needle: Vec<char> = query.chars().map(|c| c.to_ascii_lowercase()).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    buf.iter()
        .enumerate()
        .filter_map(|(line_idx, raw)| {
            let visible: Vec<char> = strip_ansi(raw).chars().collect();
            if visible.len() < needle.len() {
                return None;
            }
            let mut positions = Vec::new();
            let mut i = 0;
            while i + needle.len() <= visible.len() {
                let matched =
                    (0..needle.len()).all(|j| visible[i + j].to_ascii_lowercase() == needle[j]);
                if matched {
                    positions.extend(i..i + needle.len());
                    i += needle.len();
                } else {
                    i += 1;
                }
            }
            (!positions.is_empty()).then_some(LineMatch {
                line: line_idx,
                chars: positions,
            })
        })
        .collect()
}

/// Strip CSI escape sequences (`ESC [ ... <letter>`) so fuzzy-match positions
/// line up with the ANSI-parsed `Span` texts the row renderer hands to egui.
/// Other ANSI flavours (OSC, DCS) don't appear in our logs.
fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI: consume everything up to and including the final letter.
            for inner in chars.by_ref() {
                if inner.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Per-row search-highlight context handed to [`render_log_row`].
/// Separating `chars` from `is_cursor` (matched char positions vs
/// "this is the row Enter last landed on") keeps the row-rendering
/// signature compact and the call site readable.
struct RowHighlight<'a> {
    chars: &'a [usize],
    is_cursor: bool,
}

fn render_log_row(
    ui: &mut egui::Ui,
    line: &str,
    line_num: usize,
    line_num_width: usize,
    even_row: bool,
    mono: &egui::FontId,
    highlight: &RowHighlight<'_>,
) {
    let bg = if even_row { ZEBRA_EVEN } else { ZEBRA_ODD };
    let mut job = egui::text::LayoutJob::default();

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

    // Walk ANSI spans, splitting each into highlighted vs base char runs based
    // on the match-position set. `char_offset` tracks our position in the visible
    // (ANSI-stripped) line so per-span lookups stay aligned.
    //
    // The cursor row's background is bright orange — any light ANSI colour
    // on top would smear. Force the highlighted chars to near-black there so
    // the active match always reads cleanly; non-cursor matches keep their ANSI
    // colour over the dim olive background, which preserves the row's tone.
    let (highlight_bg, highlight_fg) = if highlight.is_cursor {
        (SEARCH_CURRENT_BG, Some(egui::Color32::from_rgb(20, 20, 20)))
    } else {
        (SEARCH_HIGHLIGHT_BG, None)
    };
    let spans = crate::ansi::parse(line);
    let mut char_offset = 0;
    for span in &spans {
        let span_char_len = span.text.chars().count();
        let mut color = span.color.unwrap_or(DEFAULT_TEXT_COLOR);
        if span.bold {
            color = egui::Color32::from_rgb(
                color.r().saturating_add(40),
                color.g().saturating_add(40),
                color.b().saturating_add(40),
            );
        }
        append_span_with_highlights(
            &mut job,
            &span.text,
            mono,
            color,
            highlight_bg,
            highlight_fg,
            highlight.chars,
            char_offset,
        );
        char_offset += span_char_len;
    }

    // Reserve a shape slot so the zebra rect paints *under* the label.
    // We need the label's rect to know the row height, and we need
    // row-level (not glyph-level) fill to span the full width.
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
}

/// Append `span_text` to `job`, splitting it into highlighted vs base runs
/// based on which character indices in `match_chars` fall inside this span.
/// `span_char_offset` is the span's start position in the visible line,
/// shifting all match positions into span-local space.
#[expect(
    clippy::too_many_arguments,
    reason = "split would force a struct-per-call-site since every arg comes from a different source"
)]
fn append_span_with_highlights(
    job: &mut egui::text::LayoutJob,
    span_text: &str,
    mono: &egui::FontId,
    color: egui::Color32,
    highlight_bg: egui::Color32,
    highlight_fg: Option<egui::Color32>,
    match_chars: &[usize],
    span_char_offset: usize,
) {
    let base_fmt = egui::text::TextFormat {
        font_id: mono.clone(),
        color,
        ..Default::default()
    };
    if match_chars.is_empty() {
        job.append(span_text, 0.0, base_fmt);
        return;
    }
    let highlight_fmt = egui::text::TextFormat {
        font_id: mono.clone(),
        color: highlight_fg.unwrap_or(color),
        background: highlight_bg,
        ..Default::default()
    };
    // Walk chars batched into contiguous highlighted / non-highlighted runs.
    // Single-char spans would explode the LayoutJob; batching keeps the span
    // count proportional to highlight runs, not text length.
    let mut run_start_byte = 0;
    let mut run_is_hl: Option<bool> = None;
    let mut chars = span_text.char_indices();
    let mut last_byte = 0;
    for (local_idx, (byte_idx, _ch)) in chars.by_ref().enumerate() {
        let global_idx = span_char_offset + local_idx;
        let is_hl = match_chars.binary_search(&global_idx).is_ok();
        match run_is_hl {
            None => {
                run_is_hl = Some(is_hl);
                run_start_byte = byte_idx;
            }
            Some(prev) if prev != is_hl => {
                let fmt = if prev {
                    highlight_fmt.clone()
                } else {
                    base_fmt.clone()
                };
                let slice = span_text
                    .get(run_start_byte..byte_idx)
                    .expect("BUG: char_indices yields valid char boundaries");
                job.append(slice, 0.0, fmt);
                run_is_hl = Some(is_hl);
                run_start_byte = byte_idx;
            }
            Some(_) => {}
        }
        last_byte = byte_idx;
    }
    // Flush the final run. `last_byte` points at the start of the last char;
    // span_text.len() is its end (one past the final byte).
    if let Some(is_hl) = run_is_hl {
        let _ = last_byte;
        let fmt = if is_hl { highlight_fmt } else { base_fmt };
        let slice = span_text
            .get(run_start_byte..)
            .expect("BUG: run_start_byte is a valid char boundary");
        job.append(slice, 0.0, fmt);
    }
}

/// Always-visible search row sitting next to the Auto-scroll checkbox.
/// Parent layout is right-to-left, so widgets are added in the order
/// clear-button → counter → input and read input → counter → clear-button
/// from left to right on screen. `total_matches` is the pre-computed match
/// count from `render_log_panel` so this widget doesn't double-scan.
fn render_search_bar(ui: &mut egui::Ui, state: &mut LogState, total_matches: usize) {
    let escape_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));
    // Glyphs courtesy of the bundled Noto Sans (see `install_noto_sans` in
    // app.rs); egui's default Ubuntu-Light renders these as tofu.
    let clear_clicked = ui.small_button("×").clicked();
    let nav_enabled = total_matches > 0;
    let next_clicked = ui
        .add_enabled(nav_enabled, egui::Button::new("▶").small())
        .clicked();
    let prev_clicked = ui
        .add_enabled(nav_enabled, egui::Button::new("◀").small())
        .clicked();
    if next_clicked {
        state.search.nav = Some(NavDir::Next);
    } else if prev_clicked {
        state.search.nav = Some(NavDir::Prev);
    }
    if (clear_clicked || escape_pressed) && !state.search.query.is_empty() {
        state.search.query.clear();
    }
    if !state.search.query.is_empty() {
        let counter = if total_matches == 0 {
            "0 matches".to_owned()
        } else {
            format!("{}/{}", state.search.cursor + 1, total_matches)
        };
        ui.colored_label(LINE_NUM_COLOR, counter);
    }
    let response = ui.add(
        egui::TextEdit::singleline(&mut state.search.query)
            .id_salt("log_search_input")
            .desired_width(180.0)
            .hint_text("Find (Ctrl/Cmd+F)"),
    );
    if std::mem::take(&mut state.search.focus_request) {
        response.request_focus();
    }
}
