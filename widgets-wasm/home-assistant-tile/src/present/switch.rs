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

//! Switches and the like: entities that are on or off, which the card turns on and off.
//! The tile is laid out as a momentary one; the icon keeps its disc only while on,
//! so the position shows whatever the colour or the icon.

use bmc_wasm_sdk::*;
use home_assistant::domain::switch::DeviceClass;
use home_assistant::domain::{automation, input_boolean, on_off, remote, siren, switch};
use home_assistant::{Entity, ServiceCall};

use super::{CardAction, Context, Presenter, card, card_action};
use crate::interaction::pending::{Request, Slot};
use crate::view::blocks::badge::{Disc, Mark};
use crate::view::card::{Card, Line, StateDisplay};
use crate::view::style::{Accent, Icon};
use crate::view::touches::Touches;

const POWER: Slot = Slot::new("power");

const BULLHORN: Svg = include_svg!("assets/mdi/bullhorn.svg");
const POWER_PLUG: Svg = include_svg!("assets/mdi/power-plug.svg");
const POWER_PLUG_OFF: Svg = include_svg!("assets/mdi/power-plug-off.svg");
const REMOTE: Svg = include_svg!("assets/mdi/remote.svg");
const REMOTE_OFF: Svg = include_svg!("assets/mdi/remote-off.svg");
const ROBOT: Svg = include_svg!("assets/mdi/robot.svg");
const ROBOT_OFF: Svg = include_svg!("assets/mdi/robot-off.svg");
const TOGGLE_SWITCH: Svg = include_svg!("assets/mdi/toggle-switch.svg");
const TOGGLE_SWITCH_OFF_OUTLINE: Svg = include_svg!("assets/mdi/toggle-switch-off-outline.svg");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// On while enabled: the tile enables and disables it, as the frontend's does, and never runs it.
    Automation,
    InputBoolean,
    Remote,
    Siren,
    Switch,
}

impl Kind {
    /// Its icon while off, also worn before the position is known.
    pub(super) fn icon(self) -> Icon {
        self.icon_in(Position::Off, None)
    }

    /// An entity in no known position wears its icon for off: a tap turns it on, as one that is off.
    fn icon_in(self, position: Position, class: Option<DeviceClass>) -> Icon {
        let (on, off) = self.icons(class);
        match position {
            Position::On => on,
            Position::Off | Position::Unknown => off,
        }
    }

    /// The frontend's icons for its domain while on and while off, but a toggle for a switch and an input boolean:
    /// a switch's rocker reads as two different objects, and an input boolean's check and cross as yes and no.
    /// A switch's device class can pick another pair, and a siren has no icon of its own for off.
    fn icons(self, class: Option<DeviceClass>) -> (Icon, Icon) {
        match (self, class) {
            (Self::Switch, Some(DeviceClass::Outlet)) => (Icon(&POWER_PLUG), Icon(&POWER_PLUG_OFF)),
            (Self::Switch, Some(DeviceClass::Switch) | None) | (Self::InputBoolean, _) => {
                (Icon(&TOGGLE_SWITCH), Icon(&TOGGLE_SWITCH_OFF_OUTLINE))
            }
            (Self::Automation, _) => (Icon(&ROBOT), Icon(&ROBOT_OFF)),
            (Self::Remote, _) => (Icon(&REMOTE), Icon(&REMOTE_OFF)),
            (Self::Siren, _) => (Icon(&BULLHORN), Icon(&BULLHORN)),
        }
    }

    /// The frontend's colour for its domain while on.
    fn color(self) -> Accent {
        match self {
            Self::Automation | Self::InputBoolean | Self::Remote | Self::Switch => Accent::Amber,
            Self::Siren => Accent::Red,
        }
    }

    fn turn(self, on: bool) -> ServiceCall {
        match (self, on) {
            (Self::Automation, true) => automation::turn_on(),
            (Self::Automation, false) => automation::turn_off(),
            (Self::InputBoolean, true) => input_boolean::turn_on(),
            (Self::InputBoolean, false) => input_boolean::turn_off(),
            (Self::Remote, true) => remote::turn_on(),
            (Self::Remote, false) => remote::turn_off(),
            (Self::Siren, true) => siren::turn_on(),
            (Self::Siren, false) => siren::turn_off(),
            (Self::Switch, true) => switch::turn_on(),
            (Self::Switch, false) => switch::turn_off(),
        }
    }

    /// What a flip from `position` asks of Home Assistant:
    /// one in no known position turns on, as Home Assistant's toggle service does.
    #[must_use]
    pub fn request(self, position: Position) -> Request {
        let on = match position {
            Position::On => false,
            Position::Off | Position::Unknown => true,
        };
        Request::flip(POWER, on, self.turn(on))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    On,
    Off,
    /// Home Assistant does not know it, such as for a switch it has not heard from since it started.
    Unknown,
}

impl Position {
    fn of(on: Option<bool>) -> Self {
        match on {
            Some(true) => Self::On,
            Some(false) => Self::Off,
            None => Self::Unknown,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::On => "On",
            Self::Off => "Off",
            Self::Unknown => "Unknown",
        }
    }
}

/// What the tile reads of an entity that is on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub position: Position,
    /// A switch's; the other kinds have none.
    pub device_class: Option<DeviceClass>,
}

impl State {
    #[must_use]
    pub fn read(kind: Kind, entity: &Entity) -> Self {
        let device_class = match kind {
            Kind::Switch => switch::device_class(entity),
            Kind::Automation | Kind::InputBoolean | Kind::Remote | Kind::Siren => None,
        };
        Self {
            position: Position::of(on_off(entity)),
            device_class,
        }
    }
}

/// One frame of a switch's tile: the card flips it.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    card: Card,
    card_action: CardAction,
}

impl Tile {
    /// A flip on its way shows at once, and another flip goes back from it, as the frontend's toggle does.
    #[must_use]
    pub fn new(kind: Kind, state: &State, context: &Context) -> Self {
        let shown = context
            .pending
            .on(POWER)
            .map_or(state.position, |on| Position::of(Some(on)));
        let on = shown == Position::On;
        let display = StateDisplay {
            line: Some(Line::State(shown.text().to_owned())),
            color: on.then_some(kind.color()),
            icon: kind.icon_in(shown, state.device_class),
            disc: if on { Disc::Tinted } else { Disc::Bare },
            mark: (shown == Position::Unknown).then_some(Mark::Unknown),
        };
        // Unlike a button's, a switch's tile set to neither tap nor hold is a status on purpose, not a mistake.
        let card_action = if context.gesture.can_fire() {
            card_action(context, true, kind.request(shown))
        } else {
            CardAction::None
        };
        Self {
            card: card(context, display, &card_action),
            card_action,
        }
    }
}

impl Presenter for Tile {
    fn view(&self, touches: &Touches) -> Node {
        self.card.centred(touches)
    }

    /// It has no blocks: the card is its switch.
    fn touched(&self, _touches: &Touches) -> Option<Request> {
        None
    }

    fn card_action(&self) -> CardAction {
        self.card_action.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use crate::interaction::gesture::{Gesture, OnTap};
    use crate::interaction::pending::Pending;
    use crate::present::Reach;
    use bmc_wasm_sdk::wasm_test;
    use home_assistant::{AttributeValue, CallId};

    const KINDS: [Kind; 5] = [
        Kind::Automation,
        Kind::InputBoolean,
        Kind::Remote,
        Kind::Siren,
        Kind::Switch,
    ];

    fn at(position: Position) -> State {
        State {
            position,
            device_class: None,
        }
    }

    fn tap() -> Gesture {
        Gesture::new(OnTap::Fires, None)
    }

    fn tile(kind: Kind, state: State) -> Tile {
        Tile::new(
            kind,
            &state,
            &fixtures::gate(&Pending::default(), &tap(), Reach::Ready),
        )
    }

    /// The tile while a flip to `on` waits for Home Assistant, which still reports `reported`.
    fn flipping(kind: Kind, reported: Position, on: bool) -> Tile {
        let mut pending = Pending::default();
        let from = if on { Position::Off } else { Position::On };
        pending.submit(kind.request(from), |_, _| Ok(CallId::from_raw(1)));
        Tile::new(
            kind,
            &at(reported),
            &fixtures::gate(&pending, &tap(), Reach::Ready),
        )
    }

    fn fired(tile: &Tile) -> Option<(&'static str, &'static str)> {
        match tile.card_action() {
            CardAction::Ready(request) => Some((request.call().domain, request.call().service)),
            CardAction::None | CardAction::Blocked => None,
        }
    }

    #[wasm_test]
    fn each_kind_turns_on_and_off_through_its_own_domain() {
        let cases = [
            (Kind::Automation, "automation"),
            (Kind::InputBoolean, "input_boolean"),
            (Kind::Remote, "remote"),
            (Kind::Siren, "siren"),
            (Kind::Switch, "switch"),
        ];
        for (kind, domain) in cases {
            assert_eq!(
                fired(&tile(kind, at(Position::Off))),
                Some((domain, "turn_on")),
                "{kind:?}"
            );
            assert_eq!(
                fired(&tile(kind, at(Position::On))),
                Some((domain, "turn_off")),
                "{kind:?}"
            );
        }
    }

    #[wasm_test]
    fn with_neither_a_tap_nor_a_hold_set_it_shows_its_position_undimmed() {
        let gesture = Gesture::new(OnTap::Nothing, None);
        let tile = Tile::new(
            Kind::Switch,
            &at(Position::On),
            &fixtures::gate(&Pending::default(), &gesture, Reach::Ready),
        );
        assert_eq!(tile.card_action(), CardAction::None);
        assert_eq!(tile.card.line, Some(Line::State("On".to_owned())));
        assert_eq!(tile.card.action, None, "a dimmed icon reads as unusable");
    }

    #[wasm_test]
    fn with_neither_a_tap_nor_a_hold_set_it_stays_undimmed_while_the_tile_cannot_act() {
        let gesture = Gesture::new(OnTap::Nothing, None);
        for reach in [Reach::Unavailable, Reach::Reconnecting] {
            let tile = Tile::new(
                Kind::Switch,
                &at(Position::On),
                &fixtures::gate(&Pending::default(), &gesture, reach),
            );
            assert_eq!(tile.card.action, None, "{reach:?}");
        }
    }

    #[wasm_test]
    fn an_entity_in_no_known_position_turns_on() {
        assert_eq!(
            fired(&tile(Kind::Switch, at(Position::Unknown))),
            Some(("switch", "turn_on"))
        );
    }

    #[wasm_test]
    fn a_flip_on_its_way_shows_the_position_asked_for() {
        let tile = flipping(Kind::Switch, Position::Off, true);
        assert_eq!(tile.card.line, Some(Line::State("On".to_owned())));
        assert_eq!(tile.card.color, Some(Accent::Amber));
        assert_eq!(tile.card.icon, Icon(&TOGGLE_SWITCH));
        assert_eq!(tile.card.disc, Disc::Tinted);
    }

    #[wasm_test]
    fn a_tap_during_a_flip_flips_back() {
        assert_eq!(
            fired(&flipping(Kind::Switch, Position::Off, true)),
            Some(("switch", "turn_off"))
        );
    }

    #[wasm_test]
    fn only_an_entity_that_is_on_wears_its_colour() {
        let color = |kind, position| tile(kind, at(position)).card.color;
        for kind in [
            Kind::Automation,
            Kind::InputBoolean,
            Kind::Remote,
            Kind::Switch,
        ] {
            assert_eq!(color(kind, Position::On), Some(Accent::Amber), "{kind:?}");
        }
        assert_eq!(color(Kind::Siren, Position::On), Some(Accent::Red));
        for kind in KINDS {
            assert_eq!(color(kind, Position::Off), None, "{kind:?}");
            assert_eq!(color(kind, Position::Unknown), None, "{kind:?}");
        }
    }

    #[wasm_test]
    fn the_icon_follows_the_position_and_a_switchs_device_class() {
        let icon = |kind, position, device_class| {
            tile(
                kind,
                State {
                    position,
                    device_class,
                },
            )
            .card
            .icon
        };
        let cases = [
            (
                Kind::Switch,
                None,
                &TOGGLE_SWITCH,
                &TOGGLE_SWITCH_OFF_OUTLINE,
            ),
            (
                Kind::Switch,
                Some(DeviceClass::Switch),
                &TOGGLE_SWITCH,
                &TOGGLE_SWITCH_OFF_OUTLINE,
            ),
            (
                Kind::Switch,
                Some(DeviceClass::Outlet),
                &POWER_PLUG,
                &POWER_PLUG_OFF,
            ),
            (
                Kind::InputBoolean,
                None,
                &TOGGLE_SWITCH,
                &TOGGLE_SWITCH_OFF_OUTLINE,
            ),
            (Kind::Automation, None, &ROBOT, &ROBOT_OFF),
            (Kind::Remote, None, &REMOTE, &REMOTE_OFF),
            (Kind::Siren, None, &BULLHORN, &BULLHORN),
        ];
        for (kind, class, on, off) in cases {
            assert_eq!(
                icon(kind, Position::On, class),
                Icon(on),
                "{kind:?} {class:?} on"
            );
            assert_eq!(
                icon(kind, Position::Off, class),
                Icon(off),
                "{kind:?} {class:?} off"
            );
            assert_eq!(
                icon(kind, Position::Unknown, class),
                Icon(off),
                "{kind:?} {class:?} unknown"
            );
        }
    }

    #[wasm_test]
    fn the_line_tells_the_position() {
        let line = |position| tile(Kind::Switch, at(position)).card.line;
        assert_eq!(line(Position::On), Some(Line::State("On".to_owned())));
        assert_eq!(line(Position::Off), Some(Line::State("Off".to_owned())));
        assert_eq!(
            line(Position::Unknown),
            Some(Line::State("Unknown".to_owned()))
        );
    }

    #[wasm_test]
    fn only_an_entity_that_is_on_keeps_the_disc_behind_its_icon() {
        let disc = |position| tile(Kind::Switch, at(position)).card.disc;
        assert_eq!(disc(Position::On), Disc::Tinted);
        assert_eq!(disc(Position::Off), Disc::Bare);
        assert_eq!(disc(Position::Unknown), Disc::Bare);
    }

    #[wasm_test]
    fn only_an_entity_in_no_known_position_is_marked_unknown() {
        let mark = |position| tile(Kind::Switch, at(position)).card.mark;
        assert_eq!(mark(Position::Unknown), Some(Mark::Unknown));
        assert_eq!(mark(Position::On), None);
        assert_eq!(mark(Position::Off), None);
    }

    #[wasm_test]
    fn only_on_and_off_read_as_a_known_position() {
        let position =
            |state| State::read(Kind::Switch, &Entity::reporting(state, vec![])).position;
        assert_eq!(position("on"), Position::On);
        assert_eq!(position("off"), Position::Off);
        for state in ["unknown", "unavailable", "On", ""] {
            assert_eq!(position(state), Position::Unknown, "{state:?}");
        }
    }

    #[wasm_test]
    fn only_a_switch_reads_a_device_class() {
        let outlet = Entity::tracked(
            "on",
            vec![("device_class", AttributeValue::Text("outlet".to_owned()))],
            switch::ATTRIBUTES,
        );
        assert_eq!(
            State::read(Kind::Switch, &outlet).device_class,
            Some(DeviceClass::Outlet)
        );
        assert_eq!(State::read(Kind::Siren, &outlet).device_class, None);
    }
}
