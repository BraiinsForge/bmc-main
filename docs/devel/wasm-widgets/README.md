# WASM Widgets

Developer-facing notes for writing WASM widgets.

Start with [Best Practices](best-practices.md) before writing or changing any widget; the other documents go deeper on
individual topics.

## Documents

- [Best Practices](best-practices.md) — the conventions every widget follows.
- [Params](params.md) — per-instance operator configuration.
- [Credentials](credentials.md) — the accounts a widget needs, whose secrets only the host sees.
- [System Settings](system-settings.md) — the host's system-wide settings and their updates.
- [Unit Testing](unit-testing.md) — writing and running a widget's `#[wasm_test]`s.
- [Regression Testing](regression-testing.md) — the visual-regression gate and its baselines.
- [Memory](memory.md) — guest memory layout and the stack budget.
- [Display Geometry](display-geometry.md) — the widget's viewport and the display around it.
- [Timers](timers.md) — periodic background work, on screen or off.
- [Resource Handles](resource-handles.md) — the handles that keep background work alive.

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
  you measure on a laptop hold on a device, and your sections show up as counter tracks in the same profile. Off by
  default — `span()` compiles to nothing, so the instrumentation can live in the widget source permanently. This is the
  one that answers "which part of *my* widget is expensive".

Compare two runs with `tools/perf_compare.py` (fuel-delta table — the metric for "did this optimization help"); inspect
a single report with `tools/perf_analyze.py`.

For why fuel beats wall-clock, the report format, and a worked ISS example, see the
[profiling-tooling devlog](../../devlogs/BDK-304-iss-position-wasm/profiling-tooling.md).
