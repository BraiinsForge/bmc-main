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

use bmc_wasm_runtime::platform_catalog::{self, Platform};

use super::{TestbedApp, dispatch_touch_events, paint, paint_placeholder, toolbar};

/// One display state of the device: which viewports show, and where.
pub(crate) struct DeviceFrame {
    pub(crate) label: String,
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
                label: viewport.label.to_owned(),
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

    Some(DeviceFrame {
        label: "Slots".to_owned(),
        screen,
        views,
    })
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
/// Height of the hand-painted title strip. Ours rather than egui's,
/// because egui hard-centres its window titles.
const TITLE_H: f32 = 24.0;
/// Title text size, and its inset from the strip's leading edge.
const TITLE_FONT: f32 = 12.0;
const TITLE_PAD: f32 = 8.0;

/// Hit area of the strip's close cross, and half the length of each arm.
const CLOSE_SIZE: f32 = 16.0;
const CLOSE_ARM: f32 = 3.5;

/// Leading between the choose card's two lines.
const LINE_GAP: f32 = 2.0;

/// How long the title strip takes to reach its hovered tone.
const HOVER_FADE_SECS: f32 = 0.12;

/// How far towards it the strip travels: the border holds at the resting
/// tone, so a full step re-opens the seam they share a colour to close.
const HOVER_STRENGTH: f32 = 0.5;

/// egui reports a window a pixel larger than the frame it paints,
/// so the stroke needs no inset to show; adding one leaves the shadow
/// as a ring between border and content.
const WINDOW_INSET: egui::Margin = egui::Margin::ZERO;

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
        egui::CornerRadius {
            nw: super::theme::WINDOW_RADIUS,
            ne: super::theme::WINDOW_RADIUS,
            sw: 0,
            se: 0,
        },
        palette
            .title_fill
            .lerp_to_gamma(palette.title_hover_fill, hover * HOVER_STRENGTH),
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
/// A cross rather than a labelled button: the strip is 24 px tall and the
/// title already fills it, and the hint says what closing means here — a
/// device is opened and closed as a whole, so either of BMC100's two frames
/// closes the device.
fn paint_close(ui: &mut egui::Ui, strip: egui::Rect, hint: &str) -> bool {
    let centre = egui::pos2(
        strip.right() - TITLE_PAD - CLOSE_SIZE / 2.0,
        strip.center().y,
    );
    let hit = egui::Rect::from_center_size(centre, egui::Vec2::splat(CLOSE_SIZE));
    let response = ui
        .interact(hit, ui.id().with(("close", hint)), egui::Sense::click())
        .on_hover_text(hint);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let colour = if response.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let arm = CLOSE_ARM;
    let stroke = egui::Stroke::new(1.5_f32, colour);
    ui.painter().line_segment(
        [centre - egui::vec2(arm, arm), centre + egui::vec2(arm, arm)],
        stroke,
    );
    ui.painter().line_segment(
        [
            centre + egui::vec2(arm, -arm),
            centre - egui::vec2(arm, -arm),
        ],
        stroke,
    );
    response.clicked()
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
    entries: &[(&'static Platform, DeviceFrame)],
    canvas: egui::Rect,
) -> (f32, Vec<egui::Pos2>) {
    let pack_at = |zoom: f32| {
        let sizes: Vec<egui::Vec2> = entries
            .iter()
            .map(|(platform, frame)| window_size(platform, frame, zoom))
            .collect();
        let positions = arrange_positions(ArrangeMode::Pack, &sizes, canvas);
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
fn window_size(platform: &Platform, frame: &DeviceFrame, zoom: f32) -> egui::Vec2 {
    let strip = if platform.led_count().is_some() {
        super::LED_STRIP_H as f32 + STRIP_SEAM
    } else {
        0.0
    };
    (frame.screen + egui::vec2(0.0, strip)) * zoom + egui::vec2(0.0, TITLE_H)
}

/// A one-shot rearrangement of every device window.
#[derive(Clone, Copy)]
pub(crate) enum ArrangeMode {
    /// Cascade from the top left, the startup default.
    Stack,
    /// Bin-pack to use the canvas tightly.
    Pack,
}

/// Where each window goes, aligned with the paint order.
fn arrange_positions(
    mode: ArrangeMode,
    sizes: &[egui::Vec2],
    canvas: egui::Rect,
) -> Vec<egui::Pos2> {
    match mode {
        ArrangeMode::Stack => (0..sizes.len())
            .map(|order| stack_position(canvas, order))
            .collect(),
        ArrangeMode::Pack => pack_positions(sizes, canvas),
    }
}

fn stack_position(canvas: egui::Rect, order: usize) -> egui::Pos2 {
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

    let mut height = canvas.height() as i32;
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
    /// One floating window per frame of every open device, so each title
    /// names exactly what its body shows and the body is nothing but the
    /// device mock.
    pub(super) fn paint_device_windows(&mut self, ctx: &egui::Context, time_s: f32) {
        let entries: Vec<(&'static Platform, DeviceFrame)> = self
            .open_platforms
            .clone()
            .into_iter()
            .flat_map(|platform| {
                device_frames(platform)
                    .into_iter()
                    .map(move |frame| (platform, frame))
            })
            // Mid-take, only the frame holding the recorded viewport: the
            // others' views are inert slabs with nothing to interact with.
            .filter(|(platform, frame)| {
                self.recording_mode.active().is_none_or(|rec| {
                    rec.target().platform.id == platform.id
                        && frame.views.iter().any(|(idx, _)| *idx == rec.active_tile())
                })
            })
            .collect();

        // Held rather than consumed until every open platform has its views:
        // a mode swap retires them all, and an arrangement computed over the
        // one-frame gap would place (and spend itself on) an empty canvas.
        let views_ready = self
            .open_platforms
            .iter()
            .all(|p| self.tiles.iter().any(|view| view.platform.id == p.id));
        let arranged = if views_ready {
            self.arrange.take()
        } else {
            None
        };
        let arranged = arranged.map(|mode| match mode {
            ArrangeMode::Stack => {
                let zoom = self.canvas.zoom();
                let sizes: Vec<egui::Vec2> = entries
                    .iter()
                    .map(|(platform, frame)| window_size(platform, frame, zoom))
                    .collect();
                arrange_positions(mode, &sizes, self.canvas.rect)
            }
            // Packing chooses its own zoom, since the tightest layout and the
            // scale it happens at are the same question.
            ArrangeMode::Pack => {
                let (zoom, positions) = pack_to_fit(&entries, self.canvas.rect);
                self.canvas.set_zoom(zoom);
                positions
            }
        });

        self.canvas.forget_bounds();
        for (order, (platform, frame)) in entries.iter().enumerate() {
            let target = arranged.as_ref().and_then(|placed| placed.get(order));
            self.paint_frame_window(ctx, platform, frame, order, target.copied(), time_s);
        }
        self.canvas.apply_pending_fit();
    }

    fn paint_frame_window(
        &mut self,
        ctx: &egui::Context,
        platform: &'static Platform,
        frame: &DeviceFrame,
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
        // Flat indices of this platform's views, in viewport order —
        // `build_views` appends them contiguously, so the order holds.
        let flat: Vec<usize> = self
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, view)| view.platform.id == platform.id)
            .map(|(idx, _)| idx)
            .collect();

        // The platform's label only repeats its id ("BMM100 BMM Narrow").
        let title = format!("{} — {}", platform.id.to_uppercase(), frame.label);
        let palette = self.theme.palette(ctx);
        let zoom = self.canvas.zoom();
        let title_width = frame.screen.x * zoom;

        let id = egui::Id::new(("device", platform.id, frame.label.clone()));
        if let Some(target) = arranged {
            let placed = self.canvas.to_canvas(target);
            self.canvas.place(id, placed);
        }
        // The canvas transform is applied here rather than to the layer, so
        // the position is already on screen and needs no clip of its own.
        let default = self
            .canvas
            .to_canvas(stack_position(self.canvas.rect, order));
        let pos = self.canvas.screen_pos(id, default);
        let window = egui::Window::new("")
            .id(id)
            .title_bar(false)
            .constrain(false)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.style()).inner_margin(WINDOW_INSET))
            .current_pos(pos);
        let close_hint = format!("close {}", platform.id.to_uppercase());
        let mut closed = false;
        let response = window.show(ctx, |ui| {
            closed = title_strip(ui, &title, title_width, palette, Some(&close_hint));
            self.paint_frame(ui, platform, frame, &flat, active_record_idx, time_s);
        });

        // Where it ended up, which is where it was put unless it was dragged.
        if let Some(response) = response {
            self.canvas.record(id, response.response.rect);
        }
        if closed {
            self.toggle_platform(platform.id, ctx);
        }
    }

    /// One device state: bezel, its views, empty slots, and the LED strip.
    /// `flat` maps this platform's viewport indices to `self.tiles` positions.
    fn paint_frame(
        &mut self,
        ui: &mut egui::Ui,
        platform: &'static Platform,
        frame: &DeviceFrame,
        flat: &[usize],
        active_record_idx: Option<usize>,
        time_s: f32,
    ) {
        // Recording pins one viewport; a frame that doesn't hold it
        // is still shown, dimmed, so the geometry stays readable.
        let frame_holds_active = active_record_idx
            .is_none_or(|active| frame.views.iter().any(|(idx, _)| *idx == active));

        let palette = self.theme.palette(ui.ctx());
        let zoom = self.canvas.zoom();
        let strip_h = if platform.led_count().is_some() {
            (super::LED_STRIP_H as f32 + STRIP_SEAM) * zoom
        } else {
            0.0
        };
        let (outer, _) = ui.allocate_exact_size(
            frame.screen * zoom + egui::vec2(0.0, strip_h),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(outer, 0.0, palette.bezel);
        let screen_origin = outer.min;

        for (view_idx, local) in &frame.views {
            // The device's own pixels, scaled onto the canvas.
            let rect = egui::Rect::from_min_size(
                screen_origin + local.min.to_vec2() * zoom,
                local.size() * zoom,
            );
            let Some(view) = flat.get(*view_idx).and_then(|i| self.tiles.get_mut(*i)) else {
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
                ui.painter().rect_filled(rect, 0.0, palette.record_slab);
                continue;
            }
            super::paint_tile_texture(ui, view, rect);
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
                    egui::Stroke::new(2.0_f32, palette.record_accent),
                    egui::StrokeKind::Inside,
                );
            }
            if self.show_view_timings
                && let Some(timings) = view.last_timings()
            {
                paint::paint_view_timings(ui.painter(), rect, &timings, view.last_slip_ms());
            }
            let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            let rec = if active_record_idx == Some(*view_idx) {
                self.recording_mode.active_mut()
            } else {
                None
            };
            dispatch_touch_events(&response, rect, view, rec, zoom);
        }

        if strip_h > 0.0 {
            // A seam of bare bezel between glass and diffuser, and a plate
            // lighter than any widget body under the glow: without both, the
            // strip reads as extra black space inside the view above it.
            let strip_rect = egui::Rect::from_min_size(
                screen_origin + egui::vec2(0.0, (frame.screen.y + STRIP_SEAM) * zoom),
                egui::vec2(frame.screen.x, super::LED_STRIP_H as f32) * zoom,
            );
            ui.painter()
                .rect_filled(strip_rect, 3.0, palette.strip_plate);
            // The device has one strip; every view drives its own runtime,
            // so the frame shows the first scene it finds.
            let scene_view = frame
                .views
                .iter()
                .filter_map(|(idx, _)| flat.get(*idx))
                .filter_map(|i| self.tiles.get(*i))
                .find(|view| view.led_scene().is_some());
            paint::paint_led_strip(ui.painter(), scene_view, strip_rect, time_s);
        }

        if !frame_holds_active {
            ui.painter()
                .rect_filled(outer, 0.0, egui::Color32::from_black_alpha(160));
        }
    }

    /// One choosing-phase overlay: the whole view is the button that starts
    /// the take on its target. A target with a fixture wears a badge saying
    /// which datasets it carries, and asks a second time before a take that
    /// will overwrite one.
    fn paint_choose_overlay(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        target: platform_catalog::Target,
    ) {
        let palette = self.theme.palette(ui.ctx());
        let accent = palette.record_accent;
        let recorded = self.recording_mode.recorded_datasets(target).join(", ");
        let confirming = self.recording_mode.is_confirming(target);

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

        // The centre card: what the click does, and what it costs — an
        // existing fixture is named here rather than off in a corner, where
        // it went unread. Asking twice only works if the first click visibly
        // answers, so confirming flips the card to solid accent rather than
        // just rewording it.
        let (title, detail) = match (confirming, recorded.is_empty()) {
            (true, _) => (
                format!("Overwrite {recorded}?"),
                "click again to replace it".to_owned(),
            ),
            (false, true) => (format!("Record {target}"), String::new()),
            (false, false) => (
                format!("Re-record {target}"),
                format!("replaces {recorded}"),
            ),
        };
        let (fill, title_colour, detail_colour) = if confirming {
            (
                accent,
                egui::Color32::WHITE,
                egui::Color32::from_white_alpha(200),
            )
        } else {
            (
                egui::Color32::from_black_alpha(210),
                ui.visuals().strong_text_color(),
                accent,
            )
        };

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
            egui::Stroke::new(if confirming { 2.0_f32 } else { 1.0_f32 }, accent),
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

    use super::{ArrangeMode, arrange_positions, device_frames, pack_to_fit, window_size};

    /// Every frame of every platform, the way the canvas opens them.
    fn all_frames() -> Vec<(&'static Platform, super::DeviceFrame)> {
        platform_catalog::PLATFORMS
            .iter()
            .flat_map(|platform| {
                device_frames(platform)
                    .into_iter()
                    .map(move |frame| (platform, frame))
            })
            .collect()
    }

    /// What a `Pack` at `zoom` would cover.
    fn packed_bounds(
        entries: &[(&'static Platform, super::DeviceFrame)],
        canvas: egui::Rect,
        zoom: f32,
    ) -> egui::Rect {
        let sizes: Vec<egui::Vec2> = entries
            .iter()
            .map(|(platform, frame)| window_size(platform, frame, zoom))
            .collect();
        arrange_positions(ArrangeMode::Pack, &sizes, canvas)
            .iter()
            .zip(&sizes)
            .map(|(pos, size)| egui::Rect::from_min_size(*pos, *size))
            .reduce(egui::Rect::union)
            .expect("BUG: the catalog offers frames to pack")
    }

    #[test]
    fn packing_takes_the_largest_zoom_the_canvas_holds() {
        let entries = all_frames();
        let canvas = egui::Rect::from_min_size(egui::pos2(0.0, 36.0), egui::vec2(1240.0, 960.0));

        let (zoom, positions) = pack_to_fit(&entries, canvas);

        assert_eq!(positions.len(), entries.len(), "every frame is placed");
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
