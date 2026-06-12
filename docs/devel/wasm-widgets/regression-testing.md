# WASM Widget Regression Testing

Every widget under `widgets-wasm/` (and every example under `bmc-wasm-runtime/examples/`) can opt into pixel-level
visual regression. The CI job `wasm-regression` runs one nix derivation per opted-in widget, each rendering the widget
in a sandbox with headless EGL, then diffing the output against compressed baselines committed in the tree. Any pixel
drift fails the job and publishes an HTML report with A/B images as a build artifact.

This document is the widget-author view: what files a widget needs, how to record them, and how to refresh baselines
after an intentional visual change. For the internals (capture binary, Mesa wrapper, fixture format, replay loop), see
`bmc-wasm-runtime/docs/regression-testing.md`.

## Opt In

A widget is included in `wasm-regression` if and only if `capture/config.toml` exists next to its `Cargo.toml`.
`nix/checks.nix` enumerates `wasmWidgetCatalog`, filters on `hasCaptureConfig`, and emits one `wasm-regression-<widget>`
derivation per match. No registration step, no allowlist — drop the config in and the next pipeline picks it up.

```
widgets-wasm/<widget>/
  Cargo.toml
  manifest.json
  src/
  capture/
    config.toml              # opts the widget into wasm-regression
    fixtures/<size>.jsonl.gz # one per declared size
    baselines.7z             # compressed reference frames
```

`capture/baselines.7z` is tracked via Git LFS. Commit it like any other tracked file; the LFS smudge filter handles the
rest.

## Capture Config

`capture/config.toml` declares which sizes the widget is captured at and tells the capture binary where to find each
size's fixture.

```toml
# capture/config.toml
settle_delay = 5

[fixtures]
small  = "fixtures/small.jsonl.gz"
medium = "fixtures/medium.jsonl.gz"
large  = "fixtures/large.jsonl.gz"
full   = "fixtures/full.jsonl.gz"
```

`settle_delay` is the number of extra frames to render after all pending I/O resolves and before the first capture. Bump
it for widgets that need a few frames to settle visual state (fetch-then-format chains, animations winding down). The
default zero is fine for purely static widgets.

Only declare the sizes your widget actually supports. The capture binary skips sizes that don't have a fixture entry; CI
will not invent baselines for missing sizes.

## Fixtures Bake In The Whole Replay State

A fixture is gzip-compressed JSONL. Line 1 is the `FixtureHeader`; subsequent lines are timeline events. The header
captures everything that would otherwise be ambient state on a real device:

- `time` — ISO 8601 wall-clock start (with timezone offset), passed to the widget as `system_time`.
- `initial_params` — the manifest params snapshot the widget sees before the first `ParamDelivery` event. Recorded from
  whatever the operator selected in the testbed's Params panel at the moment they hit Record.
- `initial_system` — the deck-wide system snapshot (timezone, formats, night mode, …) at record start. Same provenance:
  testbed System panel at the moment of Record.
- `kv` — pre-seeded KV store entries.

Timeline events (`fetch`, `ws_message`, `click`, `params_delivery`, `system_delivery`, `capture`, …) drive everything
that happens after the first frame.

The practical consequence: if your widget reacts to params or system settings, set them in the testbed sidebars **before
pressing Record**. They become the fixture header's `initial_params` / `initial_system` and replay deterministically in
CI.

A blockheight excerpt for reference:

```
{"time":"2026-05-22T10:30:00+00:00",
 "initial_params":{"numbers_font_style":"bold","show_timestamp":true},
 "initial_system":{"settings":{"timezone":"UTC","time_format":"hour24", ...}}}
{"at_ms":0,"type":"fetch","method":"GET","url":"https://public-api.braiins.com/v2/blocks?...",
 "status":200,"body":{"text":"[{\"height\":900000,\"timestamp\":\"2026-05-22T10:30:00\"}]"}}
{"at_ms":500,"type":"capture"}
```

The fetch event stubs the live API call with a fixed JSON payload; without it, the widget would attempt real network in
the sandbox and fail the capture.

## Record A Fixture

Recording uses the testbed. `just wasm::record <widget> <size>` builds the widget, launches the testbed at that size,
and arms the recorder.

```bash
just wasm::record blockheight full
```

Workflow:

1. Pick the size first — the recipe argument selects which tile the testbed previews and which fixture filename it
   writes to.
2. Adjust the Params panel and the System panel to the state you want frozen into the fixture header.
3. Hit the Record button. From this point every fetch / websocket / param-update / system-update / user gesture is
   appended to the timeline. A `Capture` event marks where the visual snapshot is taken.
4. Stop recording. The testbed writes `capture/fixtures/<size>.jsonl.gz` next to the widget.

Repeat per declared size. Each size produces an independent fixture file; sizes do not share state.

If the widget makes outbound HTTP/WebSocket calls during the recording, the testbed records the real responses into the
timeline. Re-recording against a flaky endpoint produces flaky fixtures — once a recording is good, leave it.

## Set Baselines

Once the fixture is final, regenerate the baselines:

```bash
just wasm::update-baselines blockheight
```

This re-runs `wasm-capture` against the fixture (so the baselines are captured by the same renderer CI uses, not the
testbed), then compresses the resulting frames into `capture/baselines.7z`. Commit the new fixture and the new
`baselines.7z` together.

Baselines are pixel-exact: odiff runs with threshold `0.0`. The render must be deterministic — same WASM, same fixture,
same Mesa llvmpipe renderer. This is also why baselines must be regenerated through `just wasm::update-baselines`, not
by hand-copying testbed screenshots.

## Verify Locally

Two ways, in order of fidelity:

```bash
just wasm::verify blockheight
```

Fast loop: runs the capture binary directly against the current widget. Good for iterating on a baseline diff during
development.

```bash
nix build --keep-failed -L .#checks.x86_64-linux.wasm-regression-blockheight
```

What CI runs for a single widget. On regression, inspect the build log summary. Use the direct `just wasm::verify` loop
above when you need the generated capture files for inspection.

The aggregate `nix build -L --keep-going .#checks.x86_64-linux.wasm-regression` matches CI and runs every opted-in
widget in parallel; use it before pushing if you've touched shared rendering code.

## Updating After An Intentional Visual Change

When the widget's visuals change on purpose:

1. `just wasm::verify <widget>` — confirm the diff matches what you intended. On a diff, `wasm::verify` prints the path
   of a `report.html` (A/B images + diff overlays); open it in any browser.
2. `just wasm::update-baselines <widget>` — regenerate `baselines.7z`.
3. Inspect the resulting baselines (the wasm-capture binary saves them next to the fixtures during regeneration).
4. Commit `capture/baselines.7z` in the same logical change as the widget code that moved the pixels. Reviewers should
   see "this commit changed the widget *and* refreshed the baselines" together.

Fixtures only need refreshing when the widget's input contract changes — a new param key, a new fetched endpoint, a
different shape of API response. Cosmetic widget changes do not require re-recording.

## CI Failure

The `wasm-regression-<widget>` derivation fails the moment a single pixel differs. The CI job (`wasm-regression` in
`.gitlab-ci.yml`) uploads `captures/` as a 7-day artifact; download it from the failed job and open `report.html`.

Reasons a CI run flags drift that a local run did not:

- The Mesa llvmpipe renderer is pinned in nix but not in the testbed. `just wasm::verify` already uses the wrapped
  capture binary, so this matches CI. `cargo run --bin capture` directly does not.
- A widget reads ambient state the fixture doesn't pin (host clock, env var, file under `/tmp`). Add it to the fixture
  header or remove the dependency.
- Font rendering. Fonts come from the nix-pinned corefonts package via `FONTCONFIG_FILE`; locally-installed fonts are
  not used. Verify with the wrapped binary, not raw cargo.
