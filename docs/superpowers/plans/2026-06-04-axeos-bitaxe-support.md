# AxeOS / Bitaxe Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add AxeOS / Bitaxe discovery, model hints, and `/api/system/info` telemetry to the fleet-management widget.

**Architecture:** Add one `BitaxeAdapter` that supports both upstream ESP-Miner and ESP-Miner-NerdQAxePlus. Discovery
uses `_axeos._sub._http._tcp`; mDNS TXT records become an optional model hint applied at ingest time; telemetry uses the
legacy flat `GET /api/system/info` endpoint. The existing family-generic round-robin driver stays unchanged except for
mapping `DeviceFamily::Bitaxe` to the new adapter.

**Tech Stack:** Rust 2024, `bmc-wasm-sdk`, `FamilyAdapter` trait objects, pure host unit tests for parsing and device
state, wasm-only mDNS/fetch wiring in `lib.rs` and `session.rs`.

---

## Background the implementer needs

- Work in the existing worktree `/home/fbw/doc/work/bmc-main_fbo-BDK-506-fleet-management`.
- The widget crate is `widgets-wasm/fleet-management`, inside the separate `widgets-wasm` cargo workspace.
- The current code already has `DeviceFamily::Bitaxe`, but `session::adapter_for(DeviceFamily::Bitaxe)` returns `None`.
- mDNS `Found` events delivered to widgets already include TXT records at `/txt/<key>`. For AxeOS, use
  `/txt/family`, `/txt/board`, `/txt/asic`, and `/txt/asic_count` when present.
- ESP-Miner and ESP-Miner-NerdQAxePlus both provide the current row readings on `/api/system/info`:
  `hashRate`, `power`, `temp`, and `uptimeSeconds`.
- Do not add `/api/v2/*` support in this plan. The spec defers it until a concrete compatibility gap appears.
- The worktree may have unrelated dirty files. Stage only files named by each task.

## Verification commands

Run these from the repository root unless the command says otherwise:

- Host tests:
  `rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml`
- Host clippy:
  `rtk nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`
- Wasm clippy:
  run from `widgets-wasm`: `rtk nix develop ..# -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`
- Formatting:
  `rtk nix fmt`

`nix` may print a dirty-tree warning because this branch has other work in progress. That warning is not a failure.

## File structure

| File                                                 | Responsibility                                                                 |
| ---------------------------------------------------- | ------------------------------------------------------------------------------ |
| `widgets-wasm/fleet-management/src/adapter.rs`       | Extend `DiscoveredDevice` with an optional discovery-time `MinerModel` hint.   |
| `widgets-wasm/fleet-management/src/device.rs`        | Add tested model-hint application on `DeviceList`.                             |
| `widgets-wasm/fleet-management/src/families/bos.rs`  | Preserve BOS behavior by returning no model hint from discovery.               |
| `widgets-wasm/fleet-management/src/families/ubos.rs` | Preserve uBOS behavior by returning no model hint from discovery.              |
| `widgets-wasm/fleet-management/src/lib.rs`           | Apply model hints during ingest and register the AxeOS mDNS browse.            |
| `widgets-wasm/fleet-management/src/families.rs`      | Declare the new `bitaxe` module.                                               |
| `widgets-wasm/fleet-management/src/families/bitaxe.rs` | New adapter: `_axeos` discovery, TXT model hints, `/api/system/info` parsing. |
| `widgets-wasm/fleet-management/src/session.rs`       | Map `DeviceFamily::Bitaxe` to `BitaxeAdapter`.                                 |

`manifest.json` and `render.rs` do not change.

---

### Task 1: Add discovery model-hint plumbing

**Files:**

- Modify: `widgets-wasm/fleet-management/src/adapter.rs`
- Modify: `widgets-wasm/fleet-management/src/device.rs`
- Modify: `widgets-wasm/fleet-management/src/families/bos.rs`
- Modify: `widgets-wasm/fleet-management/src/families/ubos.rs`
- Modify: `widgets-wasm/fleet-management/src/lib.rs`

- [ ] **Step 1: Write failing `DeviceList` tests for model hints**

In `widgets-wasm/fleet-management/src/device.rs`, add these tests inside the existing `#[cfg(test)] mod tests` block,
after `apply_model_stamps_model_onto_device`:

```rust
    #[test]
    fn upsert_with_model_hint_stamps_model_onto_new_device() {
        let mut list = DeviceList::new();
        list.upsert_with_model_hint(
            identity("axe._http._tcp.local.", "10.0.0.8"),
            Some(model("Bitaxe Gamma 602")),
        );
        let dev = list.iter().next().expect("device present");
        assert_eq!(
            dev.model.as_ref().map(|m| m.name.as_str()),
            Some("Bitaxe Gamma 602")
        );
    }

    #[test]
    fn upsert_with_no_model_hint_preserves_existing_model() {
        let mut list = DeviceList::new();
        list.upsert_with_model_hint(
            identity("axe._http._tcp.local.", "10.0.0.8"),
            Some(model("Bitaxe Gamma 602")),
        );
        list.upsert_with_model_hint(identity("axe._http._tcp.local.", "10.0.0.9"), None);
        let dev = list.iter().next().expect("device present");
        assert_eq!(dev.identity.host, "10.0.0.9");
        assert_eq!(
            dev.model.as_ref().map(|m| m.name.as_str()),
            Some("Bitaxe Gamma 602")
        );
    }
```

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml upsert_with_model_hint`

Expected: FAIL with `no method named 'upsert_with_model_hint' found`.

- [ ] **Step 3: Extend `DiscoveredDevice` with a model hint**

In `widgets-wasm/fleet-management/src/adapter.rs`, import `MinerModel` and update `DiscoveredDevice`.

Change the import block from:

```rust
use crate::model::ModelAccumulator;
```

to:

```rust
use crate::model::{MinerModel, ModelAccumulator};
```

Change the struct to:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredDevice {
    pub identity: DeviceIdentity,
    pub model_hint: Option<MinerModel>,
}
```

- [ ] **Step 4: Preserve BOS discovery behavior**

In `widgets-wasm/fleet-management/src/families/bos.rs`, update the `parse_found` return value to include
`model_hint: None`.

Use this body:

```rust
    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice> {
        let (name, host, port) = extract_endpoint(json)?;
        Some(DiscoveredDevice {
            identity: DeviceIdentity {
                id: DeviceId::new(name.clone()),
                family: DeviceFamily::Bos,
                name,
                host,
                port,
            },
            model_hint: None,
        })
    }
```

- [ ] **Step 5: Preserve uBOS discovery behavior**

In `widgets-wasm/fleet-management/src/families/ubos.rs`, update the `parse_found` return value to include
`model_hint: None`.

Use this body:

```rust
    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice> {
        let (name, host, port) = extract_endpoint(json)?;
        Some(DiscoveredDevice {
            identity: DeviceIdentity {
                id: DeviceId::new(name.clone()),
                family: DeviceFamily::Ubos,
                name,
                host,
                port,
            },
            model_hint: None,
        })
    }
```

- [ ] **Step 6: Add the model-hint `DeviceList` method**

In `widgets-wasm/fleet-management/src/device.rs`, add this method immediately after `upsert`:

```rust
    /// Insert or update a discovered device and apply an optional discovery
    /// model hint. A missing hint leaves any existing model intact, so later
    /// rediscovery does not erase a model learned from telemetry.
    pub fn upsert_with_model_hint(
        &mut self,
        identity: DeviceIdentity,
        model_hint: Option<MinerModel>,
    ) {
        let id = identity.id.clone();
        self.upsert(identity);
        if let Some(model) = model_hint {
            self.apply_model(&id, model);
        }
    }
```

- [ ] **Step 7: Apply model hints during wasm ingest**

In `widgets-wasm/fleet-management/src/lib.rs`, replace the `ingest` helper with:

```rust
#[cfg(target_arch = "wasm32")]
fn ingest(adapter: &dyn FamilyAdapter, json: &str) {
    let doc = JsonDoc::parse(json.as_bytes());
    if let Some(found) = adapter.parse_found(&doc) {
        let identity = found.identity;
        let model_hint = found.model_hint;
        DEVICES.with(|d| d.borrow_mut().upsert_with_model_hint(identity, model_hint));
        session::ensure_running();
        request_frame();
    }
}
```

- [ ] **Step 8: Run tests to verify the model-hint plumbing passes**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml upsert_with_model_hint`

Expected: PASS.

- [ ] **Step 9: Run full host tests and clippy**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml`

Expected: PASS.

Run:
`rtk nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`

Expected: PASS.

- [ ] **Step 10: Run wasm clippy**

Run:
from `widgets-wasm`: `rtk nix develop ..# -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`

Expected: PASS.

- [ ] **Step 11: Format and commit**

Run:

```bash
rtk nix fmt
rtk git add widgets-wasm/fleet-management/src/adapter.rs \
  widgets-wasm/fleet-management/src/device.rs \
  widgets-wasm/fleet-management/src/families/bos.rs \
  widgets-wasm/fleet-management/src/families/ubos.rs \
  widgets-wasm/fleet-management/src/lib.rs
rtk git commit -F - <<'EOF'
fleet-management: Apply discovery model hints #BDK-506

- carry optional model hints from family discovery adapters
- stamp discovery model hints onto devices during ingest
- keep absent hints from clearing previously learned models
EOF
```

---

### Task 2: Add the Bitaxe adapter and parser tests

**Files:**

- Create: `widgets-wasm/fleet-management/src/families/bitaxe.rs`
- Modify: `widgets-wasm/fleet-management/src/families.rs`

- [ ] **Step 1: Declare the new module**

In `widgets-wasm/fleet-management/src/families.rs`, add:

```rust
pub mod bitaxe;
```

- [ ] **Step 2: Add the initial failing adapter tests**

Create `widgets-wasm/fleet-management/src/families/bitaxe.rs` with this test scaffold first:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::adapter::{DiscoveredDevice, FamilyAdapter};
use crate::device::{DeviceFamily, DeviceId, DeviceIdentity};
use crate::discovery::{JsonLookup, extract_endpoint};
use crate::model::{MinerModel, ModelAccumulator};
use crate::telemetry::TelemetryReading;

const EP_INFO: &str = "/info";

pub const BITAXE_TELEMETRY_ENDPOINTS: &[&str] = &[EP_INFO];
pub const BITAXE_SERVICE_TYPES: &[&str] = &["_axeos._sub._http._tcp"];

pub struct BitaxeAdapter;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::tests_support::MapJson;

    fn axeos_found() -> MapJson {
        let mut json = MapJson::default();
        json.strings.insert("/service_type", "_http._tcp.local.");
        json.strings
            .insert("/name", "Bitaxe Gamma 602 (A1B2)._http._tcp.local.");
        json.strings.insert("/host", "192.168.1.42");
        json.ints.insert("/port", 80);
        json.strings.insert("/txt/family", "Gamma");
        json.strings.insert("/txt/board", "602");
        json.strings.insert("/txt/asic", "BM1370");
        json.strings.insert("/txt/asic_count", "1");
        json
    }

    #[test]
    fn browses_the_axeos_subtype() {
        assert_eq!(
            BitaxeAdapter.browse_service_types(),
            &["_axeos._sub._http._tcp"]
        );
    }

    #[test]
    fn parses_a_bitaxe_device_and_stamps_family() {
        let found = BitaxeAdapter
            .parse_found(&axeos_found())
            .expect("device parsed");
        assert_eq!(
            found.identity.id.as_str(),
            "Bitaxe Gamma 602 (A1B2)._http._tcp.local."
        );
        assert_eq!(found.identity.host, "192.168.1.42");
        assert_eq!(found.identity.port, 80);
        assert_eq!(found.identity.family, DeviceFamily::Bitaxe);
    }

    #[test]
    fn rejects_event_missing_host() {
        let mut json = axeos_found();
        json.strings.remove("/host");
        assert_eq!(BitaxeAdapter.parse_found(&json), None);
    }

    #[test]
    fn parses_txt_model_hint_from_discovery() {
        let found = BitaxeAdapter
            .parse_found(&axeos_found())
            .expect("device parsed");
        let model = found.model_hint.expect("model hint present");
        assert_eq!(model.id, "axeos:Gamma:602");
        assert_eq!(model.name, "Bitaxe Gamma 602");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, Some(1));
        assert_eq!(model.nominal_hashrate_ths, None);
    }

    #[test]
    fn malformed_txt_omits_only_model_hint() {
        let mut json = axeos_found();
        json.strings.remove("/txt/family");
        json.strings.remove("/txt/board");
        json.strings.insert("/txt/asic_count", "not-a-number");
        let found = BitaxeAdapter
            .parse_found(&json)
            .expect("identity still parses");
        assert_eq!(found.identity.host, "192.168.1.42");
        assert_eq!(found.model_hint, None);
    }

    fn info_json() -> MapJson {
        let mut j = MapJson::default();
        j.floats.insert("/hashRate", 1_071.197_262_612);
        j.floats.insert("/power", 35.5);
        j.floats.insert("/temp", 59.0);
        j.ints.insert("/uptimeSeconds", 382);
        j
    }

    #[test]
    fn parses_system_info_into_all_four_readings() {
        let mut r = TelemetryReading::default();
        BitaxeAdapter.parse_telemetry("/info", &info_json(), &mut r);
        let hr = r.current_hashrate_ths.expect("hashrate present");
        assert!((hr - 1.071_197_3).abs() < 1e-4, "got {hr}");
        assert_eq!(r.power_w, Some(35.5));
        assert_eq!(r.temperature_c, Some(59.0));
        assert_eq!(r.uptime_s, Some(382));
    }

    #[test]
    fn negative_uptime_is_ignored() {
        let mut j = info_json();
        j.ints.insert("/uptimeSeconds", -1);
        let mut r = TelemetryReading::default();
        BitaxeAdapter.parse_telemetry("/info", &j, &mut r);
        assert_eq!(r.uptime_s, None);
    }

    #[test]
    fn parse_clears_owned_fields_that_vanished_from_response() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(99.0),
            power_w: Some(10.0),
            temperature_c: Some(50.0),
            uptime_s: Some(123),
            nominal_hashrate_ths: None,
        };
        BitaxeAdapter.parse_telemetry("/info", &MapJson::default(), &mut r);
        assert_eq!(r, TelemetryReading::default());
    }

    #[test]
    fn reset_clears_the_endpoint_fields() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(1.0),
            power_w: Some(35.0),
            temperature_c: Some(59.0),
            uptime_s: Some(382),
            nominal_hashrate_ths: Some(7.0),
        };
        BitaxeAdapter.reset_telemetry("/info", &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(r.temperature_c, None);
        assert_eq!(r.uptime_s, None);
        assert_eq!(r.nominal_hashrate_ths, Some(7.0));
    }

    #[test]
    fn parse_model_prefers_device_model() {
        let mut j = MapJson::default();
        j.strings.insert("/deviceModel", "NerdQAxe+");
        j.strings.insert("/boardVersion", "602");
        j.strings.insert("/ASICModel", "BM1370");
        j.ints.insert("/asicCount", 4);
        let mut acc = ModelAccumulator::default();
        BitaxeAdapter.parse_model("/info", &j, &mut acc);
        let model = acc
            .into_model()
            .expect("deviceModel creates a complete model");
        assert_eq!(model.id, "NerdQAxe+");
        assert_eq!(model.name, "NerdQAxe+");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, Some(4));
        assert_eq!(model.nominal_hashrate_ths, None);
    }

    #[test]
    fn parse_model_falls_back_to_board_version() {
        let mut j = MapJson::default();
        j.strings.insert("/boardVersion", "602");
        j.strings.insert("/ASICModel", "BM1370");
        let mut acc = ModelAccumulator::default();
        BitaxeAdapter.parse_model("/info", &j, &mut acc);
        let model = acc
            .into_model()
            .expect("boardVersion creates a complete model");
        assert_eq!(model.id, "axeos-board:602");
        assert_eq!(model.name, "Bitaxe board 602");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, None);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml families::bitaxe`

Expected: FAIL because `BitaxeAdapter` does not implement `FamilyAdapter`.

- [ ] **Step 4: Add helper functions and the adapter implementation**

In the same file, insert this code above the `#[cfg(test)] mod tests` block:

```rust
fn non_empty(json: &dyn JsonLookup, path: &str) -> Option<String> {
    json.str(path).filter(|s| !s.is_empty())
}

fn txt_model_hint(json: &dyn JsonLookup) -> Option<MinerModel> {
    let family = non_empty(json, "/txt/family");
    let board = non_empty(json, "/txt/board");
    let (id, name) = match (family.as_deref(), board.as_deref()) {
        (Some(family), Some(board)) => (
            bmc_wasm_sdk::fmt!("axeos:{family}:{board}"),
            bmc_wasm_sdk::fmt!("Bitaxe {family} {board}"),
        ),
        (None, Some(board)) => (
            bmc_wasm_sdk::fmt!("axeos:{board}"),
            bmc_wasm_sdk::fmt!("Bitaxe board {board}"),
        ),
        (Some(family), None) => (
            bmc_wasm_sdk::fmt!("axeos:{family}"),
            bmc_wasm_sdk::fmt!("Bitaxe {family}"),
        ),
        (None, None) => return None,
    };
    let chip_count = non_empty(json, "/txt/asic_count").and_then(|s| s.parse::<u32>().ok());
    Some(MinerModel {
        id,
        name,
        chip_type: non_empty(json, "/txt/asic"),
        chip_count,
        nominal_hashrate_ths: None,
    })
}

impl FamilyAdapter for BitaxeAdapter {
    fn browse_service_types(&self) -> &'static [&'static str] {
        BITAXE_SERVICE_TYPES
    }

    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice> {
        let (name, host, port) = extract_endpoint(json)?;
        Some(DiscoveredDevice {
            identity: DeviceIdentity {
                id: DeviceId::new(name.clone()),
                family: DeviceFamily::Bitaxe,
                name,
                host,
                port,
            },
            model_hint: txt_model_hint(json),
        })
    }

    fn api_base_path(&self) -> &'static str {
        "/api/system"
    }

    fn telemetry_endpoints(&self) -> &'static [&'static str] {
        BITAXE_TELEMETRY_ENDPOINTS
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "sensor values fit in f32 for realistic readings"
    )]
    fn parse_telemetry(
        &self,
        endpoint: &str,
        json: &dyn JsonLookup,
        reading: &mut TelemetryReading,
    ) {
        self.reset_telemetry(endpoint, reading);
        if endpoint == EP_INFO {
            if let Some(ghs) = json.f64("/hashRate") {
                reading.current_hashrate_ths = Some((ghs / 1_000.0) as f32);
            }
            if let Some(watts) = json.f64("/power") {
                reading.power_w = Some(watts as f32);
            }
            if let Some(c) = json.f64("/temp") {
                reading.temperature_c = Some(c as f32);
            }
            if let Some(uptime) = json
                .i64("/uptimeSeconds")
                .and_then(|v| u64::try_from(v).ok())
            {
                reading.uptime_s = Some(uptime);
            }
        }
    }

    fn reset_telemetry(&self, endpoint: &str, reading: &mut TelemetryReading) {
        if endpoint == EP_INFO {
            reading.current_hashrate_ths = None;
            reading.power_w = None;
            reading.temperature_c = None;
            reading.uptime_s = None;
        }
    }

    fn parse_model(&self, endpoint: &str, json: &dyn JsonLookup, model: &mut ModelAccumulator) {
        if endpoint != EP_INFO {
            return;
        }
        if let Some(device_model) = non_empty(json, "/deviceModel") {
            model.id = Some(device_model.clone());
            model.name = Some(device_model);
        } else if let Some(board) = non_empty(json, "/boardVersion") {
            model.id = Some(bmc_wasm_sdk::fmt!("axeos-board:{board}"));
            model.name = Some(bmc_wasm_sdk::fmt!("Bitaxe board {board}"));
        }
        if let Some(asic) = non_empty(json, "/ASICModel") {
            model.chip_type = Some(asic);
        }
        if let Some(chip_count) = json.i64("/asicCount").and_then(|v| u32::try_from(v).ok()) {
            model.chip_count = Some(chip_count);
        }
    }
}
```

- [ ] **Step 5: Run Bitaxe adapter tests**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml families::bitaxe`

Expected: PASS.

- [ ] **Step 6: Run full host tests and clippy**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml`

Expected: PASS.

Run:
`rtk nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`

Expected: PASS.

- [ ] **Step 7: Run wasm clippy**

Run:
from `widgets-wasm`: `rtk nix develop ..# -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`

Expected: PASS. If wasm clippy reports a new Bitaxe item as dead code, add a temporary
`#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "wired into the driver in the next task"))]` only to that
item, then rerun the command. Remove that temporary expectation in Task 3 after runtime wiring makes the item reachable.

- [ ] **Step 8: Format and commit**

Run:

```bash
rtk nix fmt
rtk git add widgets-wasm/fleet-management/src/families.rs \
  widgets-wasm/fleet-management/src/families/bitaxe.rs
rtk git commit -F - <<'EOF'
fleet-management: Add AxeOS Bitaxe adapter #BDK-506

- browse AxeOS miners through the _axeos subtype
- parse discovery TXT records into Bitaxe model hints
- map /api/system/info telemetry and model fields
EOF
```

---

### Task 3: Wire Bitaxe into the driver and runtime

**Files:**

- Modify: `widgets-wasm/fleet-management/src/session.rs`
- Modify: `widgets-wasm/fleet-management/src/lib.rs`
- Modify: `widgets-wasm/fleet-management/src/device.rs`
- Modify: `widgets-wasm/fleet-management/src/families/bitaxe.rs` only if Task 2 added a temporary dead-code expectation

- [ ] **Step 1: Write the failing `adapter_for` test update**

In `widgets-wasm/fleet-management/src/session.rs`, replace the existing `adapter_for_maps_known_families_and_rejects_bitaxe`
test with:

```rust
    #[test]
    fn adapter_for_maps_every_supported_family() {
        assert_eq!(
            adapter_for(DeviceFamily::Bos).map(FamilyAdapter::browse_service_types),
            Some(crate::families::bos::BOS_SERVICE_TYPES)
        );
        assert_eq!(
            adapter_for(DeviceFamily::Ubos).map(FamilyAdapter::browse_service_types),
            Some(crate::families::ubos::UBOS_SERVICE_TYPES)
        );
        assert_eq!(
            adapter_for(DeviceFamily::Bitaxe).map(FamilyAdapter::browse_service_types),
            Some(crate::families::bitaxe::BITAXE_SERVICE_TYPES)
        );
    }
```

- [ ] **Step 2: Run the targeted test to verify it fails**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml adapter_for_maps_every_supported_family`

Expected: FAIL because `adapter_for(DeviceFamily::Bitaxe)` returns `None`.

- [ ] **Step 3: Map `DeviceFamily::Bitaxe` to `BitaxeAdapter`**

In `widgets-wasm/fleet-management/src/session.rs`, add the import:

```rust
use crate::families::bitaxe::BitaxeAdapter;
```

Then replace `adapter_for` with:

```rust
/// Map a device family to its adapter.
#[must_use]
pub fn adapter_for(family: DeviceFamily) -> Option<&'static dyn FamilyAdapter> {
    match family {
        DeviceFamily::Bos => Some(&BosAdapter),
        DeviceFamily::Ubos => Some(&UbosAdapter),
        DeviceFamily::Bitaxe => Some(&BitaxeAdapter),
    }
}
```

Leave the return type as `Option<&'static dyn FamilyAdapter>` so this remains a surgical change; the unsupported-family
path in the wasm driver becomes dormant but harmless.

- [ ] **Step 4: Wire the mDNS browse in `lib.rs`**

In `widgets-wasm/fleet-management/src/lib.rs`, add this import with the other family imports:

```rust
use families::bitaxe::BitaxeAdapter;
```

Add this handler after `on_ubos_event`:

```rust
#[cfg(target_arch = "wasm32")]
fn on_bitaxe_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&BitaxeAdapter, json),
        mdns::MdnsEvent::Removed(name) => {
            let id = DeviceId::new(*name);
            session::remove_token(&id);
            DEVICES.with(|d| d.borrow_mut().remove(&id));
            request_frame();
        }
    }
}
```

In `init()`, add this browse after the uBOS browse:

```rust
    if mdns::mdns_browse(BitaxeAdapter.browse_service_types(), on_bitaxe_event).is_none() {
        log_warn!("fleet: AxeOS mDNS browse rejected by host runtime limits");
    }
```

- [ ] **Step 5: Remove the Bitaxe dead-code expectation**

In `widgets-wasm/fleet-management/src/device.rs`, change the enum from:

```rust
pub enum DeviceFamily {
    Bos,
    Ubos,
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "part of the generic device model; constructed once uBOS and Bitaxe adapters land"
        )
    )]
    Bitaxe,
}
```

to:

```rust
pub enum DeviceFamily {
    Bos,
    Ubos,
    Bitaxe,
}
```

If Task 2 added a temporary wasm dead-code expectation in `families/bitaxe.rs`, remove it now and rerun wasm clippy in
Step 8.

- [ ] **Step 6: Run the targeted adapter test**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml adapter_for_maps_every_supported_family`

Expected: PASS.

- [ ] **Step 7: Run full host tests and clippy**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml`

Expected: PASS.

Run:
`rtk nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`

Expected: PASS.

- [ ] **Step 8: Run wasm clippy**

Run:
from `widgets-wasm`: `rtk nix develop ..# -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`

Expected: PASS with no unfulfilled `#[expect(dead_code)]` diagnostics.

- [ ] **Step 9: Format and commit**

Run:

```bash
rtk nix fmt
rtk git add widgets-wasm/fleet-management/src/session.rs \
  widgets-wasm/fleet-management/src/lib.rs \
  widgets-wasm/fleet-management/src/device.rs \
  widgets-wasm/fleet-management/src/families/bitaxe.rs
rtk git commit -F - <<'EOF'
fleet-management: Discover and poll AxeOS miners #BDK-506

- register the AxeOS mDNS browse in the widget runtime
- map Bitaxe devices to the AxeOS adapter
- make Bitaxe a constructed fleet-management family
EOF
```

---

### Task 4: Final verification

**Files:**

- No planned edits.

- [ ] **Step 1: Run host tests**

Run:
`rtk nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml`

Expected: PASS.

- [ ] **Step 2: Run host clippy**

Run:
`rtk nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`

Expected: PASS.

- [ ] **Step 3: Run wasm clippy**

Run:
from `widgets-wasm`: `rtk nix develop ..# -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`

Expected: PASS.

- [ ] **Step 4: Run formatter check**

Run:
`rtk nix fmt`

Expected: no unintended file churn. If formatter changes files touched in Tasks 1-3, inspect and commit them with the
task that introduced the formatting difference before continuing.

- [ ] **Step 5: Optional live testbed check**

Run from the repository root:

```bash
rtk nix develop -c just wasm::dev fleet-management
```

Expected with an AxeOS device on the LAN:

- the device is discovered through `_axeos._sub._http._tcp`
- the row shows family `Bitaxe`
- the model column shows a TXT-derived model before or by the first telemetry pass
- `/api/system/info` readings populate current hashrate, power, temperature, and uptime
- stopping the device or blocking the endpoint clears the row readings to `N/A`

If the environment lacks LAN AxeOS hardware, record that this live check was not run. Do not claim hardware verification.

---

## Self-review checklist

- Spec coverage:
  - `_axeos._sub._http._tcp` discovery: Task 2 tests browse type; Task 3 registers browse.
  - One adapter for both codebases: Task 2 uses only shared `/api/system/info` fields.
  - TXT model hints: Task 2 parses hints; Task 1 proves hints land on `KnownDevice.model`.
  - Telemetry mapping: Task 2 covers hashRate, power, temp, uptimeSeconds, stale clearing, and negative uptime.
  - No auth and no `/api/v2/*`: Task 2 does not override auth methods and uses only `/api/system/info`.
  - Driver dispatch: Task 3 maps `DeviceFamily::Bitaxe`.
- Placeholder scan:
  - No placeholder sections are intentionally left for implementation.
- Type consistency:
  - `DiscoveredDevice.model_hint: Option<MinerModel>` is introduced before `lib.rs` consumes it.
  - `DeviceList::upsert_with_model_hint` accepts `DeviceIdentity` plus `Option<MinerModel>`, matching `ingest`.
  - `BitaxeAdapter` constants are named `BITAXE_SERVICE_TYPES` and `BITAXE_TELEMETRY_ENDPOINTS`, matching tests.
