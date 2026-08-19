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
//! wherever the ramp has a step for them.
//!
//! The device mock is themed like the rest: only the widget's own texture is
//! outside the testbed's control, and everything drawn around it — enclosure,
//! LED strip, the slab standing in for a view — is chrome.

use bmc_wasm_protocol::colors as swatch;
use egui::Color32;

/// Every colour the testbed paints outside of widget textures.
pub(crate) struct Palette {
    /// Whether egui's dark visuals are the base this palette overrides.
    pub(crate) dark_base: bool,

    /// The ground the `layer_*` surfaces stack on, and each theme's extreme.
    /// Nothing sits under it, so it is what a device's unlit display shows.
    pub(crate) background: Color32,

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

    // `data_*` — a categorical series, after Carbon's `$data-N`.
    // The order tells one series from the next and nothing else,
    // so what each stands for is the chart's to decide.
    //
    // One set for both themes: the scale is built to separate on either.
    pub(crate) data: [Color32; 5],
}

pub(crate) const DARK: Palette = Palette {
    dark_base: true,
    background: swatch::BLACK.to_egui(),
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
    data: DATA_SERIES,
};

pub(crate) const LIGHT: Palette = Palette {
    dark_base: false,
    background: swatch::WHITE.to_egui(),
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
    data: DATA_SERIES,
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

/// The first five of d3's `schemeCategory10` (matplotlib's `tab10`), rather
/// than picks from the swatch: a categorical scale has to separate five hues
/// at a glance, which the ramp is not built for.
///
/// Scale order, with the last two swapped. The chart stacks these five in a
/// 2 px column, where the scale's green-then-red would touch — the pairing
/// deuteranopia loses, with no room for shape or spacing to carry it.
const DATA_SERIES: [Color32; 5] = [
    Color32::from_rgb(0x1F, 0x77, 0xB4),
    Color32::from_rgb(0xFF, 0x7F, 0x0E),
    Color32::from_rgb(0x2C, 0xA0, 0x2C),
    Color32::from_rgb(0x94, 0x67, 0xBD),
    Color32::from_rgb(0xD6, 0x27, 0x28),
];

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
