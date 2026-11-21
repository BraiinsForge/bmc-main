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
//! A request settles on its call's answer, on the next report, as the frontend's `ha-entity-toggle` keeps an on or off,
//! or on a successful answer and a report since the call went out, in either order, as a value does: see [`Settles`].
//! A report settles an on or off even while its call is in flight: a lock that jams while locking reports `jammed`,
//! and nothing more. A switch that ignores the call reports nothing, hence [`Pending::give_up`].

use std::collections::HashMap;

use home_assistant::CallFailure;
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

/// What settles a request, besides its call failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settles {
    OnAnswer,
    /// The next report, which can come before the answer, or [`Pending::give_up`] if none comes in time.
    OnReport,
    /// The call succeeding and a report since it went out, in either order,
    /// or [`Pending::give_up`] if none comes in time.
    /// Home Assistant answers once the service has run, so an integration that writes the state in the service
    /// reports before the answer; one from before the call went out confirms nothing.
    /// Any report since then counts, even one a slow device sent for an earlier change:
    /// its value shows until the next report, as the frontend's controls follow every report.
    OnAnswerAndReport,
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
            Settles::OnAnswerAndReport,
        )
    }

    #[must_use]
    pub fn choice(slot: Slot, choice: String, call: ServiceCall) -> Self {
        Self::new(
            slot,
            Wanted::Choice(choice),
            call,
            Timing::Now,
            Settles::OnAnswerAndReport,
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

    /// Also drops what was asked under `slots`, sent or not;
    /// if this request cannot be sent, only what still waits for taps.
    #[must_use]
    pub fn superseding(self, slots: &'static [Slot]) -> Self {
        Self {
            supersedes: slots,
            ..self
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
    /// It could not be sent, so its slot and those it supersedes show an earlier call still on its way
    /// or awaiting its report, or else what Home Assistant reports.
    Unsent(RequestError),
}

/// What an answer to a call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered<'a> {
    /// What Home Assistant reports is shown again.
    Done,
    /// The call sent for `slot` failed: the slot shows what Home Assistant reports again,
    /// unless a later request replaced the call.
    /// Told whatever the slot shows, as the frontend tells of every failed call.
    Failed {
        slot: Slot,
        failure: &'a CallFailure,
    },
    /// The request waits for Home Assistant to report the entity;
    /// the caller gives up with [`Pending::give_up`] if no report comes in time.
    AwaitingReport,
}

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Waiting(ServiceCall),
    Sent {
        id: CallId,
        /// Home Assistant reported the entity while the call was out.
        reported: bool,
    },
    AwaitingReport,
}

impl Stage {
    fn sent(id: CallId) -> Self {
        Self::Sent {
            id,
            reported: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    wanted: Wanted,
    timing: Timing,
    settles: Settles,
    stage: Stage,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pending {
    entries: HashMap<Slot, Entry>,
    /// Every call sent from here and not yet answered, replaced or not, with the slot it was sent for.
    /// No longer than the client's cap on calls in flight, as it answers each one, if only with a timeout.
    unanswered: Vec<(CallId, Slot)>,
}

impl Pending {
    /// Shows `request` waiting at once, replacing whatever its slot showed.
    /// One to send now goes out with `send`, which names the call or says why it could not go.
    pub fn submit(
        &mut self,
        request: Request,
        send: impl FnOnce(Slot, &ServiceCall) -> Result<CallId, RequestError>,
    ) -> Submitted {
        let Request {
            slot,
            wanted,
            call,
            timing,
            settles,
            supersedes,
        } = request;
        let (stage, submitted) = match timing {
            Timing::AfterTaps => (Stage::Waiting(call), Submitted::Waiting),
            Timing::Now => match send(slot, &call) {
                Ok(id) => {
                    self.unanswered.push((id, slot));
                    (Stage::sent(id), Submitted::Sent)
                }
                Err(error) => {
                    // A request still waiting for taps would go out after the user asked for something else.
                    self.entries.retain(|&asked, entry| {
                        let replaced = asked == slot || supersedes.contains(&asked);
                        !(replaced && matches!(entry.stage, Stage::Waiting(_)))
                    });
                    return Submitted::Unsent(error);
                }
            },
        };
        for superseded in supersedes {
            self.entries.remove(superseded);
        }
        self.entries.insert(
            slot,
            Entry {
                wanted,
                timing,
                settles,
                stage,
            },
        );
        submitted
    }

    /// The taps have stopped: sends what waited for them. Returns why each call that could not go did not.
    pub fn send_waiting(
        &mut self,
        mut send: impl FnMut(Slot, &ServiceCall) -> Result<CallId, RequestError>,
    ) -> Vec<RequestError> {
        let mut unsent = Vec::new();
        let unanswered = &mut self.unanswered;
        self.entries.retain(|&slot, entry| {
            let Stage::Waiting(call) = &entry.stage else {
                return true;
            };
            match send(slot, call) {
                Ok(id) => {
                    unanswered.push((id, slot));
                    entry.stage = Stage::sent(id);
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
    pub fn answered<'a>(
        &mut self,
        call: CallId,
        result: Result<(), &'a CallFailure>,
    ) -> Option<Answered<'a>> {
        let sent_here = self.unanswered.iter().position(|&(id, _)| id == call)?;
        let (_, slot) = self.unanswered.swap_remove(sent_here);
        let Some(entry) = self
            .entries
            .get_mut(&slot)
            .filter(|entry| matches!(entry.stage, Stage::Sent { id, .. } if id == call))
        else {
            return result
                .err()
                .map(|failure| Answered::Failed { slot, failure });
        };
        let reported = matches!(entry.stage, Stage::Sent { reported: true, .. });
        let answered = match (result, entry.settles, reported) {
            (Err(failure), _, _) => Answered::Failed { slot, failure },
            (Ok(()), Settles::OnReport, _) | (Ok(()), Settles::OnAnswerAndReport, false) => {
                entry.stage = Stage::AwaitingReport;
                return Some(Answered::AwaitingReport);
            }
            (Ok(()), Settles::OnAnswer, _) | (Ok(()), Settles::OnAnswerAndReport, true) => {
                Answered::Done
            }
        };
        self.entries.remove(&slot);
        Some(answered)
    }

    /// Home Assistant reported the entity: a request that settles on this report does,
    /// and one still waiting for its answer notes the report.
    /// Returns whether one settled.
    pub fn reported(&mut self) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|_, entry| match (entry.settles, &mut entry.stage) {
                (Settles::OnAnswerAndReport, Stage::Sent { reported, .. }) => {
                    *reported = true;
                    true
                }
                (_, Stage::Waiting(_)) | (Settles::OnAnswer, _) => true,
                (Settles::OnReport, Stage::Sent { .. })
                | (Settles::OnReport | Settles::OnAnswerAndReport, Stage::AwaitingReport) => false,
            });
        self.entries.len() < before
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

    /// What was asked for `slot` came from a run of taps, and Home Assistant has yet to confirm it.
    #[must_use]
    pub fn asked_by_taps(&self, slot: Slot) -> bool {
        self.entries
            .get(&slot)
            .is_some_and(|entry| entry.timing == Timing::AfterTaps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    const BRIGHTNESS: Slot = Slot::new("brightness");
    const SUCCEEDED: Result<(), &CallFailure> = Ok(());
    const FAILED: Result<(), &CallFailure> = Err(&CallFailure::TimedOut);
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
    fn as_call(id: usize) -> impl FnMut(Slot, &ServiceCall) -> Result<CallId, RequestError> {
        move |_, _| Ok(CallId::from_raw(id))
    }

    fn unsendable(_: Slot, _: &ServiceCall) -> Result<CallId, RequestError> {
        Err(RequestError::NotConnected)
    }

    fn id(raw: usize) -> CallId {
        CallId::from_raw(raw)
    }

    #[wasm_test]
    fn a_value_shows_at_once_and_lasts_until_its_answer_and_a_report_after_it() {
        let mut pending = Pending::default();
        assert_eq!(
            pending.submit(brightness(80.0, Timing::Now), as_call(1)),
            Submitted::Sent
        );
        assert_eq!(pending.number(BRIGHTNESS), Some(80.0));
        assert_eq!(
            pending.answered(id(1), SUCCEEDED),
            Some(Answered::AwaitingReport)
        );
        assert_eq!(pending.number(BRIGHTNESS), Some(80.0));
        pending.reported();
        assert_eq!(pending.number(BRIGHTNESS), None);
    }

    #[wasm_test]
    fn a_report_while_the_call_is_out_settles_the_value_on_its_answer() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::Now), as_call(1));
        assert!(!pending.reported(), "the call can still fail");
        assert_eq!(pending.number(BRIGHTNESS), Some(80.0));
        assert_eq!(pending.answered(id(1), SUCCEEDED), Some(Answered::Done));
        assert_eq!(pending.number(BRIGHTNESS), None);
    }

    #[wasm_test]
    fn a_report_before_the_call_went_out_confirms_nothing() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::AfterTaps), unsendable);
        pending.reported();
        pending.send_waiting(as_call(1));
        assert_eq!(
            pending.answered(id(1), SUCCEEDED),
            Some(Answered::AwaitingReport)
        );
    }

    #[wasm_test]
    fn a_value_with_no_report_after_its_answer_gives_up() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::Now), as_call(1));
        pending.answered(id(1), SUCCEEDED);
        pending.give_up();
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
        let unsent = pending.send_waiting(|_, _| {
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
        assert_eq!(
            pending.answered(id(1), SUCCEEDED),
            Some(Answered::AwaitingReport)
        );
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
                slot: POWER,
                failure: &CallFailure::TimedOut
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
                slot: POWER,
                failure: &CallFailure::TimedOut
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
                slot: BRIGHTNESS,
                failure: &CallFailure::TimedOut
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
    fn a_choice_lasts_until_its_answer_and_a_report_after_it() {
        let mut pending = Pending::default();
        pending.submit(Request::choice(MODE, "heat".to_owned(), call()), as_call(1));
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
        pending.submit(power(false).superseding(&[BRIGHTNESS]), as_call(1));
        assert_eq!(pending.number(BRIGHTNESS), None);
        let mut sent = 0;
        pending.send_waiting(|_, _| {
            sent += 1;
            Ok(id(2))
        });
        assert_eq!(sent, 0, "the brightness would turn it back on");
        assert_eq!(pending.on(POWER), Some(false));
    }

    #[wasm_test]
    fn a_request_that_cannot_go_leaves_what_it_supersedes_on_its_way() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::Now), as_call(1));
        pending.submit(power(false).superseding(&[BRIGHTNESS]), unsendable);
        assert_eq!(
            pending.number(BRIGHTNESS),
            Some(80.0),
            "its call still turns the light on"
        );
    }

    #[wasm_test]
    fn a_request_that_cannot_go_drops_what_it_supersedes_waiting_for_taps() {
        let mut pending = Pending::default();
        pending.submit(brightness(10.0, Timing::AfterTaps), unsendable);
        pending.submit(power(false).superseding(&[BRIGHTNESS]), unsendable);
        assert_eq!(pending.number(BRIGHTNESS), None);
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

    #[wasm_test]
    fn a_value_from_a_run_of_taps_reads_so_until_confirmed() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::AfterTaps), unsendable);
        pending.send_waiting(as_call(1));
        assert!(pending.asked_by_taps(BRIGHTNESS), "still on its way");
        pending.answered(id(1), SUCCEEDED);
        pending.reported();
        assert!(!pending.asked_by_taps(BRIGHTNESS));
    }

    #[wasm_test]
    fn a_report_says_whether_it_settled_a_request() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::Now), as_call(1));
        pending.answered(id(1), SUCCEEDED);
        assert!(pending.reported());
        assert!(!pending.reported(), "nothing left to settle");
    }

    #[wasm_test]
    fn a_value_sent_at_once_is_not_from_a_run_of_taps() {
        let mut pending = Pending::default();
        pending.submit(brightness(80.0, Timing::Now), as_call(1));
        assert!(pending.asked(BRIGHTNESS));
        assert!(!pending.asked_by_taps(BRIGHTNESS));
    }
}
