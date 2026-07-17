// Copyright (C) 2026  Braiins Systems s.r.o.

use bmc_wasm_sdk::ufmt;

use crate::device::DeviceFamily;
use crate::paging;
use crate::summary::GroupSummary;

/// The top-level fleet view: the grid overview or the per-model list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

/// UI navigation state, living outside the derived-data cache: page flips
/// and view switches must not refold the fleet.
#[derive(Debug)]
pub struct ViewState {
    pub mode: ViewMode,
    pub fleet_page: usize,
    pub model_detail: Option<ModelDetailSelection>,
}

impl ViewState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mode: ViewMode::Grid,
            fleet_page: 0,
            model_detail: None,
        }
    }
}

impl Default for ViewState {
    fn default() -> Self {
        Self::new()
    }
}

/// The drilled-into model group — the `summarize`
/// partition key plus the model-detail table's own page.
/// The fleet page stays in `ViewState` and is restored by Back.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDetailSelection {
    pub family: Option<DeviceFamily>,
    pub label: String,
    pub page: usize,
    /// The drilled-into device, by id;
    /// `None` shows the model breakdown.
    pub device: Option<String>,
}

/// A parsed click on one of the widget's buttons.
#[derive(Debug, Clone, PartialEq)]
pub enum ClickAction {
    OpenModelDetail {
        family: Option<DeviceFamily>,
        label: String,
    },
    /// Drill from the model breakdown into a single device.
    OpenDevice {
        id: String,
    },
    Back,
    /// Jump straight to an ancestor level from a breadcrumb segment, skipping
    /// intermediate levels that a one-step Back would stop at.
    Jump(CrumbTarget),
    Page {
        scope: PagerScope,
        turn: PageTurn,
    },
    SetView(ViewMode),
}

/// The ancestor level a breadcrumb jump targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrumbTarget {
    Fleet,
    Model,
}

/// Which view's pager a click belongs to.
/// The two pagers carry distinct click IDs: a tap queued
/// in the model-detail view must not be consumed by the fleet pager
/// when the selection vanishes between tap and render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerScope {
    Fleet,
    ModelDetail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTurn {
    Prev,
    Next,
}

/// The click ID of the grid/list view-switcher tab for a mode.
#[must_use]
pub const fn view_click_id(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Grid => "view:grid",
        ViewMode::List => "view:list",
    }
}

/// The click id of a breadcrumb segment that jumps to an ancestor level.
#[must_use]
pub const fn crumb_click_id(target: CrumbTarget) -> &'static str {
    match target {
        CrumbTarget::Fleet => "crumb:fleet",
        CrumbTarget::Model => "crumb:model",
    }
}

/// The click ID of a view's pager button.
#[must_use]
pub const fn pager_click_id(scope: PagerScope, turn: PageTurn) -> &'static str {
    match (scope, turn) {
        (PagerScope::Fleet, PageTurn::Prev) => "fleet:page_prev",
        (PagerScope::Fleet, PageTurn::Next) => "fleet:page_next",
        (PagerScope::ModelDetail, PageTurn::Prev) => "model_detail:page_prev",
        (PagerScope::ModelDetail, PageTurn::Next) => "model_detail:page_next",
    }
}

/// The click ID that opens a model group's detail view.
/// The partition key rides in the ID itself: clicks are hit-tested
/// against the previously submitted tree, and a discovery between
/// tap and render can reorder the groups, so indexes are not stable.
/// `x` marks the family-less catch-all. The `open_model_detail:` prefix
/// stays distinct from the pager's `model_detail:` so neither parses the other.
#[must_use]
pub fn model_detail_click_id(family: Option<DeviceFamily>, label: &str) -> String {
    if let Some(f) = family {
        bmc_wasm_sdk::fmt!("open_model_detail:{}:{label}", f.index())
    } else {
        bmc_wasm_sdk::fmt!("open_model_detail:x:{label}")
    }
}

/// The click id for drilling into a single device. The whole device id is the
/// suffix, so ids containing colons (manual `host:port`) round-trip intact.
#[must_use]
pub fn device_click_id(id: &str) -> String {
    bmc_wasm_sdk::fmt!("device:{id}")
}

/// Map a click ID from the render readback to its action;
/// foreign and malformed IDs are ignored.
#[must_use]
pub fn parse_click(id: &str) -> Option<ClickAction> {
    if id == "back" {
        return Some(ClickAction::Back);
    }
    for target in [CrumbTarget::Fleet, CrumbTarget::Model] {
        if id == crumb_click_id(target) {
            return Some(ClickAction::Jump(target));
        }
    }
    for mode in [ViewMode::Grid, ViewMode::List] {
        if id == view_click_id(mode) {
            return Some(ClickAction::SetView(mode));
        }
    }
    for scope in [PagerScope::Fleet, PagerScope::ModelDetail] {
        for turn in [PageTurn::Prev, PageTurn::Next] {
            if id == pager_click_id(scope, turn) {
                return Some(ClickAction::Page { scope, turn });
            }
        }
    }
    if let Some(device) = id.strip_prefix("device:") {
        return Some(ClickAction::OpenDevice {
            id: device.to_owned(),
        });
    }
    let rest = id.strip_prefix("open_model_detail:")?;
    let (family, label) = rest.split_once(':')?;
    let family = if family == "x" {
        None
    } else {
        Some(*DeviceFamily::ALL.get(family.parse::<usize>().ok()?)?)
    };
    Some(ClickAction::OpenModelDetail {
        family,
        label: label.to_owned(),
    })
}

/// Where the selection sits in the current groups, or `None`
/// when it vanished and the view must fall back to the fleet table.
#[must_use]
pub fn selected_index(groups: &[GroupSummary], selection: &ModelDetailSelection) -> Option<usize> {
    groups
        .iter()
        .position(|g| g.family == selection.family && g.label == selection.label)
}

/// Apply a click to the state. `page_count` is the page count of the frame
/// just rendered; clicks that raced a data change are clamped against it.
/// Returns whether the state changed (and the widget needs a redraw).
pub fn apply(state: &mut ViewState, action: ClickAction, page_count: usize) -> bool {
    match action {
        ClickAction::OpenModelDetail { family, label } => {
            state.model_detail = Some(ModelDetailSelection {
                family,
                label,
                page: 0,
                device: None,
            });
            true
        }
        ClickAction::OpenDevice { id } => {
            if let Some(detail) = state.model_detail.as_mut() {
                detail.device = Some(id);
                true
            } else {
                false
            }
        }
        ClickAction::Back => {
            // Back steps out one level: a device → the model breakdown, then
            // the model → the fleet.
            if let Some(detail) = state.model_detail.as_mut()
                && detail.device.take().is_some()
            {
                return true;
            }
            state.model_detail.take().is_some()
        }
        ClickAction::Jump(CrumbTarget::Fleet) => state.model_detail.take().is_some(),
        ClickAction::Jump(CrumbTarget::Model) => state
            .model_detail
            .as_mut()
            .is_some_and(|detail| detail.device.take().is_some()),
        ClickAction::SetView(mode) => {
            if state.mode == mode {
                return false;
            }
            state.mode = mode;
            true
        }
        ClickAction::Page { scope, turn } => {
            let page = match scope {
                PagerScope::Fleet => &mut state.fleet_page,
                // A model-detail-scoped click with no selection raced a fallback
                // to the fleet table; ignore it rather than flip the wrong
                // view's page.
                PagerScope::ModelDetail => match state.model_detail.as_mut() {
                    Some(detail) => &mut detail.page,
                    None => return false,
                },
            };
            turn_page(page, page_count, turn)
        }
    }
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

    fn page(scope: PagerScope, turn: PageTurn) -> ClickAction {
        ClickAction::Page { scope, turn }
    }

    #[test]
    fn parses_pager_clicks_and_ignores_foreign_ids() {
        assert_eq!(
            parse_click("fleet:page_prev"),
            Some(page(PagerScope::Fleet, PageTurn::Prev))
        );
        assert_eq!(
            parse_click("model_detail:page_next"),
            Some(page(PagerScope::ModelDetail, PageTurn::Next))
        );
        // The pager IDs are scoped per view; an unscoped ID must not
        // navigate either pager.
        assert_eq!(parse_click("page_next"), None);
        assert_eq!(parse_click("volume"), None);
    }

    #[test]
    fn pager_click_ids_round_trip() {
        for scope in [PagerScope::Fleet, PagerScope::ModelDetail] {
            for turn in [PageTurn::Prev, PageTurn::Next] {
                assert_eq!(
                    parse_click(pager_click_id(scope, turn)),
                    Some(page(scope, turn))
                );
            }
        }
    }

    #[test]
    fn next_advances_until_the_last_page() {
        let mut state = ViewState::new();
        assert!(apply(
            &mut state,
            page(PagerScope::Fleet, PageTurn::Next),
            3
        ));
        assert_eq!(state.fleet_page, 1);
        assert!(apply(
            &mut state,
            page(PagerScope::Fleet, PageTurn::Next),
            3
        ));
        assert_eq!(state.fleet_page, 2);
        assert!(!apply(
            &mut state,
            page(PagerScope::Fleet, PageTurn::Next),
            3
        ));
        assert_eq!(state.fleet_page, 2);
    }

    #[test]
    fn prev_goes_back_and_stops_at_the_first_page() {
        let mut state = ViewState::new();
        state.fleet_page = 1;
        assert!(apply(
            &mut state,
            page(PagerScope::Fleet, PageTurn::Prev),
            3
        ));
        assert_eq!(state.fleet_page, 0);
        assert!(!apply(
            &mut state,
            page(PagerScope::Fleet, PageTurn::Prev),
            3
        ));
        assert_eq!(state.fleet_page, 0);
    }

    #[test]
    fn a_stale_page_normalizes_against_the_current_count() {
        let mut state = ViewState::new();
        state.fleet_page = 9;
        assert!(apply(
            &mut state,
            page(PagerScope::Fleet, PageTurn::Prev),
            3
        ));
        assert_eq!(state.fleet_page, 1);
    }

    #[test]
    fn a_model_detail_page_click_without_a_selection_is_ignored() {
        // The tap raced a fallback: the selected group vanished and the
        // fleet table took over before the click was consumed. The fleet
        // page must not move.
        let mut state = ViewState::new();
        state.fleet_page = 1;
        assert!(!apply(
            &mut state,
            page(PagerScope::ModelDetail, PageTurn::Next),
            3
        ));
        assert_eq!(state.fleet_page, 1);
        assert!(state.model_detail.is_none());
    }

    use crate::device::DeviceFamily;

    fn group(family: Option<DeviceFamily>, label: &str) -> crate::summary::GroupSummary {
        crate::summary::GroupSummary {
            label: label.to_owned(),
            family,
            hashrate: None,
            power: None,
            efficiency: None,
            min_temperature: None,
            avg_temperature: None,
            max_temperature: None,
            total_count: 1,
            ok_count: 1,
            off_count: 0,
        }
    }

    #[test]
    fn model_detail_click_id_round_trips() {
        let id = model_detail_click_id(Some(DeviceFamily::Bos), "BMM 101");
        assert_eq!(
            parse_click(&id),
            Some(ClickAction::OpenModelDetail {
                family: Some(DeviceFamily::Bos),
                label: "BMM 101".to_owned(),
            })
        );
    }

    #[test]
    fn model_detail_click_id_round_trips_labels_with_colons_and_no_family() {
        let id = model_detail_click_id(None, "Odd: Name");
        assert_eq!(
            parse_click(&id),
            Some(ClickAction::OpenModelDetail {
                family: None,
                label: "Odd: Name".to_owned(),
            })
        );
    }

    #[test]
    fn open_model_detail_id_with_an_empty_label_is_accepted_and_falls_back_next_frame() {
        assert_eq!(
            parse_click("open_model_detail:x:"),
            Some(ClickAction::OpenModelDetail {
                family: None,
                label: String::new(),
            })
        );
    }

    #[test]
    fn malformed_open_model_detail_ids_are_ignored() {
        assert_eq!(parse_click("open_model_detail:9:Foo"), None);
        assert_eq!(parse_click("open_model_detail:Foo"), None);
        assert_eq!(parse_click("open_model_detail:"), None);
    }

    #[test]
    fn open_model_detail_click_enters_the_group_at_page_zero_keeping_the_fleet_page() {
        let mut state = ViewState::new();
        state.fleet_page = 2;
        assert!(apply(
            &mut state,
            ClickAction::OpenModelDetail {
                family: Some(DeviceFamily::Bos),
                label: "BMM 101".to_owned(),
            },
            5,
        ));
        let sel = state
            .model_detail
            .as_ref()
            .expect("BUG: selection just set");
        assert_eq!(sel.label, "BMM 101");
        assert_eq!(sel.page, 0);
        assert_eq!(state.fleet_page, 2);
    }

    #[test]
    fn back_returns_to_the_fleet_and_reports_no_change_when_already_there() {
        let mut state = ViewState::new();
        state.fleet_page = 2;
        state.model_detail = Some(ModelDetailSelection {
            family: None,
            label: "Unknown".to_owned(),
            page: 1,
            device: None,
        });
        assert!(apply(&mut state, ClickAction::Back, 5));
        assert!(state.model_detail.is_none());
        assert_eq!(state.fleet_page, 2);
        assert!(!apply(&mut state, ClickAction::Back, 5));
    }

    #[test]
    fn paging_targets_the_model_detail_page_while_drilled_in() {
        let mut state = ViewState::new();
        state.fleet_page = 2;
        state.model_detail = Some(ModelDetailSelection {
            family: Some(DeviceFamily::Bos),
            label: "BMM 101".to_owned(),
            page: 0,
            device: None,
        });
        assert!(apply(
            &mut state,
            page(PagerScope::ModelDetail, PageTurn::Next),
            3
        ));
        assert_eq!(
            state
                .model_detail
                .as_ref()
                .expect("BUG: still drilled in")
                .page,
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
        let sel = ModelDetailSelection {
            family: Some(DeviceFamily::Bos),
            label: "BMM 101".to_owned(),
            page: 0,
            device: None,
        };
        assert_eq!(selected_index(&groups, &sel), Some(1));
        let gone = ModelDetailSelection {
            family: Some(DeviceFamily::Bitaxe),
            label: "BMM 101".to_owned(),
            page: 0,
            device: None,
        };
        assert_eq!(selected_index(&groups, &gone), None);
    }

    #[test]
    fn view_toggle_click_switches_mode_and_is_idempotent() {
        assert_eq!(
            parse_click(view_click_id(ViewMode::List)),
            Some(ClickAction::SetView(ViewMode::List))
        );
        let mut state = ViewState::new();
        assert_eq!(state.mode, ViewMode::Grid);
        assert!(apply(&mut state, ClickAction::SetView(ViewMode::List), 3));
        assert_eq!(state.mode, ViewMode::List);
        assert!(
            !apply(&mut state, ClickAction::SetView(ViewMode::List), 3),
            "switching to the current view is a no-op"
        );
    }

    #[test]
    fn device_click_id_round_trips_ids_with_colons() {
        let id = device_click_id("bos/manual/host:80");
        assert_eq!(
            parse_click(&id),
            Some(ClickAction::OpenDevice {
                id: "bos/manual/host:80".to_owned(),
            })
        );
    }

    #[test]
    fn opening_a_device_then_back_steps_out_one_level_at_a_time() {
        let mut state = ViewState::new();
        state.model_detail = Some(ModelDetailSelection {
            family: Some(DeviceFamily::Bos),
            label: "BMM 101".to_owned(),
            page: 1,
            device: None,
        });
        assert!(apply(
            &mut state,
            ClickAction::OpenDevice {
                id: "bos/a".to_owned(),
            },
            1,
        ));
        assert_eq!(
            state
                .model_detail
                .as_ref()
                .expect("BUG: still drilled in")
                .device
                .as_deref(),
            Some("bos/a")
        );
        // Back: the device → the model breakdown, keeping the model page.
        assert!(apply(&mut state, ClickAction::Back, 1));
        let sel = state.model_detail.as_ref().expect("BUG: back to the model");
        assert_eq!(sel.device, None);
        assert_eq!(sel.page, 1);
        // Back again: the model → the fleet.
        assert!(apply(&mut state, ClickAction::Back, 1));
        assert!(state.model_detail.is_none());
    }

    #[test]
    fn a_device_click_without_a_selection_is_ignored() {
        let mut state = ViewState::new();
        assert!(!apply(
            &mut state,
            ClickAction::OpenDevice {
                id: "bos/a".to_owned(),
            },
            1,
        ));
        assert!(state.model_detail.is_none());
    }

    #[test]
    fn crumb_ids_round_trip_to_jumps() {
        assert_eq!(
            parse_click("crumb:fleet"),
            Some(ClickAction::Jump(CrumbTarget::Fleet))
        );
        assert_eq!(
            parse_click("crumb:model"),
            Some(ClickAction::Jump(CrumbTarget::Model))
        );
    }

    fn drilled_into_device() -> ViewState {
        let mut state = ViewState::new();
        state.fleet_page = 2;
        state.model_detail = Some(ModelDetailSelection {
            family: Some(DeviceFamily::Bos),
            label: "BMM 101".to_owned(),
            page: 1,
            device: Some("bos/a".to_owned()),
        });
        state
    }

    #[test]
    fn jump_to_fleet_from_a_device_clears_the_whole_selection() {
        let mut state = drilled_into_device();
        assert!(apply(&mut state, ClickAction::Jump(CrumbTarget::Fleet), 1));
        assert!(
            state.model_detail.is_none(),
            "a two-level jump lands on the fleet"
        );
        assert_eq!(state.fleet_page, 2, "the fleet page is restored");
        assert!(!apply(&mut state, ClickAction::Jump(CrumbTarget::Fleet), 1));
    }

    #[test]
    fn jump_to_model_from_a_device_drops_only_the_device() {
        let mut state = drilled_into_device();
        assert!(apply(&mut state, ClickAction::Jump(CrumbTarget::Model), 1));
        let sel = state
            .model_detail
            .as_ref()
            .expect("BUG: still on the model");
        assert_eq!(sel.device, None);
        assert_eq!(sel.page, 1, "the model page is kept");
        assert!(!apply(&mut state, ClickAction::Jump(CrumbTarget::Model), 1));
    }
}
