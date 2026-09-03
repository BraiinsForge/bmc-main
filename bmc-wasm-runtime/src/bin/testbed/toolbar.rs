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

//! Top toolbar: platform toggles and the host-level controls
//! (reload, offline seal, sleep switch, debug layout, clock fast-forward).
//!
//! Everything here acts on the testbed as a whole; per-widget state
//! stays in the right sidebar.

use bmc_wasm_runtime::platform_catalog::{PLATFORMS, Platform, manifest_viewport_shape};

use super::TestbedApp;
use super::theme::{Tone, spacing};
use super::ui_helpers::{Button, ICON_SIZE};

/// Tall enough for a stacked icon over its label, with breathing room; the
/// toolbar owns its height now that nothing derives the window size from
/// the tiles.
pub(super) const TOOLBAR_H: f32 = 48.0;

/// Keeps the outermost buttons off the window edges.
pub(super) const BAR_INLINE_PAD: f32 = 8.0;

/// Wide enough for the date row — five segments, their gaps and `Set` —
/// plus the inset the sections are boxed with.
///
/// The header has to be given a width from somewhere: its right-aligned
/// close button claims every pixel on offer, and without one the popover
/// stretches to the window. A segment more, or a longer word under one,
/// wraps the row and says so on sight.
const CLOCK_POPOVER_W: f32 = 376.0;

/// A setter section's inset from the box it sits in.
const SECTION_PAD: i8 = spacing::S03 as i8;

/// The surface left showing between two sunk blocks — wide enough
/// to read as a divider, narrow enough not to read as a gap.
const SECTION_SEPARATOR: f32 = spacing::S02;

/// Between one segment and the next: closer
/// than the gap that separates the date from the time.
const SEGMENT_GAP: f32 = 3.0;

/// Well under one unit per pixel: these are typed or nudged.
/// Drag that runs away by a year per swipe is worse than none.
const DATE_DRAG_SPEED: f32 = 0.05;

impl TestbedApp {
    pub(super) fn paint_toolbar(&mut self, root_ui: &mut egui::Ui) {
        let mut chosen: Option<&'static str> = None;
        let palette = self.theme.palette(root_ui.ctx());
        // egui stacks windows above panels, so the fill and the widgets both
        // go in a foreground area; the panel itself only reserves space.
        let panel = egui::TopBottomPanel::top("toolbar")
            .exact_height(TOOLBAR_H)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show_inside(root_ui, |_| {});
        let rect = panel.response.rect;
        egui::Area::new(egui::Id::new("toolbar_chrome"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(root_ui.ctx(), |area| {
                area.set_clip_rect(rect);
                area.painter().rect_filled(rect, 0.0, palette.layer);
                let mut bar = area.new_child(
                    egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(BAR_INLINE_PAD, 0.0))),
                );
                bar.horizontal_centered(|row| {
                    let engaged = self.paint_record_controls(row, palette);
                    super::ui_helpers::group_divider(row, palette.border_subtle, TOOLBAR_H);

                    // Withheld from a take: the platform is pinned to its target,
                    // and Reload would swap the wasm mid-recording.
                    // The view and debug toggles change nothing a take records;
                    // they give up the room to the recorder instead.
                    if !engaged {
                        chosen = self.paint_platform_toggles(row, palette);
                        super::ui_helpers::group_divider(row, palette.border_subtle, TOOLBAR_H);

                        self.paint_view_controls(row);
                        super::ui_helpers::group_divider(row, palette.border_subtle, TOOLBAR_H);

                        // The widget's own build and its rendering.
                        if Button::bar("Reload")
                            .icon(&mut self.icons.reload)
                            .show(row, palette)
                            .on_hover_text("re-read the widget's wasm from disk")
                            .clicked()
                        {
                            self.hot_reload.manual_reload = true;
                        }
                        let debug_on = bmc_render::tree::debug_layout_enabled();
                        if Button::bar("Debug")
                            .icon(&mut self.icons.debug)
                            .tone(Tone::switch(palette, debug_on))
                            .show(row, palette)
                            .on_hover_text(
                                "outline every layout node in the widget render, \
                                 and egui's own inspector over the chrome",
                            )
                            .clicked()
                        {
                            bmc_render::tree::toggle_debug_layout();
                        }
                        let timings_on = self.show_view_timings;
                        if Button::bar("Timings")
                            .icon(&mut self.icons.timer)
                            .tone(Tone::switch(palette, timings_on))
                            .show(row, palette)
                            .on_hover_text("show each view's own frame cost over it")
                            .clicked()
                        {
                            self.show_view_timings = !self.show_view_timings;
                        }
                        super::ui_helpers::group_divider(row, palette.border_subtle, TOOLBAR_H);
                    }

                    // Simulated conditions: what the device can and cannot reach,
                    // and when it thinks it is. Both stay through a take:
                    // a scenario is made of them, and the timeline carries what they do.
                    let offline = self.state().offline;
                    if Button::bar("Offline")
                        .icon(&mut self.icons.offline)
                        .tone(Tone::switch(palette, offline))
                        .show(row, palette)
                        .on_hover_text("seal live I/O, mirroring an offline device")
                        .clicked()
                    {
                        self.state_mut().offline = !offline;
                    }
                    // Asleep is the exception: no event carries it, so a take
                    // would bless captures of frames an off-scene slot never drew.
                    let dormant = self.state().dormant;
                    let asleep = Button::bar("Asleep")
                        .icon(&mut self.icons.sleep)
                        .tone(Tone::switch(palette, dormant))
                        .enabled(!engaged)
                        .show(row, palette)
                        .on_hover_text(
                            "take every tile off-scene: the sleep and wake hooks \
                             fire, deliveries carry on, nothing renders",
                        )
                        .on_disabled_hover_text("no event carries it, so a take cannot replay it");
                    if asleep.clicked() {
                        self.state_mut().dormant = !dormant;
                    }
                    self.paint_clock_controls(row);

                    // Chrome only, so it gives up the room like the toggles above.
                    if !engaged {
                        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |end| {
                            self.paint_theme_switch(end);
                        });
                    }
                });
            });

        if let Some(target) = chosen {
            let ctx = root_ui.ctx().clone();
            self.toggle_platform(target, &ctx);
        }
    }

    /// One toggle per device, as a segmented set: independent on/off, read as
    /// one flat control. The 1 px gaps let the panel show through as dividers,
    /// so neighbouring inactive buttons read apart.
    ///
    /// Returns the platform whose toggle was clicked — acted on after the
    /// toolbar closes, since opening one retires every view.
    fn paint_platform_toggles(
        &mut self,
        row: &mut egui::Ui,
        palette: &super::theme::Palette,
    ) -> Option<&'static str> {
        let mut chosen = None;
        row.scope(|set| {
            set.spacing_mut().item_spacing.x = 1.0;
            for p in PLATFORMS {
                let open = self.stage.is_open(p);
                let supported = platform_supported(p, &self.manifest);
                let name = super::ui_helpers::platform_name(p);
                let response = Button::bar(&name)
                    .icon(&mut self.icons.devices)
                    .tone(Tone::switch(palette, open))
                    .enabled(supported)
                    .show(set, palette)
                    .on_hover_text(p.label)
                    .on_disabled_hover_text("the manifest admits no viewport of this platform");
                if response.clicked() {
                    chosen = Some(p.id);
                }
            }
        });
        chosen
    }

    /// The record mode's toolbar controls, leading the bar.
    ///
    /// Off, this is one red button that opens the choosing phase.
    /// Engaged — choosing or mid-take — a status chip says
    /// which phase is on, Save lands the take, Cancel backs out.
    /// Returns whether the mode is engaged.
    fn paint_record_controls(
        &mut self,
        row: &mut egui::Ui,
        palette: &super::theme::Palette,
    ) -> bool {
        let choosing = self.recording_mode.is_choosing();
        let recording = self.recording_mode.active().is_some();
        let accent = palette.accent_record;

        if !choosing && !recording {
            let cause = if self.cli.perf_report_path.is_some() {
                Some("a profiling run cannot record")
            } else if self.cli.resolved_widget_root().is_none() {
                Some("recording needs a widget root to write the fixture into")
            } else if recordable_targets(&self.manifest).is_empty() {
                Some("the manifest admits no viewport")
            } else {
                None
            };
            // Red at rest too: the colour is this control's identity.
            // Only the ink, so it sits on the face its neighbours do.
            let response = Button::bar("Record")
                .icon(&mut self.icons.record)
                .tone(Tone {
                    ink: accent,
                    ..Tone::secondary(palette)
                })
                .enabled(cause.is_none())
                .show(row, palette)
                .on_hover_text("record a capture fixture from a live take");
            let response = match cause {
                Some(cause) => response.on_disabled_hover_text(cause),
                None => response,
            };
            if response.clicked() {
                let ctx = row.ctx().clone();
                self.start_choosing(&ctx);
            }
            return false;
        }

        // A status readout, not a button: the mode is left through Save or
        // Cancel, and a control that only names the phase must not look
        // like it does anything.
        let status = if recording {
            self.recording_mode
                .active()
                .map_or_else(String::new, |rec| {
                    format!(
                        "RECORDING — {}",
                        super::ui_helpers::target_name(rec.target())
                    )
                })
        } else {
            "RECORD — choose a viewport".to_owned()
        };
        let icon_rect = row
            .allocate_exact_size(egui::Vec2::splat(ICON_SIZE), egui::Sense::hover())
            .0;
        self.icons.record.paint(row, icon_rect, accent);
        row.label(egui::RichText::new(status).color(accent).strong());

        let save_cause = if !recording {
            Some("choose a viewport to record first")
        } else if self
            .recording_mode
            .active()
            .is_some_and(|rec| !rec.has_events())
        {
            Some("nothing recorded yet")
        } else if self.recording_mode.active().is_some_and(|rec| {
            !bmc_wasm_runtime::capture_config::is_valid_dataset_name(rec.dataset())
        }) {
            Some("the dataset name is invalid")
        } else {
            None
        };
        let save = Button::inline("Save")
            .icon(&mut self.icons.save)
            .tone(Tone::commit(palette))
            .enabled(save_cause.is_none())
            .show(row, palette)
            .on_hover_text("write the fixture and leave the take");
        if let Some(cause) = save_cause {
            save.on_disabled_hover_text(cause);
        } else if save.clicked() {
            let ctx = row.ctx().clone();
            self.save_recording(&ctx);
        }
        if Button::inline("Cancel")
            .show(row, palette)
            .on_hover_text("discard the take and put the canvas back")
            .clicked()
        {
            let ctx = row.ctx().clone();
            self.cancel_record_mode(&ctx);
        }
        true
    }

    /// Where the devices sit and how large they read.
    ///
    /// Arrangements are one-shot and leave the windows free-form after; the
    /// zoom is the canvas's, not each window's, so the devices stay
    /// comparable at a glance.
    fn paint_view_controls(&mut self, row: &mut egui::Ui) {
        let palette = self.theme.palette(row.ctx());
        if Button::bar("Tile")
            .icon(&mut self.icons.arrange_grid)
            .show(row, palette)
            .on_hover_text("lay the device windows out to use the canvas tightly")
            .clicked()
        {
            self.stage.request_arrange();
        }
        row.add_space(PAIR_GAP);

        let at_full_size = (self.canvas.zoom() - 1.0).abs() < f32::EPSILON;
        let mut fit = false;
        let mut full_size = false;
        row.scope(|pair| {
            pair.spacing_mut().item_spacing.x = 1.0;
            fit = Button::bar("Fit")
                .icon(&mut self.icons.scale_out)
                .show(pair, palette)
                .on_hover_text("scale the canvas until every device window is in view")
                .clicked();
            full_size = Button::bar("100%")
                .icon(&mut self.icons.scale_in)
                .enabled(!at_full_size)
                .show(pair, palette)
                .on_hover_text("show the devices at their own pixel size")
                .clicked();
        });
        if fit {
            self.canvas.request_fit();
        }
        if full_size {
            let centre = self.canvas.rect.center();
            self.canvas.zoom_about(1.0, centre);
        }
    }

    /// The simulated clock: its reading on the bar, its setters in a popover.
    ///
    /// One control rather than eight, which crowded the bar and read as
    /// unrelated. The reading takes the accent whenever it is not the host's
    /// own time, so a faked clock shows without opening anything.
    fn paint_clock_controls(&mut self, row: &mut egui::Ui) {
        let palette = self.theme.palette(row.ctx());
        let faked = self.state().clock_offset_ms != 0;
        let shown = self.simulated_now().format("%Y-%m-%d %H:%M").to_string();
        let opener = Button::bar(&shown)
            .icon(&mut self.icons.delay)
            .tone(Tone {
                ink: if faked {
                    palette.accent_record
                } else {
                    palette.text_primary
                },
                ..Tone::secondary(palette)
            })
            .show(row, palette)
            .on_hover_text("What the widgets are being shown as now — click to change it");

        let popup = egui::Popup::from_toggle_button_response(&opener)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            // Padding belongs to the sections, which run to the popover's
            // edge — and square, like every other surface the chrome paints.
            .frame(
                egui::Frame::popup(&row.ctx().style())
                    .inner_margin(0)
                    .corner_radius(0),
            );
        let close = popup
            .show(|popover| self.paint_clock_popover(popover, palette))
            .is_some_and(|shown| shown.inner);
        // This one by id: closing "all" would take any popup the rest of the
        // chrome happens to have open with it.
        if close {
            egui::Popup::close_id(row.ctx(), egui::Popup::default_response_id(&opener));
        }
    }

    /// The clock's state, then the two ways to change it.
    ///
    /// They are different operations rather than a coarse and a fine version
    /// of one: *Jump to* moves the calendar and nothing else, so no time
    /// elapses and a recording carries it as a single event, while the nudges
    /// bump the monotonic clock too — polls come due, and a recording carries
    /// the whole span for replay to walk.
    ///
    /// Returns whether the operator asked to close it.
    fn paint_clock_popover(&mut self, ui: &mut egui::Ui, palette: &super::theme::Palette) -> bool {
        ui.set_max_width(CLOCK_POPOVER_W);
        // The gap between blocks, not inside them: children inherit this,
        // so the sections put back what they want between their own rows.
        let inside = ui.spacing().item_spacing.y;
        // A gap rather than a drawn rule: the popover's own surface shows
        // through between the sunk blocks, as it does between the platform
        // toggles, and runs the full bleed because the blocks do.
        ui.spacing_mut().item_spacing.y = SECTION_SEPARATOR;
        let field = super::ui_helpers::field_height(ui);

        // The popover's frame carries no padding, so the boxes below
        // can run edge to edge; the reading brings its own instead.
        let close = egui::Frame::NONE
            .inner_margin(egui::Margin::same(SECTION_PAD))
            .show(ui, |head| {
                head.spacing_mut().item_spacing.y = inside;
                self.paint_clock_reading(head, palette, field)
            })
            .inner;
        section_frame(palette).show(ui, |boxed| {
            boxed.set_min_width(boxed.available_width());
            boxed.spacing_mut().item_spacing.y = inside;
            self.paint_clock_jump(boxed, palette, field);
        });
        section_frame(palette).show(ui, |boxed| {
            boxed.set_min_width(boxed.available_width());
            boxed.spacing_mut().item_spacing.y = inside;
            self.paint_clock_nudges(boxed, palette, field);
        });
        close
    }

    /// What the widgets are being shown as now, and the way out.
    ///
    /// Accented whenever it is not the host's own time, so a clock left
    /// faked from an earlier take cannot be mistaken for the real one.
    fn paint_clock_reading(
        &mut self,
        ui: &mut egui::Ui,
        palette: &super::theme::Palette,
        field: f32,
    ) -> bool {
        let faked = self.state().clock_offset_ms != 0;
        let mut close = false;
        let mut reset = false;
        ui.horizontal(|head| {
            // A button's height whether or not one is here, so the reading
            // does not step down the moment `Reset` appears beside it.
            head.set_min_height(field);
            let reading = if faked {
                self.simulated_now().format("%Y-%m-%d %H:%M:%S").to_string()
            } else {
                "Now — real time".to_owned()
            };
            head.label(
                egui::RichText::new(reading)
                    .monospace()
                    .strong()
                    .color(if faked {
                        palette.accent_record
                    } else {
                        palette.text_secondary
                    }),
            );
            // Beside the reading it undoes, not out by the dismiss,
            // and only while there is something to undo.
            if faked {
                head.add_space(spacing::S03);
                reset = Button::inline("Reset")
                    .icon(&mut self.icons.reload)
                    .tone(Tone::primary(palette))
                    .min_height(field)
                    .show(head, palette)
                    // Says "displayed": time already let past stays past,
                    // since the monotonic clock never rewinds.
                    .on_hover_text("Return the displayed clock to real time")
                    .clicked();
            }
            head.with_layout(egui::Layout::right_to_left(egui::Align::Center), |end| {
                let centre = end.max_rect().right_center()
                    - egui::vec2(super::ui_helpers::CLOSE_SIZE / 2.0, 0.0);
                close = super::ui_helpers::close_button(
                    end,
                    &mut self.icons.close,
                    centre,
                    end.max_rect(),
                    "Close",
                );
            });
        });
        if reset {
            self.move_simulated_clock(0);
        }
        close
    }

    /// Put the displayed clock `offset_ms` from the host's, telling a take.
    ///
    /// The monotonic clock stays put: no time elapsed, so replay reproduces
    /// the move from the event rather than by walking a span.
    fn move_simulated_clock(&mut self, offset_ms: i64) {
        self.state_mut().clock_offset_ms = offset_ms;
        // Read back rather than passed in: the event has to carry where
        // the clock landed, which for a reset is the host's own time.
        let time = self.simulated_now().fixed_offset().to_rfc3339();
        self.recording_mode
            .record_delivery(|| bmc_wasm_runtime::unified_fixture::UnifiedEvent::ClockSet { time });
    }

    /// Moves the calendar and nothing else: no time elapses, so nothing ages
    /// and a recording carries the move as one event rather than a span.
    fn paint_clock_jump(&mut self, ui: &mut egui::Ui, palette: &super::theme::Palette, field: f32) {
        // Before anything is read off the fields, so 31 February
        // cannot be spelled and then refused.
        self.clock_picker.clamp_day();
        // The one moment the fields can still spell that
        // does not exist: the hour a spring-forward skips.
        //
        // Said, rather than swallowed by a button that quietly does nothing.
        let target = self.clock_picker.resolve();

        ui.label(egui::RichText::new("Jump to").strong());
        ui.label(
            egui::RichText::new(match target {
                Some(_) => "Nothing ages.",
                None => "No such local time — daylight saving skips that hour.",
            })
            .color(match target {
                Some(_) => palette.text_secondary,
                None => palette.action_danger,
            }),
        );
        ui.add_space(spacing::S02);
        let mut set = false;
        // Top-aligned, so `Set` sits level with the fields rather
        // than being centred against a segment that is one label taller.
        ui.horizontal_top(|group| {
            // Tight between segments; the date and the time part at S05.
            group.spacing_mut().item_spacing.x = SEGMENT_GAP;
            let last_day = self.clock_picker.last_day_of_month();
            let picker = &mut self.clock_picker;
            segment(group, "Year", &mut picker.year, 1970..=2100, field, palette);
            segment(group, "Month", &mut picker.month, 1..=12, field, palette);
            // The month's own last day, so the field cannot leave the calendar.
            segment(group, "Day", &mut picker.day, 1..=last_day, field, palette);
            group.add_space(spacing::S05);
            segment(group, "Hour", &mut picker.hour, 0..=23, field, palette);
            segment(group, "Minute", &mut picker.minute, 0..=59, field, palette);
            group.add_space(spacing::S05);
            set = Button::inline("Set")
                .icon(&mut self.icons.delay)
                .tone(Tone::primary(palette))
                .min_height(field)
                .enabled(target.is_some())
                .show(group, palette)
                .clicked();
        });

        if set && let Some(target) = target {
            self.move_simulated_clock((target - chrono::Local::now()).num_milliseconds());
        }
    }

    /// Lets time elapse, so polls come due and the widgets' data ages.
    ///
    /// A recording carries the whole span, which replay walks a frame
    /// at a time — cheap by the minute, ruinous by the month.
    fn paint_clock_nudges(
        &mut self,
        ui: &mut egui::Ui,
        palette: &super::theme::Palette,
        field: f32,
    ) {
        ui.label(egui::RichText::new("Let time pass").strong());
        ui.label(
            egui::RichText::new("Data ages, so polls come due.").color(palette.text_secondary),
        );
        ui.add_space(spacing::S02);
        let mut advance_ms = 0_u64;
        ui.horizontal(|group| {
            if Button::inline("+1m")
                .min_height(field)
                .show(group, palette)
                .clicked()
            {
                advance_ms = 60_000;
            }
            if Button::inline("+5m")
                .min_height(field)
                .show(group, palette)
                .clicked()
            {
                advance_ms = 300_000;
            }
        });
        // Both clocks, unlike a jump: the span has to be real for
        // the widgets to age across it and for a recording to carry it.
        self.clock.monotonic_offset_ms += advance_ms;
        self.state_mut().clock_offset_ms += advance_ms.cast_signed();
    }

    fn paint_theme_switch(&mut self, end: &mut egui::Ui) {
        let palette = self.theme.palette(end.ctx());
        end.scope(|set| {
            set.spacing_mut().item_spacing.x = 1.0;
            // The layout runs right to left, so reversing paints Auto/Dark/Light.
            for choice in super::theme::ThemeChoice::ALL.iter().rev() {
                let selected = self.theme == *choice;
                let icon = match choice {
                    super::theme::ThemeChoice::Auto => &mut self.icons.theme_auto,
                    super::theme::ThemeChoice::Dark => &mut self.icons.theme_dark,
                    super::theme::ThemeChoice::Light => &mut self.icons.theme_light,
                };
                if Button::bar(choice.label())
                    .icon(icon)
                    .tone(Tone::switch(palette, selected))
                    .show(set, palette)
                    .on_hover_text(choice.describe())
                    .clicked()
                {
                    self.theme = *choice;
                }
            }
        });
    }
}

/// Between two controls that are the same operation at different settings —
/// closer to each other than to the groups on either side.
const PAIR_GAP: f32 = 10.0;

/// Whether the widget's manifest admits at least one of `platform`'s
/// viewports at the platform's display density.
pub(super) fn platform_supported(
    platform: &Platform,
    manifest: &bmc_widget_manifest::Manifest,
) -> bool {
    let dpi = platform.display().dpi;
    platform.viewports.iter().any(|vp| {
        manifest.supports_viewport_at_dpi(
            manifest_viewport_shape(vp.shape),
            vp.width,
            vp.height,
            dpi,
        )
    })
}

/// Whether the manifest admits `target` at its platform's display density.
pub(super) fn target_recordable(
    manifest: &bmc_widget_manifest::Manifest,
    target: bmc_wasm_runtime::platform_catalog::Target,
) -> bool {
    manifest.supports_viewport_at_dpi(
        manifest_viewport_shape(target.viewport.shape),
        target.viewport.width,
        target.viewport.height,
        target.platform.display().dpi,
    )
}

/// Sunk one tone under the popover, so each setter reads as its own block
/// rather than as a run of controls beneath a heading.
///
/// Edge to edge and flush against its neighbour: a box with air around it
/// reads as a card floating on the surface rather than as a band of it.
fn section_frame(palette: &super::theme::Palette) -> egui::Frame {
    egui::Frame::NONE
        .fill(palette.layer)
        .inner_margin(egui::Margin::same(SECTION_PAD))
}

/// One named part of a date or time: the field, and under it what it is.
///
/// Named rather than initialled, and below rather than beside — an initial
/// in front of a number reads as part of the value, and `M` serves month
/// and minute equally badly. Zero-padded, so the row reads as a stamp.
fn segment(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
    height: f32,
    palette: &super::theme::Palette,
) {
    // Wide enough for the longest thing it will hold
    // — its own name, or the widest value the range admits
    // — so a larger text style cannot clip it.
    let width = super::ui_helpers::field_width(ui, label)
        .max(super::ui_helpers::field_width(ui, &range.end().to_string()));
    ui.vertical(|column| {
        column.add_sized(
            [width, height],
            egui::DragValue::new(value)
                .range(range)
                .speed(DATE_DRAG_SPEED)
                .custom_formatter(|n, _| format!("{n:02.0}")),
        );
        column.allocate_ui_with_layout(
            egui::vec2(width, 0.0),
            egui::Layout::top_down(egui::Align::Center),
            |under| {
                under.label(egui::RichText::new(label).color(palette.text_secondary));
            },
        );
    });
}

/// Every (platform, viewport) the manifest admits, per [`target_recordable`].
pub(super) fn recordable_targets(
    manifest: &bmc_widget_manifest::Manifest,
) -> Vec<bmc_wasm_runtime::platform_catalog::Target> {
    PLATFORMS
        .iter()
        .flat_map(|platform| {
            platform.viewports.iter().filter_map(move |vp| {
                let target = bmc_wasm_runtime::platform_catalog::Target {
                    platform,
                    viewport: vp,
                };
                target_recordable(manifest, target).then_some(target)
            })
        })
        .collect()
}
