// Copyright (C) 2026  Braiins Systems s.r.o.

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
