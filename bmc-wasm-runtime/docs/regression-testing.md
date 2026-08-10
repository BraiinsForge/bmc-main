# Visual Regression Testing — Internals

Pixel-level visual regression for WASM widgets using headless EGL capture and odiff comparison. This document covers the
internals: capture pipeline, nix wrapper, capture binary, fixture format, replay loop. For the widget-author workflow
(opt-in, recording fixtures, refreshing baselines), see
[`docs/devel/wasm-widgets/regression-testing.md`](../../docs/devel/wasm-widgets/regression-testing.md).

## Pipeline

`capture verify` (the CI entry point) runs three phases sequentially:

1. **Build** — compile all WASM widget examples (parallel)
2. **Capture** — headless EGL render per configured size × variant (sequential, GPU-bound)
3. **Compare** — pixel diff against baseline archives, generate HTML report with A/B media for failures

## CI integration

The `wasm-regression` job in `.gitlab-ci.yml` runs on every MR pipeline and blocks on failure. Manual trigger is
available on other pipelines (advisory).

On failure, inspect the CI log for the regression summary.

### Nix package

The `wasm-capture` nix package (`bmc-wasm-runtime/capture.nix`) wraps the `capture` binary with:

- **odiff** and **ffmpeg** on PATH
- **Mesa llvmpipe** for software rendering on headless CI runners:
  - `LIBGL_ALWAYS_SOFTWARE=1` — force software rasterizer
  - `__EGL_VENDOR_LIBRARY_FILENAMES` — point libglvnd at Mesa's EGL ICD
  - `EGL_PLATFORM=surfaceless` — headless EGL without X11/Wayland
- **Corefonts** via `FONTCONFIG_FILE` — deterministic font rendering in nix sandbox

```bash
nix build .#wasm-capture    # build the wrapped binary
nix run .#wasm-capture      # run directly
```

The wrapper is the local/dev entry point and is also the binary that the per-widget regression derivations in
`nix/wasm-regression.nix` call (with `--widget=<name>` + a single-widget `--workspace`). It is not, on its own, what CI
invokes.

## Capture binary

The `capture` binary is a separate tool from the testbed — single-purpose: load WASM, render, save frames, exit. Clap
subcommands:

| Command        | Purpose                                                  |
| -------------- | -------------------------------------------------------- |
| `run`          | Capture a single widget at a given size                  |
| `run-all`      | Build and capture all widget examples                    |
| `diff`         | Compare captures against baselines, generate HTML report |
| `verify`       | `run-all` + `diff` (CI entry point)                      |
| `preview`      | Generate mp4 preview from captured frames                |
| `set-baseline` | Update baselines.7z from current captures                |
| `init`         | Generate default capture/config.toml template            |

## Design decisions

1. **One size per invocation** — the binary renders at a single resolution. `run-all` orchestrates the size × variant
   matrix by spawning `capture run` as a subprocess for process isolation.

2. **Host-provided time** — the runtime never calls `Instant::now()` or `Local::now()` for frame logic. `set_time()`
   sets `system_time` (`DateTime<FixedOffset>`) and `monotonic_ms` before each `render()`. The timezone from the fixture
   header is preserved — widgets see the same local hour regardless of the host machine's timezone. Testbed uses real
   clocks; capture binary increments by fixed 16ms.

3. **Widget-colocated capture data** — each widget has a `capture/` directory containing `config.toml` (capture
   settings), `fixtures/` (recorded network data), and `baselines.7z` (compressed reference images).

4. **Auto-readiness via I/O completion** — capture renders until all I/O resolves. Each `Capture` event then renders
   `settle_delay` further frames (advancing the replay clock by 16ms each and injecting any events they cover), drains
   outstanding image decodes without advancing that clock, and shoots. With fixtures, readiness is reached within 2
   frames.

5. **Near-exact comparison** — odiff at threshold `0.1`, plus a `--max-diff-pixels` (8) budget per frame. Renders are
   deterministic for a given rasteriser — same WASM + same fixtures + same host-provided time (with timezone) =
   identical pixels — but rasterisers disagree on steep antialiased edges by a whole pixel of full contrast, which no
   colour distance can absorb. The budget covers that, so llvmpipe (CI, and Linux locally) and ANGLE (macOS) can share
   baselines; frames spending from it are reported as `tolerated`. Every run names the rasteriser it drew with.

## Fixture format

Fixtures are gzip-compressed JSONL (`.jsonl.gz`). Line 1 is the `FixtureHeader`, remaining lines are `TimelineEvent`
objects — one JSON object per line. Enables `zcat fixture.jsonl.gz | head -5` for quick inspection.

The header `time` field must include a timezone offset (e.g. `"2026-03-10T18:00:00+02:00"`). The recorder writes UTC
with offset; the replay parser requires it.

```
examples/<widget>/capture/fixtures/<size>.jsonl.gz
```

### Replay timeline

The capture binary maintains two clocks:

- **`monotonic_ms`** — wall-clock time passed to the widget. Advances every frame including during captures.
- **`fixture_ms`** — position in the original recording timeline. Freezes during capture events so network events stay
  causally aligned with user actions.

### Deferred event injection

Network events from fixtures are delivered via channels. If a channel doesn't exist yet (widget hasn't opened the
connection), the event is deferred and retried next frame:

```
inject_fixture_events(fixture_ms)  →  deliver_all_io()  →  render()  →  advance clocks
```

## Baseline format

Baselines are 7z archives per widget (`capture/baselines.7z`), compressed natively via `sevenz-rust2` with solid LZMA2.
One archive per widget; one frame per declared size inside.
