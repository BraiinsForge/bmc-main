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

//! View switcher (segmented control) builder, from icons the host registers.

use bmc_ui::components::switcher::{IconTab, switcher_with_icons};
use bmc_ui::tree::Node;

use crate::assets::{Source, ensure_registered};

/// An icon tab and the click id it emits when tapped.
pub type Tab = IconTab<Source>;

/// A segmented view switcher: a rounded pill of icon tabs, `active` highlighted.
/// A `disabled` switcher renders dimmed and registers no hit regions.
#[must_use]
pub fn switcher(active: usize, disabled: bool, tabs: &[Tab]) -> Node {
    switcher_with_icons(active, disabled, tabs, ensure_registered)
}
