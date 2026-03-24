// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

const STORYBOOK_SLIDER_SKIN: Skin = include_skin!("bmc-render/assets/skins/storybook_slider/");

story_meta! { title: "ProgressBar" }

#[story(default)]
fn progress_bar_variants(ctx: &mut StoryCtx) -> Node {
    let fraction = ctx.slider("Progress", 0.6, 0.0, 1.0);
    let drag_key = ctx.action("Drag");
    ctx.bind_drag(&drag_key, fraction);

    let skin_drag_key = ctx.action("SkinnedDrag");
    let skin_fraction = ctx.slider("SkinnedProgress", 0.4, 0.0, 1.0);
    ctx.bind_drag(&skin_drag_key, skin_fraction);

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
            text(
                "Determinate (drag to change)",
                style!(size: 14, color: GRAY_30),
            ),
            progress_bar!(ProgressMode::Fraction(fraction.get()), touch_key: &drag_key),
            text("Indeterminate", style!(size: 14, color: GRAY_30)),
            progress_bar!(ProgressMode::Indeterminate, active: true),
            text(
                "Slider with custom skin (track + thumb)",
                style!(size: 14, color: GRAY_30),
            ),
            progress_bar!(
                ProgressMode::Fraction(skin_fraction.get()),
                touch_key: &skin_drag_key,
                track_h: f32::from(skinned.track_h),
                skin: Some(skinned),
            ),
        ],
    )
}
