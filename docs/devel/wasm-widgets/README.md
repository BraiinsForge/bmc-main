# WASM Widgets

Developer-facing notes for writing WASM widgets.

Start with [Best Practices](best-practices.md) before writing or changing any widget; the other documents go deeper on
individual topics.

## Documents

### [Best Practices](best-practices.md)

The conventions and host behaviors every widget should follow: the pure-logic/wasm split, formatting numbers through the
host so `number_format` applies, CSS-style alignment on both axes, stable typography, explicit handling of missing data,
and the pre-commit verification checklist. Read this first.

### [Params](params.md)

How a widget declares per-instance operator configuration in `manifest.json`, regenerates the typed `manifest_params.rs`
module, reads params from widget code, reacts to `on_params_update`, and exercises params in the testbed.

### [Credentials](credentials.md)

How a widget declares the accounts it needs in `manifest.json`, embeds the generated placeholders in outbound requests,
and reacts to `on_credentials_update`. The secret never reaches the widget: the host substitutes it as the request
leaves, and refuses to send a service-pinned credential anywhere else.

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

## Previews

`tools/render_shots.py <widget>` renders every viewport the widget's manifest supports to a PNG under
`.cache/screenshots/<widget>/`, populated with live data — it drives the capture binary's `--online` mode, where the
widget fetches its own data source (non-hermetic) and each shot waits for the response, with round faces masked to their
disc. Use it to eyeball a widget across sizes without the testbed. This is a preview aid only; baseline verification
stays [`just wasm::verify`](regression-testing.md).

## Profiling

Two ways to find where a widget spends time, for different questions:

- **CPU sampling** — `just wasm::profile <widget>` writes a samply profile to `combined.json.gz`; view it with
  `samply load <report>/combined.json.gz`. Widgets run inside the `wasmi` interpreter, so samples land on interpreter
  dispatch — this measures the *host*, not your widget's own functions.
- **Fuel profiling** — wrap sections in `profile::span("name")` and build with `--features profiling`
  (`CARGO_EXTRA_FLAGS=--features profiling`). Fuel is `wasmi`'s instruction count — hardware-independent, so the ratios
  you measure on a laptop hold on the Deck, and your sections show up as counter tracks in the same profile. Off by
  default — `span()` compiles to nothing, so the instrumentation can live in the widget source permanently. This is the
  one that answers "which part of *my* widget is expensive".

Compare two runs with `tools/perf_compare.py` (fuel-delta table — the metric for "did this optimization help"); inspect
a single report with `tools/perf_analyze.py`.

For why fuel beats wall-clock, the report format, and a worked ISS example, see the
[profiling-tooling devlog](../../devlogs/BDK-304-iss-position-wasm/profiling-tooling.md).
