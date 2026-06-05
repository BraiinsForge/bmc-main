# Parallel Telemetry Loading Design

## Context

BDK-506 builds a fleet-management WASM widget. Earlier slices added the widget skeleton, the generic device model, the discovery `FamilyAdapter` trait, mDNS discovery for BOS / uBOS / AxeOS-Bitaxe, and a fetch-driven telemetry driver (`2026-06-03-fleet-management-skeleton-design.md`, `2026-06-03-bos-telemetry-design.md`, `2026-06-04-ubos-support-design.md`, `2026-06-04-axeos-bitaxe-support-design.md`).

The current telemetry driver (`widgets-wasm/fleet-management/src/session.rs`) is a single global state machine. One `PassCursor` walks **every** discovered device, and for each device it fetches **every** telemetry endpoint, all strictly serial: device N+1 begins only when device N's last endpoint callback fires, and endpoint i+1 begins only when endpoint i's callback fires. The pass repeats on a single global 30 s timer.

Fetches are asynchronous on the host (`host_fetch` spawns a thread and returns immediately), so this never freezes the widget. But it serializes, and that serialization is bounded only by the host's global 10 s fetch timeout (`bmc-wasm-runtime/src/runtime/background/fetch.rs`). A single disconnected device therefore injects up to `10 s × endpoint-count` of latency into every pass before any later device is polled, and mDNS will not remove that device for up to ~75 min (the PTR-record TTL governs `ServiceRemoved`; a hard disconnect sends no goodbye). Several zombies blow the 30 s budget entirely and starve the refresh of healthy devices.

Self-eviction (dropping an unreachable device from our own list to stop polling it) was rejected: the `mdns-sd` daemon dedupes resolved instances, so it will not re-emit a `Found` event for a device it still holds cached. Evicting locally would make a device that had only a transient blip invisible to us until the daemon's cache expires (~75 min) and the device re-announces. The fix must instead reduce the cost of polling an unresponsive device, not stop polling it.

This slice parallelizes the telemetry loop along two axes: the three families run as independent drivers, and within a device all telemetry endpoints fire concurrently. Per-device counts are BOS = 3 (`/stats`, `/hashboards`, `/details`), uBOS = 1, Bitaxe = 1. Parallelizing devices *within* a family is explicitly out of scope for this slice (see Non-Goals).

## Goals

- A disconnected device stalls only its own family's cursor; the other two families keep refreshing on their own cadence.
- A disconnected device costs at most one fetch timeout per pass, not one-per-endpoint, because its endpoints fire together.
- No regression in correctness: each device still gets the same reading, model, and reachability it gets today.

## Non-Goals

- Parallel devices within a family. Devices stay serial per family; the family cursor advances one device at a time. The concurrency budget below leaves headroom for this as a later slice.
- Changing the 30 s pass interval, the host fetch timeout, or any mDNS behavior.
- Any change to the device list, rendering, or the `FamilyAdapter` trait surface beyond what routing requires.

## Architecture

### Per-family drivers

The single global `Driver` becomes three independent `FamilyDriver` instances, one per `DeviceFamily`, held in a `thread_local`. Each is a smaller copy of today's state machine and owns:

- `family: DeviceFamily`
- `cursor: Option<PassCursor>` over **its own** family's devices
- pacing: `elapsed_ms`, `waiting_next_pass`
- the current device's accumulator: `reading`, `model`, per-endpoint outcomes, and the `reauthed` flag

`DeviceList` gains `ids_for_family(family) -> Vec<DeviceId>`, filtering on `identity.family`. Each driver's `start_pass` snapshots through it instead of the global `ids()`. `PassCursor` is unchanged.

The module-level entry points fan out across the three drivers:

- `on_frame(delta_ms)` ticks all three; each driver that is `waiting_next_pass` and has reached `PASS_INTERVAL_MS` starts its own pass.
- `ensure_running()` starts any idle family that has devices (an empty family snapshot leaves that driver idle).
- `clear_tokens()` / `remove_token(id)` operate on a single shared `TOKENS: HashMap<DeviceId, String>`, unchanged — tokens are keyed by device, not by family driver.

`session.rs` is already ~485 lines; the routing and phase logic below add to it. The existing `mod driver` block is extracted into its own file `session/driver.rs` (mechanical move, no behavior change) so each file stays focused.

### Response routing

The SDK fetch callback is a bare `fn(&FetchResponse)` with no captured state, so parallel responses are correlated through the request id. `FetchResponse` carries `request_id`, and `FetchRequest::send` returns the same `FetchRequestId`.

A single global routing map records every in-flight telemetry fetch:

```rust
struct InFlight {
    family: DeviceFamily,
    endpoint_idx: usize,
}
static ROUTES: RefCell<HashMap<FetchRequestId, InFlight>>;
```

When a driver fires an endpoint it inserts the returned id into `ROUTES`. The one shared telemetry callback reads `response.request_id`, removes the entry to recover `(family, endpoint_idx)`, dispatches into that `FamilyDriver`, and decrements its `pending` counter. WASM is single-threaded and the host delivers fetch responses one at a time, so the `RefCell` folds are race-free.

### Per-device flow

For the family driver's current device:

1. **Phase 0 — ensure token (auth families only).** If the family has an `auth_endpoint` and no cached token, issue one login and await it. Success stores the token and proceeds to Phase 1; failure marks the device unreachable (every endpoint N/A) and advances the cursor. uBOS attaches its proactive Basic header and skips Phase 0; Bitaxe attaches nothing and skips it.
2. **Phase 1 — parallel burst.** Fire all telemetry endpoints concurrently, each with the credential header or cached token attached, registering each id in `ROUTES`. Set `pending` to the endpoint count.
3. **Barrier.** Each response folds into the accumulator (parsed on success, reset on failure) and decrements `pending`. When `pending` reaches 0, evaluate Phase 2.
4. **Phase 2 — one re-auth round (optional, auth families).** If any endpoint failed with an auth error and the device has not re-authed this pass, set `reauthed`, drop the token, issue one login, and on success re-fire **only** the auth-failed endpoints (a second gathered burst with its own barrier). A failed login leaves those endpoints N/A.
5. **Finalize.** `apply_telemetry` with `pass_reachable` over the gathered per-endpoint outcomes, `apply_model` if a model was parsed, then advance the cursor to the next device, or re-arm the family's 30 s timer when the cursor is done.

The `reauthed` flag preserves today's guarantee of at most one login per device per pass, eliminating any 401 → login → 401 loop. Re-auth is a discrete second round at the barrier, never interleaved per callback.

### Pacing

Each `FamilyDriver` keeps its own pacing clock at the existing `PASS_INTERVAL_MS = 30_000`. When a family's pass completes, that driver alone re-arms `request_frame_after` for the remainder of its 30 s window. A slow BOS pass cannot stretch the uBOS or Bitaxe cadence. The host wakes on the earliest pending timer, so multiple drivers arming independently is fine.

### Concurrency budget

Devices are serial within a family, so at most one device per family is in flight at once. Peak concurrent fetches are BOS 3 + uBOS 1 + Bitaxe 1 = 5 (a BOS login is a lone fetch during Phase 0, before its burst). Against the host's `max_fetches = 16` this is comfortable, and it documents the ceiling for a future devices-in-parallel slice, which would multiply the per-family figure.

## Error Handling

- **Unresponsive device:** each of its endpoints times out at the host's 10 s global limit, but concurrently, so the family cursor stalls ~10 s once (not per endpoint) before advancing. The device is stamped unreachable and remains in the list for mDNS to remove.
- **`send` rejected by host limits:** treated as an immediate endpoint failure (matching today), folded as N/A; the device can still be reachable via its other endpoints.
- **Login failure (Phase 0 or Phase 2):** the affected endpoints are N/A for the pass; `pass_reachable` then reports the device unreachable if no endpoint succeeded.
- **Device removed mid-pass:** routing entries whose device has since been removed resolve to no current device for that family and are dropped without folding, exactly as the cursor snapshot already tolerates.

## Testing

- **Pure, host-unit-testable:**
  - `DeviceList::ids_for_family` filters to the requested family (add to `device.rs`).
  - The Phase-2 decision is factored into a pure function — given the gathered per-endpoint outcomes and the `reauthed` flag, it returns finalize vs. re-auth-and-refire(set-of-endpoints) — and tested directly, the way `pass_reachable` already is. This keeps the branching out of the wasm-only callback and under test, and locks in the one-login-per-pass guarantee.
- **Call-path (runtime integration):** the host `fetch_interceptor` (`bmc-wasm-runtime/src/host_api.rs`) scripts canned responses so a test drives the real driver end to end and asserts:
  - a cold BOS device performs login before its three-endpoint burst;
  - an expired token triggers exactly one re-auth round, re-firing only the auth-failed endpoints;
  - a dead BOS device stalls the BOS cursor while uBOS and Bitaxe passes still complete.

  The existing fleet test harness shape is confirmed before the plan and matched rather than re-invented.

## Files Touched

- `widgets-wasm/fleet-management/src/session.rs` — fan-out entry points (`on_frame`, `ensure_running`), shared `TOKENS`, routing map, shared telemetry callback.
- `widgets-wasm/fleet-management/src/session/driver.rs` — new file: the `FamilyDriver` state machine (extracted from the current `mod driver`, then reworked for phases and the parallel burst).
- `widgets-wasm/fleet-management/src/device.rs` — `ids_for_family`.
