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

use crate::manifest_params::Country;

use crate::icons;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const HEADER_COLOR: Color = GRAY_60;
const TIMESTAMP_COLOR: Color = GRAY_60;
const COUNTRY_FONT_SIZE: u32 = 24;
const COUNTRY_ICON_SIZE: f32 = 24.0;
const COUNTRY_TEXT_GAP_SIZE: u32 = 8;

#[derive(Clone, Copy)]
struct SizeParams {
    name_font_size: u32,
    date_font_size: u32,
    country_pad_left: u32,
    country_pad_top: u32,
    date_pad_bottom: u32,
    names_pad_vertical: u32,
    names_pad_horizontal: u32,
}

const SMALL: SizeParams = SizeParams {
    name_font_size: 64,
    date_font_size: 24,
    country_pad_left: 16,
    country_pad_top: 8,
    date_pad_bottom: 16,
    names_pad_vertical: 8,
    names_pad_horizontal: 16,
};
const MEDIUM: SizeParams = SizeParams {
    name_font_size: 96,
    date_font_size: 24,
    country_pad_left: 16,
    country_pad_top: 8,
    date_pad_bottom: 16,
    names_pad_vertical: 8,
    names_pad_horizontal: 16,
};
const LARGE: SizeParams = SizeParams {
    name_font_size: 120,
    date_font_size: 24,
    country_pad_left: 16,
    country_pad_top: 8,
    date_pad_bottom: 16,
    names_pad_vertical: 8,
    names_pad_horizontal: 16,
};
const FULL: SizeParams = SizeParams {
    name_font_size: 200,
    date_font_size: 32,
    country_pad_left: 16,
    country_pad_top: 16,
    date_pad_bottom: 24,
    names_pad_vertical: 16,
    names_pad_horizontal: 16,
};

fn get_size_params(variant: SizeVariant) -> &'static SizeParams {
    match variant {
        SizeVariant::Full => &FULL,
        SizeVariant::Large => &LARGE,
        SizeVariant::Medium => &MEDIUM,
        SizeVariant::Small => &SMALL,
    }
}

impl SizeParams {
    fn scaled(self, fit: f32) -> Self {
        Self {
            name_font_size: scale_font(self.name_font_size, fit),
            date_font_size: scale_font(self.date_font_size, fit),
            ..self
        }
    }
}

/// A multi-color flag of selected country.
#[must_use]
pub(super) fn country_icon(country: Country, size: f32) -> Node {
    canvas(
        props!(width: size, height: size),
        vec![Draw::svg_contain(
            icons::get_flag_svg(country),
            size,
            TRANSPARENT,
        )],
    )
}

pub(super) fn names_draw(ws: WidgetSize, names: &str) -> Node {
    let size_params = get_size_params(ws.variant).scaled(ws.fit());

    // Names region: full width, height left between the country and date bands.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "COUNTRY_ICON_SIZE is 24.0 — positive, fits u32"
    )]
    let country_band = size_params.country_pad_top + COUNTRY_ICON_SIZE as u32;
    let date_band = size_params.date_font_size + size_params.date_pad_bottom;
    let region_h = ws
        .height
        .saturating_sub(country_band + date_band + 2 * size_params.names_pad_vertical);
    let region_w = ws
        .width
        .saturating_sub(size_params.names_pad_horizontal * 2);

    #[expect(
        clippy::cast_precision_loss,
        reason = "widget dimensions are small — u32→f32 is exact well below 2^24"
    )]
    let box_w = region_w as f32;
    #[expect(
        clippy::cast_precision_loss,
        reason = "widget dimensions are small — u32→f32 is exact well below 2^24"
    )]
    let box_h = region_h as f32;

    canvas(
        props!(width: region_w, height: region_h, inset_top: country_band + size_params.names_pad_vertical, inset_left: size_params.names_pad_horizontal),
        vec![Draw::autofit_text(
            0.0,
            0.0,
            box_w,
            box_h,
            names,
            style!(
                size: size_params.name_font_size,
                weight: FontWeight::BOLD,
                align: TextAlign::Center,
                valign: VerticalAlign::Center
            ),
        )],
    )
}

pub(super) fn country_draw(ws: WidgetSize, country: Country) -> Node {
    let size_params = get_size_params(ws.variant).scaled(ws.fit());

    row(
        props!(
            inset_top: size_params.country_pad_top,
            inset_left: size_params.country_pad_left,
            gap: COUNTRY_TEXT_GAP_SIZE,
            cross_align: CrossAlign::Center,
        ),
        [
            country_icon(country, COUNTRY_ICON_SIZE),
            text(
                country.as_manifest_label(),
                style!(
                    size: COUNTRY_FONT_SIZE,
                    weight: FontWeight::BOLD,
                    color: HEADER_COLOR,
                ),
            ),
        ],
    )
}

pub(super) fn date_draw(ws: WidgetSize, date_str: &str) -> Node {
    let size_params = get_size_params(ws.variant).scaled(ws.fit());

    center(
        props!(
            inset_bottom: size_params.date_pad_bottom,
            inset_left: 0.0,
            inset_right: 0.0,
        ),
        [text(
            date_str,
            style!(
                size: size_params.date_font_size,
                weight: FontWeight::REGULAR,
                color: TIMESTAMP_COLOR,
            ),
        )],
    )
}

// The stale indicator now uses the shared `with_stale_overlay` (SDK).
