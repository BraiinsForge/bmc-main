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

How widget action requests (sound, LED) travel from the wasm guest SDK through the host runtime, onto `deck_widget_v1`,
and into the compositor's action handler. Covers the guest-side surface (`set_effect`/`stop`, `play_sound`/`stop_sound`),
the `LedRequest` runtime channel and per-guest request-id allocator, the scene-aware `LedSceneManager`
(endless-stack supersession, per-scene temporary queue, scene-change pause/resume), the sound manager's cancellable
playback task, and discriminant pinning across the wasm-FFI, wayland, and hardware-driver boundaries.
