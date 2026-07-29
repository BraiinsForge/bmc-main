# BDK-434 credential accounts — design decisions

Written: 2026-07-29.

The decisions behind centralised credential accounts that constrain future work, kept because the reasoning is not
recoverable from the code. What the feature *is* lives in
[`docs/stories/credential-accounts.md`](../../stories/credential-accounts.md); how a widget uses it lives in
[`docs/devel/wasm-widgets/credentials.md`](../../devel/wasm-widgets/credentials.md); how it is proved on hardware lives
in [`docs/devel/credential-egress-testing.md`](../../devel/credential-egress-testing.md).

## The secret never enters guest memory

The ticket as written delivered the secret *to* the widget. It is delivered to the **host** instead, and the guest gets
only a view naming the bound account. A widget embeds `{{ credential.<slot>.<field> }}` and the host substitutes it as
the request leaves.

That inverts who has to be trusted. A widget is third-party WASM; under the ticket's design every widget that used a
credential held one in memory it could exfiltrate. Under this one, a hostile widget can learn that a slot is bound and
what the operator named the account, and nothing more.

**Substitution is the last hop before the HTTP client**, not where the request is built. Everything upstream — the fetch
key, the fetch interceptor, hermetic-breach records, recorded capture fixtures, the completed-fetch log line — therefore
only ever sees the placeholder. Moving the substitution earlier for convenience would put a live token into whichever of
those a future change happens to route through, and each of them is written to disk or into a fixture. This is not
theoretical: the delayed-fetch path did briefly log a resolved URL, caught by the on-device check.

## Egress pin grammar

An `egress.allow_hosts` entry is one of three forms, matched against the request's normalised authority and never
against the path:

- **exact host** — full host and port, lowercased and IDNA-normalised;
- **one-label wildcard** `*.example.com` — matches exactly one label and **not** the apex, following the TLS and cookie
  convention. `api.example.com` matches; `example.com` and `a.b.example.com` do not. An apex that should be reachable is
  listed separately, so widening is always something someone wrote down;
- **CIDR** `10.0.0.0/8`, `fd00::/8` — matched **only when the authority is itself an IP literal**.

**A hostname is never resolved to decide a pin.** Resolving would mean the address the check approves is not the address
the fetch dials — DNS rebinding: a widget names a host that resolves into the range for the check and anywhere it likes
a moment later. The cost is that a CIDR entry does nothing for hostname URLs, which is the intended trade.

CIDR is what makes `fleet-management` expressible at all, since it reaches miners by LAN address rather than hostname,
and it is what lets a *pinned* type be tested end-to-end against a local server instead of forcing the happy path onto
an unpinned generic.

**An empty `allow_hosts` means unfiltered**, the same as omitting `egress`. A credential that may be sent nowhere is not
usable, so there is no reason to spell one; treating the empty list as deny-all would only turn a typo into a type that
silently never authenticates. Filtering is opt-in: a type gets a pin by listing somewhere to send its secret.

The authority is read through the same URL parser the HTTP client uses. Where an egress check and the client disagree
about which host a URL names, that disagreement *is* the vulnerability — which is why neither the authority nor the CIDR
containment is hand-parsed.

## Cut: per-account egress override

The idea was an optional `Account.allow_hosts` narrowing its type's pin.

It is cut because the two operations are not the same job. A pin needs **matching** — does this authority satisfy this
rule. An override needs **containment** — does the account's list permit only what the type's list does. Once the
grammar gained wildcards and CIDR, containment became a second and harder algorithm whose cross-form pairs have to be
defined rather than falling out: `10.0.0.0/16` ⊂ `10.0.0.0/8` holds, `api.example.com` ⊂ `*.example.com` holds,
`*.a.example.com` ⊂ `*.example.com` does **not** under the one-label rule, and `10.0.0.5` ⊂ `*.example.com` is a
category error.

Nothing needs it meanwhile: the safety property already rides on the type, so a Braiins token cannot leave
`api.braiins.com` whatever an account says.

## At-rest encryption is out of scope for v1

The store stays plaintext at `0600`. The deciding fact is that **the key would have nowhere to live**. The device has no
TPM, secure element, efuse or OTP, and no device-unique secret to derive from. A key compiled into the firmware is
public, because the firmware is downloadable; a key derived from a MAC or serial is printed on the device; a boot
passphrase is unusable on a clock that must come up unattended.

The one in-tree cipher, `bmc-support/src/encrypt.rs`, says as much about itself — a fixed AES-128 key for the password
"braiins", documented as *file content obfuscation*. Reusing that shape for credentials would assert a protection nobody
has.

What actually protects the store is what shipped instead, each piece closing a path that was really open: the values
live outside `config.json`, the file is `0600`, the support-bundle walker skips it outright (bundles are *sent to
Braiins*, so that was the live egress), and `Account`'s hand-written `Debug` keeps values out of archived logs.

Revisit if the hardware grows a secure element, or if a threat appears that a discoverable key would genuinely address.
Two things worth confirming on-device meanwhile, neither of them encryption: that `bos factory_reset` wipes
`/etc/bmc/secrets.json` (that script lives outside this repo), and that no config-export path carries the file.

## Reserved now, so it need not be migrated later

Credential types outside firmware are not v1, but **id uniqueness leaks backwards** into what shipped. `Account.type_id`
is persisted as a bare string in `secrets.json` and `CredentialSlot.type_id` is a bare string in every shipped manifest,
so bolting namespacing on afterwards means a config migration plus manifest churn.

The rule, settled early because it costs nothing: **bare ids are reserved for firmware built-ins; anything admitted from
outside is always namespaced** (`<widget-uid>/<id>` or similar). Collision then cannot be expressed rather than having
to be detected, and `braiins-pool`, `generic-token` and `generic-userpass` stay valid untouched.

Egress needs no special rule for outside types. An unpinned type is already a shipped, disclosed concept — both generics
ship that way — and nothing leaks until an operator creates an account and enters a secret, the same consent gate either
way. What did need fixing is that the disclosure used to be prose inside each type's `description`; it is derived from
the structured `egress` field instead, so it stays truthful for a type whose description someone else wrote.

The `| filter` grammar is likewise reserved but unimplemented, so client-side crypto (`{{ seed | hmac }}`) can layer on
later without a format change.

## Tolerated rather than repaired

**A binding can outlive the account it names.** `RemoveAccount` refuses to delete a bound account, but `secrets.json` is
a plain file an operator can edit, so config can legitimately reference an account that is gone. Resolution treats such
a binding as no binding: the slot reads unbound, the widget degrades visibly, and a `warn!` names the widget, slot and
missing id. `effective_bindings` is the single definition of that subset and the read path uses it too, so the editor
never shows a slot bound to nothing.

**Pruning at load would be actively harmful.** A store that failed to load, or a config restored from a backup, would
have its bindings silently rewritten to empty — turning a recoverable mismatch into data loss.

**Credential staleness is deliberately not tracked.** `braiins-pool` verifies its token only at the moment it is
entered, so a token later revoked upstream keeps reading as good. That is the generic problem with any stored credential
and there is no clearly right answer to reach for: the failure appears where the egress happens. Revisit only if
operators hit it — the options are a validity column refreshed on page load or a periodic re-check, both costing a
round-trip per account for a state that changes rarely.

## Deferred

- **Testbed credentials sidebar plus capture/regression wiring.** `RuntimeConfig.credentials` reduces it to a config
  write.
- **Widget-supplied credential types**, per the namespacing rule above; needs an admission step with its own invariants
  (icon byte cap, icon format allowlist, caps on the declared schema).
- **Caret code-frame for load failures** — fits *parse* errors, which carry line and column, rather than the semantic
  slot check; would need `codespan-reporting` as a real dependency.
- **Coverage reporting for Rust and the frontend.** Python already reports Cobertura to GitLab; the other two can copy
  that shape.

`bmc-wasm-thin` needs no credential channel, contrary to an earlier note here: it opens the Wayland socket, hands the fd
to `bmc-wasm-host` and idles as a lifetime witness, so the guest and every outbound request are the host's.
