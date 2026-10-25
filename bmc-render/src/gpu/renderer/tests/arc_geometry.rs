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

use super::brightness::{W, render};
use super::{ARC_CHUNK_MAX, arc_color_at, arc_point, arc_spans, chunk_span, sweep_fraction};
use crate::renderer::prelude::*;
use bmc_ui::colors::WHITE;
use bmc_ui::{ArcCap, ArcFill, ArcSegments, Color};

fn approx(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "{a} != {b}");
}

#[test]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the probes sit on a ring inside the frame"
)]
fn a_solid_sweep_shows_no_seam_where_its_chunks_used_to_meet() {
    const CENTER: f32 = 32.0;
    const RADIUS: f32 = 20.0;
    let px = render(|r| {
        r.stroke_arc(
            CENTER,
            CENTER,
            RADIUS,
            0.0,
            3.5 * ARC_CHUNK_MAX,
            8.0,
            &ArcFill::Solid(WHITE),
            &ArcSegments::Continuous,
            ArcCap::Butt,
        );
    });
    for boundary in [1.0, 2.0, 3.0] {
        let (x, y) = arc_point(CENTER, CENTER, RADIUS, boundary * ARC_CHUNK_MAX);
        let pixel = px[y as usize * W as usize + x as usize];
        assert_eq!(
            pixel[..3],
            [255, 255, 255],
            "fully lit at chunk boundary {boundary}, got {pixel:?}"
        );
    }
}

#[test]
fn continuous_resolves_to_full_sweep() {
    assert_eq!(
        arc_spans(&ArcSegments::Continuous, 0.2, 1.4),
        vec![(0.2, 1.4)]
    );
}

#[test]
fn explicit_within_sweep_passes_unchanged() {
    let spans = vec![(0.0, 0.5), (0.6, 1.0)];
    assert_eq!(
        arc_spans(&ArcSegments::Explicit(spans.clone()), 0.0, 1.0),
        spans
    );
}

#[test]
fn explicit_clips_to_sweep_dropping_and_truncating() {
    // Sweep ends mid-second-segment: the first survives whole, the second is
    // truncated at the sweep end, anything past it is dropped.
    let spans = vec![(0.0, 0.5), (0.6, 1.0), (1.2, 1.5)];
    assert_eq!(
        arc_spans(&ArcSegments::Explicit(spans), 0.0, 0.8),
        vec![(0.0, 0.5), (0.6, 0.8)]
    );
}

#[test]
fn explicit_zero_sweep_clips_to_nothing() {
    let spans = vec![(0.0, 0.5), (0.6, 1.0)];
    assert!(arc_spans(&ArcSegments::Explicit(spans), 0.0, 0.0).is_empty());
}

#[test]
fn chunking_subdivides_and_lands_on_end() {
    let chunks = chunk_span(0.0, 1.0, 0.2);
    assert_eq!(chunks.len(), 5);
    approx(chunks[0].0, 0.0);
    approx(
        chunks
            .last()
            .expect("BUG: chunk_span must return at least one chunk")
            .1,
        1.0,
    );
    for w in chunks.windows(2) {
        approx(w[0].1, w[1].0);
    }
}

#[test]
fn tiny_span_yields_one_chunk() {
    assert_eq!(chunk_span(0.0, 0.05, 0.2).len(), 1);
}

#[test]
fn mining_clock_tick_span_fits_in_one_arc_chunk() {
    let mining_tick_slot = std::f32::consts::TAU / 28.0;
    let mining_tick_span = mining_tick_slot * 0.96;

    assert_eq!(chunk_span(0.0, mining_tick_span, ARC_CHUNK_MAX).len(), 1);
}

#[test]
fn sweep_fraction_is_independent_of_gaps() {
    approx(sweep_fraction(0.0, 0.0, 1.0), 0.0);
    approx(sweep_fraction(1.0, 0.0, 1.0), 1.0);
    approx(sweep_fraction(0.5, 0.0, 1.0), 0.5);
    approx(sweep_fraction(0.75, 0.0, 1.0), 0.75);
}

#[test]
fn color_at_hits_endpoints() {
    let red = Color::from_rgb(0xFF, 0x00, 0x00);
    let blue = Color::from_rgb(0x00, 0x00, 0xFF);
    let g = ArcFill::gradient(red, blue);
    assert_eq!(arc_color_at(&g, 0.0), red);
    assert_eq!(arc_color_at(&g, 1.0), blue);
    assert_eq!(arc_color_at(&ArcFill::Solid(red), 0.5), red);
}

#[test]
fn arc_point_uses_twelve_oclock_zero() {
    let center = (10.0, 20.0);
    let radius = 5.0;
    let top = arc_point(center.0, center.1, radius, 0.0);
    approx(top.0, 10.0);
    approx(top.1, 15.0);

    let right = arc_point(center.0, center.1, radius, std::f32::consts::FRAC_PI_2);
    approx(right.0, 15.0);
    approx(right.1, 20.0);
}
