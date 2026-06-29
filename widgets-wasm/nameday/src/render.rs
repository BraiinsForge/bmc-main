// Copyright (C) 2026  Braiins Systems s.r.o.

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

pub const STALE_DATA_TEXT: &str = "Stale data";

const STALE_TEXT_SIZE: u32 = 14;
const STALE_ICON_PX: f32 = 16.0;
const STALE_INSET: f32 = 8.0;

// A warning icon and the message on a panel, pinned to the bottom-left corner.
// The insets make the node absolutely positioned, so it floats over the view
// without taking part in its layout.
pub fn stale_banner() -> Node {
    row(
        props!(inset_bottom: STALE_INSET, inset_left: STALE_INSET),
        [row(
            props!(
                background: GRAY_100,
                padding: 6.0,
                gap: 6.0,
                cross_align: CrossAlign::Center
            ),
            [
                canvas(
                    props!(width: STALE_ICON_PX, height: STALE_ICON_PX),
                    [Draw::svg_builtin(
                        0.0,
                        0.0,
                        STALE_ICON_PX,
                        STALE_ICON_PX,
                        ICON_WARN_FILLED,
                        RED_50,
                    )],
                ),
                text(
                    STALE_DATA_TEXT,
                    style!(size: STALE_TEXT_SIZE, weight: FontWeight::BOLD, color: RED_50, line_height: 1.0),
                ),
            ],
        )],
    )
}
