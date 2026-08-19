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

//! Dynamic-heavy render benchmark: every pixel changes every frame.
//!
//! The shipped widgets all turned out to be poor benchmarks for this — their
//! animated area is either tiny (`hello-widget`'s three shapes, `metronome`'s
//! pendulum) or idle without external data (`mesh-demo`'s dice sits still,
//! `iss-position`'s globe needs a TLE fetch). Measuring against them overstates
//! how much a renderer optimisation that depends on *unchanged* content —
//! static-layer caching, damage scissoring — is worth in the general case.
//!
//! This widget is the opposite bound: bands tile the whole surface and every
//! one animates its colour, so the static/dynamic partition classifies all of
//! it dynamic, the cached static layer holds nothing worth blitting, and no
//! damage rectangle is smaller than the full surface.
//!
//! Colours are animated **host-side** rather than recomputed per frame in the
//! guest, so a frame costs layout plus draw plus GPU with no interpreter time —
//! isolating render cost, which is what the benchmark is for. `BAND_COUNT` is
//! deliberately small for the same reason: the point is area repainted, not
//! draw-call count, which `stress-test`'s draw-spam mode already covers.

use bmc_wasm_sdk::{
    AnimProperty, Color, Draw, Easing, LoopMode, WidgetSize, canvas, props, render_ui,
    request_frame_after, widget_size,
};

/// Bands tiling the surface. Enough that no single one dominates a damage
/// rectangle, few enough that draw-command recording stays negligible against
/// the fill cost being measured.
const BAND_COUNT: usize = 24;

/// One full colour cycle, in milliseconds. Long enough that consecutive frames
/// differ subtly rather than strobing, short enough that the surface has
/// visibly changed within a second of watching it.
const CYCLE_MS: u32 = 2_000;

/// Endpoints of each band's colour animation. Chosen for visible contrast so a
/// frozen or partially-updated surface is obvious on the device — this widget
/// exists to be looked at as well as measured.
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
    #[expect(
        clippy::cast_precision_loss,
        reason = "band count is small and exact in f32"
    )]
    let band_h = fh / BAND_COUNT as f32;

    let draws: Vec<Draw> = (0..BAND_COUNT)
        .map(|i| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "band index is bounded by BAND_COUNT"
            )]
            let y = i as f32 * band_h;
            // Stagger the phase so the surface never settles on one flat colour,
            // which would make a stalled frame hard to spot by eye.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the product stays below u16::MAX for a 2 s cycle"
            )]
            let delay_ms = (CYCLE_MS as usize * i / BAND_COUNT) as u16;
            Draw::rect(0.0, y, fw, band_h, BAND_FROM).animate_delayed(
                AnimProperty::Color,
                f32::from_bits(BAND_FROM.to_u32()),
                f32::from_bits(BAND_TO.to_u32()),
                CYCLE_MS,
                delay_ms,
                Easing::EaseInOut,
                LoopMode::PingPong,
            )
        })
        .collect();

    let _ = render_ui(w, h, canvas(props!(width: fw, height: fh), draws));

    // Host-side animation still needs the host to keep producing frames; ask for
    // the next one as soon as it will take it.
    request_frame_after(1);
}
