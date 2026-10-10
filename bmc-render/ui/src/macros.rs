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

//! Keyword-argument sugar over the tree builders.

/// Helper for `button!` macro — converts label to String.
#[doc(hidden)]
pub fn __macro_string_from(s: impl Into<String>) -> String {
    s.into()
}

/// Shorthand for PropsData: `props!()` or `props!(gap: 16, background: 0xFF)`
///
/// Integer literals are automatically converted to `f32` for layout fields
/// (padding, margin, gap, width, height, etc.).
///
/// Supports a special `bg_nine_patch: <NinePatch>` field that expands a
/// `NinePatch` into the underlying `bg_np_*` fields on `PropsData`.
#[macro_export]
macro_rules! props {
    () => { $crate::PropsData::default() };
    ($($field:ident: $value:expr),* $(,)?) => {{
        let mut p = $crate::PropsData::default();
        $($crate::props!(@set p, $field: $value);)*
        p
    }};
    (@set $p:ident, bg_nine_patch: $v:expr) => {{
        let np = $v;
        $p.bg_np_id = np.bitmap_id;
        $p.bg_np_left = np.left;
        $p.bg_np_top = np.top;
        $p.bg_np_right = np.right;
        $p.bg_np_bottom = np.bottom;
    }};
    (@set $p:ident, $field:ident: $v:expr) => {
        $p.$field = $crate::PropsFieldValue::into_field($v);
    };
}

/// Button node with keyword-style options and sensible defaults.
///
/// First positional arg is the click ID (string), second is the label.
/// Style and size accept bare `ButtonStyle` and `ButtonSize` variant names.
///
/// # Examples
/// ```ignore
/// button!("ok", "OK")                                        // Primary, Normal, no icon
/// button!("delete", "Delete", style: Danger)                 // Danger, Normal
/// button!("reset", "Reset", style: Secondary, size: Small)   // Secondary, Small
/// button!("search", "", icon: gear_id)                       // icon-only, Primary, Normal
/// ```
#[macro_export]
macro_rules! button {
    // Accumulator pattern: parse fields one at a time, then emit.
    // Entry point: id, label, then optional keyword args.
    ($id:expr, $label:expr $(, $($rest:tt)*)?) => {
        $crate::button!(@acc
            [$id]
            [$label]
            [style: $crate::ButtonStyle::Primary]
            [size: $crate::ButtonSize::Normal]
            [icon: None]
            [disabled: false]
            [skin: None]
            $($($rest)*)?
        )
    };

    // Terminal — all fields consumed, build the node.
    // `icon:` accepts either `SvgId` or `Option<SvgId>` (std provides
    // `From<T> for Option<T>`, so `.into()` covers both).
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $sk:expr] $(,)?) => {
        $crate::make_button($crate::__macro_string_from($id), $crate::__macro_string_from($label), $s, $sz, $i.into(), $d, $sk)
    };

    // style: Variant
    (@acc [$id:expr] [$label:expr] [style: $_s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $sk:expr]
     style: $v:ident $(, $($rest:tt)*)?) => {
        $crate::button!(@acc
            [$id]
            [$label]
            [style: $crate::ButtonStyle::$v]
            [size: $sz]
            [icon: $i]
            [disabled: $d]
            [skin: $sk]
            $($($rest)*)?
        )
    };

    // size: Variant
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $_sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $sk:expr]
     size: $v:ident $(, $($rest:tt)*)?) => {
        $crate::button!(@acc
            [$id]
            [$label]
            [style: $s]
            [size: $crate::ButtonSize::$v]
            [icon: $i]
            [disabled: $d]
            [skin: $sk]
            $($($rest)*)?
        )
    };

    // icon: expr
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $_i:expr] [disabled: $d:expr] [skin: $sk:expr]
     icon: $v:expr $(, $($rest:tt)*)?) => {
        $crate::button!(@acc
            [$id]
            [$label]
            [style: $s]
            [size: $sz]
            [icon: $v]
            [disabled: $d]
            [skin: $sk]
            $($($rest)*)?
        )
    };

    // disabled: expr
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $_d:expr] [skin: $sk:expr]
     disabled: $v:expr $(, $($rest:tt)*)?) => {
        $crate::button!(@acc
            [$id]
            [$label]
            [style: $s]
            [size: $sz]
            [icon: $i]
            [disabled: $v]
            [skin: $sk]
            $($($rest)*)?
        )
    };

    // skin: expr
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $_sk:expr]
     skin: $v:expr $(, $($rest:tt)*)?) => {
        $crate::button!(@acc
            [$id]
            [$label]
            [style: $s]
            [size: $sz]
            [icon: $i]
            [disabled: $d]
            [skin: $v]
            $($($rest)*)?
        )
    };
}

/// Progress bar node with keyword-style options and sensible defaults.
///
/// The first positional argument is the progress mode (required).
///
/// # Examples
/// ```ignore
/// progress_bar!(ProgressMode::Slider(0.5))
/// progress_bar!(ProgressMode::Indeterminate, active: true)
/// progress_bar!(ProgressMode::Slider(vol), touch_key: "volume", skin: slider_skin)
/// ```
#[macro_export]
macro_rules! progress_bar {
    // Entry point:
    ($mode:expr $(, $($rest:tt)*)?) => {
        $crate::progress_bar!(@acc
            [$mode]
            [touch_key: ::core::option::Option::None]
            [track_h: 2.0]
            [active: false]
            [fill_color: $crate::WHITE]
            [track_color: $crate::GRAY_60.with_alpha(0.44)]
            [bg_color: $crate::TRANSPARENT]
            [skin: None]
            $($($rest)*)?
        )
    };

    // Terminal — all fields consumed, build the node.
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr] $(,)?) => {
        $crate::progress_bar($tk, $th, $mode, $a, $fc, $tc, $bg, $sk)
    };

    // touch_key: expr
    (@acc [$mode:expr] [touch_key: $_tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     touch_key: $v:expr $(, $($rest:tt)*)?) => {
        $crate::progress_bar!(@acc [$mode] [touch_key: ::core::option::Option::Some($v)] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // track_h: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $_th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     track_h: $v:expr $(, $($rest:tt)*)?) => {
        $crate::progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $v] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // active: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $_a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     active: $v:expr $(, $($rest:tt)*)?) => {
        $crate::progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $v]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // fill_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $_fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     fill_color: $v:expr $(, $($rest:tt)*)?) => {
        $crate::progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $v] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // track_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $_tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     track_color: $v:expr $(, $($rest:tt)*)?) => {
        $crate::progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $v] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // bg_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $_bg:expr] [skin: $sk:expr]
     bg_color: $v:expr $(, $($rest:tt)*)?) => {
        $crate::progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $v] [skin: $sk] $($($rest)*)?)
    };

    // skin: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $_sk:expr]
     skin: $v:expr $(, $($rest:tt)*)?) => {
        $crate::progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $v] $($($rest)*)?)
    };
}

/// Number input — key + value are required, rest via `NumberInputProps` struct.
///
/// # Examples
/// ```ignore
/// number_input!("work", 25, label: "Work", suffix: "min", min: 1, max: 60)
/// number_input!("count", n)  // minimal, all defaults
/// ```
#[macro_export]
macro_rules! number_input {
    ($key:expr, $value:expr $(, $($field:ident: $val:expr),* $(,)?)?) => {{
        #[allow(unused_mut)]
        let mut p = $crate::components::number_input::NumberInputProps::default();
        $($(p.$field = $val;)*)?
        $crate::components::number_input::number_input($key, ::core::convert::Into::into($value), &p)
    }};
}

/// Unified style macro for text styling and layout.
///
/// Text style fields:
///  - size
///  - weight
///  - italic
///  - underline
///  - strikethrough
///  - line_height
///  - align
///  - max_width
///  - text_overflow: `TextOverflow::Clip` or `TextOverflow::Ellipsis`
///    cut a line instead of wrapping it; see [`TextOverflow`](crate::TextOverflow)
///  - outline_color, outline_width
///
/// Layout fields: padding, margin, gap, flex, width, height, background
/// Shared field: color (applies to both)
///
/// # Examples
/// ```ignore
/// text("Hello", style!(size: 24, color: WHITE, padding: 8.0))
/// paragraph(style!(size: 16, line_height: 1.3), [
///     span("Click ", ()),
///     span("Save", style!(weight: FontWeight::BOLD)),
///     span(" to confirm.", ()),
/// ])
/// ```
#[macro_export]
macro_rules! style {
    () => {
        $crate::StyleResult(
            $crate::TextStyle::default(),
            $crate::PropsData::default(),
            ::core::option::Option::None,
        )
    };
    ($($field:ident: $value:expr),* $(,)?) => {{
        let mut ts = $crate::TextStyle::default();
        let mut p = $crate::PropsData::default();
        // A span has to tell a size it was given from one it inherits, and the
        // style cannot say which — so the `size:` arm hands the value out here.
        // Threaded through shadowed bindings rather than assigned: an arm that
        // never runs leaves nothing for `unused_assignments` to flag.
        let span_size: ::core::option::Option<u32> = ::core::option::Option::None;
        $(let span_size = $crate::style!(@route ts, p, span_size, $field: $value);)*
        // `unused_mut` fires on whichever of the two a call leaves alone,
        // and `#[expect]` would fire on the one it touches — neither attribute
        // survives every expansion, so the borrow settles the lint instead.
        let _ = (&mut ts, &mut p);
        $crate::StyleResult(ts, p, span_size)
    }};
    // Every arm yields the running span size, so the `size:` arm can replace it
    // and every other arm passes it along. Each reads it, the replacing arm
    // included, so no field order leaves a binding for `unused_variables`.
    (@route $ts:expr, $p:expr, $s:expr, size: $v:expr) => {{
        let _superseded: ::core::option::Option<u32> = $s;
        let size = $v;
        $ts.size = size;
        ::core::option::Option::Some(size)
    }};
    (@route $ts:expr, $p:expr, $s:expr, weight: $v:expr) => {{ $ts.weight = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, family: $v:expr) => {{ $ts.family = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, italic: $v:expr) => {{ $ts.italic = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, underline: $v:expr) => {{ $ts.underline = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, strikethrough: $v:expr) => {{ $ts.strikethrough = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, line_height: $v:expr) => {{ $ts.line_height = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, align: $v:expr) => {{ $ts.align = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, valign: $v:expr) => {{ $ts.vertical_align = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, text_overflow: $v:expr) => {{ $ts.text_overflow = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, max_width: $v:expr) => {{ $ts.max_width = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, outline_color: $v:expr) => {{ $ts.outline_color = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, outline_width: $v:expr) => {{ $ts.outline_width = $v; $s }};
    // Layout fields (use coercion so integer literals work for f32 fields)
    (@route $ts:expr, $p:expr, $s:expr, padding: $v:expr) => {{ $p.padding = $crate::PropsFieldValue::into_field($v); $s }};
    (@route $ts:expr, $p:expr, $s:expr, margin: $v:expr) => {{ $p.margin = $crate::PropsFieldValue::into_field($v); $s }};
    (@route $ts:expr, $p:expr, $s:expr, gap: $v:expr) => {{ $p.gap = $crate::PropsFieldValue::into_field($v); $s }};
    (@route $ts:expr, $p:expr, $s:expr, flex: $v:expr) => {{ $p.flex = $crate::PropsFieldValue::into_field($v); $s }};
    (@route $ts:expr, $p:expr, $s:expr, width: $v:expr) => {{ $p.width = $crate::PropsFieldValue::into_field($v); $s }};
    (@route $ts:expr, $p:expr, $s:expr, height: $v:expr) => {{ $p.height = $crate::PropsFieldValue::into_field($v); $s }};
    (@route $ts:expr, $p:expr, $s:expr, background: $v:expr) => {{ $p.background = $v; $s }};
    (@route $ts:expr, $p:expr, $s:expr, color: $v:expr) => {{ $ts.color = $v; $s }};
}

/// Ergonomic path construction for canvas draw commands.
///
/// # Stroked paths (polylines)
/// ```ignore
/// path!(points, stroke: 4.0, color: WHITE)                // open, linear
/// path!(points, stroke: 4.0, color: BLUE_50, smooth)      // open, Catmull-Rom
/// path!(points, stroke: 2.0, color: WHITE, closed)        // closed outline, linear
/// path!(points, stroke: 2.0, color: WHITE, closed, smooth) // closed outline, smooth
/// ```
///
/// # Filled paths (polygons)
/// Use the [`crate::fill!`] macro for filled polygons.
#[macro_export]
macro_rules! path {
    ($pts:expr, stroke: $w:expr, color: $c:expr) => {
        $crate::Draw::path($pts, $w, $c, false, $crate::Interpolation::Linear)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, smooth) => {
        $crate::Draw::path($pts, $w, $c, false, $crate::Interpolation::CatmullRom)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, closed) => {
        $crate::Draw::path($pts, $w, $c, true, $crate::Interpolation::Linear)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, closed, smooth) => {
        $crate::Draw::path($pts, $w, $c, true, $crate::Interpolation::CatmullRom)
    };
    ($pts:expr, stroke: $w:expr, color: $c:expr, dashed: ($on:expr, $off:expr)) => {
        $crate::Draw::dashed_path($pts, $w, $c, $on, $off)
    };
}

/// Build a filled polygon ([`Draw::fill_path`](crate::Draw::fill_path)) with a solid colour or gradient.
///
/// ```ignore
/// fill!(pts, color: c)
/// fill!(pts, color: c, smooth)
/// fill!(pts, linear: (top, bottom))
/// fill!(pts, linear: (a, b), angle: 90.0)
/// fill!(pts, radial: (inner, outer))
/// ```
#[macro_export]
macro_rules! fill {
    ($pts:expr, color: $c:expr) => {
        $crate::Draw::fill_path($pts, $c, false)
    };
    ($pts:expr, color: $c:expr, smooth) => {
        $crate::Draw::fill_path($pts, $c, true)
    };
    ($pts:expr, linear: ($a:expr, $b:expr)) => {
        $crate::Draw::fill_path($pts, $crate::Fill::linear(0.0, $a, $b), false)
    };
    ($pts:expr, linear: ($a:expr, $b:expr), smooth) => {
        $crate::Draw::fill_path($pts, $crate::Fill::linear(0.0, $a, $b), true)
    };
    ($pts:expr, linear: ($a:expr, $b:expr), angle: $ang:expr) => {
        $crate::Draw::fill_path($pts, $crate::Fill::linear($ang, $a, $b), false)
    };
    ($pts:expr, linear: ($a:expr, $b:expr), angle: $ang:expr, smooth) => {
        $crate::Draw::fill_path($pts, $crate::Fill::linear($ang, $a, $b), true)
    };
    ($pts:expr, radial: ($inner:expr, $outer:expr)) => {
        $crate::Draw::fill_path($pts, $crate::Fill::radial($inner, $outer), false)
    };
    ($pts:expr, radial: ($inner:expr, $outer:expr), smooth) => {
        $crate::Draw::fill_path($pts, $crate::Fill::radial($inner, $outer), true)
    };
}

#[cfg(test)]
mod tests {
    use crate::colors::Color;
    use crate::fill::{Dash, Fill, PathPaint};
    use crate::tree::Draw;

    #[test]
    fn fill_macro_builds_a_filled_path() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)];
        let red = Color::from_rgb(0xFF, 0, 0);
        let draw = crate::fill!(pts, linear: (red, red), angle: 90.0);
        let Draw::Path { paint, .. } = draw else {
            panic!("fill! must build a Draw::Path");
        };
        assert_eq!(paint, PathPaint::Fill(Fill::linear(90.0, red, red)));
    }

    #[test]
    fn path_macro_stroke_stays_solid() {
        let pts = vec![(0.0, 0.0), (10.0, 10.0)];
        let red = Color::from_rgb(0xFF, 0, 0);
        let draw = crate::path!(pts, stroke: 2.0, color: red);
        let Draw::Path { paint, .. } = draw else {
            panic!("path! must build a Draw::Path");
        };
        assert_eq!(
            paint,
            PathPaint::Stroke {
                color: red,
                width: 2.0,
                dash: None,
            }
        );
    }

    #[test]
    fn path_macro_dashed_carries_the_pattern() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0)];
        let red = Color::from_rgb(0xFF, 0, 0);
        let draw = crate::path!(pts, stroke: 1.0, color: red, dashed: (6.0, 4.0));
        let Draw::Path { paint, .. } = draw else {
            panic!("path! must build a Draw::Path");
        };
        assert_eq!(
            paint,
            PathPaint::Stroke {
                color: red,
                width: 1.0,
                dash: Some(Dash { on: 6.0, off: 4.0 }),
            }
        );
    }
}
