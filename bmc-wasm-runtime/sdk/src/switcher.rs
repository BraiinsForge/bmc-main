// Copyright (C) 2026  Braiins Systems s.r.o.

//! View switcher (segmented control) builder.

use bmc_wasm_protocol::SvgId;

use crate::assets::{Svg, ensure_registered};
use crate::tree::Node;

/// An icon tab and the click id it emits when tapped.
#[derive(Clone, Copy, Debug)]
pub struct Tab {
    pub icon: &'static Svg,
    pub click_id: &'static str,
}

/// A [`Tab`] with its icon resolved to a registered id.
#[derive(Clone, Debug)]
pub struct SwitcherTab {
    pub(crate) icon: Option<SvgId>,
    pub(crate) click_id: String,
}

/// A segmented view switcher: a rounded pill of icon tabs, `active` highlighted.
/// A `disabled` switcher renders dimmed and registers no hit regions.
#[must_use]
pub fn switcher(active: usize, disabled: bool, tabs: &[Tab]) -> Node {
    Node::Switcher {
        active,
        disabled,
        tabs: tabs
            .iter()
            .map(|t| SwitcherTab {
                icon: ensure_registered(t.icon),
                click_id: t.click_id.to_owned(),
            })
            .collect(),
    }
}
