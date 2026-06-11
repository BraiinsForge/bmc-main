// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_sdk::ufmt;

use crate::device::DeviceFamily;
use crate::paging;
use crate::summary::GroupSummary;

/// UI navigation state, living outside the derived-data cache: page flips
/// and view switches must not refold the fleet.
pub struct ViewState {
    pub fleet_page: usize,
    pub detail: Option<DetailSelection>,
}

impl ViewState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            fleet_page: 0,
            detail: None,
        }
    }
}

impl Default for ViewState {
    fn default() -> Self {
        Self::new()
    }
}

/// The drilled-into model group — the `summarize` partition key plus the
/// detail table's own page. The fleet page stays in `ViewState` and is
/// restored by Back.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailSelection {
    pub family: Option<DeviceFamily>,
    pub label: String,
    pub page: usize,
}

/// A parsed click on one of the widget's buttons.
#[derive(Debug, Clone, PartialEq)]
pub enum ClickAction {
    Details {
        family: Option<DeviceFamily>,
        label: String,
    },
    Back,
    PagePrev,
    PageNext,
}

/// The Details click ID for a model group. The partition key rides in the
/// ID itself: clicks are hit-tested against the previously submitted tree,
/// and a discovery between tap and render can reorder the groups, so
/// indexes are not stable. `x` marks the family-less catch-all.
#[must_use]
pub fn details_click_id(family: Option<DeviceFamily>, label: &str) -> String {
    if let Some(f) = family {
        bmc_wasm_sdk::fmt!("details:{}:{label}", f.index())
    } else {
        bmc_wasm_sdk::fmt!("details:x:{label}")
    }
}

/// Map a click ID from the render readback to its action; foreign and
/// malformed IDs are ignored.
#[must_use]
pub fn parse_click(id: &str) -> Option<ClickAction> {
    match id {
        "back" => return Some(ClickAction::Back),
        "page_prev" => return Some(ClickAction::PagePrev),
        "page_next" => return Some(ClickAction::PageNext),
        _ => {}
    }
    let rest = id.strip_prefix("details:")?;
    let (family, label) = rest.split_once(':')?;
    let family = if family == "x" {
        None
    } else {
        Some(*DeviceFamily::ALL.get(family.parse::<usize>().ok()?)?)
    };
    Some(ClickAction::Details {
        family,
        label: label.to_owned(),
    })
}

/// Where the selection sits in the current groups, or `None` when it
/// vanished and the view must fall back to the fleet table.
#[must_use]
pub fn selected_index(groups: &[GroupSummary], selection: &DetailSelection) -> Option<usize> {
    groups
        .iter()
        .position(|g| g.family == selection.family && g.label == selection.label)
}

/// Apply a click to the state. `page_count` is the page count of the frame
/// just rendered; clicks that raced a data change are clamped against it.
/// Returns whether the state changed (and the widget needs a redraw).
pub fn apply(state: &mut ViewState, action: ClickAction, page_count: usize) -> bool {
    match action {
        ClickAction::Details { family, label } => {
            state.detail = Some(DetailSelection {
                family,
                label,
                page: 0,
            });
            true
        }
        ClickAction::Back => state.detail.take().is_some(),
        ClickAction::PagePrev => turn_page(active_page(state), page_count, PageTurn::Prev),
        ClickAction::PageNext => turn_page(active_page(state), page_count, PageTurn::Next),
    }
}

fn active_page(state: &mut ViewState) -> &mut usize {
    state
        .detail
        .as_mut()
        .map_or(&mut state.fleet_page, |d| &mut d.page)
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

    use crate::device::DeviceFamily;

    fn group(family: Option<DeviceFamily>, label: &str) -> crate::summary::GroupSummary {
        use units::availability::Availability;
        crate::summary::GroupSummary {
            label: label.to_owned(),
            family,
            hashrate: Availability::Unavailable,
            power: Availability::Unavailable,
            efficiency: Availability::Unavailable,
            min_temperature: Availability::Unavailable,
            avg_temperature: Availability::Unavailable,
            max_temperature: Availability::Unavailable,
            total_count: 1,
            ok_count: 1,
        }
    }

    #[test]
    fn details_click_id_round_trips() {
        let id = details_click_id(Some(DeviceFamily::Bos), "BMM 101");
        assert_eq!(
            parse_click(&id),
            Some(ClickAction::Details {
                family: Some(DeviceFamily::Bos),
                label: "BMM 101".to_owned(),
            })
        );
    }

    #[test]
    fn details_click_id_round_trips_labels_with_colons_and_no_family() {
        let id = details_click_id(None, "Odd: Name");
        assert_eq!(
            parse_click(&id),
            Some(ClickAction::Details {
                family: None,
                label: "Odd: Name".to_owned(),
            })
        );
    }

    #[test]
    fn details_id_with_an_empty_label_is_accepted_and_falls_back_next_frame() {
        assert_eq!(
            parse_click("details:x:"),
            Some(ClickAction::Details {
                family: None,
                label: String::new(),
            })
        );
    }

    #[test]
    fn malformed_details_ids_are_ignored() {
        assert_eq!(parse_click("details:9:Foo"), None);
        assert_eq!(parse_click("details:Foo"), None);
        assert_eq!(parse_click("details:"), None);
    }

    #[test]
    fn details_click_enters_the_group_at_page_zero_keeping_the_fleet_page() {
        let mut state = ViewState::new();
        state.fleet_page = 2;
        assert!(apply(
            &mut state,
            ClickAction::Details {
                family: Some(DeviceFamily::Bos),
                label: "BMM 101".to_owned(),
            },
            5,
        ));
        let sel = state.detail.as_ref().expect("BUG: selection just set");
        assert_eq!(sel.label, "BMM 101");
        assert_eq!(sel.page, 0);
        assert_eq!(state.fleet_page, 2);
    }

    #[test]
    fn back_returns_to_the_fleet_and_reports_no_change_when_already_there() {
        let mut state = ViewState::new();
        state.fleet_page = 2;
        state.detail = Some(DetailSelection {
            family: None,
            label: "Unknown".to_owned(),
            page: 1,
        });
        assert!(apply(&mut state, ClickAction::Back, 5));
        assert!(state.detail.is_none());
        assert_eq!(state.fleet_page, 2);
        assert!(!apply(&mut state, ClickAction::Back, 5));
    }

    #[test]
    fn paging_targets_the_detail_page_while_drilled_in() {
        let mut state = ViewState::new();
        state.fleet_page = 2;
        state.detail = Some(DetailSelection {
            family: Some(DeviceFamily::Bos),
            label: "BMM 101".to_owned(),
            page: 0,
        });
        assert!(apply(&mut state, ClickAction::PageNext, 3));
        assert_eq!(
            state.detail.as_ref().expect("BUG: still drilled in").page,
            1
        );
        assert_eq!(state.fleet_page, 2);
    }

    #[test]
    fn selected_index_requires_family_and_label_match() {
        let groups = vec![
            group(Some(DeviceFamily::Ubos), "BMM 101"),
            group(Some(DeviceFamily::Bos), "BMM 101"),
            group(None, "Unknown"),
        ];
        let sel = DetailSelection {
            family: Some(DeviceFamily::Bos),
            label: "BMM 101".to_owned(),
            page: 0,
        };
        assert_eq!(selected_index(&groups, &sel), Some(1));
        let gone = DetailSelection {
            family: Some(DeviceFamily::Bitaxe),
            label: "BMM 101".to_owned(),
            page: 0,
        };
        assert_eq!(selected_index(&groups, &gone), None);
    }
}
