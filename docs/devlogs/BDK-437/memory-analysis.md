# BDK-437 Memory Analysis

Device: Braiins Deck. Date: 2026-05-11.

## System Memory Overview

```
              total        used        free      shared  buff/cache   available
Mem:         246 MB       54 MB       86 MB       30 MB      105 MB      113 MB
```

## Top Memory Consumers

| Process     | RSS   | %MEM  | Notes                      |
| ----------- | ----- | ----- | -------------------------- |
| bmc-openwrt | 42 MB | 17.2% | Main compositor/controller |
| flip-clock  | 24 MB | 9.8%  | Widget process             |

### bmc-openwrt Memory Breakdown

| Category                   | Size    |
| -------------------------- | ------- |
| RssAnon (heap/stack)       | 10.8 MB |
| RssFile (mmap'd files)     | 28.5 MB |
| RssShmem (Wayland buffers) | 3 MB    |
| VmExe (executable code)    | 17 MB   |
| VmLib (shared libraries)   | 22.9 MB |

### flip-clock Memory Breakdown

| Category                   | Size   |
| -------------------------- | ------ |
| RssAnon (heap/stack)       | 7.6 MB |
| RssFile (mmap'd files)     | 14 MB  |
| RssShmem (Wayland buffers) | 2.5 MB |
| VmExe (executable code)    | 2.1 MB |
| VmLib (shared libraries)   | 22 MB  |

## GPU Memory (Etnaviv DRM/GEM)

Total GPU allocations: **34 MB**

Display: 600x1280 @ 16bpp

Notable allocations:

- 2.5 MB buffers (x2) - likely framebuffers (600x1280x32bpp with padding)
- 2.3 MB buffers (x2) - likely RGB framebuffers
- 1 MB buffer - texture or intermediate
- Many 128 KB buffers - command buffers, small textures

## Cache Breakdown

| Category       | Size  | Notes               |
| -------------- | ----- | ------------------- |
| Buffers        | 12 MB | Filesystem metadata |
| Cached (total) | 97 MB | File cache          |
| Active(file)   | 12 MB | Recently used       |
| Inactive(file) | 66 MB | Reclaimable         |
| Slab           | 22 MB | Kernel structures   |
| SReclaimable   | 6 MB  | Can be freed        |
| SUnreclaim     | 16 MB | Cannot be freed     |

## Shared Library Analysis

Libraries loaded by bmc-openwrt:

| Library                     | On-disk Size | Stripped? | Symbol Table |
| --------------------------- | ------------ | --------- | ------------ |
| libgallium-26.0.5.so (Mesa) | 18 MB        | No        | ~1.8 MB      |
| libstdc++.so.6.0.34         | 2.9 MB       | No        | ~0.7 MB      |
| libc.so.6                   | 2 MB         | No        | ~0.5 MB      |
| libzstd.so.1.5.7            | 0.8 MB       | -         | -            |
| libinput.so.10.13.0         | 0.5 MB       | -         | -            |
| libm.so.6                   | 0.4 MB       | -         | -            |
| libxkbcommon.so.0.13.1      | 0.4 MB       | -         | -            |
| libEGL.so.1.0.0             | 0.3 MB       | -         | -            |

Mesa is configured correctly with only etnaviv driver (`-Dgallium-drivers=etnaviv`), but the 18 MB libgallium still
contains:

- Full Gallium3D infrastructure
- etnaviv driver code
- Unstripped symbol tables (~1.8 MB)

## Optimization Opportunities

### 1. Strip Shared Libraries

Current nix builds have `separateDebugInfo = true` which extracts `.debug_*` sections but does NOT strip
`.symtab/.strtab`.

Potential savings: ~3 MB from symbol tables alone:

- libgallium: 1.8 MB
- libstdc++: 0.7 MB
- libc: 0.5 MB

Implementation: Add strip phase to nix derivations or use `dontStrip = false` with proper strip flags.

### 2. GPU Buffer Reduction

The 34 MB GPU allocation is substantial for a 600x1280 display. Potential areas:

- Reduce number of backbuffers (triple -> double buffering)
- Use 16bpp instead of 32bpp where possible
- Review texture allocation in widgets

### 3. Cache (No Action Needed)

The 97 MB cache is opportunistic file cache - the kernel will reclaim it when memory pressure increases. This is normal
and healthy behavior.

### 4. Binary Size (Already Optimized)

From previous work in this ticket:

- bmc-openwrt: 14M → 8.4M (-40%)
- flip-clock: 4.3M → 3.3M (-23%)

Achieved via: `strip = true`, `lto = true`, `codegen-units = 1`, `opt-level = "z"`

## Nix Store Paths

Mesa: `/nix/store/5gr5pxip9wvf9kr6dvwszpdzim2qap3m-mesa-armv7l-unknown-linux-gnueabihf-26.0.5` GCC libs:
`/nix/store/dz8iijacg14w5q8fccbbmkz47z4lnjn3-armv7l-unknown-linux-gnueabihf-gcc-15.2.0-lib` Glibc:
`/nix/store/3b7fhdfdi6p4s37y54wmbbbqi0m8bnkj-glibc-armv7l-unknown-linux-gnueabihf-2.42-51`

---

## Applied Optimizations

### Mesa Size Optimization (nix/pkgs/mesa/package.nix)

Added to meson flags:

```nix
"--buildtype=release"
"--optimization=s"
```

Added stripping:

```nix
stripAllList = [ "lib" ];
```

**Results:**

| Library              | Before       | After        | Savings           |
| -------------------- | ------------ | ------------ | ----------------- |
| libgallium-26.0.5.so | 18.8 MB      | 15.1 MB      | -3.7 MB           |
| libEGL.so.1.0.0      | 335 KB       | 247 KB       | -87 KB            |
| libGLESv2.so.2.0.0   | 63 KB        | 38 KB        | -25 KB            |
| **Total Mesa**       | **19.25 MB** | **15.38 MB** | **-3.9 MB (20%)** |

New Mesa path: `/nix/store/v88pa336hs9yraz2l2fcjx0s8fhswsf1-mesa-armv7l-unknown-linux-gnueabihf-26.0.5`

---

## Multi-Widget Memory Analysis (2026-05-11)

Tested with 5 flip-clock widgets (4 medium + 1 full across 2 scenes).

### Memory with 5 Widgets

| Resource   | Used   | Total  | Notes                   |
| ---------- | ------ | ------ | ----------------------- |
| CMA (GPU)  | ~56 MB | 128 MB | 62 MB in GEM objects    |
| System RAM | ~73 MB | 246 MB | OOM on scene transition |

### Per-Process RSS

| Process         | RssAnon | RssFile | RssShmem | Total  |
| --------------- | ------- | ------- | -------- | ------ |
| bmc-openwrt     | 8.8 MB  | 7.5 MB  | 1.7 MB   | 18 MB  |
| flip-clock (×4) | 7.5 MB  | 4 MB    | 1.7 MB   | ~13 MB |

### GPU GEM Objects (62 MB total)

- 2.5 MB buffers ×6 - framebuffers
- 1 MB buffers ×9 - texture atlases
- 640 KB buffers ×12 - widget surfaces
- 128 KB buffers ×40+ - command buffers

### Issues Found

1. **Widget heap too large (7.5 MB each)**: Font loaded twice per widget (digits.rs + digits3d.rs)
2. **Compositor texture leak**: Textures not cleaned up when widget crashes/disconnects
3. **Scene transitions cause OOM**: Both scenes' widgets active during swipe

---

## Applied Optimizations (Session 2)

### 1. Compositor Texture Cache Cleanup (Bug Fix)

**Problem:** When a widget crashes or disconnects, its buffers and textures were not cleaned up, causing:

- "No cached texture for buffer" log spam
- Orphaned GPU textures accumulating
- Memory pressure leading to more OOMs (vicious cycle)

**Fix:** Added `drop_widget_buffers()` method to clean up widget buffers and invalidate textures on disconnect.

**Files:**

- `bmc-openwrt/src/compositor/state.rs` - added `drop_widget_buffers()` method
- `bmc-openwrt/src/compositor/egl_compositor.rs` - call cleanup in disconnect handler

### 2. Shared Font in Flip-Clock

**Problem:** Each flip-clock widget loaded the font twice:

- `digits.rs:28` - for 2D textures
- `digits3d.rs:59` - for 3D meshes

Each `FontRef::try_from_slice()` maintains internal caches (2-4 MB).

**Fix:** Created shared `font.rs` module with `LazyLock<FontRef<'static>>` used by both modules.

**Files:**

- `widgets/flip-clock/src/font.rs` - new shared font module
- `widgets/flip-clock/src/digits.rs` - use shared FONT
- `widgets/flip-clock/src/digits3d.rs` - use shared FONT
- `widgets/flip-clock/src/main.rs` - added font module

**Savings:** 2-4 MB per widget (within same process)

---

## Future Optimization Opportunities

### Cross-Process Memory Sharing

Research findings for sharing memory between widget processes:

| Approach               | Feasibility | Savings/Widget | Complexity |
| ---------------------- | ----------- | -------------- | ---------- |
| memfd font sharing     | HIGH        | 50-100 KB      | Low        |
| DMA-BUF texture export | MEDIUM-HIGH | 1.3 MB         | Medium     |
| Single widget process  | MEDIUM      | 2-3 MB         | High       |

**DMA-BUF Texture Sharing:**

- `EGL_MESA_image_dma_buf_export` is supported on Etnaviv
- Compositor could pre-render digit textures and export as DMA-BUF
- Widgets import shared textures instead of creating their own
- Requires Wayland protocol extension

**Single Process Mode:**

- Run all flip-clocks in one process with multiple Wayland surfaces
- Eliminates all duplication but breaks widget isolation model

### Depth Buffer Optimization (Implemented)

Made depth renderbuffer allocation conditional on widget needs. Flat mode widgets no longer allocate depth buffers.

**Savings:** ~1.2 MB CMA memory per widget in flat mode.

---

## Session 4: smaps-Based Attribution (2026-05-11)

### Controlled full-size flip-clock runs

Measured on a Deck with `/proc/<pid>/smaps_rollup`.

#### 1 full-size extruded widget

| Process           | Rss       | Pss       | Pss_Anon | Pss_File  | Pss_Shmem | Private_Clean | Private_Dirty |
| ----------------- | --------- | --------- | -------- | --------- | --------- | ------------- | ------------- |
| `bmc-openwrt`     | 26,016 KB | 21,092 KB | 8,652 KB | 12,404 KB | 36 KB     | 7,484 KB      | 8,688 KB      |
| `bmc-widget-flip` | 22,180 KB | 17,256 KB | 7,556 KB | 7,828 KB  | 1,872 KB  | 2,908 KB      | 9,428 KB      |

#### 2 full-size extruded widgets

| Process              | Rss       | Pss       | Pss_Anon | Pss_File | Pss_Shmem | Private_Clean | Private_Dirty |
| -------------------- | --------- | --------- | -------- | -------- | --------- | ------------- | ------------- |
| `bmc-openwrt`        | 17,552 KB | 15,280 KB | 8,624 KB | 6,632 KB | 24 KB     | 5,472 KB      | 8,648 KB      |
| `bmc-widget-flip` #1 | 14,656 KB | 11,498 KB | 7,552 KB | 2,242 KB | 1,704 KB  | 200 KB        | 9,256 KB      |
| `bmc-widget-flip` #2 | 14,492 KB | 11,362 KB | 7,556 KB | 2,210 KB | 1,596 KB  | 196 KB        | 9,152 KB      |

#### 3 full-size extruded widgets

The device entered OOM-like startup behavior before stable measurements could be captured. This still establishes that
`3` heavy fullscreen extruded widgets are not a safe operating point on the current memory budget.

### What changed in the diagnosis

Before `smaps`, the biggest visible number was `RssFile`, which left open the possibility that large shared Mesa
mappings were the main problem. `smaps` rules that out as the primary target:

- the widget's duplicated `Private_Dirty` stays around `9.1-9.4 MB`
- the widget's duplicated `Pss_Anon` stays around `7.5 MB`
- `Pss_File` drops sharply when running two identical widgets, which confirms much of the library cost is shared rather
  than duplicated

The right interpretation is:

- `RSS` overstates the per-widget cost because it counts shared mappings in each process
- `PSS` and `Private_*` show the true optimization target
- the dominant duplicated cost is private EGL/GL/Mesa/widget state

### Private memory breakdown

#### Extruded widget

Top `Private_Dirty` regions:

| Region                        | Private_Dirty | Notes                                       |
| ----------------------------- | ------------- | ------------------------------------------- |
| `[heap]`                      | 5,104 KB      | Main per-process heap                       |
| `/dev/dri/renderD128` mapping | 1,024 KB      | GPU/driver-private render allocation        |
| `/dev/dri/renderD128` mapping | 984 KB        | GPU/driver-private render allocation        |
| widget binary `r--p` mapping  | 912 KB        | Process-private relocated/read-mostly pages |
| unnamed anonymous `rw-p`      | 772 KB        | Additional arena/state                      |
| `libgallium` `r--p` tail      | 324 KB        | Process-private dirty library pages         |

Rollup:

| Metric          | Value     |
| --------------- | --------- |
| `Pss`           | 15,352 KB |
| `Pss_Anon`      | 7,552 KB  |
| `Pss_File`      | 5,368 KB  |
| `Pss_Shmem`     | 2,432 KB  |
| `Private_Dirty` | 9,984 KB  |

#### Flat widget

Top `Private_Dirty` regions:

| Region                        | Private_Dirty | Notes                                             |
| ----------------------------- | ------------- | ------------------------------------------------- |
| `[heap]`                      | 5,044 KB      | Same baseline heap shape as extruded              |
| widget binary `r--p` mapping  | 912 KB        | Process-private relocated/read-mostly pages       |
| unnamed anonymous `rw-p`      | 772 KB        | Additional arena/state                            |
| `libgallium` `r--p` tail      | 324 KB        | Process-private dirty library pages               |
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

### Updated optimization direction

1. **Shared per-widget render state is still the highest-value target.** The duplicated cost is mostly private heap and
   render-driver state, not library text mappings.
2. **Extruded mode adds material GPU-private memory over flat mode.** The extra `/dev/dri/renderD128` dirty mappings are
   about `~2 MB`.
3. **Shared-library size work still matters for image size and cold-start RSS, but it is not the main fix for
   multi-widget OOMs.**
4. **Wayland/GPU shared memory is the second target after private state.** Extruded `Pss_Shmem` is about `1.6-2.4 MB`
   per widget.

---

## Session 5: 3-Scene OOM Evidence (2026-05-11)

### What actually failed

After deploying `3` fullscreen `extruded` flip-clock scenes, the device could start all three widget processes and reach
steady state. The failure happened later during scene navigation, not during initial spawn:

- scene change `0 -> 1` across 3 scenes visibly lagged for about `1.1s`
- later navigation to the third scene triggered a real kernel OOM kill

Relevant `dmesg` excerpt:

```text
32768 pages cma reserved
oom-kill:constraint=CONSTRAINT_NONE,...,global_oom,task=bmc-widget-flip,pid=4790
Out of memory: Killed process 4790 (bmc-widget-flip) total-vm:53776kB,
anon-rss:7548kB, file-rss:60kB, shmem-rss:2476kB
```

This is important because it rules out the narrow theory "only a DRM/CMA allocation failed". The kernel entered **global
OOM** and killed one widget process.

### Why `MemAvailable` looked misleading

At the same time, the device still reported tens of megabytes of available memory in `/proc/meminfo`:

| Metric         | Value                             |
| -------------- | --------------------------------- |
| `MemTotal`     | `245976 kB`                       |
| `MemAvailable` | `~83-91 MB` during the experiment |
| `CmaTotal`     | `131072 kB`                       |
| `CmaFree`      | `~36-82 MB` depending on phase    |

The key fact is that **half of the machine's RAM is reserved for CMA**:

- `65536` total RAM pages = `256 MB`
- `32768` CMA pages = `128 MB`

So the machine does not behave like a generic `256 MB` Linux box. The effective non-CMA RAM budget is much smaller, and
that makes the duplicated per-widget private memory much more dangerous during transitions.

### Live 3-widget measurements

With `3` fullscreen `extruded` widgets started successfully, we observed:

| Metric         | Value            |
| -------------- | ---------------- |
| `MemAvailable` | `82900 kB`       |
| `CmaFree`      | `37124 kB`       |
| `Shmem`        | `48668 kB`       |
| DRM GEM total  | `54718464` bytes |

After a later scene transition and recovery:

| Metric         | Value            |
| -------------- | ---------------- |
| `MemAvailable` | `84288 kB`       |
| `CmaFree`      | `76392 kB`       |
| `Shmem`        | `60188 kB`       |
| DRM GEM total  | `66514944` bytes |

This shows two things:

1. **The problem is transient as well as steady-state.** Looking only after recovery can miss the bad moment.
2. **Both private process memory and GPU/shared-memory pressure matter.** CMA pressure alone did not produce a
   standalone allocation error, but the `128 MB` CMA reserve reduced the effective general RAM budget enough that the
   system still hit global OOM under transition pressure.

### Updated conclusion

The failure mode is now best described as:

- duplicated per-widget private EGL/GL/Mesa state consumes too much normal RAM
- shared/GPU/shmem pressure grows during multi-scene transitions
- `128 MB` CMA reservation cuts deeply into the effective non-CMA RAM budget
- the combined pressure triggers **global OOM**, and the kernel kills a widget

So the optimization target is still correct:

1. reduce duplicated per-widget private render state first
2. reduce extruded-path GPU/shared-memory pressure second
3. evaluate whether the `128 MB` CMA reserve is oversized for this workload

### Practical budget estimate

The important point is that the device does **not** behave like a generic `256 MB` Linux machine for this workload:

- total RAM: `245976 kB` (`~240 MiB`)
- CMA reserve: `131072 kB` (`128 MiB`)
- effective non-CMA RAM budget: `114904 kB` (`~112 MiB`)

Using the measured steady-state process footprints:

| Component                          | Approximate footprint | Evidence                                                                       |
| ---------------------------------- | --------------------- | ------------------------------------------------------------------------------ |
| Base OS + daemons                  | `~15-25 MB`           | residual system usage outside compositor/widgets in process tables and meminfo |
| `bmc-openwrt` compositor           | `~17-26 MB RSS`       | measured across 1/2/3 widget runs                                              |
| One fullscreen extruded flip-clock | `~21-22 MB RSS`       | measured repeatedly across runs                                                |

Estimated steady-state budget for `3` fullscreen extruded widgets:

| Bucket                 | Estimate         |
| ---------------------- | ---------------- |
| Base OS + daemons      | `~15-25 MB`      |
| Compositor             | `~18-25 MB`      |
| 3 extruded widgets     | `~63-66 MB`      |
| **Total steady-state** | **`~96-116 MB`** |

That means the system is already operating at or near the effective `~112 MB` non-CMA budget even before any scene
transition overhead.

### Estimated transition overhead

From live measurements during the `3`-widget experiment:

| Metric        | Before/steady      | After transition pressure | Delta      |
| ------------- | ------------------ | ------------------------- | ---------- |
| DRM GEM total | `54,718,464` bytes | `66,514,944` bytes        | `+11.8 MB` |
| `Shmem`       | `48,668 kB`        | `60,188 kB`               | `+11.5 MB` |

These deltas are not a full accounting of every transient allocation, but they are large enough to explain the observed
lag and OOM tendency on their own.

Practical interpretation:

- the system is already close to the cliff in steady state
- a scene change only needs an additional `~10-15 MB` burst to cross it
- the kernel then has to reclaim quickly inside a memory topology where `128 MB` is CMA-reserved
- if reclaim cannot keep up, the kernel enters **global OOM** and kills a widget

So the key failure pattern is not "we ran out of memory only after adding a huge new scene". It is:

1. `3` fullscreen extruded widgets put the system near the effective budget
2. scene transition adds `~10-15 MB` of extra GPU/shared-memory pressure
3. the remaining non-CMA headroom is too small to absorb that burst safely

---

## Session 3: Mesa Thread Analysis (2026-05-11)

### Thread Breakdown

Each widget spawns 3 threads (from `/proc/PID/task/*/comm`):

| Thread            | Purpose                    | Stack Size    |
| ----------------- | -------------------------- | ------------- |
| `bmc-widget-flip` | Main thread (Wayland loop) | 132 KB actual |
| `bmc-wid:disk$0`  | Mesa disk cache            | 8 MB virtual  |
| `bmc-widget:sh0`  | Mesa shader compiler       | 8 MB virtual  |

### Memory Map Analysis

Key anonymous regions per widget:

| Region            | Size         | Content                            |
| ----------------- | ------------ | ---------------------------------- |
| Heap              | 6.4 MB       | Mesa driver state, EGL context     |
| Thread stack 1    | 8 MB virtual | Disk cache thread                  |
| Thread stack 2    | 8 MB virtual | Shader compiler thread             |
| Shader cache mmap | 2 MB         | `~/.cache/mesa_shader_cache/index` |

### Applied: Disable Mesa Shader Cache

Added `MESA_SHADER_CACHE_DISABLE=1` to widget spawner environment.

**Files:** `bmc/src/widget/spawner.rs`

**Expected savings:**

- Eliminates 2 MB shader cache mmap per widget
- Removes disk cache thread (1 fewer thread per widget)
- Reduces thread stack virtual memory reservations

**Trade-off:** Shaders recompiled each startup. Acceptable for flip-clock's simple shaders (a few hundred bytes).

### Irreducible Overhead

With all optimizations, per-widget baseline is approximately:

- ~4-5 MB Mesa/etnaviv driver heap (per EGL context, unavoidable)
- ~12-13 MB shared library pages (shared across processes via mmap)
