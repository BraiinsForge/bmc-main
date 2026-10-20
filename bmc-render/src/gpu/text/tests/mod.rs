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

use super::*;
use super::{autofit::*, glyphs::*, layout_cache::*, shaping::*};
use crate::tree::{FontFamily, FontWeight, SpanData};
use cosmic_text::FontSystem;

mod span_attribution;

mod span_group;

mod layout_cache;

mod line_layout;

mod overflow;

mod autofit;

mod rasterize;

mod glyph_draw;

mod retention;

fn font_system() -> FontSystem {
    crate::gpu::renderer::build_font_system(crate::gpu::renderer::Fonts::shared())
}

fn line_style(size: f32, italic: bool) -> LineStyle {
    LineStyle {
        family: FontFamily::Sans,
        weight: FontWeight::REGULAR,
        italic,
        size,
    }
}

fn sized_span(text: &str, size: Option<u32>) -> SpanData {
    SpanData {
        text: text.to_owned(),
        weight: None,
        color: None,
        size,
        italic: false,
        underline: false,
        strikethrough: false,
    }
}
