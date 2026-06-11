// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_sdk::SizeVariant;

// Vertical space above the table rows: 24px padding top and bottom, the
// overview cluster (taller in Full: 64px title / 48px hero), the separator,
// the 32px header row (its pager buttons set the height), and the column
// gaps between them.
const OVERHEAD_FULL: u32 = 180;
const OVERHEAD_LARGE: u32 = 164;

// One model row plus the column gap; the row holds a 32px details button,
// so the step is button height, not text height.
const FLEET_ROW_STEP: u32 = 38;

/// Model rows that fit one page of the breakdown table; at least 1 so a
/// pathological viewport still shows something.
#[must_use]
pub fn rows_per_page_fleet(height: u32, variant: SizeVariant) -> usize {
    rows_for(height, overhead(variant), FLEET_ROW_STEP)
}

fn overhead(variant: SizeVariant) -> u32 {
    match variant {
        SizeVariant::Full => OVERHEAD_FULL,
        SizeVariant::Large | SizeVariant::Medium | SizeVariant::Small => OVERHEAD_LARGE,
    }
}

fn rows_for(height: u32, overhead: u32, step: u32) -> usize {
    let rows = height.saturating_sub(overhead) / step;
    usize::try_from(rows)
        .expect("BUG: a u32 row count fits usize")
        .max(1)
}

/// Pages needed for `len` rows; at least 1 so the indicator reads `1/1`.
#[must_use]
pub fn page_count(len: usize, per_page: usize) -> usize {
    len.div_ceil(per_page).max(1)
}

/// The page actually shown: the stored page clamped into the page count, so
/// a fleet shrinking under the operator can never strand the view.
#[must_use]
pub fn effective_page(page: usize, count: usize) -> usize {
    page.min(count.saturating_sub(1))
}

/// Index range of `page`'s rows.
#[must_use]
pub fn page_bounds(len: usize, per_page: usize, page: usize) -> core::ops::Range<usize> {
    let start = (page * per_page).min(len);
    let end = (start + per_page).min(len);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::SizeVariant;

    #[test]
    fn fleet_rows_fit_the_canonical_boxes() {
        assert_eq!(rows_per_page_fleet(480, SizeVariant::Full), 7);
        assert_eq!(rows_per_page_fleet(480, SizeVariant::Large), 8);
    }

    #[test]
    fn at_least_one_row_even_when_the_viewport_is_tiny() {
        assert_eq!(rows_per_page_fleet(10, SizeVariant::Full), 1);
    }

    #[test]
    fn page_count_rounds_up_and_is_never_zero() {
        assert_eq!(page_count(0, 7), 1);
        assert_eq!(page_count(7, 7), 1);
        assert_eq!(page_count(8, 7), 2);
        assert_eq!(page_count(21, 7), 3);
    }

    #[test]
    fn effective_page_clamps_to_the_last_page() {
        assert_eq!(effective_page(0, 3), 0);
        assert_eq!(effective_page(2, 3), 2);
        assert_eq!(effective_page(5, 3), 2);
        assert_eq!(effective_page(5, 1), 0);
    }

    #[test]
    fn page_bounds_slice_full_and_partial_pages() {
        assert_eq!(page_bounds(10, 4, 0), 0..4);
        assert_eq!(page_bounds(10, 4, 1), 4..8);
        assert_eq!(page_bounds(10, 4, 2), 8..10);
    }
}
