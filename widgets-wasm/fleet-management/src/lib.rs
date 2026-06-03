// Copyright (C) 2026  Braiins Systems s.r.o.

mod adapter;
mod device;
mod discovery;
mod model;
mod telemetry;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize { width, height, .. } = widget_size();
    let root = col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [text(
                "Searching for miners…",
                style!(size: 28, color: WHITE),
            )],
        )],
    );
    let _ = render_ui(width, height, root);
}
