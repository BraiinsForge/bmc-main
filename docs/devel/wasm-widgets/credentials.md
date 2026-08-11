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
    }
  }
}
```

`type` names one of the account kinds the firmware provides:

| Type               | Fields                 | Egress                      |
| ------------------ | ---------------------- | --------------------------- |
| `generic-token`    | `token`                | anywhere                    |
| `generic-userpass` | `username`, `password` | anywhere                    |
| `braiins-pool`     | `token`                | pinned to `api.braiins.com` |

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

In a request that spends a credential, `{{` is reserved: anything between braces that is not a credential reference is
an error, not text passed through, and the request is refused rather than sent half-built. Write `{{{{` for a literal
`{{`. A request naming no credential is never examined, so a widget templating its own URL — the image widget's
`{{width}}` — is unaffected until it also spends a credential.

Everything the device logs about the request — diagnostics, recorded fixtures, hermetic-run reports — shows the
placeholder, never the resolved value.

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
