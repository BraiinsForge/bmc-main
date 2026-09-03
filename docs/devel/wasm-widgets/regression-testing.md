# WASM Widget Regression Testing

Every widget under `widgets-wasm/` (and every example under `widgets-wasm-examples/`) can opt into pixel-level visual
regression. The CI job `wasm-regression` runs one nix derivation per opted-in widget, each rendering the widget in a
sandbox with headless EGL, then diffing the output against compressed baselines committed in the tree. Drift past the
tolerances below fails the job and publishes an HTML report with A/B images as a build artifact.

This document is the widget-author view: what files a widget needs, how to record them, and how to refresh baselines
after an intentional visual change. For the internals (capture binary, Mesa wrapper, fixture format, replay loop), see
`bmc-wasm-runtime/docs/regression-testing.md`.

## Opt In

A widget is included in `wasm-regression` if and only if `capture/config.toml` exists next to its `Cargo.toml`.
`nix/wasm-regression.nix` enumerates `wasmWidgetCatalog`, filters on `hasCaptureConfig`, and emits one
`wasm-regression-report-<widget>` derivation per match. No registration step, no allowlist — drop the config in and the
next pipeline picks it up.

```
widgets-wasm/<widget>/
  Cargo.toml
  manifest.json
  src/
  capture/
    config.toml                 # opts the widget into wasm-regression
    fixtures/<dataset>.jsonl.gz # one per recorded dataset
    baselines.7z                # compressed reference frames
    README.md                   # optional, for a set worth explaining
```

A large set earns a `README.md` beside its config: what each dataset covers, and — the part a re-recording needs most —
which combinations were left out and why, so a gap reads as a decision rather than an oversight.
`widgets-wasm/formula-1/capture/README.md` is the worked example.

`capture/baselines.7z` is tracked via Git LFS. Commit it like any other tracked file; the LFS smudge filter handles the
rest.

## Capture Config

`capture/config.toml` names the datasets to replay and the targets each one runs against. A target is a
`<platform>:<viewport>` pair drawn from the platform catalog (`bmc-wasm-runtime/src/platform_catalog.rs`), so the
geometry, shape and DPI behind it are the device's own.

```toml
# capture/config.toml
settle_delay = 5

[fixtures.bmc100-small]
path = "fixtures/bmc100-small.jsonl.gz"
targets = ["bmc100:small"]

[fixtures.bmc100-full]
path = "fixtures/bmc100-full.jsonl.gz"
targets = ["bmc100:full"]
```

`settle_delay` is the number of extra frames rendered before **every** capture, ahead of the I/O drain that follows it.
Bump it for widgets that need a few frames to settle visual state (fetch-then-format chains, animations winding down).

Each settle frame advances the replay clock by 16 ms and delivers any timeline events that fall in it. So a widget whose
visuals track the clock — a countdown, a progress meter, a time-windowed chart — captures a *later* state with a settle
delay than without one, and the shift accumulates across a fixture's capture events. Changing it on such a widget
therefore changes the frames, and baselines must be refreshed with them. The default zero is right for a widget that is
ready as soon as its data lands. A dataset may override it with its own `settle_delay`, and seed extra KV with
`kv = { … }`.

Replay renders on the widget's own `request_frame_after` cadence, which is what the device host and the testbed do. A
widget that folds its data off the render loop — fleet-management folds on a ~1 s interval — therefore folds on the
schedule hardware uses, and replay samples the state hardware would.

Because a fixture carries no geometry of its own, one dataset can drive several targets:

```toml
[fixtures.common]
path = "fixtures/common.jsonl.gz"
targets = ["bmc100:full", "bmc100:large", "bmc100:medium", "bmc100:small"]
```

and one target can be driven by several datasets — give each a name that says what it holds. Frames land in
`<output>/<platform>/<viewport>/<dataset>/`.

Only declare targets your widget actually supports. Every target is validated against the catalog when the config loads,
and against the manifest's `supported_viewports` (DPI bounds included) when it is captured.

## Fixtures Bake In The Whole Replay State

A fixture is gzip-compressed JSONL. Line 1 is the `FixtureHeader`; subsequent lines are timeline events. The header
captures everything that would otherwise be ambient state on a real device:

- `time` — ISO 8601 wall-clock start (with timezone offset), passed to the widget as `system_time`.
- `initial_params` — the manifest params snapshot the widget sees before the first `ParamDelivery` event. Recorded from
  whatever the operator selected in the testbed's Params panel at the moment they hit Record.
- `initial_system` — the deck-wide system snapshot (timezone, formats, night mode, …) at record start. Same provenance:
  testbed System panel at the moment of Record.
- `kv` — pre-seeded KV store entries.

Timeline events (`fetch`, `ws_message`, `click`, `param_delivery`, `system_delivery`, `capture`, …) drive everything
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

### Response headers are allowlisted

A recorded `fetch` event carries the reply's response headers, so replay judges a body by the same `Content-Type` that
the live path would have judged it by. **Only headers named on an explicit allowlist are ever written to a fixture**,
namely `RECORDED_RESPONSE_HEADERS` in `bmc-wasm-runtime/src/unified_fixture.rs`:

```rust
pub const RECORDED_RESPONSE_HEADERS: &[&str] = &["content-type"];
```

Everything else is dropped at record time. This is a security boundary rather than a tidiness one: fixtures are
committed, this repository mirrors publicly, and a recording made against a real authenticated origin must never bake a
`Set-Cookie` into the tree.

Adding a header is deliberate. Extend the constant and update `recorded_headers_are_only_the_allowlist`, which pins the
set so an addition shows up in review rather than sliding in with whatever change wanted it. A fixture recorded before
its header was allowlisted simply does not carry one; absent means the origin sent none, never a parse failure.

## Record A Fixture

Recording uses the testbed. `just wasm::record <widget> <target> [name]` builds the widget, launches the testbed on the
target's platform, and arms the recorder.

```bash
just wasm::record blockheight bmc100:full
just wasm::record miner-info-geek bmm101:full public-down   # explicit dataset name
```

The target decides which platform the testbed opens, so `--platform` naming a different one is rejected rather than
silently overridden. Omit the dataset name and the testbed asks for it, with a button offering the conventional
`<platform>-<viewport>`; pass a third argument when one target holds several datasets and the names need to say them
apart.

A widget whose recordings differ by something the target cannot express should supply that name from its own recipe
instead of leaving it to the operator. The pool widget records against a sim scenario, so its wrapper appends one —
`bmc100-full-healthy`, leaving room for `-idle` and `-denied` beside it. Without that, every scenario writes the same
fixture per target and overwrites the last, which only a baseline diff would reveal.

Workflow:

1. Pick the target first — it selects both the platform and the tile the testbed previews.
2. Adjust the Params panel and the System panel to the state you want frozen into the fixture header.
3. Hit the Record button. From this point every fetch / websocket / param-update / system-update / user gesture is
   appended to the timeline. A `Capture` event marks where the visual snapshot is taken.
4. Stop recording. The testbed writes `capture/fixtures/<dataset>.jsonl.gz` next to the widget and adds the matching
   `[fixtures.<dataset>]` entry to `config.toml`, binding it to the target it was recorded at. Re-recording an existing
   dataset refreshes its data and leaves any other targets it already drives in place.

For a widget with a credential slot, bind an account in the sidebar's Credentials section and pass the real secret via
`just wasm::record <widget> <target> '' --secrets ../secrets.local.json` (JSON shaped `{"<slot>": {"<field>": "…"}}`,
kept gitignored at the repo root; the path is relative to `bmc-wasm-runtime/`, where `just` module recipes run) — the
recording session needs one real authenticated egress pass. The fixture stays secret-free by construction: recording
sees only the placeholder form, and substitution happens at the wire hop. Replay never needs the secret — recorded
fetches are served by method + URL before substitution would run.

Repeat per target you want covered. Each recording is an independent dataset; datasets do not share state. If several
targets can replay the same recording, bind one dataset to all of them instead of recording it again.

## Request URLs Must Be Stable

Replay serves fetches by method + URL, so a widget must produce a finite, reproducible set of URLs per logical state. A
URL built from the clock at full resolution does not. Replay follows the recorded timeline, but a poll firing a fraction
of a frame later crosses a second boundary and asks for a URL that was never recorded — a hermetic capture breach, which
re-recording only reshuffles.

So quantise any clock-derived part of a URL. The pool widget snaps its query window to `WINDOW_QUANTUM_SECS`, matching
its poll interval, so every poll within one interval reuses a single URL. Crossing a quantum boundary still mints a new
URL, so a recording that straddles one must be retaken; that fails loudly at record time, not in CI.

Worth doing whatever the capture story: a window end that follows the clock second by second makes every request unique,
and uncacheable upstream.

For a deterministic recording, record against a bmc-netsim cloud profile instead of the live API. Add
`--rewrite-url <api-origin>=http://127.0.0.1:<port>` to aim the widget's hard-coded base at the sim — both sides name an
origin, matched whole, so a lookalike host cannot pick the rewrite up — and a `--secrets` file whose slot carries a
loopback `allow_hosts` pin — the egress check judges the rewritten destination, so the account's own pin is what admits
it, and the token itself can be bogus. A widget that ships a sim owns both ends of this: the pool widget serves its
accounts with `just widgets::braiins-pool::run-netsim` and records against one of them with
`just widgets::braiins-pool::record <scenario> <target>`. The fixture keys stay canonical API URLs — the rewrite happens
after they are recorded — so sim-recorded fixtures replay exactly like live-recorded ones, without baking a real
account's numbers into the baseline.

If the widget makes outbound HTTP/WebSocket calls during the recording, the testbed records the real responses into the
timeline. Re-recording against a flaky endpoint produces flaky fixtures — once a recording is good, leave it.

## Set Baselines

Once the fixture is final, regenerate the baselines:

```bash
just wasm::update-baselines blockheight
```

This re-captures through the capture binary rather than the testbed, then compresses the resulting frames into
`capture/baselines.7z`. Commit the new fixture and the new `baselines.7z` together.

**Baselines carry the rasteriser that drew them.** The recipe builds the capture binary locally, and which rasteriser
that reaches is decided by the platform: Mesa llvmpipe on Linux — the same one CI uses, since the capture code asks EGL
for its software device and only falls back to the first device when there is none — and ANGLE's Metal backend on macOS.
Every run names it (`captured in 2.2s via …`), and so does the HTML report, so check that line before blessing anything.

Comparison is not pixel-exact. odiff runs at `--threshold 0.1`, a per-pixel colour distance that absorbs a differently
blended edge, and a frame may differ by up to `--max-diff-pixels` (8) beyond that. The budget exists because rasterisers
disagree on which side of a pixel centre a steep antialiased edge falls, and that flip is line colour against background
— full contrast, which no colour distance can absorb. Frames inside the budget are reported as `tolerated`, with their
pixel count and diff image, rather than passing quietly; anything past it fails.

That budget is what lets baselines drawn on one rasteriser verify on another. It is not a licence to hand-copy testbed
screenshots — those come from a different render path entirely.

## Verify Locally

Two ways, in order of fidelity:

```bash
just wasm::verify blockheight
```

Fast loop: runs the capture binary directly against the current widget. Good for iterating on a baseline diff during
development.

```bash
nix build -L --keep-going .#wasm-regression-report --out-link result
nix build -L .#checks.x86_64-linux.wasm-regression
```

What CI runs. The first command renders every opted-in widget in parallel and writes each one's verdict into
`result/passed/<widget>/`, `result/failed/<widget>/` or `result/broken/<widget>/`; the second turns a populated
`failed/` or `broken/` into a non-zero exit, replaying those widgets' logs. Run both before pushing if you've touched
shared rendering code.

The per-widget derivations are internal — there is no `.#checks…wasm-regression-<widget>` to build. To narrow the loop
to one widget, use `just wasm::verify <widget>` above, which also leaves the capture files in place for inspection.

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

Drift does not fail the `wasm-regression-report-<widget>` derivation. It records the widget under `failed/<widget>/` in
the report's output, and the separate `wasm-regression` check then fails because that directory exists. The ordering is
the point: the CI job uploads the whole tree as an artifact before the verdict is allowed to turn the job red. If
capture exits before writing `report.html`, the derivation records `broken/<widget>/` instead, with `verify.log` and any
capture files produced before the failure.

Download the artifact from the failed job and open `failed/<widget>/report.html` — it embeds the baseline, current and
diff images, and sits next to the A/B comparison media and `verify.log`. The job log also replays each failing widget's
`verify.log`, so the per-frame pixel counts are readable without downloading anything.

Failed and broken reports are valid Nix store paths. Retrying the GitLab job against the same commit therefore reuses
the per-widget results and reruns only the gate; it does not clear a suspected capture flake. The job has no
cache-bypass switch. Run `just wasm::verify <widget>` for a fresh local capture, or change an input and push to make CI
capture again. `nix build --rebuild .#wasm-regression-report` is not sufficient because it rebuilds only the aggregate
report, not its per-widget dependencies.

As an operational fallback for suspected flakes, CI administrators can remove the affected per-widget report path.
Remove aggregate paths that refer to it before retrying the job. Remove those paths from every runner, remote builder
and substituter serving the job; otherwise Nix can restore the same cached result. Prefer targeted `nix store delete`
operations. Check roots and referrers first, and avoid broad store garbage collection.

Reasons a CI run flags drift that a local run did not:

- The Mesa llvmpipe renderer is pinned in nix but not in the testbed. `just wasm::verify` already uses the wrapped
  capture binary, so this matches CI. `cargo run --bin capture` directly does not.
- A widget reads ambient state the fixture doesn't pin (host clock, env var, file under `/tmp`). Add it to the fixture
  header or remove the dependency.
- Font rendering. Fonts come from the nix-pinned corefonts package via `FONTCONFIG_FILE`; locally-installed fonts are
  not used. Verify with the wrapped binary, not raw cargo.
