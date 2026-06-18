// Copyright (C) 2026  Braiins Systems s.r.o.

//! Widget catalog — visual reference and interactive playground for SDK components.

use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

fn main() -> eframe::Result<()> {
    init_tracing();

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

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(storybook_env_filter())
        .init();
}

fn storybook_env_filter() -> EnvFilter {
    storybook_env_filter_from_env(
        std::env::var(EnvFilter::DEFAULT_ENV)
            .ok()
            .as_deref()
            .unwrap_or_default(),
    )
}

fn storybook_env_filter_from_env(directives: &str) -> EnvFilter {
    // Default to `info` so hot-reload progress is visible; `RUST_LOG` overrides.
    EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .parse_lossy(directives)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storybook_env_filter_defaults_to_info_for_hot_reload_progress() {
        let filter = storybook_env_filter_from_env("");

        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
    }

    #[test]
    fn storybook_env_filter_lets_rust_log_override_default() {
        let filter = storybook_env_filter_from_env("warn");

        assert_eq!(filter.max_level_hint(), Some(LevelFilter::WARN));
    }
}
