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

    /// Max length of string with names to print. Empirically measured
    /// with respect to font (style and size) used for printing names.
    max_names_len: usize,
}

const SMALL: SizeParams = SizeParams {
    name_font_size: 28,
    date_font_size: 24,
    country_pad_left: 16,
    country_pad_top: 8,
    date_pad_bottom: 16,
    max_names_len: 40,
};
const MEDIUM: SizeParams = SizeParams {
    name_font_size: 40,
    date_font_size: 24,
    country_pad_left: 16,
    country_pad_top: 8,
    date_pad_bottom: 16,
    max_names_len: 40,
};
const LARGE: SizeParams = SizeParams {
    name_font_size: 64,
    date_font_size: 24,
    country_pad_left: 16,
    country_pad_top: 8,
    date_pad_bottom: 16,
    max_names_len: 40,
};
const FULL: SizeParams = SizeParams {
    name_font_size: 64,
    date_font_size: 32,
    country_pad_left: 16,
    country_pad_top: 16,
    date_pad_bottom: 24,
    max_names_len: 80,
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

/// The compiled-SVG viewBox, read from the binary header emitted by
/// `bmc-svg-compiler`: `[viewbox_w: f32 LE][viewbox_h: f32 LE]…`. The host
/// scales X and Y independently, so a non-square glyph must be fitted by the
/// caller or it comes out stretched.
fn svg_viewbox(svg: &Svg) -> (f32, f32) {
    let d = svg.data;
    if d.len() >= 8 {
        let w = f32::from_le_bytes([d[0], d[1], d[2], d[3]]);
        let h = f32::from_le_bytes([d[4], d[5], d[6], d[7]]);
        if w > 0.0 && h > 0.0 {
            return (w, h);
        }
    }
    (1.0, 1.0)
}

/// Fit `svg` inside a `size`×`size` box preserving its aspect ratio and
/// centering it — matching a browser's default `xMidYMid meet`.
fn icon_draw(svg: &'static Svg, size: f32, color: Color) -> Draw {
    let (vw, vh) = svg_viewbox(svg);
    let scale = (size / vw).min(size / vh);
    let w = vw * scale;
    let h = vh * scale;
    Draw::svg((size - w) / 2.0, (size - h) / 2.0, w, h, svg, color)
}

/// A multi-color flag of selected country.
#[must_use]
pub(super) fn country_icon(country: Country, size: f32) -> Node {
    canvas(
        props!(width: size, height: size),
        vec![icon_draw(icons::get_flag_svg(country), size, TRANSPARENT)],
    )
}

pub(super) fn names_draw(ws: WidgetSize, names: &str) -> Node {
    let size_params = get_size_params(ws.variant).scaled(ws.fit());

    let names = truncate_with_ellipsis(names, size_params.max_names_len);

    center(
        props!(),
        [text(
            names,
            style!(
                size: size_params.name_font_size,
                weight: FontWeight::BOLD,
                text_overflow: TextOverflow::Wrap,
                align: TextAlign::Center
            ),
        )],
    )
}

/// Truncate `text` to at most `max_chars` characters, appending an ellipsis
/// when it overflows. Counts characters, not bytes, so a multi-byte text is
/// never split mid-`char` (which would panic) and the budget matches the
/// font measurement regardless of accents. A trailing separator is dropped
/// so the ellipsis never follows a `,` or space.
fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    const ELLIPSIS_STRING: &str = "...";

    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let keep = max_chars.saturating_sub(ELLIPSIS_STRING.chars().count());
    let mut out: String = text.chars().take(keep).collect();

    while out.ends_with(',') || out.ends_with(' ') {
        out.pop();
    }

    out.push_str(ELLIPSIS_STRING);
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_string_shorter_than_limit_unchanged() {
        assert_eq!(truncate_with_ellipsis("Adam", 40), "Adam");
    }

    #[test]
    fn keeps_string_exactly_at_limit_unchanged() {
        let names = "abcde";
        assert_eq!(truncate_with_ellipsis(names, 5), names);
    }

    #[test]
    fn truncates_and_appends_ellipsis_when_over_limit() {
        // 6 chars truncated to 5: keep 2 chars, then "...".
        assert_eq!(truncate_with_ellipsis("abcdef", 5), "ab...");
    }

    #[test]
    fn result_never_exceeds_limit() {
        let out = truncate_with_ellipsis("abcdefghij", 5);
        assert_eq!(out.chars().count(), 5);
    }

    #[test]
    fn counts_characters_not_bytes_for_multibyte_input() {
        // Each 'á' is two bytes but one char; the limit is in chars.
        let names = "áááááá";
        assert_eq!(names.chars().count(), 6);
        let out = truncate_with_ellipsis(names, 5);
        assert_eq!(out, "áá...");
        assert_eq!(out.chars().count(), 5);
    }

    #[test]
    fn drops_trailing_comma_before_ellipsis() {
        // "Adam, Eva" is 9 chars; limit 8 keeps the first 5 ("Adam,"), and
        // the trailing comma is dropped before the ellipsis is appended.
        assert_eq!(truncate_with_ellipsis("Adam, Eva", 8), "Adam...");
    }

    #[test]
    fn drops_trailing_comma_and_space_before_ellipsis() {
        // "Adam, Eva, Bob" limit 9 keeps the first 6 ("Adam, "); both the
        // trailing space and the comma are stripped, leaving "Adam".
        assert_eq!(truncate_with_ellipsis("Adam, Eva, Bob", 9), "Adam...");
    }

    #[test]
    fn handles_limit_smaller_than_ellipsis() {
        // keep saturates to 0, so only the ellipsis remains.
        assert_eq!(truncate_with_ellipsis("abcdef", 2), "...");
        assert_eq!(truncate_with_ellipsis("abcdef", 0), "...");
    }

    #[test]
    fn keeps_empty_string_unchanged() {
        assert_eq!(truncate_with_ellipsis("", 40), "");
    }
}
