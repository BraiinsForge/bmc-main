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

use super::geometry::{linear_endpoints, scissor_box};
#[cfg(target_os = "linux")]
use super::render_targets::gl_name;
#[cfg(target_os = "linux")]
use super::text_draw::femtovg_baseline;
use super::*;

mod brightness;
mod fonts;
#[cfg(target_os = "linux")]
mod outline_stroke;
#[cfg(target_os = "linux")]
mod svg_damage;
mod svg_tint;

#[cfg(target_os = "linux")]
mod formats;

#[cfg(target_os = "linux")]
mod layers;

#[cfg(target_os = "linux")]
mod sphere;

#[cfg(target_os = "linux")]
mod vertical_align;

#[cfg(target_os = "linux")]
mod multiline_text;

#[cfg(target_os = "linux")]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel coordinates of a 240×110 target"
)]
mod text_overflow_pixel;

mod scissor;

mod gradient_geometry;

mod arc_geometry;

#[cfg(all(target_os = "linux", feature = "atlas-inspect"))]
mod atlas_containment;

#[cfg(target_os = "linux")]
mod counter_surface;

#[cfg(target_os = "linux")]
mod rotation_continuity;
