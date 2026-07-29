# Credential Egress Testing

A credential is resolved by the host as the request leaves, so the widget never holds the secret and a pinned type can
only spend it at the hosts its type allows. The runtime tests decide both rules in isolation. Neither can show that the
secret survives the whole chain — secret store, coordinator, wayland, wasm host — and still stops in the right place,
and every hop there is somewhere a secret can be dropped, mis-delivered, or written to a log.

`deck check-credential-egress` drives that chain on a real Deck against a server the run owns, so the evidence is what
the device actually sent rather than what it reports having sent.

Run it when touching `bmc-wasm-runtime/src/runtime/imports/credentials.rs`, the resolution in `bmc/src/credential.rs`,
the delivery path in `bmc/src/widget/coordinator.rs`, or the egress policies in `bmc-field-schema/src/credential.rs`.

```sh
# all three cases
nix run .#deck -- check-credential-egress --device 192.168.1.2

# leave the test scenes and accounts on the device
nix run .#deck -- check-credential-egress --device 192.168.1.2 --keep-config

# put the previous config and accounts back
nix run .#deck -- check-credential-egress --device 192.168.1.2 --restore
```

`--dwell-seconds` sets both the scene cycling duration on the device and the wait, so a shorter run stays coherent.

## The cases

| Case        | Credential type | Expected                                                        |
| ----------- | --------------- | --------------------------------------------------------------- |
| `permitted` | `generic-token` | the request arrives, carrying the resolved secret               |
| `pinned`    | `braiins-pool`  | nothing arrives; the host refuses it for leaving the egress pin |
| `unbound`   | none bound      | nothing arrives; the host refuses an unresolvable placeholder   |

The two refusals are separate on purpose. A pin refusal is the firmware overruling the request; an unbound refusal is
the operator's configuration failing. They run through different branches, and only one of them is a security boundary —
a corpus carrying just the pin case would leave the ordinary misconfiguration untested.

## The carrier

The image widget, because it fetches a URL an operator supplies and its `expand_url` passes through anything that is not
`{{width}}` or `{{height}}`. Nothing about the widget knows it is carrying a credential, which is the point: the
substitution is the host's, and a widget that had to cooperate would not be testing the property that matters.

Bindings are written straight into the config, past the gRPC validation that would refuse a slot the widget's manifest
never declared. That is not a way around the test. A hand-edited config is a state the device has to survive, and it is
exactly the state the fail-closed checks in `egress_permitted` exist for — the secret store is a plain file an operator
can edit.

## How an outcome is decided

A request that arrives is recorded by the asset server with its query, so the permitted case is judged on the bytes the
device sent us.

A refusal never reaches the network, so it is read from the wasm host log instead. Each cause logs a distinct
`refusing fetch: …` line, and each of those belongs to exactly one case here, which is what makes the message enough to
attribute without a URL to match on.

## What guards the evidence

Every one of these exists because the failure it prevents produces a confident, wrong report rather than an obvious
break.

**The permitted case is judged first, and its absence aborts the run.** A device that never fetched at all — wrong
address, widget never spawned, compositor down — satisfies both refusal cases for free, and the run would report that
the pin held when nothing was ever tried. Hard fail.

**The secret is generated per run.** A fixed one would leave a line from an earlier run in the log to pass or fail this
run's leak check, in either direction.

**The delivered token is compared for equality, not containment.** Containment would accept a request that carried the
resolved secret *and* something else; equality also rules out the placeholder arriving exactly as the widget wrote it,
which is the other way substitution can go wrong.

**The log is searched for the secret.** The host must log the placeholder form the guest wrote; a resolved URL in a log
line puts the secret into every support archive that collects one. Hard fail.

**Only the pinned case uses a pinned credential type**, asserted in the unit tests rather than left to reading. A corpus
that drifted to unpinned types everywhere would pass every run while proving nothing about pinning.

**Firmware predating the feature fails loudly rather than passing.** A Deck with no host-side substitution sends the
placeholder as written, so the permitted case arrives carrying `{{credential.api.token}}` instead of the secret and the
equality check rejects it, while both refusal cases receive a request they should never have got. That is why this run
needs no build check of its own, unlike `deck image-formats`, whose measurements a stale build would silently distort.

**An absent secret store is backed up as an absence.** A device that never saved an account must have none afterwards,
so the restore removes the file rather than keeping the test's. A swallowed copy failure would make that same restore
delete accounts the run never saved.

## Gaps

**The wire value the widget sees is not checked here.** A refusal answers the guest without logging a status, so
`FetchOutcome::Refused` is not visible in the device log. It is covered hermetically by
`bmc-wasm-runtime/tests/credential_egress.rs`, which asserts the widget receives it rather than the network failure it
would otherwise retry.

**Only a URL carries a placeholder.** The image widget sends no custom headers and no body, so substitution across those
is left to the runtime unit tests. A widget built for this would close the gap, at the cost of a fixture widget existing
only for the test.
