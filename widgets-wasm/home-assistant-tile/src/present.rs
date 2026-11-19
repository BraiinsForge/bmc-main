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

//! Each Home Assistant domain's tile, one module per group of domains:
//! the blocks it shows, and what a touch on them asks of Home Assistant.
//!
//! The crate README describes a group module's shape.

pub mod momentary;

use bmc_wasm_sdk::*;
use home_assistant::domain::{Domain, UNAVAILABLE};
use home_assistant::{CallFailure, Entity, Presence, Status, is_entity_id};

use crate::interaction::gesture::{Availability, Gesture};
use crate::interaction::pending::{Pending, Request};
use crate::view::blocks::badge::Mark;
use crate::view::card::{Card, Line, StateDisplay};
use crate::view::style::{Accent, Icon};
use crate::view::touches::Touches;

const RECONNECTING: &str = "Reconnecting…";
/// Neither the tap nor the hold setting fires the card action.
const NO_ACTION: &str = "No action set";
const UNAVAILABLE_LINE: &str = "Unavailable";
const LOADING: &str = "Loading…";
/// A request the client could not send, told on the line for a few seconds.
pub const NOT_CONNECTED: &str = "Not connected";
/// The tile's name with neither a name nor an entity ID set.
pub const UNNAMED: &str = "Home Assistant";

const ALERT_CIRCLE_OUTLINE: Svg = include_svg!("assets/mdi/alert-circle-outline.svg");

/// Why the tile shows no entity. The user fixes most, in the widget's settings, the account or Home Assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    NoEntity,
    InvalidEntityId,
    UnsupportedDomain,
    NoAccount,
    TokenRefused,
    /// The host connects only to `http://` addresses.
    NeedsHttp,
    TooOld,
    /// Home Assistant has not answered since the tile started: a wrong address, or it is down.
    /// The client keeps trying.
    CantConnect,
    /// Nothing the user set: Home Assistant sent more than the host takes in one message,
    /// or the subscription could not be made or was refused.
    Communication,
    EntityNotFound,
}

impl Problem {
    pub const ALL: [Self; 10] = [
        Self::NoEntity,
        Self::InvalidEntityId,
        Self::UnsupportedDomain,
        Self::NoAccount,
        Self::TokenRefused,
        Self::NeedsHttp,
        Self::TooOld,
        Self::CantConnect,
        Self::Communication,
        Self::EntityNotFound,
    ];

    /// One line, short enough for a 1x1 tile.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::NoEntity => "No entity set",
            Self::InvalidEntityId => "Invalid entity ID",
            Self::UnsupportedDomain => "Type not supported yet",
            Self::NoAccount => "No account set",
            Self::TokenRefused => "Token refused",
            Self::NeedsHttp => "Use an http:// address",
            Self::TooOld => "Home Assistant too old",
            Self::CantConnect => "Can't connect, retrying…",
            Self::Communication => "Communication error",
            Self::EntityNotFound => "Entity not found",
        }
    }

    /// Why the client cannot show any entity, if it cannot.
    /// A client that is only disconnected keeps trying: it is a problem only to a tile with nothing to show yet.
    #[must_use]
    pub fn of_status(status: Status) -> Option<Self> {
        match status {
            Status::Connecting | Status::Ready | Status::Disconnected => None,
            Status::NotConfigured => Some(Self::NoAccount),
            Status::TokenRejected => Some(Self::TokenRefused),
            Status::UnsupportedAddress => Some(Self::NeedsHttp),
            Status::Unsupported => Some(Self::TooOld),
            Status::MessageTooLarge => Some(Self::Communication),
        }
    }
}

/// How far the tile has got following its entity.
#[derive(Debug, Clone, Copy)]
pub enum Following<'a> {
    /// The subscription could not be made, or Home Assistant refused it.
    Refused,
    /// What the subscription knows of the entity; none before it has said anything.
    Open(Option<&'a Entity>),
}

/// What a frame shows of an entity the tile has a group for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Showing<'a> {
    Problem(Problem),
    /// The group's skeleton, while the entity is on its way.
    Loading,
    Entity(&'a Entity),
}

/// What to show, given the client's `status` and how far `following` has got:
/// a problem with the client comes first, then one with the subscription, then the entity as it stands.
/// An entity seen before the connection dropped keeps its last values; one never seen cannot be shown.
#[must_use]
pub fn showing(status: Option<Status>, following: Following<'_>) -> Showing<'_> {
    if let Some(problem) = status.and_then(Problem::of_status) {
        return Showing::Problem(problem);
    }
    let Following::Open(entity) = following else {
        return Showing::Problem(Problem::Communication);
    };
    match entity.map(|entity| (entity, entity.presence)) {
        Some((entity, Presence::Present | Presence::Stale)) => Showing::Entity(entity),
        Some((_, Presence::Missing)) => Showing::Problem(Problem::EntityNotFound),
        None | Some((_, Presence::Awaiting)) if status == Some(Status::Disconnected) => {
            Showing::Problem(Problem::CantConnect)
        }
        None | Some((_, Presence::Awaiting)) => Showing::Loading,
    }
}

/// What the line tells of a call that failed, for a few seconds.
/// Kept short for a 1x1 tile's line; Home Assistant's own message, often a sentence or more, goes to the log.
#[must_use]
pub fn failure_notice(failure: &CallFailure) -> &'static str {
    match failure {
        CallFailure::Rejected(_) => "Home Assistant refused",
        CallFailure::TimedOut => "No answer",
        CallFailure::Disconnected => "Connection lost",
    }
}

/// The group that shows `entity_id`, or why there is none, before Home Assistant is asked.
pub fn group_of(entity_id: Option<&str>) -> Result<Group, Problem> {
    let entity_id = entity_id.ok_or(Problem::NoEntity)?;
    if !is_entity_id(entity_id) {
        return Err(Problem::InvalidEntityId);
    }
    Domain::of(entity_id)
        .and_then(Group::of)
        .ok_or(Problem::UnsupportedDomain)
}

/// The skeleton of the tile `group` draws, while its entity is on its way.
#[must_use]
pub fn loading(group: Group, name: &str, room: Room) -> Card {
    placeholder(
        name,
        Line::State(LOADING.to_owned()),
        None,
        group.icon(),
        room,
    )
}

/// Why the tile shows no entity, with a warning in place of the entity's icon.
#[must_use]
pub fn problem(problem: Problem, name: &str, room: Room) -> Card {
    let line = Line::Notice(problem.text().to_owned());
    let icon = Icon(&ALERT_CIRCLE_OUTLINE);
    placeholder(name, line, Some(Accent::Warning), icon, room)
}

fn placeholder(name: &str, line: Line, color: Option<Accent>, icon: Icon, room: Room) -> Card {
    Card {
        name: name.to_owned(),
        line: Some(line),
        color,
        icon,
        action: None,
        hold: None,
        hint: None,
        mark: None,
        width: room.width,
        height: room.height,
        size: room.size,
    }
}

/// Whether the tile can act on the entity now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Ready,
    /// The connection dropped: the values are the last ones seen, and nothing can be sent.
    Reconnecting,
    /// Home Assistant cannot reach the entity.
    Unavailable,
}

impl Reach {
    #[must_use]
    pub fn of(entity: &Entity) -> Self {
        match (entity.presence, entity.state.as_deref()) {
            (Presence::Stale, _) => Self::Reconnecting,
            (
                Presence::Awaiting | Presence::Present | Presence::Missing,
                Some(UNAVAILABLE) | None,
            ) => Self::Unavailable,
            (Presence::Awaiting | Presence::Present | Presence::Missing, Some(_)) => Self::Ready,
        }
    }

    /// Whether the tile's blocks and its card action can act now.
    #[must_use]
    pub fn can_act(self) -> bool {
        self == Self::Ready
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Room {
    pub width: f32,
    pub height: f32,
    pub size: SizeVariant,
}

/// What a frame builds a tile from besides the entity.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub pending: &'a Pending,
    pub gesture: &'a Gesture,
    pub name: &'a str,
    /// What became of something the user did, told in place of the state for a while.
    pub notice: Option<&'a str>,
    pub reach: Reach,
    pub room: Room,
}

impl Context<'_> {
    #[must_use]
    pub fn can_act(&self) -> bool {
        self.reach.can_act()
    }
}

/// What a tap or a hold on the card does now.
#[derive(Debug, Clone, PartialEq)]
pub enum CardAction {
    /// The domain has no card action: the icon is not dimmed.
    None,
    /// It has one that cannot fire now.
    Blocked,
    Ready(Request),
}

impl CardAction {
    #[must_use]
    pub fn availability(&self) -> Option<Availability> {
        match self {
            Self::None => None,
            Self::Blocked => Some(Availability::Blocked),
            Self::Ready(_) => Some(Availability::Ready),
        }
    }
}

/// `request` as the card action, where it can fire now: the tile can act,
/// a tap or a hold is set to fire it, and the domain is `ready`, such as a script that is not running.
#[must_use]
pub fn card_action(context: &Context, ready: bool, request: Request) -> CardAction {
    if ready && context.can_act() && context.gesture.can_fire() {
        CardAction::Ready(request)
    } else {
        CardAction::Blocked
    }
}

/// The card telling `display`, for an entity whose card action is `action`.
/// It tells why the tile cannot act in place of the state, and drops the state's colour meanwhile.
#[must_use]
pub fn card(context: &Context, display: StateDisplay, action: &CardAction) -> Card {
    let availability = action.availability();
    let line = match (context.reach, context.notice) {
        (Reach::Reconnecting, _) => Some(Line::State(RECONNECTING.to_owned())),
        (Reach::Ready | Reach::Unavailable, Some(notice)) => Some(Line::Notice(notice.to_owned())),
        (Reach::Unavailable, None) => Some(Line::State(UNAVAILABLE_LINE.to_owned())),
        (Reach::Ready, None) if availability.is_some() && !context.gesture.can_fire() => {
            Some(Line::Notice(NO_ACTION.to_owned()))
        }
        (Reach::Ready, None) => display.line,
    };
    Card {
        name: context.name.to_owned(),
        line,
        color: display.color.filter(|_| context.can_act()),
        icon: display.icon,
        action: availability,
        hold: context.gesture.progress(),
        hint: context
            .gesture
            .hold_only_ms()
            .filter(|_| availability == Some(Availability::Ready))
            .map(|hold_ms| fmt!("Hold {} s", hold_ms.get() / 1000)),
        mark: (context.reach == Reach::Unavailable).then_some(Mark::Unavailable),
        width: context.room.width,
        height: context.room.height,
        size: context.room.size,
    }
}

/// One frame of a group's tile, built from what Home Assistant reports and what the user asked for.
pub trait Presenter {
    /// The whole tile, with the part a finger is on drawn pressed:
    /// [`Card::centred`], or a layout of its own on [`Card::surface`].
    fn view(&self, touches: &Touches) -> Node;

    /// What a finger lifted from one of its blocks asks of Home Assistant.
    fn touched(&self, touches: &Touches) -> Option<Request>;

    /// What a tap or a hold on the card does now, as [`card_action`] decides it.
    fn card_action(&self) -> CardAction;
}

/// The groups of domains the tile shows, each with a tile of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Momentary(momentary::Kind),
}

impl Group {
    /// The group showing `domain`, if the tile shows it.
    #[must_use]
    pub fn of(domain: Domain) -> Option<Self> {
        match domain {
            Domain::Button => Some(Self::Momentary(momentary::Kind::Button)),
            Domain::InputButton => Some(Self::Momentary(momentary::Kind::InputButton)),
            Domain::Scene => Some(Self::Momentary(momentary::Kind::Scene)),
            Domain::Script => Some(Self::Momentary(momentary::Kind::Script)),
        }
    }

    /// Its domain's icon at rest.
    #[must_use]
    pub fn icon(self) -> Icon {
        match self {
            Self::Momentary(kind) => kind.icon(),
        }
    }

    #[must_use]
    pub fn tile(self, entity: &Entity, context: &Context) -> Box<dyn Presenter> {
        match self {
            Self::Momentary(kind) => Box::new(momentary::Tile::new(
                kind,
                &momentary::State::read(kind, entity),
                context,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::gesture::OnTap;
    use crate::interaction::pending::Slot;
    use crate::view::icons::CHECK;
    use crate::view::style::{Accent, Icon};
    use bmc_wasm_sdk::wasm_test;
    use home_assistant::ServiceCall;
    use home_assistant::domain::UNKNOWN;
    use std::num::NonZeroU32;

    const HOLD: Option<NonZeroU32> = NonZeroU32::new(2_000);

    fn context<'a>(pending: &'a Pending, gesture: &'a Gesture, reach: Reach) -> Context<'a> {
        Context {
            pending,
            gesture,
            name: "Gate",
            notice: None,
            reach,
            room: Room {
                width: 638.0,
                height: 238.0,
                size: SizeVariant::Medium,
            },
        }
    }

    fn request() -> Request {
        Request::no_value(Slot::new("press"), ServiceCall::new("button", "press"))
    }

    #[wasm_test]
    fn the_card_action_fires_only_while_the_tile_can_act_and_a_touch_is_set_to_fire_it() {
        let pending = Pending::default();
        let tap = Gesture::new(OnTap::Fires, None);
        let fires = |gesture: &Gesture, reach, ready| {
            matches!(
                card_action(&context(&pending, gesture, reach), ready, request()),
                CardAction::Ready(_)
            )
        };
        assert!(fires(&tap, Reach::Ready, true));
        assert!(!fires(&tap, Reach::Ready, false), "the domain is not ready");
        assert!(!fires(&tap, Reach::Reconnecting, true));
        assert!(!fires(&tap, Reach::Unavailable, true));
        assert!(
            !fires(&Gesture::new(OnTap::Nothing, None), Reach::Ready, true),
            "neither a tap nor a hold is set"
        );
    }

    #[wasm_test]
    fn only_a_hold_that_alone_fires_the_card_action_is_hinted() {
        let pending = Pending::default();
        let hint = |gesture: Gesture| {
            let context = context(&pending, &gesture, Reach::Ready);
            let action = card_action(&context, true, request());
            let display = StateDisplay {
                line: None,
                color: None,
                icon: Icon(&CHECK),
            };
            card(&context, display, &action).hint
        };
        let hold_only = Gesture::new(OnTap::Nothing, HOLD);
        assert_eq!(hint(hold_only.clone()).as_deref(), Some("Hold 2 s"));
        assert_eq!(
            hint(Gesture::new(OnTap::Fires, HOLD)),
            None,
            "a tap fires it too"
        );
        let blocked = context(&pending, &hold_only, Reach::Unavailable);
        let display = StateDisplay {
            line: None,
            color: None,
            icon: Icon(&CHECK),
        };
        assert_eq!(
            card(&blocked, display, &CardAction::Blocked).hint,
            None,
            "a hold that cannot fire now"
        );
    }

    #[wasm_test]
    fn an_unavailable_or_stale_entity_cannot_be_acted_on() {
        assert_eq!(Reach::of(&Entity::reporting("on", vec![])), Reach::Ready);
        assert_eq!(Reach::of(&Entity::reporting(UNKNOWN, vec![])), Reach::Ready);
        assert_eq!(
            Reach::of(&Entity::reporting(UNAVAILABLE, vec![])),
            Reach::Unavailable
        );
        let mut stale = Entity::reporting(UNAVAILABLE, vec![]);
        stale.presence = Presence::Stale;
        assert_eq!(Reach::of(&stale), Reach::Reconnecting);
    }

    #[wasm_test]
    fn an_unavailable_entity_says_so_without_its_colour() {
        let (pending, tap) = (Pending::default(), Gesture::new(OnTap::Fires, None));
        let context = context(&pending, &tap, Reach::Unavailable);
        let display = StateDisplay {
            line: Some(Line::State("Never pressed".to_owned())),
            color: Some(Accent::Neutral),
            icon: Icon(&CHECK),
        };
        let card = card(&context, display, &CardAction::Blocked);
        assert_eq!(card.line, Some(Line::State(UNAVAILABLE_LINE.to_owned())));
        assert_eq!(card.color, None);
    }

    #[wasm_test]
    fn only_an_unavailable_entity_is_marked() {
        let (pending, tap) = (Pending::default(), Gesture::new(OnTap::Fires, None));
        let mark = |reach| {
            let display = StateDisplay {
                line: None,
                color: None,
                icon: Icon(&CHECK),
            };
            card(
                &context(&pending, &tap, reach),
                display,
                &CardAction::Blocked,
            )
            .mark
        };
        assert_eq!(mark(Reach::Unavailable), Some(Mark::Unavailable));
        assert_eq!(mark(Reach::Ready), None);
        assert_eq!(mark(Reach::Reconnecting), None);
    }

    fn line(gesture: &Gesture, reach: Reach, action: &CardAction) -> Option<Line> {
        let pending = Pending::default();
        let display = StateDisplay {
            line: Some(Line::State("Never pressed".to_owned())),
            color: None,
            icon: Icon(&CHECK),
        };
        card(&context(&pending, gesture, reach), display, action).line
    }

    #[wasm_test]
    fn a_card_action_neither_a_tap_nor_a_hold_fires_asks_for_one() {
        let neither = Gesture::new(OnTap::Nothing, None);
        assert_eq!(
            line(&neither, Reach::Ready, &CardAction::Blocked),
            Some(Line::Notice(NO_ACTION.to_owned()))
        );
        assert_eq!(
            line(&neither, Reach::Ready, &CardAction::None),
            Some(Line::State("Never pressed".to_owned())),
            "an entity with no card action needs none set"
        );
    }

    #[wasm_test]
    fn reconnecting_reads_as_a_state_not_as_a_notice() {
        let tap = Gesture::new(OnTap::Fires, None);
        assert_eq!(
            line(&tap, Reach::Reconnecting, &CardAction::Blocked),
            Some(Line::State(RECONNECTING.to_owned()))
        );
    }

    #[wasm_test]
    fn an_entity_id_finds_its_group_or_says_what_is_wrong_with_it() {
        assert_eq!(group_of(None), Err(Problem::NoEntity));
        assert_eq!(group_of(Some("gate")), Err(Problem::InvalidEntityId));
        assert_eq!(group_of(Some("Button.Gate")), Err(Problem::InvalidEntityId));
        assert_eq!(
            group_of(Some("light.kitchen")),
            Err(Problem::UnsupportedDomain)
        );
        assert_eq!(
            group_of(Some("button.gate")),
            Ok(Group::Momentary(momentary::Kind::Button))
        );
    }

    #[wasm_test]
    fn a_client_that_only_lost_its_connection_is_no_problem_yet() {
        let problems = [
            (Status::Connecting, None),
            (Status::Ready, None),
            (Status::Disconnected, None),
            (Status::NotConfigured, Some(Problem::NoAccount)),
            (Status::TokenRejected, Some(Problem::TokenRefused)),
            (Status::UnsupportedAddress, Some(Problem::NeedsHttp)),
            (Status::Unsupported, Some(Problem::TooOld)),
            (Status::MessageTooLarge, Some(Problem::Communication)),
        ];
        for (status, problem) in problems {
            assert_eq!(Problem::of_status(status), problem, "{status:?}");
        }
    }

    #[wasm_test]
    fn a_problem_with_the_client_or_the_subscription_comes_before_the_entity() {
        let present = Entity::reporting("on", vec![]);
        let with = |presence| {
            let mut entity = present.clone();
            entity.presence = presence;
            entity
        };
        let (stale, missing, awaiting) = (
            with(Presence::Stale),
            with(Presence::Missing),
            with(Presence::Awaiting),
        );
        let ready = Some(Status::Ready);
        let dropped = Some(Status::Disconnected);
        let cases = [
            (
                Some(Status::TokenRejected),
                Following::Open(Some(&present)),
                Showing::Problem(Problem::TokenRefused),
            ),
            (
                ready,
                Following::Refused,
                Showing::Problem(Problem::Communication),
            ),
            (ready, Following::Open(None), Showing::Loading),
            (
                Some(Status::Connecting),
                Following::Open(Some(&awaiting)),
                Showing::Loading,
            ),
            (
                dropped,
                Following::Open(None),
                Showing::Problem(Problem::CantConnect),
            ),
            (
                dropped,
                Following::Open(Some(&awaiting)),
                Showing::Problem(Problem::CantConnect),
            ),
            (
                ready,
                Following::Open(Some(&missing)),
                Showing::Problem(Problem::EntityNotFound),
            ),
            (
                ready,
                Following::Open(Some(&present)),
                Showing::Entity(&present),
            ),
            (
                dropped,
                Following::Open(Some(&stale)),
                Showing::Entity(&stale),
            ),
        ];
        for (status, following, shown) in cases {
            assert_eq!(
                showing(status, following),
                shown,
                "{status:?}, {following:?}"
            );
        }
    }

    #[wasm_test]
    fn every_problem_is_listed_once() {
        // A variant left out of this match fails to compile; one left out of `ALL` fails here.
        let position = |problem| match problem {
            Problem::NoEntity => 0,
            Problem::InvalidEntityId => 1,
            Problem::UnsupportedDomain => 2,
            Problem::NoAccount => 3,
            Problem::TokenRefused => 4,
            Problem::NeedsHttp => 5,
            Problem::TooOld => 6,
            Problem::CantConnect => 7,
            Problem::Communication => 8,
            Problem::EntityNotFound => 9,
        };
        for (at, problem) in Problem::ALL.into_iter().enumerate() {
            assert_eq!(position(problem), at, "{problem:?}");
        }
    }

    fn room() -> Room {
        Room {
            width: 317.0,
            height: 238.0,
            size: SizeVariant::Small,
        }
    }

    #[wasm_test]
    fn a_problem_replaces_the_entitys_icon_with_a_warning_and_says_what_to_fix() {
        let card = problem(Problem::TokenRefused, "Gate", room());
        assert_eq!(
            card.line,
            Some(Line::Notice(Problem::TokenRefused.text().to_owned()))
        );
        assert_eq!(card.color, Some(Accent::Warning));
        assert_ne!(card.icon, Group::Momentary(momentary::Kind::Button).icon());
        assert_eq!(card.action, None, "a touch on a problem fires nothing");
    }

    #[wasm_test]
    fn loading_draws_the_skeleton_of_the_tile_on_its_way() {
        let group = Group::Momentary(momentary::Kind::Scene);
        let card = loading(group, "Lounge", room());
        assert_eq!(card.icon, group.icon());
        assert_eq!(card.line, Some(Line::State(LOADING.to_owned())));
        assert_eq!(
            card.color, None,
            "grey: nothing is wrong, nothing is live yet"
        );
    }
}
