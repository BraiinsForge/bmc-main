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

//! Chrome theming: one typed palette per theme, applied as egui visuals.
//!
//! The look is flat — background fills do the contrasting, corner rounding
//! and widget strokes are off. Colours come from the design system's swatches
//! wherever the ramp has a step for them; device-mock tones darker than the
//! ramp's floor stay literal. Device-mock colours are identical across
//! themes: a bezel is dark hardware, not chrome.

use bmc_wasm_protocol::colors as swatch;
use egui::Color32;

/// Every colour the testbed paints outside of widget textures.
pub(crate) struct Palette {
    /// Whether egui's dark visuals are the base this palette overrides.
    pub(crate) dark_base: bool,
    // Canvas.
    pub(crate) canvas_a: Color32,
    pub(crate) canvas_b: Color32,
    // Chrome.
    pub(crate) panel_fill: Color32,
    /// A window's frame: its border and the title strip continuing it.
    /// A tone of its own, so stacked windows read apart.
    pub(crate) title_fill: Color32,
    /// Behind a window's content, so the content never fills its own back.
    pub(crate) window_body: Color32,
    /// The strip under the pointer — the only cue that it can be dragged.
    pub(crate) title_hover_fill: Color32,
    /// Separators between control groups.
    pub(crate) divider: Color32,
    /// Face of interactive fields: text inputs, selects, checkboxes, buttons.
    pub(crate) field_fill: Color32,
    /// Section header bars in the sidebar.
    pub(crate) section_fill: Color32,
    // Device mock — identical in both themes.
    pub(crate) bezel: Color32,
    pub(crate) strip_plate: Color32,
    pub(crate) placeholder_fill: Color32,
    pub(crate) placeholder_outline: Color32,
    pub(crate) placeholder_glyph: Color32,
    pub(crate) record_accent: Color32,
    pub(crate) record_slab: Color32,
}

pub(crate) const DARK: Palette = Palette {
    dark_base: true,
    canvas_a: swatch::GRAY_100.to_egui(),
    canvas_b: swatch::GRAY_90.to_egui(),
    panel_fill: swatch::GRAY_100.to_egui(),
    title_fill: swatch::GRAY_80.to_egui(),
    window_body: swatch::GRAY_90.to_egui(),
    title_hover_fill: swatch::GRAY_70.to_egui(),
    divider: swatch::GRAY_80.to_egui(),
    field_fill: swatch::GRAY_80.to_egui(),
    section_fill: swatch::BLACK.to_egui(),
    bezel: BEZEL,
    strip_plate: swatch::GRAY_90.to_egui(),
    placeholder_fill: PLACEHOLDER_FILL,
    placeholder_outline: swatch::GRAY_90.to_egui(),
    placeholder_glyph: swatch::GRAY_70.to_egui(),
    record_accent: swatch::ORANGE_40.to_egui(),
    record_slab: RECORD_SLAB,
};

pub(crate) const LIGHT: Palette = Palette {
    dark_base: false,
    canvas_a: swatch::GRAY_30.to_egui(),
    canvas_b: swatch::GRAY_20.to_egui(),
    panel_fill: swatch::GRAY_20.to_egui(),
    title_fill: swatch::GRAY_30.to_egui(),
    window_body: swatch::GRAY_10.to_egui(),
    title_hover_fill: swatch::GRAY_40.to_egui(),
    divider: swatch::GRAY_30.to_egui(),
    // A full step off the panel: without widget strokes, the face fill is
    // all that separates a control from the panel it sits on.
    field_fill: swatch::WHITE.to_egui(),
    section_fill: swatch::GRAY_80.to_egui(),
    bezel: BEZEL,
    strip_plate: swatch::GRAY_90.to_egui(),
    placeholder_fill: PLACEHOLDER_FILL,
    placeholder_outline: swatch::GRAY_90.to_egui(),
    placeholder_glyph: swatch::GRAY_70.to_egui(),
    record_accent: swatch::ORANGE_40.to_egui(),
    record_slab: RECORD_SLAB,
};

/// Corner rounding on a window's head, shared by the frame and the title
/// strip that paints over it.
pub(crate) const WINDOW_RADIUS: u8 = 6;

// Device tones darker than the swatch ramp's floor (`GRAY_100`).
const BEZEL: Color32 = Color32::from_gray(8);
const PLACEHOLDER_FILL: Color32 = Color32::from_gray(14);
const RECORD_SLAB: Color32 = Color32::from_gray(12);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeChoice {
    Auto,
    Dark,
    Light,
}

impl ThemeChoice {
    pub(crate) const ALL: [Self; 3] = [Self::Auto, Self::Dark, Self::Light];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Auto => "Follow the system theme",
            Self::Dark => "Dark theme",
            Self::Light => "Light theme",
        }
    }

    /// The palette this choice means right now; Auto follows the system.
    pub(crate) fn palette(self, ctx: &egui::Context) -> &'static Palette {
        match self {
            Self::Dark => &DARK,
            Self::Light => &LIGHT,
            Self::Auto => match ctx.input(|i| i.raw.system_theme) {
                Some(egui::Theme::Light) => &LIGHT,
                Some(egui::Theme::Dark) | None => &DARK,
            },
        }
    }
}

/// Install the palette as the context's style, flattened:
/// no widget strokes, no corner rounding — fills carry the contrast.
pub(crate) fn apply(ctx: &egui::Context, palette: &Palette) {
    let mut visuals = if palette.dark_base {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    // Two tones per window: the frame — border and title strip alike — and
    // the body behind its content. A third reads as a seam between them.
    visuals.window_fill = palette.window_body;
    visuals.panel_fill = palette.panel_fill;
    // Rounded head, square foot: the body is a device mock filling its own
    // corners, so rounding the foot would clip the mock, not the chrome.
    visuals.window_corner_radius = egui::CornerRadius {
        nw: WINDOW_RADIUS,
        ne: WINDOW_RADIUS,
        sw: 0,
        se: 0,
    };
    visuals.window_stroke = egui::Stroke::new(1.0_f32, palette.title_fill);
    // Enough to lift a window off the canvas without smearing over its
    // neighbours; `PACK_GAP` leaves room for it even when packed.
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(72),
    };
    // One selection colour for both themes: the light base's pale blue
    // carries white text, which is unreadable on light chrome.
    visuals.selection.bg_fill = swatch::BLUE_60.to_egui();
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, swatch::WHITE.to_egui());
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::ZERO;
        widget.bg_stroke = egui::Stroke::NONE;
    }
    // The noninteractive stroke is what `Separator` draws with; blanket-NONE
    // above would erase every group divider along with the widget outlines.
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, palette.divider);
    // One face tone for every field — text inputs paint `extreme_bg_color`,
    // buttons and selects their widget fills — so controls sit on the panel
    // as one family instead of a white box here and a grey lump there.
    visuals.extreme_bg_color = palette.field_fill;
    visuals.widgets.inactive.bg_fill = palette.field_fill;
    visuals.widgets.inactive.weak_bg_fill = palette.field_fill;
    ctx.style_mut(|style| {
        style.visuals = visuals.clone();
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}
