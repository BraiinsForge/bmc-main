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

scene_meta! { title: "Components / Canvas / Color Spaces" }

#[scene(default)]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "stepping 64 bands across a fixed bar, and channel lerps that stay \
              between the two endpoint bytes"
)]
fn gradients(ctx: &mut SceneCtx, ui: &mut Ui) {
    ui.heading("Color Interpolation");
    ui.label("Red to Teal gradient in different color spaces");

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

    ui.heading("Oklch (perceptual)");
    ui.label("Interpolates in lightness, chroma, hue — consistent perceived brightness");
    ctx.node_stage(ui, (400_u32, 32_u32), || {
        canvas(
            props!(width: 400, height: 32),
            gradient(&|frac| from.mix(to, frac)),
        )
    });

    ui.heading("sRGB (naive component lerp)");
    ui.label("Linear R/G/B byte interpolation — muddy desaturated midtones");
    ctx.node_stage(ui, (400_u32, 32_u32), || {
        canvas(props!(width: 400, height: 32), gradient(&srgb_lerp))
    });
}
