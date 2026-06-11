// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::paging;

/// UI navigation state, living outside the derived-data cache: page flips
/// must not refold the fleet.
pub struct ViewState {
    pub fleet_page: usize,
}

impl ViewState {
    #[must_use]
    pub const fn new() -> Self {
        Self { fleet_page: 0 }
    }
}

impl Default for ViewState {
    fn default() -> Self {
        Self::new()
    }
}

/// A parsed click on one of the widget's buttons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClickAction {
    PagePrev,
    PageNext,
}

/// Map a click ID from the render readback to its action; foreign IDs are
/// ignored.
#[must_use]
pub fn parse_click(id: &str) -> Option<ClickAction> {
    match id {
        "page_prev" => Some(ClickAction::PagePrev),
        "page_next" => Some(ClickAction::PageNext),
        _ => None,
    }
}

/// Apply a click to the state. `page_count` is the page count of the view
/// the click was rendered in. Returns whether the state changed (and the
/// widget needs a redraw).
pub fn apply(state: &mut ViewState, action: ClickAction, page_count: usize) -> bool {
    match action {
        ClickAction::PagePrev => turn_page(&mut state.fleet_page, page_count, PageTurn::Prev),
        ClickAction::PageNext => turn_page(&mut state.fleet_page, page_count, PageTurn::Next),
    }
}

#[derive(Clone, Copy)]
enum PageTurn {
    Prev,
    Next,
}

fn turn_page(page: &mut usize, page_count: usize, turn: PageTurn) -> bool {
    let current = paging::effective_page(*page, page_count);
    let target = match turn {
        PageTurn::Prev => current.saturating_sub(1),
        PageTurn::Next => (current + 1).min(page_count.saturating_sub(1)),
    };
    if target == *page {
        return false;
    }
    *page = target;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pager_clicks_and_ignores_foreign_ids() {
        assert_eq!(parse_click("page_prev"), Some(ClickAction::PagePrev));
        assert_eq!(parse_click("page_next"), Some(ClickAction::PageNext));
        assert_eq!(parse_click("volume"), None);
    }

    #[test]
    fn next_advances_until_the_last_page() {
        let mut state = ViewState::new();
        assert!(apply(&mut state, ClickAction::PageNext, 3));
        assert_eq!(state.fleet_page, 1);
        assert!(apply(&mut state, ClickAction::PageNext, 3));
        assert_eq!(state.fleet_page, 2);
        assert!(!apply(&mut state, ClickAction::PageNext, 3));
        assert_eq!(state.fleet_page, 2);
    }

    #[test]
    fn prev_goes_back_and_stops_at_the_first_page() {
        let mut state = ViewState::new();
        state.fleet_page = 1;
        assert!(apply(&mut state, ClickAction::PagePrev, 3));
        assert_eq!(state.fleet_page, 0);
        assert!(!apply(&mut state, ClickAction::PagePrev, 3));
        assert_eq!(state.fleet_page, 0);
    }

    #[test]
    fn a_stale_page_normalizes_against_the_current_count() {
        let mut state = ViewState::new();
        state.fleet_page = 9;
        assert!(apply(&mut state, ClickAction::PagePrev, 3));
        assert_eq!(state.fleet_page, 1);
    }
}
