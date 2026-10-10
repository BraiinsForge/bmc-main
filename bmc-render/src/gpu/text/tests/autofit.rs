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

#![expect(clippy::cast_precision_loss)]

use super::{DEFAULT_MIN_AUTOFIT, autofit_bounds, search_fit_size};
use bmc_ui::AutoFit;

// ── autofit_bounds ──────────────────────────────────────────────
#[test]
fn shrink_bounds_are_floor_to_size() {
    assert_eq!(
        autofit_bounds(40, 14, 0, AutoFit::Shrink, Some(100.0), 1.4),
        (14, 40)
    );
}
#[test]
fn shrink_without_min_uses_default_floor() {
    assert_eq!(
        autofit_bounds(40, 0, 0, AutoFit::Shrink, Some(100.0), 1.4),
        (DEFAULT_MIN_AUTOFIT, 40)
    );
}
#[test]
fn grow_bounds_are_size_to_max() {
    assert_eq!(
        autofit_bounds(24, 0, 64, AutoFit::Grow, Some(100.0), 1.4),
        (24, 64)
    );
}
#[test]
fn grow_without_max_uses_box_height_ceiling() {
    // At line_height 1.0 the ceiling is the box height itself.
    assert_eq!(
        autofit_bounds(24, 0, 0, AutoFit::Grow, Some(80.7), 1.0),
        (24, 80)
    );
}
#[test]
fn grow_ceiling_accounts_for_line_height() {
    // A line at size S is `S * line_height` px tall, so a sub-unit
    // line_height lets the fitting size exceed the raw box height:
    // 80 / 0.8 = 100. A >= 1.0 line_height caps below it: 80 / 1.4 = 57.
    assert_eq!(
        autofit_bounds(24, 0, 0, AutoFit::Grow, Some(80.0), 0.8),
        (24, 100)
    );
    assert_eq!(
        autofit_bounds(24, 0, 0, AutoFit::Grow, Some(80.0), 1.4),
        (24, 57)
    );
}
#[test]
fn grow_ceiling_guards_non_positive_line_height() {
    // A degenerate line_height must not divide by zero or grow the text.
    assert_eq!(
        autofit_bounds(24, 0, 0, AutoFit::Grow, Some(80.0), 0.0),
        (24, 24)
    );
}
#[test]
fn grow_without_max_and_without_height_cannot_grow() {
    assert_eq!(autofit_bounds(24, 0, 0, AutoFit::Grow, None, 1.4), (24, 24));
}
#[test]
fn shrink_and_grow_spans_min_to_max() {
    assert_eq!(
        autofit_bounds(999, 14, 64, AutoFit::ShrinkAndGrow, Some(100.0), 1.4),
        (14, 64)
    );
}
#[test]
fn bounds_clamp_when_min_exceeds_max() {
    let (lo, hi) = autofit_bounds(40, 80, 20, AutoFit::ShrinkAndGrow, Some(100.0), 1.4);
    assert!(lo <= hi);
    assert_eq!((lo, hi), (80, 80));
}
#[test]
fn shrink_below_default_floor_does_not_grow() {
    // size 10 is below DEFAULT_MIN_AUTOFIT (12); a shrink-only search must
    // not be clamped up to the floor and enlarge the text.
    assert_eq!(
        autofit_bounds(10, 0, 0, AutoFit::Shrink, Some(100.0), 1.4),
        (10, 10)
    );
}

// ── search_fit_size ─────────────────────────────────────────────
// Synthetic measure: width == height == size px. So size N fits a target of N.
fn linear(size: u32) -> (f32, f32) {
    (size as f32, size as f32)
}

#[test]
fn search_picks_largest_fitting_size() {
    assert_eq!(search_fit_size(10, 40, Some(30.0), Some(30.0), linear), 30);
}
#[test]
fn search_returns_upper_when_everything_fits() {
    assert_eq!(
        search_fit_size(10, 40, Some(100.0), Some(100.0), linear),
        40
    );
}
#[test]
fn search_returns_lower_when_nothing_fits() {
    assert_eq!(search_fit_size(10, 40, Some(5.0), Some(5.0), linear), 10);
}
#[test]
fn search_absent_target_is_satisfied() {
    assert_eq!(search_fit_size(10, 40, Some(20.0), None, linear), 20);
}
#[test]
fn search_single_point_range() {
    assert_eq!(
        search_fit_size(16, 16, Some(100.0), Some(100.0), linear),
        16
    );
}
