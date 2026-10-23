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

//! The protocol core: login, keepalive, reconnect, entity subscriptions,
//! service calls and commands.
//!
//! The caller owns the socket and the timer.
//! It reports what happened to them through the [`Session`] methods,
//! which return an [`Output`]: the [`Effects`] to carry out, commands and the next wake,
//! then the events for the widget.
//! The methods the widget's own calls drive return only the [`Effects`],
//! so a widget never gets an event while it is inside a call it made.
//! Time only ever enters as "the timer fired",
//! because the widget runtime has no monotonic clock
//! and a timer re-armed from the moment it is set measures a delay exactly.

mod check;
mod client;
mod effects;
mod entity;
mod json;
mod reply;
mod service_data;

pub use check::is_entity_id;
pub use client::{
    CallId, MAX_ATTRIBUTES_PER_SUBSCRIPTION, MAX_ENTITIES_PER_SUBSCRIPTION, MAX_ENTITY_ID_BYTES,
    MAX_NAME_BYTES, MAX_PENDING_CALLS, MAX_POINTER_BYTES, MAX_REPLY_VALUES,
    MAX_SERVICE_DATA_TEXT_BYTES, MAX_SUBSCRIPTIONS, RequestError, Session, Setup, Status,
    SubscriptionId,
};
pub use effects::{Closed, Command, Effects, Output, Wake};
pub use entity::{AttributeValue, Entity, MAX_ATTRIBUTE_TEXT_BYTES, Presence};
pub use json::JsonLookup;
pub use reply::{CallFailure, Event, HaError, Reply, UNKNOWN_ERROR, Values};
pub use service_data::ServiceData;
