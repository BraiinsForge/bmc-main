# BOS miner model — design

## Goal

Populate and display the per-device miner model in the fleet-management widget. Today `KnownDevice.model` is always `None` and never rendered. We source the model from BOS REST endpoints the widget already polls, store it on the device, and show it as an interim column in the device list. The model also becomes the key the list will later be grouped by.

## Context

The widget polls three BOS REST endpoints per device in a round-robin pass (`session.rs`): `/miner/stats`, `/miner/hw/hashboards`, `/miner/details`. Two of these already carry everything we need for the model, so no new requests or auth are introduced.

`/miner/details` is auth-gated, so model data is only obtained when login with the shared password succeeds — the same constraint that already applies to uptime.

## Data sourced

All fields come from endpoints already polled. JSON keys are the proto field names in snake_case, matching the paths the widget already parses (e.g. `/hashboards/{i}/highest_chip_temp/temperature/degree_c`).

| `MinerModel` field | Source path | Wire form | Handling |
|---|---|---|---|
| `id` | `/miner/details` → `/platform` | integer enum | mapped to a slug via a widget-side table mirroring `proto::Platform`; unmapped value yields no slug |
| `name` | `/miner/details` → `/miner_identity/miner_model` | string | display-ready product name |
| `chip_type` | `/miner/hw/hashboards/{i}/chip_type` | string | first non-null board |
| `chip_count` | `/miner/hw/hashboards/{i}/chips_count` | integer | sum across boards |
| `nominal_hashrate_ths` | — | — | left `None`, out of scope |

`platform` is serialized as a raw integer over REST: boser-grpc generates types with `tonic_build` + `#[derive(serde::Serialize)]` and no enum-to-string override, so prost enum fields emit their `i32` value (consistent with the OpenAPI `int32`). The slug table covers the eight real `BosPlatform` variants:

```
am1-s9  am2-s17  am3-bbb  am3-aml
zynq-bm3-am2  cvitek-bm1-am2
stm32mp157c-ii1-am2  stm32mp157c-ii2-bmm1
```

The exact integer for each variant is read from the boser-grpc proto during implementation. `Unspecified`/`0` and any unmapped integer map to no slug.

## Parsing and accumulation

The model is built across two endpoints in one pass, mirroring how `TelemetryReading` is built. `FamilyAdapter` gains a `parse_model` method with a default no-op body, so the not-yet-implemented uBOS and Bitaxe adapters are unaffected:

```rust
fn parse_model(&self, endpoint: &str, json: &dyn JsonLookup, model: &mut ModelAccumulator);
```

`ModelAccumulator` is an all-`Option` builder. The driver fills it alongside the existing `TelemetryReading` during the pass, then at `finalize_device` converts it into a `MinerModel` and stamps it onto the device via a new `DeviceList::apply_model`, parallel to `apply_telemetry`.

`MinerModel.id` and `MinerModel.name` change from `String` to `Option<String>`, so "model name known but platform unmapped" and the inverse are both representable. `KnownDevice.model` is set to `Some(..)` once `name` is known; an accumulator with neither field leaves `model` untouched.

The model is re-parsed every pass. The responses are identical between passes, but re-parsing keeps the value correct after a relabel or upgrade without special-casing, and avoids a separate refresh path. `clear_all_telemetry` (triggered by a shared-password change) also clears `model`, since the model came from an auth-gated endpoint.

## Display

A model column is inserted immediately after the device name in the device row (`render.rs`), keeping the row single-line. This is interim: once devices are grouped by model the per-row column goes away.

The cell shows the product `name` (`miner_model`), falling back to `N/A` when the model is absent. The platform `id` is stored but not displayed — it is the key the future model-grouping will use. `chip_type` and `chip_count` are parsed and stored but not shown in this interim column; they surface in the eventual per-model view.

## Testing

- platform integer to slug: every `BosPlatform` variant maps to its slug; `Unspecified`/unknown maps to none
- `parse_model` extracts `miner_model` from `/miner/details` and `chip_type` / summed `chips_count` from `/miner/hw/hashboards`
- `apply_model` stamps the model onto the device; `clear_all_telemetry` clears it
- render shows the model column with the product name and `N/A` fallback

## Out of scope

- `nominal_hashrate_ths` (sticker hashrate) — more involved, deferred
- uBOS and Bitaxe model parsing — adapters not yet implemented
- model-based grouping of the device list — the consumer of `id`, a later step
