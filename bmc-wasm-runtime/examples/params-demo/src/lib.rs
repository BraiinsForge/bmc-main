// Copyright (C) 2026  Braiins Systems s.r.o.

//! Read-back exemplar for the manifest params slice of the SDK.
//! Every cell on screen is what `bmc_wasm_sdk::params::current().get_*("key")` returned this frame.
//! No interactive controls, no animations — the widget is honest about its role.
//!
//! Stage F coverage. The matching manifest declares one entry per `ParamKind` variant
//! plus the structural-flag matrix
//!  - `format:`
//!  - `min/max/step`
//!  - `enum_values`
//!  - `optional` w/o `default`
//!
//! The four optional-without-default rows render as `(unset)` until an operator
//! wires a value (or again as `(unset)` if cleared); every other cell carries
//! the manifest default — or the operator's override once one is set — through
//! the host-side params plumbing into the SDK's `params::current()`.

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

use std::cell::Cell;

thread_local! {
    static WIDTH: Cell<u32> = const { Cell::new(1_280) };
    static HEIGHT: Cell<u32> = const { Cell::new(480) };
}

const BG_COLOR: Color = Color::from_hex(0x14_16_1B);

#[unsafe(no_mangle)]
pub extern "C" fn init(width: u32, height: u32) {
    WIDTH.set(width);
    HEIGHT.set(height);
}

/// Lifecycle hook fired by the host on every params delivery after the first.
/// Dormant until BDK-432 PLAN stage E lands the host-side invocation; defined here so the export
/// is in place when the gate flips.
/// Canonical pattern: read `current()`, optionally diff against `previous()`, schedule a
/// re-render if anything visible changed.
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let _current = params::current();
    let _previous = params::previous();
    request_frame();
}

/// Layout-tunable sizes per widget-size variant.
/// `small` shrinks fonts / padding / gaps so the full per-key grid fits in 317×238; bigger
/// variants get more generous spacing for legibility.
#[derive(Clone, Copy)]
struct Sizes {
    section_header: u32,
    key: u32,
    hint: u32,
    value: u32,
    col_padding: f32,
    col_gap: f32,
    cell_gap: f32,
    label_width: f32,
    footer_size: u32,
}

const SIZES_FULL: Sizes = Sizes {
    section_header: 14,
    key: 11,
    hint: 8,
    value: 11,
    col_padding: 12.0,
    col_gap: 8.0,
    cell_gap: 6.0,
    label_width: 110.0,
    footer_size: 10,
};

const SIZES_SMALL: Sizes = Sizes {
    section_header: 9,
    key: 10,
    hint: 8,
    value: 10,
    col_padding: 8.0,
    col_gap: 8.0,
    cell_gap: 2.0,
    label_width: 110.0,
    footer_size: 7,
};

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let w = WIDTH.get();
    let h = HEIGHT.get();
    let p = params::current();

    let size = WidgetSize::from_dimensions(w, h);
    match size.variant {
        // Small tiles (317×238) drop the "Required / Optional" split and pack all 14 entries
        // across two balanced columns of seven; the section headers + the optional column's
        // tail are luxuries we don't have the pixels for.
        SizeVariant::Small => render_compact(w, h, &p, &SIZES_SMALL),
        SizeVariant::Medium | SizeVariant::Large | SizeVariant::Full => {
            render_grid(w, h, &p, &SIZES_FULL);
        }
    }
}

/// Compact single-column layout for the small variant.
/// Each row is a one-liner (`key  value`) — the hint subtitle is dropped to win vertical
/// density, and the full 317 px tile width is available for values, so URLs and longer strings
/// don't wrap.
/// All 14 keys stack with comfortable gap.
fn render_compact(w: u32, h: u32, p: &params::Params, sizes: &Sizes) {
    let _ = render_ui(
        w,
        h,
        col(
            props!(flex: 1.0, gap: sizes.cell_gap, background: GRAY_90.with_alpha(0.85), padding: sizes.col_padding),
            [
                kv_line_str("free_string", p.get_str("free_string"), sizes),
                kv_line_str("string_enum", p.get_str("string_enum"), sizes),
                kv_line_str("string_uri", p.get_str("string_uri"), sizes),
                kv_line_str("string_date", p.get_str("string_date"), sizes),
                kv_line_i32("integer_range", p.get_i32("integer_range"), sizes),
                kv_line_i32("integer_enum", p.get_i32("integer_enum"), sizes),
                kv_line_f64("double_range", p.get_f64("double_range"), sizes),
                kv_line_f64("double_enum", p.get_f64("double_enum"), sizes),
                kv_line_bool("boolean_flag", p.get_bool("boolean_flag"), sizes),
                kv_line_str("tz", p.get_str("tz"), sizes),
                kv_line_str("optional_string", p.get_str("optional_string"), sizes),
                kv_line_i32("optional_integer", p.get_i32("optional_integer"), sizes),
                kv_line_f64("optional_double", p.get_f64("optional_double"), sizes),
                kv_line_bool("optional_boolean", p.get_bool("optional_boolean"), sizes),
            ],
        ),
    );
}

/// Single-line cell used by [`render_compact`].
/// Wider label column than the 2-line variant since there's no hint underneath.
fn kv_line(key: &str, value: impl Into<String>, sizes: &Sizes) -> Node {
    row(
        props!(gap: sizes.col_gap),
        [
            col(
                props!(width: sizes.label_width),
                [text(key, style!(size: sizes.key, color: GRAY_40))],
            ),
            text(value, style!(size: sizes.value, color: GRAY_10)),
        ],
    )
}

fn kv_line_str(key: &str, value: Option<&str>, sizes: &Sizes) -> Node {
    kv_line(key, value.unwrap_or("(unset)"), sizes)
}

fn kv_line_i32(key: &str, value: Option<i32>, sizes: &Sizes) -> Node {
    match value {
        Some(v) => kv_line(key, fmt!("{v}"), sizes),
        None => kv_line(key, "(unset)", sizes),
    }
}

fn kv_line_f64(key: &str, value: Option<f64>, sizes: &Sizes) -> Node {
    match value {
        Some(v) => kv_line(key, format_f64_fixed(v, 2), sizes),
        None => kv_line(key, "(unset)", sizes),
    }
}

fn kv_line_bool(key: &str, value: Option<bool>, sizes: &Sizes) -> Node {
    let display = match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "(unset)",
    };
    kv_line(key, display, sizes)
}

/// Full per-key grid. Layout tuning comes from [`Sizes`]; the cell structure (label column with
/// hint subtitle, value to the right) stays identical across variants so the small view differs
/// from the others only in pixels, not shape.
fn render_grid(w: u32, h: u32, p: &params::Params, sizes: &Sizes) {
    let required = col(
        props!(flex: 1.0, gap: sizes.cell_gap, background: GRAY_90.with_alpha(0.85), padding: sizes.col_padding),
        [
            section_header("Required (per ParamKind variant)", sizes),
            kv_str("free_string", "", p.get_str("free_string"), sizes),
            kv_str(
                "string_enum",
                "enum_values",
                p.get_str("string_enum"),
                sizes,
            ),
            kv_str("string_uri", "format: uri", p.get_str("string_uri"), sizes),
            kv_str(
                "string_date",
                "format: date",
                p.get_str("string_date"),
                sizes,
            ),
            kv_i32(
                "integer_range",
                "min/max/step",
                p.get_i32("integer_range"),
                sizes,
            ),
            kv_i32(
                "integer_enum",
                "enum_values",
                p.get_i32("integer_enum"),
                sizes,
            ),
            kv_f64(
                "double_range",
                "min/max/step",
                p.get_f64("double_range"),
                sizes,
            ),
            kv_f64(
                "double_enum",
                "enum_values",
                p.get_f64("double_enum"),
                sizes,
            ),
            kv_bool("boolean_flag", "", p.get_bool("boolean_flag"), sizes),
            kv_str("tz", "Timezone", p.get_str("tz"), sizes),
        ],
    );

    let optional = col(
        props!(flex: 1.0, gap: sizes.cell_gap, background: GRAY_90.with_alpha(0.85), padding: sizes.col_padding),
        [
            section_header("Optional, no default (null-on-wire)", sizes),
            kv_str("optional_string", "", p.get_str("optional_string"), sizes),
            kv_i32("optional_integer", "", p.get_i32("optional_integer"), sizes),
            kv_f64("optional_double", "", p.get_f64("optional_double"), sizes),
            kv_bool(
                "optional_boolean",
                "",
                p.get_bool("optional_boolean"),
                sizes,
            ),
            spacer(1.0),
            text(
                fmt!("Snapshot carries {} key(s)", p.keys().count()),
                style!(size: sizes.footer_size, color: GRAY_50),
            ),
            text(
                "(unset) means the host delivered null or the key was absent.",
                style!(size: sizes.footer_size, color: GRAY_50),
            ),
        ],
    );

    let _ = render_ui(
        w,
        h,
        row(
            props!(background: BG_COLOR, padding: sizes.col_padding, gap: sizes.col_gap, flex: 1.0),
            [required, optional],
        ),
    );
}

fn section_header(label: &str, sizes: &Sizes) -> Node {
    text(
        label,
        style!(size: sizes.section_header, weight: 700, color: GRAY_10),
    )
}

/// Two-line label column (key + optional structural hint) followed by the value.
/// Label column width and font sizes come from [`Sizes`] so the same shape renders at multiple
/// scales without forking the cell structure.
fn kv(key: &str, hint: &str, value: impl Into<String>, sizes: &Sizes) -> Node {
    row(
        props!(gap: sizes.col_gap),
        [
            col(
                props!(width: sizes.label_width, gap: 1.0),
                [
                    text(key, style!(size: sizes.key, color: GRAY_40)),
                    text(hint, style!(size: sizes.hint, color: GRAY_60)),
                ],
            ),
            text(value, style!(size: sizes.value, color: GRAY_10)),
        ],
    )
}

fn kv_str(key: &str, hint: &str, value: Option<&str>, sizes: &Sizes) -> Node {
    kv(key, hint, value.unwrap_or("(unset)"), sizes)
}

fn kv_i32(key: &str, hint: &str, value: Option<i32>, sizes: &Sizes) -> Node {
    match value {
        Some(v) => kv(key, hint, fmt!("{v}"), sizes),
        None => kv(key, hint, "(unset)", sizes),
    }
}

fn kv_f64(key: &str, hint: &str, value: Option<f64>, sizes: &Sizes) -> Node {
    match value {
        Some(v) => kv(key, hint, format_f64_fixed(v, 2), sizes),
        None => kv(key, hint, "(unset)", sizes),
    }
}

fn kv_bool(key: &str, hint: &str, value: Option<bool>, sizes: &Sizes) -> Node {
    let display = match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "(unset)",
    };
    kv(key, hint, display, sizes)
}
