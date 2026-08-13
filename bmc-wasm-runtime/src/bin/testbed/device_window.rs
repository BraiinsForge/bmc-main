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

use bmc_wasm_runtime::platform_catalog::Platform;

use super::{RecordingAction, TestbedApp, dispatch_touch_events, paint, paint_placeholder};

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
/// The recording panel's content area, which its readouts assume;
/// the stats readout sizes itself, and this stands in for it when arranging.
const STATS_WINDOW_SIZE: egui::Vec2 = egui::vec2(400.0, 280.0);
/// Height of the hand-painted title strip. Ours rather than egui's,
/// because egui hard-centres its window titles.
const TITLE_H: f32 = 24.0;

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
fn title_strip(ui: &mut egui::Ui, title: &str, width: f32, palette: &super::theme::Palette) {
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
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(12.0),
        ui.visuals().strong_text_color(),
    );
    // The body meets the strip as a window edge, not as the next widget down.
    // Taken here rather than off the window's style, which the body inherits.
    let gap = ui.spacing().item_spacing.y;
    ui.add_space(-gap);
}

/// Air between windows in a packed arrangement.
const PACK_GAP: f32 = 12.0;

/// The window's outer size, near enough for arranging.
fn window_size(platform: &Platform, frame: &DeviceFrame) -> egui::Vec2 {
    let strip = if platform.led_count().is_some() {
        super::LED_STRIP_H as f32 + STRIP_SEAM
    } else {
        0.0
    };
    frame.screen + egui::vec2(0.0, strip + TITLE_H)
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
            .collect();

        // A requested arrangement re-homes every window — the stats window
        // included, as the last entry — and brings the canvas back to its
        // origin, so the result is on screen.
        let arranged = self.arrange.take().map(|mode| {
            self.pan = egui::Vec2::ZERO;
            let mut sizes: Vec<egui::Vec2> = entries
                .iter()
                .map(|(platform, frame)| window_size(platform, frame))
                .collect();
            sizes.push(STATS_WINDOW_SIZE + egui::vec2(0.0, TITLE_H));
            arrange_positions(mode, &sizes, self.canvas)
        });
        if let Some(placed) = &arranged {
            self.arranged_stats_pos = placed.last().copied();
        }

        for (order, (platform, frame)) in entries.iter().enumerate() {
            let target = arranged.as_ref().and_then(|placed| placed.get(order));
            self.paint_frame_window(ctx, platform, frame, order, target.copied(), time_s);
        }
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
            .state
            .as_ref()
            .filter(|r| r.target.platform.id == platform.id)
            .map(|r| r.active_tile);
        // Flat indices of this platform's views, in viewport order —
        // `build_views` appends them contiguously, so the order holds.
        let flat: Vec<usize> = self
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, view)| view.platform.id == platform.id)
            .map(|(idx, _)| idx)
            .collect();

        let title = format!(
            "{} {} — {}",
            platform.id.to_uppercase(),
            platform.label,
            frame.label
        );
        let palette = self.theme.palette(ctx);
        let title_width = frame.screen.x;
        // The window lays out in canvas space while its clip defaults
        // to the screen, so panning cuts it off; undoing the pan restores it.
        // `constrain_to` also re-enables clamping, so `constrain` follows it.
        let mut window = egui::Window::new("")
            .id(egui::Id::new(("device", platform.id, frame.label.clone())))
            .title_bar(false)
            .constrain_to(ctx.screen_rect().translate(-self.pan))
            .constrain(false)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.style()).inner_margin(WINDOW_INSET))
            .default_pos(stack_position(self.canvas, order));
        if let Some(target) = arranged {
            window = window.current_pos(target);
        }
        let response = window.show(ctx, |ui| {
            title_strip(ui, &title, title_width, palette);
            self.paint_frame(ui, platform, frame, &flat, active_record_idx, time_s);
        });

        // Translating the window's layer is what makes the canvas pannable:
        // the window keeps its canvas-space position, the transform moves it
        // on screen, and egui routes input back through the same transform.
        if let Some(response) = response {
            ctx.set_transform_layer(
                response.response.layer_id,
                egui::emath::TSTransform::from_translation(self.pan),
            );
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
        let strip_h = if platform.led_count().is_some() {
            super::LED_STRIP_H as f32 + STRIP_SEAM
        } else {
            0.0
        };
        let (outer, _) = ui.allocate_exact_size(
            frame.screen + egui::vec2(0.0, strip_h),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(outer, 0.0, palette.bezel);
        let screen_origin = outer.min;

        for (view_idx, local) in &frame.views {
            let rect = local.translate(screen_origin.to_vec2());
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
            if active_record_idx == Some(*view_idx) {
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(2.0_f32, palette.record_accent),
                    egui::StrokeKind::Inside,
                );
            }
            let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            let rec = if active_record_idx == Some(*view_idx) {
                self.recording_mode.state.as_mut()
            } else {
                None
            };
            dispatch_touch_events(&response, rect, view, rec);
        }

        if strip_h > 0.0 {
            // A seam of bare bezel between glass and diffuser, and a plate
            // lighter than any widget body under the glow: without both, the
            // strip reads as extra black space inside the view above it.
            let strip_rect = egui::Rect::from_min_size(
                screen_origin + egui::vec2(0.0, frame.screen.y + STRIP_SEAM),
                egui::vec2(frame.screen.x, super::LED_STRIP_H as f32),
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

    /// Stats — or, while recording, the recording panel — in its own window.
    /// Chrome, not canvas: the layer is never pan-transformed.
    pub(super) fn paint_stats_window(&mut self, ctx: &egui::Context) {
        let mut action = None;
        let title = if self.recording_mode.state.is_some() {
            "Recording"
        } else {
            "Stats"
        };
        let palette = self.theme.palette(ctx);
        let mut window = egui::Window::new("")
            .id(egui::Id::new("stats"))
            .title_bar(false)
            .resizable(false)
            // Flush like the device windows; the readouts pad themselves.
            .frame(egui::Frame::window(&ctx.style()).inner_margin(WINDOW_INSET))
            .default_pos(egui::pos2(40.0, 620.0));
        if let Some(target) = self.arranged_stats_pos.take() {
            window = window.current_pos(target);
        }
        let response = window.show(ctx, |ui| {
            title_strip(ui, title, STATS_WINDOW_SIZE.x, palette);
            if self.recording_mode.state.is_some() {
                let (rect, _) = ui.allocate_exact_size(STATS_WINDOW_SIZE, egui::Sense::hover());
                action = self.paint_recording_panel(ui, rect);
            } else {
                self.paint_stats_panel(ui);
            }
        });
        // Device windows share this layer order and stack by last click;
        // holding the top keeps the readout from settling between them.
        if let Some(response) = response {
            ctx.move_to_top(response.response.layer_id);
        }
        match action {
            Some(RecordingAction::Save) => self.finish_recording(),
            Some(RecordingAction::Cancel) => self.recording_mode.state = None,
            Some(RecordingAction::Capture) => self.push_manual_capture(),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_runtime::platform_catalog;

    use super::device_frames;

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
