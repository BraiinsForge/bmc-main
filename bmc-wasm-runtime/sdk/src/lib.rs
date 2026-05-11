// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM Widget SDK for Braiins Deck.
//!
//! Provides host bindings and UI primitives for building widgets.
//! Layout is computed on the host side for minimal WASM binary size.
//!
//! When compiled for native targets (non-wasm32), FFI-dependent modules
//! are gated out. The tree-building API (`col`, `row`, `text`, `button!`,
//! `props!`, `style!`) and pure types remain available for the storybook
//! and other native consumers.

// wasm32: usize == u32, so these truncation warnings are false positives.
// cast_sign_loss only fires on wasm32 (gated FFI code).
#![expect(clippy::cast_possible_truncation, clippy::cast_lossless)]
#![cfg_attr(target_arch = "wasm32", expect(clippy::cast_sign_loss))]

// Embed protocol version as a WASM export.
// The host calls this after instantiation to verify compatibility.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn __bmc_sdk_version() -> u64 {
    version_pack(SDK_VERSION)
}

// -- WASM-only modules (require host FFI) --
#[cfg(target_arch = "wasm32")]
pub mod alloc;
pub mod assets;
#[cfg(target_arch = "wasm32")]
pub mod calendar;
#[cfg(target_arch = "wasm32")]
pub mod format;
pub mod host;
#[cfg(target_arch = "wasm32")]
pub mod http_listener;
#[cfg(target_arch = "wasm32")]
pub mod json;
pub mod json_str;
#[cfg(target_arch = "wasm32")]
pub mod kv;
#[cfg(target_arch = "wasm32")]
pub mod led;
#[cfg(target_arch = "wasm32")]
pub mod log;
#[cfg(feature = "math-3d")]
pub mod math;
#[cfg(target_arch = "wasm32")]
pub mod mdns;
pub mod mesh;
pub mod modal;
#[cfg(target_arch = "wasm32")]
pub mod net;
pub mod notification;
pub mod number_input;
pub mod orientation;
pub mod progress_bar;
#[cfg(target_arch = "wasm32")]
pub mod slot;
#[cfg(target_arch = "wasm32")]
pub mod socket;
#[cfg(target_arch = "wasm32")]
pub mod ssdp;
pub mod text;
pub mod tree;
#[cfg(target_arch = "wasm32")]
pub mod udp_broadcast;
#[cfg(target_arch = "wasm32")]
pub mod ws;
#[cfg(target_arch = "wasm32")]
pub mod xml;

pub use bmc_render_macros::*;
pub use bmc_wasm_protocol::*;
pub use bmc_wasm_sdk_macros::*;
#[cfg(target_arch = "wasm32")]
pub use format::{format_date, format_duration};
pub use host::*;
#[cfg(target_arch = "wasm32")]
pub use json::JsonDoc;
pub use json_str::JsonStr;
#[cfg(target_arch = "wasm32")]
pub use led::LedEffect;
pub use mesh::*;
#[cfg(target_arch = "wasm32")]
pub use net::*;
pub use number_input::*;
pub use orientation::Orientation;
pub use tree::*;
pub use ufmt;
#[cfg(target_arch = "wasm32")]
pub use ws::{Ws, WsEvent, ws_connect};
#[cfg(target_arch = "wasm32")]
pub use xml::XmlDoc;

/// Helper for `button!` macro — converts label to String.
#[doc(hidden)]
pub fn __macro_string_from(s: impl Into<String>) -> String {
    s.into()
}

// ── Props field coercion ─────────────────────────────────────────────

/// Trait that allows integer literals to be used for `f32` fields in `props!`.
///
/// Identity impl covers all types (f32→f32, bool→bool, Color→Color, etc.).
/// Additional impls convert integer types to f32 so you can write
/// `props!(gap: 12, padding: 16)` instead of `props!(gap: 12.0, padding: 16.0)`.
#[doc(hidden)]
pub trait PropsFieldValue<T> {
    fn into_field(self) -> T;
}

impl<T> PropsFieldValue<T> for T {
    fn into_field(self) -> T {
        self
    }
}

macro_rules! impl_int_to_f32_lossless {
    ($($t:ty),*) => {
        $(impl PropsFieldValue<f32> for $t {
            fn into_field(self) -> f32 { f32::from(self) }
        })*
    };
}

macro_rules! impl_int_to_f32_lossy {
    ($($t:ty),*) => {
        $(impl PropsFieldValue<f32> for $t {
            #[expect(clippy::cast_precision_loss)]
            fn into_field(self) -> f32 { self as f32 }
        })*
    };
}

impl_int_to_f32_lossless!(i8, i16, u8, u16);
impl_int_to_f32_lossy!(i32, i64, u32, u64, isize, usize);

/// Shorthand for PropsData: `props!()` or `props!(gap: 16, background: 0xFF)`
///
/// Integer literals are automatically converted to `f32` for layout fields
/// (padding, margin, gap, width, height, etc.).
///
/// Supports a special `bg_nine_patch: <NinePatch>` field that expands a
/// `NinePatch` into the underlying `bg_np_*` fields on `PropsData`.
#[macro_export]
macro_rules! props {
    () => { $crate::tree::PropsData::default() };
    ($($field:ident: $value:expr),* $(,)?) => {{
        let mut p = $crate::tree::PropsData::default();
        $(props!(@set p, $field: $value);)*
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
/// Style and size accept bare variant names resolved through the SDK.
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
        button!(@acc
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
    // `icon:` accepts either `IconId` or `Option<IconId>` (std provides
    // `From<T> for Option<T>`, so `.into()` covers both).
    (@acc [$id:expr] [$label:expr] [style: $s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $sk:expr] $(,)?) => {
        $crate::make_button($crate::__macro_string_from($id), $crate::__macro_string_from($label), $s, $sz, $i.into(), $d, $sk)
    };

    // style: Variant
    (@acc [$id:expr] [$label:expr] [style: $_s:expr] [size: $sz:expr] [icon: $i:expr] [disabled: $d:expr] [skin: $sk:expr]
     style: $v:ident $(, $($rest:tt)*)?) => {
        button!(@acc
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
        button!(@acc
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
        button!(@acc
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
        button!(@acc
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
        button!(@acc
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
/// progress_bar!(ProgressMode::Fraction(0.5))
/// progress_bar!(ProgressMode::Indeterminate, active: true)
/// progress_bar!(ProgressMode::Fraction(vol), touch_key: "volume", skin: slider_skin)
/// ```
#[macro_export]
macro_rules! progress_bar {
    // Entry point:
    ($mode:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc
            [$mode]
            [touch_key: ""]
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
        progress_bar!(@acc [$mode] [touch_key: $v] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // track_h: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $_th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     track_h: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $v] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // active: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $_a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     active: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $v]
            [fill_color: $fc] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // fill_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $_fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     fill_color: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $v] [track_color: $tc] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // track_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $_tc:expr] [bg_color: $bg:expr] [skin: $sk:expr]
     track_color: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $v] [bg_color: $bg] [skin: $sk] $($($rest)*)?)
    };

    // bg_color: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $_bg:expr] [skin: $sk:expr]
     bg_color: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
            [fill_color: $fc] [track_color: $tc] [bg_color: $v] [skin: $sk] $($($rest)*)?)
    };

    // skin: expr
    (@acc [$mode:expr] [touch_key: $tk:expr] [track_h: $th:expr] [active: $a:expr]
     [fill_color: $fc:expr] [track_color: $tc:expr] [bg_color: $bg:expr] [skin: $_sk:expr]
     skin: $v:expr $(, $($rest:tt)*)?) => {
        progress_bar!(@acc [$mode] [touch_key: $tk] [track_h: $th] [active: $a]
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
        let mut p = $crate::number_input::NumberInputProps::default();
        $($(p.$field = $val;)*)?
        $crate::number_input::number_input($key, ::core::convert::Into::into($value), &p)
    }};
}

/// Connect to a WebSocket with optional headers.
///
/// # Examples
/// ```ignore
/// ws!("ws://host/api", on_event)
/// ws!("ws://host/api", on_event, headers: [
///     ("Authorization", "Bearer xyz"),
///     ("X-Custom", "value"),
/// ])
/// ```
#[macro_export]
macro_rules! ws {
    ($url:expr, $callback:expr $(,)?) => {
        $crate::ws::ws_connect($url, None, $callback)
    };
    ($url:expr, $callback:expr, headers: [$(($k:expr, $v:expr)),+ $(,)?] $(,)?) => {{
        let joined = [$( concat!($k, ": ", $v) ),+].join("\n");
        $crate::ws::ws_connect($url, Some(&joined), $callback)
    }};
}

/// Lightweight string interpolation without pulling in `core::fmt`.
/// Drop-in replacement for `format!()` in widget code.
///
/// Supports captured variable syntax like `std::format!`:
/// - `fmt!("{year}-{month}")` desugars to `ufmt::uwrite!(s, "{}-{}", year, month)`
/// - `fmt!("{val:x}")` desugars to `ufmt::uwrite!(s, "{:x}", val)`
/// - `fmt!("{}-{}", a, b)` still works (positional args pass through)
/// - Mixed: `fmt!("{year}-{}", month)` — captured args append after positional ones
#[macro_export]
macro_rules! fmt {
    ($($arg:tt)*) => {
        $crate::fmt_impl!(@ufmt_path = $crate::ufmt; $($arg)*)
    };
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
///  - text_overflow (`TextOverflow::Clip`, `TextOverflow::Ellipsis`)
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
///     span("Save", style!(weight: 700)),
///     span(" to confirm.", ()),
/// ])
/// ```
#[macro_export]
macro_rules! style {
    () => {
        $crate::tree::StyleResult($crate::tree::TextStyle::default(), $crate::tree::PropsData::default())
    };
    ($($field:ident: $value:expr),* $(,)?) => {{
        let mut ts = $crate::tree::TextStyle::default();
        let mut p = $crate::tree::PropsData::default();
        $(style!(@route ts, p, $field: $value);)*
        $crate::tree::StyleResult(ts, p)
    }};
    // Text style fields
    (@route $ts:expr, $p:expr, size: $v:expr) => { $ts.size = $v; };
    (@route $ts:expr, $p:expr, weight: $v:expr) => { $ts.weight = $v; };
    (@route $ts:expr, $p:expr, italic: $v:expr) => { $ts.italic = $v; };
    (@route $ts:expr, $p:expr, underline: $v:expr) => { $ts.underline = $v; };
    (@route $ts:expr, $p:expr, strikethrough: $v:expr) => { $ts.strikethrough = $v; };
    (@route $ts:expr, $p:expr, line_height: $v:expr) => { $ts.line_height = $v; };
    (@route $ts:expr, $p:expr, align: $v:expr) => { $ts.align = $v; };
    (@route $ts:expr, $p:expr, text_overflow: $v:expr) => { $ts.text_overflow = $v; };
    (@route $ts:expr, $p:expr, max_width: $v:expr) => { $ts.max_width = $v; };
    (@route $ts:expr, $p:expr, outline_color: $v:expr) => { $ts.outline_color = $v; };
    (@route $ts:expr, $p:expr, outline_width: $v:expr) => { $ts.outline_width = $v; };
    // Layout fields (use coercion so integer literals work for f32 fields)
    (@route $ts:expr, $p:expr, padding: $v:expr) => { $p.padding = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, margin: $v:expr) => { $p.margin = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, gap: $v:expr) => { $p.gap = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, flex: $v:expr) => { $p.flex = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, width: $v:expr) => { $p.width = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, height: $v:expr) => { $p.height = $crate::PropsFieldValue::into_field($v); };
    (@route $ts:expr, $p:expr, background: $v:expr) => { $p.background = $v; };
    (@route $ts:expr, $p:expr, color: $v:expr) => { $ts.color = $v; };
}
