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

//! Read-back exemplar for the manifest params slice of the SDK.
//! Every cell on screen is what `params::current().get_*("key")`
//! returned this frame.
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

use bmc_wasm_sdk::system;
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

/// Outer canvas visible only in the gaps between panes — `TEAL_90`.
/// Slightly lighter than [`PANE_BG`] so the gap reads as a hairline tint
/// between adjacent dark panels.
const BG_COLOR: Color = TEAL_90;
/// Pane (column) fill — `TEAL_100`, opaque. Solid so the pane shade
/// is the same regardless of what's behind it (the SMALL variant has
/// no outer canvas, so any alpha would render it inconsistently
/// against the surrounding testbed UI).
const PANE_BG: Color = TEAL_100;
/// Decay duration of the amber change-highlight tint, in milliseconds.
/// 800 ms is long enough to register as "something flashed" without lingering
/// past the next likely operator action.
const DECAY_MS: u32 = 800;
/// Peak alpha of the amber tint at t=0 of the decay window.
/// Decays linearly to 0 over `DECAY_MS`.
const TINT_PEAK_ALPHA: f32 = 0.45;

/// Every credential slot this widget's manifest declares,
/// sorted by key the way codegen emits them.
///
/// All are rendered, so an unbound slot
/// shows as unbound rather than simply absent.
const CREDENTIAL_SLOTS: [&str; 4] = ["media", "pool", "pool_backup", "weather"];

/// Lifecycle hook fired by the host on every params-snapshot delivery
/// after the first. Diffs `Params::current()` against `Params::previous()`
/// and stamps every changed key with a fresh decay window so the matching
/// demo cells tint amber.
///
/// Channel isolation: this hook fires only on params deliveries. System
/// deliveries land in [`on_system_update`] — the two channels have
/// separate hooks so each diff sees a fresh, just-rotated `previous()`.
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let Some(previous) = Params::previous() else {
        return;
    };
    stamp_decay(Params::current().changed_keys(&previous));
}

/// Spend the declared credentials on a real request, so the exemplar exercises
/// host-side substitution and the egress pin rather than only describing them.
///
/// The URL is the `string_uri` param, which is where a `{{ credential.… }}`
/// placeholder goes to aim one slot's secret at a destination.
///
/// The response is dropped deliberately.
/// This screen showcases params and credential bindings, and rendering
/// an outcome would tie its capture baselines to the request.
/// Where the request went, and whether the host refused it,
/// shows up in the host log instead.
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let url = Params::current().string_uri;
    // The host must not be handed something that is not a request.
    if url.starts_with("http://") || url.starts_with("https://") {
        let _ = net::fetch(&url, None, |_| {});
    }
}

/// Lifecycle hook fired on every credential delivery after the first.
///
/// Without it the widget would still pick up a rebind,
/// but only at its next data tick.
/// The point of the hook is that binding an account
/// changes the display immediately.
#[unsafe(no_mangle)]
pub extern "C" fn on_credentials_update() {
    request_frame();
}

/// Lifecycle hook fired by the host on every system-snapshot delivery
/// after the first. System fields don't have a key-set abstraction —
/// compare each field by value between current and previous snapshots
/// so the matching demo cells tint amber on change.
#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    let cur = system::current();
    let prev = system::previous();
    let mut changed: Vec<&'static str> = Vec::new();
    if cur.timezone() != prev.timezone() {
        changed.push("timezone");
    }
    if cur.time_format() != prev.time_format() {
        changed.push("time_format");
    }
    if cur.date_format() != prev.date_format() {
        changed.push("date_format");
    }
    if cur.number_format() != prev.number_format() {
        changed.push("number_format");
    }
    if cur.first_day_of_week() != prev.first_day_of_week() {
        changed.push("first_day_of_week");
    }
    if cur.temperature_unit() != prev.temperature_unit() {
        changed.push("temperature_unit");
    }
    if cur.unit_system() != prev.unit_system() {
        changed.push("unit_system");
    }
    if next_alarm_value(&cur) != next_alarm_value(&prev) {
        changed.push("next_alarm");
    }
    stamp_decay(changed);
}

/// Push `keys` into the decay map and request a follow-up frame.
/// A no-op on an empty `keys` so callers can `stamp_decay(diff_result)`
/// unconditionally without branching on emptiness.
fn stamp_decay(keys: Vec<&'static str>) {
    if keys.is_empty() {
        return;
    }
    DECAY_MS_REMAINING.with(|m| {
        let mut m = m.borrow_mut();
        for key in keys {
            m.insert(key.to_owned(), DECAY_MS);
        }
    });
    request_frame();
}

/// Owned (fire_at_utc_ms, name) pair for change-detection comparison.
/// `system::next_alarm()` returns a borrowed view that doesn't outlive
/// the `Snapshot`; this helper copies it into an owned shape
/// so a stale `Snapshot` can be dropped between the two reads.
fn next_alarm_value(snap: &system::Snapshot) -> Option<(i64, String)> {
    snap.next_alarm()
        .map(|n| (n.fire_at_utc_ms, n.name.to_owned()))
}

/// Background tint for a cell whose key is mid-decay.
/// Returns transparent when no decay is active.
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
/// Called once at the end of every `render`; if it returns `true`,
/// the widget requests another frame so the tint continues animating
/// without operator interaction.
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
/// `small` shrinks fonts / padding / gaps so the full per-key grid fits in 317×238;
/// bigger variants get more generous spacing for legibility.
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
    cell_gap: 4.0,
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

/// Mid-tier used by the LARGE tile's two-pane layout: bigger than SMALL,
/// smaller than FULL. LARGE has 2× the vertical room of MEDIUM at the same
/// width, so the rows can breathe.
const SIZES_MEDIUM: Sizes = Sizes {
    section_header: 13,
    key: 12,
    hint: 9,
    value: 12,
    col_padding: 10.0,
    col_gap: 10.0,
    cell_gap: 5.0,
    label_width: 130.0,
    footer_size: 9,
};

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let size = widget_size();
    let w = size.width;
    let h = size.height;
    let p = Params::current();

    // Per-variant choice of layout function and `Sizes` tier.
    // Keeping the dispatch in one tuple-match means the actual call site
    // below stays identical across variants — only the (function pointer, sizes)
    // pair differs. A future widget adopting the same scaled-layout pattern
    // can copy this shape verbatim.
    let (render_layout, sizes): (fn(u32, u32, &Params, &Sizes), &Sizes) = match size.variant {
        // Small (317×238): single-column compact — narrow tile,
        // can only afford one stream of rows.
        SizeVariant::Small => (render_compact, &SIZES_SMALL),
        // Medium (638×238): same vertical budget as Small but 2× the width;
        // split into a params/system two-pane so the right half isn't wasted.
        SizeVariant::Medium => (render_two_pane, &SIZES_SMALL),
        // Large (638×480): same two-pane shape as Medium with bigger fonts /
        // padding — LARGE has 2× the height, so rows can breathe.
        SizeVariant::Large => (render_two_pane, &SIZES_MEDIUM),
        // Full (1280×480): three-column grid with key + hint subtitle.
        SizeVariant::Full => (render_grid, &SIZES_FULL),
    };
    render_layout(w, h, &p, sizes);

    // Advance the per-key decay-highlight counters and keep requesting frames
    // while any cell is still mid-fade. The host caps the cadence
    // at `animation_frame_delay_ms` (~33 ms), so the decay budget gets ~24 frames
    // at 800 ms — smooth enough to read as a fade.
    if tick_decay(delta_ms) {
        request_frame();
    }
}

/// Compact single-column layout for the Small variant.
/// Each row is a one-liner (`key  value`) — the hint subtitle
/// is dropped to win vertical density, and the full 317 px tile
/// width is available for values, so URLs and longer strings
/// don't wrap. The 14 params stack with `timezone` + `next_alarm`
/// appended as a teaser of the System snapshot.
fn render_compact(w: u32, h: u32, p: &Params, sizes: &Sizes) {
    let sys = system::current();
    let mut rows = params_rows(p, sizes);
    rows.extend(geometry_rows(sizes));
    rows.push(kv_line(
        "timezone",
        sys.timezone().unwrap_or(MISSING_LABEL),
        sizes,
    ));
    rows.push(kv_line("next_alarm", format_next_alarm(&sys), sizes));
    rows.extend(credential_rows(sizes));
    let _ = render_ui(
        w,
        h,
        col(
            props!(flex: 1.0, gap: sizes.cell_gap, background: PANE_BG, padding: sizes.col_padding),
            rows,
        ),
    );
}

/// Two-pane compact layout for the Medium / Large variants.
/// Left pane: all 14 params; right pane: the full 8-field system snapshot.
/// `Sizes` controls scale so the same shape works at MEDIUM (cramped)
/// and LARGE (breathing room).
fn render_two_pane(w: u32, h: u32, p: &Params, sizes: &Sizes) {
    let pane_props =
        props!(flex: 1.0, gap: sizes.cell_gap, background: PANE_BG, padding: sizes.col_padding);
    let mut right_rows = system_rows(sizes);
    right_rows.extend(geometry_rows(sizes));
    right_rows.extend(credential_rows(sizes));
    let _ = render_ui(
        w,
        h,
        row(
            props!(background: BG_COLOR, gap: sizes.col_gap / 2.0, flex: 1.0),
            [
                col(pane_props, params_rows(p, sizes)),
                col(pane_props, right_rows),
            ],
        ),
    );
}

/// 14-row params list, one `kv_line` per manifest entry.
/// Shared by [`render_compact`] (Small) and [`render_two_pane`] (Medium / Large).
fn params_rows(p: &Params, sizes: &Sizes) -> Vec<Node> {
    vec![
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
    ]
}

/// 8-row system-snapshot list for the right pane of [`render_two_pane`].
/// One `kv_line` per `SystemSnapshot` field; the `next_alarm` cell
/// renders the active alarm (or `(none)`).
fn system_rows(sizes: &Sizes) -> Vec<Node> {
    let sys = system::current();
    vec![
        kv_line("timezone", sys.timezone().unwrap_or(MISSING_LABEL), sizes),
        kv_line("time_format", time_format_label(sys.time_format()), sizes),
        kv_line("date_format", date_format_label(sys.date_format()), sizes),
        kv_line(
            "number_format",
            number_format_label(sys.number_format()),
            sizes,
        ),
        kv_line(
            "first_day_of_week",
            weekday_label(sys.first_day_of_week()),
            sizes,
        ),
        kv_line(
            "temperature_unit",
            temperature_unit_label(sys.temperature_unit()),
            sizes,
        ),
        kv_line("unit_system", unit_system_label(sys.unit_system()), sizes),
        kv_line("next_alarm", format_next_alarm(&sys), sizes),
    ]
}

/// Two `kv_line` rows reporting the SDK's current viewport and display geometry.
/// Shared by the compact, two-pane, and grid layouts so the same readout
/// appears regardless of widget size, making the BFM round/BMC rectangular
/// distinction visible in every variant.
fn geometry_rows(sizes: &Sizes) -> Vec<Node> {
    vec![
        kv_line("viewport", viewport_text(widget_viewport()), sizes),
        kv_line("display", display_text(display_info()), sizes),
    ]
}

fn viewport_shape_label(shape: ViewportShape) -> &'static str {
    match shape {
        ViewportShape::Rectangular => "rectangular",
        ViewportShape::Round => "round",
    }
}

fn display_shape_label(shape: DisplayShape) -> &'static str {
    match shape {
        DisplayShape::Rectangular => "rectangular",
        DisplayShape::Round => "round",
    }
}

fn viewport_text(v: WidgetViewport) -> String {
    fmt!("{}x{} {}", v.width, v.height, viewport_shape_label(v.shape))
}

fn display_text(d: DisplayInfo) -> String {
    fmt!(
        "{}x{} {} dpi={}",
        d.width,
        d.height,
        display_shape_label(d.shape),
        d.dpi
    )
}

#[cfg(test)]
mod geometry_tests {
    use super::{
        DisplayInfo, DisplayShape, ViewportShape, WidgetViewport, display_shape_label,
        display_text, viewport_shape_label, viewport_text,
    };

    #[test]
    fn shape_labels_are_stable() {
        assert_eq!(
            viewport_shape_label(ViewportShape::Rectangular),
            "rectangular"
        );
        assert_eq!(viewport_shape_label(ViewportShape::Round), "round");
        assert_eq!(
            display_shape_label(DisplayShape::Rectangular),
            "rectangular"
        );
        assert_eq!(display_shape_label(DisplayShape::Round), "round");
    }

    #[test]
    fn viewport_text_renders_size_and_shape() {
        let rect = viewport_text(WidgetViewport {
            width: 480,
            height: 320,
            shape: ViewportShape::Rectangular,
        });
        assert_eq!(rect, "480x320 rectangular");

        let round = viewport_text(WidgetViewport {
            width: 480,
            height: 480,
            shape: ViewportShape::Round,
        });
        assert_eq!(round, "480x480 round");
    }

    #[test]
    fn display_text_includes_dpi() {
        let round = display_text(DisplayInfo {
            width: 480,
            height: 480,
            shape: DisplayShape::Round,
            dpi: 1,
        });
        assert_eq!(round, "480x480 round dpi=1");

        let rect = display_text(DisplayInfo {
            width: 1280,
            height: 480,
            shape: DisplayShape::Rectangular,
            dpi: 2,
        });
        assert_eq!(rect, "1280x480 rectangular dpi=2");
    }
}

/// Single-line cell used by [`render_compact`].
/// Wider label column than the 2-line variant since there's no hint underneath.
/// Background routes through [`decay_tint`] so the cell briefly flashes
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

/// One row per credential slot this widget declares, showing
/// the bound account's name, or [`MISSING_LABEL`] when nothing is bound.
///
/// This is the whole of what a widget can learn about its credentials
/// — enough to say "no account yet" instead of failing silently.
///
/// The secrets themselves only ever exist host-side,
/// behind the placeholders in `manifest_params`.
fn credential_rows(sizes: &Sizes) -> Vec<Node> {
    let bound = credentials::current();
    CREDENTIAL_SLOTS
        .into_iter()
        .map(|slot| {
            let account = bound
                .get(slot)
                .map_or(MISSING_LABEL, |b| b.account_name.as_str());
            kv_line(slot, account, sizes)
        })
        .collect()
}

/// Grid-variant cells for the same slots, with the resolved credential type
/// as the hint subtitle. The type is what the host keys the egress policy off,
/// so it belongs next to the account name.
fn credential_cells(sizes: &Sizes) -> Vec<Node> {
    let bound = credentials::current();
    CREDENTIAL_SLOTS
        .into_iter()
        .map(|slot| {
            let (hint, account) = bound.get(slot).map_or(("unbound", MISSING_LABEL), |b| {
                (b.type_id.as_str(), b.account_name.as_str())
            });
            kv(slot, hint, account, sizes)
        })
        .collect()
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

/// Full per-key grid. Layout tuning comes from [`Sizes`];
/// the cell structure (label column with hint subtitle,
/// value to the right) stays identical across variants
/// so the small view differs from the others only in pixels, not shape.
fn render_grid(w: u32, h: u32, p: &Params, sizes: &Sizes) {
    let required = col(
        props!(flex: 1.0, gap: sizes.cell_gap, background: PANE_BG, padding: sizes.col_padding),
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

    // The header speaks of 14 keys because the typed struct mirrors
    // the manifest one-to-one; the optional cells fall back to `(unset)`
    // when the host delivered null.
    let mut optional_cells = vec![
        section_header("Optional, no default (null-on-wire)", sizes),
        kv_opt_str("optional_string", "", p.optional_string.as_deref(), sizes),
        kv_opt_i32("optional_integer", "", p.optional_integer, sizes),
        kv_opt_f64("optional_double", "", p.optional_double, sizes),
        kv_opt_bool("optional_boolean", "", p.optional_boolean, sizes),
        section_header("Credentials (bound account)", sizes),
    ];
    optional_cells.extend(credential_cells(sizes));
    optional_cells.extend([
        spacer(1.0),
        text(
            "Snapshot carries 14 key(s)",
            style!(size: sizes.footer_size, color: GRAY_50),
        ),
        text(
            "(unset) means the host delivered null or the key was absent.",
            style!(size: sizes.footer_size, color: GRAY_50),
        ),
    ]);
    let optional = col(
        props!(flex: 1.0, gap: sizes.cell_gap, background: PANE_BG, padding: sizes.col_padding),
        optional_cells,
    );

    // Third column for the deck-wide system snapshot
    // — separates conceptual groups so the Required and Optional columns
    //   aren't crowded out by the 8 system entries.
    let sys = system::current();
    let system_col = col(
        props!(flex: 1.0, gap: sizes.cell_gap, background: PANE_BG, padding: sizes.col_padding),
        [
            section_header("System (deck-wide)", sizes),
            kv(
                "timezone",
                "IANA identifier",
                sys.timezone().unwrap_or(MISSING_LABEL),
                sizes,
            ),
            kv(
                "time_format",
                "12h / 24h",
                time_format_label(sys.time_format()),
                sizes,
            ),
            kv(
                "date_format",
                "layout / separators",
                date_format_label(sys.date_format()),
                sizes,
            ),
            kv(
                "number_format",
                "thousands / decimal",
                number_format_label(sys.number_format()),
                sizes,
            ),
            kv(
                "first_day_of_week",
                "calendar start",
                weekday_label(sys.first_day_of_week()),
                sizes,
            ),
            kv(
                "temperature_unit",
                "°C / °F",
                temperature_unit_label(sys.temperature_unit()),
                sizes,
            ),
            kv(
                "unit_system",
                "metric / imperial",
                unit_system_label(sys.unit_system()),
                sizes,
            ),
            kv(
                "next_alarm",
                "soonest scheduled",
                format_next_alarm(&sys),
                sizes,
            ),
            kv(
                "viewport",
                "widget rect",
                viewport_text(widget_viewport()),
                sizes,
            ),
            kv("display", "panel rect", display_text(display_info()), sizes),
        ],
    );

    let _ = render_ui(
        w,
        h,
        row(
            props!(background: BG_COLOR, gap: sizes.col_gap / 2.0, flex: 1.0),
            [required, optional, system_col],
        ),
    );
}

fn section_header(label: &str, sizes: &Sizes) -> Node {
    text(
        label,
        style!(size: sizes.section_header, weight: FontWeight::BOLD, color: GRAY_10),
    )
}

/// Two-line label column (key + optional structural hint) followed by the value.
/// Label column width and font sizes come from [`Sizes`] so the same shape renders
/// at multiple scales without forking the cell structure.
///
/// Background routes through [`decay_tint`] so the cell briefly flashes
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

/// Display label for an absent or malformed system snapshot entry.
const MISSING_LABEL: &str = "—";

/// Friendly labels for the six enum-typed system fields. Each accepts
/// the SDK's `Option<T>` directly and renders [`MISSING_LABEL`] when
/// the entry is absent — matches the testbed's system-mutation sidebar.
fn time_format_label(t: Option<system::TimeFormat>) -> &'static str {
    use system::TimeFormat;
    match t {
        Some(TimeFormat::Hour12) => "Hour12",
        Some(TimeFormat::Hour24) => "Hour24",
        None => MISSING_LABEL,
    }
}

fn date_format_label(d: Option<system::DateFormat>) -> &'static str {
    use system::DateFormat;
    match d {
        Some(DateFormat::DdMmYyyyDot) => "DD.MM.YYYY",
        Some(DateFormat::DdMmYyyySlash) => "DD/MM/YYYY",
        Some(DateFormat::DMYyyySlash) => "D/M/YYYY",
        Some(DateFormat::MDYyyySlash) => "M/D/YYYY",
        Some(DateFormat::DdMmYyyyDash) => "DD-MM-YYYY",
        Some(DateFormat::YyyyMDSlash) => "YYYY/M/D",
        Some(DateFormat::YyyyMmDdDot) => "YYYY.MM.DD",
        Some(DateFormat::YyyyMmDdDash) => "YYYY-MM-DD",
        None => MISSING_LABEL,
    }
}

fn number_format_label(n: Option<system::NumberFormat>) -> &'static str {
    use system::NumberFormat;
    match n {
        Some(NumberFormat::SpaceGroupCommaDecimal) => "1 234 567,89",
        Some(NumberFormat::CommaGroupDotDecimal) => "1,234,567.89",
        Some(NumberFormat::DotGroupCommaDecimal) => "1.234.567,89",
        Some(NumberFormat::SpaceGroupDotDecimal) => "1 234 567.89",
        None => MISSING_LABEL,
    }
}

fn weekday_label(w: Option<system::Weekday>) -> &'static str {
    use system::Weekday;
    match w {
        Some(Weekday::Monday) => "Mon",
        Some(Weekday::Tuesday) => "Tue",
        Some(Weekday::Wednesday) => "Wed",
        Some(Weekday::Thursday) => "Thu",
        Some(Weekday::Friday) => "Fri",
        Some(Weekday::Saturday) => "Sat",
        Some(Weekday::Sunday) => "Sun",
        None => MISSING_LABEL,
    }
}

fn temperature_unit_label(u: Option<system::TemperatureUnit>) -> &'static str {
    use system::TemperatureUnit;
    match u {
        Some(TemperatureUnit::Celsius) => "Celsius",
        Some(TemperatureUnit::Fahrenheit) => "Fahrenheit",
        None => MISSING_LABEL,
    }
}

fn unit_system_label(u: Option<system::UnitSystem>) -> &'static str {
    use system::UnitSystem;
    match u {
        Some(UnitSystem::Metric) => "Metric",
        Some(UnitSystem::Imperial) => "Imperial",
        None => MISSING_LABEL,
    }
}

/// Format the system snapshot's next-alarm entry for display in the demo.
/// `"(none)"` when no alarm is scheduled; otherwise `"name @ YYYY-MM-DD HH:MM"`
/// using the host's `chrono` formatter through `strftime`.
fn format_next_alarm(snap: &system::Snapshot) -> String {
    match snap.next_alarm() {
        Some(next) => {
            let when = strftime(next.fire_at_utc_ms / 1000, "%Y-%m-%d %H:%M");
            fmt!("{} @ {}", next.name, when)
        }
        None => "(none)".to_owned(),
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
