# Glyph Cache Performance Envelope

`bmc-render` owns an evictable Gray8 glyph atlas instead of femtovg's unbounded RGBA one: 10 normal 512×512 pages plus
one scratch page (2.75 MiB GPU cap), a glyph-cache metadata ceiling of 3 MiB, and a direct rasterization path for glyphs
above 92 px. Eviction is LRU under either cap; misses rasterize with swash and upload into the atlas during the draw.
This document records the measured envelope on hardware and the one accepted degenerate case.

The 3 MiB metadata figure covers only `GlyphCache::metadata_capacity_bytes`: the glyph cache's containers plus the
etagere and swash estimates. The paragraph layout cache is bounded separately to 448 entries and 12 288 resident glyphs.
At roughly 72 bytes per `PositionedGlyphInfo`, its glyph records consume about 864 KiB at the limit; line vectors,
entries, and hash-table storage bring the resident layout cache allowance to approximately 1 MiB. The two resident text
caches therefore allow approximately 4 MiB of app-owned metadata in total. A single layout larger than the resident
glyph limit is kept only as the current transient result, so its input-dependent allocation is outside that resident
estimate.

The `glyph-bench` harness is a separate, low-priority follow-up that will land after the cache itself. Until then, this
document records its results, but the `glyph-bench` feature is not available in-tree. The measurements use a quiesced
Deck: performance governor at 650 MHz, compositor stopped, frames ended by an EGL fence, release binaries, and
`GL_RENDERER` Vivante GC400 recorded in every trace. The frame budget is 16.7 ms (60 Hz). Each cache result uses five
independent 1 000-frame traces; p99 values pool the steady frames after scenario fill.

## Measured envelope

| Scenario                               | Cache                                | Unbounded atlas (master)                     | Verdict                           |
| -------------------------------------- | ------------------------------------ | -------------------------------------------- | --------------------------------- |
| Cold start (steady after 8-frame fill) | 12.6 ms wall; 5.8 ms CPU p99         | —                                            | within budget; see fill caveat    |
| Cold fill (first paint, 8 frames)      | 179–202 ms total; 21–44 ms per frame | —                                            | over budget — bounded, see caveat |
| Warm steady state                      | 16.7 ms wall; 4.8 ms CPU p99         | 25.0 ms p99 (100 % frames over)              | at budget; see thin margin        |
| Eviction churn (adversarial)           | 189 ms p99                           | 4.4 s p50, worst 9.7–11.7 s; dies at ~10 min | over budget — accepted, see below |

On the final retained-layout implementation, render-thread CPU time was 3.335 ms p50 and 4.836 ms p99 warm, and 4.198 ms
p50 and 5.803 ms p99 cold. Per-trace medians stayed within 3.318–3.355 ms warm and 4.171–4.229 ms cold across the five
runs. The warm scenario shaped no text after filling the cache; the cold scenario shaped exactly seven runs per frame.

Two caveats belong next to the passing rows:

- The warm margin is thin: 16.66 vs 16.7 ms on a text-heavy static screen. The scene's raw draw cost, not glyph work,
  consumes nearly the whole budget: rasterizations, uploads, evictions, and shaping are all zero. The reported misses
  are negative-cache hits for spaces, now reported separately from rasterizations.
- The cold row's 12.6 ms is the steady p99 *after* the fill: the harness splits the eight fill frames off and computes
  the percentile over the steady frames alone, so no dilution is involved and the fill is reported separately. Each of
  those frames measures 21–44 ms, so a screen transition still pays a handful of over-budget frames while the atlas
  fills, exactly as an uncached first paint always has.

## Accepted worst case: sustained full-miss churn

The churn scenario sweeps three text lanes through continuously changing font sizes (0.1 px steps, forever), so every
frame misses on 100+ glyphs and the working set never fits the 11 pages. Frame time attribution from the traces:
rasterizing and uploading the misses costs ~57 ms per frame even when nothing is evicted; heavy eviction pressure
roughly doubles that (evictions themselves are microseconds — they correlate with the larger glyphs that are expensive
to re-rasterize). The changing sizes also reshape text, unlike the static warm scenario. A miss that ultimately falls
back to scratch can first discard up to `MAX_EVICTIONS_PER_MISS` (64) cold entries without finding space in the normal
pages.

This failure mode is accepted rather than mitigated, for two reasons:

- It is not a regression. The unbounded atlas is over 40× worse at p50 on the same workload (4.4 s vs 104 ms) and its
  GPU memory grows monotonically — 69.7 → 152.9 MB GEM in 10 minutes with `MemAvailable` falling from 71 to 19 MB, until
  the process dies. Its churn figures come from short 40-frame traces because the process does not survive a long one.
  The cache reached exactly 2 883 584 bytes at frame 75 in every 1 000-frame trace, stayed flat thereafter, and dropped
  zero glyphs. It fails only the frame budget, never memory.
- No real widget renders this way. Realistic text (static screens, screen transitions, settling animations) is the warm
  and cold rows. A size animation converges a few frames after it settles; only a never-settling sweep across more
  distinct sizes than the atlas holds stays degenerate, and that is a synthetic construction.

The consequence of hitting it is degraded frame rate, not corruption: text still renders correctly, memory stays at the
cap, and the system recovers the moment the workload settles. If such a workload ever appears in practice, the known
remedy is a per-frame miss budget with nearest-size fallback (cap rasterization work per frame, draw stale-resolution
glyphs until the queue drains); it is deliberately not implemented until something real needs it.
