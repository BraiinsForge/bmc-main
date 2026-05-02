# BMC Developer Documentation

Engineering-facing documentation for components that warrant their own write-up beyond a user story — protocol
specifications, internal interfaces, spawn-environment contracts, and architectural rationale. Stories live in
[`docs/stories/`](../stories/README.md); this directory is for the implementation-side details a contributor needs to
work on or change a component.

Components large enough to grow multiple documents get their own subdirectory here.

## Documents

### [Widget Runtime Configuration](widget-runtime-configuration.md)

How a widget process receives its geometry, per-instance params, and current system settings over the `deck_widget_v1`
Wayland protocol. Covers the spawn-environment contract (no BMC-specific env vars), identity resolution via
`SO_PEERCRED` on the Wayland socket, the configure-batch handshake widgets use to fetch their initial state, and the
runtime hot-reload path that pushes fresh params on the existing surface for geometry-stable updates.

### [Scene Management gRPC](grpc/scene-management.md)

Frontend-facing API contract for scene and widget management. Covers `SceneManagementService` RPCs, full-update
semantics of `UpdateWidget`, preview-stream exclusivity and lifecycle behavior, and server-side validation/precondition
rules that callers must handle.

### [Widget Hardware Actions](widget-hardware-actions.md)

How widget action requests (sound, LED) flow over `deck_widget_v1` to the action handler and on to `SoundController` /
`LedController`. Covers the dispatch architecture, why sound playback runs in its own cancellable task, and the wire →
hardware effect-type conversion that lives in `bmc/src/widget/action_handler.rs`.
