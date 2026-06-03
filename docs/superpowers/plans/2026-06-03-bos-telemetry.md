# BOS Telemetry Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fetch per-device telemetry (current hashrate, power, temperature, uptime) from each discovered BOS miner via a
single round-robin cursor, and show live readings (or `N/A`) per device.

**Architecture:** Pure, host-tested modules grow: `JsonLookup` gains `f64`; the `FamilyAdapter` trait gains
login/telemetry methods that `BosAdapter` implements with reset-then-populate parsing; `session.rs` holds a pure
round-robin cursor + reachability decision. A wasm32-only driver in `session.rs` polls one device fully at a time (login
→ 3 GETs), folds readings into the device, updates a reachability flag, and paces passes ~30 s apart using accumulated
`render(delta_ms)`. A shared `miner_password` param feeds login; `on_params_update` clears tokens and credential-derived
readings.

**Tech Stack:** Rust 2024, `bmc-wasm-sdk` (`FetchRequest`, `JsonDoc`, `format_number!`, params, `request_frame_after`),
the WASM testbed for live verification.

**Spec:** `docs/superpowers/specs/2026-06-03-bos-telemetry-design.md`

**Conventions (read before starting):**

- Existing fleet-management code is from the discovery slice. Pure modules (`device`, `model`, `telemetry`, `discovery`,
  `adapter`, `families/bos`, the pure part of `session`) stay host-testable — no host imports. Gate host-import code
  (`FetchRequest`, `JsonDoc`, `request_frame*`, `widget_size`, `render_ui`, `format_number!`, `mdns::*`, `log_warn!`,
  params) behind `#[cfg(target_arch = "wasm32")]`.
- `JsonDoc` is a host import; parse through the crate-local `JsonLookup` trait (tested via `MapJson`), mirroring
  `widgets-wasm/mining-info/src/miner_api.rs`.
- Build strings with the SDK `fmt!` macro, never `std::format!`.
- All cargo runs from `widgets-wasm/`, sandboxed: `nix develop -c cargo …`.
- **Run `nix fmt` (plain, from repo root) before every commit.** Then commit. Commit subjects: imperative, ≤72 chars,
  end-reference `#BDK-506`.
- We are on branch `fbo/BDK-506/fleet-management` (not master).
- **cdylib dead-code gate.** This crate is a `cdylib`: on the `wasm32` target the only dead-code reachability roots are
  the `#[no_mangle]` exports (`init`/`render`/`on_*`). A task that adds code earlier exports don't yet call will fail
  `clippy --target wasm32 -- -D warnings` with `dead_code`. To keep **every commit green on both targets**, when a
  task's wasm clippy run flags a not-yet-wired item, annotate that item
  `#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "wired into the driver in Task 6"))]` (host builds and
  `--tests` still exercise it, so the attribute is wasm-only and the lint stays meaningful on host). Task 6 (driver) and
  Task 7 (render) wire these items into the export graph; as each is wired, its now-**fulfilled** `expect` must be
  removed — clippy reports exactly which. This mirrors how the discovery slice handled the same crate property. Each
  task's verification below runs **both** `clippy --tests` (host) and `clippy --target wasm32-unknown-unknown` (wasm) so
  the commit passes the full gate.

---

## File Structure

- `src/discovery.rs` — `JsonLookup` gains `f64`; `JsonDoc` impl + `MapJson` mock updated.
- `src/adapter.rs` — `FamilyAdapter` gains login + telemetry methods.
- `src/families/bos.rs` — `BosAdapter` implements them; pure `parse_login`/`parse_telemetry`/`reset_telemetry` + host
  tests.
- `src/session.rs` (new) — pure `PassCursor` + `pass_reachable` (host-tested); wasm32-only round-robin driver +
  per-device token state.
- `src/device.rs` — `DeviceList` gains `iter`, `ids`, `apply_telemetry`, `clear_all_telemetry`.
- `src/render.rs` — list all devices with readings or `N/A`.
- `src/lib.rs` — `DEVICES` becomes `pub(crate)`; `mod session`/`mod manifest_params`; read `miner_password`; kick driver
  from `ingest`; `on_params_update`; feed `delta_ms` to the driver.
- `manifest.json` — add `miner_password` param; regenerate `src/manifest_params.rs`.

---

## Task 1: Add `f64` to `JsonLookup`

**Files:** Modify `widgets-wasm/fleet-management/src/discovery.rs`

- [ ] **Step 1: Extend the trait, the wasm impl, and the mock**

In `discovery.rs`, add an `f64` method to the trait:

```rust
pub trait JsonLookup {
    fn str(&self, path: &str) -> Option<String>;
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
}
```

Add it to the wasm32 `JsonDoc` impl (below the existing `i64`):

```rust
    fn f64(&self, path: &str) -> Option<f64> {
        self.f64(path)
    }
```

Extend `tests_support::MapJson` with a floats map and impl method:

```rust
    #[derive(Default)]
    pub(crate) struct MapJson {
        pub(crate) strings: BTreeMap<&'static str, &'static str>,
        pub(crate) ints: BTreeMap<&'static str, i64>,
        pub(crate) floats: BTreeMap<&'static str, f64>,
    }
```

```rust
        fn f64(&self, path: &str) -> Option<f64> {
            self.floats.get(path).copied()
        }
```

- [ ] **Step 2: Verify host tests + clippy**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management
nix develop -c cargo clippy -p fleet-management --tests -- -D warnings
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
```

Expected: existing 15 tests still pass; host clippy clean. For the wasm run, `JsonDoc::f64` is a trait-impl method and
`MapJson` is test-only, so nothing new is dead on wasm — it should be clean. If the wasm run does flag `f64` as unused,
apply the `#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "wired into the driver in Task 6"))]` pattern
from the conventions.

- [ ] **Step 3: Commit**

```bash
cd /home/fbw/doc/work/bmc-main_fbo-BDK-506-fleet-management && nix fmt
git add widgets-wasm/fleet-management/src/discovery.rs
git commit -F - <<'MSG'
fleet-management: Add f64 accessor to JsonLookup

- extend the JSON seam with floats for telemetry parsing
- update the JsonDoc impl and the MapJson test mock

- #BDK-506
MSG
```

---

## Task 2: BOS telemetry parsing on the adapter

**Files:** Modify `src/adapter.rs`, `src/families/bos.rs`

- [ ] **Step 1: Grow the `FamilyAdapter` trait**

In `adapter.rs`, add imports and the new methods. The trait stays object-safe.

```rust
use crate::device::{DeviceFamily, DeviceIdentity};
use crate::discovery::JsonLookup;
use crate::telemetry::TelemetryReading;
```

```rust
pub trait FamilyAdapter {
    fn browse_service_types(&self) -> &'static [&'static str];
    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice>;

    fn api_base_path(&self) -> &'static str;
    fn telemetry_endpoints(&self) -> &'static [&'static str];
    fn parse_telemetry(&self, endpoint: &str, json: &dyn JsonLookup, reading: &mut TelemetryReading);
    fn reset_telemetry(&self, endpoint: &str, reading: &mut TelemetryReading);

    // Authentication — default NONE. Auth families (BOS) override these;
    // no-auth families (Bitaxe) inherit the defaults and fetch unauthenticated.
    fn auth_endpoint(&self) -> Option<&'static str> {
        None
    }
    fn login_body(&self, _password: &str) -> String {
        String::new()
    }
    fn parse_login(&self, _json: &dyn JsonLookup) -> Option<String> {
        None
    }
    fn auth_header(&self, token: &str) -> String {
        bmc_wasm_sdk::fmt!("Authorization: {token}")
    }
    fn is_auth_error(&self, _status: u32) -> bool {
        false
    }
}
```

`adapter.rs` needs `use bmc_wasm_sdk::fmt;` is NOT required — `auth_header`'s default uses the fully-qualified
`bmc_wasm_sdk::fmt!`. `DeviceFamily` may now be unused in `adapter.rs` — if clippy flags it, drop it from the `use`.
(`fmt!` is host-available, so the default `auth_header` compiles on both targets.)

- [ ] **Step 2: Write the failing BOS telemetry tests**

In `families/bos.rs`, add to the existing `#[cfg(test)] mod tests` block. Add the import at the top of the test module:
`use crate::telemetry::TelemetryReading;`

```rust
    fn stats_json() -> MapJson {
        let mut j = MapJson::default();
        j.floats
            .insert("/miner_stats/real_hashrate/last_1m/gigahash_per_second", 122_480.0);
        j.floats
            .insert("/power_stats/approximated_consumption/watt", 3_250.0);
        j
    }

    #[test]
    fn parses_stats_into_hashrate_and_power() {
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/stats", &stats_json(), &mut r);
        assert_eq!(r.current_hashrate_ths, Some(122.48));
        assert_eq!(r.power_w, Some(3_250.0));
    }

    #[test]
    fn parses_hashboards_temperature_as_max_chip_across_boards() {
        let mut j = MapJson::default();
        j.floats
            .insert("/hashboards/0/highest_chip_temp/temperature/degree_c", 61.0);
        j.floats
            .insert("/hashboards/1/highest_chip_temp/temperature/degree_c", 67.5);
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/hw/hashboards", &j, &mut r);
        assert_eq!(r.temperature_c, Some(67.5));
    }

    #[test]
    fn parses_details_uptime() {
        let mut j = MapJson::default();
        j.ints.insert("/bosminer_uptime_s", 187_020);
        let mut r = TelemetryReading::default();
        BosAdapter.parse_telemetry("/miner/details", &j, &mut r);
        assert_eq!(r.uptime_s, Some(187_020));
    }

    #[test]
    fn parse_clears_owned_field_that_vanished_from_response() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(99.0),
            power_w: Some(10.0),
            ..TelemetryReading::default()
        };
        // Empty stats response: both owned fields must go back to None.
        BosAdapter.parse_telemetry("/miner/stats", &MapJson::default(), &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.power_w, None);
    }

    #[test]
    fn reset_clears_only_the_endpoints_own_fields() {
        let mut r = TelemetryReading {
            current_hashrate_ths: Some(50.0),
            power_w: Some(20.0),
            temperature_c: Some(60.0),
            uptime_s: Some(100),
            nominal_hashrate_ths: None,
        };
        BosAdapter.reset_telemetry("/miner/stats", &mut r);
        assert_eq!(r.current_hashrate_ths, None);
        assert_eq!(r.power_w, None);
        // Other endpoints' fields untouched.
        assert_eq!(r.temperature_c, Some(60.0));
        assert_eq!(r.uptime_s, Some(100));
    }

    #[test]
    fn parses_login_token() {
        let mut j = MapJson::default();
        j.strings.insert("/token", "abc123");
        assert_eq!(BosAdapter.parse_login(&j), Some("abc123".to_owned()));
    }

    #[test]
    fn login_without_token_is_none() {
        assert_eq!(BosAdapter.parse_login(&MapJson::default()), None);
    }

    #[test]
    fn flags_401_and_403_as_auth_errors() {
        assert!(BosAdapter.is_auth_error(401));
        assert!(BosAdapter.is_auth_error(403));
        assert!(!BosAdapter.is_auth_error(200));
        assert!(!BosAdapter.is_auth_error(500));
    }

    #[test]
    fn bos_advertises_a_login_endpoint() {
        assert_eq!(BosAdapter.auth_endpoint(), Some("/auth/login"));
    }
```

- [ ] **Step 3: Run the tests to confirm they fail**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management families::bos
```

Expected: FAIL to compile (methods not implemented yet).

- [ ] **Step 4: Implement the telemetry methods on `BosAdapter`**

In `families/bos.rs`, add imports and the endpoint paths near the top:

```rust
use crate::telemetry::TelemetryReading;

const EP_STATS: &str = "/miner/stats";
const EP_HASHBOARDS: &str = "/miner/hw/hashboards";
const EP_DETAILS: &str = "/miner/details";

pub const BOS_TELEMETRY_ENDPOINTS: &[&str] = &[EP_STATS, EP_HASHBOARDS, EP_DETAILS];

fn ths_from_ghs(ghs: f64) -> f32 {
    (ghs / 1_000.0) as f32
}
```

Add the methods inside `impl FamilyAdapter for BosAdapter`:

```rust
    fn api_base_path(&self) -> &'static str {
        "/api/v1"
    }

    fn telemetry_endpoints(&self) -> &'static [&'static str] {
        BOS_TELEMETRY_ENDPOINTS
    }

    fn auth_endpoint(&self) -> Option<&'static str> {
        Some("/auth/login")
    }

    fn login_body(&self, password: &str) -> String {
        bmc_wasm_sdk::fmt!(
            r#"{{"username":"root","password":"{}"}}"#,
            bmc_wasm_sdk::JsonStr(password)
        )
    }

    fn parse_login(&self, json: &dyn JsonLookup) -> Option<String> {
        json.str("/token")
    }

    fn is_auth_error(&self, status: u32) -> bool {
        status == 401 || status == 403
    }

    fn parse_telemetry(&self, endpoint: &str, json: &dyn JsonLookup, reading: &mut TelemetryReading) {
        self.reset_telemetry(endpoint, reading);
        match endpoint {
            EP_STATS => {
                if let Some(ghs) =
                    json.f64("/miner_stats/real_hashrate/last_1m/gigahash_per_second")
                {
                    reading.current_hashrate_ths = Some(ths_from_ghs(ghs));
                }
                if let Some(watt) = json.f64("/power_stats/approximated_consumption/watt") {
                    reading.power_w = Some(watt as f32);
                }
            }
            EP_HASHBOARDS => {
                let mut max: Option<f32> = None;
                let mut i = 0usize;
                loop {
                    let path =
                        bmc_wasm_sdk::fmt!("/hashboards/{}/highest_chip_temp/temperature/degree_c", i);
                    match json.f64(&path) {
                        Some(c) => {
                            let c = c as f32;
                            max = Some(max.map_or(c, |m| m.max(c)));
                            i += 1;
                        }
                        None => break,
                    }
                }
                reading.temperature_c = max;
            }
            EP_DETAILS => {
                if let Some(uptime) =
                    json.i64("/bosminer_uptime_s").and_then(|v| u64::try_from(v).ok())
                {
                    reading.uptime_s = Some(uptime);
                }
            }
            _ => {}
        }
    }

    fn reset_telemetry(&self, endpoint: &str, reading: &mut TelemetryReading) {
        match endpoint {
            EP_STATS => {
                reading.current_hashrate_ths = None;
                reading.power_w = None;
            }
            EP_HASHBOARDS => reading.temperature_c = None,
            EP_DETAILS => reading.uptime_s = None,
            _ => {}
        }
    }
```

- [ ] **Step 5: Run the tests to confirm they pass**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management
nix develop -c cargo clippy -p fleet-management --tests -- -D warnings
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
```

Expected: tests all pass (15 prior + 9 new: 3 endpoint parses, stale-clear, reset, login token, tokenless,
`is_auth_error`, `auth_endpoint`); host clippy clean. The **wasm** run will flag the new adapter telemetry/auth methods,
`BOS_TELEMETRY_ENDPOINTS`, the `EP_*` consts, and `ths_from_ghs` as `dead_code` (nothing calls them on wasm until the
Task 6 driver). Annotate each flagged item with
`#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "wired into the driver in Task 6"))]` per the
conventions, then re-run until the wasm clippy is clean.

- [ ] **Step 6: Commit**

```bash
cd /home/fbw/doc/work/bmc-main_fbo-BDK-506-fleet-management && nix fmt
git add widgets-wasm/fleet-management/src/adapter.rs widgets-wasm/fleet-management/src/families/bos.rs
git commit -F - <<'MSG'
fleet-management: Parse BOS login and telemetry

- grow FamilyAdapter with login and telemetry methods
- map BOS stats, hashboards, and details to the reading
- reset endpoint-owned fields before populating to avoid stale values
- cover parsing, stale-clearing, and token extraction with tests

- #BDK-506
MSG
```

---

## Task 3: Pure round-robin cursor and reachability

**Files:** Create `widgets-wasm/fleet-management/src/session.rs`; modify `src/lib.rs` (declare module)

- [ ] **Step 1: Create `session.rs` with the pure logic and tests**

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::device::DeviceId;

/// A one-pass cursor over a snapshot of device ids. Snapshotting at pass start
/// means devices added or removed mid-pass are only seen on the next pass.
pub struct PassCursor {
    ids: Vec<DeviceId>,
    index: usize,
}

impl PassCursor {
    #[must_use]
    pub fn new(ids: Vec<DeviceId>) -> Self {
        Self { ids, index: 0 }
    }

    #[must_use]
    pub fn current(&self) -> Option<&DeviceId> {
        self.ids.get(self.index)
    }

    pub fn advance(&mut self) {
        self.index += 1;
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.index >= self.ids.len()
    }
}

/// A device is reachable when its latest pass obtained usable telemetry from
/// at least one endpoint. Login failure leaves every endpoint failed, so this
/// also captures "shared-password rejected -> unreachable".
#[must_use]
pub fn pass_reachable(endpoint_oks: &[bool]) -> bool {
    endpoint_oks.iter().any(|&ok| ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<DeviceId> {
        (0..n).map(|i| DeviceId::new(format!("d{i}._http._tcp.local."))).collect()
    }

    #[test]
    fn cursor_advances_then_completes() {
        let mut c = PassCursor::new(ids(2));
        assert_eq!(c.current().map(DeviceId::as_str), Some("d0._http._tcp.local."));
        assert!(!c.is_done());
        c.advance();
        assert_eq!(c.current().map(DeviceId::as_str), Some("d1._http._tcp.local."));
        c.advance();
        assert!(c.is_done());
        assert_eq!(c.current(), None);
    }

    #[test]
    fn empty_cursor_is_immediately_done() {
        let c = PassCursor::new(ids(0));
        assert!(c.is_done());
        assert_eq!(c.current(), None);
    }

    #[test]
    fn cursor_iterates_exactly_its_snapshot_in_order() {
        // The cursor owns its id snapshot, so it yields exactly the captured
        // ids in order regardless of later device-list changes. Re-snapshotting
        // for a fresh pass (and thus seeing mid-pass adds/removes only next
        // pass) is the driver's job, verified in the testbed.
        let mut c = PassCursor::new(ids(3));
        let mut seen = Vec::new();
        while let Some(id) = c.current() {
            seen.push(id.as_str().to_owned());
            c.advance();
        }
        assert_eq!(
            seen,
            vec![
                "d0._http._tcp.local.".to_owned(),
                "d1._http._tcp.local.".to_owned(),
                "d2._http._tcp.local.".to_owned(),
            ]
        );
        assert!(c.is_done());
    }

    #[test]
    fn reachable_only_when_an_endpoint_succeeded() {
        assert!(!pass_reachable(&[]));
        assert!(!pass_reachable(&[false, false]));
        assert!(pass_reachable(&[false, true, false]));
    }
}
```

- [ ] **Step 2: Declare the module in `lib.rs`**

In `lib.rs`, add `mod session;` to the ungated module declarations (with `mod adapter; mod device; …`).

- [ ] **Step 3: Run host tests + clippy**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management session
nix develop -c cargo clippy -p fleet-management --tests -- -D warnings
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
```

Expected: 4 new session tests pass; host clippy clean. The **wasm** run will flag `PassCursor` (and its methods) and
`pass_reachable` as `dead_code` — nothing calls them on wasm until the Task 6 driver. Annotate each with
`#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "wired into the driver in Task 6"))]` per the
conventions, then re-run until the wasm clippy is clean.

- [ ] **Step 4: Commit**

```bash
cd /home/fbw/doc/work/bmc-main_fbo-BDK-506-fleet-management && nix fmt
git add widgets-wasm/fleet-management/src/session.rs widgets-wasm/fleet-management/src/lib.rs
git commit -F - <<'MSG'
fleet-management: Add round-robin cursor and reachability

- add a pure pass cursor over a device-id snapshot
- add a pure reachability decision from per-endpoint outcomes
- cover both with host unit tests

- #BDK-506
MSG
```

---

## Task 4: DeviceList telemetry application

**Files:** Modify `widgets-wasm/fleet-management/src/device.rs`

- [ ] **Step 1: Write the failing tests**

Append to `device.rs`'s `#[cfg(test)] mod tests`. Add at the top of the test module:
`use crate::telemetry::TelemetryReading;`

```rust
    #[test]
    fn apply_telemetry_stamps_reading_and_reachability() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        let reading = TelemetryReading {
            power_w: Some(3_000.0),
            ..TelemetryReading::default()
        };
        list.apply_telemetry(&DeviceId::new("a._http._tcp.local."), reading, true);
        let dev = list.iter().next().expect("device present");
        assert!(dev.reachable);
        let snap = dev.telemetry.as_ref().expect("telemetry present");
        assert_eq!(snap.reading.power_w, Some(3_000.0));
    }

    #[test]
    fn apply_telemetry_can_mark_unreachable() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_telemetry(
            &DeviceId::new("a._http._tcp.local."),
            TelemetryReading::default(),
            false,
        );
        assert!(!list.iter().next().expect("present").reachable);
    }

    #[test]
    fn clear_all_telemetry_drops_readings() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.apply_telemetry(
            &DeviceId::new("a._http._tcp.local."),
            TelemetryReading::default(),
            true,
        );
        list.clear_all_telemetry();
        let dev = list.iter().next().expect("present");
        assert!(dev.telemetry.is_none());
        assert!(!dev.reachable);
    }

    #[test]
    fn ids_lists_every_device_in_order() {
        let mut list = DeviceList::new();
        list.upsert(identity("a._http._tcp.local.", "10.0.0.1"));
        list.upsert(identity("b._http._tcp.local.", "10.0.0.2"));
        let ids = list.ids();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].as_str(), "a._http._tcp.local.");
        assert_eq!(ids[1].as_str(), "b._http._tcp.local.");
    }
```

- [ ] **Step 2: Run to confirm failure**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management device
```

Expected: FAIL to compile (`iter`, `ids`, `apply_telemetry`, `clear_all_telemetry` missing).

- [ ] **Step 3: Implement the methods**

In `device.rs`, add `use crate::telemetry::TelemetrySnapshot;` to the existing telemetry import line (it already imports
`TelemetrySnapshot` — confirm; if only `TelemetryReading` was used, add `TelemetrySnapshot`). Then add inside
`impl DeviceList`:

```rust
    pub fn iter(&self) -> impl Iterator<Item = &KnownDevice> {
        self.devices.iter()
    }

    #[must_use]
    pub fn ids(&self) -> Vec<DeviceId> {
        self.devices.iter().map(|d| d.identity.id.clone()).collect()
    }

    /// Stamp the latest telemetry reading and reachability onto a device.
    pub fn apply_telemetry(&mut self, id: &DeviceId, reading: TelemetryReading, reachable: bool) {
        self.seq += 1;
        let seq = self.seq;
        if let Some(dev) = self.devices.iter_mut().find(|d| &d.identity.id == id) {
            dev.telemetry = Some(TelemetrySnapshot {
                reading,
                refreshed_seq: seq,
            });
            dev.reachable = reachable;
        }
    }

    /// Drop every device's telemetry and mark it unreachable (e.g. after a
    /// credential change). Devices stay listed; their readings go back to
    /// absent and reachability is recomputed on the next telemetry pass.
    pub fn clear_all_telemetry(&mut self) {
        for dev in &mut self.devices {
            dev.telemetry = None;
            dev.reachable = false;
        }
    }
```

Also add `use crate::telemetry::TelemetryReading;` to `device.rs`'s top-level imports if not already present (it imports
`TelemetrySnapshot` from the skeleton; add `TelemetryReading`).

- [ ] **Step 4: Run tests + clippy**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management
nix develop -c cargo clippy -p fleet-management --tests -- -D warnings
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
```

Expected: tests all pass; host clippy clean (the new methods are exercised by host tests). The **wasm** run will flag
`iter`/`ids`/`apply_telemetry`/`clear_all_telemetry` as `dead_code` — they are wired into the driver/render in Tasks
6–7. Annotate each with
`#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "wired into the driver in Task 6"))]` per the
conventions, then re-run until the wasm clippy is clean.

- [ ] **Step 5: Commit**

```bash
cd /home/fbw/doc/work/bmc-main_fbo-BDK-506-fleet-management && nix fmt
git add widgets-wasm/fleet-management/src/device.rs
git commit -F - <<'MSG'
fleet-management: Add telemetry application to DeviceList

- add iter and ids over all known devices
- apply a telemetry reading and reachability by device id
- clear all telemetry on demand for credential changes
- cover the new operations with host unit tests

- #BDK-506
MSG
```

---

## Task 5: Add the `miner_password` param

**Files:** Modify `manifest.json`; create `src/manifest_params.rs` (generated); modify `src/lib.rs`

- [ ] **Step 1: Add the param to the manifest**

In `widgets-wasm/fleet-management/manifest.json`, add a `params` object after `supported_viewports`:

```json
  "params": {
    "miner_password": {
      "name": "Miner password",
      "description": "Shared root password used to log into every BOS miner on the network",
      "type": "string",
      "default_value": "root"
    }
  }
```

- [ ] **Step 2: Generate `manifest_params.rs`**

Run from the repo root:

```
nix develop -c just wasm::gen fleet-management
```

Expected: writes `widgets-wasm/fleet-management/src/manifest_params.rs` exposing `Params { miner_password: String, .. }`
with `Params::current()`. (From `bmc-wasm-runtime/` the recipe is `just gen fleet-management`.)

- [ ] **Step 3: Declare the module**

In `lib.rs`, add `#[cfg(target_arch = "wasm32")] mod manifest_params;` (params reads are host calls, so the module is
wasm32-only, matching `mining-info`).

- [ ] **Step 4: Verify host tests + both clippy targets**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management
nix develop -c cargo clippy -p fleet-management --tests -- -D warnings
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
```

Expected: host tests + clippy clean. The generated `manifest_params` module declares the `Params` struct (and a
`current()`/`changed_keys()` API) that nothing calls until Task 6. If the wasm clippy flags any generated item as
`dead_code`, annotate the `#[cfg(target_arch = "wasm32")] mod manifest_params;` declaration in `lib.rs` with
`#[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "params read by the driver in Task 6"))]` (or annotate
the specific items if the generated file is hand-editable), then re-run until clean. Task 6 removes it once `Params` is
read.

- [ ] **Step 5: Commit**

```bash
cd /home/fbw/doc/work/bmc-main_fbo-BDK-506-fleet-management && nix fmt
git add widgets-wasm/fleet-management/manifest.json widgets-wasm/fleet-management/src/manifest_params.rs widgets-wasm/fleet-management/src/lib.rs
git commit -F - <<'MSG'
fleet-management: Add shared miner password param

- declare a miner_password param defaulting to root
- regenerate the manifest params module

- #BDK-506
MSG
```

---

## Task 6: Round-robin telemetry driver

**Files:** Modify `src/session.rs` (add wasm32 driver), `src/lib.rs` (wiring)

- [ ] **Step 1: Make `DEVICES` reachable from `session.rs`**

In `lib.rs`, change the `DEVICES` thread-local to `pub(crate)`:

```rust
#[cfg(target_arch = "wasm32")]
thread_local! {
    pub(crate) static DEVICES: RefCell<DeviceList> = RefCell::new(DeviceList::new());
}
```

- [ ] **Step 2: Add the wasm32 driver to `session.rs`**

Append to `session.rs`:

```rust
#[cfg(target_arch = "wasm32")]
mod driver {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use bmc_wasm_sdk::{FetchRequest, fmt, log_warn, request_frame, request_frame_after};

    use super::{PassCursor, pass_reachable};
    use crate::adapter::FamilyAdapter;
    use crate::device::DeviceId;
    use crate::families::bos::BosAdapter;
    use crate::manifest_params::Params;
    use crate::telemetry::TelemetryReading;

    const PASS_INTERVAL_MS: u32 = 30_000;

    struct Driver {
        cursor: Option<PassCursor>,
        endpoint_idx: usize,
        reading: TelemetryReading,
        endpoint_oks: Vec<bool>,
        // One re-auth attempt per device per pass guards against a 401 -> login
        // -> 401 loop when the shared password is wrong.
        reauthed: bool,
        elapsed_ms: u32,
        waiting_next_pass: bool,
    }

    impl Driver {
        const fn idle() -> Self {
            Self {
                cursor: None,
                endpoint_idx: 0,
                reading: TelemetryReading {
                    current_hashrate_ths: None,
                    nominal_hashrate_ths: None,
                    power_w: None,
                    uptime_s: None,
                    temperature_c: None,
                },
                endpoint_oks: Vec::new(),
                reauthed: false,
                elapsed_ms: 0,
                waiting_next_pass: false,
            }
        }
    }

    thread_local! {
        static DRIVER: RefCell<Driver> = const { RefCell::new(Driver::idle()) };
        static TOKENS: RefCell<HashMap<DeviceId, String>> = RefCell::new(HashMap::new());
    }

    fn base_url(host: &str, port: u16) -> String {
        fmt!("http://{}:{}{}", host, port, BosAdapter.api_base_path())
    }

    /// Look up the host/port for the cursor's current device.
    fn current_endpoint() -> Option<(DeviceId, String, u16)> {
        let id = DRIVER.with(|d| d.borrow().cursor.as_ref()?.current().cloned())?;
        crate::DEVICES.with(|devs| {
            let devs = devs.borrow();
            let dev = devs.iter().find(|d| d.identity.id == id)?;
            Some((id.clone(), dev.identity.host.clone(), dev.identity.port))
        })
    }

    /// Discovery found a device; start a pass if the driver is idle.
    pub fn ensure_running() {
        let should_start = DRIVER.with(|d| {
            let d = d.borrow();
            d.cursor.is_none() && !d.waiting_next_pass
        });
        if should_start {
            start_pass();
        }
    }

    /// Accumulate frame time and start the next pass once the interval elapses.
    pub fn on_frame(delta_ms: u32) {
        let start = DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            d.elapsed_ms = d.elapsed_ms.saturating_add(delta_ms);
            d.waiting_next_pass && d.elapsed_ms >= PASS_INTERVAL_MS
        });
        if start {
            start_pass();
        }
    }

    /// Clear all cached tokens (e.g. after a password change).
    pub fn clear_tokens() {
        TOKENS.with(|t| t.borrow_mut().clear());
    }

    /// Drop one device's cached session state when discovery removes it.
    pub fn remove_token(id: &DeviceId) {
        TOKENS.with(|t| t.borrow_mut().remove(id));
    }

    fn start_pass() {
        let ids = crate::DEVICES.with(|d| d.borrow().ids());
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            d.elapsed_ms = 0;
            d.waiting_next_pass = false;
            if ids.is_empty() {
                d.cursor = None;
            } else {
                d.cursor = Some(PassCursor::new(ids));
            }
        });
        // Wake near the 30s mark to pace the next pass even while idle.
        request_frame_after(PASS_INTERVAL_MS);
        begin_device();
    }

    fn begin_device() {
        let done = DRIVER.with(|d| d.borrow().cursor.as_ref().is_none_or(PassCursor::is_done));
        if done {
            // Re-arm the wake explicitly: request_frame() calls during the pass
            // may have superseded the pass-start timer, so guarantee a frame at
            // the remaining time to the 30s mark rather than relying on it.
            let remaining = DRIVER.with(|d| {
                let mut d = d.borrow_mut();
                d.cursor = None;
                d.waiting_next_pass = true;
                PASS_INTERVAL_MS.saturating_sub(d.elapsed_ms)
            });
            request_frame_after(remaining.max(1));
            return;
        }
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            d.endpoint_idx = 0;
            d.endpoint_oks.clear();
            d.reauthed = false;
            d.reading = Driver::idle().reading;
        });
        fetch_endpoint();
    }

    /// GET the current endpoint, attaching the auth header only if a token is
    /// already cached. No-auth families simply never have one cached.
    fn fetch_endpoint() {
        let endpoints = BosAdapter.telemetry_endpoints();
        let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
        if idx >= endpoints.len() {
            finalize_device();
            return;
        }
        let Some((id, host, port)) = current_endpoint() else {
            advance_device();
            return;
        };
        let url = fmt!("{}{}", base_url(&host, port), endpoints[idx]);
        let token = TOKENS.with(|t| t.borrow().get(&id).cloned());
        let header = token.map(|t| BosAdapter.auth_header(&t));
        if FetchRequest::get(&url)
            .headers_opt(header.as_deref())
            .send(on_endpoint)
            .is_none()
        {
            log_warn!("fleet: telemetry send rejected for {}", host);
            record_endpoint(false);
        }
    }

    /// React to a telemetry reply: re-authenticate on an adapter-reported auth
    /// error (once per device per pass), else fold or reset the endpoint.
    fn on_endpoint(response: &bmc_wasm_sdk::FetchResponse) {
        let endpoints = BosAdapter.telemetry_endpoints();
        let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
        let Some(ep) = endpoints.get(idx) else {
            advance_device();
            return;
        };

        let can_reauth = BosAdapter.auth_endpoint().is_some()
            && BosAdapter.is_auth_error(response.status)
            && !DRIVER.with(|d| d.borrow().reauthed);
        if can_reauth {
            DRIVER.with(|d| d.borrow_mut().reauthed = true);
            if let Some((id, _, _)) = current_endpoint() {
                TOKENS.with(|t| t.borrow_mut().remove(&id));
            }
            login_then_retry();
            return;
        }

        if response.ok() {
            let doc = response.json();
            DRIVER.with(|d| BosAdapter.parse_telemetry(ep, &doc, &mut d.borrow_mut().reading));
            record_endpoint(true);
        } else {
            DRIVER.with(|d| BosAdapter.reset_telemetry(ep, &mut d.borrow_mut().reading));
            record_endpoint(false);
        }
    }

    /// Log in with the shared password, then retry the SAME endpoint with the
    /// fresh token. A failed login leaves the endpoint as N/A and advances.
    fn login_then_retry() {
        let Some((_, host, port)) = current_endpoint() else {
            advance_device();
            return;
        };
        let Some(auth_path) = BosAdapter.auth_endpoint() else {
            record_endpoint(false);
            return;
        };
        let password = Params::current().miner_password;
        let url = fmt!("{}{}", base_url(&host, port), auth_path);
        let body = BosAdapter.login_body(&password);
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

    fn on_login(response: &bmc_wasm_sdk::FetchResponse) {
        let token = if response.ok() {
            BosAdapter.parse_login(&response.json())
        } else {
            None
        };
        match (current_endpoint(), token) {
            (Some((id, _, _)), Some(token)) => {
                TOKENS.with(|t| t.borrow_mut().insert(id, token));
                // Retry the same endpoint (endpoint_idx unchanged) with the token.
                fetch_endpoint();
            }
            // Login failed: this endpoint is N/A; move on.
            _ => {
                let endpoints = BosAdapter.telemetry_endpoints();
                let idx = DRIVER.with(|d| d.borrow().endpoint_idx);
                if let Some(ep) = endpoints.get(idx) {
                    DRIVER.with(|d| BosAdapter.reset_telemetry(ep, &mut d.borrow_mut().reading));
                }
                record_endpoint(false);
            }
        }
    }

    /// Record the current endpoint's outcome and move to the next one.
    fn record_endpoint(ok: bool) {
        DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            d.endpoint_oks.push(ok);
            d.endpoint_idx += 1;
        });
        fetch_endpoint();
    }

    fn finalize_device() {
        let id = DRIVER.with(|d| d.borrow().cursor.as_ref().and_then(|c| c.current().cloned()));
        if let Some(id) = id {
            let (reading, reachable) = DRIVER.with(|d| {
                let d = d.borrow();
                (d.reading, pass_reachable(&d.endpoint_oks))
            });
            crate::DEVICES.with(|devs| devs.borrow_mut().apply_telemetry(&id, reading, reachable));
            request_frame();
        }
        advance_device();
    }

    fn advance_device() {
        DRIVER.with(|d| {
            if let Some(cursor) = d.borrow_mut().cursor.as_mut() {
                cursor.advance();
            }
        });
        begin_device();
    }
}

#[cfg(target_arch = "wasm32")]
pub use driver::{clear_tokens, ensure_running, on_frame, remove_token};
```

Note: `TelemetryReading` derives `Copy` (skeleton), so `d.reading` reads by copy in `finalize_device`. `DeviceId`
derives `Clone`. `is_none_or` is stable. Every per-endpoint failure — including a rejected login and a send rejection —
funnels through `record_endpoint(false)`, which advances and ultimately reaches `finalize_device`; so a device whose
login is rejected (or that is fully unreachable) is always stamped with an all-`None` reading and
`reachable = pass_reachable(&[false, ..]) == false`. `finalize_device` is the single point that applies telemetry, so
there is no failure path that leaves a stale reading or a stale `reachable`.

- [ ] **Step 3: Wire the driver into `lib.rs`**

In `lib.rs`:

(a) In `ingest`, after the `upsert`, kick the driver:

```rust
#[cfg(target_arch = "wasm32")]
fn ingest(adapter: &dyn FamilyAdapter, json: &str) {
    let doc = JsonDoc::parse(json.as_bytes());
    if let Some(found) = adapter.parse_found(&doc) {
        DEVICES.with(|d| d.borrow_mut().upsert(found.identity));
        session::ensure_running();
        request_frame();
    }
}
```

(b) In `on_bos_event`, drop the device's cached session state when discovery removes it. The existing `Removed` arm
becomes:

```rust
        mdns::MdnsEvent::Removed(name) => {
            let id = DeviceId::new(*name);
            session::remove_token(&id);
            DEVICES.with(|d| d.borrow_mut().remove(&id));
            request_frame();
        }
```

(c) Replace `render` to feed frame time to the driver before building the tree:

```rust
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    session::on_frame(delta_ms);
    let WidgetSize { width, height, .. } = widget_size();
    let root = DEVICES.with(|d| render::view(&d.borrow(), width, height));
    let _ = render_ui(width, height, root);
}
```

(d) Add `on_params_update` (a credential change must not leave stale tokens or readings; `clear_all_telemetry` also
marks devices unreachable until the next pass re-confirms):

```rust
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let changed = manifest_params::Params::previous().map_or(true, |prev| {
        manifest_params::Params::current()
            .changed_keys(&prev)
            .contains(&"miner_password")
    });
    if changed {
        session::clear_tokens();
        DEVICES.with(|d| d.borrow_mut().clear_all_telemetry());
        request_frame();
    }
}
```

- [ ] **Step 4: Verify host tests and both clippy targets**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management
nix develop -c cargo clippy -p fleet-management --tests -- -D warnings
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
```

Expected: host tests pass (unchanged count — the driver is wasm-only); both clippy targets clean. The driver now
references `PassCursor`, `pass_reachable`, the adapter telemetry/auth methods, `manifest_params::Params`, and the
`DeviceList` methods, so **remove the `#[cfg_attr(target_arch = "wasm32", expect(dead_code, …))]` attributes added in
Tasks 1–5 for every item this driver now reaches** (`render`-only items — `family_label` is already used, the rest of
render lands in Task 7). A fulfilled `expect` is itself a clippy error, so clippy names exactly which attributes to
drop; iterate until both targets are clean. Anything still only reached by render (Task 7) keeps its attribute until
then.

- [ ] **Step 5: Commit**

```bash
cd /home/fbw/doc/work/bmc-main_fbo-BDK-506-fleet-management && nix fmt
git add widgets-wasm/fleet-management/src/session.rs widgets-wasm/fleet-management/src/lib.rs
git commit -F - <<'MSG'
fleet-management: Drive round-robin BOS telemetry polling

- poll one device fully at a time, logging in then fetching endpoints
- pace passes ~30s apart using accumulated frame deltas
- fold readings per endpoint and update per-device reachability
- clear tokens and readings when the miner password changes

- #BDK-506
MSG
```

---

## Task 7: Render the readings

**Files:** Modify `widgets-wasm/fleet-management/src/render.rs`

- [ ] **Step 1: Show readings per device, listing all discovered devices**

Replace the body of `view` in `render.rs`. It now iterates **all** devices (the interim list), showing readings or
`N/A`:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

#[expect(
    clippy::wildcard_imports,
    reason = "render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::device::DeviceList;
use crate::telemetry::TelemetryReading;

fn hashrate_cell(reading: Option<&TelemetryReading>) -> String {
    match reading.and_then(|r| r.current_hashrate_ths) {
        Some(v) => fmt!("{} TH/s", format_number!(f64::from(v), 2)),
        None => "N/A".to_owned(),
    }
}

fn power_cell(reading: Option<&TelemetryReading>) -> String {
    match reading.and_then(|r| r.power_w) {
        Some(v) => fmt!("{} W", format_number!(f64::from(v), 0)),
        None => "N/A".to_owned(),
    }
}

fn temp_cell(reading: Option<&TelemetryReading>) -> String {
    match reading.and_then(|r| r.temperature_c) {
        Some(v) => fmt!("{} °C", format_number!(f64::from(v), 1)),
        None => "N/A".to_owned(),
    }
}

fn uptime_cell(reading: Option<&TelemetryReading>) -> String {
    match reading.and_then(|r| r.uptime_s) {
        // Whole hours; the per-device interim view does not need finer detail.
        // try_from -> u32 keeps clippy's cast lints happy under -D warnings.
        Some(s) => {
            let hours = u32::try_from(s / 3_600).unwrap_or(u32::MAX);
            fmt!("{} h", format_number!(f64::from(hours), 0))
        }
        None => "N/A".to_owned(),
    }
}

#[must_use]
pub fn view(devices: &DeviceList, _width: u32, _height: u32) -> Node {
    if devices.is_empty() {
        return col(
            props!(background: BLACK),
            [center(
                props!(flex: 1.0),
                [text("Searching for miners\u{2026}", style!(size: 28, color: WHITE))],
            )],
        );
    }

    let mut children: Vec<Node> = vec![text(
        fmt!("{} miners", devices.len()),
        style!(size: 28, weight: FontWeight::BOLD, color: WHITE),
    )];

    for dev in devices.iter() {
        let reading = dev.telemetry.as_ref().map(|s| &s.reading);
        children.push(row(
            props!(gap: 12.0, cross_align: CrossAlign::Center),
            [
                text(dev.identity.name.clone(), style!(size: 20, color: WHITE, flex: 1.0)),
                text(hashrate_cell(reading), style!(size: 20, color: GRAY_40, align: TextAlign::Right)),
                text(power_cell(reading), style!(size: 20, color: GRAY_40, align: TextAlign::Right)),
                text(temp_cell(reading), style!(size: 20, color: GRAY_40, align: TextAlign::Right)),
                text(uptime_cell(reading), style!(size: 20, color: GRAY_40, align: TextAlign::Right)),
            ],
        ));
    }

    col(
        props!(background: BLACK, inset_top: 16.0, inset_left: 16.0, inset_right: 16.0, gap: 8.0),
        children,
    )
}
```

- [ ] **Step 2: Verify host tests and wasm clippy**

Run from `widgets-wasm/`:

```
nix develop -c cargo test -p fleet-management
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
```

Expected: host tests unchanged (render is wasm-only); wasm clippy clean. Render now consumes any remaining render-only
items (e.g. `iter`, the reading fields), so **remove their leftover
`#[cfg_attr(target_arch = "wasm32", expect(dead_code, …))]` attributes** — a fulfilled `expect` errors and names itself.
After this task no `expect(dead_code)` for telemetry items should remain.

- [ ] **Step 3: Format gates**

Run `nix fmt` (repo root, plain). Then from `widgets-wasm/`:

```
nix develop -c just validate-wasm-no-fmt
```

Expected: PASS (no allocating `std` formatting macros).

- [ ] **Step 4: Live verification (testbed)**

Run from the repo root: `nix develop -c just wasm::dev fleet-management` (or `just dev fleet-management` from
`bmc-wasm-runtime/`). With the shared password set in the Params panel and a reachable BOS miner on the LAN, confirm
each discovered device shows live hashrate / power / temperature / uptime, refreshing roughly every 30 s, and `N/A`
across those fields when a miner is unreachable or the password is wrong. (Headless agents cannot run the GUI testbed —
the human performs this step.)

- [ ] **Step 5: Commit**

```bash
cd /home/fbw/doc/work/bmc-main_fbo-BDK-506-fleet-management && nix fmt
git add widgets-wasm/fleet-management/src/render.rs
git commit -F - <<'MSG'
fleet-management: Show per-device telemetry readings

- list every discovered device with hashrate, power, temperature, uptime
- format numbers through the localized host formatter
- show N/A for unreachable miners or not-yet-loaded fields

- #BDK-506
MSG
```

---

## Self-Review Notes

- **Spec coverage:** family-generic optional auth + reactive on-demand re-auth via `is_auth_error` (Task 2 trait/BOS,
  Task 6 driver); shared password (Task 5 param); no backoff — one re-auth per device per pass, next pass retries (Task
  6); round-robin one-at-a-time + ~30 s between pass starts via `delta_ms`/`request_frame_after` (Task 6); per-endpoint
  reset-then-populate incl. `Some→None` (Task 2); login-failure/unreachable funnels through `finalize_device` →
  all-`None` reading + `reachable = false` (Task 6); reachability flag from pass outcome, interim list shows all (Tasks
  3, 4, 7); `on_params_update` clears tokens + readings + marks unreachable, and discovery removal drops the cached
  token via `remove_token` (Tasks 4, 6); field mapping incl. max-chip temp and GH/s→TH/s, nominal left `None` (Task 2);
  render hashrate/power/temp/uptime or `N/A` (Task 7); host tests for parsing, stale-clearing, token, `is_auth_error`,
  reachability, and the cursor's advance/completion/snapshot (Tasks 1–4). Nominal hashrate and okay/not-okay are
  deferred per spec — no task, intentionally.
- **Cursor coverage scope:** the pure `PassCursor` tests cover advance, pass-completion, and iterating exactly its
  captured snapshot. "Return to start" and re-snapshotting (so mid-pass adds/removes are seen only next pass) are
  *driver* behaviors (a fresh `PassCursor` per `start_pass`), verified in the testbed, not unit-tested — claimed
  accordingly, not as cursor unit tests.
- **Type consistency:** `parse_telemetry(&self, endpoint: &str, json: &dyn JsonLookup, reading: &mut TelemetryReading)`
  and `reset_telemetry(&self, endpoint, reading)` match across adapter and driver;
  `apply_telemetry(&DeviceId, TelemetryReading, bool)` matches between Task 4 and `finalize_device`;
  `pass_reachable(&[bool])`, `PassCursor::{new,current,advance,is_done}`,
  `session::{ensure_running,on_frame,clear_tokens}` are used as defined.
- **Every commit is green on both targets:** because this is a `cdylib`, Tasks 1–5 add code not yet reachable from the
  wasm exports, which `clippy --target wasm32 -- -D warnings` flags as `dead_code`. Each of those tasks runs **both**
  host and wasm clippy and annotates the not-yet-wired items with
  `#[cfg_attr(target_arch = "wasm32", expect(dead_code, …))]` (host `--tests` still exercises them). Tasks 6–7 wire the
  items into the export graph and remove the now- fulfilled attributes (a fulfilled `expect` is itself a clippy error).
  So every commit compiles and passes clippy on host and wasm — matching how the discovery slice handled the same crate
  property.
- **Test coverage honesty:** Tasks 5–7 add no host unit tests because their code (manifest param, wasm driver, render)
  is wasm-only and verified in the testbed; all host-testable logic (parsing, login-token, `is_auth_error`, cursor,
  reachability, `DeviceList` ops) is covered in Tasks 1–4.
