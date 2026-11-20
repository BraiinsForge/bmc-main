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

//! Home Assistant Tile widget: one entity as a card, with the controls its size has room for.

pub mod fixtures;
pub mod interaction;
#[cfg(target_arch = "wasm32")]
mod manifest_params;
pub mod present;
pub mod view;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use std::cell::RefCell;
    use std::num::NonZeroU32;

    use bmc_wasm_sdk::timer::{self, Timer, TimerCallback};
    use bmc_wasm_sdk::*;
    use home_assistant::domain::{Domain, FRIENDLY_NAME};
    use home_assistant::{
        CallFailure, CallId, Client, Event, Reply, RequestError, ServiceCall, Status, Subscription,
    };

    use crate::interaction::gesture::{Availability, Gesture, OnTap, Step, Touch};
    use crate::interaction::notice::{FAILURE_MS, Notice, RELEASED_EARLY_MS};
    use crate::interaction::pending::{Answered, Pending, Request, Slot, Submitted};
    use crate::manifest_params::{Params, TapAction};
    use crate::present::{
        self, CardAction, Context, Following, Group, Precision, Presenter, Problem, Reach, Room,
        Showing, UNNAMED,
    };
    use crate::view::card::{self, Card};
    use crate::view::touches::Touches;

    const SLOT: &str = "ha";
    /// How long the taps in a run must stop for before their change is sent: the shortest a timer runs.
    const QUIET_MS: u32 = 1_000;
    /// The frontend's toggle waits this long after its call for a report before showing the reported position;
    /// a value asked on the tile waits as long.
    const GRACE_MS: u32 = 2_000;
    /// A report from Home Assistant shows within this long, and a run of them is drawn once in it;
    /// one confirming a change made on the tile shows at once.
    const REPORT_FRAME_MS: u32 = 1_000;
    /// How long the LED strip previews the entity after the last change made on the tile.
    const LED_PREVIEW_MS: u32 = 3_000;
    /// Home Assistant cannot tell the tile that one entity's display precision changed
    /// without sending it every registry change in the installation, so the tile asks again this often.
    const PRECISION_REFRESH_MS: u32 = 5 * 60 * 1_000;

    struct Followed {
        entity_id: String,
        subscription: Subscription,
        subscribed: Subscribed,
        /// None for an entity shown without a display precision.
        precision: Option<PrecisionWatch>,
    }

    struct PrecisionWatch {
        /// The last one read, which stays shown while a refresh is on its way.
        shown: Precision,
        lookup: Option<CallId>,
        /// None where the host had no timer to spare, so the precision is looked up only on login and on saving.
        _refresh: Option<Timer>,
    }

    impl PrecisionWatch {
        fn start() -> Self {
            let refresh = timer::start(PRECISION_REFRESH_MS, on_precision_refresh);
            if refresh.is_none() {
                log_warn!("home-assistant-tile: no timer to refresh the display precision with");
            }
            Self {
                shown: Precision::Awaiting,
                lookup: None,
                _refresh: refresh,
            }
        }
    }

    /// What Home Assistant made of the subscription since the client last logged in.
    /// A refused one goes out again after the next login.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Subscribed {
        Open,
        Refused,
    }

    /// Everything a new entity starts over: replacing it drops the subscription, the timers and what is pending.
    struct State {
        followed: Option<Followed>,
        touches: Touches,
        pending: Pending,
        gesture: Gesture,
        notice: Notice,
        quiet: Option<Timer>,
        grace: Option<Timer>,
        preview: Option<Preview>,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                followed: None,
                touches: Touches::default(),
                pending: Pending::default(),
                gesture: Gesture::new(OnTap::Fires, None),
                notice: Notice::default(),
                quiet: None,
                grace: None,
                preview: None,
            }
        }
    }

    /// The LED strip showing the entity for a while after a change made on the tile.
    struct Preview {
        until: Timer,
        /// What the strip was last set to; none while it is dark.
        shown: Option<Color>,
    }

    thread_local! {
        static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) };
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// What a frame shows.
    enum Screen {
        /// Why there is no entity to show.
        Placeholder(Card),
        /// The skeleton of the tile `Group` draws, while its entity is on its way.
        Loading(Group, Card),
        Tile {
            presenter: Box<dyn Presenter>,
            reach: Reach,
        },
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let client = home_assistant::connect(SLOT, on_event)
            .expect("BUG: the widget starts no timer before its client");
        CLIENT.set(Some(client));
        follow();
        request_frame();
    }

    /// The host delivers touches only to a widget that exports this, and draws no frame for them by itself.
    #[unsafe(no_mangle)]
    pub extern "C" fn on_touch() {
        request_frame();
    }

    /// An entity not followed yet is followed afresh.
    /// Any other change leaves it, what was asked of it and a finger on the card alone.
    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let params = Params::current();
        let followed = STATE.with_borrow(|state| {
            state
                .followed
                .as_ref()
                .map(|followed| followed.entity_id.clone())
        });
        if followed.as_deref() == entity_id(&params) {
            let (tap, hold_ms) = settings(&params);
            STATE.with_borrow_mut(|state| state.gesture.set(tap, hold_ms));
            // Saving the settings shows a precision changed in Home Assistant without the 5-minute wait.
            look_up_precision();
        } else {
            follow();
        }
        resume();
    }

    /// The account may be another Home Assistant, whose precision the last one's must not stand in for.
    #[unsafe(no_mangle)]
    pub extern "C" fn on_credentials_update() {
        STATE.with_borrow_mut(|state| {
            if let Some(watch) = precision_watch(state) {
                watch.shown = Precision::Awaiting;
                watch.lookup = None;
            }
        });
        look_up_precision();
        resume();
    }

    fn resume() {
        CLIENT.with_borrow(|client| {
            if let Some(client) = client {
                client.resume();
            }
        });
        request_frame();
    }

    /// Follow the configured entity in place of the one followed before.
    fn follow() {
        let params = Params::current();
        let followed = entity_id(&params).and_then(|entity_id| {
            let domain = Domain::of(entity_id)?;
            let group = present::group_of(Some(entity_id)).ok()?;
            let mut attributes = vec![FRIENDLY_NAME];
            attributes.extend_from_slice(domain.attributes());
            let subscribed = CLIENT
                .with_borrow(|client| client.as_ref()?.subscribe(&[entity_id], &attributes).ok());
            if subscribed.is_none() {
                log_warn!("home-assistant-tile: could not subscribe to {}", entity_id);
            }
            Some(Followed {
                entity_id: entity_id.to_owned(),
                subscription: subscribed?,
                subscribed: Subscribed::Open,
                precision: group.reads_precision().then(PrecisionWatch::start),
            })
        });
        let (tap, hold_ms) = settings(&params);
        let (mut gesture, previewing) =
            STATE.with_borrow(|state| (state.gesture.clone(), state.preview.is_some()));
        gesture.set_for_another_action(tap, hold_ms);
        if previewing {
            led::stop();
        }
        STATE.set(State {
            followed,
            gesture,
            ..State::default()
        });
        look_up_precision();
    }

    fn precision_watch(state: &mut State) -> Option<&mut PrecisionWatch> {
        state.followed.as_mut()?.precision.as_mut()
    }

    fn on_precision_refresh(_timer: &Timer, _elapsed_periods: u32) {
        look_up_precision();
    }

    /// Ask for the followed entity's display precision, keeping the last one shown meanwhile.
    /// A client that cannot send now asks again once it has logged in.
    fn look_up_precision() {
        let Some(entity_id) = STATE.with_borrow(|state| {
            state
                .followed
                .as_ref()
                .filter(|followed| followed.precision.is_some())
                .map(|followed| followed.entity_id.clone())
        }) else {
            return;
        };
        let lookup = CLIENT.with_borrow(|client| {
            client
                .as_ref()
                .ok_or(RequestError::NotConnected)
                .and_then(|client| client.display_precision(&entity_id))
        });
        let lookup = match lookup {
            Ok(call) => Some(call),
            Err(RequestError::NotConnected) => None,
            Err(error) => {
                log_warn!(
                    "home-assistant-tile: the display precision lookup could not be sent: {}",
                    error.name()
                );
                None
            }
        };
        STATE.with_borrow_mut(|state| {
            if let Some(watch) = precision_watch(state) {
                watch.lookup = lookup;
            }
        });
    }

    /// Whether `call` was the precision lookup, which takes its answer.
    fn precision_answered(call: CallId, result: Result<&Reply, &CallFailure>) -> bool {
        STATE.with_borrow_mut(|state| {
            let Some(watch) = precision_watch(state).filter(|watch| watch.lookup == Some(call))
            else {
                return false;
            };
            watch.lookup = None;
            match result {
                Err(CallFailure::Rejected(error)) => log_warn!(
                    "home-assistant-tile: the display precision lookup was refused ({}): {}",
                    error.code.as_str(),
                    error.message.as_str()
                ),
                Err(CallFailure::TimedOut) => {
                    log_warn!("home-assistant-tile: the display precision lookup got no answer");
                }
                Ok(_) | Err(CallFailure::Disconnected) => {}
            }
            watch.shown = watch.shown.answered(result);
            true
        })
    }

    /// What a tap does, and how long a hold takes to fire; none where holding fires nothing.
    fn settings(params: &Params) -> (OnTap, Option<NonZeroU32>) {
        let tap = match params.tap_action {
            TapAction::Toggle => OnTap::Fires,
            TapAction::None => OnTap::Nothing,
        };
        let seconds = u32::try_from(params.hold_seconds.as_manifest_value())
            .expect("BUG: the manifest's hold options are whole seconds, none negative");
        (tap, NonZeroU32::new(seconds * 1_000))
    }

    fn entity_id(params: &Params) -> Option<&str> {
        Some(params.entity_id.trim()).filter(|entity_id| !entity_id.is_empty())
    }

    fn on_event(_client: &Client, event: &Event) {
        match event {
            Event::EntityChanged { .. } => {
                if STATE.with_borrow_mut(|state| state.pending.reported()) {
                    request_frame();
                } else {
                    request_frame_after(REPORT_FRAME_MS);
                }
            }
            Event::CallFinished { call, result } => {
                if !precision_answered(*call, result.as_ref()) {
                    answered(*call, result.as_ref().map(|_| ()));
                }
                request_frame();
            }
            Event::StatusChanged(status) => {
                if *status == Status::Ready {
                    STATE.with_borrow_mut(|state| {
                        if let Some(followed) = &mut state.followed {
                            followed.subscribed = Subscribed::Open;
                        }
                    });
                    look_up_precision();
                }
                request_frame();
            }
            Event::SubscriptionFailed { subscription, .. } => {
                STATE.with_borrow_mut(|state| {
                    if let Some(followed) = &mut state.followed
                        && followed.subscription.id() == *subscription
                    {
                        followed.subscribed = Subscribed::Refused;
                    }
                });
                request_frame();
            }
        }
    }

    fn answered(call: CallId, result: Result<(), &CallFailure>) {
        let answered = STATE.with_borrow_mut(|state| state.pending.answered(call, result));
        match answered {
            Some(Answered::AwaitingReport) => {
                if !STATE.with_borrow_mut(|state| run_for(&mut state.grace, GRACE_MS, on_grace)) {
                    // Waiting with no grace to end it would leave a device that ignores the call shown as asked.
                    log_warn!(
                        "home-assistant-tile: no timer to wait for a report; showing the reported state"
                    );
                    STATE.with_borrow_mut(|state| state.pending.give_up());
                }
            }
            Some(Answered::Failed { slot, failure }) => {
                let why = match failure {
                    CallFailure::Rejected(error) => fmt!(
                        "Home Assistant refused it ({}): {}",
                        error.code.as_str(),
                        error.message.as_str()
                    ),
                    CallFailure::Disconnected => {
                        "the connection dropped before Home Assistant answered".to_owned()
                    }
                    CallFailure::TimedOut => "Home Assistant did not answer".to_owned(),
                };
                log_warn!(
                    "home-assistant-tile: {} failed: {}",
                    slot.name(),
                    why.as_str()
                );
                show_notice(present::failure_notice(failure).to_owned(), FAILURE_MS);
            }
            Some(Answered::Done) | None => {}
        }
    }

    fn on_grace(_timer: &Timer, _elapsed_periods: u32) {
        STATE.with_borrow_mut(|state| {
            state.grace = None;
            state.pending.give_up();
        });
        request_frame();
    }

    fn on_quiet(_timer: &Timer, _elapsed_periods: u32) {
        let unsent = CLIENT.with_borrow(|client| {
            STATE.with_borrow_mut(|state| {
                state.quiet = None;
                let Some(entity_id) = state
                    .followed
                    .as_ref()
                    .map(|followed| followed.entity_id.clone())
                else {
                    return Vec::new();
                };
                state
                    .pending
                    .send_waiting(|slot, call| send(client.as_ref(), &entity_id, slot, call))
            })
        });
        if let Some(&error) = unsent.first() {
            show_notice(present::unsent_notice(error).to_owned(), FAILURE_MS);
        }
        request_frame();
    }

    fn show_notice(text: String, shown_ms: u32) {
        STATE.with_borrow_mut(|state| state.notice.show(text, shown_ms));
        request_frame();
    }

    fn submit(request: Request) {
        let submitted = CLIENT.with_borrow(|client| {
            STATE.with_borrow_mut(|state| {
                let entity_id = state.followed.as_ref()?.entity_id.clone();
                // A notice outranks the state line, so one about an earlier request would hide this one's.
                state.notice.clear();
                Some(state.pending.submit(request, |slot, call| {
                    send(client.as_ref(), &entity_id, slot, call)
                }))
            })
        });
        match submitted {
            Some(Submitted::Waiting) => {
                STATE.with_borrow_mut(|state| run_for(&mut state.quiet, QUIET_MS, on_quiet));
            }
            Some(Submitted::Unsent(error)) => {
                show_notice(present::unsent_notice(error).to_owned(), FAILURE_MS);
            }
            Some(Submitted::Sent) | None => {}
        }
        // The frame that took the touch was drawn before the request,
        // and a finger held still on a filled hold asks for no other.
        request_frame();
    }

    /// Runs `timer` for `period_ms` from now, and tells whether it runs.
    /// One already running is pushed out rather than replaced:
    /// a new one would need a free timer while the old one still holds its own.
    fn run_for(timer: &mut Option<Timer>, period_ms: u32, on_end: TimerCallback) -> bool {
        if timer
            .as_ref()
            .is_some_and(|running| running.set_period(period_ms))
        {
            return true;
        }
        *timer = timer::start(period_ms, on_end);
        timer.is_some()
    }

    /// Starts the preview, or keeps one already on for [`LED_PREVIEW_MS`] more; frames set its colour.
    fn preview_for_a_while() {
        let pushed_out = STATE.with_borrow(|state| {
            state
                .preview
                .as_ref()
                .is_some_and(|preview| preview.until.set_period(LED_PREVIEW_MS))
        });
        if pushed_out {
            return;
        }
        let Some(until) = timer::start(LED_PREVIEW_MS, on_preview_end) else {
            log_warn!("home-assistant-tile: no timer to end the LED preview, so none starts");
            return;
        };
        STATE.with_borrow_mut(|state| {
            let shown = state.preview.take().and_then(|preview| preview.shown);
            state.preview = Some(Preview { until, shown });
        });
    }

    fn on_preview_end(_timer: &Timer, _elapsed_periods: u32) {
        let previewed = STATE.with_borrow_mut(|state| state.preview.take());
        if previewed.is_some_and(|preview| preview.shown.is_some()) {
            led::stop();
        }
    }

    /// Sets the strip to what the tile previews now, when that changed.
    fn update_preview(color: Option<Color>) {
        let changed = STATE.with_borrow_mut(|state| match &mut state.preview {
            Some(preview) if preview.shown != color => {
                preview.shown = color;
                true
            }
            Some(_) | None => false,
        });
        if changed {
            match color {
                Some(color) => led::set_effect(led::LedEffect::Solid, color, 0, None),
                None => led::stop(),
            }
        }
    }

    fn send(
        client: Option<&Client>,
        entity_id: &str,
        slot: Slot,
        call: &ServiceCall,
    ) -> Result<CallId, RequestError> {
        let sent = client
            .ok_or(RequestError::NotConnected)
            .and_then(|client| client.call(entity_id, call));
        if let Err(error) = sent {
            log_warn!(
                "home-assistant-tile: {} ({}.{}) could not be sent: {}",
                slot.name(),
                call.domain,
                call.service,
                error.name()
            );
        }
        sent
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(delta_ms: u32) {
        let size = widget_size();
        #[expect(clippy::cast_precision_loss, reason = "a widget a few hundred px wide")]
        let width = size.width as f32;
        #[expect(clippy::cast_precision_loss, reason = "a widget a few hundred px high")]
        let height = size.height as f32;
        let touches = STATE.with_borrow_mut(|state| {
            state.notice.tick(delta_ms);
            state.touches.clone()
        });
        let room = Room {
            width,
            height,
            size: size.variant,
        };
        let screen = screen(room);
        let node = match &screen {
            Screen::Placeholder(card) => card.centred(&touches),
            Screen::Loading(group, card) => group.skeleton(card, &touches),
            Screen::Tile { presenter, .. } => presenter.view(&touches),
        };
        update_preview(match &screen {
            Screen::Tile { presenter, .. } => presenter.led(),
            Screen::Placeholder(_) | Screen::Loading(..) => None,
        });
        let drawn_with = touches;
        let touches = Touches::read(render_ui(size.width, size.height, node));
        // This frame was drawn from the last one's touches: one more shows a finger landing, lifting or cancelled.
        if !touches.holds_as(&drawn_with) {
            request_frame();
        }
        match &screen {
            Screen::Tile { presenter, reach } => {
                if reach.can_act()
                    && let Some(request) = presenter.touched(&touches)
                {
                    submit(request);
                    // A value dialled in on the rows shows on the strip; an on or off shows in the room.
                    preview_for_a_while();
                }
                fire_card_action(card::touch(&touches), presenter.card_action(), delta_ms);
            }
            // A finger on the placeholder stays down when the tile comes, and fires nothing when lifted.
            Screen::Placeholder(_) | Screen::Loading(..) => {
                fire_card_action(card::touch(&touches), CardAction::Blocked, delta_ms);
            }
        }
        if let Some(left_ms) = STATE.with_borrow(|state| state.notice.left_ms()) {
            request_frame_after(left_ms);
        }
        STATE.with_borrow_mut(|state| state.touches = touches);
    }

    /// A touch on the card, under the tap and hold settings: firing sends the card action.
    fn fire_card_action(touch: Touch, action: CardAction, delta_ms: u32) {
        let (availability, request) = match action {
            CardAction::Ready(request) => (Availability::Ready, Some(request)),
            CardAction::None | CardAction::Blocked => (Availability::Blocked, None),
        };
        let (step, filling) = STATE.with_borrow_mut(|state| {
            let step = state.gesture.frame(delta_ms, touch, availability);
            (step, state.gesture.filling())
        });
        match (step, request) {
            (Step::Fired, Some(request)) => submit(request),
            (Step::ReleasedEarly, _) => {
                show_notice("Released too early".to_owned(), RELEASED_EARLY_MS);
            }
            (Step::Fired | Step::Nothing, _) => {}
        }
        if filling {
            request_frame();
        }
    }

    /// The tile for the followed entity, its skeleton while the entity loads, or why there is none.
    fn screen(room: Room) -> Screen {
        let params = Params::current();
        let configured = params
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let entity_id = entity_id(&params);
        let name = configured.or(entity_id).unwrap_or(UNNAMED);
        let group = match present::group_of(entity_id) {
            Ok(group) => group,
            Err(problem_now) => return problem(problem_now, name, room),
        };
        let status = CLIENT.with_borrow(|client| client.as_ref().map(Client::status));
        STATE.with_borrow(|state| {
            let open = state
                .followed
                .as_ref()
                .filter(|followed| followed.subscribed == Subscribed::Open);
            let entity =
                open.and_then(|followed| followed.subscription.entity(&followed.entity_id));
            let following = match open {
                Some(_) => Following::Open(entity.as_ref()),
                None => Following::Refused,
            };
            match present::showing(status, following) {
                Showing::Problem(problem_now) => problem(problem_now, name, room),
                Showing::Loading => Screen::Loading(group, present::loading(group, name, room)),
                Showing::Entity(entity) => {
                    let precision = state
                        .followed
                        .as_ref()
                        .and_then(|followed| followed.precision.as_ref())
                        .map_or(Precision::Known(None), |watch| watch.shown);
                    let context = Context {
                        pending: &state.pending,
                        gesture: &state.gesture,
                        name: configured
                            .or_else(|| entity.text(FRIENDLY_NAME))
                            .unwrap_or(name),
                        notice: state.notice.text(),
                        reach: Reach::of(entity),
                        room,
                    };
                    Screen::Tile {
                        presenter: group.tile(entity, &context, precision),
                        reach: context.reach,
                    }
                }
            }
        })
    }

    fn problem(problem: Problem, name: &str, room: Room) -> Screen {
        Screen::Placeholder(present::problem(problem, name, room))
    }
}
