# bmc-netsim

A generic **mDNS + REST network-resource simulator**. It advertises fake devices over real mDNS on the LAN and serves
their HTTP endpoints, so a widget (or anything else) discovers and polls them exactly as it would real hardware — no
physical devices required.

The engine knows nothing about miners. A **device profile** declares how a resource announces itself, what endpoints it
serves, and the shape of their responses. The miner profiles (BOS+, uBOS, AxeOS) ship as slim, typed modules under
`src/devices/`; the engine (`announce`, `respond`, `value`, `render`) is device-agnostic.

## Scope: a subset modelled from upstream

Each device profile reproduces **only** the fields the widget's family adapters
(`widgets-wasm/fleet-management/src/families/`) actually read, shaped to the upstream device APIs (BOS+ boser REST, uBOS
`/api/info`, ESP-Miner `/api/system/info`). It is a deliberate subset, not a mirror of those APIs.

So a field being absent from a profile — or from an adapter — is **not** evidence that the upstream API lacks it; it
only means the widget has not needed it yet. Before concluding a field is unavailable, check the upstream source of
truth (the API's openapi/spec or the firmware), never netsim. When the widget starts reading a new field, model it here
too.

Vendored snapshots of those upstream contracts live under [`reference/`](reference/) for in-repo lookup — currently the
BOS+ boser openapi; AxeOS and uBOS are noted there.

## Quick start

```sh
# Run the fleet described by a blueprint, advertising on every LAN interface.
cargo run -p bmc-netsim -- run bmc-netsim/blueprints/example.json5

# Print the blueprint JSON schema (editor autocomplete / validation, CI drift guard).
cargo run -p bmc-netsim -- schema
```

The n-th device listens on `20000 + n`. A widget browsing the LAN finds each over mDNS and polls its `host:port`. To
point a real Deck at it, run the sim on a machine on the Deck's Wi-Fi and open the fleet-management widget — the sim
accepts any BOS/uBOS credentials.

## Blueprint

A blueprint is JSON5, loaded from disk: a list of device instances. Each names a `device`, supplies that device's typed
`params`, and optionally a `count`. Point `$schema` at the generated schema for editor autocomplete and validation.

```json5
{
  "$schema": "../blueprint.schema.json",
  instances: [
    { device: "bos", count: 3 },                                    // a healthy Braiins Mini Miner BMM 101 fleet
    { device: "bos", count: 1, params: { hashrate_ths: 0.15 } },    // degraded: under 20% of nominal
    { device: "bos-libre", params: { status: 503 } },               // present over mDNS, API fails
    { device: "axeos", count: 2 },
  ],
}
```

Scenarios are just params — there is no scenario type. A degraded miner is a low `hashrate_ths`; an unreachable one is
`status: 503` (discoverable, but its telemetry endpoints error).

## Dynamic values

Any leaf of a response body may be dynamic, re-evaluated on every request:

```json5
{ "$value": { kind: "drift",  center: 100000, amp: 2000, period_s: 300, jitter: 400 } }
{ "$value": { kind: "ranged", min: 60, max: 70 } }
{ "$value": { kind: "fixed",  value: 65 } }
```

`drift` is a slow sine wander around `center` (with light `jitter`), `ranged` is a uniform draw, `fixed` is a constant.
Everything else in a body is served verbatim. Device profiles build these from their typed params, e.g. a hashrate that
drifts around `hashrate_ths`.

## Devices

| `device`       | mDNS service                   | endpoints                                                                                                   |
| -------------- | ------------------------------ | ----------------------------------------------------------------------------------------------------------- |
| `bos`          | `_bos._sub._http._tcp`         | boser `/api/v1/{auth/login, miner/stats, miner/hw/hashboards, miner/details}`                               |
| `bos-libre`    | `_ubos._tcp`                   | `/api/info`                                                                                                 |
| `axeos`        | `_axeos._sub._http._tcp` + TXT | `/api/system/info`                                                                                          |
| `braiins-pool` | — (cloud, not announced)       | FPPS `/pool/v2/user/{hashrate,workers}/{current,history}`, `rewards/latest`, `financials`, `payouts/recent` |

Each device's params — `model_name`, `hashrate_ths`, `power_w`, `temp_c`, `uptime_s`, `status` — live in its module and
appear in the schema under `BosParams` / `UbosParams` / `AxeosParams` / `BraiinsPoolParams`.

## Cloud profiles

A profile may also simulate a cloud API rather than a LAN device: it announces nothing (`announce: None`) and is reached
by its port alone — how a consumer routes traffic to that port is the consumer's business. `braiins-pool` is the first
such profile: its windowed endpoints read their query string (`Body::Respond`), history is generated on demand as a pure
function of each five-minute slot's absolute time — so any window depth paginates deterministically — and payouts land
on the `payout_period_s` grid.

## Adding a device

1. Copy `src/devices/<name>.rs`: a `Params` struct (`#[derive(Deserialize, JsonSchema)]`, `#[serde(default)]`,
   `#[schemars(rename = "<Name>Params")]`) plus a `resource()` builder that returns its announce + endpoints. Assets, if
   any, go in `src/devices/<name>/`.
2. Register it: add `pub mod <name>;` to `src/devices.rs` and a `<Name>` variant to the `Instance` enum in
   `src/blueprint.rs`.
3. Regenerate the schema: `UPDATE_SCHEMA=1 cargo test -p bmc-netsim`.

## Schema and drift guard

`blueprint.schema.json` is a committed, generated artifact — the source of editor validation. A test
(`blueprint_schema_is_current`) regenerates it in memory and fails if the committed file is stale, so the schema and the
Rust types can never diverge. Regenerate with `UPDATE_SCHEMA=1 cargo test -p bmc-netsim` (or
`cargo run -p bmc-netsim -- schema`).
