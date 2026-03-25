// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Canvas/Color Spaces" }

#[story(default)]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn gradients(c: &mut StoryCtx) {
    c.ui.header(
        "Color Interpolation",
        "Red to Teal gradient in different color spaces",
    );

    let from = RED_50;
    let to = TEAL_50;
    let steps: usize = 64;
    let w = 400.0;
    let step_w = w / steps as f32;
    let bar_h = 32.0;

    let gradient = |lerp_fn: &dyn Fn(f32) -> Color| -> Vec<Draw> {
        (0..steps)
            .map(|idx| {
                let frac = idx as f32 / (steps - 1) as f32;
                let x0 = (idx as f32 * step_w).floor();
                let x1 = ((idx + 1) as f32 * step_w).ceil();
                Draw::rect(x0, 0.0, x1 - x0, bar_h, lerp_fn(frac))
            })
            .collect()
    };

    let srgb_lerp = |frac: f32| -> Color {
        let lerp =
            |a: u8, b: u8| -> u8 { (f32::from(a) + (f32::from(b) - f32::from(a)) * frac) as u8 };
        Color::from_rgb(
            lerp(from.red(), to.red()),
            lerp(from.green(), to.green()),
            lerp(from.blue(), to.blue()),
        )
    };

    c.ui.header(
        "Oklch (perceptual)",
        "Interpolates in lightness, chroma, hue — consistent perceived brightness",
    );
    c.ui.div(
        (400, 32),
        canvas(
            props!(width: 400, height: 32),
            gradient(&|frac| from.mix(to, frac)),
        ),
    );

    c.ui.header(
        "sRGB (naive component lerp)",
        "Linear R/G/B byte interpolation — muddy desaturated midtones",
    );
    c.ui.div(
        (400, 32),
        canvas(props!(width: 400, height: 32), gradient(&srgb_lerp)),
    );
}
