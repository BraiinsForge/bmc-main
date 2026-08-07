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

//! The Deck widget gallery host. `just gallery::run` opens the window;
//! `just gallery::hot` adds live reload. What it shows is configured in `gallery.toml`.

use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

fn main() -> gallery::eframe::Result {
    // Default to `info` so hot-reload progress is visible; `RUST_LOG` overrides.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();
    gallery::launch!(
        |_| {},
        gallery::Settings::new(gallery::Renderer::Glow)
            .controls_default_width(260.0)
            .collapsed(true)
    )
}
