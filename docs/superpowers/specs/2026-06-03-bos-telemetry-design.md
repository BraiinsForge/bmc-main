# BOS Telemetry Slice Design

## Context

BDK-506 builds a fleet-management WASM widget. The first slice (`2026-06-03-fleet-management-skeleton-design.md`) added
the widget, the generic device model, the discovery-only `FamilyAdapter` trait, BOS mDNS discovery, and an on-screen
device list. This second slice fetches per-device telemetry from BOS miners and shows live readings next to each
discovered device.

"Proper fleet management" decomposes into three further slices, built and verified in sequence: (1) **BOS telemetry** —
this slice; (2) aggregation — fleet and per-model rollups; (3) the final overview / per-model UI. Keeping them separate
keeps each shippable and reviewable.

## Goals

- fetch per-device telemetry from each discovered BOS miner: current hashrate, power, temperature, and uptime
- grow the `FamilyAdapter` trait with telemetry methods and an optional, family-owned auth scheme (default none), so an
  auth family (BOS) and a no-auth family (Bitaxe) both fit one generic driver — keeping all parsing pure and host-tested
- poll the fleet within the host's 16-fetch budget via a single round-robin cursor (one device at a time),
  authenticating reactively only when the adapter reports an auth error
- maintain a per-device reachability signal from each telemetry outcome, so the later aggregation slice counts only
  reachable devices rather than inheriting discovery-only presence
- show the live readings (or `N/A`) per device in the existing list
- keep the orchestration testable: pure parsing, pure reachability decision, and pure cursor logic on the host; only the
  fetch and timing wiring behind the wasm boundary

## Non-Goals

- nominal hashrate (deliberately skipped this slice — see Deferred)
- okay / not-okay classification (depends on nominal — see Deferred)
- fleet or per-model aggregation, and the online/okay counts
- the final overview / per-model breakdown UI
- uBOS or Bitaxe telemetry
- persistence

## Scope and Fleet-Size Assumption

Per-device telemetry only. For each BOS device the mDNS slice discovered, log in, fetch its readings, fold them into
that device's `TelemetryReading`, and render the values (or `N/A`). Discovery is unchanged — the persistent mDNS browse
from the skeleton slice still owns the device list.

This targets the story's **home / small-fleet** user — on the order of a few up to a few dozen miners. At that scale a
single one-device-at-a-time pass completes in a few seconds, well under the 30 s refresh target (see Cadence). The
design degrades gracefully past that scale rather than breaking, but ~30 s refresh is only guaranteed within the
small-fleet assumption.

## Authentication (family-generic)

Authentication is a per-family capability the adapter owns, not something the driver assumes — because families differ:
BOS+ stats endpoints require a login token, while Bitaxe/AxeOS needs none for the telemetry we read. The trait exposes
auth optionally, defaulting to **none**; a family that needs auth overrides it (see FamilyAdapter Growth). The driver is
auth-agnostic and reacts the same way for every family.

The flow is **reactive / on-demand** rather than login-first, which keeps it generic and also absorbs token expiry:

- telemetry endpoints are fetched with `adapter.auth_header(token)` attached only when a token is already cached (none
  on first contact)
- on each response the driver asks `adapter.is_auth_error(status)`; when that is true, the family has an auth scheme
  (`auth_endpoint()` is `Some`), and the device has not already re-authenticated this pass, the driver drops the cached
  token, logs in once, caches the new token, and **retries the same endpoint** with it
- a per-device "already re-authenticated this pass" guard prevents a 401 → login → 401 loop; if re-auth fails, that
  endpoint stays `N/A` and the next pass simply tries again (the ~30 s cadence is the natural retry interval — no
  backoff scheduler)

For **BOS** specifically: `auth_endpoint()` is `Some("/auth/login")`, `is_auth_error` is
`status == 401 || status == 403`, the whole fleet shares one password (`miner_password` param, default `"root"`), the
login body is `{"username":"root","password":"<pw>"}`, the token comes from the reply's `/token`, and it is sent as the
`Authorization: <token>` header. So a BOS miner's first contact does GET → 401 → login → retry → 200, caches the token,
and reuses it on later endpoints and passes until it expires. A **Bitaxe** miner (later) keeps the default
`is_auth_error == false`, never logs in, and is fetched unauthenticated.

`base` is built from the mDNS-resolved host and port plus the family's API base path — for BOS
`http://{host}:{port}/api/v1` (the BOS advertisement uses port 80).

## Polling Architecture

A single rotating cursor over the discovered devices drives all telemetry. The driver polls one miner fully, then
advances to the next, looping back to the first after the last. Because at most one device is being fetched at a time
and that device fetches its endpoints sequentially, in-flight fetches stay at roughly one — far under the 16-fetch host
ceiling, at any fleet size.

The driver is idle when there are no devices. Discovery adding the first device re-kicks it; a pass that finds the fleet
empty returns to idle.

### Cadence

The target is to **start each pass roughly every 30 s** — i.e. the gap is measured between successive pass *starts*, so
per-device refresh is `max(30 s, pass duration)`. For the assumed small fleet a pass is far shorter than 30 s, so each
device refreshes about every 30 s, meeting the story's "roughly every 30 seconds." If a fleet is large enough that a
pass exceeds 30 s, passes run effectively back-to-back (pass duration dominates) without ever overlapping or exceeding
the fetch budget.

Within a pass, devices run back-to-back with no artificial gap. The inter-pass timing uses no wall clock: elapsed time
since the pass start is accumulated from the `render(delta_ms)` frame deltas (the SDK's documented per-frame timer
mechanism), and `request_frame_after(30_000)` is issued at pass start to guarantee a wake near the 30 s mark. When ≥ 30
s has accumulated and the driver is idle (pass complete), the next pass starts and the accumulator resets; intervening
frames (e.g. from data folding calling `request_frame`) accumulate but do not trip the threshold early.

### Per-device cycle

For the device at the cursor, iterate its `adapter.telemetry_endpoints()` in order:

1. GET the endpoint, attaching `adapter.auth_header(token)` only if a token is cached
2. on the reply:
   - if `adapter.is_auth_error(status)` and the family can authenticate and we have not re-authenticated this device yet
     this pass → drop the token, POST the login request, cache the new token, and **retry the same endpoint** (do not
     advance)
   - else on a 2xx → `parse_telemetry(endpoint, json, reading)` folds the fields
   - else (other failure, or auth error with no scheme / already retried) → `reset_telemetry(endpoint, reading)` so the
     endpoint's fields are `N/A`
3. record per-endpoint success, advance to the next endpoint
4. when the device's endpoints are exhausted, update its reachability from this pass's outcome (see Reachability),
   advance the cursor, and start the next device; at end of pass, arm the next pass per Cadence

Each step is fetch-callback driven: the reply handler folds, resets, or re-authenticates, then chains the next request.

### Session state

Per-device session state — the cached token (for auth families) and the per-pass re-auth guard — is wasm-only, keyed by
`DeviceId`, and lives in the orchestration layer. It is kept separate from the generic `KnownDevice`, which holds the
resulting telemetry and reachability. A device removed by discovery drops its session state. No-auth families simply
never populate a token.

## Reachability and Device Presence

This slice's device list is an explicit **interim / debug** view: every device discovered by mDNS stays listed, shown
with its readings or `N/A`. It is not the final fleet UI, and "listed here" is not the same as "counted in the fleet."

To keep the later aggregation slice from inheriting discovery-only presence, each telemetry pass updates a per-device
**reachability** flag (`KnownDevice.reachable`, which already exists): a device is reachable when its latest pass
obtained usable telemetry from **at least one endpoint**. For an auth family a rejected login leaves every endpoint
failed, so a miner whose shared password is wrong falls out naturally; a no-auth family is reachable whenever its
unauthenticated endpoints respond. An unreachable device has `reachable = false` and clears to `N/A`, but remains in the
interim list. The aggregation slice will count and aggregate only `reachable` devices, satisfying the story's "track,
count, and aggregate only currently reachable devices" / "an unreachable device is omitted." The reachability decision
is a pure function of the pass's per-endpoint outcomes, so it is host-tested.

## FamilyAdapter Growth

The trait gains telemetry methods alongside the existing discovery ones. All parsing is pure and host-tested; request
building is pure string construction:

```rust
pub trait FamilyAdapter {
    fn browse_service_types(&self) -> &'static [&'static str];
    fn parse_found(&self, json: &dyn JsonLookup) -> Option<DiscoveredDevice>;

    fn api_base_path(&self) -> &'static str;
    fn telemetry_endpoints(&self) -> &'static [&'static str];

    /// Reset the fields this endpoint owns to `None`, then populate those
    /// present in `json`. Resetting first guarantees a value that disappeared
    /// from the response transitions `Some(..) -> None` instead of going stale.
    fn parse_telemetry(&self, endpoint: &str, json: &dyn JsonLookup, reading: &mut TelemetryReading);

    /// Reset only the fields this endpoint owns to `None`. Called on an
    /// endpoint fetch failure so a flaky endpoint never leaves stale values.
    fn reset_telemetry(&self, endpoint: &str, reading: &mut TelemetryReading);

    // Authentication — default is NONE. Families needing auth (e.g. BOS)
    // override these; no-auth families (e.g. Bitaxe) inherit the defaults and
    // are fetched unauthenticated.

    /// The login endpoint path, or `None` if the family needs no auth.
    fn auth_endpoint(&self) -> Option<&'static str> { None }
    /// Build the login request body from the shared password.
    fn login_body(&self, _password: &str) -> String { String::new() }
    /// Extract a session token from a login reply.
    fn parse_login(&self, _json: &dyn JsonLookup) -> Option<String> { None }
    /// Format the auth header carrying a cached token.
    fn auth_header(&self, token: &str) -> String { fmt!("Authorization: {token}") }
    /// Whether a telemetry response status indicates an auth failure that a
    /// re-login could fix. Default `false`: no-auth families never auth-fail.
    fn is_auth_error(&self, _status: u32) -> bool { false }
}
```

The trait stays object-safe (used as `&dyn FamilyAdapter`). All of these are pure (the string builders use the SDK
`fmt!`/`JsonStr` helpers, which are host-available, as `mining-info`'s `endpoint()` already demonstrates). The wasm
orchestration turns the adapter's paths plus the device host/port/token into real `FetchRequest`s, applies
`is_auth_error` to decide when to re-authenticate, and feeds replies (or failures) back into the parse/reset methods.

BOS overrides `auth_endpoint` (→ `Some("/auth/login")`), `login_body`, `parse_login`, and `is_auth_error` (→
`status == 401 || status == 403`); the default `auth_header` already fits. Bitaxe overrides none of the auth methods.

## Field Mapping (BOS)

Hashrates are reported in GH/s and converted to TH/s (divide by 1000):

- `/miner/stats`
  - current hashrate ← `/miner_stats/real_hashrate/last_1m/gigahash_per_second`
  - power (W) ← `/power_stats/approximated_consumption/watt`
- `/miner/hw/hashboards`
  - temperature (°C) ← the **maximum chip temperature across boards**
    (`/hashboards/N/highest_chip_temp/temperature/degree_c`)
- `/miner/details`
  - uptime (s) ← `/bosminer_uptime_s`

`TelemetryReading.nominal_hashrate_ths` keeps its field but is left `None` this slice. The per-device temperature is the
hottest chip across the device's boards (the thermal-limiting metric).

## Failure, Freshness, and Staleness

Each endpoint owns a disjoint set of `TelemetryReading` fields. The contract is **reset-then-populate per endpoint**:

- on a successful response, `parse_telemetry` first resets the endpoint's owned fields to `None`, then writes those
  present in the payload — so a field that vanishes from an otherwise-200 response correctly becomes `None` rather than
  retaining its previous `Some(..)`
- on an endpoint fetch failure (network error, non-2xx), the driver calls `reset_telemetry` for that endpoint, clearing
  only its owned fields

So a single flaky endpoint never wipes another's data, an unreachable miner shows `N/A` rather than last-good numbers,
and stale `Some -> None` transitions are handled. This honors "exclude missing readings rather than substituting
previous values" and keeps each field independently fresh-or-absent, which is what the later per-field aggregates need.

A device is never removed by telemetry outcome; only discovery removal drops it from the list. Telemetry outcome instead
drives the reachability flag (see Reachability).

## Parameter Updates

The widget exports `on_params_update`. When `miner_password` changes, stale state tied to the old credential must not
linger:

- clear every device's cached token, so each re-authenticates with the new password at its next slot
- clear all telemetry readings to `None` — every telemetry endpoint here is authenticated, so all readings are
  credential-derived
- reset reachability accordingly and `request_frame`

This mirrors the deliberate credential-change reset `mining-info` performs in its own `on_params_update`.

## Data Model

`TelemetryReading` and `TelemetrySnapshot` keep their skeleton shapes (all fields `Option`). The driver folds an
endpoint's parse into the device's working reading, then stamps it onto the device. `DeviceList` gains a keyed mutator
so the driver can update a device by id, plus a way to set reachability:

- `reading_mut(&DeviceId) -> Option<&mut TelemetryReading>` (or an equivalent fold-in operation)
- a `set_reachable(&DeviceId, bool)` (or fold reachability into the same update)

## Rendering

Each device row in the existing list gains its readings: current hashrate (TH/s), power (W), and temperature (°C), each
shown as a value or `N/A`; an unreachable device shows `N/A` across the board. Numbers are formatted through
`format_number!` so the device localization setting applies (the manifest already declares `localization`); native/test
builds use a plain fallback for the magnitude per the best-practices split. This is still the interim list, not the
final aggregate UI.

## Modules

- `families/bos.rs` — adapter telemetry methods plus the pure `parse_login`, `parse_telemetry`, and `reset_telemetry`
  (with host tests).
- `telemetry.rs` — existing data types, unchanged in shape.
- `session.rs` (new) — the fetch-driven round-robin driver, the inter-pass timing, and per-device token state
  (wasm32-only). Its **pure** pieces — the round-robin cursor (which device is next, pass-complete detection, pass
  snapshotting) and the reachability decision — are small host-tested structs/functions; only the fetch and
  `render`/`request_frame_after` wiring is gated.
- `device.rs` — add the keyed reading mutator and reachability setter.
- `lib.rs` — read the `miner_password` param; export `on_params_update` for the credential-change reset; kick the driver
  when discovery adds the first device, let it idle when the fleet empties; accumulate `delta_ms` for pass cadence.
- `manifest.json` — add the `miner_password` string param (default `"root"`); regenerate `manifest_params.rs` with
  `just wasm::gen fleet-management` (from the repo root; from `bmc-wasm-runtime/` it is `just gen fleet-management`).

## Testing

Host unit tests for the pure surface:

- `parse_telemetry` for each endpoint from `MapJson` fixtures: GH/s→TH/s conversion, temperature as the max chip across
  boards, power and uptime, and missing fields preserved as `None`
- **stale-clearing**: starting from a fully-populated `TelemetryReading`, a `parse_telemetry` whose payload omits an
  owned field drives that field `Some(..) -> None`; and `reset_telemetry` clears exactly the endpoint's owned fields and
  nothing else
- `parse_login` extracts the token, and returns `None` on a tokenless reply
- `is_auth_error`: BOS reports `true` for `401`/`403` and `false` for `200`; the default (no-auth) implementation
  reports `false` for any status
- the round-robin cursor (pure): advance through its snapshot, detect pass completion, and iterate exactly the captured
  ids. Re-snapshotting per pass — so a device added or removed mid-pass is only seen next pass — and return-to-start are
  driver behaviors (a fresh cursor per pass), verified in the testbed rather than unit-tested
- the reachability decision: reachable when ≥1 endpoint produced usable data; unreachable when none did (which also
  covers a rejected login leaving every endpoint failed)

The fetch wiring, the live round-robin cadence, and the credential-change reset are verified in the testbed
(`just wasm::dev fleet-management` from the repo root, or `just dev fleet-management` from `bmc-wasm-runtime/`) against
a reachable BOS miner.

## Deferred

- **Nominal hashrate** — skipped this slice; the `TelemetryReading` field stays but is unpopulated. Re-introduced when
  okay/not-okay needs it.
- **Okay / not-okay classification** (current hashrate < 20 % of nominal) — a BDK-506 acceptance criterion that depends
  on nominal, so it moves to the aggregation slice alongside nominal.
- **Aggregation, online/okay counts, and the final overview / per-model UI** — the two following slices. The
  reachability flag this slice maintains is the hook those counts and aggregates will filter on.

## Success Criteria

- Each discovered BOS miner is polled for current hashrate, power, temperature, and uptime, authenticating reactively
  with the shared password when the adapter reports an auth error (no backoff; one re-auth per device per pass).
- Auth is a family-owned, optional adapter capability: BOS provides it, the default is none, and the driver is
  auth-agnostic — a no-auth family fetches unauthenticated through the same path.
- Telemetry is fetched one device at a time in a round-robin, with passes starting roughly every 30 s for the targeted
  small fleet, keeping in-flight fetches well under the host budget.
- A failed or vanished endpoint clears only its own fields to `N/A` (including `Some -> None`); no stale values are
  shown.
- Each device carries a reachability flag updated from its latest pass; auth-rejected and unreachable miners are
  `reachable = false` while staying in the interim list, so later aggregation counts only reachable devices.
- A `miner_password` change clears cached tokens and credential-derived readings.
- The device list shows live readings (or `N/A`) per device on the device.
- Parsing, login-token extraction, stale-clearing, the reachability decision, and the round-robin cursor are covered by
  host unit tests; no family-specific code leaks into the generic registry.
