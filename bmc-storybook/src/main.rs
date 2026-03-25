// Copyright (C) 2026  Braiins Systems s.r.o.

//! Widget catalog — visual reference and interactive playground for SDK components.

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();

    // Honour DEBUG_LAYOUT=1 env var on startup; the in-app top-bar toggle
    // below can flip it interactively.
    bmc_render::tree::init_debug_flags();

    let hot_reload = std::env::args().any(|a| a == "--hot-reload");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1_600.0, 900.0])
            .with_title("Widget Catalog"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "Widget Catalog",
        options,
        Box::new(move |cc| Ok(Box::new(bmc_storybook::StorybookApp::new(cc, hot_reload)))),
    )
}
