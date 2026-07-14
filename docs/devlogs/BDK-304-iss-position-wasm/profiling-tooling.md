# WASM fuel profiling + single-file report consolidation

Ticket: BDK-221 — Deck-side tooling tech debt (Task under the BDK-221 parent).

A side investigation that grew out of the BDK-304 ISS freeze hunt. This is profiling tooling, not ISS code.

## Why this was needed

We wanted to know *which part* of the ISS widget is expensive per frame, to chase the on-device freeze. The obvious move
— `just wasm::profile` + `samply load` — does not answer it: the widget runs inside the `wasmi` interpreter, so a CPU
sampler attributes everything to the interpreter dispatch loop. The crate breakdown bears this out — `wasmi` ~38%, the
rest egui/eframe/winit harness, and the widget's own functions are invisible (they are interpreter operands, not native
frames). The CPU profile measures the *host*, not the guest.

## The insight: measure in fuel, not time

`wasmi` already meters **fuel** (instruction count) for the runtime's execution budget. Fuel is a property of the
program, not the machine — the same code burns the same fuel on a desktop and on the Deck's ARM core. So if we attribute
fuel to named sections of the widget, the *ratios* we measure on a laptop hold on the device, and the interpreter no
longer hides the cost. We trade absolute wall-clock for hardware-independent instruction counts, which is the better
trade for "where does this widget spend its work".

## What was built

**SDK `profile` module** (`bmc-wasm-runtime/sdk/src/profile.rs`). A `profile::span("name")` RAII guard reads the fuel
remaining on entry and, on drop, ships `(name, fuel_spent)` to the host. Gated behind a `profiling` cargo feature: with
it off, `span()` returns a zero-sized no-op and the call sites compile to nothing. Verified — the production release
wasm contains zero references to the import or the section name strings, so the instrumentation can live permanently in
the widget source at no cost. Profiling is opt-in via the build, never a runtime branch.

**Host side.** Two imports: `host_fuel_remaining` (reads `Caller::get_fuel`) and `host_profile_section` (accumulates the
pair into `HostState.profile_sections`). The runtime exposes `take_profile_sections`, and the testbed drains it once per
frame, alongside the existing `FrameTimings`. The per-section fuel is averaged over the frames a section actually fired
in — cached-tree frames run no guest code and would otherwise drag the per-frame number down. `perf.json` gained a
`per_frame` block (timing + fuel arrays) plus the section averages.

**`perf_finalize.py` — the consolidation.** The report dir used to carry three artifacts that no single tool read whole:
`profile.json.gz` (samply's CPU profile), `symbols.json` (our addr2line sidecar, because samply cannot symbolize wasm),
and `perf.json` (the testbed's own frame data, which the CPU sampler never sees). `perf_finalize` folds all three into a
**new** `combined.json.gz` in Firefox Profiler format: the funcTable symbolized in place, and one **counter** track per
timing phase and per fuel section. The three sources are left untouched as provenance — the merge is non-destructive and
re-runnable.

**Getting the counter schema right.** This was the fiddly part. The Firefox Profiler format is versioned, and the
documented `RawCounter` type on `main` is *newer* than what samply 0.13 emits (it has a whole `display` block we would
have had to hand-roll). The ground truth is the writer samply actually uses — `fxprof-processed-profile` 0.8 — whose
serializer emits a simpler shape (`category/name/description/mainThreadIndex/pid/samples{length,count,number,time}` plus
optional `color`). We mirror that exactly rather than guessing. Both source links are pinned in the script header.
`count` is a per-frame delta the profiler accumulates, so a section's graph slope is its per-frame cost.

**Plumbing.** `perf_record.py` now runs finalize as the last step, so one `just wasm::profile` produces the combined
file end-to-end; the summary points `samply load` / analyze at it with absolute paths (the relative hint used to break
when run from the repo root). `perf_analyze` and `perf_compare` now read everything out of `combined.json.gz` — the
sidecars are no longer consumed. `perf_compare` had silently rotted (it `KeyError`ed on an `avg_fps` key the current
`perf.json` stopped emitting); rewriting it onto the combined counters fixed it and added a fuel-comparison table, which
is the metric we actually want when checking whether an optimization helped.

## What it found

On a 25 s / 4000-frame capture of the ISS widget:

- **`track`** (the ground-track → globe projection, whose source rebuild is cached for 10 s) spikes at startup and then
  at ~11 s and ~22 s — the two cache refreshes within the window, evenly spaced. Each refresh frame costs ~711k fuel,
  about **9× the ~80k steady frame**.
- **`propagate`** (SGP4 at the current time) spikes only at startup. It runs every frame and has nothing to refresh, so
  it is flat at ~30k fuel — confirming the per-frame SGP4 cost is steady and the earlier cache work paid off.
- Section medians match the independently-computed averages, a sanity check that the per-frame plumbing is correct.

So the cost structure is: cheap, uniform steady state, with one ~9× frame every 10 s from the `ground_track` rebuild.
That periodic spike is the concrete freeze candidate to carry to the device — an order-of-magnitude frame every 10 s is
exactly what reads as a stutter on slower hardware — and the obvious lever is to amortize the rebuild instead of doing
it in a single frame.

## Decisions made with Josef

- **Single combined file, sidecars as provenance.** The goal was one report artifact the tooling reads, without mangling
  samply's original output — hence a new file rather than rewriting `profile.json.gz` in place.
- **Counters, not markers.** Counters are the format's home for steady per-frame metrics. samply 0.13 predates the
  `line-rate` graph type, so the refresh shows as a cumulative step rather than a discrete needle — same information.
- **Instrumentation stays committed, compiled out when off.** No add/remove dance around profiling sessions.

## Status

Tooling complete and validated end-to-end against real captures. Pending the usual gate (`just validate` +
`just validate-wasm`) before the BDK-221 commit, kept separate from the BDK-304 widget commits.
