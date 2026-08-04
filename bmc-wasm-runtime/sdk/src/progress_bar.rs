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

//! Progress bar / slider builder.

use bmc_render_skin::SliderSkin;
use bmc_wasm_protocol::Color;

use crate::tree::Node;

/// Progress bar mode.
#[derive(Clone, Copy, Debug)]
pub enum ProgressMode {
    /// A draggable slider at a fraction (0.0–1.0): fill plus drag thumb,
    /// the HTML `<input type=range>` of the family.
    Slider(f32),
    /// Unknown duration — animated indicator across full width.
    Indeterminate,
    /// A passive meter at a fraction (0.0–1.0): plain bar,
    /// no drag thumb or squiggle — the HTML `<progress>` of the family.
    Meter(f32),
}

/// Create a progress bar node.
///
/// - `touch_key`: interaction key for slider drag.
/// - `track_h`: track thickness in pixels (also controls squiggle amplitude).
/// - `mode`: see [`ProgressMode`].
/// - `active`: when true, filled portion uses animated squiggle.
/// - `fill_color`: fill, drag thumb, and squiggle color.
/// - `track_color`: background track color.
/// - `bg_color`: used to clip squiggle past the drag thumb. Pass `0` when not active.
#[expect(clippy::too_many_arguments)]
#[must_use]
pub fn progress_bar(
    touch_key: &str,
    track_h: f32,
    mode: ProgressMode,
    active: bool,
    fill_color: Color,
    track_color: Color,
    bg_color: Color,
    skin: Option<SliderSkin>,
) -> Node {
    Node::ProgressBar {
        touch_key: String::from(touch_key),
        track_h,
        mode,
        active,
        fill_color,
        track_color,
        bg_color,
        skin,
    }
}
