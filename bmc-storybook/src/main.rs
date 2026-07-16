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
