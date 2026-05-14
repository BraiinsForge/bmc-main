# Visual Regression Testing

Pixel-level visual regression testing for WASM widgets using headless EGL capture, odiff comparison, and CI integration
via GitLab.

## Quick start

```bash
# Local: capture all widgets and diff against baselines
make regression-test-all

# Single widget:
make regression-test EXAMPLE=calendar

# Locally reproduce the CI gate (per-widget derivations); on regression
# the per-widget build sandbox is preserved by --keep-failed, with
# captures + report.html under /tmp/nix-build-wasm-regression-<name>.drv-*/captures/
nix build --keep-failed -L --keep-going .#checks.x86_64-linux.wasm-regression
```

## Pipeline

`capture verify` (the CI entry point) runs three phases sequentially:

1. **Build** — compile all WASM widget examples (parallel)
2. **Capture** — headless EGL render per configured size × variant (sequential, GPU-bound)
3. **Compare** — pixel diff against baseline archives, generate HTML report with A/B media for failures

## CI integration

The `wasm-regression` job in `.gitlab-ci.yml` runs on every MR pipeline and blocks on failure. Manual trigger is
available on other pipelines (advisory).

On failure, the full `captures/` directory (HTML report + diff images) is published as a browsable artifact with 7-day
TTL.

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
`nix/checks.nix` call (with `--example=<name>` + a single-widget `--widgets-dir`). It is not, on its own, what CI
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

4. **Auto-readiness via I/O completion** — capture renders until all I/O resolves, then waits `settle_delay` additional
   frames, then captures. With fixtures, readiness is reached within 2 frames.

5. **Exact-match comparison** — odiff with threshold `0.0`. Exact match works because captures are deterministic renders
   — same WASM + same fixtures + same host-provided time (with timezone) = identical pixels. Baselines must be captured
   with the same renderer (llvmpipe in CI, or via `nix run .#wasm-capture` locally).

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

## Baseline management

Baselines are 7z archives per widget (`capture/baselines.7z`), compressed natively via `sevenz-rust2` with solid LZMA2.

To update baselines after intentional visual changes:

```bash
make update-baselines EXAMPLE=calendar
```

## Makefile targets

```bash
make capture EXAMPLE=x            # capture one widget at all sizes
make capture-all                   # capture all widgets
make update-baselines EXAMPLE=x   # capture + compress into baselines.7z
make regression-test EXAMPLE=x    # capture + diff one widget
make regression-test-all           # capture + diff all widgets
make preview EXAMPLE=x            # generate mp4 preview from captures
make preview EXAMPLE=x SIZE=full  # preview one size only
make record EXAMPLE=x SIZE=full   # record fixtures interactively
```
