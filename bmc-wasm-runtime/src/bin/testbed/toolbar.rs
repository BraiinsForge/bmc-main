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
//! (reload, offline seal, debug layout, clock fast-forward).
//!
//! Everything here acts on the testbed as a whole; per-widget state
//! stays in the right sidebar.

use bmc_wasm_runtime::platform_catalog::{PLATFORMS, Platform, manifest_viewport_shape};

use super::TestbedApp;

/// Tall enough for a stacked icon over its label, with breathing room; the
/// toolbar owns its height now that nothing derives the window size from
/// the tiles.
pub(super) const TOOLBAR_H: f32 = 48.0;

/// Keeps the outermost buttons off the window edges.
pub(super) const BAR_INLINE_PAD: f32 = 8.0;

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
                    // While the record mode is engaged it owns the whole bar:
                    // nothing else the toolbar offers belongs to a take.
                    if self.paint_record_controls(row, palette) {
                        return;
                    }
                    super::ui_helpers::group_divider(row, palette.border_subtle, TOOLBAR_H);

                    chosen = self.paint_platform_toggles(row, palette);
                    super::ui_helpers::group_divider(row, palette.border_subtle, TOOLBAR_H);

                    self.paint_view_controls(row);
                    super::ui_helpers::group_divider(row, palette.border_subtle, TOOLBAR_H);

                    // The widget's own build and its rendering.
                    if bar_button(row, Some(&mut self.icons.reload), "Reload", false, palette)
                        .on_hover_text("re-read the widget's wasm from disk")
                        .clicked()
                    {
                        self.hot_reload.manual_reload = true;
                    }
                    let debug_on = bmc_render::tree::debug_layout_enabled();
                    if bar_button(row, Some(&mut self.icons.debug), "Debug", debug_on, palette)
                        .on_hover_text(
                            "outline every layout node in the widget render, \
                             and egui's own inspector over the chrome",
                        )
                        .clicked()
                    {
                        bmc_render::tree::toggle_debug_layout();
                    }
                    let timings_on = self.show_view_timings;
                    if bar_button(
                        row,
                        Some(&mut self.icons.timer),
                        "Timings",
                        timings_on,
                        palette,
                    )
                    .on_hover_text("show each view's own frame cost over it")
                    .clicked()
                    {
                        self.show_view_timings = !self.show_view_timings;
                    }
                    super::ui_helpers::group_divider(row, palette.border_subtle, TOOLBAR_H);

                    // Simulated conditions: what the device can and cannot
                    // reach, and when it thinks it is.
                    let offline = self.offline;
                    if bar_button(
                        row,
                        Some(&mut self.icons.offline),
                        "Offline",
                        offline,
                        palette,
                    )
                    .on_hover_text("seal live I/O, mirroring an offline device")
                    .clicked()
                    {
                        self.offline = !self.offline;
                    }
                    self.paint_clock_controls(row);

                    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |end| {
                        self.paint_theme_switch(end);
                    });
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
                let response = set
                    .add_enabled_ui(supported, |ui| {
                        let icon = &mut self.icons.devices;
                        bar_button(
                            ui,
                            Some(icon),
                            &super::ui_helpers::platform_name(p),
                            open,
                            palette,
                        )
                    })
                    .inner
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
    /// Off, this is one red button that opens the choosing phase. Engaged —
    /// choosing or mid-take — the mode owns the bar: a status chip says which
    /// phase is on, Save lands the take, Cancel backs out.
    /// Returns whether the mode is engaged, hiding the rest of the bar.
    fn paint_record_controls(
        &mut self,
        row: &mut egui::Ui,
        palette: &super::theme::Palette,
    ) -> bool {
        let choosing = self.recording_mode.is_choosing();
        let recording = self.recording_mode.active().is_some();
        let accent = palette.accent_record;

        if !choosing && !recording {
            // Red in rest too: this is the one destructive-adjacent control
            // on the bar, and its colour is its identity.
            let response = row
                .scope(|ui| {
                    let widgets = &mut ui.style_mut().visuals.widgets;
                    widgets.inactive.fg_stroke.color = accent;
                    widgets.hovered.fg_stroke.color = accent;
                    widgets.active.fg_stroke.color = accent;
                    let cause = if self.cli.perf_report_path.is_some() {
                        Some("a profiling run cannot record")
                    } else if self.cli.resolved_widget_root().is_none() {
                        Some("recording needs a widget root to write the fixture into")
                    } else if recordable_targets(&self.manifest).is_empty() {
                        Some("the manifest admits no viewport")
                    } else {
                        None
                    };
                    let inner = ui
                        .add_enabled_ui(cause.is_none(), |ui| {
                            bar_button(ui, Some(&mut self.icons.record), "Record", false, palette)
                                .on_hover_text("record a capture fixture from a live take")
                        })
                        .inner;
                    match cause {
                        Some(cause) => inner.on_disabled_hover_text(cause),
                        None => inner,
                    }
                })
                .inner;
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
        let save = super::ui_helpers::accent_button(
            row,
            "Save",
            super::ui_helpers::Accent::record(palette),
            save_cause.is_none(),
            palette,
        )
        .on_hover_text("write the fixture and leave the take");
        if let Some(cause) = save_cause {
            save.on_disabled_hover_text(cause);
        } else if save.clicked() {
            let ctx = row.ctx().clone();
            self.save_recording(&ctx);
        }
        if super::ui_helpers::with_pointer(row.button("Cancel"))
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
        if bar_button(
            row,
            Some(&mut self.icons.arrange_grid),
            "Tile",
            false,
            palette,
        )
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
            fit = bar_button(pair, Some(&mut self.icons.scale_out), "Fit", false, palette)
                .on_hover_text("scale the canvas until every device window is in view")
                .clicked();
            full_size = pair
                .add_enabled_ui(!at_full_size, |ui| {
                    bar_button(ui, Some(&mut self.icons.scale_in), "100%", false, palette)
                        .on_hover_text("show the devices at their own pixel size")
                })
                .inner
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

    /// An advance bumps both the display and monotonic offsets, to reach
    /// time-gated states like staleness; "reset" zeroes only the display one
    /// so the monotonic clock never rewinds past pending deadlines.
    fn paint_clock_controls(&mut self, row: &mut egui::Ui) {
        let palette = self.theme.palette(row.ctx());
        let secs = self.clock.offset_ms / 1_000;
        let offset = format!("+{}:{:02}", secs / 60, secs % 60);
        bar_readout(row, Some(&mut self.icons.delay), &offset)
            .on_hover_text("how far the simulated clock runs ahead of real time");

        let mut advance_ms = 0_u64;
        let mut reset = false;
        row.scope(|group| {
            group.spacing_mut().item_spacing.x = 1.0;
            if bar_button(group, Some(&mut self.icons.delay), "+1m", false, palette).clicked() {
                advance_ms = 60_000;
            }
            if bar_button(group, Some(&mut self.icons.delay), "+5m", false, palette).clicked() {
                advance_ms = 300_000;
            }
            reset = bar_button(group, Some(&mut self.icons.delay), "Reset", false, palette)
                .on_hover_text("return the clock to real time")
                .clicked();
        });
        self.clock.offset_ms += advance_ms;
        self.clock.monotonic_offset_ms += advance_ms;
        if reset {
            self.clock.offset_ms = 0;
        }
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
                if bar_button(set, Some(icon), choice.label(), selected, palette)
                    .on_hover_text(choice.describe())
                    .clicked()
                {
                    self.theme = *choice;
                }
            }
        });
    }
}

/// Toolbar icons are square and sized to sit over the button text.
const ICON_SIZE: f32 = 14.0;

/// Between a stacked button's icon and its label.
const STACK_GAP: f32 = 2.0;

/// Between two controls that are the same operation at different settings —
/// closer to each other than to the groups on either side.
const PAIR_GAP: f32 = 10.0;

/// Width every button takes at least, so a one-word control and its
/// three-word neighbour still read as the same kind of thing.
const MIN_BUTTON_W: f32 = 54.0;

/// A button on the bar: an icon over its label, both centred.
///
/// Stacked rather than lengthwise, because the bar carries enough controls
/// that laying each one out sideways ran it off the window's edge. The icon
/// row is reserved whether or not a control has one, and every button takes
/// at least [`MIN_BUTTON_W`], so labels line up across the bar and neighbours
/// do not vary in size for the length of a word.
///
/// The icon takes whatever text colour the button's own state resolves to,
/// so it dims, highlights and inverts exactly as the label does.
fn bar_button(
    ui: &mut egui::Ui,
    icon: Option<&mut super::icon::Icon>,
    label: &str,
    selected: bool,
    palette: &super::theme::Palette,
) -> egui::Response {
    let (rect, response) = allocate_bar_slot(ui, label, egui::Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    // `interact_selectable` pins a selected control to the selection colour
    // whatever the pointer is doing, so a selected button has to answer hover
    // itself or it answers not at all.
    let fill = if selected && response.hovered() {
        palette.action_primary_hover
    } else {
        visuals.weak_bg_fill
    };
    ui.painter().rect_filled(rect, visuals.corner_radius, fill);
    paint_bar_slot(ui, rect, icon, label, visuals.fg_stroke.color);
    super::ui_helpers::with_pointer(response)
}

/// A reading on the bar, shaped like the buttons around it but inert.
fn bar_readout(
    ui: &mut egui::Ui,
    icon: Option<&mut super::icon::Icon>,
    label: &str,
) -> egui::Response {
    let (rect, response) = allocate_bar_slot(ui, label, egui::Sense::hover());
    let colour = ui.visuals().weak_text_color();
    paint_bar_slot(ui, rect, icon, label, colour);
    response
}

/// The room one control takes on the bar, uniform by construction.
fn allocate_bar_slot(
    ui: &mut egui::Ui,
    label: &str,
    sense: egui::Sense,
) -> (egui::Rect, egui::Response) {
    let padding = ui.spacing().button_padding;
    let text = bar_label(ui, label);
    let content = egui::vec2(
        text.size().x.max(ICON_SIZE),
        ICON_SIZE + STACK_GAP + text.size().y,
    );
    let size = egui::vec2(
        (content.x + 2.0 * padding.x).max(MIN_BUTTON_W),
        content.y + 2.0 * padding.y,
    );
    ui.allocate_exact_size(size, sense)
}

fn paint_bar_slot(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: Option<&mut super::icon::Icon>,
    label: &str,
    colour: egui::Color32,
) {
    let inner = rect.shrink2(ui.spacing().button_padding);
    if let Some(icon) = icon {
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(inner.center().x - ICON_SIZE / 2.0, inner.min.y),
            egui::Vec2::splat(ICON_SIZE),
        );
        icon.paint(ui, icon_rect, colour);
    }
    let text = bar_label(ui, label);
    ui.painter().galley(
        egui::pos2(
            inner.center().x - text.size().x / 2.0,
            inner.max.y - text.size().y,
        ),
        text,
        colour,
    );
}

fn bar_label(ui: &egui::Ui, label: &str) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        egui::Color32::PLACEHOLDER,
    )
}

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
