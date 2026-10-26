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

//! What a finished call answers with, and how a pending call reads it from the result.

use std::collections::BTreeMap;

use super::client::{CallId, Status, SubscriptionId};
use super::entity::{AttributeValue, read_value};
use super::history::{HistoryPoint, Series, read_history};
use super::json::{JsonLookup, Pointer, PointerBuf};

/// Home Assistant's code for an error it has no more specific code for,
/// and the one a failure arrives with when it names none.
pub const UNKNOWN_ERROR: &str = "unknown_error";

/// An error code and message as Home Assistant returns them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaError {
    /// [`UNKNOWN_ERROR`] when the failure names no code.
    pub code: String,
    /// Empty when the failure carries no message.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallFailure {
    Rejected(HaError),
    /// The connection dropped before an answer came.
    /// Home Assistant may or may not have run the call,
    /// so repeat only a history request, never a service call or a command,
    /// and only once the status is back to [`Status::Ready`].
    /// After [`Status::MessageTooLarge`] the client stays stopped:
    /// the oversized message may have been any pending reply or a subscription's event,
    /// so ask for a shorter time range or fewer entities, then [`Session::resume`](super::Session::resume).
    Disconnected,
    /// No answer came within 30 to 60 s.
    /// The session has no clock, so it times a call in its 30 s keepalive ticks
    /// and fails it at the second tick after it was sent.
    TimedOut,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StatusChanged(Status),
    EntityChanged {
        subscription: SubscriptionId,
        entity_id: String,
    },
    SubscriptionFailed {
        subscription: SubscriptionId,
        error: HaError,
    },
    CallFinished {
        call: CallId,
        result: Result<Reply, CallFailure>,
    },
}

/// What a finished call answered with, one kind per kind of request.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// A service call finished; it carries no data.
    Done,
    /// The values a command asked for.
    Values(Values),
    /// Each entity's states, by entity id; one without states in the period is left out.
    History(BTreeMap<String, Series<HistoryPoint>>),
}

/// The values a command found at the JSON Pointers it named.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Values(BTreeMap<String, AttributeValue>);

impl Values {
    /// The value at `pointer`, one of those the command asked for.
    ///
    /// `None` when the reply has nothing there, or holds a value the client does not keep:
    /// `null`, an object (point inside it instead), text over
    /// [`MAX_ATTRIBUTE_TEXT_BYTES`](super::MAX_ATTRIBUTE_TEXT_BYTES),
    /// or a list over its cap or mixing types, as for an entity's attributes.
    #[must_use]
    pub fn get(&self, pointer: &str) -> Option<&AttributeValue> {
        self.0.get(pointer)
    }

    /// A numeric value, `None` when absent or not a number.
    #[must_use]
    pub fn number(&self, pointer: &str) -> Option<f64> {
        self.get(pointer)?.as_number()
    }

    /// A text value, `None` when absent or not text.
    #[must_use]
    pub fn text(&self, pointer: &str) -> Option<&str> {
        self.get(pointer)?.as_text()
    }
}

#[derive(Debug)]
pub(super) struct PendingCall {
    pub(super) id: CallId,
    pub(super) ticks_waited: usize,
    pub(super) wanted: Wanted,
}

/// What a pending call's [`Reply`] keeps from its result.
#[derive(Debug)]
pub(super) enum Wanted {
    /// Nothing: a service call's result carries no data.
    Nothing,
    /// JSON Pointers into the result.
    Values(Vec<String>),
    /// Entity ids whose history the result holds.
    History(Vec<String>),
}

/// The values at `pointers` into a command's result.
fn read_values<'a>(message: &impl JsonLookup, pointers: impl Iterator<Item = &'a str>) -> Values {
    let mut path = PointerBuf::new(Pointer::root().key("result"));
    let result = path.mark();
    Values(
        pointers
            .filter_map(|pointer| {
                path.truncate(result);
                path.push_escaped(pointer);
                Some((pointer.to_owned(), read_value(message, &mut path)?))
            })
            .collect(),
    )
}

impl PendingCall {
    pub(super) fn reply(&self, message: &impl JsonLookup) -> Reply {
        match &self.wanted {
            Wanted::Nothing => Reply::Done,
            Wanted::Values(pointers) => {
                Reply::Values(read_values(message, pointers.iter().map(String::as_str)))
            }
            Wanted::History(entity_ids) => Reply::History(read_history(message, entity_ids)),
        }
    }
}
