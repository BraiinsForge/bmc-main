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

//! Runs the protocol core on the widget SDK: its WebSocket, one timer per client,
//! and the host's JSON parser.
//!
//! The widget holds a [`Client`] and its [`Subscription`]s, and never the socket.
//! Dropping a handle ends what it stands for.

use core::cell::RefCell;
use core::mem::ManuallyDrop;

use bmc_wasm_sdk::json::JsonDoc;
use bmc_wasm_sdk::log_warn;
use bmc_wasm_sdk::timer;
use bmc_wasm_sdk::timer::Timer;
// `log_warn!` expands to `ufmt` macros that name the crate at the call site.
use bmc_wasm_sdk::ufmt;
use bmc_wasm_sdk::ws::{Ws, WsEvent, WsHostClose, ws_connect};

use crate::credential;
use crate::proto;
use crate::proto::{
    CallId, Closed, Command, Effects, Entity, Event, JsonLookup, Output, RequestError, ServiceData,
    Status, SubscriptionId, Wake,
};

/// Receives a client's events, with the client they came from.
///
/// It runs only from the client's socket and timer callbacks, never inside a call the widget made,
/// so it may borrow whatever the widget's state needs, and call back into the client,
/// such as to press a button on a state change.
pub type EventCallback = fn(&Client, &Event);

/// The period a client's timer waits at while the core wants no wake, about 49 days.
/// The client keeps its timer rather than stopping it,
/// so a later wake cannot find the widget's timers all taken.
const PARKED_MS: u32 = u32::MAX;

struct Running {
    core: proto::Session,
    ws: Option<Ws>,
    timer: Timer,
    on_event: EventCallback,
}

thread_local! {
    /// Indexed by [`ClientId`].
    /// A dropped client leaves `None`, and its index is never reused, so a stale [`Subscription`] reaches nothing.
    static CLIENTS: RefCell<Vec<Option<Running>>> = const { RefCell::new(Vec::new()) };
}

/// Where a client sits in [`CLIENTS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClientId(usize);

/// A connection to the Home Assistant account bound to one credential slot.
/// Dropping it closes the connection.
#[derive(Debug, PartialEq, Eq)]
pub struct Client(ClientId);

/// Entities followed through a [`Client`]. Dropping it stops following them.
#[derive(Debug, PartialEq, Eq)]
pub struct Subscription {
    client: ClientId,
    id: SubscriptionId,
}

/// Why [`connect`] made no client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectError {
    /// The widget already runs as many timers as the host allows,
    /// and the client needs one of its own until it is dropped.
    NoTimerAvailable,
}

/// Connect to the Home Assistant account bound to the credential slot `slot`.
/// The client holds one of the widget's timers until it is dropped.
///
/// An unbound slot, one the manifest does not declare,
/// or a credential the host refuses stops the client as [`Status::NotConfigured`],
/// reported as an event once the host refuses it, after this returns.
/// Call [`Client::resume`] from `on_credentials_update` and `on_params_update`
/// so that it starts again once the widget is configured.
pub fn connect(slot: &str, on_event: EventCallback) -> Result<Client, ConnectError> {
    let core = proto::Session::new(setup(slot));
    let timer = timer::start(PARKED_MS, on_timer).ok_or(ConnectError::NoTimerAvailable)?;
    let id = CLIENTS.with_borrow_mut(|clients| {
        clients.push(Some(Running {
            core,
            ws: None,
            timer,
            on_event,
        }));
        ClientId(clients.len() - 1)
    });
    with_running(id, |running| running.call(|core| ((), core.start())));
    Ok(Client(id))
}

impl Client {
    #[must_use]
    pub fn status(&self) -> Status {
        with_running(self.0, |running| running.core.status()).unwrap_or(Status::NotConfigured)
    }

    /// Follow `entities`, keeping only the `attributes` named.
    pub fn subscribe(
        &self,
        entities: &[&str],
        attributes: &[&str],
    ) -> Result<Subscription, RequestError> {
        let id = self.request(|core| core.subscribe(entities, attributes))?;
        Ok(Subscription { client: self.0, id })
    }

    /// Call `domain.service` on `entity_id`; the result arrives as [`Event::CallFinished`].
    pub fn call_service(
        &self,
        domain: &str,
        service: &str,
        entity_id: &str,
        data: &ServiceData,
    ) -> Result<CallId, RequestError> {
        self.request(|core| core.call_service(domain, service, entity_id, data))
    }

    /// Send the one-off command `command`; the values found at `wanted`
    /// in its result arrive as [`Event::CallFinished`].
    pub fn command(
        &self,
        command: &str,
        data: &ServiceData,
        wanted: &[&str],
    ) -> Result<CallId, RequestError> {
        self.request(|core| core.command(command, data, wanted))
    }

    /// Start again after a stop, or start over an attempt still opening,
    /// as [`proto::Session::resume`] says.
    ///
    /// The status reads [`Status::Connecting`] once this returns, with no event for it,
    /// so read it again here.
    pub fn resume(&self) {
        with_running(self.0, |running| running.call(|core| ((), core.resume())));
    }

    /// A dropped client reads as not connected.
    fn request<T>(
        &self,
        request: impl FnOnce(&mut proto::Session) -> Result<(T, Effects), RequestError>,
    ) -> Result<T, RequestError> {
        with_running(self.0, |running| {
            running.call(|core| match request(core) {
                Ok((value, effects)) => (Ok(value), effects),
                Err(error) => (Err(error), Effects::default()),
            })
        })
        .unwrap_or(Err(RequestError::NotConnected))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        CLIENTS.with_borrow_mut(|clients| {
            if let Some(running) = clients.get_mut(self.0.0) {
                *running = None;
            }
        });
    }
}

impl Subscription {
    #[must_use]
    pub fn id(&self) -> SubscriptionId {
        self.id
    }

    #[must_use]
    pub fn entity(&self, entity_id: &str) -> Option<Entity> {
        with_running(self.client, |running| {
            running.core.entity(self.id, entity_id).cloned()
        })
        .flatten()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let id = self.id;
        with_running(self.client, |running| {
            running.call(|core| ((), core.unsubscribe(id)));
        });
    }
}

/// The session for the account bound to the credential slot `slot`, on the widget SDK.
fn setup(slot: &str) -> proto::Setup {
    proto::Setup {
        websocket_url: credential::placeholder(slot, "websocket_url"),
        token: credential::placeholder(slot, "token"),
        reserved: credential::names_credential,
    }
}

/// `f` on the client `id`, `None` once it is dropped.
fn with_running<R>(id: ClientId, f: impl FnOnce(&mut Running) -> R) -> Option<R> {
    CLIENTS.with_borrow_mut(|clients| clients.get_mut(id.0)?.as_mut().map(f))
}

fn find_client(matches: impl Fn(&Running) -> bool) -> Option<ClientId> {
    CLIENTS.with_borrow(|clients| {
        clients
            .iter()
            .position(|running| running.as_ref().is_some_and(&matches))
            .map(ClientId)
    })
}

/// Hand the widget the events a step of client `id` produced.
/// Runs with the registry released, since the callback may call back into the client,
/// and skips an event about a client or subscription the widget dropped during an earlier one.
fn dispatch(id: ClientId, (on_event, events): (EventCallback, Vec<Event>)) {
    // Borrowed for the events only; dropping it would close the connection.
    let client = ManuallyDrop::new(Client(id));
    for event in &events {
        match with_running(id, |running| running.core.wants(event)) {
            None => return,
            Some(false) => {}
            Some(true) => on_event(&client, event),
        }
    }
}

impl Running {
    /// One step of the core for a call the widget made, with what it asks carried out.
    /// Such a step reports no events.
    fn call<R>(&mut self, step: impl FnOnce(&mut proto::Session) -> (R, Effects)) -> R {
        let (result, effects) = step(&mut self.core);
        self.carry_out(effects);
        result
    }

    /// One step of the core for its socket or timer, with what it asks carried out,
    /// returning the events to hand the widget.
    fn step(
        &mut self,
        step: impl FnOnce(&mut proto::Session) -> Output,
    ) -> (EventCallback, Vec<Event>) {
        let out = step(&mut self.core);
        self.carry_out(out.effects);
        (self.on_event, out.events)
    }

    /// Carry out the commands and the wake, in order.
    fn carry_out(&mut self, effects: Effects) {
        for command in effects.commands {
            match command {
                Command::Connect { url } => {
                    // `None` past the widget's socket limit. Nothing answers then,
                    // so the core's connect timeout retries it, never this call.
                    self.ws = ws_connect(&url, None, on_ws_event);
                }
                Command::Send(text) => {
                    if let Some(ws) = &self.ws {
                        ws.send(&text);
                    }
                }
                Command::Close => self.ws = None,
            }
        }
        self.set_wake(effects.wake);
    }

    fn set_wake(&self, wake: Option<Wake>) {
        let period_ms = match wake {
            None => return,
            Some(Wake::Never) => PARKED_MS,
            Some(Wake::In(secs)) => secs.saturating_mul(1000),
        };
        let set = self.timer.set_period(period_ms);
        debug_assert!(
            set,
            "BUG: every period the client sets is a second or more, above the host minimum"
        );
        if !set {
            log_warn!("home-assistant: the host refused the client's timer period");
        }
    }
}

fn on_ws_event(ws: &Ws, event: &WsEvent<'_>) {
    let Some(id) = find_client(|running| running.ws.as_ref() == Some(ws)) else {
        return;
    };
    match event {
        // Home Assistant speaks first: the core waits for `auth_required`.
        WsEvent::Open => {}
        WsEvent::Message(text) => {
            let message = JsonDoc::parse(text.as_bytes());
            if let Some(stepped) =
                with_running(id, |running| running.step(|core| core.on_message(&message)))
            {
                dispatch(id, stepped);
            }
        }
        WsEvent::Close(_) => socket_ended(id, Closed::Dropped),
        WsEvent::HostClosed(reason) => socket_ended(
            id,
            match reason {
                WsHostClose::Credential => Closed::CredentialRefused,
                WsHostClose::AccountChanged => Closed::AccountChanged,
                WsHostClose::MessageTooLarge => Closed::MessageTooLarge,
                WsHostClose::UnsupportedScheme => Closed::UnsupportedScheme,
                WsHostClose::InvalidRequest => Closed::InvalidRequest,
            },
        ),
    }
}

fn socket_ended(id: ClientId, closed: Closed) {
    let stepped = with_running(id, |running| {
        // Dropping the socket only tells the host; nothing calls back into the widget.
        running.ws = None;
        running.step(|core| core.on_close(closed))
    });
    if let Some(stepped) = stepped {
        dispatch(id, stepped);
    }
}

fn on_timer(timer: &Timer, _elapsed_periods: u32) {
    let Some(id) = find_client(|running| running.timer == *timer) else {
        return;
    };
    if let Some(stepped) = with_running(id, |running| running.step(proto::Session::on_timer)) {
        dispatch(id, stepped);
    }
}

impl JsonLookup for JsonDoc {
    fn str(&self, path: &str) -> Option<String> {
        JsonDoc::str(self, path)
    }

    fn str_max(&self, path: &str, max: usize) -> Option<String> {
        JsonDoc::str_max(self, path, max)
    }

    fn i64(&self, path: &str) -> Option<i64> {
        JsonDoc::i64(self, path)
    }

    fn f64(&self, path: &str) -> Option<f64> {
        JsonDoc::f64(self, path)
    }

    fn bool(&self, path: &str) -> Option<bool> {
        JsonDoc::bool(self, path)
    }

    fn len(&self, path: &str) -> Option<usize> {
        JsonDoc::len(self, path)
    }

    fn exists(&self, path: &str) -> bool {
        JsonDoc::kind(self, path).is_some()
    }
}
