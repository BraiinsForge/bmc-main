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

scene_meta! { title: "Components / Feedback / StatusOverlay" }

#[scene(default)]
fn status_overlay(ctx: &mut SceneCtx, ui: &mut Ui) {
    let error = ctx.radio("Variant", &["Stale (age)", "Error (reason)"], 0) == 1;
    let age = ctx.slider("Last refresh age (s)", 120.0, 0.0, 200_000.0, 1.0);
    let reason = match ctx.radio(
        "Error reason",
        &[
            "Image too large",
            "Unsupported image",
            "Failed to load image",
        ],
        0,
    ) {
        1 => "Unsupported image",
        2 => "Failed to load image",
        _ => "Image too large",
    };
    let round = ctx.toggle("Round face", false);

    let secs = age as i64;
    // The gallery clock ticks up from 0, so the anchor sits `secs` in the past.
    let anchor = SystemTime { unix_secs: -secs };

    let (shape, frame, tile_h) = if round {
        (ViewportShape::Round, Round(480), 480.0_f32)
    } else {
        (
            ViewportShape::Rectangular,
            DeckSize::from((480_u32, 240_u32)),
            240.0_f32,
        )
    };
    ctx.node_stage(ui, frame, || {
        let tile = col(
            props!(width: 480, height: tile_h, padding: 24, cross_align: CrossAlign::Center),
            [text("Weather · 21°C", style!(size: 22, color: WHITE))],
        );
        if error {
            with_error_overlay(tile, reason, shape)
        } else {
            with_stale_overlay(tile, anchor, shape)
        }
    });
}
