# WASM Widget Memory

Each widget instance owns WebAssembly linear memory containing its stack, static data, and heap. Every wasm build
reserves a 64 KiB stack, which is one WebAssembly page. This keeps the per-instance floor small while retaining
substantial headroom over the production widget catalog's observed use. The scaling figures below use 200 instantiated
widgets to make the saving concrete; that number illustrates the trend rather than recording a supported limit.

The stack reservation is configured for every Wasm workspace in `workspace.nix` through the wasm linker's `-zstack-size`
argument, and again in each wasm root's `.cargo/config.toml` so that builds driven straight from cargo match; the
`wasm-stack-size` check reads the reservation back off every built module and fails if the two ever drift apart. It is a
build-time limit, not a runtime setting and not a growable stack. WebAssembly memory can grow in 64 KiB pages for heap
allocations, but growing linear memory does not move or enlarge the stack region laid out by the linker.

## Measured stack use

Measurements taken on 2026-08-12 used optimized modules. Post-link instrumentation recorded the lowest shadow-stack
pointer reached during each regression-capture scenario. That instrumentation is not part of the tree, so the tables
below are a point-in-time snapshot rather than something the build regenerates: nothing re-measures them when a widget
changes, and no gate fails if a widget's high-water mark grows while staying under the reservation. What the repo does
enforce is the reservation itself, through the `wasm-stack-size` check.

### Production widgets

The production run covered all 36 configured scenarios across all 12 widgets.

| Widget            | Maximum observed stack use |
| ----------------- | -------------------------: |
| Blockheight       |                2,144 bytes |
| Clock             |                3,920 bytes |
| Fleet Management  |                6,176 bytes |
| Halving Countdown |                4,016 bytes |
| Image             |                2,256 bytes |
| ISS Position      |                3,952 bytes |
| Mining Clock      |                1,504 bytes |
| Mining Info       |                7,120 bytes |
| Nameday           |                1,648 bytes |
| Random Facts      |                  768 bytes |
| SpaceX Launch     |                4,496 bytes |
| Weather           |                4,016 bytes |

### Example widgets

The example run covered all ten configured scenarios across all nine widgets. Their uninstrumented 64 KiB builds also
matched all 997 baseline capture frames.

| Widget         | Maximum observed stack use |
| -------------- | -------------------------: |
| Calendar       |                6,112 bytes |
| Hello Widget   |                5,936 bytes |
| Home Assistant |                4,800 bytes |
| Media Control  |                5,488 bytes |
| Mesh Demo      |                1,888 bytes |
| Metronome      |                3,584 bytes |
| Params Demo    |                4,864 bytes |
| Pomodoro       |                4,992 bytes |
| Stress Test    |                1,184 bytes |

Mining Info was the maximum across both catalogs at 7,120 bytes. A 64 KiB reservation is about 9.2 times that
observation and leaves 58,416 bytes of headroom.

These values are exact for the exercised fixture paths, not a proof that every possible execution stays below them.
Fixtures do not cover arbitrary malformed or maximum-sized responses, every lifecycle sequence, or future compiler and
dependency changes. The regression suite proves that the configured paths do not overflow the selected stack; source
review and the margin protect the paths that were not measured.

## What uses stack

Stack growth comes from live call frames and their inline locals. The main risks are:

- recursion whose depth is controlled by external data;
- deeply nested UI trees, because tree serialization follows nesting depth recursively;
- large local arrays or structs containing inline arrays;
- deep call chains that keep large locals alive simultaneously; and
- parsing, formatting, or serialization code with many compiler-spilled temporaries.

Collection length is not itself a stack cost when the collection is heap-backed. A `Vec<T>` keeps only its pointer,
length, and capacity on the stack; its elements occupy the guest heap. `String`, maps, deques, and boxed slices follow
the same principle. Prefer an explicit `Vec` worklist over input-dependent recursion and prefer a `Vec` over a large
fixed local array.

Heap allocation still consumes linear memory. This guidance moves variable or large storage away from the fixed stack;
it does not make that storage free. Dropping values lets the guest allocator reuse their space, but does not guarantee
that WebAssembly memory shrinks or that pages immediately leave the process's resident set.

## Scaling and resident memory

Two hundred 64 KiB stack reservations account for 12.5 MiB of linear-memory capacity. Compared with the former 1 MiB
stacks, this removes 960 KiB per widget and 187.5 MiB across 200 widgets.

Those figures isolate stack capacity. Initial linear memory is rounded to 64 KiB pages and also contains static data, so
the process-level saving depends on each module's layout and which pages become resident. Zram can compress cold
anonymous pages, but does not remove the per-instance linear-memory layout. Renderer assets are host-side resources with
their own lifecycle; dropping them reduces renderer memory independently of the guest stack reservation.

Going lower would buy little and leaves less protection against unmeasured paths. Because static data is laid out
directly above the stack, the two share a page ceiling: a smaller stack only drops a page when it pulls the combined
total under a multiple of 64 KiB, which for most widgets it does not. Treat 64 KiB as the practical floor unless new
measurements and a different runtime layout justify changing it.
