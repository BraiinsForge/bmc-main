// Copyright (C) 2026  Braiins Systems s.r.o.

//! WASM Widget SDK for Braiins Deck.
//!
//! Provides host bindings and UI primitives for building widgets.
//! Layout is computed on the host side for minimal WASM binary size.

// wasm32: usize == u32, so these truncation warnings are false positives.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

// Embed protocol version as a WASM export.
// The host calls this after instantiation to verify compatibility.
#[unsafe(no_mangle)]
pub extern "C" fn __bmc_sdk_version() -> u64 {
    version_pack(SDK_VERSION)
}

pub mod alloc;
pub mod format;
pub mod host;
pub mod json;
pub mod net;
pub mod tree;

pub use bmc_wasm_protocol::*;
pub use bmc_wasm_sdk_macros::{include_bitmap, include_icon};
pub use format::format_duration;
pub use host::{
    ButtonStyle, SizeVariant, SystemTime, WidgetSize, draw_text, fill_rect, parse_date,
    request_frame, request_frame_after,
};
pub use json::JsonDoc;
pub use net::{FetchResponse, fetch, fetch_after};
pub use tree::{
    AnimationDef, Bitmap, Draw, Icon, ModalProps, Node, NotificationKind, PropsData, Span,
    StyleResult, TextStyle, TransitionDef, TreeRenderResult, begin_tree, bitmap, button, canvas,
    center, centered, circle, col, icon, icon_builtin, modal, modal_styled, notification, orbit,
    paragraph, rect, render_ui, rotated, row, spacer, span, text, with_buffer,
};
pub use ufmt;

/// Shorthand for PropsData: `props!()` or `props!(gap: 16.0, background: 0xFF)`
#[macro_export]
macro_rules! props {
    () => { $crate::tree::PropsData::default() };
    ($($field:ident: $value:expr),* $(,)?) => {
        $crate::tree::PropsData { $($field: $value),*, ..Default::default() }
    };
}

/// Lightweight string interpolation without pulling in `core::fmt`.
/// Drop-in replacement for `format!()` in widget code.
#[macro_export]
macro_rules! fmt {
    ($($arg:tt)*) => {{
        let mut s = String::new();
        _ = $crate::ufmt::uwrite!(s, $($arg)*);
        s
    }};
}

/// Unified style macro for text styling and layout.
///
/// Text style fields: size, weight, italic, underline, strikethrough, line_height, align, max_width
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
    (@route $ts:expr, $p:expr, max_width: $v:expr) => { $ts.max_width = $v; };
    // Layout fields
    (@route $ts:expr, $p:expr, padding: $v:expr) => { $p.padding = $v; };
    (@route $ts:expr, $p:expr, margin: $v:expr) => { $p.margin = $v; };
    (@route $ts:expr, $p:expr, gap: $v:expr) => { $p.gap = $v; };
    (@route $ts:expr, $p:expr, flex: $v:expr) => { $p.flex = $v; };
    (@route $ts:expr, $p:expr, width: $v:expr) => { $p.width = $v; };
    (@route $ts:expr, $p:expr, height: $v:expr) => { $p.height = $v; };
    (@route $ts:expr, $p:expr, background: $v:expr) => { $p.background = $v; };
    // Shared field (goes to both)
    (@route $ts:expr, $p:expr, color: $v:expr) => { $ts.color = $v; $p.color = $v; };
}
