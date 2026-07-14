# EGL/Mesa Memory Allocation Proof

Captured: 2026-05-11

## EGL Initialization Memory (instrumented bmc-widget-flip-clock)

Log from `/var/log/bmc/flip-clock-widget.log`:

```
2026-05-11T12:34:06.314740Z  INFO bmc_widget::egl: [MEM] EGL init start: RssAnon = 1088 KB
2026-05-11T12:34:06.316705Z  INFO bmc_widget::egl: [MEM] After GPU open: RssAnon = 1096 KB (+8)
2026-05-11T12:34:06.439359Z  INFO bmc_widget::egl: [MEM] After GBM device: RssAnon = 1964 KB (+868)
2026-05-11T12:34:06.459212Z  INFO bmc_widget::egl: [MEM] After EGL display: RssAnon = 2028 KB (+64)
2026-05-11T12:34:06.482855Z  INFO bmc_widget::egl: [MEM] After EGL context: RssAnon = 3184 KB (+1156)
2026-05-11T12:34:06.483453Z  INFO bmc_widget::egl: [MEM] After make_current: RssAnon = 3184 KB (+0)
2026-05-11T12:34:06.489176Z  INFO bmc_widget::egl: [MEM] After glow context: RssAnon = 3192 KB (+8)
2026-05-11T12:34:06.489438Z  INFO bmc_widget::egl: [MEM] EGL init complete: RssAnon = 3192 KB (total +2104)
```

### EGL Init Breakdown (PROVEN)

| Stage              | RssAnon  | Delta         | Notes                  |
| ------------------ | -------- | ------------- | ---------------------- |
| Start              | 1,088 KB | -             | Process baseline       |
| GPU open           | 1,096 KB | +8 KB         | /dev/dri/renderD128 fd |
| **GBM device**     | 1,964 KB | **+868 KB**   | Mesa driver init       |
| EGL display        | 2,028 KB | +64 KB        | EGLDisplay wrapper     |
| **EGL context**    | 3,184 KB | **+1,156 KB** | GL state machine       |
| make_current       | 3,184 KB | +0 KB         | -                      |
| glow context       | 3,192 KB | +8 KB         | Rust GL bindings       |
| **Total EGL init** | 3,192 KB | **+2,104 KB** | ~2.1 MB                |

## Post-Rendering Memory (observed, not instrumented)

Current widget memory after rendering started:

```
PID 7636 (flip-clock):
  VmRSS:    23156 kB
  RssAnon:   7548 kB
  RssFile:  13132 kB
  RssShmem:  2476 kB

PID 7637 (flip-clock):
  VmRSS:    21892 kB
  RssAnon:   7492 kB
  RssFile:  12996 kB
  RssShmem:  1404 kB
```

### Post-Init Allocations (NOT PROVEN - source unknown)

| Metric  | At EGL init | After rendering | Delta         |
| ------- | ----------- | --------------- | ------------- |
| RssAnon | 3,192 KB    | ~7,500 KB       | **+4,308 KB** |

The +4.3 MB happens somewhere after EGL init. Possible sources:

- `Renderer::new()` - shader program creation (GL calls, not yet executed)
- `DigitTextures::new()` - texture uploads
- `Digit3DMeshes::new()` - mesh buffer uploads
- First `gl.clear()` call - triggers driver initialization
- First shader execution in `render_clock()` - Mesa compiles to GPU ISA
- GPU command buffer allocation

**Note**: We attempted to instrument these stages but the widget couldn't be tested due to deployment issues. The 4.3 MB
source remains unproven.

## Summary

### Proven allocations (EGL init): ~2.1 MB

| Component            | Size    | Source              |
| -------------------- | ------- | ------------------- |
| GBM device init      | ~870 KB | Mesa driver loading |
| EGL context creation | ~1.2 MB | GL state machine    |

### Unproven allocations (post-init): ~4.3 MB

Could be any combination of:

- Shader compilation/execution
- Texture/buffer uploads
- GPU command buffers
- Mesa internal state expansion

## Comparison: Shared vs Separate EGL Contexts

| Configuration                           | Total EGL heap         |
| --------------------------------------- | ---------------------- |
| 3 widgets, separate contexts            | ~23.7 MB               |
| 3 widgets, shared context (theoretical) | ~10-12 MB              |
| **Potential savings**                   | **~5-6 MB per widget** |

Shared context would save compiled shaders, Mesa threads, and context state. Each surface still needs framebuffers (~few
KB each).

## smaps Validation

The earlier `RssAnon`-only measurements were enough to identify EGL/Mesa as a suspect, but not enough to say whether the
cost was duplicated per widget or just looked large because of shared libraries. `smaps_rollup` answers that.

### 1 full-size extruded flip-clock

| Process           | Rss       | Pss       | Pss_Anon | Pss_File  | Pss_Shmem | Private_Clean | Private_Dirty |
| ----------------- | --------- | --------- | -------- | --------- | --------- | ------------- | ------------- |
| `bmc-openwrt`     | 26,016 KB | 21,092 KB | 8,652 KB | 12,404 KB | 36 KB     | 7,484 KB      | 8,688 KB      |
| `bmc-widget-flip` | 22,180 KB | 17,256 KB | 7,556 KB | 7,828 KB  | 1,872 KB  | 2,908 KB      | 9,428 KB      |

### 2 full-size extruded flip-clocks

| Process              | Rss       | Pss       | Pss_Anon | Pss_File | Pss_Shmem | Private_Clean | Private_Dirty |
| -------------------- | --------- | --------- | -------- | -------- | --------- | ------------- | ------------- |
| `bmc-openwrt`        | 17,552 KB | 15,280 KB | 8,624 KB | 6,632 KB | 24 KB     | 5,472 KB      | 8,648 KB      |
| `bmc-widget-flip` #1 | 14,656 KB | 11,498 KB | 7,552 KB | 2,242 KB | 1,704 KB  | 200 KB        | 9,256 KB      |
| `bmc-widget-flip` #2 | 14,492 KB | 11,362 KB | 7,556 KB | 2,210 KB | 1,596 KB  | 196 KB        | 9,152 KB      |

### Key conclusion

This proves the dominant per-widget cost is duplicated private memory, not shared file-backed Mesa code:

- per extruded widget `Private_Dirty` stays around `9.1-9.4 MB`
- per extruded widget `Pss_Anon` stays around `7.5 MB`
- shared Mesa text mappings still look large in RSS, but their per-widget PSS contribution is much smaller than the
  private dirty growth

In other words: the optimization target is not "RSS from big shared libraries", it is private EGL/GL/Mesa/widget state
allocated inside each widget process.

## What The Duplicated Private Memory Contains

### Extruded widget (`PID 2246`)

Top `Private_Dirty` regions:

| Region                        | Private_Dirty | Notes                                                                |
| ----------------------------- | ------------- | -------------------------------------------------------------------- |
| `[heap]`                      | 5,104 KB      | Main per-process heap; likely Mesa driver state + widget allocations |
| `/dev/dri/renderD128` mapping | 1,024 KB      | GPU/driver-private render allocation                                 |
| `/dev/dri/renderD128` mapping | 984 KB        | GPU/driver-private render allocation                                 |
| widget binary `r--p` mapping  | 912 KB        | Relocated/read-only data becoming process-private                    |
| unnamed anonymous `rw-p`      | 772 KB        | Additional private arena/state                                       |
| `libgallium` `r--p` tail      | 324 KB        | Process-private relocated/dirty library pages                        |

Rollup:

| Metric          | Value     |
| --------------- | --------- |
| `Pss`           | 15,352 KB |
| `Pss_Anon`      | 7,552 KB  |
| `Pss_File`      | 5,368 KB  |
| `Pss_Shmem`     | 2,432 KB  |
| `Private_Dirty` | 9,984 KB  |

### Flat widget (`PID 2247`)

Top `Private_Dirty` regions:

| Region                        | Private_Dirty | Notes                                             |
| ----------------------------- | ------------- | ------------------------------------------------- |
| `[heap]`                      | 5,044 KB      | Same heap baseline as extruded mode               |
| widget binary `r--p` mapping  | 912 KB        | Relocated/read-only data becoming process-private |
| unnamed anonymous `rw-p`      | 772 KB        | Additional private arena/state                    |
| `libgallium` `r--p` tail      | 324 KB        | Process-private relocated/dirty library pages     |
| `/dev/dri/renderD128` mapping | 128 KB        | Much smaller GPU/driver-private render allocation |
| `/dev/dri/renderD128` mapping | 96 KB         | Much smaller GPU/driver-private render allocation |

Rollup:

| Metric          | Value     |
| --------------- | --------- |
| `Pss`           | 12,814 KB |
| `Pss_Anon`      | 7,492 KB  |
| `Pss_File`      | 5,006 KB  |
| `Pss_Shmem`     | 316 KB    |
| `Private_Dirty` | 7,808 KB  |

### Interpretation

The duplicated private memory is not a single bucket:

- about `5 MB` is plain process heap in both flat and extruded modes
- extruded mode adds roughly `~2 MB` of extra `/dev/dri/renderD128` private-dirty mappings versus flat mode
- another `~1.0-1.5 MB` comes from process-private dirty pages in the widget binary, anonymous arenas, and Mesa
  relocation/state pages

That means the first serious optimization target remains per-widget render context/state sharing. A secondary target is
the extra render/GPU-private state needed by the extruded path.

## 3-Scene Stress Result

Attempting to start `3` full-size extruded flip-clocks caused the device to stall into OOM-like behavior during startup
before stable measurements could be captured. Even without full numbers, that is still useful evidence:

- memory scaling is large enough to make `3` heavy widgets unsafe on this device
- the per-widget private cost is high enough that scene count alone can push the system over the edge
