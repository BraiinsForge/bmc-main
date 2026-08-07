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

scene_meta! { title: "Components / Canvas / Drop Shadow" }

const W: u32 = 320;
const H: u32 = 140;

/// A pale plate behind the shapes: a dark, semi-transparent composite
/// reads as nothing against the stage's own backdrop.
fn plate() -> Draw {
    Draw::rect(0.0, 0.0, px(W), px(H), GRAY_30)
}

fn shadow(dx: f32, dy: f32, blur: f32, alpha: f32) -> DropShadow {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "an alpha slider bounded to 0..=1, scaled to a byte"
    )]
    let a = (alpha * 255.0) as u8;
    DropShadow {
        dx,
        dy,
        blur,
        color: Color::from_rgba(0, 0, 0, a),
    }
}

/// Shadows on the shapes that carry them, at knob-driven offset and blur.
///
/// The unshadowed square on the right is load-bearing rather than decorative:
/// it is drawn after both shadows, so it appears only if `drop_shadow`
/// hands the frame's target back. Nothing else in the gallery draws one.
#[scene(default)]
fn shapes(ctx: &mut SceneCtx, ui: &mut Ui) {
    let dx = ctx.slider("Offset X", 4.0, -16.0, 16.0, 1.0);
    let dy = ctx.slider("Offset Y", 4.0, -16.0, 16.0, 1.0);
    let blur = ctx.slider("Blur", 6.0, 0.0, 24.0, 1.0);
    let alpha = ctx.slider("Shadow alpha", 0.5, 0.0, 1.0, 0.05);

    ctx.node_stage(ui, (W, H), || {
        canvas(
            props!(width: 320, height: 140),
            [
                plate(),
                Draw::rect(20.0, 30.0, 80.0, 80.0, VIOLET_60)
                    .with_drop_shadow(shadow(dx, dy, blur, alpha)),
                Draw::circle(160.0, 70.0, 40.0, TEAL_50)
                    .with_drop_shadow(shadow(dx, dy, blur, alpha)),
                Draw::rect(220.0, 30.0, 80.0, 80.0, RED_50),
            ],
        )
    });
}
