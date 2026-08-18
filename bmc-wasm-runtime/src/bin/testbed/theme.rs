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

    // `canvas_*` — the pannable surface the device windows float over.
    /// The checkerboard's two squares.
    pub(crate) canvas: Color32,
    pub(crate) canvas_alt: Color32,

    // `layer_*` — the surfaces chrome sits on, in stacking order.
    /// Panels: the toolbar, the sidebar, a dialog.
    pub(crate) layer: Color32,
    /// Behind a window's content, so the content never fills its own back.
    pub(crate) layer_alt: Color32,
    /// A window's frame: its border and the title strip continuing it.
    /// A tone of its own, so stacked windows read apart.
    pub(crate) layer_accent: Color32,
    /// The title strip under the pointer — the only cue that it can be dragged.
    pub(crate) layer_accent_hover: Color32,
    /// Sunk into a panel: the sidebar's section header bars.
    pub(crate) layer_inset: Color32,

    /// Face of interactive fields: text inputs, selects, checkboxes, buttons.
    pub(crate) field: Color32,
    pub(crate) field_hover: Color32,
    /// Separators between control groups.
    pub(crate) border_subtle: Color32,

    /// Recording mode's identity, carried by everything the mode owns —
    /// the toolbar chip, the choose overlays, the take's panel.
    pub(crate) accent_record: Color32,
    pub(crate) accent_record_hover: Color32,

    // `support_*` — how an outcome reports itself, as against `action_*`,
    // which is something to click.
    pub(crate) support_success: Color32,
    pub(crate) support_error: Color32,

    // `action_*` — what the operator clicks to commit something.
    /// The primary action of a dialog, and the selection highlight — one
    /// colour, so "the thing to click" reads the same wherever it appears.
    pub(crate) action_primary: Color32,
    pub(crate) action_primary_hover: Color32,
    /// A confirming action that destroys something — loud on purpose.
    pub(crate) action_danger: Color32,
    pub(crate) action_danger_hover: Color32,
    /// An action that cannot be taken yet: the whole control goes flat, not
    /// just its label, so it reads as unavailable rather than as low contrast.
    pub(crate) action_disabled: Color32,
    pub(crate) text_disabled: Color32,
    /// Text riding on `action_primary` or `action_danger`, where the chrome's
    /// own text colours have nothing to do with the contrast that matters.
    pub(crate) text_on_color: Color32,

    // `device_*` — the hardware mock, identical in both themes.
    // A bezel is dark plastic, not chrome, so it does not follow the theme.
    pub(crate) device_bezel: Color32,
    /// The LED diffuser's plate.
    pub(crate) device_strip: Color32,
    /// A view held inert while another records.
    pub(crate) device_slab: Color32,
    pub(crate) device_placeholder: Color32,
    pub(crate) device_placeholder_border: Color32,
    pub(crate) device_placeholder_text: Color32,
}

pub(crate) const DARK: Palette = Palette {
    dark_base: true,
    canvas: swatch::GRAY_100.to_egui(),
    canvas_alt: swatch::GRAY_90.to_egui(),
    layer: swatch::GRAY_100.to_egui(),
    layer_alt: swatch::GRAY_90.to_egui(),
    layer_accent: swatch::GRAY_80.to_egui(),
    layer_accent_hover: swatch::GRAY_70.to_egui(),
    layer_inset: swatch::BLACK.to_egui(),
    field: swatch::GRAY_80.to_egui(),
    field_hover: swatch::GRAY_70.to_egui(),
    border_subtle: swatch::GRAY_80.to_egui(),
    accent_record: swatch::ORANGE_50.to_egui(),
    accent_record_hover: swatch::ORANGE_60.to_egui(),
    support_success: swatch::GREEN_60.to_egui(),
    support_error: swatch::RED_60.to_egui(),
    action_primary: swatch::VIOLET_60.to_egui(),
    action_primary_hover: swatch::VIOLET_70.to_egui(),
    action_danger: swatch::RED_60.to_egui(),
    action_danger_hover: swatch::RED_70.to_egui(),
    action_disabled: swatch::GRAY_70.to_egui(),
    text_disabled: swatch::GRAY_50.to_egui(),
    text_on_color: swatch::WHITE.to_egui(),
    device_bezel: BEZEL,
    device_strip: swatch::GRAY_90.to_egui(),
    device_slab: RECORD_SLAB,
    device_placeholder: PLACEHOLDER_FILL,
    device_placeholder_border: swatch::GRAY_90.to_egui(),
    device_placeholder_text: swatch::GRAY_70.to_egui(),
};

pub(crate) const LIGHT: Palette = Palette {
    dark_base: false,
    canvas: swatch::GRAY_30.to_egui(),
    canvas_alt: swatch::GRAY_20.to_egui(),
    layer: swatch::GRAY_20.to_egui(),
    layer_alt: swatch::GRAY_10.to_egui(),
    layer_accent: swatch::GRAY_30.to_egui(),
    layer_accent_hover: swatch::GRAY_40.to_egui(),
    layer_inset: swatch::GRAY_30.to_egui(),
    // A full step off the panel: without widget strokes, the face fill is
    // all that separates a control from the panel it sits on.
    field: swatch::WHITE.to_egui(),
    field_hover: swatch::GRAY_20.to_egui(),
    border_subtle: swatch::GRAY_30.to_egui(),
    accent_record: swatch::ORANGE_50.to_egui(),
    accent_record_hover: swatch::ORANGE_60.to_egui(),
    support_success: swatch::GREEN_60.to_egui(),
    support_error: swatch::RED_60.to_egui(),
    action_primary: swatch::VIOLET_60.to_egui(),
    action_primary_hover: swatch::VIOLET_70.to_egui(),
    action_danger: swatch::RED_60.to_egui(),
    action_danger_hover: swatch::RED_70.to_egui(),
    action_disabled: swatch::GRAY_70.to_egui(),
    text_disabled: swatch::GRAY_50.to_egui(),
    text_on_color: swatch::WHITE.to_egui(),
    device_bezel: BEZEL,
    device_strip: swatch::GRAY_90.to_egui(),
    device_slab: RECORD_SLAB,
    device_placeholder: PLACEHOLDER_FILL,
    device_placeholder_border: swatch::GRAY_90.to_egui(),
    device_placeholder_text: swatch::GRAY_70.to_egui(),
};

/// The spacing ladder, named after IBM Carbon's `$spacing-NN` scale so a gap
/// is chosen from a step rather than typed as a number.
///
/// Only the steps the chrome reaches for are here; add the next one from
/// Carbon's ladder rather than inventing a value between two of these.
pub(crate) mod spacing {
    pub(crate) const S02: f32 = 4.0;
    pub(crate) const S03: f32 = 8.0;
    pub(crate) const S05: f32 = 16.0;
    pub(crate) const S06: f32 = 24.0;
}

/// Control heights, named after Carbon's `$size-*` scale. A field and the
/// button beside it take the same step, which is what keeps a row level.
pub(crate) mod size {
    /// Inline controls in the sidebar's grid, at its density.
    pub(crate) const XS: f32 = 24.0;
    /// A dialog's footer actions, which carry the weight of the decision.
    pub(crate) const LG: f32 = 48.0;
}

/// The shadow under something floating clear of the chrome — a banner over
/// the canvas. Cast long and soft rather than tight and dark: the point is
/// that it reads as *high*, and a dense shadow reads as a thick border.
pub(crate) const OVERLAY_SHADOW: egui::epaint::Shadow = egui::epaint::Shadow {
    offset: [0, 10],
    blur: 28,
    spread: 0,
    color: Color32::from_black_alpha(120),
};

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
    visuals.window_fill = palette.layer_alt;
    visuals.panel_fill = palette.layer;
    // Square: a rounded head needs the frame, the title strip and the body to
    // agree on the same curve, and they are painted by three different things
    // — the seams showed as artefacts at every corner.
    visuals.window_corner_radius = egui::CornerRadius::ZERO;
    visuals.window_stroke = egui::Stroke::new(1.0_f32, palette.layer_accent);
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
    visuals.selection.bg_fill = palette.action_primary;
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
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, palette.border_subtle);
    // One face tone for every field — text inputs paint `extreme_bg_color`,
    // buttons and selects their widget fills — so controls sit on the panel
    // as one family instead of a white box here and a grey lump there.
    visuals.extreme_bg_color = palette.field;
    visuals.widgets.inactive.bg_fill = palette.field;
    visuals.widgets.inactive.weak_bg_fill = palette.field;
    ctx.style_mut(|style| {
        style.visuals = visuals.clone();
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        // egui's default leaves a text field shorter than a button beside it,
        // since the button grows by its padding and the field does not.
        style.spacing.interact_size.y = size::XS;
    });
}
