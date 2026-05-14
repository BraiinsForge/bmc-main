// Copyright (C) 2026  Braiins Systems s.r.o.

//! Read-back exemplar for the manifest params slice of the SDK.
//! Every cell on screen is what `params::current().get_*("key")` returned this frame.
//!
//! When the operator changes a param at runtime, the affected cell briefly
//! tints amber and fades back — proof that `on_params_update` is wired
//! and that `previous()` correctly preserves the pre-change snapshot for diffing.
//!
//! Stage F coverage. The matching manifest declares one entry
//! per `ParamKind` variant plus the structural-flag matrix
//!  - `format:`
//!  - `min/max/step`
//!  - `enum_values`
//!  - `optional` w/o `default`
//!
//! The four optional-without-default rows render as `(unset)`
//! until an operator wires a value (or again as `(unset)` if cleared);
//!
//! Every other cell carries the manifest default — or the operator's override
//! once one is set — through the host-side params plumbing into the SDK's
//! `params::current()`.

// Generated from `manifest.json` by `bmc-widget-codegen`; regenerate
// via `just wasm::gen params-demo` after editing the manifest.
mod manifest_params;

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

use manifest_params::Params;

use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    /// Per-key milliseconds-remaining on the change-decay highlight.
    /// Bumped to `DECAY_MS` inside `on_params_update` for every key whose value differs
    /// from the previous snapshot, decremented by `delta_ms` at the end of every `render`.
    /// `BTreeMap<String, u32>` keeps key iteration deterministic; bounded by manifest size
    /// (14 entries today), so the allocation cost is irrelevant.
    static DECAY_MS_REMAINING: RefCell<BTreeMap<String, u32>> =
        const { RefCell::new(BTreeMap::new()) };
}

const BG_COLOR: Color = Color::from_hex(0x14_16_1B);
/// Decay duration of the amber change-highlight tint, in milliseconds.
/// 800 ms is long enough to register as "something flashed" without lingering
/// past the next likely operator action.
const DECAY_MS: u32 = 800;
/// Peak alpha of the amber tint at t=0 of the decay window.
/// Decays linearly to 0 over `DECAY_MS`.
const TINT_PEAK_ALPHA: f32 = 0.45;

/// Lifecycle hook fired by the host on every params delivery after the first.
/// Diffs `current()` against `previous()` and stamps every changed key with a fresh decay
/// window; the render path consumes the per-key counter to tint the affected cells.
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let Some(previous) = Params::previous() else {
        return;
    };
    let changed = Params::current().changed_keys(&previous);
    if changed.is_empty() {
        return;
    }
    DECAY_MS_REMAINING.with(|m| {
        let mut m = m.borrow_mut();
        for key in changed {
            m.insert(key.to_owned(), DECAY_MS);
        }
    });
    request_frame();
}

/// Background tint for a cell whose key is mid-decay. Returns transparent when no decay is active.
/// Alpha fades linearly from `TINT_PEAK_ALPHA` at t=0 to 0 at t=`DECAY_MS`.
fn decay_tint(key: &str) -> Color {
    let remaining = DECAY_MS_REMAINING.with(|m| m.borrow().get(key).copied().unwrap_or(0));
    if remaining == 0 {
        return Color::from_rgba(0, 0, 0, 0);
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "`remaining` and `DECAY_MS` are both <= 800; f32 round-trip is exact"
    )]
    let alpha = TINT_PEAK_ALPHA * (remaining as f32 / DECAY_MS as f32);
    YELLOW_30.with_alpha(alpha)
}

/// Advance the decay state by `delta_ms` and return whether any decay is still active.
/// Called once at the end of every `render`; if it returns `true`, the widget requests another
/// frame so the tint continues animating without operator interaction.
fn tick_decay(delta_ms: u32) -> bool {
    DECAY_MS_REMAINING.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|_, remaining| {
            *remaining = remaining.saturating_sub(delta_ms);
            *remaining > 0
        });
        !m.is_empty()
    })
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
    section_header: 18,
    key: 14,
    hint: 11,
    value: 14,
    col_padding: 14.0,
    col_gap: 10.0,
    cell_gap: 8.0,
    label_width: 140.0,
    footer_size: 12,
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
pub extern "C" fn render(delta_ms: u32) {
    let size = widget_size();
    let w = size.width;
    let h = size.height;
    let p = Params::current();

    match size.variant {
        // Small (317×238) and Medium (638×238) both lack the vertical room for the two-line
        // grid layout (Required has 10 entries, each a key+hint line pair). They fall back to
        // the compact single-line variant; Medium just gets wider rows than Small.
        SizeVariant::Small | SizeVariant::Medium => render_compact(w, h, &p, &SIZES_SMALL),
        SizeVariant::Large | SizeVariant::Full => {
            render_grid(w, h, &p, &SIZES_FULL);
        }
    }

    // Advance the per-key decay-highlight counters and keep requesting frames while any cell is
    // still mid-fade. The host caps the cadence at `animation_frame_delay_ms` (~33 ms), so the
    // decay budget gets ~24 frames at 800 ms — smooth enough to read as a fade.
    if tick_decay(delta_ms) {
        request_frame();
    }
}

/// Compact single-column layout for the small variant.
/// Each row is a one-liner (`key  value`) — the hint subtitle is dropped to win vertical
/// density, and the full 317 px tile width is available for values, so URLs and longer strings
/// don't wrap.
/// All 14 keys stack with comfortable gap.
fn render_compact(w: u32, h: u32, p: &Params, sizes: &Sizes) {
    let _ = render_ui(
        w,
        h,
        col(
            props!(flex: 1.0, gap: sizes.cell_gap, background: GRAY_90.with_alpha(0.85), padding: sizes.col_padding),
            [
                kv_line("free_string", &p.free_string, sizes),
                kv_line(
                    "string_enum",
                    fmt!(
                        "{} ({})",
                        p.string_enum.as_manifest_value(),
                        p.string_enum.as_manifest_label()
                    ),
                    sizes,
                ),
                kv_line("string_uri", &p.string_uri, sizes),
                kv_line("string_date", &p.string_date, sizes),
                kv_line("integer_range", fmt!("{}", p.integer_range), sizes),
                kv_line(
                    "integer_enum",
                    fmt!(
                        "{} ({})",
                        p.integer_enum.as_manifest_value(),
                        p.integer_enum.as_manifest_label()
                    ),
                    sizes,
                ),
                kv_line("double_range", format_f64_fixed(p.double_range, 2), sizes),
                kv_line(
                    "double_enum",
                    fmt!(
                        "{} ({})",
                        format_f64_fixed(p.double_enum.as_manifest_value(), 2),
                        p.double_enum.as_manifest_label(),
                    ),
                    sizes,
                ),
                kv_line(
                    "boolean_flag",
                    if p.boolean_flag { "on" } else { "off" },
                    sizes,
                ),
                kv_line("tz", &p.tz, sizes),
                kv_line_opt_str("optional_string", p.optional_string.as_deref(), sizes),
                kv_line_opt_i32("optional_integer", p.optional_integer, sizes),
                kv_line_opt_f64("optional_double", p.optional_double, sizes),
                kv_line_opt_bool("optional_boolean", p.optional_boolean, sizes),
            ],
        ),
    );
}

/// Single-line cell used by [`render_compact`].
/// Wider label column than the 2-line variant since there's no hint underneath.
/// Background routes through [`decay_tint`] so the cell briefly flashes amber
/// when its key changes (transparent the rest of the time).
fn kv_line(key: &str, value: impl Into<String>, sizes: &Sizes) -> Node {
    row(
        props!(gap: sizes.col_gap, background: decay_tint(key)),
        [
            col(
                props!(width: sizes.label_width),
                [text(key, style!(size: sizes.key, color: GRAY_40))],
            ),
            text(value, style!(size: sizes.value, color: GRAY_10)),
        ],
    )
}

fn kv_line_opt_str(key: &str, value: Option<&str>, sizes: &Sizes) -> Node {
    kv_line(key, value.unwrap_or("(unset)"), sizes)
}

fn kv_line_opt_i32(key: &str, value: Option<i32>, sizes: &Sizes) -> Node {
    match value {
        Some(v) => kv_line(key, fmt!("{v}"), sizes),
        None => kv_line(key, "(unset)", sizes),
    }
}

fn kv_line_opt_f64(key: &str, value: Option<f64>, sizes: &Sizes) -> Node {
    match value {
        Some(v) => kv_line(key, format_f64_fixed(v, 2), sizes),
        None => kv_line(key, "(unset)", sizes),
    }
}

fn kv_line_opt_bool(key: &str, value: Option<bool>, sizes: &Sizes) -> Node {
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
fn render_grid(w: u32, h: u32, p: &Params, sizes: &Sizes) {
    let required = col(
        props!(flex: 1.0, gap: sizes.cell_gap, background: GRAY_90.with_alpha(0.85), padding: sizes.col_padding),
        [
            section_header("Required (per ParamKind variant)", sizes),
            kv("free_string", "", &p.free_string, sizes),
            kv(
                "string_enum",
                "enum_values",
                fmt!(
                    "{} ({})",
                    p.string_enum.as_manifest_value(),
                    p.string_enum.as_manifest_label()
                ),
                sizes,
            ),
            kv("string_uri", "format: uri", &p.string_uri, sizes),
            kv("string_date", "format: date", &p.string_date, sizes),
            kv(
                "integer_range",
                "min/max/step",
                fmt!("{}", p.integer_range),
                sizes,
            ),
            kv(
                "integer_enum",
                "enum_values",
                fmt!(
                    "{} ({})",
                    p.integer_enum.as_manifest_value(),
                    p.integer_enum.as_manifest_label()
                ),
                sizes,
            ),
            kv(
                "double_range",
                "min/max/step",
                format_f64_fixed(p.double_range, 2),
                sizes,
            ),
            kv(
                "double_enum",
                "enum_values",
                fmt!(
                    "{} ({})",
                    format_f64_fixed(p.double_enum.as_manifest_value(), 2),
                    p.double_enum.as_manifest_label(),
                ),
                sizes,
            ),
            kv(
                "boolean_flag",
                "",
                if p.boolean_flag { "on" } else { "off" },
                sizes,
            ),
            kv("tz", "Timezone", &p.tz, sizes),
        ],
    );

    // The header speaks of 14 keys because the typed struct mirrors the manifest one-to-one;
    // the optional cells fall back to `(unset)` when the host delivered null.
    let optional = col(
        props!(flex: 1.0, gap: sizes.cell_gap, background: GRAY_90.with_alpha(0.85), padding: sizes.col_padding),
        [
            section_header("Optional, no default (null-on-wire)", sizes),
            kv_opt_str("optional_string", "", p.optional_string.as_deref(), sizes),
            kv_opt_i32("optional_integer", "", p.optional_integer, sizes),
            kv_opt_f64("optional_double", "", p.optional_double, sizes),
            kv_opt_bool("optional_boolean", "", p.optional_boolean, sizes),
            spacer(1.0),
            text(
                "Snapshot carries 14 key(s)",
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
            props!(background: BG_COLOR, gap: sizes.col_gap, flex: 1.0),
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
/// Label column width and font sizes come from [`Sizes`] so the same shape renders
/// at multiple scales without forking the cell structure.
///
/// Background routes through [`decay_tint`] so the cell briefly flashes amber
/// when its key changes (transparent the rest of the time).
fn kv(key: &str, hint: &str, value: impl Into<String>, sizes: &Sizes) -> Node {
    row(
        props!(gap: sizes.col_gap, background: decay_tint(key)),
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

fn kv_opt_str(key: &str, hint: &str, value: Option<&str>, sizes: &Sizes) -> Node {
    kv(key, hint, value.unwrap_or("(unset)"), sizes)
}

fn kv_opt_i32(key: &str, hint: &str, value: Option<i32>, sizes: &Sizes) -> Node {
    match value {
        Some(v) => kv(key, hint, fmt!("{v}"), sizes),
        None => kv(key, hint, "(unset)", sizes),
    }
}

fn kv_opt_f64(key: &str, hint: &str, value: Option<f64>, sizes: &Sizes) -> Node {
    match value {
        Some(v) => kv(key, hint, format_f64_fixed(v, 2), sizes),
        None => kv(key, hint, "(unset)", sizes),
    }
}

fn kv_opt_bool(key: &str, hint: &str, value: Option<bool>, sizes: &Sizes) -> Node {
    let display = match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "(unset)",
    };
    kv(key, hint, display, sizes)
}
