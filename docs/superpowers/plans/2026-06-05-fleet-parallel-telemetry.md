# Parallel Telemetry Loading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single serial telemetry driver in the fleet-management widget with three independent per-family drivers that fire each device's endpoints in parallel, so one disconnected device can no longer stall the whole load loop.

**Architecture:** Three `FamilyDriver` instances (BOS / uBOS / Bitaxe), each walking only its own family's devices serially but firing that device's telemetry endpoints concurrently. Parallel responses route back by `request_id` through a global map; a `generation` stamp makes mid-pass removal safe. BOS auth is login-first with one optional re-auth round. A single module-level scheduler owns frame timing so the families pace independently. The branching logic (reachability, re-auth decision, scheduler next-wake) is factored into pure module-level functions that compile and test on the host; the fetch-issuing state machine is wasm-only.

**Tech Stack:** Rust, `wasm32-unknown-unknown`, `bmc-wasm-sdk` (`FetchRequest`, `request_frame_after`), the fleet-management widget crate.

**Spec:** `docs/superpowers/specs/2026-06-05-fleet-parallel-telemetry-design.md`

---

## File Structure

- `widgets-wasm/fleet-management/src/device.rs` — add `DeviceList::ids_for_family`. Host-compiled, host-tested.
- `widgets-wasm/fleet-management/src/session.rs` — keep the host-compiled pure helpers (`PassCursor`, `adapter_for`); add `EndpointOutcome`, refactor `pass_reachable`, add `reauth_decision`/`ReauthDecision`, add `FamilyWake`/`next_wake`. Declare `mod driver;` and re-export its public entry points.
- `widgets-wasm/fleet-management/src/session/driver.rs` — **new file.** The wasm-only `FamilyDriver` state machine: per-family drivers, the routing map, the shared fetch callback, the per-device Phase 0/1/2 flow, the scheduler wiring, and the removal-abandon path. Moved out of the current `mod driver` block in `session.rs`, then reworked.
- `widgets-wasm/fleet-management/src/adapter.rs` — update the now-stale `credential_header` doc comment.

`lib.rs` is **not** modified: it already calls `session::on_frame`, `session::ensure_running`, `session::remove_token`, and `session::clear_tokens` at the right points.

### Two build configs both exercise the pure helpers

The commit gate runs exactly two builds, and the new pure functions are used in both — so no `dead_code` guards are needed:
- `cargo test -p fleet-management` (host + `#[cfg(test)]`): pure functions used by their unit tests.
- `cargo clippy -p fleet-management --target wasm32-unknown-unknown`: pure functions used by the wasm-only driver.

There is no host-without-test build in the gate, which is why `pass_reachable` is not flagged today and the new helpers will not be either.

### Verify commands (used throughout)

- Host tests: `nix develop -c cargo test -p fleet-management`
- Wasm lint gate (compiles the gated driver): `nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`

Run both before each commit that touches the widget. Per the repo rules, never run cargo clippy and cargo test in parallel (shared `target/`).

---

## Task 1: `DeviceList::ids_for_family`

**Files:**
- Modify: `widgets-wasm/fleet-management/src/device.rs` (add method near `ids`, ~line 169; add test in the existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `device.rs`:

```rust
#[test]
fn ids_for_family_filters_to_one_family() {
    let mut list = DeviceList::new();
    list.upsert(identity("bos._http._tcp.local.", "10.0.0.1"));
    let mut ubos = identity("ubos._ubos._tcp.local.", "10.0.0.2");
    ubos.family = DeviceFamily::Ubos;
    list.upsert(ubos);

    let bos_ids = list.ids_for_family(DeviceFamily::Bos);
    assert_eq!(bos_ids.len(), 1);
    assert_eq!(bos_ids[0].as_str(), "bos._http._tcp.local.");

    assert_eq!(list.ids_for_family(DeviceFamily::Ubos).len(), 1);
    assert!(list.ids_for_family(DeviceFamily::Bitaxe).is_empty());
}
```

If the existing `identity(..)` test helper does not let you set the family, set it on the returned struct as shown (`ubos.family = DeviceFamily::Ubos;`). Confirm `DeviceFamily` is imported in the test module; add `use super::DeviceFamily;` if needed.

- [ ] **Step 2: Run the test to verify it fails**

Run: `nix develop -c cargo test -p fleet-management ids_for_family_filters_to_one_family`
Expected: FAIL — `no method named ids_for_family`.

- [ ] **Step 3: Implement the method**

Add to `impl DeviceList` directly after `ids`:

```rust
#[must_use]
pub fn ids_for_family(&self, family: DeviceFamily) -> Vec<DeviceId> {
    self.devices
        .iter()
        .filter(|d| d.identity.family == family)
        .map(|d| d.identity.id.clone())
        .collect()
}
```

If `DeviceFamily` does not already derive `PartialEq`, add it to its `derive(..)` (it is a fieldless enum; `#[derive(Clone, Copy, PartialEq, Eq, Debug)]` is the expected shape — match the existing derives and add `PartialEq, Eq` if absent).

- [ ] **Step 4: Run the test to verify it passes**

Run: `nix develop -c cargo test -p fleet-management ids_for_family_filters_to_one_family`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add widgets-wasm/fleet-management/src/device.rs
git commit -F - <<'EOF'
fleet-management: Add per-family device id snapshot #BDK-506

- add DeviceList::ids_for_family for the per-family drivers
- filter the device list by identity.family
EOF
```

---

## Task 2: `EndpointOutcome`, `pass_reachable` refactor, `reauth_decision`

This replaces the boolean per-endpoint result with a three-state outcome so the re-auth decision is expressible, and adds the pure decision function the driver's barrier will call.

**Files:**
- Modify: `widgets-wasm/fleet-management/src/session.rs` (add types/functions at module level near `pass_reachable`, ~line 51; update the existing `reachable_only_when_an_endpoint_succeeded` test)

- [ ] **Step 1: Write the failing tests**

In `session.rs`, replace the existing `reachable_only_when_an_endpoint_succeeded` test and add the re-auth tests:

```rust
#[test]
fn reachable_only_when_an_endpoint_succeeded() {
    use EndpointOutcome::{AuthFailed, Failed, Ok};
    assert!(!pass_reachable(&[]));
    assert!(!pass_reachable(&[Failed, AuthFailed]));
    assert!(pass_reachable(&[Failed, Ok, Failed]));
}

#[test]
fn reauth_decision_finalizes_when_no_auth_failure() {
    use EndpointOutcome::{Failed, Ok};
    assert_eq!(
        reauth_decision(&[Ok, Failed], false),
        ReauthDecision::Finalize
    );
}

#[test]
fn reauth_decision_refires_only_auth_failed_endpoints() {
    use EndpointOutcome::{AuthFailed, Ok};
    assert_eq!(
        reauth_decision(&[Ok, AuthFailed, AuthFailed], false),
        ReauthDecision::Reauth {
            endpoints: vec![1, 2]
        }
    );
}

#[test]
fn reauth_decision_finalizes_once_already_reauthed() {
    use EndpointOutcome::AuthFailed;
    assert_eq!(
        reauth_decision(&[AuthFailed], true),
        ReauthDecision::Finalize
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop -c cargo test -p fleet-management reauth_decision`
Expected: FAIL — `cannot find type EndpointOutcome` / `reauth_decision` not found.

- [ ] **Step 3: Add the types and functions; update `pass_reachable`**

In `session.rs`, replace the existing `pass_reachable` definition (the `#[must_use] pub fn pass_reachable(endpoint_oks: &[bool]) -> bool { ... }`) with the outcome-based version, and add the new items just above it:

```rust
/// The result of fetching one telemetry endpoint in a pass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EndpointOutcome {
    Ok,
    Failed,
    AuthFailed,
}

/// What the barrier does once every endpoint of a device has reported.
#[derive(PartialEq, Eq, Debug)]
pub enum ReauthDecision {
    Finalize,
    Reauth { endpoints: Vec<usize> },
}

/// Decide the post-burst action: re-authenticate and re-fire the auth-failed
/// endpoints once per device per pass, otherwise finalize. The `reauthed` guard
/// prevents a 401 -> login -> 401 loop.
#[must_use]
pub fn reauth_decision(outcomes: &[EndpointOutcome], reauthed: bool) -> ReauthDecision {
    if reauthed {
        return ReauthDecision::Finalize;
    }
    let endpoints: Vec<usize> = outcomes
        .iter()
        .enumerate()
        .filter(|(_, o)| **o == EndpointOutcome::AuthFailed)
        .map(|(i, _)| i)
        .collect();
    if endpoints.is_empty() {
        ReauthDecision::Finalize
    } else {
        ReauthDecision::Reauth { endpoints }
    }
}

/// A device is reachable when at least one endpoint returned usable telemetry.
#[must_use]
pub fn pass_reachable(outcomes: &[EndpointOutcome]) -> bool {
    outcomes.iter().any(|o| *o == EndpointOutcome::Ok)
}
```

Delete the old doc comment block that described `pass_reachable` in terms of `endpoint_oks` booleans (it is replaced by the comment above).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop -c cargo test -p fleet-management reauth_decision pass_reachable reachable_only`
Expected: PASS for all. (The wasm driver still references the old `endpoint_oks: Vec<bool>`; that whole module is rewritten in Task 5, so do **not** run the wasm clippy gate yet — it will fail to compile until Task 5. This task is committed on host-test green only.)

- [ ] **Step 5: Commit**

```bash
git add widgets-wasm/fleet-management/src/session.rs
git commit -F - <<'EOF'
fleet-management: Add endpoint outcome and re-auth decision #BDK-506

- replace the boolean endpoint result with a three-state outcome
- add reauth_decision returning finalize or refire-auth-failed
- base pass_reachable on the new outcome type
EOF
```

---

## Task 3: `FamilyWake` and `next_wake` scheduler function

The pure function that encodes per-family pacing isolation: only between-pass (`Waiting`) families contribute a timer; mid-pass (`Active`) and `Idle` families contribute nothing, so a slow family cannot displace another's wake.

**Files:**
- Modify: `widgets-wasm/fleet-management/src/session.rs` (add at module level; add tests)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `session.rs`:

```rust
#[test]
fn next_wake_is_min_of_waiting_families() {
    use FamilyWake::{Active, Idle, Waiting};
    assert_eq!(
        next_wake(&[Waiting(30_000), Active, Waiting(12_000)]),
        Some(12_000)
    );
    assert_eq!(next_wake(&[Idle, Waiting(5_000), Idle]), Some(5_000));
}

#[test]
fn next_wake_arms_nothing_when_no_family_is_waiting() {
    use FamilyWake::{Active, Idle};
    assert_eq!(next_wake(&[Active, Active, Idle]), None);
    assert_eq!(next_wake(&[]), None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop -c cargo test -p fleet-management next_wake`
Expected: FAIL — `cannot find type FamilyWake` / `next_wake` not found.

- [ ] **Step 3: Implement the type and function**

Add to `session.rs` at module level (near the other pure helpers):

```rust
/// A family driver's contribution to frame scheduling.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FamilyWake {
    /// No devices and not counting down — contributes no timer.
    Idle,
    /// A pass is in progress; progress is driven by fetch delivery, not a
    /// timer. Contributes no timer (returning one would busy-render).
    Active,
    /// Between passes; the value is the remaining ms until the next pass.
    Waiting(u32),
}

/// The single next frame-after delay to arm: the soonest pending pass across
/// the families. `Active`/`Idle` families never contribute, so a slow mid-pass
/// family cannot stretch another family's cadence.
#[must_use]
pub fn next_wake(wakes: &[FamilyWake]) -> Option<u32> {
    wakes
        .iter()
        .filter_map(|w| match w {
            FamilyWake::Waiting(ms) => Some(*ms),
            FamilyWake::Idle | FamilyWake::Active => None,
        })
        .min()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop -c cargo test -p fleet-management next_wake`
Expected: PASS. (Wasm clippy still blocked until Task 5 — host-test green only.)

- [ ] **Step 5: Commit**

```bash
git add widgets-wasm/fleet-management/src/session.rs
git commit -F - <<'EOF'
fleet-management: Add per-family frame scheduler function #BDK-506

- add FamilyWake describing each driver's scheduling contribution
- add next_wake returning the soonest pending pass across families
- exclude active and idle families so a slow family is isolated
EOF
```

---

## Task 4: Extract `mod driver` into `session/driver.rs` (mechanical)

A pure move so the rewrite in Task 5 happens in a focused file. No behavior change.

**Files:**
- Create: `widgets-wasm/fleet-management/src/session/driver.rs`
- Modify: `widgets-wasm/fleet-management/src/session.rs`

- [ ] **Step 1: Move the module body to the new file**

Cut the entire current `#[cfg(target_arch = "wasm32")] mod driver { ... }` block body (everything **between** the `mod driver {` line and its closing `}`) out of `session.rs` and paste it into a new file `widgets-wasm/fleet-management/src/session/driver.rs`. Do not include the `mod driver {` wrapper or its closing brace in the new file.

- [ ] **Step 2: Replace the inline module with a file declaration**

In `session.rs`, where the `mod driver { ... }` block was, leave:

```rust
#[cfg(target_arch = "wasm32")]
mod driver;
```

Keep the existing re-export line unchanged:

```rust
#[cfg(target_arch = "wasm32")]
pub use driver::{clear_tokens, ensure_running, on_frame, remove_token};
```

In the moved `driver.rs`, the existing `use super::{PassCursor, adapter_for, pass_reachable};` line stays valid (it now refers to `session.rs`). Leave it as-is for this task — Task 5 updates the imports.

- [ ] **Step 3: Verify it still compiles unchanged**

Run: `nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`
Expected: the **same** result as before Task 2 — i.e. it now fails only on the `pass_reachable(&[bool])` signature mismatch introduced in Task 2 (the driver still passes `Vec<bool>`). That mismatch is expected and is resolved in Task 5. Confirm there are no *new* errors about the module move itself (no "file not found for module driver", no unresolved `super::` imports).

Because the wasm build cannot be green until Task 5, this task is a structural checkpoint. Commit it so the rewrite starts from a clean move.

- [ ] **Step 4: Commit**

```bash
git add widgets-wasm/fleet-management/src/session.rs widgets-wasm/fleet-management/src/session/driver.rs
git commit -F - <<'EOF'
fleet-management: Extract telemetry driver into its own module #BDK-506

- move the wasm-only driver block from session.rs to session/driver.rs
- declare it via `mod driver;` in the 2018 module style
EOF
```

---

## Task 5: Rewrite the driver for per-family parallel telemetry

The core change. The new `driver.rs` replaces the single global state machine with three `FamilyDriver`s, a routing map, the shared fetch callback, the Phase 0/1/2 per-device flow, the scheduler wiring, and the removal-abandon path.

This is a wasm-only module with no host unit tests, so it cannot be driven by TDD. Verification is: the wasm clippy gate compiles and lints clean, the host tests (Tasks 1–3) stay green, and the visual-regression smoke (`just wasm::verify fleet-management`, CI/GPU only) is unchanged. Implement the whole file, then verify, then commit once (it compiles as a unit).

**Files:**
- Replace contents: `widgets-wasm/fleet-management/src/session/driver.rs`

- [ ] **Step 1: Replace `driver.rs` with the new implementation**

Write `widgets-wasm/fleet-management/src/session/driver.rs` with exactly this content:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_protocol::FetchRequestId;
use bmc_wasm_sdk::{FetchRequest, fmt, log_warn, request_frame, request_frame_after};

use super::{
    EndpointOutcome, FamilyWake, PassCursor, ReauthDecision, adapter_for, next_wake,
    pass_reachable, reauth_decision,
};
use crate::adapter::FamilyAdapter;
use crate::device::{DeviceFamily, DeviceId};
use crate::manifest_params::Params;
use crate::model::ModelAccumulator;
use crate::telemetry::TelemetryReading;

const PASS_INTERVAL_MS: u32 = 30_000;
const FAMILIES: [DeviceFamily; 3] = [DeviceFamily::Bos, DeviceFamily::Ubos, DeviceFamily::Bitaxe];

enum FetchKind {
    Login,
    Telemetry { endpoint_idx: usize },
}

struct InFlight {
    family: DeviceFamily,
    device: DeviceId,
    generation: u64,
    kind: FetchKind,
}

struct FamilyDriver {
    cursor: Option<PassCursor>,
    elapsed_ms: u32,
    waiting_next_pass: bool,
    generation: u64,
    pending: usize,
    reading: TelemetryReading,
    model: ModelAccumulator,
    outcomes: Vec<Option<EndpointOutcome>>,
    reauthed: bool,
}

impl FamilyDriver {
    fn idle() -> Self {
        Self {
            cursor: None,
            elapsed_ms: 0,
            waiting_next_pass: false,
            generation: 0,
            pending: 0,
            reading: TelemetryReading {
                current_hashrate_ths: None,
                nominal_hashrate_ths: None,
                power_w: None,
                uptime_s: None,
                temperature_c: None,
            },
            model: ModelAccumulator {
                id: None,
                name: None,
                chip_type: None,
                chip_count: None,
            },
            outcomes: Vec::new(),
            reauthed: false,
        }
    }

    fn current_device(&self) -> Option<DeviceId> {
        self.cursor.as_ref()?.current().cloned()
    }

    fn wake(&self) -> FamilyWake {
        if self.cursor.is_some() {
            FamilyWake::Active
        } else if self.waiting_next_pass {
            FamilyWake::Waiting(PASS_INTERVAL_MS.saturating_sub(self.elapsed_ms))
        } else {
            FamilyWake::Idle
        }
    }
}

thread_local! {
    static DRIVERS: RefCell<[FamilyDriver; 3]> =
        RefCell::new([FamilyDriver::idle(), FamilyDriver::idle(), FamilyDriver::idle()]);
    static TOKENS: RefCell<HashMap<DeviceId, String>> = RefCell::new(HashMap::new());
    static ROUTES: RefCell<HashMap<FetchRequestId, InFlight>> = RefCell::new(HashMap::new());
}

const fn family_index(family: DeviceFamily) -> usize {
    match family {
        DeviceFamily::Bos => 0,
        DeviceFamily::Ubos => 1,
        DeviceFamily::Bitaxe => 2,
    }
}

fn with_driver<R>(family: DeviceFamily, f: impl FnOnce(&mut FamilyDriver) -> R) -> R {
    DRIVERS.with(|d| f(&mut d.borrow_mut()[family_index(family)]))
}

fn base_url(adapter: &dyn FamilyAdapter, host: &str, port: u16) -> String {
    fmt!("http://{}:{}{}", host, port, adapter.api_base_path())
}

fn resolve_identity(id: &DeviceId) -> Option<(DeviceFamily, String, u16)> {
    crate::DEVICES.with(|devs| {
        let devs = devs.borrow();
        let dev = devs.iter().find(|d| &d.identity.id == id)?;
        Some((dev.identity.family, dev.identity.host.clone(), dev.identity.port))
    })
}

/// Discovery found a device; start a pass for any idle family that has devices.
pub fn ensure_running() {
    for family in FAMILIES {
        let should_start = with_driver(family, |d| d.cursor.is_none() && !d.waiting_next_pass);
        if should_start {
            start_pass(family);
        }
    }
    reschedule();
}

/// Accumulate frame time per family and start each family's next pass when due.
pub fn on_frame(delta_ms: u32) {
    for family in FAMILIES {
        let due = with_driver(family, |d| {
            d.elapsed_ms = d.elapsed_ms.saturating_add(delta_ms);
            d.waiting_next_pass && d.elapsed_ms >= PASS_INTERVAL_MS
        });
        if due {
            start_pass(family);
        }
    }
    reschedule();
}

/// Clear all cached tokens (e.g. after a password change).
pub fn clear_tokens() {
    TOKENS.with(|t| t.borrow_mut().clear());
}

/// Drop one device's cached session state and, if it is a family's current
/// device, abandon the in-flight pass for it so the cursor never stalls.
pub fn remove_token(id: &DeviceId) {
    TOKENS.with(|t| t.borrow_mut().remove(id));
    for family in FAMILIES {
        let is_current = with_driver(family, |d| d.current_device().as_ref() == Some(id));
        if is_current {
            abandon_current(family);
        }
    }
    reschedule();
}

fn start_pass(family: DeviceFamily) {
    let ids = crate::DEVICES.with(|d| d.borrow().ids_for_family(family));
    with_driver(family, |d| {
        d.elapsed_ms = 0;
        d.waiting_next_pass = false;
        d.cursor = if ids.is_empty() {
            None
        } else {
            Some(PassCursor::new(ids))
        };
    });
    begin_device(family);
}

fn begin_device(family: DeviceFamily) {
    let done = with_driver(family, |d| d.cursor.as_ref().is_none_or(PassCursor::is_done));
    if done {
        with_driver(family, |d| {
            d.cursor = None;
            d.waiting_next_pass = true;
        });
        return;
    }
    let Some(id) = with_driver(family, |d| d.current_device()) else {
        advance_device(family);
        return;
    };
    let Some((dev_family, host, port)) = resolve_identity(&id) else {
        advance_device(family);
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        log_warn!("fleet: no adapter for discovered device family; marking unreachable");
        crate::DEVICES.with(|devs| {
            devs.borrow_mut()
                .apply_telemetry(&id, TelemetryReading::default(), false);
        });
        request_frame();
        advance_device(family);
        return;
    };
    let endpoint_count = adapter.telemetry_endpoints().len();
    with_driver(family, |d| {
        d.generation = d.generation.wrapping_add(1);
        d.pending = 0;
        d.reading = TelemetryReading::default();
        d.model = ModelAccumulator::default();
        d.outcomes = vec![None; endpoint_count];
        d.reauthed = false;
    });
    let needs_login =
        adapter.auth_endpoint().is_some() && TOKENS.with(|t| !t.borrow().contains_key(&id));
    if needs_login {
        issue_login(family, &id, &host, port, adapter);
    } else {
        fire_pending(family, &id, &host, port, adapter);
    }
}

/// Fire every still-pending (`None`) endpoint of the current device in parallel.
fn fire_pending(
    family: DeviceFamily,
    id: &DeviceId,
    host: &str,
    port: u16,
    adapter: &dyn FamilyAdapter,
) {
    let endpoints = adapter.telemetry_endpoints();
    let token = TOKENS.with(|t| t.borrow().get(id).cloned());
    let header = adapter
        .credential_header()
        .or_else(|| token.map(|t| adapter.auth_header(&t)));
    let generation = with_driver(family, |d| d.generation);
    let pending_idxs: Vec<usize> = with_driver(family, |d| {
        d.outcomes
            .iter()
            .enumerate()
            .filter(|(_, o)| o.is_none())
            .map(|(i, _)| i)
            .collect()
    });
    let mut sent = 0_usize;
    for idx in pending_idxs {
        let url = fmt!("{}{}", base_url(adapter, host, port), endpoints[idx]);
        let req_id = FetchRequest::get(&url)
            .headers_opt(header.as_deref())
            .send(on_fetch);
        match req_id {
            Some(req_id) => {
                ROUTES.with(|r| {
                    r.borrow_mut().insert(
                        req_id,
                        InFlight {
                            family,
                            device: id.clone(),
                            generation,
                            kind: FetchKind::Telemetry { endpoint_idx: idx },
                        },
                    );
                });
                sent += 1;
            }
            None => {
                log_warn!("fleet: telemetry send rejected for {}", host);
                with_driver(family, |d| d.outcomes[idx] = Some(EndpointOutcome::Failed));
            }
        }
    }
    with_driver(family, |d| d.pending = sent);
    if sent == 0 {
        barrier(family);
    }
}

fn issue_login(
    family: DeviceFamily,
    id: &DeviceId,
    host: &str,
    port: u16,
    adapter: &dyn FamilyAdapter,
) {
    let Some(auth_path) = adapter.auth_endpoint() else {
        finalize_failed(family, id);
        return;
    };
    let password = Params::current().miner_password;
    let url = fmt!("{}{}", base_url(adapter, host, port), auth_path);
    let body = adapter.login_body(&password);
    let generation = with_driver(family, |d| d.generation);
    let req_id = FetchRequest::post(&url)
        .headers("Content-Type: application/json")
        .body(body.as_bytes())
        .send(on_fetch);
    match req_id {
        Some(req_id) => {
            ROUTES.with(|r| {
                r.borrow_mut().insert(
                    req_id,
                    InFlight {
                        family,
                        device: id.clone(),
                        generation,
                        kind: FetchKind::Login,
                    },
                );
            });
        }
        None => {
            log_warn!("fleet: login send rejected for {}", host);
            finalize_failed(family, id);
        }
    }
}

/// Shared callback for every login and telemetry fetch. Routes by request id;
/// drops responses whose device generation no longer matches (abandoned).
fn on_fetch(response: &bmc_wasm_sdk::FetchResponse) {
    let Some(route) = ROUTES.with(|r| r.borrow_mut().remove(&response.request_id)) else {
        return;
    };
    let current_generation = with_driver(route.family, |d| d.generation);
    if route.generation != current_generation {
        return;
    }
    match route.kind {
        FetchKind::Login => on_login(route.family, &route.device, response),
        FetchKind::Telemetry { endpoint_idx } => {
            on_telemetry(route.family, endpoint_idx, response);
        }
    }
}

fn on_login(family: DeviceFamily, id: &DeviceId, response: &bmc_wasm_sdk::FetchResponse) {
    let Some((dev_family, host, port)) = resolve_identity(id) else {
        advance_device(family);
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        advance_device(family);
        return;
    };
    let token = if response.ok() {
        adapter.parse_login(&response.json())
    } else {
        None
    };
    match token {
        Some(token) => {
            TOKENS.with(|t| t.borrow_mut().insert(id.clone(), token));
            fire_pending(family, id, &host, port, adapter);
        }
        None => {
            with_driver(family, |d| {
                for outcome in &mut d.outcomes {
                    if outcome.is_none() {
                        *outcome = Some(EndpointOutcome::Failed);
                    }
                }
                d.pending = 0;
            });
            finalize_device(family, id);
        }
    }
}

fn on_telemetry(
    family: DeviceFamily,
    endpoint_idx: usize,
    response: &bmc_wasm_sdk::FetchResponse,
) {
    let Some(id) = with_driver(family, |d| d.current_device()) else {
        return;
    };
    let Some((dev_family, _, _)) = resolve_identity(&id) else {
        return;
    };
    let Some(adapter) = adapter_for(dev_family) else {
        return;
    };
    let endpoints = adapter.telemetry_endpoints();
    let Some(ep) = endpoints.get(endpoint_idx) else {
        return;
    };

    let outcome = if adapter.auth_endpoint().is_some() && adapter.is_auth_error(response.status) {
        EndpointOutcome::AuthFailed
    } else if response.ok() {
        let doc = response.json();
        with_driver(family, |d| {
            let d = &mut *d;
            adapter.parse_telemetry(ep, &doc, &mut d.reading);
            adapter.parse_model(ep, &doc, &mut d.model);
        });
        EndpointOutcome::Ok
    } else {
        with_driver(family, |d| adapter.reset_telemetry(ep, &mut d.reading));
        EndpointOutcome::Failed
    };

    let done = with_driver(family, |d| {
        d.outcomes[endpoint_idx] = Some(outcome);
        d.pending = d.pending.saturating_sub(1);
        d.pending == 0
    });
    if done {
        barrier(family);
    }
}

/// All endpoints reported: re-authenticate once if any auth-failed, else finalize.
fn barrier(family: DeviceFamily) {
    let Some(id) = with_driver(family, |d| d.current_device()) else {
        advance_device(family);
        return;
    };
    let (outcomes, reauthed) = with_driver(family, |d| {
        let outcomes: Vec<EndpointOutcome> = d
            .outcomes
            .iter()
            .map(|o| o.unwrap_or(EndpointOutcome::Failed))
            .collect();
        (outcomes, d.reauthed)
    });
    match reauth_decision(&outcomes, reauthed) {
        ReauthDecision::Reauth { endpoints } => {
            with_driver(family, |d| {
                d.reauthed = true;
                for idx in &endpoints {
                    d.outcomes[*idx] = None;
                }
            });
            TOKENS.with(|t| t.borrow_mut().remove(&id));
            let Some((dev_family, host, port)) = resolve_identity(&id) else {
                advance_device(family);
                return;
            };
            let Some(adapter) = adapter_for(dev_family) else {
                advance_device(family);
                return;
            };
            issue_login(family, &id, &host, port, adapter);
        }
        ReauthDecision::Finalize => finalize_device(family, &id),
    }
}

fn finalize_device(family: DeviceFamily, id: &DeviceId) {
    let (reading, reachable, model) = with_driver(family, |d| {
        let outcomes: Vec<EndpointOutcome> = d
            .outcomes
            .iter()
            .map(|o| o.unwrap_or(EndpointOutcome::Failed))
            .collect();
        (d.reading, pass_reachable(&outcomes), d.model.clone())
    });
    crate::DEVICES.with(|devs| {
        let mut devs = devs.borrow_mut();
        devs.apply_telemetry(id, reading, reachable);
        if let Some(model) = model.into_model() {
            devs.apply_model(id, model);
        }
    });
    request_frame();
    advance_device(family);
}

/// Mark every endpoint failed and finalize (used when login cannot be sent).
fn finalize_failed(family: DeviceFamily, id: &DeviceId) {
    with_driver(family, |d| {
        for outcome in &mut d.outcomes {
            *outcome = Some(EndpointOutcome::Failed);
        }
        d.pending = 0;
    });
    finalize_device(family, id);
}

/// Abandon the in-flight device (removed mid-pass): bump the generation so its
/// outstanding responses are dropped, then advance the cursor.
fn abandon_current(family: DeviceFamily) {
    with_driver(family, |d| {
        d.generation = d.generation.wrapping_add(1);
        d.pending = 0;
        d.outcomes.clear();
        d.reading = TelemetryReading::default();
        d.model = ModelAccumulator::default();
        d.reauthed = false;
        if let Some(cursor) = d.cursor.as_mut() {
            cursor.advance();
        }
    });
    begin_device(family);
}

fn advance_device(family: DeviceFamily) {
    with_driver(family, |d| {
        if let Some(cursor) = d.cursor.as_mut() {
            cursor.advance();
        }
    });
    begin_device(family);
}

fn reschedule() {
    let wakes = DRIVERS.with(|d| {
        let d = d.borrow();
        [d[0].wake(), d[1].wake(), d[2].wake()]
    });
    if let Some(delay_ms) = next_wake(&wakes) {
        request_frame_after(delay_ms);
    }
}
```

- [ ] **Step 2: Run the wasm clippy gate**

Run: `nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS, no warnings. Resolve any issues before moving on. Likely things to check:
- `ModelAccumulator` and `TelemetryReading` field names in `idle()` must match their definitions exactly (copy from `model.rs` / `telemetry.rs` if they differ).
- `Params::current().miner_password` access matches `manifest_params.rs`.
- If `FetchRequestId` is not re-exported from `bmc_wasm_protocol` at that path, find its actual path (it is the type of `FetchResponse::request_id`).

- [ ] **Step 3: Run the host tests**

Run: `nix develop -c cargo test -p fleet-management`
Expected: PASS — all Task 1–3 tests plus the pre-existing `PassCursor`/`adapter_for` tests. (Run this **after** the clippy gate, not in parallel.)

- [ ] **Step 4: Format**

Run: `nix fmt`
Expected: clean (no diff, or only formats the new file).

- [ ] **Step 5: Commit**

```bash
git add widgets-wasm/fleet-management/src/session.rs widgets-wasm/fleet-management/src/session/driver.rs
git commit -F - <<'EOF'
fleet-management: Load telemetry per family in parallel #BDK-506

- split the driver into three independent per-family drivers
- fire each device's endpoints concurrently, routed by request id
- guard responses with a per-device generation for safe removal
- log in before the burst for auth families, re-auth once on 401
- schedule frames from the soonest pending pass across families
EOF
```

---

## Task 6: Update the stale adapter doc comment

**Files:**
- Modify: `widgets-wasm/fleet-management/src/adapter.rs` (the `credential_header` doc comment, ~lines 45-50)

- [ ] **Step 1: Update the comment**

Replace the `credential_header` doc comment that currently says BOS uses "reactive token auth" with text describing login-first:

```rust
    /// A proactive credential header attached to every request, preferred over
    /// any cached token. Default none. Families with static credentials (uBOS)
    /// override it; families with fetched-token auth (BOS) leave it none and the
    /// driver logs in before the telemetry burst, re-authenticating once if a
    /// cached token has expired.
    fn credential_header(&self) -> Option<String> {
        None
    }
```

- [ ] **Step 2: Verify the gates still pass**

Run: `nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`
Then: `nix develop -c cargo test -p fleet-management`
Expected: both PASS (comment-only change).

- [ ] **Step 3: Commit**

```bash
git add widgets-wasm/fleet-management/src/adapter.rs
git commit -F - <<'EOF'
fleet-management: Describe BOS login-first auth in adapter doc #BDK-506

- replace the stale "reactive token auth" note on credential_header
- state the driver logs in before the burst and re-auths once on 401
EOF
```

---

## Final verification

- [ ] Run the full widget gate one more time, serially:
  - `nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`
  - `nix develop -c cargo test -p fleet-management`
  - `nix fmt`
- [ ] Confirm `git log --oneline` shows the six focused commits and the working tree is clean for the files this plan owns.
