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

//! What the session reports to the widget: its events, and how a call failed.

use super::client::{CallId, Status, SubscriptionId};

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
    /// Home Assistant may or may not have run the call, so never repeat it on the widget's own.
    /// After [`Status::MessageTooLarge`] the client stays stopped:
    /// the oversized message may have been any pending reply or a subscription's event,
    /// so ask for fewer entities, then [`Session::resume`](super::Session::resume).
    Disconnected,
    /// No answer came within 30 to 60 s.
    /// The session has no clock, so it times a call in its 30 s keepalive ticks
    /// and fails it at the second tick after it was sent.
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
        result: Result<(), CallFailure>,
    },
}

#[derive(Debug)]
pub(super) struct PendingCall {
    pub(super) id: CallId,
    pub(super) ticks_waited: usize,
}
