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

### [BMC Profiles](profiles.md)

How `bmc-nix` builds custom profile generations, how the optimized symlink tree is structured, and where hooks,
activation scripts, generated files, and manifests fit into the profile lifecycle.

### [Upgrades](upgrades.md)

How application and firmware upgrades are resolved and applied: package indexes, feeds, and servers, the no-downgrade
resolution algorithm, the feed-resolved index used during BOS upgrades, BOS downgrade on BMM101, rollback, garbage
collection, and store initialization.

### [Firmware and Application Upgrade Interlinking](firmware-package-interlinking.md)

Why every firmware upgrade also invokes the application package upgrade from the sysupgrade tarball, how that path runs
from the `bmc-main` API through OpenWrt, how `bos-main` and `bos-packages` assemble the firmware-build payload, and why
an already-current package profile is the only normal no-op.

### [End-to-End Package Upgrade Harness](upgrade-e2e-harness.md)

How `deck upgrade-e2e` drives a full package upgrade cycle against a real Deck: serving locally built packages from the
developer machine via `nix run .#upgrade-server`, registering the server on the device, and exercising CheckForUpgrade
and StartUpgrade over gRPC while asserting the bmc profile advanced.

### [End-to-End Firmware Sysupgrade Harness](sysupgrade-e2e-harness.md)

How `deck e2e-sysupgrade` flashes two full firmware images against a real Deck to exercise the firmware Nix flow: the
init path (clear the store, flash image A) and the in-place upgrade path (flash image B), served from a local package
rig — including how to build the two differing-version images through the bos-main CI.

### [End-to-End gRPC Sysupgrade Harness](grpc-sysupgrade-e2e-harness.md)

How `deck e2e-grpc-sysupgrade` upgrades a real Deck through the production `CheckForUpgrade` / `StartUpgrade` gRPC path
served from a host firmware index: anchoring the device version so the offer appears, pointing bmc at the index via
`BMC_INDEX_URL` while keeping it procd-supervised, and proving the flash by a boot-id change — the gRPC counterpart of
`e2e-sysupgrade`.

### [Mock Upgrade Scenarios](mock/upgrade-scenarios.md)

How `bmc-mock` simulates every upgrade state offline: the runtime-editable `upgrade-scenario.json` state selector, the
throttled local firmware blob server, how each failure surfaces on the gRPC stream, the simulated post-upgrade reboot
(`just fe::serve-loop`), and the gRPC e2e suite covering the matrix.

### [Service Orchestrator](service-orchestrator.md)

How OpenWrt services are reconciled after a profile activation: why the orchestrator runs as a detached transient procd
service, how it synchronizes with the activation through the profile lock and verifies via `current` that activation was
not reverted, and how per-service `etc/init.d.conf` configuration shapes the stop/start/upgrade action plan.

### [OpenWrt Firmware Tarball](openwrt-tarball.md)

What the firmware sysupgrade tarball contains from the Nix upgrade point of view: the on-tarball `bmc-nix-cli` binary
and the shipped `servers.json.default` registry, why the upgrade resolves its index through a per-firmware package feed
entry, and the release-blocking invariants a firmware build must satisfy.

### [Firmware Index Test Serving](firmware/README.md)

How to test firmware upgrades against a locally served release index: the `firmware-index-serve` flake app's local and
proxy modes, the `BMC_INDEX_URL` device override, and the example `index.v1.json`.

### [Widget Runtime Configuration](widget-runtime-configuration.md)

How a widget process receives its geometry, per-instance params, and current system settings over the `deck_widget`
Wayland protocol. Covers the spawn-environment contract (no BMC-specific env vars), identity resolution via
`SO_PEERCRED` on the Wayland socket, the configure-batch handshake widgets use to fetch viewport/display geometry and
initial state, and the runtime hot-reload path that pushes fresh params on the existing surface for geometry-stable
updates.

### [Widget Lifecycle](widget-lifecycle.md)

How the compositor derives widget lifecycle state from scene cycling and drag state, then sends
`deck_widget_surface_v1.lifecycle` events over Wayland. Covers initial lifecycle emission after the configure batch,
release/acquire batching, client flush ordering, valid transitions, and client-side event delivery.

### [Frontend](frontend.md)

Why the frontend and backend treat changes to their shared gRPC schema as one atomic product change, without preserving
obsolete RPCs and fields, compatibility shims, or protobuf reservations solely for older frontend builds.

### [Scene Management gRPC](grpc/scene-management.md)

Frontend-facing API contract for scene and widget management. Covers `SceneManagementService` RPCs, full-update
semantics of `UpdateWidget`, preview-stream exclusivity and lifecycle behavior, and server-side validation/precondition
rules that callers must handle.

### [Hardware gRPC](grpc/hardware-service.md)

Frontend-facing API contract for querying platform hardware capabilities. Covers
`HardwareService.GetHardwareCapabilities` and how `combined_scenes_supported` (derived from the platform's slot grid)
gates combined-scene controls and RPCs.

### [Widget Hardware Actions](widget-hardware-actions.md)

How widget action requests (sound, LED) travel from the wasm guest SDK through the host runtime, onto `deck_widget`, and
into the compositor's action handler. Covers the guest-side surface (`set_effect` / `set_effect_global` / `stop`,
`play_sound` / `stop_sound`), the two-tier LED model (local always wins; global fills in when nothing local is playing),
endless-slot supersession and temporary-queue serialization within each tier, scene-change drop-and-expire of the active
temp, the sound manager's cancellable playback task, and discriminant pinning across the wasm-FFI, wayland, and
hardware-driver boundaries.

### [Widget Manifest Specification](widget-manifest.md)

System-level concerns around widget manifests: on-disk location, compositor discovery, the parsing path from manifest
into the runtime, supported viewport constraints, and the rationale behind the validation rules. The per-field grammar
is intentionally not duplicated here — it lives in the Rust types of `bmc-widget-manifest` and is mirrored into the
committed `manifest.schema.json` artifact (with rustdoc propagated into the schema's `description` fields).

### [Image Widget Format Testing](image-widget-format-testing.md)

Test corpus for the image widget's decoder set: a verified sample per supported format, plus near-boundary and oversized
inputs that probe the pixel and allocation budgets from both sides. Covers the JPEG scale-on-load headroom, why
high-precision formats hit the ceiling sooner, the on-device scene-cycling procedure, and the log greps that show which
formats decoded.

### [Credential Egress Testing](credential-egress-testing.md)

How `deck check-credential-egress` proves on a real Deck that a resolved credential reaches the wire only where its type
allows: the three cases, why the image widget carries the placeholder, why bindings are written past the API's
validation, and the guards that stop a run where nothing fetched from reporting that the pin held.

### [WASM Widgets](wasm-widgets/)

How WASM widgets consume host-delivered inputs. Covers per-widget params generated from `manifest.json`, hardcoded
deck-wide system settings exposed by the SDK, widget/display geometry, update hooks, examples, and testbed usage.

### [System Overlays](system-overlays/)

How privileged, non-widget UI surfaces (startup connection progress, an offline indicator, the swipe-from-top
quick-settings panel) are built as `wlr-layer-shell` clients that stack above the active scene. Covers the
`bmc-system-overlay` framework crate and its `SystemOverlay` trait, the hosted-vs-standalone run modes, the compositor's
layer-shell compositing/buffer-tracking/edge-gesture support, the three vendored protocols (`deck_screen_edge_v1`,
`deck_settings_v1`, `deck_alarm_v1`), and the concrete overlays.

### [License Headers](license-headers.md)

The copyright and GPLv3 header every first-party source file carries: the exact header format, per-language comment
styles, how copyright lines are attributed from git authorship, and which third-party or generated files are exempt.

### WASM Host

Implementation notes for the multi-widget WASM runtime:

- [Process Model](wasm-host/process-model.md) - thin wrapper lifetime, Wayland fd passing, lazy host daemon startup, the
  thin/host control protocol, and teardown behavior.
- [Render Loop](wasm-host/render-loop.md) - slot lifecycle states, render-target ownership, render gating, frame
  scheduling, runtime delivery polling, and compositor lifecycle emission.
- [Renderer Asset Lifecycle](wasm-host/renderer-assets.md) - static package extraction, package/cache/volatile backing,
  stable renderer reservations, selective dormancy suspension, wake restoration, and image-widget cache behavior.
