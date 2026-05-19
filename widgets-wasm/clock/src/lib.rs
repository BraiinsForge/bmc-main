// Copyright (C) 2026  Braiins Systems s.r.o.

//! Clock widget — three render modes (analog round / analog rectangular / digital)
//! and four sizes (Small / Medium / Large / Full).

use bmc_wasm_sdk::{GRAY_10, WidgetSize, center, props, render_ui, style, text, widget_size};

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        ..
    } = widget_size();

    let root = center(
        props!(),
        [text("00:00 AM", style!(size: 200, color: GRAY_10))],
    );

    let _ = render_ui(w, h, root);
}
