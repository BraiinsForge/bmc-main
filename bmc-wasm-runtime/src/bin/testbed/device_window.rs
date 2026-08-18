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

//! The floating device window and the frames inside it.
//!
//! A frame is one whole state the physical display can be in. A fullscreen
//! viewport fills the display alone, so it gets a frame of its own; the
//! slot-spanning viewports coexist on the slot grid, so they share one.
//! Every viewport of the platform appears in exactly one frame, and all of
//! them stay live at once — the point of the preview is watching one wasm
//! change render everywhere it can land.
//!
//! The states are alternatives the one display cannot be in at once, so they
//! stack inside a single window per device rather than floating separately —
//! two windows read as two devices. Each keeps its own LED strip: the views
//! of a state drive their own runtimes, and the strip is how that state's
//! effect shows.

use bmc_wasm_runtime::platform_catalog::{self, Platform};

use super::{TestbedApp, dispatch_touch_events, paint, paint_placeholder, toolbar};

/// One display state of the device: which viewports show, and where.
pub(crate) struct DeviceFrame {
    /// The full display area, in logical pixels.
    pub(crate) screen: egui::Vec2,
    /// Viewport index paired with its rect in screen-local coordinates.
    pub(crate) views: Vec<(usize, egui::Rect)>,
}

/// Derive the frames for `platform` from its catalog facts.
pub(crate) fn device_frames(platform: &Platform) -> Vec<DeviceFrame> {
    use bmc_wasm_runtime::platform_catalog::Placement;

    let display = platform.display();
    let screen = egui::vec2(display.logical_width as f32, display.logical_height as f32);

    let mut frames = Vec::new();
    for (idx, viewport) in platform.viewports.iter().enumerate() {
        if matches!(viewport.placement, Placement::Fullscreen) {
            frames.push(DeviceFrame {
                screen,
                views: vec![(idx, egui::Rect::from_min_size(egui::Pos2::ZERO, screen))],
            });
        }
    }
    if let Some(slots) = slot_frame(platform, screen) {
        frames.push(slots);
    }
    frames
}

/// Place the slot-spanning viewports on the platform's grid, first-fit in
/// catalog order. The catalog orders them largest first, which packs the
/// grid the way the device's own layout engine does.
fn slot_frame(platform: &Platform, screen: egui::Vec2) -> Option<DeviceFrame> {
    use bmc_wasm_runtime::platform_catalog::Placement;

    let grid = platform.slot_grid()?;
    let cell = egui::vec2(screen.x / grid.columns as f32, screen.y / grid.rows as f32);
    let mut occupied = vec![false; grid.columns * grid.rows];
    let mut views = Vec::new();

    for (idx, viewport) in platform.viewports.iter().enumerate() {
        let Placement::SlotSpan(span) = viewport.placement else {
            continue;
        };
        let Some((col, row)) =
            first_fit(&occupied, grid.columns, grid.rows, span.columns, span.rows)
        else {
            tracing::warn!(
                viewport = viewport.id,
                "viewport does not fit the slot grid; skipping in the Slots frame"
            );
            continue;
        };
        for r in row..row + span.rows {
            for c in col..col + span.columns {
                occupied[r * grid.columns + c] = true;
            }
        }
        // The viewport's own pixel size, not the span's: real widgets are a
        // hair smaller than their slots, and the sliver between them is the
        // gap the device shows too.
        views.push((
            idx,
            egui::Rect::from_min_size(
                egui::pos2(col as f32 * cell.x, row as f32 * cell.y),
                egui::vec2(viewport.width as f32, viewport.height as f32),
            ),
        ));
    }
    if views.is_empty() {
        return None;
    }

    Some(DeviceFrame { screen, views })
}

/// Top-left-most position where a `span_cols`×`span_rows` block fits.
fn first_fit(
    occupied: &[bool],
    columns: usize,
    rows: usize,
    span_cols: usize,
    span_rows: usize,
) -> Option<(usize, usize)> {
    for row in 0..rows.checked_sub(span_rows.saturating_sub(1))? {
        for col in 0..columns.checked_sub(span_cols.saturating_sub(1))? {
            let free = (row..row + span_rows)
                .all(|r| (col..col + span_cols).all(|c| !occupied[r * columns + c]));
            if free {
                return Some((col, row));
            }
        }
    }
    None
}

/// Bare enclosure showing between the screen's bottom edge and the LED diffuser.
const STRIP_SEAM: f32 = 4.0;
/// The diffuser's rounded ends.
const STRIP_RADIUS: f32 = 3.0;
/// Height of the hand-painted title strip. Ours rather than egui's,
/// because egui hard-centres its window titles.
const TITLE_H: f32 = 24.0;
/// Title text size, and its inset from the strip's leading edge.
const TITLE_FONT: f32 = 12.0;
const TITLE_PAD: f32 = 8.0;

/// Leading between the choose card's two lines.
const LINE_GAP: f32 = 2.0;

/// Bare canvas between one display state and the next. Chrome, so it does
/// not scale with the zoom — the seam is the only thing saying that two
/// alternatives of one display are not one tall screen.
const STATE_GAP: f32 = 4.0;

/// Between a view and its neighbour on a shared screen, and under the last
/// LED strip as the casing below it. Device pixels, so it scales with the
/// rest of the mock.
const STATE_ITEM_GAP: f32 = 8.0;

/// How long the title strip takes to reach its hovered tone.
const HOVER_FADE_SECS: f32 = 0.12;

/// How far towards it the strip travels: the border holds at the resting
/// tone, so a full step re-opens the seam they share a colour to close.
const HOVER_STRENGTH: f32 = 0.5;

/// egui reports a window a pixel larger than the frame it paints,
/// so the stroke needs no inset to show; adding one leaves the shadow
/// as a ring between border and content.
const WINDOW_INSET: egui::Margin = egui::Margin::ZERO;

/// Outline a rect the device mock painted rather than allocated.
///
/// egui's own inspector only knows widgets, so the enclosure, the views
/// and the LED strips are invisible to it — exactly the geometry worth
/// doubting when a mock looks wrong. Under Debug they outline themselves.
fn debug_outline(ui: &egui::Ui, rect: egui::Rect, what: &str, anchor: egui::Align2) {
    const MARK: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x00, 0xFF);

    if !bmc_render::tree::debug_layout_enabled() {
        return;
    }
    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, MARK),
        egui::StrokeKind::Outside,
    );
    // Nested rects share a corner, so each kind is anchored to one of its own
    // rather than overprinting the mark it is measured against.
    ui.painter().text(
        anchor.pos_in_rect(&rect),
        anchor,
        format!(
            "{what} {:.0}×{:.0} y{:.0}..{:.0}",
            rect.width(),
            rect.height(),
            rect.top(),
            rect.bottom(),
        ),
        egui::FontId::monospace(9.0),
        MARK,
    );
}

/// Views that sit above one another: two share a column when their
/// horizontal extents overlap. Each column is listed top to bottom.
fn columns(frame: &DeviceFrame) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..frame.views.len()).collect();
    order.sort_by(|a, b| {
        let (a, b) = (frame.views[*a].1, frame.views[*b].1);
        a.min
            .x
            .total_cmp(&b.min.x)
            .then(a.min.y.total_cmp(&b.min.y))
    });

    let mut columns: Vec<Vec<usize>> = Vec::new();
    for idx in order {
        let rect = frame.views[idx].1;
        let column = columns.iter_mut().find(|column| {
            column.iter().any(|&other| {
                let other = frame.views[other].1;
                other.min.x < rect.max.x && rect.min.x < other.max.x
            })
        });
        match column {
            Some(column) => column.push(idx),
            None => columns.push(vec![idx]),
        }
    }
    columns
}

/// Where each view sits inside its state, and how tall that makes the state.
///
/// A view's LED strip is drawn beneath it, so a column stacks view-and-strip
/// pairs with a gap between them rather than following the device's own grid,
/// which has no room for the strips. The tallest column sets the height,
/// and a shorter one spreads its slack evenly above, between and below
/// its views — pooled at the bottom it reads as a misalignment rather than as air.
fn state_layout(frame: &DeviceFrame, strip: f32) -> (Vec<egui::Pos2>, egui::Vec2) {
    // Only a shared screen needs spacing, and only between its views. Nothing
    // needs it against the enclosure: the device's glass runs to its own edge,
    // and an inset there just frames the widget in bezel.
    let gap = if frame.views.len() > 1 {
        STATE_ITEM_GAP
    } else {
        0.0
    };

    let columns = columns(frame);
    let content: Vec<f32> = columns
        .iter()
        .map(|column| {
            let stacked: f32 = column
                .iter()
                .map(|&idx| frame.views[idx].1.height() + strip)
                .sum();
            stacked + gap * (column.len() as f32 - 1.0)
        })
        .collect();
    let height = content.iter().copied().fold(frame.screen.y, f32::max);
    // The device's own columns sit a sliver apart, and that sliver is lost
    // once each view carries a strip; the gap that separates them vertically
    // separates them sideways too.
    let spread = gap * (columns.len() as f32 - 1.0).max(0.0);

    let mut places = vec![egui::Pos2::ZERO; frame.views.len()];
    for (order, (column, content)) in columns.iter().zip(&content).enumerate() {
        let lead = (height - content).max(0.0) / (column.len() as f32 + 1.0);
        let mut y = lead;
        for (place, &idx) in column.iter().enumerate() {
            if place > 0 {
                y += gap;
            }
            let view = frame.views[idx].1;
            places[idx] = egui::pos2(view.min.x + gap * order as f32, y);
            y += view.height() + strip + lead;
        }
    }
    // The one inset that is not decoration: flush against the enclosure's
    // bottom edge a strip reads as a bar cropped along its length rather than
    // as the diffuser it is. A state with no strip needs none of it.
    let casing = if strip > 0.0 { STATE_ITEM_GAP } else { 0.0 };
    (places, egui::vec2(frame.screen.x + spread, height + casing))
}

/// A start-aligned title strip, drawn where egui's title bar would sit.
///
/// `width` is the body's: a window sizes to its widest child, so a strip
/// taking whatever is offered would set the width and pad narrow platforms.
///
/// `close_hint` adds a close affordance and names what it closes; strips with
/// no meaningful close pass `None`. Returns whether it was clicked.
fn title_strip(
    ui: &mut egui::Ui,
    title: &str,
    width: f32,
    palette: &super::theme::Palette,
    close_hint: Option<&str>,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, TITLE_H), egui::Sense::hover());
    // Nothing else marks these as draggable, and fading rather than
    // switching keeps the cue from flickering under a moving cursor.
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let hover = ui
        .ctx()
        .animate_bool_with_time(response.id, response.hovered(), HOVER_FADE_SECS);
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::ZERO,
        palette
            .layer_accent
            .lerp_to_gamma(palette.layer_accent_hover, hover * HOVER_STRENGTH),
    );
    ui.painter().text(
        rect.left_center() + egui::vec2(TITLE_PAD, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(TITLE_FONT),
        ui.visuals().strong_text_color(),
    );
    let closed = close_hint.is_some_and(|hint| paint_close(ui, rect, hint));
    // The body meets the strip as a window edge, not as the next widget down.
    // Taken here rather than off the window's style, which the body inherits.
    let gap = ui.spacing().item_spacing.y;
    ui.add_space(-gap);
    closed
}

/// The strip's close affordance, at its trailing edge.
///
/// The hint says what closing means here — a device is opened and closed as
/// a whole, so either of BMC100's two frames closes the device.
fn paint_close(ui: &mut egui::Ui, strip: egui::Rect, hint: &str) -> bool {
    let centre = egui::pos2(
        strip.right() - TITLE_PAD - super::ui_helpers::CLOSE_SIZE / 2.0,
        strip.center().y,
    );
    super::ui_helpers::close_button(ui, centre, hint)
}

/// Air between windows in a packed arrangement.
const PACK_GAP: f32 = 12.0;

/// Halvings taken to find the zoom a `Pack` tiles the canvas at. Packing is
/// pure geometry, so a candidate costs arithmetic rather than a frame.
const PACK_SEARCH_STEPS: u8 = 20;

/// The largest zoom whose packing still fits the canvas, and that packing.
///
/// Zoom and layout are not independent: a smaller zoom fits more windows
/// per row, which is a different arrangement and not a scaled one.
/// Scaling one layout to fit therefore wastes canvas, so every candidate
/// zoom is packed afresh and judged on what it produced.
fn pack_to_fit(
    entries: &[(&'static Platform, Vec<DeviceFrame>)],
    canvas: egui::Rect,
) -> (f32, Vec<egui::Pos2>) {
    let pack_at = |zoom: f32| {
        let sizes: Vec<egui::Vec2> = entries
            .iter()
            .map(|(platform, frames)| window_size(platform, frames, zoom))
            .collect();
        let positions = pack_positions(&sizes, canvas);
        let bounds = positions
            .iter()
            .zip(&sizes)
            .map(|(pos, size)| egui::Rect::from_min_size(*pos, *size))
            .reduce(egui::Rect::union);
        (positions, bounds)
    };

    let (mut low, mut high) = (super::canvas::MIN_ZOOM, super::canvas::MAX_ZOOM);
    let mut best = None;
    for _ in 0..PACK_SEARCH_STEPS {
        let zoom = 0.5 * (low + high);
        let (positions, bounds) = pack_at(zoom);
        // With nothing to place there is nothing to overflow.
        if bounds.is_none_or(|bounds| canvas.contains_rect(bounds)) {
            best = Some((zoom, positions));
            low = zoom;
        } else {
            high = zoom;
        }
    }
    best.unwrap_or_else(|| {
        let (positions, _) = pack_at(super::canvas::MIN_ZOOM);
        (super::canvas::MIN_ZOOM, positions)
    })
}

/// The window's on-screen size, near enough for arranging.
///
/// The device scales with the canvas and the chrome around it does not,
/// so there is no one canvas-space size and arranging works on screen.
fn window_size(platform: &Platform, frames: &[DeviceFrame], zoom: f32) -> egui::Vec2 {
    let states = frames.len() as f32;
    let stacked = states_extent(platform, frames);
    stacked * zoom + egui::vec2(0.0, TITLE_H + STATE_GAP * (states - 1.0).max(0.0))
}

/// The device's states stacked: as wide as the widest, as tall as the sum.
fn states_extent(platform: &Platform, frames: &[DeviceFrame]) -> egui::Vec2 {
    let strip = if platform.led_count().is_some() {
        crate::LED_STRIP_H as f32 + STRIP_SEAM
    } else {
        0.0
    };
    frames
        .iter()
        .map(|frame| state_layout(frame, strip).1)
        .fold(egui::Vec2::ZERO, |stacked, size| {
            egui::vec2(stacked.x.max(size.x), stacked.y + size.y)
        })
}

/// Where a window sits until something places it: stepped off the canvas
/// corner so a device opened mid-session does not land on top of another.
fn default_position(canvas: egui::Rect, order: usize) -> egui::Pos2 {
    canvas.min + egui::vec2(40.0 + 48.0 * order as f32, 40.0 + 48.0 * order as f32)
}

/// Pack the windows into the canvas width, growing downward as needed —
/// the canvas pans, so height is the direction with room to spare.
fn pack_positions(sizes: &[egui::Vec2], canvas: egui::Rect) -> Vec<egui::Pos2> {
    use binpack2d::maxrects::{Heuristic, MaxRectsBin};

    let wanted: Vec<binpack2d::Dimension> = sizes
        .iter()
        .enumerate()
        .map(|(order, size)| {
            binpack2d::Dimension::with_id(
                order.cast_signed(),
                (size.x + PACK_GAP) as i32,
                (size.y + PACK_GAP) as i32,
                0,
            )
        })
        .collect();

    // Growing the bin downward never fits a window wider than the canvas,
    // so it starts at least that wide. The canvas pans, so overflow is fine.
    let widest = sizes.iter().map(|s| s.x + PACK_GAP).fold(0.0_f32, f32::max);
    let bin_width = canvas.width().max(widest) as i32;

    // Doubling never grows a zero height, which a canvas has before layout.
    let mut height = (canvas.height() as i32).max(1);
    loop {
        let mut bin = MaxRectsBin::new(bin_width, height);
        let (placements, rejected) = bin.insert_list(&wanted, Heuristic::BestAreaFit);
        if rejected.is_empty() {
            let mut positions = vec![canvas.min; sizes.len()];
            for spot in placements {
                if let Some(pos) = positions.get_mut(spot.id().unsigned_abs()) {
                    *pos = canvas.min + egui::vec2(spot.x() as f32, spot.y() as f32);
                }
            }
            return positions;
        }
        height *= 2;
    }
}

impl TestbedApp {
    /// One floating window per open device, holding every display state that
    /// device can be in.
    pub(super) fn paint_device_windows(&mut self, ctx: &egui::Context, time_s: f32) {
        let entries: Vec<(&'static Platform, Vec<DeviceFrame>)> = self
            .stage
            .open()
            .iter()
            .map(|platform| {
                let mut frames = device_frames(platform);
                // Mid-take, only the state holding the recorded viewport: the
                // others are inert slabs with nothing to interact with.
                if let Some(rec) = self.recording_mode.active() {
                    frames.retain(|frame| {
                        rec.target().platform.id == platform.id
                            && frame.views.iter().any(|(idx, _)| *idx == rec.active_tile())
                    });
                }
                (*platform, frames)
            })
            .collect();

        // Packing chooses its own zoom, since the tightest layout and the
        // scale it happens at are the same question.
        let arranged = self.stage.take_arrange().then(|| {
            let (zoom, positions) = pack_to_fit(&entries, self.canvas.rect);
            self.canvas.set_zoom(zoom);
            positions
        });

        self.canvas.forget_bounds();
        for (order, (platform, frames)) in entries.iter().enumerate() {
            let target = arranged.as_ref().and_then(|placed| placed.get(order));
            self.paint_device_window(ctx, platform, frames, order, target.copied(), time_s);
        }
        self.canvas.apply_pending_fit();
    }

    fn paint_device_window(
        &mut self,
        ctx: &egui::Context,
        platform: &'static Platform,
        frames: &[DeviceFrame],
        order: usize,
        arranged: Option<egui::Pos2>,
        time_s: f32,
    ) {
        // Recording pins one platform; other windows never hold its index.
        let active_record_idx = self
            .recording_mode
            .active()
            .filter(|r| r.target().platform.id == platform.id)
            .map(super::RecordingState::active_tile);
        let flat = self.stage.tiles_of(platform);

        // The platform's label only repeats its id ("BMM100 BMM Narrow").
        let title = super::ui_helpers::platform_name(platform);
        let palette = self.theme.palette(ctx);
        let zoom = self.canvas.zoom();
        // The widest state sets the window's width, so the strip and the
        // captions span the same edge-to-edge run as the body below them.
        let title_width = states_extent(platform, frames).x * zoom;

        let id = egui::Id::new(("device", platform.id));
        if let Some(target) = arranged {
            let placed = self.canvas.to_canvas(target);
            self.canvas.place(id, placed);
        }
        // The canvas transform is applied here rather than to the layer, so
        // the position is already on screen and needs no clip of its own.
        let default = self
            .canvas
            .to_canvas(default_position(self.canvas.rect, order));
        let pos = self.canvas.screen_pos(id, default);
        let window = egui::Window::new("")
            .id(id)
            .title_bar(false)
            .constrain(false)
            .resizable(false)
            // The window is the enclosure, so it is filled like one: every
            // gap between the states and around the views then reads as the
            // device's own casing rather than as window chrome or canvas.
            .frame(
                egui::Frame::window(&ctx.style())
                    .inner_margin(WINDOW_INSET)
                    .fill(palette.device_bezel),
            )
            .current_pos(pos);
        let close_hint = format!("close {}", super::ui_helpers::platform_name(platform));
        let mut closed = false;
        let response = window.show(ctx, |ui| {
            // The states are spaced by hand below, so the window's own
            // between-widgets gap would only be a second, invisible one.
            ui.spacing_mut().item_spacing.y = 0.0;
            closed = title_strip(ui, &title, title_width, palette, Some(&close_hint));
            for (state, frame) in frames.iter().enumerate() {
                if state > 0 {
                    ui.add_space(STATE_GAP);
                }
                self.paint_state(ui, platform, frame, &flat, active_record_idx, time_s);
            }
        });

        // Where it ended up, which is where it was put unless it was dragged.
        if let Some(response) = response {
            self.canvas.record(id, response.response.rect);
        }
        if closed {
            self.toggle_platform(platform.id, ctx);
        }
    }

    /// One display state: bezel, its views, empty slots, and the LED strip.
    /// `flat` maps this platform's viewport indices to stage tile positions.
    fn paint_state(
        &mut self,
        ui: &mut egui::Ui,
        platform: &'static Platform,
        frame: &DeviceFrame,
        flat: &[usize],
        active_record_idx: Option<usize>,
        time_s: f32,
    ) {
        let palette = self.theme.palette(ui.ctx());
        let zoom = self.canvas.zoom();
        let strip = if platform.led_count().is_some() {
            crate::LED_STRIP_H as f32 + STRIP_SEAM
        } else {
            0.0
        };
        let (places, size) = state_layout(frame, strip);
        let (outer, _) = ui.allocate_exact_size(size * zoom, egui::Sense::hover());
        ui.painter().rect_filled(outer, 0.0, palette.device_bezel);
        let screen_origin = outer.min;

        for (place, (view_idx, local)) in frame.views.iter().enumerate() {
            // Its own pixels, scaled onto the canvas and set where the state's
            // layout put it — the device's grid leaves no room for the strip
            // that goes under each view.
            let rect = egui::Rect::from_min_size(
                screen_origin + places[place].to_vec2() * zoom,
                local.size() * zoom,
            );
            let Some(view) = flat.get(*view_idx).and_then(|i| self.stage.tile_mut(*i)) else {
                continue;
            };
            if !view.is_live() {
                paint_placeholder(ui.painter(), rect, view.label(), palette);
                continue;
            }
            // Recording focuses one viewport: the rest keep stale FBOs, so
            // they get slabs instead of textures and no touch input,
            // keeping the fixture timeline clean.
            if active_record_idx.is_some_and(|active| active != *view_idx) {
                ui.painter().rect_filled(rect, 0.0, palette.device_slab);
                continue;
            }
            super::paint_tile_texture(ui, view, rect);
            debug_outline(ui, rect, "view", egui::Align2::LEFT_TOP);
            // A choose overlay per candidate viewport: clicking it is what
            // starts the take, so it swallows the view's touch input.
            if self.recording_mode.is_choosing() {
                let target = platform_catalog::Target {
                    platform,
                    viewport: &platform.viewports[*view_idx],
                };
                if toolbar::target_recordable(&self.manifest, target) {
                    self.paint_choose_overlay(ui, rect, target);
                }
                continue;
            }
            if active_record_idx == Some(*view_idx) {
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(2.0_f32, palette.accent_record),
                    egui::StrokeKind::Inside,
                );
            }
            if self.show_view_timings
                && let Some(timings) = view.last_timings()
            {
                paint::paint_view_timings(ui.painter(), rect, &timings, view.last_slip_ms());
            }
            // `interact`, not `allocate_rect`: the enclosure is already allocated,
            // and allocating a view inside it rewinds the cursor to the view's bottom,
            // dropping the strip's room so the next state paints over it.
            let response =
                ui.interact(rect, ui.id().with(*view_idx), egui::Sense::click_and_drag());
            let rec = if active_record_idx == Some(*view_idx) {
                self.recording_mode.active_mut()
            } else {
                None
            };
            dispatch_touch_events(&response, rect, view, rec, zoom);
        }

        if strip > 0.0 {
            // A strip under each view, as wide as the view it serves:
            // the device shares one, but a preview showing a single scene
            // would hide what the other widgets on screen drive.
            //
            // A seam of bare bezel between glass and diffuser, and a plate
            // lighter than any widget body under the glow: without both,
            // the strip reads as extra black space inside the view above it.
            for (place, (view_idx, local)) in frame.views.iter().enumerate() {
                let at = places[place];
                let strip_rect = egui::Rect::from_min_size(
                    screen_origin + egui::vec2(at.x, at.y + local.height() + STRIP_SEAM) * zoom,
                    egui::vec2(local.width(), crate::LED_STRIP_H as f32) * zoom,
                );
                ui.painter()
                    .rect_filled(strip_rect, STRIP_RADIUS, palette.device_strip);
                let view = flat.get(*view_idx).and_then(|i| self.stage.tile(*i));

                // The glow is a plain quad mesh, so at full size it paints
                // over the plate's rounded ends and squares them off.
                // Inset by the radius and the diffuser keeps its shape lit or dark.
                paint::paint_led_strip(ui.painter(), view, strip_rect.shrink(STRIP_RADIUS), time_s);
                debug_outline(ui, strip_rect, "strip", egui::Align2::LEFT_TOP);
            }
        }

        // Last, so the views' textures cannot paint over the marks measuring them.
        debug_outline(ui, outer, "state", egui::Align2::RIGHT_BOTTOM);
    }

    /// One choosing-phase overlay: the whole view is the button that opens
    /// its target's naming dialog, which is where the take actually starts.
    fn paint_choose_overlay(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        target: platform_catalog::Target,
    ) {
        let palette = self.theme.palette(ui.ctx());
        let accent = palette.accent_record;
        let recorded = self.recording_mode.recorded_datasets(target).len();

        let response = ui.interact(
            rect,
            ui.id()
                .with(("choose", target.platform.id, target.viewport.id)),
            egui::Sense::click(),
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // A light veil, so the live content stays legible under the card;
        // hover answers with the accent border, marking the view as the button.
        let veil = if response.hovered() { 40 } else { 80 };
        ui.painter()
            .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(veil));
        if response.hovered() {
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(2.0_f32, accent),
                egui::StrokeKind::Inside,
            );
        }

        // The centre card names the target and how much it already carries,
        // so the choice is informed before the dialog opens on it.
        let title = format!("Record {}", super::ui_helpers::target_name(target));
        let detail = match recorded {
            0 => String::new(),
            1 => "1 dataset recorded".to_owned(),
            n => format!("{n} datasets recorded"),
        };
        let fill = egui::Color32::from_black_alpha(210);
        let title_colour = ui.visuals().strong_text_color();
        let detail_colour = accent;

        let title =
            ui.painter()
                .layout_no_wrap(title, egui::FontId::proportional(15.0), title_colour);
        let detail = (!detail.is_empty()).then(|| {
            ui.painter()
                .layout_no_wrap(detail, egui::FontId::proportional(12.0), detail_colour)
        });
        let icon_side = 18.0;
        let text_w = title
            .size()
            .x
            .max(detail.as_ref().map_or(0.0, |d| d.size().x));
        let text_h = title.size().y + detail.as_ref().map_or(0.0, |d| d.size().y + LINE_GAP);
        let pad = egui::vec2(14.0, 10.0);
        let content = egui::vec2(icon_side + 8.0 + text_w, icon_side.max(text_h));
        let card = egui::Rect::from_center_size(rect.center(), content + 2.0 * pad);
        ui.painter().rect_filled(card, 4.0, fill);
        ui.painter().rect_stroke(
            card,
            4.0,
            egui::Stroke::new(1.0_f32, accent),
            egui::StrokeKind::Inside,
        );
        let inner = card.shrink2(pad);
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(inner.min.x, inner.center().y - icon_side / 2.0),
            egui::Vec2::splat(icon_side),
        );
        self.icons.record.paint(ui, icon_rect, title_colour);
        let text_x = icon_rect.max.x + 8.0;
        let mut text_y = inner.center().y - text_h / 2.0;
        let title_h = title.size().y;
        ui.painter()
            .galley(egui::pos2(text_x, text_y), title, title_colour);
        if let Some(detail) = detail {
            text_y += title_h + LINE_GAP;
            ui.painter()
                .galley(egui::pos2(text_x, text_y), detail, detail_colour);
        }

        if response.clicked() {
            self.recording_mode.choose(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_runtime::platform_catalog::{self, Platform};

    use super::{device_frames, pack_positions, pack_to_fit, window_size};

    /// Every view's strip keeps the enclosure's margin beneath it.
    /// Fitting is not enough: flush against the edge a strip reads as
    /// a cropped bar rather than as the diffuser it is.
    #[test]
    fn every_led_strip_keeps_its_casing_inside_the_state() {
        let strip = crate::LED_STRIP_H as f32 + super::STRIP_SEAM;
        for platform in platform_catalog::PLATFORMS {
            if platform.led_count().is_none() {
                continue;
            }
            for frame in device_frames(platform) {
                let (places, size) = super::state_layout(&frame, strip);
                for (place, (_, view)) in frame.views.iter().enumerate() {
                    let bottom = places[place].y
                        + view.height()
                        + super::STRIP_SEAM
                        + crate::LED_STRIP_H as f32;
                    assert!(
                        bottom + super::STATE_ITEM_GAP <= size.y,
                        "{}: a strip ending at {bottom} leaves no casing \
                         inside a state {} tall",
                        platform.id,
                        size.y,
                    );
                }
            }
        }
    }

    /// A state with one view spends nothing on spacing: the enclosure is the
    /// display, so a lone viewport starts at its origin and is as wide as it.
    /// Only a shared screen pays for the gaps that tell its views apart.
    #[test]
    fn a_lone_viewport_fills_its_enclosure() {
        for platform in platform_catalog::PLATFORMS {
            let strip = if platform.led_count().is_some() {
                crate::LED_STRIP_H as f32 + super::STRIP_SEAM
            } else {
                0.0
            };
            for frame in device_frames(platform)
                .iter()
                .filter(|f| f.views.len() == 1)
            {
                let (places, size) = super::state_layout(frame, strip);
                assert_eq!(
                    places[0],
                    egui::Pos2::ZERO,
                    "{}: a lone viewport is inset from its own enclosure",
                    platform.id,
                );
                assert!(
                    (size.x - frame.screen.x).abs() < f32::EPSILON,
                    "{}: a lone viewport's enclosure is {} wide, not the display's {}",
                    platform.id,
                    size.x,
                    frame.screen.x,
                );
            }
        }
    }

    /// Every platform with its display states, the way the canvas opens them.
    fn all_devices() -> Vec<(&'static Platform, Vec<super::DeviceFrame>)> {
        platform_catalog::PLATFORMS
            .iter()
            .map(|platform| (platform, device_frames(platform)))
            .collect()
    }

    /// What a `Pack` at `zoom` would cover.
    fn packed_bounds(
        entries: &[(&'static Platform, Vec<super::DeviceFrame>)],
        canvas: egui::Rect,
        zoom: f32,
    ) -> egui::Rect {
        let sizes: Vec<egui::Vec2> = entries
            .iter()
            .map(|(platform, frames)| window_size(platform, frames, zoom))
            .collect();
        pack_positions(&sizes, canvas)
            .iter()
            .zip(&sizes)
            .map(|(pos, size)| egui::Rect::from_min_size(*pos, *size))
            .reduce(egui::Rect::union)
            .expect("BUG: the catalog offers devices to pack")
    }

    /// A canvas has no height until egui has laid it out once,
    /// and the bin search grows by doubling, which never leaves zero.
    #[test]
    fn packing_into_a_canvas_with_no_height_still_finishes() {
        let canvas = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1240.0, 0.0));
        let placed = pack_positions(&[egui::vec2(320.0, 240.0)], canvas);
        assert_eq!(placed.len(), 1);
    }

    #[test]
    fn packing_takes_the_largest_zoom_the_canvas_holds() {
        let entries = all_devices();
        let canvas = egui::Rect::from_min_size(egui::pos2(0.0, 36.0), egui::vec2(1240.0, 960.0));

        let (zoom, positions) = pack_to_fit(&entries, canvas);

        assert_eq!(positions.len(), entries.len(), "every device is placed");
        let bounds = packed_bounds(&entries, canvas, zoom);
        assert!(
            canvas.contains_rect(bounds),
            "{bounds:?} escapes the canvas {canvas:?} at zoom {zoom}"
        );
        // Being inside is easy at any small enough scale; the point of packing
        // is that nothing larger would have fitted.
        let bigger = packed_bounds(&entries, canvas, zoom * 1.1);
        assert!(
            !canvas.contains_rect(bigger),
            "zoom {zoom} left room — {bigger:?} still fits {canvas:?}"
        );
    }

    fn frames_for(id: &str) -> Vec<super::DeviceFrame> {
        let platform = platform_catalog::platform(id)
            .unwrap_or_else(|| panic!("BUG: '{id}' must be in the catalog"));
        device_frames(platform)
    }

    #[test]
    fn bmc100_splits_into_fullscreen_and_slots() {
        let frames = frames_for("bmc100");
        assert_eq!(frames.len(), 2, "one fullscreen frame plus the slot grid");
        assert_eq!(frames[0].views.len(), 1);
        assert_eq!(frames[1].views.len(), 3, "large, medium and small coexist");

        // Count cells no view covers, which pins the first-fit packing without
        // depending on the catalog's exact pixel geometry.
        let platform =
            platform_catalog::platform("bmc100").expect("BUG: 'bmc100' must be in the catalog");
        let grid = platform
            .slot_grid()
            .expect("BUG: 'bmc100' must declare a slot grid");
        let cell = egui::vec2(
            frames[1].screen.x / grid.columns as f32,
            frames[1].screen.y / grid.rows as f32,
        );
        let free = (0..grid.rows)
            .flat_map(|row| (0..grid.columns).map(move |col| (col, row)))
            .filter(|(col, row)| {
                let centre = egui::pos2((*col as f32 + 0.5) * cell.x, (*row as f32 + 0.5) * cell.y);
                !frames[1]
                    .views
                    .iter()
                    .any(|(_, rect)| rect.contains(centre))
            })
            .count();
        assert_eq!(
            free, 1,
            "a 4x2 grid holding 2x2 + 2x1 + 1x1 leaves one slot free"
        );
    }

    #[test]
    fn every_viewport_lands_in_exactly_one_frame() {
        for platform in platform_catalog::PLATFORMS {
            let frames = frames_for(platform.id);
            let mut seen: Vec<usize> = frames
                .iter()
                .flat_map(|f| f.views.iter().map(|(idx, _)| *idx))
                .collect();
            seen.sort_unstable();
            let all: Vec<usize> = (0..platform.viewports.len()).collect();
            assert_eq!(
                seen, all,
                "{}: frames must cover the viewports",
                platform.id
            );
        }
    }

    #[test]
    fn slot_views_stay_inside_the_screen() {
        for frame in frames_for("bmc100") {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, frame.screen);
            for (idx, rect) in &frame.views {
                assert!(
                    screen.contains_rect(*rect),
                    "viewport {idx} at {rect:?} escapes the {screen:?} screen"
                );
            }
        }
    }

    #[test]
    fn slot_views_do_not_overlap() {
        let frames = frames_for("bmc100");
        let views = &frames[1].views;
        for (i, (_, a)) in views.iter().enumerate() {
            for (_, b) in &views[i + 1..] {
                assert!(!a.intersects(*b), "slot placements collide: {a:?} vs {b:?}");
            }
        }
    }
}
