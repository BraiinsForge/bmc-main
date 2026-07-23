# Vendored upstream API references

The widget's family adapters (`widgets-wasm/fleet-management/src/families/`) and the netsim device profiles
(`bmc-netsim/src/devices/`) are both modelled against the upstream miner APIs. This directory vendors those upstream
contracts so the shapes can be checked in-repo, without spelunking other repositories — see the "Scope" note in
[`../README.md`](../README.md).

These are **snapshots, not living copies**: each carries its provenance below. Refresh when the widget needs a field the
current snapshot predates.

## Device capabilities at a glance

What each family's API actually exposes for the fleet widget — recorded here so it need not be re-derived from the
upstream sources every time:

| family    | MAC address                              | temperature sensors                              |
| --------- | ---------------------------------------- | ------------------------------------------------ |
| **BOS+**  | `mac_address` on `/api/v1/miner/details` | one per hashboard → a genuine min/avg/max spread |
| **AxeOS** | `macAddr` on `/api/system/info`          | `temp`, plus `temp2` on multi-sensor boards      |
| **uBOS**  | **none** — the API carries no MAC field  | a **single** `temperatureCelsius`                |

MAC is best-effort per the ticket ("IP address and/or MAC where available"): BOS+ and AxeOS expose it, uBOS does not.
Only multi-sensor devices have a real temperature spread; single-sensor uBOS reports one value — the model should not
fabricate a min/avg/max from it.

## `boser-openapi.json` — BOS+ (BOSminer) REST API

- **Spec**: Braiins OS Public REST API, `v1.3.0` (OpenAPI 3.1).
- **Source**: `git@gitlab.ii.zone:bos/bos-main.git`, path `open/boser/openapi.json`.
- **Commit**: `14b4e3d975` on branch `mfi/BOM-1584/mbos+ubos` (a feature branch as of 2026-07-14; refresh from `master`
  once it lands).
- **Retrieved**: 2026-07-14, pretty-printed through `jq`.

Refresh:

```sh
git -C <bos-main> show mfi/BOM-1584/mbos+ubos:open/boser/openapi.json | jq . > bmc-netsim/reference/boser-openapi.json
```

Consumed by `families/bos.rs` (parses `/api/v1/miner/{stats,hw/hashboards,details}`) and `src/devices/bos.rs`. MAC and
hostname live on `GetMinerDetailsResponse` (`/api/v1/miner/details`) and `GetNetworkInfoResponse`; hashboard
temperatures on the `Hashboard` / `Temperature` schemas.

## AxeOS (ESP-Miner) — not vendored

Open source. The `/api/system/info` contract (including `macAddr`, `temp`, `temp2`) is specified in
[`main/http_server/openapi.yaml`](https://github.com/bitaxeorg/ESP-Miner/blob/master/main/http_server/openapi.yaml).
Snapshot it here if an offline copy is wanted.

## uBOS (micro-BOS) — source located, not vendored

The uBOS API lives in a **separate repo**, `gitlab.ii.zone/jan.krejci/rusty-boards`, under
`boards/ubos-main/ubos/ubos-api/src` (linked from ticket BDK-506). `bos-main`'s `testing/ubos/` is only a stratum device
simulator, not the widget-facing REST API. Its `SystemInfo` (`/api/system/info`) and `SystemNetwork`
(`/api/system/network`) schemas expose `ip`, `hostname`, and a single `temperatureCelsius` — **no MAC**, one sensor.

Caveat: that firmware is migrating to a Bitaxe-shaped `/api/system/info` (`apiVariant: "ubos-0.1"`, camelCase, no power
field), while our `families/ubos.rs` still reads the legacy `/api/info` (`power_out_mw`, `temperature`, `name`). The
adapter is likely stale against current firmware; netsim mirrors the adapter so the demo is unaffected, but real uBOS
hardware needs a follow-up (tracked in BDK-625). Vendor a snapshot here once the endpoint the widget targets is settled.
