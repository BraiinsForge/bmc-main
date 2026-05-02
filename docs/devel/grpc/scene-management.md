# Scene Management gRPC (`SceneManagementService`)

## Scope

This document covers the gRPC API contract used by the frontend to manage scenes/widgets and drive live preview.
Widget-runtime protocol details (`deck_widget_v1`) are documented separately in
[`widget-runtime-configuration.md`](../widget-runtime-configuration.md).

Primary sources:

- `bmc-grpc/proto/web/scene_management.proto`
- `bmc/src/web/grpc/scene_management.rs`

## Key RPCs

- Scene CRUD/order/state: `ListScenes`, `GetScene`, `AddFullscreenScene`, `AddCombinedScene`, `UpdateScene`,
  `MoveScene`, `CloneScene`, `RemoveScene`
- Preview: `PreviewScene` (server stream; scene stays active while stream is open)
- Widget CRUD: `AddWidget`, `UpdateWidget`, `RemoveWidget`
- Scene cycling: `GetSceneCycling`, `SetSceneCycling`
- Manifest catalog: `GetAvailableWidgets`, `GetWidgetManifest`

## Contract Notes

### `UpdateWidget` is full-update, not patch

- `UpdateWidget` replaces the widget params map as a whole.
- Missing keys are accepted on add (defaults applied), but rejected on update for required fields.
- Unknown keys are rejected.
- Param type and value constraints are validated against manifest:
  - type checks
  - enum membership
  - integer/double bounds
  - finite doubles
  - valid timezone strings

## Lifecycle Semantics

### Respawn behavior

- `UpdateWidget` with size change respawns widget process.
- `UpdateWidget` without size change does not respawn; params are hot-updated on the running instance.
- Position-only change does not respawn.

### Preview behavior

- `PreviewScene` is exclusive: only one preview stream can be active at a time.
- While preview is active, removing that same scene fails (`failed_precondition`).
- If previewed scene is disabled, widgets are spawned for preview duration and stopped when stream closes.

## Preconditions and Error Surface

### Manifest and size

- Add/update requires installed widget manifest.
- Requested size must be supported by the manifest.

### Placement and scene invariants

- Placement validation rejects out-of-bounds and overlaps in combined scenes.
- Fullscreen-scene widget is immutable in shape/placement via `UpdateWidget` constraints.

## Frontend Guidance

- Always send complete params in `UpdateWidget`.
- Keep local validation aligned with manifest schema, but still treat server validation errors as authoritative.
- Treat preview stream as a leased resource (single owner).
