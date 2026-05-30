# WASM Widgets

Developer-facing notes for writing WASM widgets.

Start with [Best Practices](best-practices.md) before writing or changing any widget; the other documents go deeper on
individual topics.

## Documents

### [Best Practices](best-practices.md)

The conventions and host behaviors every widget should follow: the pure-logic/wasm split, formatting numbers through the
host so `number_format` applies, layout with flex instead of main-axis alignment, stable typography, explicit handling
of missing data, and the pre-commit verification checklist. Read this first.

### [Params](params.md)

How a widget declares per-instance operator configuration in `manifest.json`, regenerates the typed `manifest_params.rs`
module, reads params from widget code, reacts to `on_params_update`, and exercises params in the testbed.

### [System Settings](system-settings.md)

How a widget reads the host-defined deck-wide system snapshot: timezone, formatting settings, next alarm, and night
mode. Covers the fixed SDK accessors, update hook, and the testbed's System panel.

### [Regression Testing](regression-testing.md)

How a widget opts into the `wasm-regression` CI gate: the `capture/config.toml` toggle, recording fixtures (with params
and system snapshots baked into the header), setting baselines, verifying locally, and refreshing baselines after
intentional visual changes.

### [Display Geometry](display-geometry.md)

How a widget reads its assigned viewport and the active logical display through `widget_viewport()` and
`display_info()`. Covers rectangular versus round shape signals, platform viewports, testbed platform selection, and the
temporary compatibility fallback to `widget_size()` / `SizeVariant::{Small, Medium, Large, Full}`.
