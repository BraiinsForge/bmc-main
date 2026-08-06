# Upgrade progress UI design

**Ticket:** BDK-451 **Date:** 2026-08-06 **Status:** approved design

## Context

Firmware upgrades currently run without on-device feedback. The stable 26.02 display stack showed a full-screen Slint
overlay for download, installation, success, and failure, but that path disappeared with `display_controller`.

The current upgrade pipeline supports both firmware and package-only upgrades. It already exposes the complete lifecycle
through `UpgradeRunState`: firmware download/verify/apply, package realize/verify/build/activate, byte progress,
finished, and typed failure. The web UI projects those states into `UpgradeProgress` in `upgrade.proto`.

The on-device UI must project that same source. It must not reproduce upgrade decisions or infer progress from unrelated
system state.

## Decision

Add a Deck-owned `deck_upgrade_v1` Wayland protocol and a `bmc-overlay-upgrade` system-overlay crate. The protocol
relays a display projection of `UpgradeRunState` through the compositor. The overlay crate exposes two static
layer-shell clients:

- a modal full-screen surface for every run containing a firmware upgrade
- a passive bottom-right surface for package-only runs

Both clients bind the same protocol and receive the same events. Each maps only for the upgrade kind it renders. An
inactive client stays unmapped and owns no DMA-BUFs.

This keeps firmware and package presentation separate without adding runtime surface reconfiguration to the overlay
framework.

## Goals

- Show the current upgrade stage without requiring the web UI.
- Show determinate download progress when the total is known.
- Keep widgets visible and interactive during package-only upgrades.
- Block the scene during firmware and combined upgrades.
- Replace stale progress immediately with a recognizable failure state.
- Show firmware success after reboot, then return to normal scene rotation.
- Keep `UpgradeRunState` as the single source of upgrade behavior.
- Preserve the hidden-overlay guarantee: an unmapped overlay retains no export buffers.

## Non-goals

- Do not add upgrade controls to the on-device UI.
- Do not expose internal `SystemUpgradeError` strings to users.
- Do not invent an overall percentage for stages that have no measurable progress.
- Do not duplicate the web UI's full changeset or release-note presentation.
- Do not make layer-shell placement dynamically configurable in this ticket.

## Data flow

```text
                         +-> gRPC UpgradeProgress -> web UI
UpgradeRunState stream --+
                         +-> display-state projection
                               |
                               v
                         latest-value watch
                               |
                               v
                         bmc startup bridge
                               |
                               v
                         CompositorCommand
                               |
                               v
                    compositor snapshot cache
                               |
                               v deck_upgrade_v1
                    +----------+----------+
                    |                     |
                    v                     v
             firmware overlay      package overlay
             full screen            bottom right
```

`UpgradeRunStream` is point-to-point, so the display does not independently consume it. A forwarding adapter observes
each state, updates the display projection, and passes the original state through unchanged to its existing consumer.
This follows the current `forward_led_events` shape.

The display projection uses a separate latest-value channel rather than expanding the lossy `SystemUpgradeState` used by
LED and restart policy. Its snapshot is equivalent to:

```rust
struct UpgradeDisplaySnapshot {
    generation: UpgradeGeneration,
    state: UpgradeDisplayState,
}

enum UpgradeDisplayState {
    Running {
        kind: UpgradeKind,
        phase: Option<SystemUpgradePhase>,
        progress: Option<DownloadProgress>,
    },
    Succeeded { kind: UpgradeKind },
    Failed { kind: UpgradeKind },
}
```

`UpgradeKind` is `Firmware` or `Packages`. A firmware run may contain package phases; it remains `Firmware` because it
uses the modal presentation and ends in a reboot. `phase` is initially `None` between claiming the run and receiving its
first phase. A new phase clears progress from the previous phase.

`UpgradeGeneration` is a monotonically increasing, process-local run identity allocated after a successful claim and
retained through every state of that run. Post-reboot marker success receives its own generation. The generation is not
sent over Wayland because every wire update is self-contained; it lets the compositor distinguish a duplicate terminal
snapshot from a later run with the same kind and outcome even when the watch channel coalesces the later run's
intermediate running states.

The bridge between the display-state channel and `Compositor` belongs in startup, matching the alarm-overlay bridge. The
upgrade service therefore does not depend directly on the compositor.

## `deck_upgrade_v1`

Create a protocol crate at `deck-upgrade-v1/`, beside `deck-alarm-v1`, `deck-settings-v1`, and `deck-screen-edge-v1`. It
owns the XML and generates both client and server bindings with `wayland_scanner`.

The version-one interface has no control requests other than `destroy`:

| Member                                                                                                   | Kind    | Meaning                                                       |
| -------------------------------------------------------------------------------------------------------- | ------- | ------------------------------------------------------------- |
| `destroy`                                                                                                | request | Destroy this protocol resource.                               |
| `started(kind)`                                                                                          | event   | Start a package-only or firmware-containing run.              |
| `phase(phase)`                                                                                           | event   | Report the current firmware or package phase.                 |
| `download_progress(downloaded_bytes_hi, downloaded_bytes_lo)`                                            | event   | Report downloaded bytes when the total is unknown.            |
| `download_progress_with_total(downloaded_bytes_hi, downloaded_bytes_lo, total_bytes_hi, total_bytes_lo)` | event   | Report downloaded and total bytes for determinate progress.   |
| `succeeded(remaining_ms)`                                                                                | event   | Report success and the remaining terminal-display lifetime.   |
| `failed(remaining_ms)`                                                                                   | event   | Report failure and the remaining terminal-display lifetime.   |
| `snapshot_done`                                                                                          | event   | Atomically commit the preceding full snapshot event sequence. |

The `kind` enum contains `packages` and `firmware`. The `phase` enum mirrors the existing backend phases:

- `firmware_downloading`
- `firmware_verifying`
- `firmware_applying`
- `package_realizing`
- `package_verifying`
- `package_building`
- `package_activating`

Wayland integer arguments are 32-bit. Byte counts are split into high and low words and reconstructed by helpers in the
server and client bindings. Separate known-total and unknown-total events preserve `Option<u64>` semantics without using
zero as a sentinel. `remaining_ms` is a bounded 32-bit duration, initially five seconds and reduced for replay; clients
use it as their terminal unmap deadline.

The protocol deliberately omits raw error text. Current backend errors are implementation-facing and not localized; the
overlay renders a stable generic failure message. A later protocol version may add a user-facing failure enum if the
product requires actionable on-device recovery instructions.

## Compositor behavior

The compositor owns a latest upgrade snapshot and the set of bound `deck_upgrade_v1` resources. Each command carries an
authoritative full snapshot, not a delta. One emission function is used for both ordinary fan-out and late-bind replay;
it always reconstructs a coherent sequence:

1. send `started(kind)`
2. send the current `phase`, when present
3. send the current progress or terminal event
4. send `snapshot_done`

This full emission is required even for an already-bound resource: a watch receiver may coalesce initial running and
first-phase values, and post-reboot success has no preceding run in the new compositor process. The client stages the
sequence and replaces its last coherent state only on `snapshot_done`, so re-sending `started` never exposes a partial
view.

When a generation first enters a terminal state, the cache records a five-second monotonic deadline. Terminal events
carry the remaining lifetime in milliseconds; a late binder receives only the remaining interval, and nothing is
replayed after expiry. Repeating a terminal command for the same generation must not extend its deadline; a different
generation always receives a fresh deadline even when kind and outcome are identical. Connected overlays unmap when
their received interval expires. This bounds stale success/failure replay without adding an upgrade-service timer.

## Overlay surfaces and stacking

`bmc-overlay-upgrade` exports two `SystemOverlay` implementations because `LayerConfig` is static.

### Firmware overlay

- full-screen layer-shell surface
- `Layer::Top`
- full input region
- visible for `UpgradeKind::Firmware` only
- opaque black background

Register the firmware overlay before the alarm overlay. Surfaces of the same rank paint in registration order, so the
later alarm surface stays reachable over an upgrade. A mapped firmware surface remains a full-screen blocker, which
suppresses scene gestures and preempts the settings tray through existing generic compositor policy.

### Package overlay

- bottom-right layer-shell surface sized to its compact content
- `Layer::Bottom`
- empty input region
- visible for `UpgradeKind::Packages` only
- opaque card over the live scene

The existing offline indicator already occupies the bottom-right `Background` layer. Putting upgrade progress on
`Bottom` gives deterministic stacking: the package card temporarily covers the offline chip rather than z-fighting with
it. It still reserves no exclusive zone and consumes no input, so widgets remain visible and interactive.

Both surfaces commit a NULL buffer and free their export buffers when hidden.

## Presentation

### Stable fullscreen parity

The firmware presentation reproduces the `bmc/stable-26.02` upgrade screens rather than merely borrowing their visual
language. The stable implementation uses a 1280 x 480 black canvas and a vertically centered column whose children are
also horizontally centered. Preserve these values:

| Element               | Stable value                                                         |
| --------------------- | -------------------------------------------------------------------- |
| Background            | `#000000`                                                            |
| Typeface              | Braiins Sans                                                         |
| Vertical spacing      | 15 px between column children                                        |
| Icon                  | 80 x 80 px                                                           |
| Icon block padding    | 40 px above the icon and 15 px below it                              |
| Title                 | 24 px, weight 700, `#FFFFFF`                                         |
| Supporting text       | 18 px, weight 400, `#8D8D8D`                                         |
| Download label        | 18 px, weight 400, `#FFFFFF`                                         |
| Download label inset  | 20 px left padding                                                   |
| Download progress bar | 80% of the screen width (1024 px), centered at x = 128 px, 7 px high |
| Downloaded-size text  | 18 px, weight 400, `#8D8D8D`                                         |
| Running/download icon | stable `tools.svg`, violet `#6B50FF`                                 |
| Success icon          | stable `main_checkmark.svg`, green `#13A454`                         |
| Failure icon          | stable `error_circle.svg`, red `#F95355`                             |

#### Asset vendoring

Vendor the icons from commit `6fde43f8cc883f9f9b74a3f2a408863b5eb6e8af` on `bmc/stable-26.02` into the new overlay
crate:

| Stable source                                     | New destination                                               |
| ------------------------------------------------- | ------------------------------------------------------------- |
| `bmc-display/ui/assets/images/tools.svg`          | `system-overlays/bmc-overlay-upgrade/assets/tools.svg`        |
| `bmc-display/ui/assets/images/main_checkmark.svg` | `system-overlays/bmc-overlay-upgrade/assets/checkmark.svg`    |
| `bmc-display/ui/assets/images/error_circle.svg`   | `system-overlays/bmc-overlay-upgrade/assets/error-circle.svg` |

Follow the settings-tray asset pattern: declare the SVG constants in `system-overlays/bmc-overlay-upgrade/src/icons.rs`
with `include_svg!` and register them with the renderer. The overlay crate's `Cargo.toml` must retain the vendored files
in the filtered Nix source with:

```toml
[package.metadata.nix]
include = ["assets/**"]
```

Treat this stanza as part of adding the assets: a Cargo build may find the working-tree files while the filtered Nix
build silently omits them without it.

Do not copy the stable font files into the overlay crate. The current renderer already embeds the equivalent
`assets/fonts/BraiinsSans-Regular.otf`, `BraiinsSans-SemiBold.otf`, and `BraiinsSans-Bold.otf`; select them through
`FontFamily::Sans` with the stable weights.

The three SVGs are intrinsically 80 x 80 and should be ported verbatim before making renderer-driven adaptations. The
stable download bar is Slint's standard `ProgressIndicator`; its colors are not Deck palette constants. Match its
rendered appearance during target visual validation instead of assigning an unsupported palette value in the design.

The stable screens have three distinct content arrangements:

- download: tools icon, 18 px regular download label, progress bar, and transferred-size text
- non-download progress: tools icon, 24 px bold stage title, and the gray safety message
- terminal: semantic 80 px icon and 24 px bold result title, with no supporting text

Use the new phase-specific labels in place of stable's generic `Updating...`, while preserving the corresponding stable
arrangement. Use the stable safety copy, `Keep the device plugged in and online during update`, for non-download
firmware progress. Download screens remain as sparse as stable and do not add that extra line.

The observable `Running { phase: None }` state uses the non-download arrangement with the title `Preparing update` and
the same safety copy for firmware. The compact form uses `Preparing update` without inventing progress. Both surfaces
map immediately, so this state must never render blank.

```text
+----------------------------------------------------------+
|                                                          |
|                       [tools icon]                       |
|                                                          |
|                 Downloading firmware 54%...              |
|       ===================---------------  54%            |
|                    82 MB of 151 MB                       |
|                                                          |
+----------------------------------------------------------+
```

### Compact package adaptation

The package presentation derives from the same assets, colors, font family, centered alignment, and three content
arrangements. It scales the icon, type, gaps, and bar down together inside a bottom-right card; it does not redesign the
states or introduce a second color scheme. The card contents are centered within the card even though the card itself is
anchored to the bottom-right of the display.

Exact compact dimensions belong to implementation-time visual validation on the 1280 x 480 output. Derive them from a
uniformly reduced version of the stable geometry and the established overlay spacing tokens rather than introducing
unsupported values in this design.

```text
                                      +----------------------+
                                      | [icon] Updating       |
                                      |        packages       |
                                      | Verifying packages   |
                                      | ---------------  *   |
                                      +----------------------+
```

Stage labels are:

| Phase                | Label                | Progress treatment                                      |
| -------------------- | -------------------- | ------------------------------------------------------- |
| Firmware downloading | Downloading firmware | Determinate when total is known; otherwise active       |
| Firmware verifying   | Verifying firmware   | Stable supporting copy in fullscreen                    |
| Package realizing    | Downloading packages | Determinate when total is known; otherwise active       |
| Package verifying    | Verifying packages   | Active in compact; stable supporting copy in fullscreen |
| Package building     | Building packages    | Active in compact; stable supporting copy in fullscreen |
| Package activating   | Activating packages  | Active in compact; stable supporting copy in fullscreen |
| Firmware applying    | Applying firmware    | Stable supporting copy in fullscreen                    |

The UI does not map stage count to a fabricated overall percentage. Progress events update the existing mapped surface
in place. The overlay marks content dirty only when the decoded state changes; the current backend throttle admits
download updates at most every 300 ms, comfortably supporting the ticket's once-per-second requirement without
rebuilding the surface or flickering.

## Storybook design validation

The overlay must expose the same state-driven render boundary to production and Storybook. Follow the existing alarm and
settings-tray shape:

```rust
pub fn render_upgrade(
    renderer: &mut dyn Renderer,
    size: (u32, u32),
    render_state: &mut UpgradeRenderState,
    view: &UpgradeView,
    now: Instant,
);
```

`UpgradeView` is the presentation-owned current state: upgrade kind, running phase and optional download progress, or a
terminal success/failure. The protocol handler updates the overlay's state; `SystemOverlay::render` derives an
`UpgradeView` and calls `render_upgrade`. Storybook constructs the same view directly and calls the same function. Do
not create a story-only renderer or drive stories through synthetic Wayland events.

`UpgradeRenderState` owns the retained `TreeUi`, registered icon handles, and render timestamp. Give every Storybook
cell its own thread-local render state, as the current alarm and settings-tray stories do, so icon registration and
retained-tree state cannot leak between examples. Keep tree construction separately testable as a pure function where
practical.

Add the upgrade gallery to `system-overlays/bmc-system-overlay/src/overlays.stories.rs`. Add `bmc-overlay-upgrade`
dependencies to both `bmc-storybook/Cargo.toml` and `bmc-storybook/stories/Cargo.toml`; the first builds static stories
and the second builds the hot-reload cdylib.

The gallery covers every visually distinct state and every phase label that must fit:

- firmware/fullscreen: preparing, known-total download, unknown-total download, firmware verification, nested package
  phases, firmware application, success, and failure
- package/compact: preparing, known-total download, unknown-total download, verification, building, activation, success,
  and failure

Render firmware cells at 1280 x 480. Render package cells at the compact surface's exported logical size, with the same
toggleable checkerboard backdrop used by other transparent overlays. Storybook validates the card itself; a framework
test validates that its real layer surface is anchored bottom-right, avoiding a second placement implementation used
only for previews.

Use `just storybook` for the initial visual comparison with stable and `just storybook-hot` while tuning geometry,
typography, progress treatment, and the compact layout. Storybook inspection is an explicit design acceptance gate, not
a substitute for render-tree, state, and layer-configuration tests.

Visual acceptance on 2026-08-06 fixes the compact package surface at 384 x 192 logical pixels. At that size, the 48 px
icon, 18 px title, 14 px supporting text, and 5 px progress track remain readable while the card occupies only the
bottom-right portion of the 1280 x 480 display. The runtime layer and Storybook cells use the same exported size.

## Lifecycle

### Package-only run

1. `started(packages)` maps the compact surface.
2. Phase and progress events update it in place.
3. `finished` becomes `succeeded`; the overlay shows success for the terminal interval sent by the compositor.
4. The compact surface unmaps and widgets remain in their existing scene.

### Firmware or combined run

1. `started(firmware)` maps the full-screen surface before the first phase.
2. Firmware and any nested package phases update the same surface.
3. `firmware_applying` remains visible until the compositor exits for reboot.
4. After reboot, startup establishes the compositor bridge, consumes the upgrade marker, and only then starts
   `autoupgrade_init`.
5. A successfully consumed marker publishes `Succeeded { kind: Firmware }` only when `BmcState::Operational`; the
   full-screen success state appears for the terminal interval sent by the compositor. Setup, factory-default, and Wi-Fi
   reconfiguration states consume the marker silently so the upgrade overlay cannot cover their setup experience.
6. The surface unmaps and normal startup/scene rotation continues.

### Failure

Any `UpgradeRunState::Failed` after a run is claimed becomes `Failed` with the remembered kind. It immediately replaces
the progress view, remains visible for the bounded terminal interval, and then unmaps. Firmware failure returns to the
scene after the existing widget restart guard restores widgets; package failure leaves the scene running throughout.

Failures that occur before a run is successfully claimed do not map an on-device overlay because no upgrade began.

## Verification

Tests should encode the following intent:

- the display projector preserves kind, every phase, optional totals, terminal state, and phase-driven progress reset
- forwarding display state does not change or consume the gRPC `UpgradeRunState` sequence
- protocol high/low helpers round-trip boundary `u64` values
- compositor fan-out reaches both resources and late bind replays a coherent ordered snapshot
- framework dispatch decodes every protocol event into the latest upgrade event
- each overlay ignores the other kind and maps only for its own kind
- known totals render determinate progress; unknown totals and non-download phases do not show fake percentages
- failure replaces progress immediately
- terminal states unmap at the compositor-owned terminal deadline
- terminal replay uses only the unexpired remainder and is suppressed after its compositor deadline
- Storybook invokes the same state-driven renderer as the production overlays and exposes all layout-significant states
- the package surface has an empty input region and non-`Background` stacking
- the firmware surface is a full-screen blocker and the alarm paints above it
- a post-reboot upgrade marker is removed once, produces firmware success only in `BmcState::Operational`, and is
  consumed silently in every non-operational state

The mock upgrade scenarios already exercise package-only, firmware-only, combined, and failure streams. Extend their
state assertions where useful, while keeping rendering and Wayland replay tests local to their owning crates.

Run the repository validation workflow after implementation; no test or formatter step may be silently skipped.

## Alternatives considered

### Dynamically reconfigure one layer surface

A single client could switch its size, anchors, layer, and input policy when the kind changes. This requires new runtime
configuration APIs in `SystemOverlay`, hosted and standalone remap handling, input-region transitions, render-target
resize/reallocation behavior, and corresponding framework tests. It has a broader blast radius than the feature needs.

### Draw the compact card into a transparent full-screen surface

This minimizes framework work but retains full-screen buffers during package upgrades. Static input policy must either
block the scene through transparent pixels or allow input through the modal firmware screen. It does not satisfy the
small passive-surface intent.

### Separate protocols for firmware and packages

Two protocols make placement explicit but duplicate the lifecycle vocabulary, compositor fan-out/cache plumbing, and
framework dispatch. The backend already represents both as one arbitrated upgrade run, so one domain protocol is the
more faithful boundary.

## Implementation plan

Implementation dependency: this repository consumes `/etc/upgrade_result` after startup but does not write it. Confirm
that the current external firmware-image build places the marker in an upgraded image. Treat marker production as an
explicit cross-repository acceptance dependency and verify it before the final on-target pass.

External dependency audit on 2026-08-07: the available BOS/OpenWrt image build does not create the marker, and searches
of the related local repositories found readers and test fixtures only. Post-reboot success therefore remains blocked
until the firmware-image build adds the writer. Verify the produced ii3 sysupgrade artifact contains the marker before
performing the on-target reboot-success pass.

### 1. Add the Wayland protocol boundary

- Create `deck-upgrade-v1/` following `deck-alarm-v1`: `Cargo.toml`, `build.rs`, `protocol/deck-upgrade-v1.xml`, and
  generated client/server modules in `src/lib.rs`.
- Add it to the root workspace and workspace dependencies, including the protocol XML in `[package.metadata.nix]`.
- Define the version-one events and enums from this design and shared helpers for splitting and joining `u64` byte
  counts.
- Test enum conversion and `u64` round trips at zero, the 32-bit boundary, and `u64::MAX`.

Checkpoint: both generated sides compile and the XML contains no BMC or renderer types.

### 2. Project upgrade runs into latest display state

- Add the presentation-neutral boundary types to `bmc/src/compositor.rs`: upgrade kind, phase, optional byte progress,
  monotonically increasing generation, and running/succeeded/failed state. Make them public `Clone + Eq` value types and
  convert explicitly from the crate-private `SystemUpgradePhase` and `UpgradeRunState` types.
- In `bmc/src/system_upgrade.rs`, add a dedicated watch-backed display-state service alongside the existing lossy LED
  `StateService`.
- Replace `forward_led_events` with one forwarding adapter that receives the claimed kind, publishes the initial running
  state before the first phase, allocates and retains one generation, projects every subsequent `UpgradeRunState`,
  retains kind across combined runs, clears progress on phase changes, and forwards the original stream item unchanged.
- Keep claim failures invisible: only install the forwarding adapter after `claim_upgrade` returns an
  `AvailableSystemUpgrade`.
- Expose a receiver from `SystemUpgradeService` for startup to subscribe without exposing its sender.
- Expose a narrow `publish_post_reboot_success()` method for the marker path; do not expose the watch sender.

Checkpoint: table-driven tests cover both kinds, generation retention and increment, every phase, known/unknown totals,
failure, finish, progress reset, and equality of the input and forwarded run sequences.

### 3. Relay and replay state through the compositor

- Extend the `bmc::compositor::Compositor` trait and mock with one full-state broadcast operation.
- Add `CompositorCommand::SetUpgradeState` in `bmc-openwrt/src/compositor/commands.rs` and handle it in
  `egl_compositor.rs`.
- Add `bmc-openwrt/src/compositor/upgrade.rs`, an upgrade cache/resource list mirroring the tested parts of `alarm.rs`:
  prune dead resources, remove on `destroy`, attach the five-second deadline on a new terminal snapshot, and use one
  full-snapshot emitter for both ordinary fan-out and late-bind replay.
- Store that state in `compositor/state.rs`, advertise the global during compositor initialization, and add the protocol
  dependency to `bmc-openwrt`.
- In `bmc/src/startup.rs`, enforce the startup order
  `display-to-compositor bridge -> upgrade marker consumption -> autoupgrade_init`. Step 6 adds the marker call between
  the bridge and autoupgrade initialization; no live run may publish before the marker decision completes.

Checkpoint: compositor tests verify an already-phased first observation, post-reboot terminal without a preceding run,
ordered replay, terminal remaining-time replay before expiry and suppression after expiry, replacement by a new run,
identical terminal outcomes with different generations, fan-out bookkeeping, and command mapping. Include a coalesced
second package failure whose intermediate running state is not observed; the mock verifies the startup bridge receives
projected states.

### 4. Decode upgrade state in the overlay framework

- Add the protocol dependency to `bmc-system-overlay` and bind it only when `SystemOverlay::uses_upgrade()` opts in.
- Add an upgrade callback carrying a coherent current state, plus client-side decoding in `surface.rs`.
- Stage `started`, `phase`, progress, and terminal events as a candidate and atomically publish it on `snapshot_done`.
  Do not use a one-event slot: a late-bind replay delivers the entire sequence.
- Treat a valid `started` as the only way to establish kind. Ignore phase, progress, and terminal events before it;
  invalidate a candidate containing an unknown kind, phase, or malformed ordering; preserve the last coherent snapshot
  when an invalid candidate reaches `snapshot_done`. Unknown values must never map the wrong surface.
- Deliver the snapshot before `tick` in both `hosted.rs` and `standalone.rs`, matching settings/alarm delivery order.
- Test every event, unknown wire enums, high/low byte reconstruction, replay coalescing, and opt-in binding.

Checkpoint: a test overlay observes the same final state in hosted and standalone delivery without depending on a live
compositor, including malformed, out-of-order, and unknown-enum sequences.

### 5. Build and visually accept the overlay crate

- Create `system-overlays/bmc-overlay-upgrade` with shared `UpgradeView`, `UpgradeRenderState`, icon registration, pure
  tree construction, and the public `render_upgrade` entry point.
- Vendor the three stable SVGs at the paths specified above and add `include = ["assets/**"]` to the crate's
  `[package.metadata.nix]`. Add the same direct SVG support dependencies used by the settings tray: `bmc-render-macros`
  and `bmc-wasm-sdk`.
- Add two standalone binaries, `bmc-overlay-upgrade-firmware` and `bmc-overlay-upgrade-packages`, each passing its fixed
  overlay implementation to `run_standalone`; keep the library shared.
- Implement `FirmwareUpgradeOverlay` with a full-screen `Top`/full-input surface and `PackageUpgradeOverlay` with an
  explicitly constructed `Bottom`/bottom-right/no-input surface. Do not use `LayerConfig::bottom_right`, which selects
  `Background` for the offline chip.
- Keep protocol-to-view state and the received terminal deadline in the overlay logic; keep layout generation free of
  clocks and Wayland objects.
- Add unit tests for kind filtering, visibility, dirty transitions, remaining-time behavior, preparing and stable
  labels, progress modes, icon selection, both standalone constructors, and both layer configurations.
- Add the upgrade gallery to `overlays.stories.rs` and both Storybook manifests, using independent render state per
  cell.

Checkpoint: run `just storybook-hot` and compare every gallery state with stable. Record the visually accepted compact
surface size before proceeding; the stories and runtime must use that same exported size.

### 6. Integrate both clients and reboot success

- Add the single `bmc-overlay-upgrade` crate to the root workspace members/dependencies and add the default
  `overlay-upgrade` feature and optional dependency to `bmc-wasm-host/Cargo.toml`.
- Register the firmware client before the alarm client so the later same-layer alarm paints above it; register the
  package client as a second instance of the same crate. Update the nearby stacking explanation because the new package
  surface deliberately sits above the `Background` offline chip.
- Replace `check_and_remove_upgrade_marker()` with a consume contract that distinguishes absent, consumed, and removal
  failure. Both the OpenWrt and mock managers must remove on success. After successful consumption, publish firmware
  success through `publish_post_reboot_success()` only when the current state is `BmcState::Operational`; consume
  silently in all other states. Do not synthesize a completed gRPC run.
- Cover absent, consumed, removal-failure, and two-successive-check behavior (`consumed`, then `absent`) as well as
  operational publication, silent non-operational consumption, strict bridge/marker/autoupgrade ordering, and overlay
  construction/ordering in startup and host tests.

Checkpoint: package state maps only the compact client, firmware state maps only the fullscreen client, an alarm remains
above firmware, an operational consumed marker produces one five-second firmware-success presentation, and a
non-operational consumed marker produces none.

### 7. Validate the complete path

- Exercise the existing mock scenarios for package-only, firmware-only, combined, and failure runs and inspect both
  Storybook galleries at 1280 x 480 geometry.
- Run the repository's focused `just` recipes during iteration, then finish with bare `just validate`; completion
  requires its final `validate: OK` marker.
- Build `.#bmc-openwrt-armv7-glibc-release` and the native Storybook/host outputs so filtered sources prove that both
  the protocol XML and vendored SVGs are present and cross-compilation catches compositor integration issues.
- Perform the final visual pass on target geometry, including widget interaction behind the package card, alarm
  preemption, hidden-overlay buffer release, and firmware success after reboot.

No step duplicates firmware/package upgrade business logic in the UI.

## Commit boundaries

Create one commit after each numbered step passes its checkpoint. Keep the protocol, state projection, compositor relay,
overlay-framework decoding, visual overlay/story implementation, and startup integration independently reviewable. Step
7 does not require an empty validation commit; use fixup commits against the step that introduced any validation
correction, and do not autosquash them without explicit authorization.
