// Copyright (C) 2026  Braiins Systems s.r.o.

mod format;
mod manifest_params;
mod model;

#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::wildcard_imports,
    reason = "widget render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;
#[cfg(target_arch = "wasm32")]
use manifest_params::Params;

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    request_frame();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let viewport = widget_viewport();
    let params = Params::current();
    let root = center(
        props!(background: BLACK),
        [text(
            params.view.as_manifest_label(),
            style!(size: 32, weight: FontWeight::BOLD, color: WHITE),
        )],
    );
    let _ = render_ui(viewport.width, viewport.height, root);
}
