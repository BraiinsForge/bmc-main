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

//! A Home Assistant client for widgets, over its WebSocket API.
//!
//! [`proto`] is the protocol core.
//! It owns no socket and no clock, and does not depend on the widget SDK:
//! it reads every message through [`proto::JsonLookup`], so its tests drive it with documents of their own,
//! such as the allocation test's model of lookups the host answers.
//! The adapter runs it on the widget SDK, and is the widget's whole interface:
//!
//! ```ignore
//! let client = home_assistant::connect("ha", on_ha_event)?;
//! let sensor = client.subscribe(&["sensor.outdoor"], &["friendly_name", "unit_of_measurement"])?;
//! let call = client.call_service("button", "press", "button.gate", &ServiceData::new())?;
//! let config = client.command("get_config", &ServiceData::new(), &["/time_zone"])?;
//! ```
//!
//! Every type a widget handles is re-exported here; [`proto`] holds the engine behind them.
//! A widget that redraws on each [`Event::EntityChanged`] can answer it with `request_frame_after`,
//! which merges every request before a frame into one.

#[cfg(target_arch = "wasm32")]
mod adapter;
#[cfg(any(target_arch = "wasm32", test))]
mod credential;
pub mod format;
pub mod proto;

#[cfg(target_arch = "wasm32")]
pub use adapter::{Client, ConnectError, EventCallback, Subscription, connect};
pub use proto::{
    AttributeValue, CallFailure, CallId, Entity, Event, HaError, MAX_ATTRIBUTE_LIST_BYTES,
    MAX_ATTRIBUTE_NUMBERS, MAX_ATTRIBUTE_TEXT_BYTES, MAX_ATTRIBUTES_PER_SUBSCRIPTION,
    MAX_ENTITIES_PER_SUBSCRIPTION, MAX_ENTITY_ID_BYTES, MAX_NAME_BYTES, MAX_PENDING_CALLS,
    MAX_POINTER_BYTES, MAX_REPLY_VALUES, MAX_SERVICE_DATA_TEXT_BYTES, MAX_SUBSCRIPTIONS, Presence,
    Reply, RequestError, ServiceData, Status, SubscriptionId, UNKNOWN_ERROR, Values, is_entity_id,
};
