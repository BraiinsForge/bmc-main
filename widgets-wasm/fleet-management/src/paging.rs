// Copyright (C) 2026  Braiins Systems s.r.o.

/// The page actually shown: the stored page clamped into the page count, so
/// a fleet shrinking under the operator can never strand the view.
#[must_use]
pub fn effective_page(page: usize, count: usize) -> usize {
    page.min(count.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_page_clamps_to_the_last_page() {
        assert_eq!(effective_page(0, 3), 0);
        assert_eq!(effective_page(2, 3), 2);
        assert_eq!(effective_page(5, 3), 2);
        assert_eq!(effective_page(5, 1), 0);
    }
}
