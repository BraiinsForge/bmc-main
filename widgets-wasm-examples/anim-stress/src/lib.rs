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

//! Dynamic-heavy render benchmark, in two scenes picked by the `scene` param.
//!
//! **Full surface** (the default): every pixel changes every frame.
//! The shipped widgets animate a small share of their surface, which overstates
//! what a renderer optimisation depending on *unchanged* content — static-layer
//! caching, damage scissoring — is worth in general. This scene is the opposite
//! bound: bands tile the surface and every one animates its colour, so the
//! partition classifies all of it dynamic and no damage rectangle is smaller
//! than the whole surface.
//!
//! **Layered**: static content over, and beside, an animated strip;
//! see `layered.rs`.
//!
//! Colours animate **host-side**, so the host keeps producing frames on its own.
//! The `frames` param picks the pipeline those frames take:
//! `guest` re-runs the guest every frame to rebuild the tree,
//! `host` replays the cached tree scissored to damage, with no interpreter time.
//! `BAND_COUNT` is small because the point is area repainted,
//! not draw-call count, which `stress-test` covers.

// Generated from `manifest.json` by `bmc-widget-codegen`; regenerate
// via `just wasm::gen anim-stress` after editing the manifest.
mod manifest_params;

mod layered;

use bmc_wasm_sdk::{
    AnimProperty, Color, Draw, Easing, LoopMode, WidgetSize, canvas, props, render_ui,
    request_frame_after, widget_size,
};

use manifest_params::{Frames, Params, Scene};

/// Bands tiling the surface. Enough that no single one dominates a damage
/// rectangle, few enough that draw-command recording stays negligible against
/// the fill cost being measured.
const BAND_COUNT: usize = 24;

/// One full colour cycle, in milliseconds: slow enough not to strobe, fast
/// enough to have visibly changed within a second of watching.
const CYCLE_MS: u32 = 2_000;

/// Endpoints of each band's colour animation, contrasting enough that a frozen
/// or partially-updated surface is obvious on the device.
const BAND_FROM: Color = Color::from_rgb(20, 20, 40);
const BAND_TO: Color = Color::from_rgb(230, 90, 30);

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        ..
    } = widget_size();

    #[expect(
        clippy::cast_precision_loss,
        reason = "surface dimensions are well under f32 precision"
    )]
    let (fw, fh) = (w as f32, h as f32);

    let params = Params::current();
    let tree = match params.scene {
        Scene::Full => canvas(props!(width: fw, height: fh), bands(fw, fh, BAND_COUNT)),
        Scene::Layered => layered::tree(fw, fh),
    };
    let _ = render_ui(w, h, tree);

    // Forces a guest run on every frame,
    // where the host would otherwise replay the cached tree while its animations run.
    if params.frames == Frames::Guest {
        request_frame_after(1);
    }
}

/// `count` colour-animating bands tiling a `w` by `h` box.
fn bands(w: f32, h: f32, count: usize) -> Vec<Draw> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "band count is small and exact in f32"
    )]
    let band_h = h / count as f32;
    (0..count)
        .map(|i| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "band index is bounded by the band count"
            )]
            let y = i as f32 * band_h;
            // Staggered so the surface never settles on one flat colour, where
            // a stalled frame would be hard to spot by eye.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the product stays below u16::MAX for a 2 s cycle"
            )]
            let delay_ms = (CYCLE_MS as usize * i / count) as u16;
            Draw::rect(0.0, y, w, band_h, BAND_FROM).animate_delayed(
                AnimProperty::Color,
                f32::from_bits(BAND_FROM.to_u32()),
                f32::from_bits(BAND_TO.to_u32()),
                CYCLE_MS,
                delay_ms,
                Easing::EaseInOut,
                LoopMode::PingPong,
            )
        })
        .collect()
}
