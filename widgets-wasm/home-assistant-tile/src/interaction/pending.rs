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

//! What the user asked of the entity and Home Assistant has yet to confirm,
//! shown in place of what Home Assistant reports, each under the slot it is shown in.
//!
//! A request settles either on its call's answer, as the frontend's `PendingValueSync` keeps a value,
//! or on the next report, as its `ha-entity-toggle` keeps an on or off: see [`Settles`].
//! A report settles one even while its call is in flight: a lock that jams while locking reports `jammed`,
//! and nothing more. A switch that ignores the call reports nothing, hence [`Pending::give_up`].

use std::collections::HashMap;

use home_assistant::CallId;
use home_assistant::RequestError;
use home_assistant::ServiceCall;

/// Where a wanted value is shown, such as a light's brightness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot(&'static str);

impl Slot {
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }
}

/// What the user wants shown until Home Assistant confirms it.
#[derive(Debug, Clone, PartialEq)]
pub enum Wanted {
    On(bool),
    Number(f32),
    Choice(String),
    /// Nothing to show, such as for a button's press: only that it is on its way.
    NoValue,
}

/// When a request goes to Home Assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    Now,
    /// Once the taps stop, as − and + come in runs: each request replaces the one before.
    AfterTaps,
}

/// What settles a request, besides its call failing: the call's answer,
/// or Home Assistant's next report, which can come before the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settles {
    OnAnswer,
    /// The next report, or [`Pending::give_up`] if none comes in time.
    OnReport,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    slot: Slot,
    wanted: Wanted,
    call: ServiceCall,
    timing: Timing,
    settles: Settles,
    /// Other slots it makes moot, which stop waiting: a light turned off drops a brightness still to be sent,
    /// which would turn it back on.
    supersedes: &'static [Slot],
}

impl Request {
    /// Turning the entity on or off, sent at once and shown until Home Assistant reports it.
    #[must_use]
    pub fn flip(slot: Slot, on: bool, call: ServiceCall) -> Self {
        Self::new(slot, Wanted::On(on), call, Timing::Now, Settles::OnReport)
    }

    #[must_use]
    pub fn number(slot: Slot, number: f32, call: ServiceCall, timing: Timing) -> Self {
        Self::new(
            slot,
            Wanted::Number(number),
            call,
            timing,
            Settles::OnAnswer,
        )
    }

    #[must_use]
    pub fn choice(slot: Slot, choice: String, call: ServiceCall) -> Self {
        Self::new(
            slot,
            Wanted::Choice(choice),
            call,
            Timing::Now,
            Settles::OnAnswer,
        )
    }

    #[must_use]
    pub fn no_value(slot: Slot, call: ServiceCall) -> Self {
        Self::new(slot, Wanted::NoValue, call, Timing::Now, Settles::OnAnswer)
    }

    fn new(
        slot: Slot,
        wanted: Wanted,
        call: ServiceCall,
        timing: Timing,
        settles: Settles,
    ) -> Self {
        Self {
            slot,
            wanted,
            call,
            timing,
            settles,
            supersedes: &[],
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn call(&self) -> &ServiceCall {
        &self.call
    }
}

/// What became of a request just submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submitted {
    Sent,
    /// The caller (re)starts its wait for the taps to stop, then calls [`Pending::send_waiting`].
    Waiting,
    /// It could not be sent, so the slot shows an earlier call still on its way or awaiting its report,
    /// or else what Home Assistant reports.
    Unsent(RequestError),
}

/// What an answer to a call did; `F` is why a call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered<F> {
    /// What Home Assistant reports is shown again.
    Done,
    /// The call failed: its slot, if it still held one, shows what Home Assistant reports again.
    /// No slot once a report settled the call, or a later request replaced it and stays shown;
    /// still a failure to tell, as the frontend tells of every failed call.
    Failed { slot: Option<Slot>, failure: F },
    /// The request waits for Home Assistant to report the entity;
    /// the caller gives up with [`Pending::give_up`] if no report comes in time.
    AwaitingReport,
}

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Waiting(ServiceCall),
    Sent(CallId),
    AwaitingReport,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    wanted: Wanted,
    settles: Settles,
    stage: Stage,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pending {
    entries: HashMap<Slot, Entry>,
    /// Every call sent from here and not yet answered, replaced or not.
    /// No longer than the client's cap on calls in flight, as it answers each one, if only with a timeout.
    unanswered: Vec<CallId>,
}

impl Pending {
    /// Shows `request` waiting at once, replacing whatever its slot showed.
    /// One to send now goes out with `send`, which names the call or says why it could not go.
    pub fn submit(
        &mut self,
        request: Request,
        send: impl FnOnce(&ServiceCall) -> Result<CallId, RequestError>,
    ) -> Submitted {
        let Request {
            slot,
            wanted,
            call,
            timing,
            settles,
            supersedes,
        } = request;
        for superseded in supersedes {
            self.entries.remove(superseded);
        }
        let (stage, submitted) = match timing {
            Timing::AfterTaps => (Stage::Waiting(call), Submitted::Waiting),
            Timing::Now => match send(&call) {
                Ok(id) => {
                    self.unanswered.push(id);
                    (Stage::Sent(id), Submitted::Sent)
                }
                Err(error) => {
                    // A request still waiting for taps would go out after the user asked for something else.
                    if self
                        .entries
                        .get(&slot)
                        .is_some_and(|entry| matches!(entry.stage, Stage::Waiting(_)))
                    {
                        self.entries.remove(&slot);
                    }
                    return Submitted::Unsent(error);
                }
            },
        };
        self.entries.insert(
            slot,
            Entry {
                wanted,
                settles,
                stage,
            },
        );
        submitted
    }

    /// The taps have stopped: sends what waited for them. Returns why each call that could not go did not.
    pub fn send_waiting(
        &mut self,
        mut send: impl FnMut(&ServiceCall) -> Result<CallId, RequestError>,
    ) -> Vec<RequestError> {
        let mut unsent = Vec::new();
        let unanswered = &mut self.unanswered;
        self.entries.retain(|_, entry| {
            let Stage::Waiting(call) = &entry.stage else {
                return true;
            };
            match send(call) {
                Ok(id) => {
                    unanswered.push(id);
                    entry.stage = Stage::Sent(id);
                    true
                }
                Err(error) => {
                    unsent.push(error);
                    false
                }
            }
        });
        unsent
    }

    /// Home Assistant answered `call` with `result`.
    /// A success is none once nothing waits for it,
    /// as a later request replaced or superseded it, or a report settled it.
    /// An answer to a call not sent from here is none.
    pub fn answered<F>(&mut self, call: CallId, result: Result<(), F>) -> Option<Answered<F>> {
        let sent_here = self.unanswered.iter().position(|&id| id == call)?;
        self.unanswered.swap_remove(sent_here);
        let Some((&slot, entry)) = self
            .entries
            .iter_mut()
            .find(|(_, entry)| matches!(entry.stage, Stage::Sent(id) if id == call))
        else {
            return result.err().map(|failure| Answered::Failed {
                slot: None,
                failure,
            });
        };
        let answered = match (result, entry.settles) {
            (Err(failure), _) => Answered::Failed {
                slot: Some(slot),
                failure,
            },
            (Ok(()), Settles::OnReport) => {
                entry.stage = Stage::AwaitingReport;
                return Some(Answered::AwaitingReport);
            }
            (Ok(()), Settles::OnAnswer) => Answered::Done,
        };
        self.entries.remove(&slot);
        Some(answered)
    }

    /// Home Assistant reported the entity: a request that settles on a report does, unless it still waits for taps.
    pub fn reported(&mut self) {
        self.entries.retain(|_, entry| {
            entry.settles == Settles::OnAnswer || matches!(entry.stage, Stage::Waiting(_))
        });
    }

    /// No report came in time: a request whose call succeeded and that waits for one shows what Home Assistant reports.
    pub fn give_up(&mut self) {
        self.entries
            .retain(|_, entry| !matches!(entry.stage, Stage::AwaitingReport));
    }

    #[must_use]
    pub fn on(&self, slot: Slot) -> Option<bool> {
        match self.entries.get(&slot)?.wanted {
            Wanted::On(on) => Some(on),
            Wanted::Number(_) | Wanted::Choice(_) | Wanted::NoValue => None,
        }
    }

    #[must_use]
    pub fn number(&self, slot: Slot) -> Option<f32> {
        match self.entries.get(&slot)?.wanted {
            Wanted::Number(number) => Some(number),
            Wanted::On(_) | Wanted::Choice(_) | Wanted::NoValue => None,
        }
    }

    #[must_use]
    pub fn choice(&self, slot: Slot) -> Option<&str> {
        match &self.entries.get(&slot)?.wanted {
            Wanted::Choice(choice) => Some(choice),
            Wanted::On(_) | Wanted::Number(_) | Wanted::NoValue => None,
        }
    }

    /// Something was asked for `slot` that Home Assistant has yet to confirm.
    #[must_use]
    pub fn asked(&self, slot: Slot) -> bool {
        self.entries.contains_key(&slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    const BRIGHTNESS: Slot = Slot::new("brightness");
    const SUCCEEDED: Result<(), ()> = Ok(());
    const FAILED: Result<(), ()> = Err(());
    const POWER: Slot = Slot::new("power");
    const MODE: Slot = Slot::new("mode");

    fn call() -> ServiceCall {
        ServiceCall::new("light", "turn_on")
    }

    fn brightness(percent: f32, timing: Timing) -> Request {
        Request::number(BRIGHTNESS, percent, call(), timing)
    }

    fn power(on: bool) -> Request {
        Request::flip(POWER, on, call())
    }

    /// Sends as call `id`.
    fn as_call(id: usize) -> impl FnMut(&ServiceCall) -> Result<CallId, RequestError> {
        move |_| Ok(CallId::from_raw(id))
    }

    fn unsendable(_: &ServiceCall) -> Result<CallId, RequestError> {
        Err(RequestError::NotConnected)
    }

    fn id(raw: usize) -> CallId {
        CallId::from_raw(raw)
    }

    #[wasm_test]
    fn a_value_shows_at_once_and_lasts_until_its_call_is_answered() {
        let mut pending = Pending::default();
        assert_eq!(
            pending.submit(brightness(80.0, Timing::Now), as_call(1)),
            Submitted::Sent
        );
        assert_eq!(pending.number(BRIGHTNESS), Some(80.0));
        assert_eq!(pending.answered(id(1), SUCCEEDED), Some(Answered::Done));
        assert_eq!(pending.number(BRIGHTNESS), None);
    }

    #[wasm_test]
    fn taps_in_a_run_send_only_the_last() {
        let mut pending = Pending::default();
        pending.submit(brightness(70.0, Timing::AfterTaps), unsendable);
        assert_eq!(
            pending.submit(brightness(80.0, Timing::AfterTaps), unsendable),
            Submitted::Waiting
        );
        let mut sent = 0;
        let unsent = pending.send_waiting(|_| {
            sent += 1;
            Ok(id(1))
        });
        assert_eq!((sent, unsent), (1, Vec::new()));
        assert_eq!(pending.number(BRIGHTNESS), Some(80.0));
    }

    #[wasm_test]
    fn a_call_sent_once_the_taps_stop_is_answered_as_any_other() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::AfterTaps), unsendable);
        pending.send_waiting(as_call(1));
        assert_eq!(pending.answered(id(1), SUCCEEDED), Some(Answered::Done));
    }

    #[wasm_test]
    fn an_answer_to_a_replaced_call_changes_nothing() {
        let mut pending = Pending::default();
        pending.submit(brightness(70.0, Timing::Now), as_call(1));
        pending.submit(brightness(80.0, Timing::AfterTaps), unsendable);
        assert_eq!(pending.answered(id(1), SUCCEEDED), None);
        assert_eq!(pending.number(BRIGHTNESS), Some(80.0));
    }

    #[wasm_test]
    fn a_replaced_call_that_failed_is_told_and_changes_nothing() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(1));
        pending.submit(power(false), as_call(2));
        assert_eq!(
            pending.answered(id(1), FAILED),
            Some(Answered::Failed {
                slot: None,
                failure: ()
            })
        );
        assert_eq!(pending.on(POWER), Some(false));
    }

    #[wasm_test]
    fn a_call_a_report_settled_is_told_when_it_fails() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(1));
        pending.reported();
        assert_eq!(
            pending.answered(id(1), FAILED),
            Some(Answered::Failed {
                slot: None,
                failure: ()
            })
        );
    }

    #[wasm_test]
    fn a_call_sent_from_elsewhere_is_neither_told_nor_shown() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(2));
        assert_eq!(pending.answered(id(1), FAILED), None);
        assert_eq!(pending.on(POWER), Some(true));
    }

    #[wasm_test]
    fn a_call_is_answered_only_once() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(1));
        pending.submit(power(false), as_call(2));
        pending.answered(id(1), FAILED);
        assert_eq!(pending.answered(id(1), FAILED), None);
    }

    #[wasm_test]
    fn a_refused_call_shows_what_home_assistant_reports() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::Now), as_call(1));
        assert_eq!(
            pending.answered(id(1), FAILED),
            Some(Answered::Failed {
                slot: Some(BRIGHTNESS),
                failure: ()
            })
        );
        assert_eq!(pending.number(BRIGHTNESS), None);
    }

    #[wasm_test]
    fn a_call_that_cannot_go_shows_nothing_waiting() {
        let mut pending = Pending::default();
        pending.submit(brightness(70.0, Timing::AfterTaps), unsendable);
        assert_eq!(
            pending.submit(brightness(80.0, Timing::Now), unsendable),
            Submitted::Unsent(RequestError::NotConnected)
        );
        assert_eq!(pending.number(BRIGHTNESS), None, "nor the earlier wish");

        pending.submit(brightness(70.0, Timing::AfterTaps), unsendable);
        assert_eq!(
            pending.send_waiting(unsendable),
            vec![RequestError::NotConnected]
        );
        assert_eq!(pending.number(BRIGHTNESS), None);
    }

    #[wasm_test]
    fn a_call_that_cannot_go_leaves_the_one_on_its_way_shown() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(1));
        pending.submit(power(false), unsendable);
        assert_eq!(pending.on(POWER), Some(true));
        assert_eq!(
            pending.answered(id(1), SUCCEEDED),
            Some(Answered::AwaitingReport)
        );
    }

    #[wasm_test]
    fn a_report_leaves_a_value_waiting() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::Now), as_call(1));
        pending.reported();
        assert_eq!(pending.number(BRIGHTNESS), Some(80.0));
    }

    #[wasm_test]
    fn a_flip_whose_call_succeeded_waits_for_a_report() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(1));
        assert_eq!(
            pending.answered(id(1), SUCCEEDED),
            Some(Answered::AwaitingReport)
        );
        assert_eq!(pending.on(POWER), Some(true));
        pending.reported();
        assert_eq!(pending.on(POWER), None);
    }

    #[wasm_test]
    fn a_choice_asked_to_end_on_a_report_waits_for_one() {
        let mut pending = Pending::default();
        let heat = Request {
            settles: Settles::OnReport,
            ..Request::choice(MODE, "heat".to_owned(), call())
        };
        pending.submit(heat, as_call(1));
        assert_eq!(
            pending.answered(id(1), SUCCEEDED),
            Some(Answered::AwaitingReport)
        );
        assert_eq!(pending.choice(MODE), Some("heat"));
        pending.reported();
        assert_eq!(pending.choice(MODE), None);
    }

    #[wasm_test]
    fn a_flip_with_no_report_in_time_gives_up() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(1));
        pending.answered(id(1), SUCCEEDED);
        pending.give_up();
        assert_eq!(pending.on(POWER), None);
    }

    #[wasm_test]
    fn a_report_ends_a_flip_even_while_its_call_is_in_flight() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(1));
        pending.reported();
        assert_eq!(pending.on(POWER), None, "a lock that jammed while locking");
        assert_eq!(pending.answered(id(1), SUCCEEDED), None);
    }

    #[wasm_test]
    fn giving_up_leaves_a_flip_still_in_flight() {
        let mut pending = Pending::default();
        pending.submit(power(true), as_call(1));
        pending.answered(id(1), SUCCEEDED);
        pending.submit(power(false), as_call(2));
        pending.give_up();
        assert_eq!(pending.on(POWER), Some(false));
    }

    #[wasm_test]
    fn a_request_drops_what_it_supersedes() {
        let mut pending = Pending::default();
        pending.submit(brightness(10.0, Timing::AfterTaps), unsendable);
        pending.submit(
            Request {
                supersedes: &[BRIGHTNESS],
                ..power(false)
            },
            as_call(1),
        );
        assert_eq!(pending.number(BRIGHTNESS), None);
        let mut sent = 0;
        pending.send_waiting(|_| {
            sent += 1;
            Ok(id(2))
        });
        assert_eq!(sent, 0, "the brightness would turn it back on");
        assert_eq!(pending.on(POWER), Some(false));
    }

    #[wasm_test]
    fn a_request_without_a_value_lasts_until_its_call_is_answered() {
        let press = Slot::new("press");
        let mut pending = Pending::default();
        pending.submit(Request::no_value(press, call()), as_call(1));
        pending.reported();
        assert!(pending.asked(press), "a report says nothing of a press");
        assert_eq!(pending.answered(id(1), SUCCEEDED), Some(Answered::Done));
        assert!(!pending.asked(press));
    }

    #[wasm_test]
    fn a_slot_reads_only_as_what_was_asked_for_it() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::Now), as_call(1));
        assert_eq!(pending.on(BRIGHTNESS), None);
        assert_eq!(pending.choice(BRIGHTNESS), None);
        assert_eq!(pending.number(POWER), None);
    }
}
