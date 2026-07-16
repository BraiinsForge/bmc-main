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

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const HEADER_COLOR: Color = GRAY_60;

#[derive(Clone, Copy)]
struct SizeParams {
    header_font_size: u32,
    header_pad_top: u32,
    header_pad_left: u32,

    fact_font_size: u32,
    fact_padding: u32,
}

const SMALL: SizeParams = SizeParams {
    header_font_size: 24,
    header_pad_top: 8,
    header_pad_left: 16,
    fact_font_size: 64,
    fact_padding: 8,
};
const MEDIUM: SizeParams = SizeParams {
    header_font_size: 24,
    header_pad_top: 8,
    header_pad_left: 16,
    fact_font_size: 96,
    fact_padding: 16,
};
const LARGE: SizeParams = SizeParams {
    header_font_size: 24,
    header_pad_top: 8,
    header_pad_left: 16,
    fact_font_size: 120,
    fact_padding: 16,
};
const FULL: SizeParams = SizeParams {
    header_font_size: 32,
    header_pad_top: 16,
    header_pad_left: 16,
    fact_font_size: 200,
    fact_padding: 16,
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
            header_font_size: scale_font(self.header_font_size, fit),
            fact_font_size: scale_font(self.fact_font_size, fit),
            ..self
        }
    }
}

/// Static "Random Facts" label, pinned to the top, left-centered.
/// The `inset_*` props make the node absolutely positioned so it floats above
/// the centered fact without taking part in its layout.
pub(super) fn header_draw(ws: WidgetSize) -> Node {
    let size_params = get_size_params(ws.variant).scaled(ws.fit());

    row(
        props!(
            inset_top: size_params.header_pad_top,
            inset_left: size_params.header_pad_left,
        ),
        [text(
            "Random Facts",
            style!(
                size: size_params.header_font_size,
                weight: FontWeight::BOLD,
                color: HEADER_COLOR,
            ),
        )],
    )
}

/// The fact text, centered in the whole tile, wrapped across lines and
/// automatically fitted.
pub(super) fn fact_draw(ws: WidgetSize, fact: &str) -> Node {
    let size_params = get_size_params(ws.variant).scaled(ws.fit());

    // Fact region: full width, height left under the header.
    let header_band = size_params.header_pad_top + size_params.header_font_size;
    let region_h = ws
        .height
        .saturating_sub(header_band + 2 * size_params.fact_padding);
    let region_w = ws.width.saturating_sub(size_params.fact_padding * 2);

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
        props!(width: region_w, height: region_h, inset_top: header_band + size_params.fact_padding, inset_left: size_params.fact_padding),
        vec![Draw::autofit_text(
            0.0,
            0.0,
            box_w,
            box_h,
            fact,
            style!(
                size: size_params.fact_font_size,
                weight: FontWeight::BOLD,
                align: TextAlign::Center,
                valign: VerticalAlign::Center,
            ),
        )],
    )
}
