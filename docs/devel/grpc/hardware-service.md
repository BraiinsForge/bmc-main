# Hardware gRPC (`HardwareService`)

## Scope

This document covers the gRPC API the frontend uses to query hardware capabilities of the active platform. It exposes
the single feature flag the frontend needs to decide which scene-management controls to show. Scene/widget management
itself is documented in [`scene-management.md`](scene-management.md).

Primary sources:

- `bmc-grpc/proto/web/hardware.proto`
- `bmc/src/web/grpc/hardware.rs`

## RPCs

- `GetHardwareCapabilities` returns the `HardwareCapabilities` message for the active platform.

## `HardwareCapabilities`

- `combined_scenes_supported` (`bool`) — whether the platform supports combined scenes (multiple widgets placed on a
  slot grid). It is derived as `caps.slot_grid.is_some()`: a platform reports `true` only when its
  `bmc-platform::HardwareProfile` defines a slot grid. Today only `BMC100` has a slot grid; `BMM100`, `BMM101`, and
  `BFM100` report `false`.

## Capability Gating

`combined_scenes_supported` mirrors the backend precondition enforced by `SceneManagementService`: when `slot_grid` is
absent, the combined-scene RPCs (`AddCombinedScene`, and combined-scene paths of `AddWidget`/`UpdateWidget`) are
rejected with `FailedPrecondition`. The frontend should hide combined-scene controls when this flag is `false` rather
than relying on the RPC error.

## Frontend Guidance

- Query `GetHardwareCapabilities` once at startup and gate combined-scene UI on `combined_scenes_supported`.
- Treat the backend precondition as authoritative; the flag is a UI hint, not a substitute for handling
  `FailedPrecondition`.
