# Parallel Telemetry Loading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single serial telemetry driver in the fleet-management widget with three independent per-family drivers that fire each device's endpoints in parallel, so one disconnected device can no longer stall the whole load loop.

**Architecture:** Three `FamilyDriver` instances (BOS / uBOS / Bitaxe), each walking only its own family's devices serially but firing that device's telemetry endpoints concurrently. Parallel responses route back by `request_id` through a global map; a `(device, generation)` stamp makes mid-pass removal safe. BOS auth is login-first with one optional re-auth round. A single module-level scheduler owns frame timing so the families pace independently. The branching logic (reachability, re-auth decision, scheduler next-wake) is factored into pure module-level functions that compile and test on the host; the fetch-issuing state machine is wasm-only.

**Tech Stack:** Rust, `wasm32-unknown-unknown`, `bmc-wasm-sdk` (`FetchRequest`, `FetchRequestId`, `request_frame_after`), the fleet-management widget crate.

**Spec:** `docs/superpowers/specs/2026-06-05-fleet-parallel-telemetry-design.md`

---

## Commit structure and why it is only three commits

The widget is a **cdylib** (`crate-type = ["cdylib"]`) and `session`/`device` are **private** modules. In a cdylib, rustc's `dead_code` lint behaves like a binary's: any `pub` item not reachable from a `#[no_mangle]` export is flagged. The wasm gate runs `-D warnings`, so a helper introduced in one commit but only consumed by the driver in a later commit is **dead-code on the wasm build in between** and fails the gate.

That makes a "TDD each helper in its own commit" structure impossible to keep green: the helpers (`EndpointOutcome`, `reauth_decision`, `next_wake`, `ids_for_family`) are consumed only by the rewritten driver. They must land in the **same commit** as that driver. So the feature is three commits, each green on both gates:

1. **Extract** `mod driver` into `session/driver.rs` — mechanical, no logic change, both gates green.
2. **Rewrite** — `ids_for_family` + the pure helpers + their host tests + the driver rewrite, as one atomic commit. TDD happens *inside* this commit (write the pure-function tests, watch them fail on host, implement, then write the driver), but the commit boundary is the whole change because that is the smallest unit that compiles on wasm.
3. **Doc** — update the stale adapter comment.

### Verify before every commit (all three)

Run all of these, **serially** (never `cargo clippy` and `cargo test` in parallel — shared `target/` produces phantom errors):

- `nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings` — the wasm lint gate; this is the one that actually compiles the gated driver.
- `nix develop -c cargo test -p fleet-management` — host unit tests.
- `nix fmt` — formatting.

> **Shell commands:** shown plain (`nix`, `cargo`, `git`). The Claude Code RTK hook rewrites them to `rtk …` transparently at execution time, so prefixing them here would double-wrap. Do not hand-edit `rtk` into these commands.

---

## Task 1: Extract `mod driver` into `session/driver.rs` (mechanical)

A pure move so the rewrite in Task 2 happens in a focused file. No behavior change; both gates stay green because the driver logic is untouched.

**Files:**
- Create: `widgets-wasm/fleet-management/src/session/driver.rs`
- Modify: `widgets-wasm/fleet-management/src/session.rs`

- [ ] **Step 1: Move the module body to the new file**

Cut the entire current `#[cfg(target_arch = "wasm32")] mod driver { ... }` block body (everything **between** the `mod driver {` line and its matching closing `}`) out of `session.rs` and paste it into a new file `widgets-wasm/fleet-management/src/session/driver.rs`. Do **not** include the `mod driver {` wrapper or its closing brace in the new file. Prepend the standard copyright header line that every source file in this tree carries:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.
```

- [ ] **Step 2: Replace the inline module with a file declaration**

In `session.rs`, where the `mod driver { ... }` block was, leave exactly:

```rust
#[cfg(target_arch = "wasm32")]
mod driver;
```

Keep the existing re-export line unchanged:

```rust
#[cfg(target_arch = "wasm32")]
pub use driver::{clear_tokens, ensure_running, on_frame, remove_token};
```

The moved file's existing `use super::{PassCursor, adapter_for, pass_reachable};` line stays valid (it now refers to `session.rs`). Leave it as-is for this task — Task 2 rewrites these imports.

- [ ] **Step 3: Verify both gates (serially)**

```
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
nix develop -c cargo test -p fleet-management
nix fmt
```
Expected: clippy PASS (no new errors — in particular no "file not found for module `driver`" and no unresolved `super::` imports), host tests PASS, fmt clean. This is behavior-preserving, so it must be fully green.

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

## Task 2: Rewrite the driver for per-family parallel telemetry

The core change, committed atomically (see the commit-structure note). Order the work as TDD for the pure logic, then the wasm driver, then the full gate.

**Files:**
- Modify: `widgets-wasm/fleet-management/src/device.rs` (add `ids_for_family` + test)
- Modify: `widgets-wasm/fleet-management/src/session.rs` (add pure types/functions, refactor `pass_reachable`, update/add tests)
- Replace contents: `widgets-wasm/fleet-management/src/session/driver.rs`

### Part A — pure logic, host-tested (TDD)

- [ ] **Step 1: Write the failing host tests**

In `device.rs`, add to the existing `#[cfg(test)] mod tests` (ensure `use super::DeviceFamily;` is in scope):

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

In `session.rs`, replace the existing `reachable_only_when_an_endpoint_succeeded` test and add the new ones:

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
    assert_eq!(reauth_decision(&[Ok, Failed], false), ReauthDecision::Finalize);
}

#[test]
fn reauth_decision_refires_only_auth_failed_endpoints() {
    use EndpointOutcome::{AuthFailed, Ok};
    assert_eq!(
        reauth_decision(&[Ok, AuthFailed, AuthFailed], false),
        ReauthDecision::Reauth { endpoints: vec![1, 2] }
    );
}

#[test]
fn reauth_decision_finalizes_once_already_reauthed() {
    use EndpointOutcome::AuthFailed;
    assert_eq!(reauth_decision(&[AuthFailed], true), ReauthDecision::Finalize);
}

#[test]
fn next_wake_is_min_of_waiting_families() {
    use FamilyWake::{Active, Idle, Waiting};
    assert_eq!(next_wake(&[Waiting(30_000), Active, Waiting(12_000)]), Some(12_000));
    assert_eq!(next_wake(&[Idle, Waiting(5_000), Idle]), Some(5_000));
}

#[test]
fn next_wake_arms_nothing_when_no_family_is_waiting() {
    use FamilyWake::{Active, Idle};
    assert_eq!(next_wake(&[Active, Active, Idle]), None);
    assert_eq!(next_wake(&[]), None);
}
```

- [ ] **Step 2: Run host tests to verify they fail**

Run: `nix develop -c cargo test -p fleet-management`
Expected: FAIL to compile — `cannot find type EndpointOutcome` / `ReauthDecision` / `FamilyWake`, `no method named ids_for_family`.

- [ ] **Step 3: Implement `ids_for_family`**

In `device.rs`, add to `impl DeviceList` directly after `ids`:

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

If `DeviceFamily` does not already derive `PartialEq`/`Eq`, add them to its `#[derive(...)]` (it is a fieldless enum; the expected shape is `#[derive(Clone, Copy, PartialEq, Eq, Debug)]`).

- [ ] **Step 4: Implement the pure session helpers; refactor `pass_reachable`**

In `session.rs`, replace the existing `pass_reachable(endpoint_oks: &[bool])` definition (and its old doc comment) with the items below, added at module level next to the other pure helpers:

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

- [ ] **Step 5: Run host tests to verify the pure logic passes**

Run: `nix develop -c cargo test -p fleet-management`
Expected: PASS for the new tests and the pre-existing `PassCursor`/`adapter_for` tests. (The wasm build is still broken here — the old driver references the removed `pass_reachable(&[bool])` shape — which is why this is one commit: the driver is rewritten next, before committing.)

### Part B — the wasm driver

- [ ] **Step 6: Replace `driver.rs` with the new implementation**

Write `widgets-wasm/fleet-management/src/session/driver.rs` with exactly this content:

```rust
// Copyright (C) 2026  Braiins Systems s.r.o.

use std::cell::RefCell;
use std::collections::HashMap;

use bmc_wasm_sdk::{
    FetchRequest, FetchRequestId, fmt, log_warn, request_frame, request_frame_after,
};

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
            reading: TelemetryReading::default(),
            model: ModelAccumulator::default(),
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

/// Discovery found a device; start a pass for any idle family. A family with no
/// devices stays idle (see `start_pass`), so this never parks an empty family.
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
            if !d.waiting_next_pass {
                return false;
            }
            d.elapsed_ms = d.elapsed_ms.saturating_add(delta_ms);
            d.elapsed_ms >= PASS_INTERVAL_MS
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

/// Snapshot this family's devices and begin a pass. An empty snapshot leaves the
/// driver fully idle — it must not park as `waiting_next_pass`, or a device
/// discovered seconds later would not be polled until the 30s timer.
fn start_pass(family: DeviceFamily) {
    let ids = crate::DEVICES.with(|d| d.borrow().ids_for_family(family));
    if ids.is_empty() {
        with_driver(family, |d| {
            d.cursor = None;
            d.waiting_next_pass = false;
            d.elapsed_ms = 0;
        });
        return;
    }
    with_driver(family, |d| {
        d.elapsed_ms = 0;
        d.waiting_next_pass = false;
        d.cursor = Some(PassCursor::new(ids));
    });
    begin_device(family);
}

fn begin_device(family: DeviceFamily) {
    let done = with_driver(family, |d| d.cursor.as_ref().is_none_or(PassCursor::is_done));
    if done {
        with_driver(family, |d| {
            d.cursor = None;
            d.waiting_next_pass = true;
            d.elapsed_ms = 0;
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
                    InFlight { family, device: id.clone(), generation, kind: FetchKind::Login },
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
/// drops responses whose `(device, generation)` no longer matches the family's
/// current device (the pass was abandoned or has moved on).
fn on_fetch(response: &bmc_wasm_sdk::FetchResponse) {
    let Some(route) = ROUTES.with(|r| r.borrow_mut().remove(&response.request_id)) else {
        return;
    };
    let (current_generation, current_device) =
        with_driver(route.family, |d| (d.generation, d.current_device()));
    if route.generation != current_generation || current_device.as_ref() != Some(&route.device) {
        return;
    }
    match route.kind {
        FetchKind::Login => on_login(route.family, &route.device, response),
        FetchKind::Telemetry { endpoint_idx } => {
            on_telemetry(route.family, endpoint_idx, response);
        }
    }
    reschedule();
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

fn on_telemetry(family: DeviceFamily, endpoint_idx: usize, response: &bmc_wasm_sdk::FetchResponse) {
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

- [ ] **Step 7: Verify both gates (serially) and format**

```
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
nix develop -c cargo test -p fleet-management
nix fmt
```
Expected: clippy PASS (no warnings), host tests PASS, fmt clean. Things to check if clippy complains:
- `TelemetryReading::default()` and `ModelAccumulator::default()` must exist. The current code already calls both (`session.rs` uses `TelemetryReading::default()` and `ModelAccumulator::default()`), so they do — but if a `derive(Default)` is missing on either, add it rather than hand-writing a literal.
- `Params::current().miner_password` field access matches `manifest_params.rs`.
- No `clippy::wildcard_enum_match_arm` hits — the `match` arms over `DeviceFamily` and `FamilyWake` are exhaustive and explicit (no `_`).

- [ ] **Step 8: Commit**

```bash
git add widgets-wasm/fleet-management/src/device.rs widgets-wasm/fleet-management/src/session.rs widgets-wasm/fleet-management/src/session/driver.rs
git commit -F - <<'EOF'
fleet-management: Load telemetry per family in parallel #BDK-506

- split the driver into three independent per-family drivers
- fire each device's endpoints concurrently, routed by request id
- guard responses with a (device, generation) stamp for safe removal
- log in before the burst for auth families, re-auth once on 401
- schedule frames from the soonest pending pass across families
- leave a family with no devices idle so late discovery polls at once
EOF
```

---

## Task 3: Update the stale adapter doc comment

**Files:**
- Modify: `widgets-wasm/fleet-management/src/adapter.rs` (the `credential_header` doc comment, ~lines 45-50)

- [ ] **Step 1: Update the comment**

Replace the `credential_header` doc comment that says BOS uses "reactive token auth" with:

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

- [ ] **Step 2: Verify both gates (serially) and format**

```
nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings
nix develop -c cargo test -p fleet-management
nix fmt
```
Expected: both PASS, fmt clean (comment-only change).

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

## Testing scope

**Delivered by this plan (host unit tests, Task 2 Part A):**
- `ids_for_family` filters by family.
- `reauth_decision` — finalize vs. re-fire only the auth-failed endpoints, and the one-re-auth-per-pass guard.
- `pass_reachable` over `EndpointOutcome`.
- `next_wake` — the per-family isolation property: only `Waiting` families contribute a timer; `Active`/`Idle` contribute nothing, so a slow mid-pass family cannot stretch another's cadence. This is the deterministic proof of the isolation goal.

**Deliberately not in this plan (call-path / runtime integration):** a recording-`fetch_interceptor` test asserting "login precedes the burst" and "exactly one re-auth round" would need a runtime integration harness that loads the **built** fleet-management widget (today's `bmc-wasm-runtime/tests/*` use WAT probes, not real widgets) and drives mDNS + frames. That harness work is unverified and out of scope here, matching the spec's stance that the wall-clock stall test also needs new infrastructure. If you want call-path coverage, it should be scoped as its own task after confirming the runtime can load the widget with an interceptor — say so and I will investigate the harness and write it concretely.

---

## Final verification

- [ ] Run the full widget gate once more, serially:
  - `nix develop -c cargo clippy -p fleet-management --target wasm32-unknown-unknown -- -D warnings`
  - `nix develop -c cargo test -p fleet-management`
  - `nix fmt`
- [ ] Confirm `git log --oneline` shows the three focused commits (extract, rewrite, doc) and the working tree is clean for the files this plan owns.
