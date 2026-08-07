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

use bmc_gallery::prelude::*;

const SLIDER_SKIN: Skin = include_skin!("bmc-render/assets/skins/gallery_slider/");

const DRAG_KEY: &str = "progress::drag";
const SKIN_DRAG_KEY: &str = "progress::skin-drag";

scene_meta! { title: "Components / Feedback / ProgressBar" }

#[scene(default)]
fn progress_bar_variants(ctx: &mut SceneCtx, ui: &mut Ui) {
    let fraction = ctx.slider("Progress", 0.6, 0.0, 1.0, 0.01);
    // Whole pixels: the track is a thickness, and the squiggle's amplitude follows it.
    let track_h = ctx.slider("Track height", 4.0, 2.0, 16.0, 1.0);

    let fired = ctx.node_stage_input(ui, Page, || {
        // Registered in here, where the registrars are live: a scene rendered
        // before any stage has drawn has nothing to register against.
        let track = SLIDER_SKIN
            .get_nine_patch("slider_track")
            .expect("BUG: slider skin missing slider_track asset");
        let thumb = SLIDER_SKIN.get_nine_patch("slider_thumb");
        let thumb_pressed = SLIDER_SKIN.get_nine_patch("slider_thumb_pressed");
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
                    ProgressMode::Slider(fraction),
                    touch_key: DRAG_KEY,
                    track_h: track_h,
                ),
                text("Indeterminate", style!(size: 14, color: GRAY_30)),
                progress_bar!(ProgressMode::Indeterminate, active: true, track_h: track_h),
                text(
                    "Meter (no drag thumb, rounded caps)",
                    style!(size: 14, color: GRAY_30),
                ),
                progress_bar!(ProgressMode::Meter(fraction), track_h: track_h),
                text(
                    "Slider with custom skin (track + thumb)",
                    style!(size: 14, color: GRAY_30),
                ),
                progress_bar!(
                    ProgressMode::Slider(fraction),
                    touch_key: SKIN_DRAG_KEY,
                    track_h: f32::from(skinned.track_h),
                    skin: Some(skinned),
                ),
            ],
        )
    });

    // Both sliders fill from the same knob, so either one dragged writes it
    // and the other follows — which is the comparison the pair is here to make.
    for key in [DRAG_KEY, SKIN_DRAG_KEY] {
        if let Some(fraction) = fired.dragged(key) {
            ctx.set_slider("Progress", fraction);
        }
    }
}
