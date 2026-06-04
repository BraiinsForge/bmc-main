# uBOS Family Support Design

## Context

BDK-506 builds a fleet-management WASM widget. Earlier slices added the widget skeleton, the generic device model, the
discovery-only `FamilyAdapter` trait, BOS mDNS discovery, and a fetch-driven round-robin telemetry driver
(`2026-06-03-fleet-management-skeleton-design.md`, `2026-06-03-bos-telemetry-design.md`). BOS is the only family wired
in so far.

This slice adds **uBOS** as the second family: mDNS discovery plus telemetry. uBOS devices appear in the existing
interim device list next to BOS devices, showing the same readings. To host a second family the telemetry driver — today
hardcoded to `BosAdapter` — becomes family-generic; that refactor is part of this slice because a second family is
impossible without it.

uBOS advertises itself as the full mDNS service type `_ubos._tcp` and exposes a flat JSON API. The sample reading is:

```json
{
  "name": "BMM Adapter W5500",
  "hashrate": 1071197262612,
  "uptime": 382,
  "temperature": 59,
  "power_out_mw": 35000,
  "fan_rpm": 1260,
  "fan_pwm": 20,
  "pools": 1,
  "ip": "192.168.89.109",
  "time": "07:31:51 UTC"
}
```

Fields are shown with their real API names; `power_out_mw` is power in milliwatts.

A prerequisite change (`2026-06-04-bos-miner-model-design.md`) adds the generic model-capture machinery — the
`parse_model` trait method, `ModelAccumulator`, `DeviceList::apply_model`, the driver accumulation, and the model column.
It will be in place by the time this slice is implemented, so this slice builds on it directly, including uBOS's
`parse_model` override.

**Merge order:** the BOS-miner-model change lands first; this slice rebases on top and adds the `parse_model` override
against the in-place machinery.

## Goals

- discover uBOS miners via the `_ubos._tcp` browse and list them next to BOS devices
- fetch per-device telemetry — current hashrate, power, temperature, uptime — from uBOS `/api/info`
- support a proactive, family-owned credential header (HTTP Basic for uBOS) alongside the existing reactive token auth
  (BOS), through one auth-agnostic driver
- make the telemetry driver dispatch to each device's family adapter instead of assuming BOS
- capture each uBOS device's model name from `/api/info` into the generic device model
- keep all parsing and dispatch pure and host-tested; only fetch and timing wiring stays behind the wasm boundary

## Non-Goals

- Bitaxe / AxeOS discovery or telemetry
- fleet or per-model aggregation, okay/not-okay classification, nominal hashrate
- the final overview / per-model UI
- the model-capture machinery (`ModelAccumulator`, `apply_model`, driver accumulation, the model column) — owned by the
  concurrent BOS-miner-model change; this slice adds only uBOS's `parse_model` override
- persistence
- wiring uBOS credentials to a configurable parameter (hardcoded `root:root` this slice — see Authentication)

## Driver: Family-Generic Dispatch

The driver in `session.rs` currently calls `BosAdapter.method()` directly for the API base path, telemetry endpoints,
login, and auth-error checks. It becomes family-generic:

- a free function `adapter_for(family: DeviceFamily) -> Option<&'static dyn FamilyAdapter>` maps a family to its adapter:
  `Some` for `Bos`/`Ubos` (a constant-promoted `&BosAdapter` / `&UbosAdapter`) and `None` for `Bitaxe`, which has no
  adapter yet. The match is exhaustive with no wildcard arm, and the enum carries no unenforced "never constructed"
  invariant — an unsupported family is represented, not assumed-away.
- when `adapter_for` returns `None` for a device (a family with no adapter), the driver logs once, then marks the device
  unreachable with cleared telemetry — `apply_telemetry(id, TelemetryReading::default(), false)`, the same outcome as a
  pass where every endpoint failed — and advances the cursor. This step is required, not cosmetic:
  `DeviceList::upsert` marks a freshly discovered device `reachable = true`, so merely skipping it would leave it falsely
  reachable and counted by the later reachable-only aggregation. No `Bitaxe` browse is registered today, so this path is
  not exercised; it degrades to an unreachable, `N/A` device rather than panicking if a future family is discovered
  before its adapter lands.
- `current_endpoint()` additionally returns the device's `family`; each per-device cycle resolves `adapter_for(family)`
  once and uses the adapter for `api_base_path`, `telemetry_endpoints`, `parse_telemetry`, `reset_telemetry`, and the
  auth methods.
- the login / re-auth path stays gated on `adapter.auth_endpoint().is_some()`, so a no-login family (uBOS) skips it. BOS
  behavior is unchanged.

The round-robin cadence, the per-pass re-auth guard, the token cache, and reachability are all unchanged in shape; only
the adapter selection moves from a constant to a per-device lookup.

## Authentication: Proactive Credential Header

The trait gains one method, defaulting to none:

```rust
fn credential_header(&self) -> Option<String> { None }
```

The driver attaches it to every request, preferring it over any cached token:

```rust
let header = adapter
    .credential_header()
    .or_else(|| token.map(|t| adapter.auth_header(&t)));
```

BOS returns `None` and keeps its reactive token flow (GET → 401 → login → cache token → retry). uBOS returns a static
HTTP Basic header and never logs in. The two schemes are mutually exclusive per family, so they never conflict. The
owned `String` return matches the existing `auth_header(token) -> String`.

uBOS credentials are hardcoded to `root:root` this slice, so `credential_header()` returns
`"Authorization: Basic cm9vdDpyb290".to_owned()` (`base64("root:root")`) with a comment recording the plaintext, so no
base64 encoder is needed now. The owned return type is what keeps a future configurable password feasible without
changing the trait: that change swaps the hardcoded constant for a header computed from the param. It is not literally
one line — it also needs a base64 of `root:<pw>` — but it is localized to `UbosAdapter` and the trait shape is unchanged.

uBOS overrides none of the existing auth methods: `auth_endpoint()` stays `None` and `is_auth_error()` stays `false`.
Wrong credentials therefore yield a 401, no re-auth, the endpoint's fields reset to `N/A`, and the device becomes
`reachable = false` — the correct outcome, retried at the next pass.

## uBOS Discovery

uBOS advertises the full service type `_ubos._tcp`, unlike BOS which advertises the `_bos` subtype of `_http._tcp`. A
second browse is registered in `lib.rs`:

- `UbosAdapter::browse_service_types()` returns `["_ubos._tcp"]`.
- `parse_found` reuses `extract_endpoint` (name, host, port) and stamps `DeviceFamily::Ubos`. The `DeviceId` is the mDNS
  service instance fullname (`<instance>._ubos._tcp.local.`, the `name` field of the `Found` JSON — the service instance
  label, not necessarily the resolved host), distinct from BOS device ids, so the two families coexist in one
  `DeviceList`.
- `lib.rs` adds `on_ubos_event`, mirroring `on_bos_event` but routing to `UbosAdapter`, and registers the uBOS browse in
  `init()` alongside the BOS browse. The host allows up to four browses, so two is within budget.
- the `#[expect(dead_code)]` on `DeviceFamily::Ubos` is dropped, since the adapter now constructs it.

## uBOS Telemetry

uBOS has a single telemetry endpoint:

- `api_base_path()` = `"/api"`, `telemetry_endpoints()` = `["/info"]`, giving the URL `http://{host}:{port}/api/info`.
- the port comes from the mDNS SRV record. **Assumption:** `_ubos._tcp` advertises the API port (8080 in the sample),
  verified live in the testbed.

`parse_telemetry("/info", json, reading)` follows the reset-then-populate contract; the single endpoint owns all four
fields. uBOS reports integers, which the host's `as_f64()` coerces, so the existing `f64` accessor reads them:

- current hashrate (TH/s) ← `/hashrate` (H/s), divided by `1e12`
- power (W) ← `/power_out_mw` (milliwatts), divided by `1000`
- temperature (°C) ← `/temperature`
- uptime (s) ← `/uptime`, already in seconds, mapped directly

`reset_telemetry("/info", reading)` clears exactly those four fields, so a vanished or failed reading transitions
`Some -> None` instead of going stale.

## Model Capture

The model-capture machinery is provided by the concurrent BOS-miner-model change
(`2026-06-04-bos-miner-model-design.md`), not this slice: the `FamilyAdapter::parse_model(endpoint, json, &mut
ModelAccumulator)` method (default no-op), the all-`Option` `ModelAccumulator` builder, the driver's per-pass
accumulation and `apply_model` at `finalize_device`, the `MinerModel.id`/`name` change to `Option<String>`, and the
interim model column in `render.rs`. This slice depends on that change and adds only uBOS's `parse_model` override.

uBOS exposes the model as the `/api/info` `name` field (`"BMM Adapter W5500"` in the sample). `UbosAdapter::parse_model`
for the `/info` endpoint sets the accumulator's model name from `/name`, leaving `id`, `chip_type`, `chip_count`, and
`nominal_hashrate_ths` untouched — uBOS does not expose them. The name comes from the same `/api/info` response as
telemetry, so it is obtained whenever the device is reachable; unlike BOS there is no separate auth-gated endpoint
guarding it.

The driver re-parses the model every pass (machinery behavior); uBOS's responses are identical between passes, so this
just keeps the value correct after a relabel. Once both changes land, uBOS devices show their model name in the column
the concurrent change adds.

## Rendering

This slice adds no `render.rs` code. `render::view` already iterates every device and renders hashrate, power,
temperature, and uptime (or `N/A`), so uBOS rows appear automatically once the driver polls them. The interim model
column is added by the concurrent BOS-miner-model change; uBOS's parsed model populates it once both land. This is still
the interim list, not the final aggregate UI.

## Modules

- `families/ubos.rs` (new) — `UbosAdapter`: discovery, the telemetry endpoint and field mapping, the credential header,
  and the uBOS `parse_model` override, with host tests.
- `families.rs` — declare `pub mod ubos;`.
- `adapter.rs` — add the `credential_header` default method. (`parse_model` and `ModelAccumulator` arrive with the
  concurrent BOS-miner-model change.)
- `session.rs` — `adapter_for` and per-device adapter selection, plus credential-header attachment. The per-device
  dispatch also routes `parse_model` through the device's adapter, merging with the concurrent change's accumulation
  wiring.
- `lib.rs` — `on_ubos_event`, register the uBOS browse, drop the `Ubos` dead-code expectation.
- `device.rs` — drop the `#[expect(dead_code)]` on `DeviceFamily::Ubos` once it is constructed. (`apply_model` arrives
  with the concurrent change.)

`manifest.json` is unchanged — uBOS adds no parameter this slice.

## Testing

Host unit tests for the pure surface:

- `UbosAdapter::parse_found` parses a `_ubos._tcp`-shaped `Found` fixture, stamps `DeviceFamily::Ubos`, and rejects an
  event missing host or port
- `parse_telemetry` from a `MapJson` fixture: `hashrate` 1071197262612 → 1.07 TH/s, `power_out_mw` → watts (÷1000),
  `temperature` direct, `uptime` 382 seconds mapped directly, and missing fields preserved as `None`
- stale-clearing: a populated reading whose payload omits an owned field drives that field `Some -> None`, and
  `reset_telemetry` clears exactly the endpoint's owned fields
- `credential_header` returns the Basic header for uBOS and `None` for BOS
- `UbosAdapter::parse_model` for `/info` sets the model name from `/name` into a `ModelAccumulator`, leaving the other
  fields untouched
- `adapter_for` returns `Some` matching adapter for `Bos`/`Ubos` and `None` for `Bitaxe`

The fetch wiring, the SRV-port assumption, the mixed BOS+uBOS round-robin, and live readings are verified in the testbed
(`just dev fleet-management` from `bmc-wasm-runtime/`) against a reachable uBOS device.

## Success Criteria

- uBOS miners are discovered via the `_ubos._tcp` browse and listed alongside BOS miners in one device list.
- Each uBOS miner is polled at `/api/info` with a hardcoded HTTP Basic header for current hashrate, power, temperature,
  and uptime, with the correct unit conversions (H/s → TH/s, mW → W).
- The telemetry driver dispatches to each device's family adapter rather than assuming BOS; BOS behavior is unchanged.
- A proactive credential header is a family-owned, optional adapter capability that coexists with the reactive token
  scheme through one auth-agnostic driver.
- uBOS's `parse_model` sets the model name from `/api/info`, so it lands on `KnownDevice.model` through the shared
  model-capture machinery from the concurrent BOS-miner-model change.
- A failed or vanished uBOS endpoint clears only its own fields to `N/A`; wrong credentials leave the device
  `reachable = false`.
- Parsing, field mapping, the credential header, and adapter dispatch are covered by host unit tests; no family-specific
  code leaks into the generic registry.
