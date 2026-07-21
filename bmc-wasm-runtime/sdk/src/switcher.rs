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
