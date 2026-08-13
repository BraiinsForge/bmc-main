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

use bmc_wasm_runtime::platform_catalog::{DisplayShape, PLATFORMS, Platform};

use super::TestbedApp;

/// Tall enough for buttons with breathing room; the toolbar owns its height
/// now that nothing derives the window size from the tiles.
pub(super) const TOOLBAR_H: f32 = 36.0;

/// Keeps the outermost buttons off the window edges.
const BAR_INLINE_PAD: f32 = 8.0;

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
                area.painter().rect_filled(rect, 0.0, palette.panel_fill);
                let mut bar = area.new_child(
                    egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(BAR_INLINE_PAD, 0.0))),
                );
                bar.horizontal_centered(|row| {
                    let recording = self.recording_mode.state.is_some();
                    // A segmented set: independent on/off per device, one flat
                    // control. The 1 px gaps let the panel show through as
                    // dividers, so neighbouring inactive buttons read apart.
                    row.scope(|set| {
                        set.spacing_mut().item_spacing.x = 1.0;
                        for p in PLATFORMS {
                            let open = self.open_platforms.iter().any(|o| o.id == p.id);
                            let supported = platform_supported(p, &self.manifest);
                            let response = set
                                .add_enabled(
                                    supported && !recording,
                                    egui::Button::new(p.id.to_uppercase()).selected(open),
                                )
                                .on_hover_text(p.label)
                                .on_disabled_hover_text(if recording {
                                    "the platform is pinned while recording"
                                } else {
                                    "the manifest admits no viewport of this platform"
                                });
                            if response.clicked() {
                                chosen = Some(p.id);
                            }
                        }
                    });
                    group_divider(row, palette.divider);

                    self.paint_view_controls(row);
                    group_divider(row, palette.divider);

                    // The widget's own build and its rendering.
                    if icon_button(row, &mut self.icons.reload, Some("Reload WASM"), false)
                        .on_hover_text("re-read the widget's wasm from disk")
                        .clicked()
                    {
                        self.hot_reload.manual_reload = true;
                    }
                    let debug_on = bmc_render::tree::debug_layout_enabled();
                    if icon_button(row, &mut self.icons.debug, Some("Debug layout"), debug_on)
                        .on_hover_text("outline every layout node in the widget render")
                        .clicked()
                    {
                        bmc_render::tree::toggle_debug_layout();
                    }
                    group_divider(row, palette.divider);

                    // Simulated conditions: what the device can and cannot
                    // reach, and when it thinks it is.
                    let offline = self.offline;
                    if icon_button(row, &mut self.icons.offline, Some("Offline"), offline)
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

    /// Where the devices sit and how large they read.
    ///
    /// Arrangements are one-shot and leave the windows free-form after; the
    /// zoom is the canvas's, not each window's, so the devices stay
    /// comparable at a glance.
    fn paint_view_controls(&mut self, row: &mut egui::Ui) {
        if icon_button(row, &mut self.icons.arrange_cascade, Some("Stack"), false)
            .on_hover_text("cascade the device windows from the top left")
            .clicked()
        {
            self.arrange = Some(super::device_window::ArrangeMode::Stack);
        }
        if icon_button(row, &mut self.icons.arrange_grid, Some("Pack"), false)
            .on_hover_text("pack the device windows to use the canvas tightly")
            .clicked()
        {
            self.arrange = Some(super::device_window::ArrangeMode::Pack);
        }
        if icon_button(row, &mut self.icons.scale_out, Some("Fit"), false)
            .on_hover_text("scale the canvas until every device window is in view")
            .clicked()
        {
            self.canvas.request_fit();
        }
        let at_full_size = (self.canvas.zoom() - 1.0).abs() < f32::EPSILON;
        if row
            .add_enabled_ui(!at_full_size, |ui| {
                icon_button(ui, &mut self.icons.scale_in, Some("100%"), false)
                    .on_hover_text("show the devices at their own pixel size")
            })
            .inner
            .clicked()
        {
            let centre = self.canvas.rect.center();
            self.canvas.zoom_about(1.0, centre);
        }
    }

    /// An advance bumps both the display and monotonic offsets, to reach
    /// time-gated states like staleness; "reset" zeroes only the display one
    /// so the monotonic clock never rewinds past pending deadlines.
    fn paint_clock_controls(&mut self, row: &mut egui::Ui) {
        let secs = self.clock.offset_ms / 1_000;
        row.label(format!("Clock +{}:{:02}", secs / 60, secs % 60));
        if row.button("+1m").clicked() {
            self.clock.offset_ms += 60_000;
            self.clock.monotonic_offset_ms += 60_000;
        }
        if row.button("+5m").clicked() {
            self.clock.offset_ms += 300_000;
            self.clock.monotonic_offset_ms += 300_000;
        }
        if row
            .button("Reset")
            .on_hover_text("return the clock to real time")
            .clicked()
        {
            self.clock.offset_ms = 0;
        }
    }

    fn paint_theme_switch(&mut self, end: &mut egui::Ui) {
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
                if icon_button(set, icon, Some(choice.label()), selected)
                    .on_hover_text(choice.describe())
                    .clicked()
                {
                    self.theme = *choice;
                }
            }
        });
    }
}

/// Toolbar icons are square and sized to sit beside the button text.
const ICON_SIZE: f32 = 14.0;

/// How far a group divider stops short of the toolbar's edges.
const DIVIDER_INSET: f32 = 9.0;

/// Separate two groups of controls.
///
/// Painted rather than `Separator`-ed: egui's rules the bar's full height,
/// which partitions regions instead of parting neighbours.
fn group_divider(row: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = row.allocate_exact_size(
        egui::vec2(row.spacing().item_spacing.x * 2.0, TOOLBAR_H),
        egui::Sense::hover(),
    );
    row.painter().vline(
        rect.center().x,
        egui::Rangef::new(rect.top() + DIVIDER_INSET, rect.bottom() - DIVIDER_INSET),
        egui::Stroke::new(1.0_f32, color),
    );
}

/// A button carrying an icon, and optionally a label beside it.
///
/// The icon takes whatever text colour the button's own state resolves to,
/// so it dims, highlights and inverts exactly as the label does.
fn icon_button(
    ui: &mut egui::Ui,
    icon: &mut super::icon::Icon,
    label: Option<&str>,
    selected: bool,
) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let text = label.map(|label| {
        ui.painter().layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            egui::Color32::PLACEHOLDER,
        )
    });
    let gap = if text.is_some() {
        ui.spacing().icon_spacing
    } else {
        0.0
    };
    let content = egui::vec2(
        ICON_SIZE + gap + text.as_ref().map_or(0.0, |t| t.size().x),
        ICON_SIZE.max(text.as_ref().map_or(0.0, |t| t.size().y)),
    );
    let (rect, response) = ui.allocate_exact_size(content + 2.0 * padding, egui::Sense::click());

    let visuals = ui.style().interact_selectable(&response, selected);
    ui.painter()
        .rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);

    let inner = rect.shrink2(padding);
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(inner.min.x, inner.center().y - ICON_SIZE / 2.0),
        egui::Vec2::splat(ICON_SIZE),
    );
    icon.paint(ui, icon_rect, visuals.fg_stroke.color);
    if let Some(text) = text {
        let pos = egui::pos2(
            icon_rect.max.x + gap,
            inner.center().y - text.size().y / 2.0,
        );
        ui.painter().galley(pos, text, visuals.fg_stroke.color);
    }
    response
}

/// Whether the widget's manifest admits at least one of `platform`'s
/// viewports at the platform's display density.
fn platform_supported(platform: &Platform, manifest: &bmc_widget_manifest::Manifest) -> bool {
    let dpi = platform.display().dpi;
    platform.viewports.iter().any(|vp| {
        let shape = match vp.shape {
            DisplayShape::Rectangular => bmc_widget_manifest::ViewportShape::Rectangular,
            DisplayShape::Round => bmc_widget_manifest::ViewportShape::Round,
        };
        manifest.supports_viewport_at_dpi(shape, vp.width, vp.height, dpi)
    })
}
