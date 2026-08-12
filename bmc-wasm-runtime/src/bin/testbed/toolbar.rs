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

/// Pinned rather than content-sized: the fixed tile layout still derives
/// the window height, so it has to account for this slice of it.
pub(super) const TOOLBAR_H: f32 = 32.0;

impl TestbedApp {
    pub(super) fn paint_toolbar(&mut self, root_ui: &mut egui::Ui) {
        let mut chosen: Option<&'static str> = None;
        egui::TopBottomPanel::top("toolbar")
            .exact_height(TOOLBAR_H)
            .show_inside(root_ui, |ui| {
                ui.horizontal_centered(|row| {
                    let recording = self.recording_mode.state.is_some();
                    for p in PLATFORMS {
                        let selected = p.id == self.active_platform.id;
                        let supported = platform_supported(p, &self.manifest);
                        let response = row
                            .add_enabled(
                                supported && !recording,
                                egui::SelectableLabel::new(selected, p.id),
                            )
                            .on_hover_text(p.label)
                            .on_disabled_hover_text(if recording {
                                "the platform is pinned while recording"
                            } else {
                                "the manifest admits no viewport of this platform"
                            });
                        if response.clicked() && !selected {
                            chosen = Some(p.id);
                        }
                    }
                    row.separator();

                    if row.button("Reload WASM").clicked() {
                        self.hot_reload.manual_reload = true;
                    }
                    row.checkbox(&mut self.offline, "Offline")
                        .on_hover_text("seal live I/O, mirroring an offline device");
                    let mut debug_on = bmc_render::tree::debug_layout_enabled();
                    if row.checkbox(&mut debug_on, "Debug layout").changed() {
                        bmc_render::tree::toggle_debug_layout();
                    }
                    row.separator();

                    // An advance bumps both the display and monotonic offsets,
                    // to reach time-gated states like staleness; "reset" zeroes
                    // only the display one so the monotonic clock never rewinds
                    // past pending deadlines.
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
                    if row.button("reset").clicked() {
                        self.clock.offset_ms = 0;
                    }
                });
            });

        if let Some(target) = chosen {
            let ctx = root_ui.ctx().clone();
            self.switch_platform(target, &ctx);
        }
    }
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
