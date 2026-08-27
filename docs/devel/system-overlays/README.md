# System Overlays

System overlays are privileged, full-screen-or-corner UI surfaces that live *outside* the scene-widget model: WiFi setup
progress, an offline indicator, a swipe-from-top quick-settings panel, a full-screen firing-alarm screen,
upgrade-progress surfaces, and later notifications. They are `wlr-layer-shell` clients, not `deck_widget` widgets, and
they stack above the active scene.

The protocol rationale — why these are layer-shell surfaces rather than widgets, and why two small Wayland extensions
are vendored — is recorded in
[`../../devlogs/BDK-416/non-widget-ui-protocol-strategy.md`](../../devlogs/BDK-416/non-widget-ui-protocol-strategy.md).
This directory documents how the framework, compositor support, protocols, and concrete overlays are actually built.

## Why not widgets

`deck_widget` carries widget semantics: surface registration, compositor-provided size and viewport, per-instance
params, settings delivery, action requests, and a lifecycle tied to scene cycling. Scene placement itself is not in the
protocol — it stays owned by the scene configuration and the compositor. Overlays are not placed in scenes at all and
need z-ordering above scenes, edge anchoring, exclusive zones, and explicit input regions — exactly what
`wlr-layer-shell` models. Smithay ships server-side layer-shell support, so the compositor work is wiring and policy
rather than protocol design. Keeping shell concerns out of the widget protocol is the
[recorded decision](../../devlogs/BDK-416/non-widget-ui-protocol-strategy.md).

## Run modes

Every overlay crate always opens its own Wayland connection — from the compositor's view it is a separate client in both
modes. Only three things differ by mode:

|                    | Standalone                | Hosted (in `bmc-wasm-host`)                                |
| ------------------ | ------------------------- | ---------------------------------------------------------- |
| Wayland connection | its own                   | its own (a separate `wl_display` inside the host process)  |
| Renderer           | its own `FemtoVgRenderer` | borrows the host's shared renderer for one render callback |
| Event loop         | its own poll loop         | driven by the host main loop                               |

The expensive, memory-bearing resources — the GL context and the font cache — are shared in the hosted case; everything
else is identical code. For the current memory-constrained target the overlays compile into `bmc-wasm-host`; standalone
mode (`bmc_system_overlay::run_standalone`) stays a supported shape and is what each overlay's `src/main.rs` runs.

The production system config starts `bmc-wasm-host` as a compositor-owned command and supplies the compositor's
`WAYLAND_DISPLAY`. This keeps hosted overlays available when the scene configuration contains no WASM widgets. The
compositor supervises the host and stops it before tearing down the Wayland display; widget thins only wait for the host
and never control its lifetime.

The host lends its renderer to exactly one overlay at a time, only for the duration of that overlay's `render` callback,
the same single-user guarantee the host already relies on for WASM widget slots. See [`framework.md`](framework.md) for
the trait and the hosted driver, and [`../wasm-host/render-loop.md`](../wasm-host/render-loop.md) for the surrounding
host loop.

## Repository layout

The overlay crates are grouped under the top-level `system-overlays/` folder, mirroring `widgets/` and `widgets-wasm/`:

- `system-overlays/bmc-system-overlay` — the framework crate (the `SystemOverlay` trait, the layer-surface client, the
  render target, the hosted and standalone entrypoints).
- `system-overlays/bmc-overlay-device-info` — full-screen boot, setup and connect-info screens.
- `system-overlays/bmc-overlay-offline` — bottom-right offline indicator.
- `system-overlays/bmc-overlay-settings-tray` — swipe-from-top quick-settings panel.
- `system-overlays/bmc-overlay-alarm` — full-screen firing-alarm screen (Stop / Snooze).
- `system-overlays/bmc-overlay-upgrade` — upgrade progress: a full-screen firmware blocker and a passive package card,
  two `SystemOverlay` impls from one crate.
- `system-overlays/layer-shell-test-client` — a standalone layer-shell client used to exercise the compositor support
  directly.

The overlay protocol crates do **not** live under `system-overlays/`. They are shared between the compositor and the
overlay framework, so they sit at the workspace root beside `bmc-widget-protocol`, keeping protocol crates grouped and
the compositor free of a dependency into the overlay folder:

- `deck-screen-edge-v1` — top/bottom edge swipe-reveal (`deck_screen_edge_v1`).
- `deck-settings-v1` — compositor-relayed settings control and modal-overlay preemption (`deck_settings_v1`).
- `deck-device-info-v1` — one-way device-lifecycle and setup-progress state (`deck_device_info_v1`).
- `deck-alarm-v1` — firing-alarm ring/stop signalling and dismiss/snooze return path (`deck_alarm_v1`).
- `deck-upgrade-v1` — one-way upgrade-progress snapshots (`deck_upgrade_v1`).

See [`protocols.md`](protocols.md) for all five.

## The concrete overlays

| Overlay       | Crate                       | Layer        | Placement    | Input | Screen edge | Compositor IPC        |
| ------------- | --------------------------- | ------------ | ------------ | ----- | ----------- | --------------------- |
| Device info   | `bmc-overlay-device-info`   | `Bottom`     | full-screen  | full  | no          | `deck_device_info_v1` |
| Offline       | `bmc-overlay-offline`       | `Background` | bottom-right | none  | no          | no                    |
| Settings tray | `bmc-overlay-settings-tray` | `Overlay`    | full-screen  | full  | `Top`       | `deck_settings_v1`    |
| Alarm         | `bmc-overlay-alarm`         | `Top`        | full-screen  | full  | no          | `deck_alarm_v1`       |
| Upgrade (fw)  | `bmc-overlay-upgrade`       | `Top`        | full-screen  | full  | no          | `deck_upgrade_v1`     |
| Upgrade (pkg) | `bmc-overlay-upgrade`       | `Bottom`     | bottom-right | none  | no          | `deck_upgrade_v1`     |

The startup screen also binds `deck_upgrade_v1`: a boot that follows a package restart skips it, and a boot that follows
a firmware upgrade opens on the "Update Finished" screen it owns.

Stacking is by layer rank, except between same-rank surfaces, where registration order in `overlay_specs()` decides —
see [`compositor-integration.md`](compositor-integration.md).

The settings tray also *receives* modal-overlay preemption over `deck_settings_v1` (it retracts when a full-screen modal
overlay such as the alarm or the firmware blocker maps below it). Their behavior, data sources, and dismiss rules are in
[`overlays.md`](overlays.md).

## Enabling and disabling

Hosted overlays are gated twice:

- **Compile time** — each is an optional dependency behind a Cargo feature on `bmc-wasm-host` (`overlay-offline`,
  `overlay-device-info`, `overlay-settings-tray`, `overlay-alarm`, `overlay-upgrade`), all on by `default`. Dropping a
  feature removes the crate from the build entirely.
- **Runtime** — each compiled-in overlay maps to an env var `BMC_OVERLAY_<NAME>` (name uppercased, `-`→`_`, e.g.
  `BMC_OVERLAY_SETTINGS_TRAY`). Overlays are default-on; a value of `0`/`false`/`off` (case-insensitive) skips one — it
  is logged and never connected. Unknown values keep it on.

The env gate is a development convenience (silencing the startup overlay during compositor iteration), not a product
configuration mechanism. Both gates live in `bmc-wasm-host/src/overlays.rs` (`build_overlays`, `overlay_enabled`).

## Documents

- [Framework](framework.md) — the `bmc-system-overlay` crate: the `SystemOverlay` trait, the per-pass hosted driver and
  its render/wake gates, the layer-surface client, the double-buffered render target and GPU-fence discipline, the
  declarative tree UI, and the blit-only reveal animation.
- [Compositor integration](compositor-integration.md) — how the Smithay compositor advertises `wlr-layer-shell`,
  composites layer surfaces above the scene, tracks their buffers, evicts textures on a NULL-buffer unmap, hit-tests
  touch, suppresses scene-drag, and recognizes the edge-reveal gesture.
- [Protocols](protocols.md) — the five Wayland protocols `deck_screen_edge_v1`, `deck_settings_v1`,
  `deck_device_info_v1`, `deck_alarm_v1`, and `deck_upgrade_v1`: interfaces, requests, events, the responsibility split,
  and how the forked two diverge from their upstreams.
- [Overlays](overlays.md) — the five concrete overlays: what each shows, when it maps and dismisses, its data sources,
  and its platform gating.
