# uBOS Family Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add uBOS as a second device family (mDNS discovery + `/api/info` telemetry) to the fleet-management WASM widget, making the telemetry driver dispatch per family instead of assuming BOS.

**Architecture:** A new `UbosAdapter` implements the existing `FamilyAdapter` trait. The trait gains one proactive `credential_header()` hook (HTTP Basic for uBOS) that coexists with BOS's reactive token login. The round-robin driver in `session.rs` resolves each device's adapter via a new `adapter_for(family)` lookup rather than calling `BosAdapter` directly. The model-capture machinery (`parse_model`, `ModelAccumulator`, `apply_model`) already exists from the BOS-miner-model change; this slice only adds uBOS's `parse_model` override.

**Tech Stack:** Rust `cdylib` WASM widget on `bmc-wasm-sdk`; `FamilyAdapter` trait objects; thread-local round-robin fetch driver. Pure parsing/dispatch is host-unit-tested; fetch/timing wiring lives behind `#[cfg(target_arch = "wasm32")]` and is verified on the wasm target and in the testbed.

---

## Background the implementer needs

- The widget crate is `widgets-wasm/fleet-management`, a member of the **separate** `widgets-wasm` cargo workspace (not the top-level workspace). All commands below use that crate's manifest explicitly.
- The crate is `crate-type = ["cdylib"]`. On the **wasm** target, unused `pub` items ARE flagged as dead code (`-D warnings`). Consequence: a new adapter or trait method is "unused" on wasm until the driver wires it in. Tasks 1 and 2 therefore verify on the **host** only; the first wasm-target check happens in Task 3, once the driver references the new code. This is expected — do not try to make wasm clippy pass at Tasks 1–2.
- `JsonLookup` (in `discovery.rs`) is the parse-time abstraction: `str(path) -> Option<String>`, `i64(path) -> Option<i64>`, `f64(path) -> Option<f64>`. The host coerces integer JSON to `f64`, so `f64()` reads uBOS's integer fields. Tests use the `MapJson` mock (`discovery::tests_support::MapJson`) with separate `strings` / `ints` / `floats` maps.
- The reset-then-populate telemetry contract: `parse_telemetry` first calls `reset_telemetry` for its endpoint (clearing exactly that endpoint's owned fields), then sets whatever the payload contains — so a vanished field goes `Some -> None`.
- `base64("root:root")` = `cm9vdDpyb290`.

## Verification commands (used throughout)

- **Host tests:** `nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml`
- **Host clippy:** `nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`
- **Wasm clippy:** `cd widgets-wasm && nix develop ..# -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings; cd ..`
- **Format before each commit:** `nix fmt`

All three commands have been confirmed to run in this environment. The "Git tree is dirty" warnings nix prints are benign.

## File structure

| File | Change |
|---|---|
| `widgets-wasm/fleet-management/src/adapter.rs` | Add `credential_header` default trait method (Task 1) |
| `widgets-wasm/fleet-management/src/families/bos.rs` | Add one test: BOS `credential_header` is `None` (Task 1) |
| `widgets-wasm/fleet-management/src/families/ubos.rs` | **New** — `UbosAdapter`: discovery, telemetry, credential header, `parse_model` + tests (Task 2) |
| `widgets-wasm/fleet-management/src/families.rs` | Declare `pub mod ubos;` (Task 2) |
| `widgets-wasm/fleet-management/src/session.rs` | Add `adapter_for`; make the driver family-generic; add `adapter_for` test (Task 3) |
| `widgets-wasm/fleet-management/src/device.rs` | Drop the `#[expect(dead_code)]` on `DeviceFamily::Ubos` (Task 3) |
| `widgets-wasm/fleet-management/src/lib.rs` | Add `on_ubos_event` + register the `_ubos._tcp` browse (Task 4) |

`render.rs` and `manifest.json` are unchanged: `render::view` already renders every device, and uBOS adds no parameter this slice.

---

### Task 1: Add the `credential_header` trait hook

**Files:**
- Modify: `widgets-wasm/fleet-management/src/adapter.rs`
- Test: `widgets-wasm/fleet-management/src/families/bos.rs`

- [ ] **Step 1: Write the failing test**

Add this test inside the existing `#[cfg(test)] mod tests` block in `families/bos.rs` (e.g. after `bos_advertises_a_login_endpoint`):

```rust
    #[test]
    fn bos_has_no_proactive_credential_header() {
        assert_eq!(BosAdapter.credential_header(), None);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml bos_has_no_proactive_credential_header`
Expected: FAIL — `no method named 'credential_header' found for struct 'BosAdapter'`.

- [ ] **Step 3: Add the trait method**

In `adapter.rs`, add the method to the `FamilyAdapter` trait immediately after the `parse_model` default method (before the `// Authentication — default NONE.` comment):

```rust
    /// A proactive credential header attached to every request, preferred over
    /// any cached token. Default none; families with reactive token auth (BOS)
    /// leave it none, families with static credentials (uBOS) override it.
    fn credential_header(&self) -> Option<String> {
        None
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml bos_has_no_proactive_credential_header`
Expected: PASS.

- [ ] **Step 5: Host clippy**

Run: `nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`
Expected: clean. (Do NOT run wasm clippy yet — `credential_header` is unused on wasm until Task 3.)

- [ ] **Step 6: Format and commit**

```bash
nix fmt
git add widgets-wasm/fleet-management/src/adapter.rs widgets-wasm/fleet-management/src/families/bos.rs
git commit -F - <<'EOF'
fleet-management: Add proactive credential header hook #BDK-506

- add FamilyAdapter::credential_header defaulting to none
- prepare for a static HTTP Basic header alongside token auth
EOF
```

---

### Task 2: Add the `UbosAdapter` family

**Files:**
- Create: `widgets-wasm/fleet-management/src/families/ubos.rs`
- Modify: `widgets-wasm/fleet-management/src/families.rs`

- [ ] **Step 1: Declare the module**

In `families.rs`, add below `pub mod bos;`:

```rust
pub mod ubos;
```

- [ ] **Step 2: Write the adapter and its tests**

Create `widgets-wasm/fleet-management/src/families/ubos.rs` with exactly this content:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::adapter::{DiscoveredDevice, FamilyAdapter};
use crate::device::{DeviceFamily, DeviceId, DeviceIdentity};
use crate::discovery::{JsonLookup, extract_endpoint};
use crate::model::ModelAccumulator;
use crate::telemetry::TelemetryReading;

const EP_INFO: &str = "/info";

pub const UBOS_TELEMETRY_ENDPOINTS: &[&str] = &[EP_INFO];

/// uBOS advertises the full service type `_ubos._tcp` (not a subtype of
/// `_http._tcp` like BOS), so every event on this browse is a uBOS device.
pub const UBOS_SERVICE_TYPES: &[&str] = &["_ubos._tcp"];

pub struct UbosAdapter;

impl FamilyAdapter for UbosAdapter {
    fn browse_service_types(&self) -> &'static [&'static str] {
        UBOS_SERVICE_TYPES
    }

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
        })
    }

    fn api_base_path(&self) -> &'static str {
        "/api"
    }

    fn telemetry_endpoints(&self) -> &'static [&'static str] {
        UBOS_TELEMETRY_ENDPOINTS
    }

    // Hardcoded HTTP Basic `root:root`; `cm9vdDpyb290` is base64("root:root").
    // The owned return keeps a future configurable password local to this
    // adapter without changing the trait.
    fn credential_header(&self) -> Option<String> {
        Some("Authorization: Basic cm9vdDpyb290".to_owned())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "sensor values fit in f32 for realistic readings"
    )]
    fn parse_telemetry(&self, endpoint: &str, json: &dyn JsonLookup, reading: &mut TelemetryReading) {
        self.reset_telemetry(endpoint, reading);
        if endpoint == EP_INFO {
            if let Some(hs) = json.f64("/hashrate") {
                reading.current_hashrate_ths = Some((hs / 1e12) as f32);
            }
            if let Some(mw) = json.f64("/power_out_mw") {
                reading.power_w = Some((mw / 1_000.0) as f32);
            }
            if let Some(c) = json.f64("/temperature") {
                reading.temperature_c = Some(c as f32);
            }
            if let Some(uptime) = json.i64("/uptime").and_then(|v| u64::try_from(v).ok()) {
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
        if endpoint == EP_INFO
            && let Some(name) = json.str("/name").filter(|s| !s.is_empty())
        {
            model.name = Some(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::tests_support::MapJson;

    fn ubos_found() -> MapJson {
        let mut json = MapJson::default();
        json.strings.insert("/service_type", "_ubos._tcp.local.");
        json.strings.insert("/name", "bmm-01._ubos._tcp.local.");
        json.strings.insert("/host", "192.168.89.109");
        json.ints.insert("/port", 8080);
        json
    }

    #[test]
    fn browses_the_full_ubos_service_type() {
        assert_eq!(UbosAdapter.browse_service_types(), &["_ubos._tcp"]);
    }

    #[test]
    fn parses_a_ubos_device_and_stamps_family() {
        let found = UbosAdapter.parse_found(&ubos_found()).expect("device parsed");
        assert_eq!(found.identity.id.as_str(), "bmm-01._ubos._tcp.local.");
        assert_eq!(found.identity.host, "192.168.89.109");
        assert_eq!(found.identity.port, 8080);
        assert_eq!(found.identity.family, DeviceFamily::Ubos);
    }

    #[test]
    fn rejects_event_missing_port() {
        let mut json = ubos_found();
        json.ints.remove("/port");
        assert_eq!(UbosAdapter.parse_found(&json), None);
    }

    fn info_json() -> MapJson {
        let mut j = MapJson::default();
        j.floats.insert("/hashrate", 1_071_197_262_612.0);
        j.floats.insert("/power_out_mw", 35_000.0);
        j.floats.insert("/temperature", 59.0);
        j.ints.insert("/uptime", 382);
        j
    }

    #[test]
    fn parses_info_into_all_four_readings() {
        let mut r = TelemetryReading::default();
        UbosAdapter.parse_telemetry("/info", &info_json(), &mut r);
        let hr = r.current_hashrate_ths.expect("hashrate present");
        assert!((hr - 1.071_197_3).abs() < 1e-4, "got {hr}");
        assert_eq!(r.power_w, Some(35.0));
        assert_eq!(r.temperature_c, Some(59.0));
        assert_eq!(r.uptime_s, Some(382));
    }

    #[test]
    fn parse_clears_owned_field_that_vanished_from_response() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(99.0),
            power_w: Some(10.0),
            temperature_c: Some(50.0),
            uptime_s: Some(123),
            nominal_hashrate_ths: None,
        };
        UbosAdapter.parse_telemetry("/info", &MapJson::default(), &mut r);
        assert_eq!(r, TelemetryReading::default());
    }

    #[test]
    fn reset_clears_the_endpoints_fields() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(1.0),
            power_w: Some(35.0),
            temperature_c: Some(59.0),
            uptime_s: Some(382),
            nominal_hashrate_ths: Some(7.0),
        };
        UbosAdapter.reset_telemetry("/info", &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        assert_eq!(r.temperature_c, None);
        assert_eq!(r.uptime_s, None);
        assert_eq!(r.nominal_hashrate_ths, Some(7.0));
    }

    #[test]
    fn credential_header_is_basic_root_root() {
        assert_eq!(
            UbosAdapter.credential_header(),
            Some("Authorization: Basic cm9vdDpyb290".to_owned())
        );
    }

    #[test]
    fn parse_model_sets_name_only() {
        let mut j = MapJson::default();
        j.strings.insert("/name", "BMM Adapter W5500");
        let mut acc = ModelAccumulator::default();
        UbosAdapter.parse_model("/info", &j, &mut acc);
        assert_eq!(acc.name.as_deref(), Some("BMM Adapter W5500"));
        assert_eq!(acc.id, None);
        assert_eq!(acc.chip_type, None);
        assert_eq!(acc.chip_count, None);
    }
}
```

- [ ] **Step 3: Run the uBOS tests**

Run: `nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml families::ubos`
Expected: PASS — 8 tests (`browses_the_full_ubos_service_type`, `parses_a_ubos_device_and_stamps_family`, `rejects_event_missing_port`, `parses_info_into_all_four_readings`, `parse_clears_owned_field_that_vanished_from_response`, `reset_clears_the_endpoints_fields`, `credential_header_is_basic_root_root`, `parse_model_sets_name_only`).

- [ ] **Step 4: Host clippy**

Run: `nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`
Expected: clean. (Still do NOT run wasm clippy — `UbosAdapter` is unused on wasm until Task 3.)

- [ ] **Step 5: Format and commit**

```bash
nix fmt
git add widgets-wasm/fleet-management/src/families.rs widgets-wasm/fleet-management/src/families/ubos.rs
git commit -F - <<'EOF'
fleet-management: Add uBOS family adapter #BDK-506

- discover uBOS miners via the full _ubos._tcp service type
- map /api/info to hashrate, power, temperature, uptime readings
- attach a static HTTP Basic root:root credential header
- capture the model name from the /api/info name field
EOF
```

---

### Task 3: Make the telemetry driver family-generic

**Files:**
- Modify: `widgets-wasm/fleet-management/src/session.rs`
- Modify: `widgets-wasm/fleet-management/src/device.rs`

This task replaces every hardcoded `BosAdapter` reference in the driver with a per-device adapter resolved from `adapter_for(family)`, adds the credential-header attachment, and handles a family with no adapter. It also drops the `DeviceFamily::Ubos` dead-code expectation, because `adapter_for` now references `UbosAdapter` from wasm-reachable code, which constructs the variant on wasm (verified: this is the exact point the expectation becomes unfulfilled).

- [ ] **Step 1: Write the failing `adapter_for` test**

Add this test inside the existing `#[cfg(test)] mod tests` block at the bottom of `session.rs` (after `reachable_only_when_an_endpoint_succeeded`):

```rust
    #[test]
    fn adapter_for_maps_known_families_and_rejects_bitaxe() {
        assert_eq!(
            adapter_for(DeviceFamily::Bos).map(FamilyAdapter::browse_service_types),
            Some(crate::families::bos::BOS_SERVICE_TYPES)
        );
        assert_eq!(
            adapter_for(DeviceFamily::Ubos).map(FamilyAdapter::browse_service_types),
            Some(crate::families::ubos::UBOS_SERVICE_TYPES)
        );
        assert!(adapter_for(DeviceFamily::Bitaxe).is_none());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml adapter_for_maps_known_families_and_rejects_bitaxe`
Expected: FAIL — `cannot find function 'adapter_for'` and unresolved `DeviceFamily` / `FamilyAdapter`.

- [ ] **Step 3: Add the top-level imports and `adapter_for`**

Replace the top of `session.rs` — the lines from the copyright header through the `PassCursor` doc comment — with:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::adapter::FamilyAdapter;
use crate::device::{DeviceFamily, DeviceId};
use crate::families::bos::BosAdapter;
use crate::families::ubos::UbosAdapter;

/// Map a device family to its adapter. `None` for a family with no adapter yet
/// (Bitaxe); the driver then marks such a device unreachable rather than
/// assuming the family is never discovered.
#[must_use]
pub fn adapter_for(family: DeviceFamily) -> Option<&'static dyn FamilyAdapter> {
    match family {
        DeviceFamily::Bos => Some(&BosAdapter),
        DeviceFamily::Ubos => Some(&UbosAdapter),
        DeviceFamily::Bitaxe => None,
    }
}

/// A one-pass cursor over a snapshot of device ids. Snapshotting at pass start
```

(`PassCursor` and `pass_reachable` below this point are unchanged.)

- [ ] **Step 4: Update the driver module imports**

In `session.rs`, inside `mod driver`, replace these four `use` lines:

```rust
    use super::{PassCursor, pass_reachable};
    use crate::adapter::FamilyAdapter;
    use crate::device::DeviceId;
    use crate::families::bos::BosAdapter;
```

with:

```rust
    use super::{PassCursor, adapter_for, pass_reachable};
    use crate::adapter::FamilyAdapter;
    use crate::device::{DeviceFamily, DeviceId};
```

- [ ] **Step 5: Make `base_url` and `current_endpoint` adapter/family aware**

Replace the `base_url` function and the `current_endpoint` function with:

```rust
    fn base_url(adapter: &dyn FamilyAdapter, host: &str, port: u16) -> String {
        fmt!("http://{}:{}{}", host, port, adapter.api_base_path())
    }

    /// Look up the family, host, and port for the cursor's current device.
    fn current_endpoint() -> Option<(DeviceId, DeviceFamily, String, u16)> {
        let id = DRIVER.with(|d| d.borrow().cursor.as_ref()?.current().cloned())?;
        crate::DEVICES.with(|devs| {
            let devs = devs.borrow();
            let dev = devs.iter().find(|d| d.identity.id == id)?;
            Some((
                id.clone(),
                dev.identity.family,
                dev.identity.host.clone(),
                dev.identity.port,
            ))
        })
    }
```

- [ ] **Step 6: Add the no-adapter branch to `begin_device`**

In `begin_device`, between the `if done { ... return; }` block and the `DRIVER.with(|d| { ... })` state-reset block, insert:

```rust
        // A device whose family has no adapter yet cannot be polled. `upsert`
        // left it reachable, so mark it unreachable with cleared telemetry and
        // move on, rather than assuming the family is never discovered.
        if let Some((id, family, _, _)) = current_endpoint()
            && adapter_for(family).is_none()
        {
            log_warn!("fleet: no adapter for discovered device family; marking unreachable");
            crate::DEVICES.with(|devs| {
                devs.borrow_mut()
                    .apply_telemetry(&id, TelemetryReading::default(), false);
            });
            request_frame();
            advance_device();
            return;
        }
```

- [ ] **Step 7: Replace `fetch_endpoint` with the adapter-resolving version**

Replace the entire `fetch_endpoint` function with:

```rust
    /// GET the current endpoint via its family adapter, attaching the adapter's
    /// proactive credential header if any, else a cached token if present.
    fn fetch_endpoint() {
        let Some((id, family, host, port)) = current_endpoint() else {
            advance_device();
            return;
        };
        let Some(adapter) = adapter_for(family) else {
            advance_device();
            return;
        };
        let endpoints = adapter.telemetry_endpoints();
        let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
        if idx >= endpoints.len() {
            finalize_device();
            return;
        }
        let url = fmt!("{}{}", base_url(adapter, &host, port), endpoints[idx]);
        let token = TOKENS.with(|t| t.borrow().get(&id).cloned());
        let header = adapter
            .credential_header()
            .or_else(|| token.map(|t| adapter.auth_header(&t)));
        if FetchRequest::get(&url)
            .headers_opt(header.as_deref())
            .send(on_endpoint)
            .is_none()
        {
            log_warn!("fleet: telemetry send rejected for {}", host);
            record_endpoint(false);
        }
    }
```

- [ ] **Step 8: Replace `on_endpoint` with the adapter-resolving version**

Replace the entire `on_endpoint` function with:

```rust
    /// React to a telemetry reply: re-authenticate on an adapter-reported auth
    /// error (once per device per pass), else fold or reset the endpoint.
    fn on_endpoint(response: &bmc_wasm_sdk::FetchResponse) {
        let Some((id, family, _, _)) = current_endpoint() else {
            advance_device();
            return;
        };
        let Some(adapter) = adapter_for(family) else {
            advance_device();
            return;
        };
        let endpoints = adapter.telemetry_endpoints();
        let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
        let Some(ep) = endpoints.get(idx) else {
            advance_device();
            return;
        };

        let can_reauth = adapter.auth_endpoint().is_some()
            && adapter.is_auth_error(response.status)
            && !DRIVER.with(|d| d.borrow().reauthed);
        if can_reauth {
            DRIVER.with(|d| d.borrow_mut().reauthed = true);
            TOKENS.with(|t| t.borrow_mut().remove(&id));
            login_then_retry();
            return;
        }

        if response.ok() {
            let doc = response.json();
            DRIVER.with(|d| {
                let mut d = d.borrow_mut();
                let d = &mut *d;
                adapter.parse_telemetry(ep, &doc, &mut d.reading);
                adapter.parse_model(ep, &doc, &mut d.model);
            });
            record_endpoint(true);
        } else {
            DRIVER.with(|d| adapter.reset_telemetry(ep, &mut d.borrow_mut().reading));
            record_endpoint(false);
        }
    }
```

- [ ] **Step 9: Replace `login_then_retry` with the adapter-resolving version**

Replace the entire `login_then_retry` function with:

```rust
    /// Log in with the shared password, then retry the SAME endpoint with the
    /// fresh token. A failed login leaves the endpoint as N/A and advances.
    fn login_then_retry() {
        let Some((_, family, host, port)) = current_endpoint() else {
            advance_device();
            return;
        };
        let Some(adapter) = adapter_for(family) else {
            advance_device();
            return;
        };
        let Some(auth_path) = adapter.auth_endpoint() else {
            record_endpoint(false);
            return;
        };
        let password = Params::current().miner_password;
        let url = fmt!("{}{}", base_url(adapter, &host, port), auth_path);
        let body = adapter.login_body(&password);
        if FetchRequest::post(&url)
            .headers("Content-Type: application/json")
            .body(body.as_bytes())
            .send(on_login)
            .is_none()
        {
            log_warn!("fleet: login send rejected for {}", host);
            record_endpoint(false);
        }
    }
```

- [ ] **Step 10: Replace `on_login` with the adapter-resolving version**

Replace the entire `on_login` function with:

```rust
    fn on_login(response: &bmc_wasm_sdk::FetchResponse) {
        let Some((id, family, _, _)) = current_endpoint() else {
            advance_device();
            return;
        };
        let Some(adapter) = adapter_for(family) else {
            advance_device();
            return;
        };
        let token = if response.ok() {
            adapter.parse_login(&response.json())
        } else {
            None
        };
        if let Some(token) = token {
            TOKENS.with(|t| t.borrow_mut().insert(id, token));
            // Retry the same endpoint (endpoint_idx unchanged) with the token.
            fetch_endpoint();
        } else {
            // Login failed: this endpoint is N/A; move on.
            let endpoints = adapter.telemetry_endpoints();
            let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
            if let Some(ep) = endpoints.get(idx) {
                DRIVER.with(|d| adapter.reset_telemetry(ep, &mut d.borrow_mut().reading));
            }
            record_endpoint(false);
        }
    }
```

- [ ] **Step 11: Drop the `DeviceFamily::Ubos` dead-code expectation**

In `device.rs`, in the `DeviceFamily` enum, replace:

```rust
    Bos,
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "part of the generic device model; constructed once uBOS and Bitaxe adapters land"
        )
    )]
    Ubos,
    #[cfg_attr(
```

with:

```rust
    Bos,
    Ubos,
    #[cfg_attr(
```

(Leave the `Bitaxe` variant's expectation and the `family_label` expectation exactly as they are — Bitaxe is still never constructed, and `family_label` is still unused on wasm.)

- [ ] **Step 12: Run host tests**

Run: `nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml`
Expected: PASS — 49 tests, including `adapter_for_maps_known_families_and_rejects_bitaxe`.

- [ ] **Step 13: Host clippy**

Run: `nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`
Expected: clean.

- [ ] **Step 14: Wasm clippy (first wasm check of this slice)**

Run: `cd widgets-wasm && nix develop ..# -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings; cd ..`
Expected: clean. (If it reports `UbosAdapter is never constructed` or `credential_header is never used`, a `BosAdapter` reference was missed in the driver — re-check Steps 7–10. If it reports the `Ubos` expectation is unfulfilled, Step 11 was not applied.)

- [ ] **Step 15: Format and commit**

```bash
nix fmt
git add widgets-wasm/fleet-management/src/session.rs widgets-wasm/fleet-management/src/device.rs
git commit -F - <<'EOF'
fleet-management: Dispatch telemetry per device family #BDK-506

- resolve each device's adapter via adapter_for instead of BosAdapter
- prefer a proactive credential header over a cached token per request
- mark a device with no adapter unreachable instead of skipping it
- construct DeviceFamily::Ubos, so drop its dead-code expectation
EOF
```

---

### Task 4: Discover uBOS miners

**Files:**
- Modify: `widgets-wasm/fleet-management/src/lib.rs`

`lib.rs`'s discovery and entry-point code is `#[cfg(target_arch = "wasm32")]`, so it is only compiled by the wasm target — host clippy/tests do not exercise it. Verification for this task is the wasm clippy build.

- [ ] **Step 1: Import `UbosAdapter`**

In `lib.rs`, after the existing:

```rust
#[cfg(target_arch = "wasm32")]
use families::bos::BosAdapter;
```

add:

```rust
#[cfg(target_arch = "wasm32")]
use families::ubos::UbosAdapter;
```

- [ ] **Step 2: Add the `on_ubos_event` handler**

In `lib.rs`, immediately before the existing `fn ingest(...)` (the `#[cfg(target_arch = "wasm32")] fn ingest`), insert:

```rust
#[cfg(target_arch = "wasm32")]
fn on_ubos_event(_browse: mdns::MdnsBrowse, event: &mdns::MdnsEvent<'_>) {
    match event {
        mdns::MdnsEvent::Found(json) => ingest(&UbosAdapter, json),
        mdns::MdnsEvent::Removed(name) => {
            let id = DeviceId::new(*name);
            session::remove_token(&id);
            DEVICES.with(|d| d.borrow_mut().remove(&id));
            request_frame();
        }
    }
}
```

- [ ] **Step 3: Register the uBOS browse in `init`**

In `init()`, after the BOS browse block and before `request_frame();`, insert:

```rust
    if mdns::mdns_browse(UbosAdapter.browse_service_types(), on_ubos_event).is_none() {
        log_warn!("fleet: uBOS mDNS browse rejected by host runtime limits");
    }
```

(The host allows up to four browses; two is within budget.)

- [ ] **Step 4: Wasm clippy**

Run: `cd widgets-wasm && nix develop ..# -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings; cd ..`
Expected: clean.

- [ ] **Step 5: Host tests + host clippy (regression guard)**

Run: `nix develop -c cargo test --manifest-path widgets-wasm/fleet-management/Cargo.toml`
Run: `nix develop -c cargo clippy --manifest-path widgets-wasm/fleet-management/Cargo.toml --tests -- -D warnings`
Expected: both clean (no change vs Task 3, since `lib.rs` is wasm-only).

- [ ] **Step 6: Format and commit**

```bash
nix fmt
git add widgets-wasm/fleet-management/src/lib.rs
git commit -F - <<'EOF'
fleet-management: Discover uBOS miners via _ubos._tcp #BDK-506

- register a second mDNS browse for the full _ubos._tcp service type
- route its events to the uBOS adapter alongside BOS discovery
EOF
```

---

### Task 5: Testbed verification (manual, against a live uBOS device)

The fetch wiring, the SRV-port assumption (`_ubos._tcp` advertises the API port, 8080 in the sample), the mixed BOS+uBOS round-robin, and live unit conversions cannot be unit-tested — verify them in the testbed, per the spec.

- [ ] **Step 1: Run the widget in the testbed**

From `bmc-wasm-runtime/`, run: `just dev fleet-management`
(Requires a reachable uBOS device on the network advertising `_ubos._tcp`.)

- [ ] **Step 2: Confirm discovery and telemetry**

Verify in the testbed UI:
- the uBOS miner appears in the device list alongside any BOS miners
- its row shows hashrate ≈ `1.07 TH/s` for the sample reading (H/s ÷ 1e12), power in W (mW ÷ 1000), temperature in °C, and a sensible uptime
- the model name from `/api/info` `name` appears in the model column
- with wrong credentials (if testable), the uBOS row reads `N/A` and the device is not counted as reachable

- [ ] **Step 3: Record the result**

Note the testbed outcome in the BDK-506 task notes. No commit — this task is verification only.

---

## Self-review notes

- **Spec coverage:** discovery (Tasks 2, 4) · `/api/info` telemetry with H/s→TH/s and mW→W (Task 2) · proactive credential header coexisting with token auth (Tasks 1, 3) · family-generic dispatch incl. no-adapter handling (Task 3) · uBOS `parse_model` name capture (Task 2) · reset-then-populate stale clearing (Task 2 tests) · host unit tests for every pure surface (Tasks 1–3) · testbed verification (Task 5). Rendering and `manifest.json` are unchanged by design.
- **Prerequisite:** the BOS-miner-model machinery (`parse_model`, `ModelAccumulator`, `apply_model`, the model column) is already present in the tree, so no part of this plan reimplements it — Task 2 only adds uBOS's `parse_model` override and Task 3 routes it through the per-device adapter.
- **All code in this plan was compiled and verified** (host: 49 tests + clippy clean; wasm: clippy `-D warnings` clean) before the plan was written, then reverted. Hashrate is asserted with an epsilon to avoid `f32` bit-equality fragility.
- **Type consistency:** `adapter_for(DeviceFamily) -> Option<&'static dyn FamilyAdapter>`, `current_endpoint() -> Option<(DeviceId, DeviceFamily, String, u16)>`, and `credential_header(&self) -> Option<String>` are used identically everywhere they appear.
