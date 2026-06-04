# AxeOS / Bitaxe family support design

## Context

BDK-506 builds the fleet-management WASM widget. The current widget has a generic family adapter shape, a round-robin
telemetry driver, BOS support, uBOS support, and a placeholder `DeviceFamily::Bitaxe` that is intentionally mapped to no
adapter yet.

This slice adds the AxeOS / Bitaxe family as the final currently planned miner family. It targets both upstream
`~/src/ESP-Miner` and the `~/src/ESP-Miner-NerdQAxePlus` fork with one adapter. Both expose compatible legacy telemetry
on `GET /api/system/info`, and both will advertise the `_axeos._sub._http._tcp` DNS-SD subtype.

The design deliberately avoids the NerdQAxePlus `/api/v2/*` endpoints in this slice. They are richer, but the legacy
endpoint already contains the fields the widget needs for the interim row display.

## Goals

- discover AxeOS miners via `_axeos._sub._http._tcp`
- support upstream ESP-Miner and ESP-Miner-NerdQAxePlus with one adapter and one telemetry parser
- fetch current hashrate, power, temperature, and uptime from `/api/system/info`
- capture model identity from mDNS TXT records and from `/api/system/info` when available
- keep the existing family-generic telemetry driver unchanged apart from wiring `DeviceFamily::Bitaxe` to the new
  adapter
- keep parsing pure and host-tested; only the existing fetch and mDNS wiring remains wasm-only

## Non-goals

- `/api/v2/dashboard`, `/api/v2/system`, CAN-node fleet expansion, per-chip temperature tables, or structured fan data
- authentication or OTP flows
- new widget parameters
- final fleet aggregation or per-model UI
- distinguishing upstream ESP-Miner from NerdQAxePlus at runtime unless a concrete incompatibility appears later

## Discovery

`BitaxeAdapter::browse_service_types()` returns:

```rust
&["_axeos._sub._http._tcp"]
```

The host runtime already delivers mDNS `Found` events as JSON shaped like:

```json
{
  "service_type": "_http._tcp.local.",
  "name": "Bitaxe Gamma 602 (A1B2)._http._tcp.local.",
  "host": "192.168.1.42",
  "port": 80,
  "txt": {
    "board": "602",
    "family": "Gamma",
    "asic": "BM1370",
    "asic_count": "1",
    "fw_version": "..."
  }
}
```

The `txt` object is available to widgets through JSON pointer paths such as `/txt/family` and `/txt/asic_count`. No
runtime or SDK change is needed.

The adapter reuses `extract_endpoint` for `name`, `host`, and `port`, stamps `DeviceFamily::Bitaxe`, and uses the mDNS
full service name as `DeviceId`, matching the BOS and uBOS adapters. `lib.rs` adds `on_bitaxe_event`, registers the
third browse in `init()`, and removes the `Bitaxe` dead-code expectation once the adapter constructs it. BOS + uBOS +
AxeOS uses three browses, under the host limit of four.

## Discovery model hint

Today `DiscoveredDevice` carries only `DeviceIdentity`, so TXT records are discarded after discovery. This slice extends
it to carry an optional model hint:

```rust
pub struct DiscoveredDevice {
    pub identity: DeviceIdentity,
    pub model_hint: Option<MinerModel>,
}
```

BOS and uBOS return `None`. `BitaxeAdapter::parse_found` builds `Some(MinerModel)` when discovery provides enough TXT
metadata.

The hint is applied immediately after `DeviceList::upsert`, before telemetry polling. This gives upstream ESP-Miner
devices a useful model even if `/api/system/info` lacks `deviceModel` and `asicCount`. Later telemetry model parsing may
overwrite the hint with a richer model, for example on NerdQAxePlus.

TXT model rules:

- `id` is stable per model, not per device. Prefer `axeos:<family>:<board>` when both `/txt/family` and `/txt/board`
  exist. Fall back to `axeos:<board>` if only board exists, then to `axeos:<family>` if only family exists.
- `name` is human-readable. Prefer `Bitaxe <family> <board>`, then `Bitaxe board <board>`, then `Bitaxe <family>`.
- `chip_type` comes from `/txt/asic` when non-empty.
- `chip_count` comes from `/txt/asic_count` when it parses as `u32`.
- no model hint is produced if neither family nor board is present.

## Telemetry

AxeOS has one telemetry endpoint:

```rust
fn api_base_path(&self) -> &'static str { "/api/system" }
fn telemetry_endpoints(&self) -> &'static [&'static str] { &["/info"] }
```

The resulting URL is:

```text
http://{host}:{port}/api/system/info
```

No auth methods are overridden. `credential_header()` stays `None`, `auth_endpoint()` stays `None`, and
`is_auth_error()` stays `false`. A 401 or 403 is treated like any other endpoint failure: the endpoint fields reset to
`N/A`, and the device becomes unreachable for that pass.

The single endpoint owns all telemetry fields and follows the existing reset-then-populate contract:

- current hashrate (TH/s) <- `/hashRate` in GH/s, divided by `1000`
- power (W) <- `/power`
- temperature (C) <- `/temp`
- uptime (s) <- `/uptimeSeconds`, converted to `u64` only when non-negative

The parser does not use `/hashRate_1m` for the main current value. The current row display is meant to show the live
value, and both codebases expose `hashRate` for that.

## Telemetry model parsing

`BitaxeAdapter::parse_model("/info", json, model)` supplements or replaces the discovery hint when `/api/system/info`
contains model fields.

Rules:

- if `/deviceModel` is present and non-empty, set both `id` and `name` from it. This is the NerdQAxePlus path and groups
  all devices with the same model together.
- otherwise, if `/boardVersion` is present and non-empty, set `id = "axeos-board:<boardVersion>"` and
  `name = "Bitaxe board <boardVersion>"`. This is a fallback for upstream ESP-Miner when TXT was unavailable.
- set `chip_type` from `/ASICModel` when non-empty.
- set `chip_count` from `/asicCount` when present and convertible to `u32`.
- leave `nominal_hashrate_ths` absent; the legacy endpoint exposes `expectedHashrate`, but that is operating-point
  dependent, not a stable nominal model value.

If telemetry model parsing cannot produce both `id` and `name`, `ModelAccumulator::into_model()` returns `None`, and the
previous discovery hint remains on the device.

## ESP-Miner and NerdQAxePlus compatibility

The shared parser relies only on fields present in the legacy endpoint of both codebases for telemetry:

- `hashRate`
- `power`
- `temp`
- `uptimeSeconds`

For model identity, NerdQAxePlus provides `deviceModel`, `ASICModel`, and `asicCount` on `/api/system/info`. Upstream
ESP-Miner provides `ASICModel`, `boardVersion`, and mDNS TXT records (`family`, `board`, `asic`, `asic_count`). That is
enough for one adapter as long as model parsing is additive:

- discovery TXT creates the early upstream model hint
- `/api/system/info` can refine NerdQAxePlus models
- missing model fields never block telemetry

The `/api/v2/*` endpoints are deferred. Add them only if hardware testing shows `/api/system/info` is insufficient for
the widget's required readings or for a future UI that needs richer NerdQAxePlus-specific data.

## Modules

- `families/bitaxe.rs` (new) - `BitaxeAdapter`: discovery, TXT model hint parsing, `/api/system/info` telemetry
  mapping, reset behavior, and model parsing.
- `families.rs` - declare `pub mod bitaxe;`.
- `adapter.rs` - add `model_hint: Option<MinerModel>` to `DiscoveredDevice`.
- `lib.rs` - add `on_bitaxe_event`, register the `_axeos` browse, and apply a discovery model hint after upsert.
- `session.rs` - map `DeviceFamily::Bitaxe` to `Some(&BitaxeAdapter)` in `adapter_for`.
- `device.rs` - drop the target-arch dead-code expectation on `DeviceFamily::Bitaxe` once the adapter constructs it.

`manifest.json` is unchanged.

## Testing

Host unit tests cover the pure surface:

- `BitaxeAdapter::browse_service_types()` returns exactly `["_axeos._sub._http._tcp"]`
- `parse_found` parses an AxeOS `Found` fixture, stamps `DeviceFamily::Bitaxe`, and rejects missing host or port
- `parse_found` reads `/txt/family`, `/txt/board`, `/txt/asic`, and `/txt/asic_count` into a model hint
- malformed or absent TXT omits only the model hint; discovery still succeeds when name, host, and port are valid
- a pure ingest helper or `DeviceList` test proves the discovery model hint is applied to `KnownDevice.model` immediately
  after `upsert`; this guards the current `ingest` path, which otherwise could keep calling only
  `upsert(found.identity)` and silently drop the hint
- `parse_telemetry("/info", ...)` maps `hashRate` GH/s to TH/s, `power` to W, `temp` to C, and `uptimeSeconds` to
  seconds
- negative `/uptimeSeconds` is ignored and leaves `uptime_s` as `None`
- stale-clearing verifies a second `/info` response missing owned fields clears previous values
- `reset_telemetry("/info", ...)` clears only the endpoint-owned fields and leaves `nominal_hashrate_ths` untouched
- `parse_model` prefers `deviceModel` when present, falls back to `boardVersion`, sets `ASICModel` as chip type, and
  parses `asicCount`
- `adapter_for(DeviceFamily::Bitaxe)` returns the Bitaxe adapter

Runtime verification in the testbed:

- with an AxeOS device advertising `_axeos._sub._http._tcp`, the device appears in the fleet list
- `/api/system/info` is fetched without auth
- live hashrate, power, temperature, and uptime render for reachable devices
- a stopped or unreachable endpoint clears the row readings to `N/A` and marks the device unreachable

## Success criteria

- AxeOS / Bitaxe devices from both ESP-Miner and NerdQAxePlus are discovered through `_axeos._sub._http._tcp`.
- One adapter supports both codebases without checking which fork produced the response.
- `/api/system/info` populates the same telemetry fields as BOS and uBOS rows, with correct GH/s to TH/s conversion.
- TXT records from the mDNS `Found` event are used for upstream ESP-Miner model hints.
- NerdQAxePlus `deviceModel` from `/api/system/info` can override or refine the discovery hint.
- Missing TXT or missing model fields never prevent telemetry polling.
- The host unit tests cover discovery, TXT parsing, telemetry mapping, reset behavior, model parsing, and adapter
  dispatch.
