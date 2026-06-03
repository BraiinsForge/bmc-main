# Fleet Management Widget Skeleton Design

## Context

BDK-506 adds a WASM widget that discovers miners on the local network,
polls telemetry, and renders fleet-level and per-model aggregates. The first
implementation slice should create only the widget skeleton and generic
device/model support. Family-specific API support starts later, with BOS as
the first implementation.

The BDK-506 Jira task is saved locally in
`tasks/BDK-506-fleet-management-widget.md`.

## Goals

- add a new `fleet-management` WASM widget crate skeleton
- define generic data structures for discovered devices, device identity,
  families, models, and telemetry
- keep a list of currently known devices in widget state
- make family-specific API support pluggable through adapter traits
- avoid implementing BOS, uBOS, or Bitaxe telemetry parsing in this slice
- keep the skeleton testable without the WASM host

## Non-Goals

- implement BOS API polling
- implement uBOS or Bitaxe API polling
- compute fleet aggregates
- render the final fleet overview or per-model breakdown
- add historical data or persistence
- add a per-device UI

## Widget Crate

Add a new production WASM widget crate at `widgets-wasm/fleet-management`.

The crate should follow the existing widget workspace conventions:

- package name: `fleet-management`
- crate type: `cdylib`
- dependency on `bmc-wasm-sdk`
- listed in `widgets-wasm/Cargo.toml`
- manifest file at `widgets-wasm/fleet-management/manifest.json`

The manifest should make the widget buildable and selectable, but it does
not need final production copy or parameters. The initial supported viewport
decision should match the story direction: full and large are the meaningful
targets, while small and medium are not supported unless later design work
changes that.

Nix widget packaging is filesystem-catalog driven for WASM widgets, so a
valid manifest and workspace entry should be enough for the normal
`widget-fleet-management` package path to appear through the existing
catalog.

## Module Layout

Use focused modules with clear ownership:

- `lib.rs`: WASM exports, root state, mDNS browse registration, and minimal
  render entry point.
- `discovery.rs`: mDNS event parsing and conversion into discovery
  candidates.
- `device.rs`: device identity, family enum, known-device state, and
  device-list operations.
- `model.rs`: model identifiers, model names, chip metadata, and nominal
  model capabilities.
- `telemetry.rs`: normalized telemetry readings and freshness metadata.
- `adapter.rs`: generic family-adapter trait and shared adapter types.
- `render.rs`: minimal empty/list render state for the skeleton.
- `manifest_params.rs`: generated manifest params if the widget manifest has
  parameters.

This split keeps the family-specific API implementations out of the generic
device registry. Later `families/bos.rs`, `families/ubos.rs`, and
`families/bitaxe.rs` modules can implement the adapter trait without changing
the common data model.

## Core Types

The generic model should be data-first rather than trait-object-heavy.

`DeviceFamily` should be an enum:

- `Bos`
- `Ubos`
- `Bitaxe`

These values describe the API family, not a specific hardware model.

`DeviceId` should be a stable key for the runtime session. It should be
derived from mDNS discovery identity, such as service type plus service name
or resolved host. The exact representation can be an owned string wrapper so
the list can be keyed deterministically without introducing UUID generation.

`DeviceIdentity` should carry user-facing and routing identity:

- `id`
- `family`
- `name`
- `host`
- `port`

`KnownDevice` should combine identity and current state:

- `identity`
- `model`, initially optional
- `telemetry`, initially absent
- `last_seen_ms`, from runtime-relative time or discovery event sequencing
- reachability state for the current session

Use `Option` for absent data. Do not encode missing values as zero.

## Device List

`DeviceList` is the in-memory list of devices discovered in the current
widget session. It should provide small, testable operations:

- insert or update a discovered device
- mark a device seen again
- remove a device when discovery reports removal
- iterate known reachable devices
- return count and empty-state information

The list should not persist to storage. A widget restart rebuilds it from
mDNS.

The first skeleton only needs list maintenance, not expiry. If later polling
needs to omit unreachable devices after failed telemetry, that logic should
live in the polling layer or a later `mark_unreachable` operation.

## Discovery

Use the WASM SDK mDNS support:

- browse `_bos._sub._http._tcp` directly for BOS miners
- add separate uBOS and Bitaxe/AxeOS browse service types later when their
  advertisements are defined
- keep discovery parsing generic enough for uBOS and Bitaxe service names or
  TXT records to be added later

`discovery.rs` should expose a host-independent parser around the JSON string
delivered by `MdnsEvent::Found`. The parser should return a
`DiscoveredDevice` with a `DeviceIdentity` when it can classify the family.
Unknown services should be ignored rather than stored as devices.

Because family-specific matching details for uBOS and Bitaxe are not settled
yet, the skeleton may include enum variants and parser tests for BOS-shaped
input only if needed to validate the list path. Full BOS discovery behavior
belongs to the BOS implementation slice.

## Adapter Boundary

Define a small `FamilyAdapter` trait for later family modules:

- identify whether a discovered device belongs to the family
- build telemetry fetch requests for a known device
- parse family-specific replies into normalized telemetry and model metadata

The skeleton should not store trait objects in the device list. Instead,
family selection can happen at the orchestration boundary based on
`DeviceFamily`. This keeps the runtime state serializable-looking and simple
for unit tests.

## Telemetry Model

`TelemetryReading` should normalize the fields from BDK-506:

- current hashrate in TH/s
- nominal hashrate in TH/s
- power in W
- uptime in seconds or hours, with one canonical internal unit
- temperature in deg C

Each field should be optional because missing readings are excluded from the
affected aggregate later.

`TelemetrySnapshot` should associate a reading with the device and the time
it was refreshed. Aggregation is not part of this skeleton, but the types
should make latest-only aggregation natural later.

## Model Support

`MinerModel` should represent the hardware model independently from API
family. It should support:

- a stable model id
- display name
- optional chip type
- optional chip count
- optional nominal hashrate

The generic model should allow family adapters to report model metadata when
known, while keeping all fields optional until a family implementation can
populate them.

## Minimal Rendering

The skeleton render path should only prove the widget is wired:

- show an explicit empty state when `DeviceList` is empty
- show a compact count/list summary when devices are present

This is not the final BDK-506 UI. The final overview and per-model breakdown
will be designed and implemented after discovery, telemetry, and aggregation
are in place.

## Testing

Add unit tests for the host-independent modules:

- `DeviceList` inserts new devices
- `DeviceList` updates an existing device with the same id
- `DeviceList` removes a device by id
- discovery parser ignores unknown mDNS services
- discovery parser can produce a device candidate from a known family-shaped
  event if the skeleton includes a BOS-shaped fixture
- telemetry/model constructors preserve missing values as `None`

Build verification should include the widget workspace check or package build
appropriate for WASM widgets.

## Success Criteria

- `fleet-management` exists as a buildable WASM widget crate.
- The widget has a minimal render path with an explicit empty/list state.
- The generic modules define device families, device identity, model
  metadata, telemetry readings, adapter boundary, and device-list operations.
- No family-specific API polling is implemented.
- The code is covered by focused unit tests for the generic device/model
  behavior.
