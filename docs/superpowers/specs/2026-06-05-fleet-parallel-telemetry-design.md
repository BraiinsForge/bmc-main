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
- No regression in the steady state: a device with a valid (or no) token gets the same reading, model, and reachability it gets today. The cold-token path for auth families changes deliberately (see Architecture → Per-device flow): the initial unauthenticated 401 round-trips are removed in favor of logging in first. The end-state telemetry is unchanged; only the request sequence differs.

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
- the current device's accumulator: `reading`, `model`, per-endpoint outcomes, the `reauthed` flag, the outstanding `pending` count, and the device `generation` (see Response routing)

`DeviceList` gains `ids_for_family(family) -> Vec<DeviceId>`, filtering on `identity.family`. Each driver's `start_pass` snapshots through it instead of the global `ids()`. `PassCursor` is unchanged.

The module-level entry points fan out across the three drivers:

- `on_frame(delta_ms)` ticks all three; each driver that is `waiting_next_pass` and has reached `PASS_INTERVAL_MS` starts its own pass.
- `ensure_running()` starts any idle family that has devices (an empty family snapshot leaves that driver idle).
- `clear_tokens()` / `remove_token(id)` operate on a single shared `TOKENS: HashMap<DeviceId, String>`, unchanged — tokens are keyed by device, not by family driver.

`session.rs` is already ~485 lines; the routing and phase logic below add to it. The existing `mod driver` block is extracted into its own file `session/driver.rs` (mechanical move, no behavior change) so each file stays focused.

### Response routing

Every fetch callback — telemetry **and** login — is a bare `fn(&FetchResponse)` with no captured state, so all responses are correlated through the request id. `FetchResponse` carries `request_id`, and `FetchRequest::send` returns the same `FetchRequestId`.

A single global routing map records every in-flight fetch, login or telemetry:

```rust
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
static ROUTES: RefCell<HashMap<FetchRequestId, InFlight>>;
```

When a driver fires a fetch it inserts the returned id into `ROUTES`. The one shared callback reads `response.request_id`, removes the entry, and dispatches by `kind` into the named `FamilyDriver`: a `Login` response runs the login handler (store token → fire the burst, or fail the device); a `Telemetry` response folds into the accumulator and decrements `pending`.

`device` plus `generation` guard against stale responses. Each family stamps a monotonically increasing `generation` onto its current device when that device begins; every fetch it issues for that device carries the stamp. A response whose `(device, generation)` no longer matches the family's current device is **abandoned**: it does not touch the accumulator and does not decrement the current `pending` (which belongs to a different generation). This is what makes mid-pass removal safe — see Per-device flow.

WASM is single-threaded and the host delivers fetch responses one at a time, so the `RefCell` folds are race-free.

### Per-device flow

For the family driver's current device:

1. **Phase 0 — ensure token (auth families only).** If the family has an `auth_endpoint` and no cached token, issue one login and await it. Success stores the token and proceeds to Phase 1; failure marks the device unreachable (every endpoint N/A) and advances the cursor. uBOS attaches its proactive Basic header and skips Phase 0; Bitaxe attaches nothing and skips it.
2. **Phase 1 — parallel burst.** Fire all telemetry endpoints concurrently, each with the credential header or cached token attached, registering each id in `ROUTES`. Set `pending` to the endpoint count.
3. **Barrier.** Each response folds into the accumulator (parsed on success, reset on failure) and decrements `pending`. When `pending` reaches 0, evaluate Phase 2.
4. **Phase 2 — one re-auth round (optional, auth families).** If any endpoint failed with an auth error and the device has not re-authed this pass, set `reauthed`, drop the token, issue one login, and on success re-fire **only** the auth-failed endpoints (a second gathered burst with its own barrier). A failed login leaves those endpoints N/A.
5. **Finalize.** `apply_telemetry` with `pass_reachable` over the gathered per-endpoint outcomes, `apply_model` if a model was parsed, then advance the cursor to the next device, or end the pass when the cursor is done (handing back to the scheduler, below).

The `reauthed` flag preserves today's guarantee of at most one login per device per pass, eliminating any 401 → login → 401 loop. Re-auth is a discrete second round at the barrier, never interleaved per callback.

**Barrier completion and mid-pass removal.** The `pending == 0` barrier is only ever reached by current-generation responses; stale-generation responses are dropped by the router without decrementing it (see Response routing). A device removed mid-burst would therefore leave `pending` above zero forever — a stalled cursor. To prevent this, removal is an explicit abandon: when discovery removes a device (`remove_token` already fires on `MdnsEvent::Removed`), if that device is a family's current device, the driver bumps the family's `generation`, resets `pending` to 0, discards the partial accumulator, and advances the cursor. The orphaned in-flight responses then carry the old generation and are dropped harmlessly when they arrive. A response that simply finds no current device (cursor already past it) is likewise dropped. The cursor never waits on a response that can no longer arrive.

### Pacing

Each `FamilyDriver` keeps its own pacing clock at the existing `PASS_INTERVAL_MS = 30_000`: a family's next pass is due 30 s after its previous pass completed, independent of the other families.

The drivers must **not** each call `request_frame_after` directly. `host_request_frame_after` writes a single `widget_delay_ms` slot on the host — last caller wins, not the minimum (`bmc-wasm-runtime/src/runtime/imports/render.rs`). Three drivers arming independently would clobber each other, and a family that armed early could be starved by a later family's longer delay — defeating the independent-cadence goal.

Instead, a single module-level scheduler owns every `request_frame_after` call. After each `on_frame` tick (and after `ensure_running`), it asks all three drivers for their next-wake — the remaining ms until each family's pass is due, or 0 for a family with an active pass that still needs prompt frames — takes the minimum, and issues exactly one `request_frame_after(min)`. Because there is a single writer recomputing the minimum every frame, last-write-wins is correct. Individual drivers may still call `request_frame()` (immediate) to re-render on a telemetry update; that drives rendering, not pass scheduling. A slow BOS pass cannot stretch the uBOS or Bitaxe cadence, because each family's next-wake is computed from its own completion time.

### Concurrency budget

Devices are serial within a family, so at most one device per family is in flight at once. Peak concurrent fetches are BOS 3 + uBOS 1 + Bitaxe 1 = 5 (a BOS login is a lone fetch during Phase 0, before its burst). Against the host's `max_fetches = 16` this is comfortable, and it documents the ceiling for a future devices-in-parallel slice, which would multiply the per-family figure.

## Error Handling

- **Unresponsive device:** each of its endpoints times out at the host's 10 s global limit, but concurrently, so the family cursor stalls ~10 s once (not per endpoint) before advancing. The device is stamped unreachable and remains in the list for mDNS to remove.
- **`send` rejected by host limits:** treated as an immediate endpoint failure (matching today), folded as N/A; the device can still be reachable via its other endpoints.
- **Login failure (Phase 0 or Phase 2):** the affected endpoints are N/A for the pass; `pass_reachable` then reports the device unreachable if no endpoint succeeded.
- **Device removed mid-pass:** removal explicitly abandons the in-flight device (bump `generation`, reset `pending`, advance the cursor), so the barrier never waits on responses that can no longer complete it; the orphaned responses are dropped by the router when they arrive. See Architecture → Per-device flow → Barrier completion and mid-pass removal.

## Testing

- **Pure, host-unit-testable:**
  - `DeviceList::ids_for_family` filters to the requested family (add to `device.rs`).
  - The Phase-2 decision is factored into a pure function — given the gathered per-endpoint outcomes and the `reauthed` flag, it returns finalize vs. re-auth-and-refire(set-of-endpoints) — and tested directly, the way `pass_reachable` already is. This keeps the branching out of the wasm-only callback and under test, and locks in the one-login-per-pass guarantee.
- **Call-path (runtime integration):** the `fetch_interceptor` returns its response immediately, so it can verify request *sequencing* but cannot model a fetch that occupies the 10 s timeout. The stall-isolation case needs virtual time, which the `UnifiedFixture` timeline provides: `Fetch` events carry an `at_ms` virtual timestamp (`bmc-wasm-runtime/src/unified_fixture.rs`), and the runtime advances a monotonic clock. The cases:
  - **Sequencing (interceptor or fixture):** a cold BOS device performs login before its three-endpoint burst; an expired token triggers exactly one re-auth round, re-firing only the auth-failed endpoints.
  - **Stall isolation (fixture, virtual time):** schedule the BOS endpoints' responses with `status: 0` (the host's network-error/timeout result) at `at_ms` ≈ 10 000, and the uBOS/Bitaxe responses at `at_ms` ≈ 50; a `capture` event placed between those times shows uBOS and Bitaxe populated and refreshed while the BOS device is still outstanding — proving the BOS stall does not block the other families. This needs no real sleeps.

  The existing fleet fixture shape is confirmed before the plan and matched rather than re-invented.

## Files Touched

- `widgets-wasm/fleet-management/src/session.rs` — fan-out entry points (`on_frame`, `ensure_running`), the module-level frame scheduler, shared `TOKENS`, the routing map, and the shared login+telemetry callback. `remove_token` gains the mid-pass abandon (bump `generation`, reset `pending`, advance) when it removes a family's current device.
- `widgets-wasm/fleet-management/src/session/driver.rs` — new file: the `FamilyDriver` state machine (extracted from the current `mod driver`, then reworked for phases and the parallel burst).
- `widgets-wasm/fleet-management/src/device.rs` — `ids_for_family`.
- `widgets-wasm/fleet-management/src/adapter.rs` — the `credential_header` doc comment describing BOS as "reactive token auth" is updated to reflect login-first, with reactive re-auth kept only as the expired-token fallback. No trait-surface change.
