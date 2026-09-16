# Hardware gRPC (`HardwareService`)

## Scope

This document covers the frozen legacy gRPC API the frontend uses during initial setup and for combined-scene capability
gating. New platform capabilities are exposed through `window.SYSTEM.capabilities` in `/system.js`; do not add them to
the `HardwareCapabilities` message. Scene/widget management itself is documented in
[`scene-management.md`](scene-management.md).

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
- `wifi_supported` (`bool`) — the profile's Wi-Fi support.
- `ethernet_supported` (`bool`) — the profile's ethernet support.
- `mining_supported` (`bool`) — whether the product exposes mining setup.
- `product_name` (`string`) — the product name shown during initial setup.
- `boser_managed` (`bool`) — whether boser manages the product.

This field set is frozen. The frontend setup flow consumes the product, Wi-Fi, ethernet, and mining fields; the
combined-scene editor consumes `combined_scenes_supported`.

## Capability Gating

`combined_scenes_supported` mirrors the backend precondition enforced by `SceneManagementService`: when `slot_grid` is
absent, the combined-scene RPCs (`AddCombinedScene`, and combined-scene paths of `AddWidget`/`UpdateWidget`) are
rejected with `FailedPrecondition`. The frontend should hide combined-scene controls when this flag is `false` rather
than relying on the RPC error.

## Frontend Guidance

- Query `GetHardwareCapabilities` once at startup and gate combined-scene UI on `combined_scenes_supported`.
- Treat the backend precondition as authoritative; the flag is a UI hint, not a substitute for handling
  `FailedPrecondition`.
- Read new browser-facing platform capabilities from `window.SYSTEM.capabilities` instead of extending this message.
