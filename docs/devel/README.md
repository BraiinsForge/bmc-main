# BMC Developer Documentation

Engineering-facing documentation for components that warrant their own write-up beyond a user story — protocol
specifications, internal interfaces, spawn-environment contracts, and architectural rationale. Stories live in
[`docs/stories/`](../stories/README.md); this directory is for the implementation-side details a contributor needs to
work on or change a component.

Components large enough to grow multiple documents get their own subdirectory here.

## Documents

### [Supported Platforms](platforms.md)

How `bmc-openwrt` detects the active hardware platform, how BOS platform strings map to products, and how
`bmc-platform::HardwareProfile` describes display geometry, slot-grid support, LED strips, and frontend/backend
capabilities for `BMC100`, `BMM100`, `BMM101`, and `BFM100`.

### [Widget Runtime Configuration](widget-runtime-configuration.md)

How a widget process receives its geometry, per-instance params, and current system settings over the `deck_widget_v1`
Wayland protocol. Covers the spawn-environment contract (no BMC-specific env vars), identity resolution via
`SO_PEERCRED` on the Wayland socket, the configure-batch handshake widgets use to fetch viewport/display geometry and
initial state, and the runtime hot-reload path that pushes fresh params on the existing surface for geometry-stable
updates.

### [Widget Lifecycle](widget-lifecycle.md)

How the compositor derives widget lifecycle state from scene cycling and drag state, then sends
`deck_widget_surface_v1.lifecycle` events over Wayland. Covers initial lifecycle emission after the configure batch,
release/acquire batching, client flush ordering, valid transitions, and client-side event delivery.

### [Scene Management gRPC](grpc/scene-management.md)

Frontend-facing API contract for scene and widget management. Covers `SceneManagementService` RPCs, full-update
semantics of `UpdateWidget`, preview-stream exclusivity and lifecycle behavior, and server-side validation/precondition
rules that callers must handle.

### [Widget Hardware Actions](widget-hardware-actions.md)

How widget action requests (sound, LED) flow over `deck_widget_v1` to the action handler and on to `SoundController` /
`LedController`. Covers the dispatch architecture, why sound playback runs in its own cancellable task, and the wire →
hardware effect-type conversion that lives in `bmc/src/widget/action_handler.rs`.

### [Widget Manifest Specification](widget-manifest.md)

System-level concerns around widget manifests: on-disk location, compositor discovery, the parsing path from manifest
into the runtime, supported viewport constraints, and the rationale behind the validation rules. The per-field grammar
is intentionally not duplicated here — it lives in the Rust types of `bmc-widget-manifest` and is mirrored into the
committed `manifest.schema.json` artifact (with rustdoc propagated into the schema's `description` fields).

### [WASM Widgets](wasm-widgets/)

How WASM widgets consume host-delivered inputs. Covers per-widget params generated from `manifest.json`, hardcoded
deck-wide system settings exposed by the SDK, widget/display geometry, update hooks, examples, and testbed usage.

### WASM Host

Implementation notes for the multi-widget WASM runtime:

- [Process Model](wasm-host/process-model.md) - thin wrapper lifetime, Wayland fd passing, lazy host daemon startup, the
  thin/host control protocol, and teardown behavior.
- [Render Loop](wasm-host/render-loop.md) - slot lifecycle states, render-target ownership, render gating, frame
  scheduling, runtime delivery polling, and compositor lifecycle emission.
