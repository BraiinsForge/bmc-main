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

//! Buttons, scenes and scripts: entities whose one action fires at a touch on the card.

use bmc_wasm_sdk::*;
use home_assistant::Entity;
use home_assistant::domain::{button, input_button, scene, script};

use super::{CardAction, Context, Presenter, card, card_action};
use crate::interaction::pending::{Request, Slot};
use crate::view::blocks::badge::Disc;
use crate::view::card::{Card, Line, StateDisplay};
use crate::view::style::{Accent, Icon};
use crate::view::touches::Touches;

pub const CARD_ACTION: Slot = Slot::new("card_action");

const BUTTON_POINTER: Svg = include_svg!("assets/mdi/button-pointer.svg");
const PALETTE: Svg = include_svg!("assets/mdi/palette.svg");
const SCRIPT_TEXT: Svg = include_svg!("assets/mdi/script-text.svg");
const SCRIPT_TEXT_PLAY: Svg = include_svg!("assets/mdi/script-text-play.svg");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Button,
    InputButton,
    Scene,
    Script,
}

impl Kind {
    /// When it was last pressed, activated or run, as Home Assistant's timestamp.
    fn last_used(self, entity: &Entity) -> Option<&str> {
        match self {
            Self::Button => button::last_pressed(entity),
            Self::InputButton => input_button::last_pressed(entity),
            Self::Scene => scene::last_activated(entity),
            Self::Script => script::last_triggered(entity),
        }
    }

    /// How long ago it was last used, as the frontend's tile tells it, such as "5 minutes ago";
    /// "Never pressed" for a button, as a scene or a script is activated rather than pressed.
    fn line(self, last_used: Option<SystemTime>) -> Line {
        last_used.map_or_else(
            || {
                Line::State(
                    match self {
                        Self::Button | Self::InputButton => "Never pressed",
                        Self::Scene | Self::Script => "Never activated",
                    }
                    .to_owned(),
                )
            },
            Line::Since,
        )
    }

    /// While a firing in `state` waits for Home Assistant's answer, before the line can tell what came of it.
    fn firing_line(self, state: &State) -> &'static str {
        match (self, state) {
            (Self::Script, State::Running) => "Stopping…",
            (Self::Button | Self::InputButton, _) => "Pressing…",
            (Self::Scene | Self::Script, _) => "Activating…",
        }
    }

    /// The frontend's default icon for its domain.
    pub(super) fn icon(self) -> Icon {
        match self {
            Self::Button | Self::InputButton => Icon(&BUTTON_POINTER),
            Self::Scene => Icon(&PALETTE),
            Self::Script => Icon(&SCRIPT_TEXT),
        }
    }

    /// What firing it in `state` asks of Home Assistant:
    /// a running script stops, as the frontend's toggle turns off a script that is on.
    #[must_use]
    pub fn request(self, state: &State) -> Request {
        let call = match (self, state) {
            (Self::Script, State::Running) => script::turn_off(),
            (Self::Script, State::Idle { .. }) => script::turn_on(),
            (Self::Button, _) => button::press(),
            (Self::InputButton, _) => input_button::press(),
            (Self::Scene, _) => scene::turn_on(),
        };
        Request::no_value(CARD_ACTION, call)
    }

    /// The frontend colours a button or a scene in its icon colour, and a script only while it runs.
    fn color(self) -> Option<Accent> {
        match self {
            Self::Button | Self::InputButton | Self::Scene => Some(Accent::Neutral),
            Self::Script => None,
        }
    }
}

/// What the tile reads of a button, a scene or a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle {
        last_used: Option<SystemTime>,
    },
    /// A script mid-run.
    Running,
}

impl State {
    #[must_use]
    pub fn read(kind: Kind, entity: &Entity) -> Self {
        if kind == Kind::Script && script::running(entity) {
            return Self::Running;
        }
        Self::Idle {
            last_used: kind
                .last_used(entity)
                .and_then(parse_datetime)
                .map(|unix_secs| SystemTime { unix_secs }),
        }
    }
}

/// One frame of a momentary tile: the card is the button.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    card: Card,
    card_action: CardAction,
}

impl Tile {
    #[must_use]
    pub fn new(kind: Kind, state: &State, context: &Context) -> Self {
        let mut display = match *state {
            State::Idle { last_used } => StateDisplay {
                line: Some(kind.line(last_used)),
                color: kind.color(),
                icon: kind.icon(),
                disc: Disc::Tinted,
                mark: None,
            },
            State::Running => StateDisplay {
                line: Some(Line::State("Running".to_owned())),
                color: Some(Accent::Amber),
                icon: Icon(&SCRIPT_TEXT_PLAY),
                disc: Disc::Tinted,
                mark: None,
            },
        };
        let firing = context.pending.asked(CARD_ACTION);
        if firing {
            display.line = Some(Line::State(kind.firing_line(state).to_owned()));
        }
        let card_action = card_action(context, !firing, kind.request(state));
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

    /// It has no blocks: the card is its one action.
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
    use crate::fixtures::gate as context;
    use crate::interaction::gesture::{Gesture, OnTap};
    use crate::interaction::pending::Pending;
    use crate::present::Reach;
    use crate::view::card::Line;
    use bmc_wasm_sdk::wasm_test;
    use home_assistant::CallId;

    const NEVER_USED: State = State::Idle { last_used: None };

    fn tile(kind: Kind, state: &State, reach: Reach, gesture: &Gesture) -> Tile {
        Tile::new(kind, state, &context(&Pending::default(), gesture, reach))
    }

    fn tap() -> Gesture {
        Gesture::new(OnTap::Fires, None)
    }

    fn fired(tile: &Tile) -> Option<Request> {
        match tile.card_action() {
            CardAction::Ready(request) => Some(request),
            CardAction::None | CardAction::Blocked => None,
        }
    }

    #[wasm_test]
    fn each_kind_fires_through_its_own_service() {
        let cases = [
            (Kind::Button, ("button", "press")),
            (Kind::InputButton, ("input_button", "press")),
            (Kind::Scene, ("scene", "turn_on")),
            (Kind::Script, ("script", "turn_on")),
        ];
        for (kind, (domain, service)) in cases {
            let request = fired(&tile(kind, &NEVER_USED, Reach::Ready, &tap()))
                .expect("BUG: an idle entity fires");
            assert_eq!(
                (request.call().domain, request.call().service),
                (domain, service),
                "{kind:?}"
            );
        }
    }

    #[wasm_test]
    fn a_running_script_stops() {
        let request = fired(&tile(Kind::Script, &State::Running, Reach::Ready, &tap()))
            .expect("BUG: a running script takes a tap");
        assert_eq!(
            (request.call().domain, request.call().service),
            ("script", "turn_off")
        );
    }

    #[wasm_test]
    fn an_unavailable_entity_does_not_fire() {
        assert_eq!(
            fired(&tile(Kind::Script, &NEVER_USED, Reach::Unavailable, &tap())),
            None
        );
    }

    #[wasm_test]
    fn a_firing_on_its_way_takes_no_other() {
        let mut pending = Pending::default();
        let gesture = tap();
        pending.submit(Kind::Button.request(&NEVER_USED), |_| {
            Ok(CallId::from_raw(1))
        });
        let tile = Tile::new(
            Kind::Button,
            &NEVER_USED,
            &context(&pending, &gesture, Reach::Ready),
        );
        assert_eq!(fired(&tile), None);
    }

    #[wasm_test]
    fn a_firing_on_its_way_says_so_in_place_of_the_last_use() {
        let gesture = tap();
        let cases = [
            (Kind::Button, NEVER_USED, "Pressing…"),
            (Kind::Scene, NEVER_USED, "Activating…"),
            (Kind::Script, NEVER_USED, "Activating…"),
            (Kind::Script, State::Running, "Stopping…"),
        ];
        for (kind, state, line) in cases {
            let mut pending = Pending::default();
            pending.submit(kind.request(&state), |_| Ok(CallId::from_raw(1)));
            let tile = Tile::new(kind, &state, &context(&pending, &gesture, Reach::Ready));
            assert_eq!(
                tile.card.line,
                Some(Line::State(line.to_owned())),
                "{kind:?}"
            );
        }
    }

    #[wasm_test]
    fn with_neither_a_tap_nor_a_hold_set_it_does_not_fire() {
        let neither = Gesture::new(OnTap::Nothing, None);
        assert_eq!(
            fired(&tile(Kind::Button, &NEVER_USED, Reach::Ready, &neither)),
            None
        );
    }

    #[wasm_test]
    fn a_script_wears_a_colour_only_while_running() {
        let color = |kind, state: &State, reach| tile(kind, state, reach, &tap()).card.color;
        assert_eq!(
            color(Kind::Scene, &NEVER_USED, Reach::Ready),
            Some(Accent::Neutral)
        );
        assert_eq!(color(Kind::Button, &NEVER_USED, Reach::Unavailable), None);
        assert_eq!(color(Kind::Script, &NEVER_USED, Reach::Ready), None);
        assert_eq!(
            color(Kind::Script, &State::Running, Reach::Ready),
            Some(Accent::Amber)
        );
    }

    #[wasm_test]
    fn the_line_tells_when_it_was_last_used() {
        let line = |kind, state: &State| tile(kind, state, Reach::Ready, &tap()).card.line;
        let pressed = SystemTime {
            unix_secs: 1_794_564_000,
        };
        assert_eq!(
            line(
                Kind::Button,
                &State::Idle {
                    last_used: Some(pressed)
                }
            ),
            Some(Line::Since(pressed))
        );
        assert_eq!(
            line(Kind::Button, &NEVER_USED),
            Some(Line::State("Never pressed".to_owned()))
        );
        assert_eq!(
            line(Kind::Scene, &NEVER_USED),
            Some(Line::State("Never activated".to_owned()))
        );
        assert_eq!(
            line(Kind::Script, &State::Running),
            Some(Line::State("Running".to_owned()))
        );
    }

    #[wasm_test]
    fn the_last_use_is_the_time_home_assistant_reports() {
        let pressed = Entity::reporting("2026-11-13T10:00:00+00:00", vec![]);
        assert_eq!(
            State::read(Kind::Button, &pressed),
            State::Idle {
                last_used: Some(SystemTime {
                    unix_secs: 1_794_564_000
                })
            }
        );
        assert_eq!(
            State::read(Kind::Button, &Entity::reporting("unknown", vec![])),
            NEVER_USED
        );
    }

    #[wasm_test]
    fn only_a_script_reads_as_running() {
        let running = Entity::reporting("on", vec![]);
        assert_eq!(State::read(Kind::Script, &running), State::Running);
        assert!(matches!(
            State::read(Kind::Scene, &running),
            State::Idle { .. }
        ));
    }
}
