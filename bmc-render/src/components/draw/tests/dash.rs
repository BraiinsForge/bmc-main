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

use super::*;

#[test]
fn dash_runs_splits_a_horizontal_line_into_even_dashes() {
    // 100px line, 10 on / 10 off → dashes at 0-10, 20-30, 40-50, 60-70, 80-90.
    let runs = dash_runs(&[(0.0, 0.0), (100.0, 0.0)], 10.0, 10.0);
    assert_eq!(runs.len(), 5);
    assert_eq!(runs[0], vec![(0.0, 0.0), (10.0, 0.0)]);
    assert_eq!(runs[1], vec![(20.0, 0.0), (30.0, 0.0)]);
    assert_eq!(runs[4], vec![(80.0, 0.0), (90.0, 0.0)]);
}

#[test]
fn dash_runs_returns_a_solid_run_for_a_degenerate_pattern() {
    let pts = [(0.0, 0.0), (10.0, 5.0)];
    assert_eq!(dash_runs(&pts, 0.0, 4.0), vec![pts.to_vec()]);
    // A negative gap once walked the arc backwards forever; non-finite
    // params slip past the sum guard too. Both must fold to one solid run.
    assert_eq!(dash_runs(&pts, 10.0, -5.0), vec![pts.to_vec()]);
    assert_eq!(dash_runs(&pts, f32::NAN, 4.0), vec![pts.to_vec()]);
    assert_eq!(dash_runs(&pts, 10.0, f32::INFINITY), vec![pts.to_vec()]);
    // A zero gap left the walk budget at 0, so it advanced by nothing
    // for as long as the segment lasted; it reads as solid anyway.
    assert_eq!(dash_runs(&pts, 5.0, 0.0), vec![pts.to_vec()]);
    // Sub-pixel runs stop moving an f32 walk long before they draw.
    assert_eq!(dash_runs(&pts, 1e-9, 1e-9), vec![pts.to_vec()]);
}

#[test]
fn dash_runs_draws_a_path_too_long_to_dash_as_one_solid_run() {
    // Points are guest data. The walk cannot spin here, but at an 8 px period
    // it would take millions of steps to reach the magnitude where the step
    // stops registering — bounded, and a stall all the same.
    let pts = [(0.0, 0.0), (1e30, 0.0)];
    assert_eq!(dash_runs(&pts, 4.0, 4.0), vec![pts.to_vec()]);
}

#[test]
fn dash_runs_draws_an_unmeasurable_path_as_one_solid_run() {
    // Compared by shape, not value: a run carrying NaN is never `==` itself.
    for far in [f32::NAN, f32::INFINITY] {
        let pts = [(0.0, 0.0), (far, 0.0)];
        let runs = dash_runs(&pts, 4.0, 4.0);
        assert_eq!(runs.len(), 1, "x = {far}: expected one solid run");
        assert_eq!(runs[0].len(), pts.len(), "x = {far}: run kept every point");
    }
}

#[test]
fn dash_runs_flips_where_a_band_boundary_lands_on_a_vertex() {
    // Integer points with an integer dash put the band's end exactly on the
    // vertex at x=2. The flip has to be emitted there, or `budget` leaves
    // the segment at zero and every later segment inherits the wedge.
    let pts = [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (4.0, 0.0)];
    assert_eq!(
        dash_runs(&pts, 2.0, 2.0),
        vec![vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]],
        "dash 0–2 then a gap to the end"
    );
}

#[test]
fn dash_runs_keeps_painting_past_a_vertex_hit_in_the_gap() {
    // The same wedge, landing while the pattern is off.
    // Everything past it went undrawn, `painting` never flipping back.
    let pts = [(0.0, 0.0), (3.0, 0.0), (4.0, 0.0), (8.0, 0.0)];
    assert_eq!(
        dash_runs(&pts, 2.0, 2.0),
        vec![vec![(0.0, 0.0), (2.0, 0.0)], vec![(4.0, 0.0), (6.0, 0.0)],],
        "dash 0–2, gap 2–4, dash 4–6, gap 6–8"
    );
}

#[test]
fn dash_runs_spans_a_corner() {
    // Two 10px legs (horizontal then vertical),
    // dash 15 on / 5 off: the first dash must cross
    // the corner, carrying into the second leg.
    let runs = dash_runs(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 15.0, 5.0);
    assert_eq!(runs[0], vec![(0.0, 0.0), (10.0, 0.0), (10.0, 5.0)]);
}
