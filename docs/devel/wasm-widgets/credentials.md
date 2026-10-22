# WASM Widget Credentials

A credential slot is a widget's declaration that it needs an account of some kind — a pool token, an API key, a username
and password. The operator binds one of their saved accounts to the slot, and the device puts the secret into the
widget's outbound requests on its behalf.

The widget never receives the secret. It embeds a placeholder, and the host substitutes the value at the moment the
request leaves. This is the only supported way for a widget to authenticate; see
[why a param will not do](params.md#never-put-a-secret-in-a-param).

## Declare a Slot

Declare slots in the manifest's `credentials` object, beside `params`. The key is the slot name the operator sees bound
to an account, and the name your placeholders use.

```json
{
  "credentials": {
    "pool": {
      "type": "braiins-pool",
      "label": "Pool account",
      "description": "Used to read your worker stats",
      "required": true
    },
    "weather": {
      "type": "generic-token",
      "label": "Weather service"
    },
    "home": {
      "type": "home-assistant",
      "label": "Home Assistant"
    }
  }
}
```

`type` names one of the account kinds the firmware provides:

| Type               | Configured fields      | Spendable fields                | Egress                               |
| ------------------ | ---------------------- | ------------------------------- | ------------------------------------ |
| `generic-token`    | `token`                | `token`                         | anywhere                             |
| `generic-userpass` | `username`, `password` | `username`, `password`, `basic` | anywhere                             |
| `braiins-pool`     | `token`                | `token`                         | pinned to `api.braiins.com`          |
| `local-file-token` | `path`                 | `token`                         | anywhere                             |
| `home-assistant`   | `url`, `token`         | `url`, `token`, `websocket_url` | pinned to the host and port of `url` |

A slot's placeholders are generated for the **spendable** fields. For `local-file-token` the operator configures a file
path on the device and bmc reads the token from it (polled every 5 s), so the widget spends `token` and never sees the
path. While the file is missing or unusable the slot stays bound but its `token` is absent, and a fetch using the
placeholder is refused like any unknown field.

For `generic-userpass`, `basic` is `username:password` in base64, the credential HTTP Basic auth sends. The widget
cannot encode a placeholder itself, so bmc does it: send `Authorization: Basic {{ credential.<slot>.basic }}`.

For `home-assistant`, `url` is the instance's `http://` address, stored as its bare origin such as
`http://homeassistant.local:8123`, so `{{ credential.<slot>.url }}/api/states` reaches the REST API. `websocket_url` is
the WebSocket API derived from it: `ws://homeassistant.local:8123/api/websocket`. An `https://` address is refused when
the account is saved, since the host cannot dial `wss://` yet. The token may go only to the host and port of `url`, and
the account takes no destination list of its own.

`label` is what the operator sees in the picker and is required. `description` is optional helper text. `required`
defaults to `false`; a required slot left unbound warns the operator in the editor but never blocks saving, so your
widget must still render something sensible without it.

A slot naming a type the firmware does not know is a manifest error, and the widget is skipped at discovery with a
message naming the slot, the bad type and the valid set.

## Generate Placeholders

Credential placeholders are generated into `src/manifest_params.rs` alongside the typed params, one module per slot:

```bash
just wasm::gen <widget-name>
```

For the manifest above that emits:

```rust
pub mod credentials {
    pub mod home {
        pub const TOKEN: &str = "{{ credential.home.token }}";
        pub const URL: &str = "{{ credential.home.url }}";
        pub const WEBSOCKET_URL: &str = "{{ credential.home.websocket_url }}";
    }
    pub mod pool {
        pub const TOKEN: &str = "{{ credential.pool.token }}";
    }
    pub mod weather {
        pub const TOKEN: &str = "{{ credential.weather.token }}";
    }
}
```

Use the constants rather than writing the placeholder text yourself — a mistyped field name is then an ordinary compile
error instead of a request that fails at runtime.

## Spend a Credential

Put the constant wherever the value belongs — URL, header, or body — and make the request as usual:

```rust
use crate::manifest_params::credentials;

let url = fmt!("https://api.braiins.com/v1/stats?token={}", credentials::pool::TOKEN);
```

The host resolves the placeholder as the request leaves. Two things can stop it, and both refuse the request rather than
sending it half-built:

- the slot is unbound, or the placeholder names a field the account kind does not have;
- the destination lies outside the credential's egress pin — a `braiins-pool` token sent anywhere but `api.braiins.com`.
  An account of a kind with no pin of its own may carry one the operator wrote, so a widget cannot assume a generic
  credential reaches every host: treat a refusal as configuration, not as a bug to code around.

A refused request comes back as `FetchOutcome::Refused`, not the `Network` failure a widget would retry — neither cause
becomes true on a second attempt. Rebinding the slot or fixing the placeholder is what clears it.

### On a WebSocket

`ws_connect` substitutes placeholders in the URL and headers, and `Ws::send` in every message. A message has no URL of
its own, so the host judges it against the host and port the socket connected to. The handshake follows no redirect: a
3xx answer fails the connect, so the socket only ever talks to the host its URL names, the one the pin judged. A service
that authenticates on the socket, as Home Assistant does, takes the token in a message:

```rust
let Some(ws) = ws_connect(credentials::home::WEBSOCKET_URL, None, on_event) else {
    return; // past the socket limit
};

// Once `on_event` sees `WsEvent::Open`:
let auth = fmt!(r#"{{"type":"auth","access_token":"{}"}}"#, credentials::home::TOKEN);
ws.send(&auth);
```

The same two causes stop it. As with fetch, the widget learns of either from a host callback, never inside the call that
caused it, and reconnecting does not clear it:

- a refused connect still hands back a socket, which is never dialled. The widget receives
  `WsEvent::HostClosed(WsHostClose::Credential)` at the next delivery;
- a refused message is not sent. The host closes the socket instead, and the widget receives the same
  `WsEvent::HostClosed(WsHostClose::Credential)`. `Ws::send` returns nothing, so that event is how the widget learns of
  it.

The host's own closes arrive as `WsEvent::HostClosed`, which no peer can send, so a peer's 1008 or 1012 in a
`WsEvent::Close` is never mistaken for one. The peer still gets a close frame: 1008 for a refused message, 1012 for a
changed account. A widget built for a protocol older than 0.10 has no `HostClosed` to read, and gets the same close as a
`WsEvent::Close` with that code instead.

A socket whose URL the host cannot read a host from carries no credential in any message. The value goes in verbatim:
the host does not escape it for JSON or anything else.

When the operator changes or removes the account behind a slot a socket has spent, in its URL, headers or any message,
the host closes that socket and the widget receives `WsEvent::HostClosed(WsHostClose::AccountChanged)`. A service keeps
a socket authenticated after the message that spent the token, so this is what makes a rotated or removed token stop
working at once. Reconnect; the new socket spends the account as it now is. Renaming an account changes nothing the
socket spent and leaves it open.

### Reserved Braces

In a request or message that spends a credential, `{{` is reserved: anything between braces that is not a credential
reference is an error, not text passed through, and the request is refused rather than sent half-built. Write `{{{{` for
a literal `{{`. A request or message naming no credential is never examined, so a widget templating its own URL — the
image widget's `{{width}}` — or sending Home Assistant a Jinja template is unaffected until it also spends a credential.

Everything the device logs about the request or socket — diagnostics, recorded fixtures, hermetic-run reports — shows
the placeholder, never the resolved value.

## React to Binding Changes

The widget can see which slots are bound, and to which account, but never the values:

```rust
use bmc_wasm_sdk::credentials;

let bound = credentials::current();
if bound.is_bound("pool") {
    // `bound.get("pool")` also carries the account's type and the name the operator gave it.
}
```

Delivery updates the snapshot and invokes the optional hook, but does not itself schedule a render. Call
`request_frame()` or `request_frame_after()` when the change should repaint; otherwise the new snapshot is observed on
the next naturally scheduled render. An immediate display update therefore requires `on_credentials_update` to request a
frame.

Request a frame only when the changed credentials affect visible output:

```rust
use bmc_wasm_sdk::{credentials, request_frame};

#[unsafe(no_mangle)]
pub extern "C" fn on_credentials_update() {
    let current = credentials::current();
    let previous = credentials::previous();
    if current != previous {
        request_frame();
    }
}
```

`credentials::previous()` holds the snapshot from immediately before, so a hook can tell a fresh binding from a swapped
account.

Rotating an account's value fires the hook without changing the view, since the values are not part of it. The hook can
restart authenticated work without repainting when the visible binding is unchanged.

## Reference Example

`widgets-wasm-examples/params-demo` declares one slot per credential type — required and optional, single-field and
two-field, pinned and unpinned — and renders each slot's bound account or a placeholder dash when unbound.
