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

use crate::assets::Svg;
use crate::ids::SvgId;
use crate::tree::{Node, SwitcherTab};

/// An icon tab and the click id it emits when tapped.
#[derive(Debug)]
pub struct IconTab<S: 'static> {
    pub icon: &'static Svg<S>,
    pub click_id: &'static str,
}

impl<S> Clone for IconTab<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for IconTab<S> {}

/// A segmented view switcher over `tabs`, their icons registered by `register`.
#[must_use]
pub fn switcher_with_icons<S>(
    active: usize,
    disabled: bool,
    tabs: &[IconTab<S>],
    register: impl Fn(&Svg<S>) -> Option<SvgId>,
) -> Node {
    switcher_with_ids(
        active,
        disabled,
        tabs.iter()
            .map(|tab| SwitcherTab {
                icon: register(tab.icon),
                click_id: tab.click_id.to_owned(),
            })
            .collect(),
    )
}

/// A segmented view switcher: a rounded pill of icon tabs, `active` highlighted,
/// from tabs whose icons are already registered.
/// A `disabled` switcher renders dimmed and registers no hit regions.
#[must_use]
pub fn switcher_with_ids(active: usize, disabled: bool, tabs: Vec<SwitcherTab>) -> Node {
    Node::Switcher {
        active,
        disabled,
        tabs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static CLOCK: Svg<()> = Svg {
        source: (),
        name: "clock",
        viewbox: (1.0, 1.0),
    };

    static CALENDAR: Svg<()> = Svg {
        source: (),
        name: "calendar",
        viewbox: (1.0, 1.0),
    };

    #[test]
    fn each_tab_gets_its_registered_icon_and_click_id() {
        let tabs = [
            IconTab {
                icon: &CLOCK,
                click_id: "a",
            },
            IconTab {
                icon: &CALENDAR,
                click_id: "b",
            },
        ];
        let register = |icon: &Svg<()>| match icon.name {
            "clock" => SvgId::from_wire(5),
            "calendar" => SvgId::from_wire(6),
            _ => None,
        };

        let Node::Switcher { tabs, .. } = switcher_with_icons(1, false, &tabs, register) else {
            panic!("switcher_with_icons should produce Node::Switcher");
        };

        let tabs: Vec<_> = tabs
            .into_iter()
            .map(|tab| (tab.icon, tab.click_id))
            .collect();
        assert_eq!(
            tabs,
            [
                (SvgId::from_wire(5), "a".to_owned()),
                (SvgId::from_wire(6), "b".to_owned())
            ]
        );
    }
}
