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

use crate::prelude::*;

const STORYBOOK_SLIDER_SKIN: Skin = include_skin!("bmc-render/assets/skins/storybook_slider/");

story_meta! { title: "ProgressBar" }

#[story(default)]
fn progress_bar_variants(ctx: &mut StoryCtx) -> Node {
    let fraction = ctx.slider("Progress", 0.6, 0.0, 1.0);
    let track_h = ctx.slider("Track height", 4.0, 2.0, 16.0);
    let drag_key = ctx.action("Drag");
    ctx.bind_drag(&drag_key, fraction);

    let skin_drag_key = ctx.action("SkinnedDrag");
    ctx.bind_drag(&skin_drag_key, fraction);

    let track = STORYBOOK_SLIDER_SKIN
        .get_nine_patch("slider_track")
        .expect("BUG: storybook slider skin missing slider_track asset");
    let thumb = STORYBOOK_SLIDER_SKIN.get_nine_patch("slider_thumb");
    let thumb_pressed = STORYBOOK_SLIDER_SKIN.get_nine_patch("slider_thumb_pressed");
    let skinned = SliderSkin {
        track: track.nine_patch,
        track_h: track.height,
        thumb_id: thumb.and_then(|t| t.nine_patch.bitmap_id),
        thumb_w: thumb.map_or(0, |t| t.width),
        thumb_h: thumb.map_or(0, |t| t.height),
        thumb_pressed_id: thumb_pressed.and_then(|t| t.nine_patch.bitmap_id),
    };

    col(
        props!(gap: 16, padding: 16, max_width: 400),
        [
            text("Slider (drag to change)", style!(size: 14, color: GRAY_30)),
            progress_bar!(
                ProgressMode::Slider(fraction.get()),
                touch_key: &drag_key,
                track_h: track_h.get(),
            ),
            text("Indeterminate", style!(size: 14, color: GRAY_30)),
            progress_bar!(ProgressMode::Indeterminate, active: true, track_h: track_h.get()),
            text(
                "Meter (no drag thumb, rounded caps)",
                style!(size: 14, color: GRAY_30),
            ),
            progress_bar!(ProgressMode::Meter(fraction.get()), track_h: track_h.get()),
            text(
                "Slider with custom skin (track + thumb)",
                style!(size: 14, color: GRAY_30),
            ),
            progress_bar!(
                ProgressMode::Slider(fraction.get()),
                touch_key: &skin_drag_key,
                track_h: f32::from(skinned.track_h),
                skin: Some(skinned),
            ),
        ],
    )
}
