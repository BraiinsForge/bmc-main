# Credential Egress Testing

A credential is resolved by the host as the request leaves, so the widget never holds the secret and it is spent only at
the hosts its account, or failing that its type, allows. The runtime tests decide both rules in isolation. Neither can
show that the secret survives the whole chain — secret store, coordinator, wayland, wasm host — and still stops in the
right place, and every hop there is somewhere a secret can be dropped, mis-delivered, or written to a log.

`deck check-credential-egress` drives that chain on a real Deck against a server the run owns, so the evidence is what
the device actually sent rather than what it reports having sent.

Run it when touching `bmc-wasm-runtime/src/runtime/imports/credentials.rs`, the resolution in `bmc/src/credential.rs`,
the delivery path in `bmc/src/widget/coordinator.rs`, or the egress policies in `bmc-field-schema/src/credential.rs`.

```sh
# every case
nix run .#deck -- check-credential-egress --device 192.168.1.2

# leave the test scenes and accounts on the device
nix run .#deck -- check-credential-egress --device 192.168.1.2 --keep-config

# put the previous config and accounts back
nix run .#deck -- check-credential-egress --device 192.168.1.2 --restore
```

`--dwell-seconds` sets both the scene cycling duration on the device and the wait, so a shorter run stays coherent.

## The cases

| Case                   | Credential                | Expected                                                          |
| ---------------------- | ------------------------- | ----------------------------------------------------------------- |
| `permitted`            | `generic-token`           | arrives, carrying the resolved secret                             |
| `pinned`               | `braiins-pool`            | refused for leaving the type's egress pin                         |
| `unbound`              | none bound                | refused: no secret available for the slot                         |
| `undeclared`           | `generic-token`           | withheld: the installed manifest no longer declares the slot      |
| `redirect`             | `generic-userpass`        | arrives, and the 302 it is answered with is not followed          |
| `reshaped`             | `braiins-pool`            | refused: the secret would end the authority early                 |
| `account_pin`          | `generic-token` + own pin | arrives; the account's own pin admits us                          |
| `account_pin_denied`   | `generic-token` + own pin | refused; the account's pin excludes us where the type would not   |
| `account_pin_replaces` | `braiins-pool` + own pin  | arrives; the account's pin replaces the type's rather than adding |

The refusals are kept distinct on purpose, because they are not the same kind of event. A pin refusal is the firmware
overruling the request; an unbound refusal is the operator's configuration failing; a withheld binding is the manifest
having changed under a stored config. They run through different branches, and only some are security boundaries — a
corpus carrying just the pin case would leave the ordinary misconfigurations untested.

The last three exist because an account may carry a pin of its own, which **replaces** the type's wherever it is
non-empty. `account_pin_replaces` is the only one of the nine that cannot be set up through the API at all: the gRPC
handler refuses a host list on a type that already pins its own destination, so writing the store directly is the only
way to reach it — and it is the semantic worth proving on hardware rather than in isolation.

## The carrier

`params-demo`, which fetches whatever its `string_uri` param holds. Its manifest declares one slot per credential type
the firmware knows, plus a second pinned slot. That matters: a slot has to be declared for resolution to authorise it,
so a widget declaring none cannot carry a credential at all.

Nothing about the widget knows it is carrying a credential, which is the point — the substitution is the host's, and a
widget that had to cooperate would not be testing the property that matters.

Bindings and accounts are written straight into the config and the secret store, past the gRPC validation that would
refuse a slot the manifest never declared or a host list on a pinned type. That is not a way around the test. A
hand-edited file is a state the device has to survive, and it is exactly the state the fail-closed checks in
`egress_permitted` exist for — the secret store is a plain file an operator can edit.

## How an outcome is decided

A request that arrives is recorded by the asset server with its query, so the permitted case is judged on the bytes the
device sent us.

A refusal never reaches the network, so it is read from the wasm host log instead. The message names the cause and the
slot, and no two refusal-judged cases share a slot — a corpus invariant the unit tests hold — so a refusal belongs to
exactly one of them without a URL to match on. Three cases now share the pin message, which is why the slot rather than
the wording is what attributes it. Cases judged on what arrived are attributed by path instead, and may share a slot.

## What guards the evidence

Every one of these exists because the failure it prevents produces a confident, wrong report rather than an obvious
break.

**The permitted case is judged first, and its absence aborts the run.** A device that never fetched at all — wrong
address, widget never spawned, compositor down — satisfies every refusal case for free, and the run would report that
the pin held when nothing was ever tried. Hard fail.

**The secret is generated per run.** A fixed one would leave a line from an earlier run in the log to pass or fail this
run's leak check, in either direction.

**The delivered token is compared for equality, not containment.** Containment would accept a request that carried the
resolved secret *and* something else; equality also rules out the placeholder arriving exactly as the widget wrote it,
which is the other way substitution can go wrong.

**The log is searched for the secret.** The host must log the placeholder form the guest wrote; a resolved URL in a log
line puts the secret into every support archive that collects one. Hard fail.

**The cases using a pinned credential type are named in the unit tests**, rather than counted or left to reading. A
corpus that drifted to unpinned types everywhere would pass every run while proving nothing about pinning, so gaining a
pinned case is a deliberate edit.

**Every refusal message is checked against the Rust sources.** Each verdict substring-matches a log line the firmware
writes, so a rewording blinds the run silently while the unit tests, which hold the constants symbolically, stay green.
That is not hypothetical: the pin refusal drifted exactly so when the pin stopped being the credential type's alone.

**Firmware predating the feature fails loudly rather than passing.** A Deck with no host-side substitution sends the
placeholder as written, so the permitted case arrives carrying `{{credential.weather.token}}` instead of the secret and
the equality check rejects it, while the refusal cases receive a request they should never have got. That is why this
run needs no build check of its own, unlike `deck image-formats`, whose measurements a stale build would silently
distort.

**An absent secret store is backed up as an absence.** A device that never saved an account must have none afterwards,
so the restore removes the file rather than keeping the test's. A swallowed copy failure would make that same restore
delete accounts the run never saved.

## Gaps

**The wire value the widget sees is not checked here.** A refusal answers the guest without logging a status, so
`FetchOutcome::Refused` is not visible in the device log. It is covered hermetically by
`bmc-wasm-runtime/tests/credential_egress.rs`, which asserts the widget receives it rather than the network failure it
would otherwise retry.

**Only a URL carries a placeholder.** `params-demo` sends no custom headers and no body, so substitution across those is
left to the runtime unit tests. A widget built for this would close the gap, at the cost of a fixture widget existing
only for the test.
