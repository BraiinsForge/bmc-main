# home-assistant

A Home Assistant client for WASM widgets, over Home Assistant's WebSocket API. It follows entities, calls services,
sends one-off commands and reads history and long-term statistics. It needs Home Assistant 2024.3 or later
(`MIN_VERSION`).

The rustdoc is the reference (`cargo doc -p home-assistant --open`); this page is the overview.

## Layout

| Module       | What it is                                                                                                      |
| ------------ | --------------------------------------------------------------------------------------------------------------- |
| `proto`      | The protocol core, with no socket, clock or SDK: it reads messages through `JsonLookup`, which tests implement. |
| `adapter`    | Runs the core on the widget SDK's WebSocket, a timer and the host JSON parser. `wasm32` only.                   |
| `credential` | The host's `{{ credential.… }}` placeholders, the only part that knows them. Private.                           |
| `format`     | Numbers as the Home Assistant frontend shows them: decimals, display precision, unit placement.                 |

A widget uses the crate root, which re-exports everything it handles. It never touches `proto::Session` directly.

## Credentials

The client connects to the account bound to a credential slot of type `home-assistant`, declared in the widget manifest.
The slot's key is the name `connect` takes. A name the manifest does not declare stops the client as
`Status::NotConfigured`, as an unbound slot does:

```json
{
  "credentials": {
    "ha": { "type": "home-assistant", "label": "Home Assistant" }
  }
}
```

The host substitutes the WebSocket URL and the token, so the widget never sees either. See
[credentials](../../../docs/devel/wasm-widgets/credentials.md). The host cannot dial `wss://` yet, so an account refuses
an `https://` address when it is saved, and a `wss://` URL stops the client as `Status::UnsupportedAddress`.

## Using it

```rust
use std::cell::RefCell;

use bmc_wasm_sdk::*;
use home_assistant::{Client, ConnectError, Event, Presence, Subscription};

struct Ha {
    client: Client,
    outdoor: Subscription,
}

thread_local! {
    static HA: RefCell<Option<Ha>> = const { RefCell::new(None) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let client = match home_assistant::connect("ha", on_ha_event) {
        Ok(client) => client,
        Err(ConnectError::NoTimerAvailable) => {
            log_error!("my-widget: no timer left for the Home Assistant client");
            return;
        }
    };
    match client.subscribe(&["sensor.outdoor"], &["unit_of_measurement"]) {
        Ok(outdoor) => HA.set(Some(Ha { client, outdoor })),
        Err(_) => log_error!("my-widget: Home Assistant subscription refused"),
    }
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn on_credentials_update() {
    HA.with_borrow(|ha| {
        if let Some(ha) = ha {
            ha.client.resume();
        }
    });
    request_frame();
}
// on_params_update: the same

fn on_ha_event(_client: &Client, event: &Event) {
    match event {
        Event::StatusChanged(_) | Event::EntityChanged { .. } => request_frame(),
        Event::SubscriptionFailed { .. } => { /* Home Assistant refused it */ }
        Event::CallFinished { .. } => { /* match `call` against the CallId you kept */ }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn render(_delta_ms: u32) {
    let Some((status, outdoor)) = HA.with_borrow(|ha| {
        let ha = ha.as_ref()?;
        Some((ha.client.status(), ha.outdoor.entity("sensor.outdoor")?))
    }) else {
        return; // `init` failed: draw the widget's own error
    };
    match (outdoor.presence, outdoor.state.as_deref()) {
        (Presence::Present, Some(state)) => { /* draw `state` */ }
        (Presence::Stale, Some(state)) => { /* draw `state` dimmed: the last value seen */ }
        (Presence::Missing, _) => { /* Home Assistant has no `sensor.outdoor` */ }
        _ => { /* nothing yet: draw `status` (Connecting, Disconnected, NotConfigured, …) */ }
    }
}
```

Draw a value as soon as there is one, stale or not, and fall back to the status only while there is nothing to show.

- **Handles own the resources.** Dropping the `Client` closes the connection; dropping a `Subscription` unsubscribes.
  Keep both in the widget's state.
- **The callback is a plain `fn`**: it captures nothing, and reaches the widget's state as the widget's other callbacks
  do. It runs only from the client's socket and timer callbacks, never inside a call the widget made, so borrowing that
  state there cannot panic, and it may call back into the client.
- **Call `client.resume()` from `on_credentials_update` and `on_params_update`.** An unbound slot, a rejected token or
  an address the host cannot dial (`Status::UnsupportedAddress`) stops the client until then. A release older than
  `MIN_VERSION` (`Status::Unsupported`) is also checked again every 5 minutes, before any login, so an upgrade is picked
  up on its own. `resume` sets `Status::Connecting` without an event.
- **Resume after `Status::MessageTooLarge` yourself.** A message larger than the host accepts stops the client too, and
  waiting for a setting to change would leave it stopped: ask for less, then call `client.resume()`.
- **Batch redraws.** A widget following many entities should answer each `EntityChanged` with `request_frame_after`
  instead, which merges the requests before a frame into one.

## Requests

| Method                                                 | Answer                                                 |
| ------------------------------------------------------ | ------------------------------------------------------ |
| `subscribe(entities, attributes)`                      | `Subscription`; then `EntityChanged` events            |
| `call_service(domain, service, entity_id, data)`       | `CallFinished` with `Reply::Done`                      |
| `command(type, data, json_pointers)`                   | `CallFinished` with `Reply::Values` at those pointers  |
| `display_precision(entity_id)`                         | `CallFinished` with `Reply::Precision`                 |
| `history(entity_ids, start_secs, end_secs)`            | `CallFinished` with `Reply::History`                   |
| `statistics(entity_ids, start_secs, end_secs, period)` | `CallFinished` with `Reply::Statistics` (mean/min/max) |

A `command` pointer must reach a text, number, boolean or list. An object or `null` there reads as absent from
`Values::get`, as does a value over the attribute caps in [Limits](#limits); point inside an object instead.

Times are seconds since the Unix epoch; an `end_secs` of `None` reads up to now. `period` is a `Period`, the span each
statistic covers. `Reply::History` and `Reply::Statistics` map each entity id to a `Series`: its `points` oldest first,
as Home Assistant answered them, and `dropped`, how many of the oldest the client left out to stay within
`MAX_HISTORY_POINTS`, so a shorter period would show them. Where a series begins is its first point's time: Home
Assistant times the state at the start of the period there, and has nothing earlier for an entity that begins later. An
entity with nothing in the period is left out of the map.

`ServiceData` is a builder for the extra fields of a call or command: `.text()`, `.integer()`,
`.decimal(key, v, places)`, `.boolean()`, and list forms of the first three. A key, like a command type, is any text up
to `MAX_NAME_BYTES`. A key or text that names a credential the way the host reads one (`{{ credential.… }}`) is refused
as `RequestError::InvalidData`, so text a widget passes on, such as an option name a device chose, can neither stop the
client nor have the host fill in the token. Other braces, Jinja included, are sent as written.

A subscription made while disconnected is sent after the next login, and survives reconnects. A call needs a live
connection, otherwise it fails at once with `RequestError::NotConnected`. A call is never sent twice: one cut off by a
dropped connection fails as `CallFailure::Disconnected`, one unanswered for 30–60 s as `CallFailure::TimedOut`. After a
`Disconnected`, repeat only a `history` or `statistics` request, and only once the status is back to `Ready`: a service
call or a `command` may have run. After `MessageTooLarge` every pending call fails this way, though only one message, a
reply or a subscription's event, was too large: ask for a shorter time range or fewer entities, then resume.

A service call targets exactly one entity. `command` is for one-off requests only; a streaming command such as
`subscribe_events` is not supported.

## Entities

`Subscription::entity(id)` returns an `Option<Entity>`, `None` for an id it does not follow or a client that is gone:

- `presence`: `Awaiting` until the first snapshot, then `Present` or `Missing`; `Stale` once the connection drops, with
  the last values seen, until the next snapshot.
- `state`: the raw state string (`on`, `21.5`, `unavailable`). Parse a number with `format::state_number`; whether the
  frontend would show it as a number, rather than as written, is `format::is_numeric`'s call.
- `last_changed`: Unix seconds; an attribute changing alone does not move it.
- Attributes through `.text()`, `.number()`, `.boolean()`, `.list()`, `.numbers()`. Only the attributes the subscription
  named are kept. Objects are dropped, and so are arrays that mix types.

## Limits

Everything the client holds is bounded. A request past one of these is refused with a `RequestError` before anything is
sent:

| Constant                          | Value | Bounds                                                      |
| --------------------------------- | ----- | ----------------------------------------------------------- |
| `MAX_SUBSCRIPTIONS`               | 8     | subscriptions a client holds                                |
| `MAX_ENTITIES_PER_SUBSCRIPTION`   | 32    | entities in a subscription, a history or statistics request |
| `MAX_ATTRIBUTES_PER_SUBSCRIPTION` | 16    | attributes a subscription keeps                             |
| `MAX_PENDING_CALLS`               | 8     | calls awaiting an answer                                    |
| `MAX_REPLY_VALUES`                | 16    | pointers in a `command`                                     |
| `MAX_ENTITY_ID_BYTES`             | 255   | an entity id                                                |
| `MAX_NAME_BYTES`                  | 64    | a domain, service, command type, data key or attribute name |
| `MAX_SERVICE_DATA_TEXT_BYTES`     | 1 KiB | a service data text, list items included                    |
| `MAX_SERVICE_DATA_LIST_ITEMS`     | 32    | a service data list                                         |
| `MAX_POINTER_BYTES`               | 255   | a `command` pointer                                         |

These trim what the client keeps instead:

| Constant                   | Value                                   |
| -------------------------- | --------------------------------------- |
| `MAX_HISTORY_POINTS`       | 1 000 (latest, shared between entities) |
| `MAX_ATTRIBUTE_TEXT_BYTES` | 1 KiB                                   |
| `MAX_ATTRIBUTE_LIST_BYTES` | 4 KiB                                   |
| `MAX_ATTRIBUTE_NUMBERS`    | 128                                     |

An attribute over its cap reads as absent. A reply over the host's WebSocket message cap (4 MiB by default) stops the
client as `Status::MessageTooLarge`, so keep the time range short for a sensor that changes often.

Each client holds one of the widget's timers and one socket until it is dropped; `connect` fails with
`ConnectError::NoTimerAvailable` when the widget has no timer left.
