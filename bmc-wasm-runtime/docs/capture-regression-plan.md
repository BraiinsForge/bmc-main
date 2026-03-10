# BDK-331: Headless Screenshot Capture & Visual Regression Testing

## Status: Core Implementation Complete

All base infrastructure is implemented and working. See `capture-extensions-plan.md` for remaining extensions.

### What's done

- **Capture binary** (`src/bin/capture.rs`) — headless EGL + pbuffer rendering, deterministic frame loop
- **Host-provided time** — `set_time()` API, 16ms fixed timestep, no `Instant::now()` in frame logic
- **Fixture system** — fetch interceptor/observer hooks via `RuntimeConfig`, fixture recording + replay
- **Event fixtures** — SSDP, mDNS, WebSocket, TCP socket, UDP broadcast recording and replay
- **Widget capture configs** — `capture.toml` with time, settle_delay, timeout, record_timeout, KV variants
- **KV variant matrix** — `[kv]` defaults, `[[variants]]` overrides, `--variant` CLI flag
- **Fetch cycle detection** — auto-stops recording when widget re-polls previously seen URLs
- **Capture orchestrator** — `tools/capture_run.py` runs all widgets × variants × sizes
- **Regression compare** — `tools/regression_compare.py` (odiff-based pixel diff)
- **GLES 2.0 capture context** — explicit `#version 100` shaders + GLES context request
- **Feature organization** — `capture` and `testbed` features, shared `host` deps
- **Clean architecture** — `RuntimeConfig` single constructor, generic hooks, no fixture-specific runtime state

### Recorded widget fixtures

| Widget         | Fixture data                    | Status |
| -------------- | ------------------------------- | ------ |
| calendar       | 4 iCal feeds + synthetic events | Done   |
| iss-position   | Position API + TLE API          | Done   |
| spacex-launch  | Launch Library 2 API            | Done   |
| hello-widget   | No fetches needed               | N/A    |
| stress-test    | No fetches needed               | N/A    |
| home-assistant | WebSocket auth + 63 entities    | Done   |
| media-control  | SSDP/mDNS/WS/HTTP               | Next   |

## Key Design Decisions

1. **New binary** (`capture`) — not bolting onto testbed. Single-purpose: load WASM, render one size, save frames, exit.

2. **One size per invocation** — the binary renders at a single resolution. Orchestrator loops over sizes.

3. **Host-provided time** — the runtime never calls `Instant::now()` or `Local::now()` for frame logic. `set_time()`
   sets `system_time` and `monotonic_ms` before each `render()`. Testbed sets from real clocks. Capture binary
   increments by fixed 16ms.

4. **Widget-colocated fixtures** — widgets ship a `fixtures/` directory with pre-recorded responses. Capture host serves
   from fixtures by default. Escape hatch: `--live-network`. Recording: `--record-fixtures`.

5. **Widget-colocated capture config** (`capture.toml`) — settle delay, explicit capture frames, start time, timeout,
   record_timeout, KV variants. CLI flags override config values.

6. **Auto-readiness via I/O completion** — no new host API needed. Capture renders until all I/O resolves, then waits
   `settle_delay` additional frames, then captures. With fixtures, readiness is reached within 2 frames.

7. **Generic runtime hooks** — the runtime provides `FetchInterceptor` and `FetchObserver` callbacks plus a
   `record_events` flag. All fixture-specific logic (file I/O, JSON format, cycle detection) lives in the capture
   binary. The runtime has zero knowledge of fixtures.

## Output Directory Structure

```
captures/<widget_name>/<variant>/<size>/frame_NNNN.png
```

When no `[[variants]]` defined, no variant subdirectory.

Sizes: `full` (1280x480), `large` (638x480), `medium` (638x238), `small` (317x238).

## CLI

```
capture <wasm_file> --size=<WxH> --output=<dir>
    [--frames=1,30,60]
    [--time=2026-01-15T12:00:00]
    [--live-network]
    [--record-fixtures]
    [--realtime]
    [--variant=<name>]
    [--list-variants]
```

## Files

All paths relative to `bmc-wasm-runtime/`.

| File                          | Purpose                                       |
| ----------------------------- | --------------------------------------------- |
| `src/bin/capture.rs`          | Headless capture binary                       |
| `src/bin/testbed.rs`          | Development testbed (windowed)                |
| `src/runtime_wasmi.rs`        | WASM runtime with interceptor/observer hooks  |
| `src/host_api.rs`             | Host state types                              |
| `src/runtime.rs`              | Module re-exports                             |
| `src/lib.rs`                  | Crate re-exports                              |
| `src/gpu/sphere.rs`           | GL sphere renderer (GLSL ES 1.00)             |
| `tools/capture_run.py`        | Capture orchestrator (all widgets × variants) |
| `tools/regression_compare.py` | odiff-based pixel comparison                  |
| `captures/.gitignore`         | Ignore current/diff dirs                      |
| `Cargo.toml`                  | Feature flags: `capture`, `testbed`, `host`   |
| `Makefile`                    | Build/capture/validate targets                |
