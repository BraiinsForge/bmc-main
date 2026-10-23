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

//! One connection to one Home Assistant instance, shared by any number of subscriptions.

use std::collections::{BTreeMap, BTreeSet};

use super::check::{check_entity_ids, distinct, is_entity_id, is_free_name, is_name, is_pointer};
use super::effects::{Closed, Command, Effects, Output, Wake};
use super::entity::{Entity, EntityEvent};
use super::json::{JsonLookup, ObjectWriter};
use super::reply::{CallFailure, Event, HaError, PendingCall, UNKNOWN_ERROR, Wanted};
use super::service_data::{CheckedData, ServiceData};

/// Connect timeout, from the connect until `auth_ok`,
/// as the Home Assistant JS client applies it.
const CONNECT_TIMEOUT_SECS: u32 = 10;
/// The keepalive tick while ready, the Home Assistant frontend's ping cadence.
/// Each tick sends a ping, unless the last one is still unanswered, which drops the connection:
/// a pong gets a whole tick, 30 s, where the frontend allows 15.
/// A pong never moves the tick, so a slow one cannot stretch a call's timeout.
const TICK_SECS: u32 = 30;
/// Longest wait between reconnect attempts, as the Home Assistant JS client caps it.
const MAX_BACKOFF_SECS: u32 = 5;
/// The ticks a call waits for its answer, as [`CallFailure::TimedOut`] describes.
const CALL_TIMEOUT_TICKS: usize = 2;

pub const MAX_SUBSCRIPTIONS: usize = 8;
pub const MAX_ENTITIES_PER_SUBSCRIPTION: usize = 32;
pub const MAX_ATTRIBUTES_PER_SUBSCRIPTION: usize = 16;
pub const MAX_PENDING_CALLS: usize = 8;
pub const MAX_REPLY_VALUES: usize = 16;
/// Home Assistant's own cap on an entity id.
pub const MAX_ENTITY_ID_BYTES: usize = 255;
pub const MAX_NAME_BYTES: usize = 64;
pub const MAX_SERVICE_DATA_TEXT_BYTES: usize = 1024;
/// The same cap as an entity id, which a pointer may name.
pub const MAX_POINTER_BYTES: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubscriptionId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallId(usize);

/// The `id` of a message sent to Home Assistant, which numbers them per connection from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RequestId(u64);

/// The connection as the widget should present it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The first attempt since the client started or resumed is under way.
    Connecting,
    Ready,
    /// Home Assistant cannot be reached, or the connection dropped; reconnecting.
    Disconnected,
    /// Home Assistant refused the token. No retry until [`Session::resume`].
    TokenRejected,
    /// The credential slot is unbound, or the host refused the credential to connect or send.
    /// No retry until [`Session::resume`].
    NotConfigured,
    /// Home Assistant sent a message larger than the host accepts,
    /// so asking again would only bring it back.
    /// No retry until [`Session::resume`], which the widget calls itself once it asks for less.
    MessageTooLarge,
    /// The account's address is one the host cannot connect to: a `wss://` one, or one that does not parse.
    /// No retry until [`Session::resume`].
    UnsupportedAddress,
    /// Home Assistant is too old for `subscribe_entities`. No retry until [`Session::resume`].
    Unsupported,
}

/// Why a request was refused before anything was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    TooManySubscriptions,
    NoEntities,
    /// Over [`MAX_ENTITIES_PER_SUBSCRIPTION`] entities in a subscription.
    TooManyEntities,
    TooManyAttributes,
    /// Not shaped `domain.object_id`, or over [`MAX_ENTITY_ID_BYTES`].
    InvalidEntityId,
    /// A domain or service that is not slug-shaped, a command, data key or attribute name that is empty,
    /// any of them over [`MAX_NAME_BYTES`], a data key a command reserves,
    /// or a command the session sends itself.
    InvalidName,
    /// A service data text over [`MAX_SERVICE_DATA_TEXT_BYTES`].
    /// Also a command, data key or text the transport reserves, as [`Setup::reserved`] says.
    InvalidData,
    TooManyReplyValues,
    /// A pointer neither empty nor starting with `/`, or over [`MAX_POINTER_BYTES`].
    InvalidPointer,
    NotConnected,
    TooManyPendingCalls,
}

/// What the session takes from the platform it runs on:
/// what it writes where the transport fills in the account,
/// and what it must never pass on from a caller.
#[derive(Debug, Clone)]
pub struct Setup {
    /// The address to dial, as the transport takes it.
    pub websocket_url: String,
    /// The access token, as the transport takes it in the `auth` message.
    pub token: String,
    /// Whether the transport would act on `text` in an outgoing message.
    /// A request carrying such text from its caller is refused before anything is sent,
    /// so the only text the transport acts on is the session's own.
    pub reserved: fn(&str) -> bool,
}

pub(super) fn owned<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    texts.into_iter().map(str::to_owned).collect()
}

/// Whether a ready connection has shown it stays up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    /// Logged in, but no pong yet: it may drop right after every login.
    Unproven,
    /// A pong came back, so it outlasted a ping interval.
    Proven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// The socket is opening, or open and waiting for `auth_required`.
    Connecting,
    Authenticating,
    Ready {
        awaiting_pong: Option<RequestId>,
        link: Link,
    },
    /// Waiting out the delay before the next attempt.
    Backoff,
    Stopped,
}

/// Whether the caller's socket still needs closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Socket {
    Open,
    Ended,
}

/// Why the connection was lost, which decides how soon to reconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Loss {
    /// A drop, a refused open or an unanswered ping.
    Dropped,
    /// The host closed it for a new account.
    AccountChanged,
}

#[derive(Debug)]
enum Pending {
    Subscribe(SubscriptionId),
    Call(PendingCall),
}

/// Request ids and the requests awaiting a result, on the current connection.
#[derive(Debug)]
struct Requests {
    next: RequestId,
    pending: BTreeMap<RequestId, Pending>,
}

impl Requests {
    fn new() -> Self {
        Self {
            next: RequestId(1),
            pending: BTreeMap::new(),
        }
    }

    /// An id for a request not kept in `pending`.
    fn next_id(&mut self) -> RequestId {
        let id = self.next;
        self.next.0 += 1;
        id
    }

    fn start(&mut self, pending: Pending) -> RequestId {
        let id = self.next_id();
        self.pending.insert(id, pending);
        id
    }
}

#[derive(Debug)]
struct Subscription {
    id: SubscriptionId,
    entities: BTreeMap<String, Entity>,
    attributes: Vec<String>,
    stage: Stage,
}

/// Where a subscription's `subscribe_entities` request stands on the current connection.
/// Home Assistant tags the subscription's events with the request's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Not sent on this connection, or refused.
    NotSent,
    /// Sent; the first event, a snapshot of every entity, has not arrived.
    AwaitingSnapshot(RequestId),
    /// The snapshot arrived, so events are changes.
    Live(RequestId),
}

impl Stage {
    fn request(self) -> Option<RequestId> {
        match self {
            Self::NotSent => None,
            Self::AwaitingSnapshot(id) | Self::Live(id) => Some(id),
        }
    }
}

impl Subscription {
    fn send(&mut self, requests: &mut Requests, effects: &mut Effects) {
        let request = requests.start(Pending::Subscribe(self.id));
        self.stage = Stage::AwaitingSnapshot(request);
        let message = ObjectWriter::new()
            .uint("id", request.0)
            .string("type", "subscribe_entities")
            .strings("entity_ids", self.entities.keys().map(String::as_str))
            .finish();
        effects.commands.push(Command::Send(message));
    }
}

/// The protocol state of one connection.
#[derive(Debug)]
pub struct Session {
    setup: Setup,
    phase: Phase,
    status: Status,
    /// Consecutive failed attempts since a connection last answered a ping.
    failed_attempts: usize,
    requests: Requests,
    subscriptions: Vec<Subscription>,
    next_subscription: usize,
    next_call: usize,
}

impl Session {
    #[must_use]
    pub fn new(setup: Setup) -> Self {
        Self {
            setup,
            phase: Phase::Idle,
            status: Status::Connecting,
            failed_attempts: 0,
            requests: Requests::new(),
            subscriptions: Vec::new(),
            next_subscription: 1,
            next_call: 1,
        }
    }

    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    #[must_use]
    pub fn entity(&self, subscription: SubscriptionId, entity_id: &str) -> Option<&Entity> {
        self.subscriptions
            .iter()
            .find(|sub| sub.id == subscription)?
            .entities
            .get(entity_id)
    }

    /// Whether `event` is still about something the session holds:
    /// a subscription's events stop once it is unsubscribed, even those already produced.
    #[must_use]
    pub fn wants(&self, event: &Event) -> bool {
        match event {
            Event::EntityChanged { subscription, .. }
            | Event::SubscriptionFailed { subscription, .. } => {
                self.subscriptions.iter().any(|sub| sub.id == *subscription)
            }
            Event::StatusChanged(_) | Event::CallFinished { .. } => true,
        }
    }

    /// Open the first connection.
    #[must_use]
    pub fn start(&mut self) -> Effects {
        let mut effects = Effects::default();
        if self.phase == Phase::Idle {
            self.connect(&mut effects);
        }
        effects
    }

    /// Start again after a stop, when the widget's credentials or parameters changed,
    /// and start over an attempt still opening, which may carry the credentials they replaced.
    /// Does nothing while logging in, connected, or waiting to retry.
    ///
    /// The status reads [`Status::Connecting`] at once, with no event for it.
    #[must_use]
    pub fn resume(&mut self) -> Effects {
        let mut effects = Effects::default();
        match self.phase {
            Phase::Stopped => {}
            // The host may already have queued its refusal of the old credentials,
            // which closing the socket discards.
            Phase::Connecting => effects.commands.push(Command::Close),
            Phase::Idle | Phase::Authenticating | Phase::Ready { .. } | Phase::Backoff => {
                return effects;
            }
        }
        self.failed_attempts = 0;
        self.status = Status::Connecting;
        self.connect(&mut effects);
        effects
    }

    /// The socket ended.
    #[must_use]
    pub fn on_close(&mut self, closed: Closed) -> Output {
        let mut out = Output::default();
        match self.phase {
            Phase::Connecting | Phase::Authenticating | Phase::Ready { .. } => {}
            Phase::Idle | Phase::Backoff | Phase::Stopped => return out,
        }
        match closed {
            Closed::CredentialRefused => {
                self.stop(Status::NotConfigured, Socket::Ended, &mut out);
            }
            Closed::MessageTooLarge => self.stop(Status::MessageTooLarge, Socket::Ended, &mut out),
            Closed::UnsupportedScheme | Closed::InvalidRequest => {
                self.stop(Status::UnsupportedAddress, Socket::Ended, &mut out);
            }
            Closed::AccountChanged => {
                self.lose_connection(Socket::Ended, Loss::AccountChanged, &mut out);
            }
            Closed::Dropped => {
                self.lose_connection(Socket::Ended, Loss::Dropped, &mut out);
            }
        }
        out
    }

    #[must_use]
    pub fn on_timer(&mut self) -> Output {
        let mut out = Output::default();
        match self.phase {
            // Too long to log in, or the last tick's ping unanswered.
            Phase::Connecting
            | Phase::Authenticating
            | Phase::Ready {
                awaiting_pong: Some(_),
                ..
            } => self.lose_connection(Socket::Open, Loss::Dropped, &mut out),
            Phase::Backoff => self.connect(&mut out.effects),
            Phase::Ready {
                awaiting_pong: None,
                link,
            } => self.tick(link, &mut out),
            Phase::Idle | Phase::Stopped => out.effects.wake = Some(Wake::Never),
        }
        out
    }

    #[must_use]
    pub fn on_message(&mut self, message: &impl JsonLookup) -> Output {
        let mut out = Output::default();
        let Some(kind) = message.str("/type") else {
            return out;
        };
        let id = message
            .i64("/id")
            .and_then(|id| u64::try_from(id).ok())
            .map(RequestId);
        match (kind.as_str(), self.phase) {
            ("auth_required", Phase::Connecting) => {
                self.phase = Phase::Authenticating;
                let auth = ObjectWriter::new()
                    .string("type", "auth")
                    .string("access_token", &self.setup.token)
                    .finish();
                out.effects.commands.push(Command::Send(auth));
            }
            ("auth_ok", Phase::Authenticating) => self.become_ready(&mut out),
            ("auth_invalid", Phase::Authenticating) => {
                self.stop(Status::TokenRejected, Socket::Open, &mut out);
            }
            ("pong", Phase::Ready { awaiting_pong, .. }) if id.is_some() && id == awaiting_pong => {
                // Only now, not at `auth_ok`: a server that drops every socket
                // right after the login must not be redialled at once, over and over.
                self.failed_attempts = 0;
                self.phase = Phase::Ready {
                    awaiting_pong: None,
                    link: Link::Proven,
                };
            }
            ("result", Phase::Ready { .. }) => {
                if let Some(id) = id {
                    self.handle_result(id, message, &mut out);
                }
            }
            ("event", Phase::Ready { .. }) => {
                if let Some(id) = id {
                    self.handle_event(id, message, &mut out);
                }
            }
            _ => {}
        }
        out
    }

    /// Follow `entities`, keeping only the `attributes` named.
    /// Sent at once when connected, otherwise after the next login.
    pub fn subscribe(
        &mut self,
        entities: &[&str],
        attributes: &[&str],
    ) -> Result<(SubscriptionId, Effects), RequestError> {
        if self.subscriptions.len() >= MAX_SUBSCRIPTIONS {
            return Err(RequestError::TooManySubscriptions);
        }
        let entity_ids = distinct(entities);
        check_entity_ids(&entity_ids)?;
        let attributes: BTreeSet<&str> = attributes.iter().copied().collect();
        if attributes.len() > MAX_ATTRIBUTES_PER_SUBSCRIPTION {
            return Err(RequestError::TooManyAttributes);
        }
        if !attributes
            .iter()
            .all(|name| !name.is_empty() && name.len() <= MAX_NAME_BYTES)
        {
            return Err(RequestError::InvalidName);
        }

        let id = SubscriptionId(self.next_subscription);
        self.next_subscription += 1;
        let mut subscription = Subscription {
            id,
            entities: entity_ids
                .into_iter()
                .map(|entity| (entity.to_owned(), Entity::awaiting()))
                .collect(),
            attributes: owned(attributes.iter().copied()),
            stage: Stage::NotSent,
        };
        let mut effects = Effects::default();
        if matches!(self.phase, Phase::Ready { .. }) {
            subscription.send(&mut self.requests, &mut effects);
        }
        self.subscriptions.push(subscription);
        Ok((id, effects))
    }

    /// Stop following a subscription's entities. Its later events are ignored.
    #[must_use]
    pub fn unsubscribe(&mut self, subscription: SubscriptionId) -> Effects {
        let mut effects = Effects::default();
        let Some(index) = self
            .subscriptions
            .iter()
            .position(|sub| sub.id == subscription)
        else {
            return effects;
        };
        let removed = self.subscriptions.remove(index);
        if let (Some(request), Phase::Ready { .. }) = (removed.stage.request(), self.phase) {
            let message = ObjectWriter::new()
                .uint("id", self.requests.next_id().0)
                .string("type", "unsubscribe_events")
                .uint("subscription", request.0)
                .finish();
            effects.commands.push(Command::Send(message));
        }
        self.requests
            .pending
            .retain(|_, pending| !matches!(pending, Pending::Subscribe(id) if *id == subscription));
        effects
    }

    /// Call `domain.service` on `entity_id`.
    /// It targets exactly one entity: services with no target,
    /// or with several entities, an area or a device, are not supported.
    /// The result arrives as [`Event::CallFinished`], and a call is never sent twice:
    /// one cut off by a dropped connection fails as [`CallFailure::Disconnected`].
    /// One left unanswered fails as [`CallFailure::TimedOut`].
    pub fn call_service(
        &mut self,
        domain: &str,
        service: &str,
        entity_id: &str,
        data: &ServiceData,
    ) -> Result<(CallId, Effects), RequestError> {
        if !is_name(domain) || !is_name(service) {
            return Err(RequestError::InvalidName);
        }
        let data = data.checked(self.setup.reserved)?;
        if !is_entity_id(entity_id) {
            return Err(RequestError::InvalidEntityId);
        }
        let (call, request) = self.start_call(Wanted::Nothing)?;
        let mut message = ObjectWriter::new()
            .uint("id", request.0)
            .string("type", "call_service")
            .string("domain", domain)
            .string("service", service)
            .object("target", ObjectWriter::new().string("entity_id", entity_id));
        if !data.is_empty() {
            message = message.object("service_data", data.write_into(ObjectWriter::new()));
        }

        Ok((call, Effects::send(message.finish())))
    }

    /// Send the one-off command `command`, with `data` beside its id and type,
    /// such as `config/entity_registry/get` with the `entity_id` to look up.
    /// The result arrives as [`Event::CallFinished`], its [`Reply`](super::Reply) holding the values found at `wanted`:
    /// JSON Pointers into the command's `result`, as a lookup cannot list its keys.
    /// Like a service call, a command is never sent twice, and times out as one does.
    ///
    /// For one-off requests only.
    /// A command that streams, such as `subscribe_events` or `render_template`, is not supported:
    /// the client drops its events, and Home Assistant keeps sending them until the connection ends.
    /// `auth` and `ping` are the session's own, so they are refused:
    /// it logs in and keeps the connection alive by itself.
    pub fn command(
        &mut self,
        command: &str,
        data: &ServiceData,
        wanted: &[&str],
    ) -> Result<(CallId, Effects), RequestError> {
        if !is_free_name(command)
            || matches!(command, "auth" | "ping")
            || data.has_key("id")
            || data.has_key("type")
        {
            return Err(RequestError::InvalidName);
        }
        if (self.setup.reserved)(command) {
            return Err(RequestError::InvalidData);
        }
        let data = data.checked(self.setup.reserved)?;
        if wanted.len() > MAX_REPLY_VALUES {
            return Err(RequestError::TooManyReplyValues);
        }
        if !wanted.iter().all(|pointer| is_pointer(pointer)) {
            return Err(RequestError::InvalidPointer);
        }
        self.send_command(
            command,
            &data,
            Wanted::Values(owned(wanted.iter().copied())),
        )
    }

    fn send_command(
        &mut self,
        command: &str,
        data: &CheckedData<'_>,
        wanted: Wanted,
    ) -> Result<(CallId, Effects), RequestError> {
        let (call, request) = self.start_call(wanted)?;
        let message = data.write_into(
            ObjectWriter::new()
                .uint("id", request.0)
                .string("type", command),
        );

        Ok((call, Effects::send(message.finish())))
    }

    fn start_call(&mut self, wanted: Wanted) -> Result<(CallId, RequestId), RequestError> {
        if !matches!(self.phase, Phase::Ready { .. }) {
            return Err(RequestError::NotConnected);
        }
        let calls = self
            .requests
            .pending
            .values()
            .filter(|pending| matches!(pending, Pending::Call(_)))
            .count();
        if calls >= MAX_PENDING_CALLS {
            return Err(RequestError::TooManyPendingCalls);
        }
        let call = CallId(self.next_call);
        self.next_call += 1;
        let request = self.requests.start(Pending::Call(PendingCall {
            id: call,
            ticks_waited: 0,
            wanted,
        }));
        Ok((call, request))
    }

    fn set_status(&mut self, status: Status, out: &mut Output) {
        if self.status != status {
            self.status = status;
            out.events.push(Event::StatusChanged(status));
        }
    }

    fn connect(&mut self, effects: &mut Effects) {
        // Home Assistant numbers requests per connection.
        self.requests.next = RequestId(1);
        self.phase = Phase::Connecting;
        effects.commands.push(Command::Connect {
            url: self.setup.websocket_url.clone(),
        });
        effects.wake = Some(Wake::In(CONNECT_TIMEOUT_SECS));
    }

    fn become_ready(&mut self, out: &mut Output) {
        self.phase = Phase::Ready {
            awaiting_pong: None,
            link: Link::Unproven,
        };
        self.set_status(Status::Ready, out);
        for subscription in &mut self.subscriptions {
            subscription.send(&mut self.requests, &mut out.effects);
        }
        out.effects.wake = Some(Wake::In(TICK_SECS));
    }

    /// A keepalive tick with the last ping answered: age the calls, and ping again.
    fn tick(&mut self, link: Link, out: &mut Output) {
        self.requests.pending.retain(|_, pending| match pending {
            Pending::Call(call) => {
                call.ticks_waited += 1;
                let timed_out = call.ticks_waited >= CALL_TIMEOUT_TICKS;
                if timed_out {
                    out.events.push(Event::CallFinished {
                        call: call.id,
                        result: Err(CallFailure::TimedOut),
                    });
                }
                !timed_out
            }
            Pending::Subscribe(_) => true,
        });

        let id = self.requests.next_id();
        self.phase = Phase::Ready {
            awaiting_pong: Some(id),
            link,
        };
        out.effects.commands.push(Command::Send(
            ObjectWriter::new()
                .uint("id", id.0)
                .string("type", "ping")
                .finish(),
        ));
        out.effects.wake = Some(Wake::In(TICK_SECS));
    }

    fn handle_result(&mut self, id: RequestId, message: &impl JsonLookup, out: &mut Output) {
        let Some(pending) = self.requests.pending.remove(&id) else {
            return;
        };
        let result = if message.bool("/success") == Some(true) {
            Ok(())
        } else {
            Err(HaError {
                code: message
                    .str("/error/code")
                    .unwrap_or_else(|| UNKNOWN_ERROR.to_owned()),
                message: message.str("/error/message").unwrap_or_default(),
            })
        };
        match (pending, result) {
            (Pending::Subscribe(_), Ok(())) => {}
            (Pending::Subscribe(_), Err(error)) if error.code == "unknown_command" => {
                self.stop(Status::Unsupported, Socket::Open, out);
            }
            (Pending::Subscribe(subscription), Err(error)) => {
                if let Some(sub) = self
                    .subscriptions
                    .iter_mut()
                    .find(|sub| sub.id == subscription)
                {
                    sub.stage = Stage::NotSent;
                }
                out.events.push(Event::SubscriptionFailed {
                    subscription,
                    error,
                });
            }
            (Pending::Call(call), result) => {
                let result = match result {
                    Ok(()) => Ok(call.reply(message)),
                    Err(error) => Err(CallFailure::Rejected(error)),
                };
                out.events.push(Event::CallFinished {
                    call: call.id,
                    result,
                });
            }
        }
    }

    fn handle_event(&mut self, id: RequestId, message: &impl JsonLookup, out: &mut Output) {
        let Some(sub) = self
            .subscriptions
            .iter_mut()
            .find(|sub| sub.stage.request() == Some(id))
        else {
            return;
        };
        let mut event = EntityEvent::new(message, sub.stage == Stage::AwaitingSnapshot(id));
        sub.stage = Stage::Live(id);
        for (entity_id, entity) in &mut sub.entities {
            if entity.apply(entity_id, &sub.attributes, &mut event) {
                out.events.push(Event::EntityChanged {
                    subscription: sub.id,
                    entity_id: entity_id.clone(),
                });
            }
        }
    }

    /// End the current session: fail its calls, forget its requests,
    /// and keep the entities' last values, marked stale.
    fn end_session(&mut self, out: &mut Output) {
        // In the order they were sent: ids only grow on a connection.
        for pending in std::mem::take(&mut self.requests.pending).into_values() {
            match pending {
                Pending::Call(call) => out.events.push(Event::CallFinished {
                    call: call.id,
                    result: Err(CallFailure::Disconnected),
                }),
                Pending::Subscribe(_) => {}
            }
        }
        for sub in &mut self.subscriptions {
            sub.stage = Stage::NotSent;
            for (entity_id, entity) in &mut sub.entities {
                if entity.go_stale() {
                    out.events.push(Event::EntityChanged {
                        subscription: sub.id,
                        entity_id: entity_id.clone(),
                    });
                }
            }
        }
    }

    /// The connection is gone, or must go.
    /// One that answered a ping, or one the account change closed, is retried at once,
    /// as the Home Assistant JS client retries; any other attempt counts as failed
    /// and waits longer each time, up to [`MAX_BACKOFF_SECS`].
    fn lose_connection(&mut self, socket: Socket, loss: Loss, out: &mut Output) {
        let was_up = matches!(
            self.phase,
            Phase::Ready {
                link: Link::Proven,
                ..
            }
        );
        if socket == Socket::Open {
            out.effects.commands.push(Command::Close);
        }
        self.end_session(out);
        self.set_status(Status::Disconnected, out);

        if was_up || loss == Loss::AccountChanged {
            self.failed_attempts = 0;
            self.connect(&mut out.effects);
        } else {
            self.failed_attempts += 1;
            self.phase = Phase::Backoff;
            let backoff_secs = u32::try_from(self.failed_attempts)
                .map_or(MAX_BACKOFF_SECS, |attempts| attempts.min(MAX_BACKOFF_SECS));
            out.effects.wake = Some(Wake::In(backoff_secs));
        }
    }

    fn stop(&mut self, status: Status, socket: Socket, out: &mut Output) {
        if socket == Socket::Open {
            out.effects.commands.push(Command::Close);
        }
        self.end_session(out);
        self.phase = Phase::Stopped;
        self.set_status(status, out);
        out.effects.wake = Some(Wake::Never);
    }
}

#[cfg(test)]
mod tests;
