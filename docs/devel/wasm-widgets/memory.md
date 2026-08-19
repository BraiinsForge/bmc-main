# WASM Widget Memory

Each widget instance owns WebAssembly linear memory containing its stack, static data, and heap. Every wasm build
reserves a 64 KiB stack, which is one WebAssembly page. This keeps the per-instance floor small while retaining
substantial headroom over the production widget catalog's observed use. The scaling figures below use 200 instantiated
widgets to make the saving concrete; that number illustrates the trend rather than recording a supported limit.

The stack reservation is configured for every Wasm workspace in `workspace.nix` through the wasm linker's `-zstack-size`
argument, and again in the repository's `.cargo/config.toml` so that builds driven straight from cargo match; the
`wasm-stack-size` check reads the reservation back off every built module and fails if the two ever drift apart. It is a
build-time limit, not a runtime setting and not a growable stack. WebAssembly memory can grow in 64 KiB pages for heap
allocations, but growing linear memory does not move or enlarge the stack region laid out by the linker.

## Measured stack use

Measurements taken on 2026-08-18 used optimized modules. Post-link instrumentation recorded the lowest shadow-stack
pointer reached during each regression-capture scenario. Re-run the same measurement with `just wasm::stack-profile`;
the resulting `stack-usage.md` under `result/` aggregates every configured size and variant after comparing the
instrumented renders with their regression baselines. This is a developer report, not a CI gate: the build enforces the
reservation through `wasm-stack-size`, but does not fail when an observed high-water mark changes while remaining below
it. The instrumented 64 KiB builds matched all regression baselines across both catalogs.

### Production widgets

The production run covered all 49 configured scenarios across all 15 widgets.

| Widget            | Maximum observed stack use |
| ----------------- | -------------------------: |
| Blockheight       |                2,384 bytes |
| Braiins Pool      |                4,992 bytes |
| Clock             |                4,320 bytes |
| Fleet Management  |                6,832 bytes |
| Halving Countdown |                4,512 bytes |
| Image             |                2,512 bytes |
| ISS Position      |                4,128 bytes |
| Mining Clock      |                1,424 bytes |
| Mining Info       |                7,904 bytes |
| Nameday           |                1,808 bytes |
| Random Facts      |                  736 bytes |
| SpaceX Launch     |                4,928 bytes |
| Ticker List       |                3,952 bytes |
| Ticker Single     |                3,184 bytes |
| Weather           |                4,512 bytes |

### Example widgets

The example run covered all ten configured scenarios across all nine widgets.

| Widget         | Maximum observed stack use |
| -------------- | -------------------------: |
| Calendar       |                6,272 bytes |
| Hello Widget   |                6,640 bytes |
| Home Assistant |                4,800 bytes |
| Media Control  |                6,112 bytes |
| Mesh Demo      |                2,144 bytes |
| Metronome      |                3,984 bytes |
| Params Demo    |                5,424 bytes |
| Pomodoro       |                5,648 bytes |
| Stress Test    |                1,312 bytes |

Mining Info was the maximum across both catalogs at 7,904 bytes. A 64 KiB reservation is about 8.3 times that
observation and leaves 57,632 bytes of headroom.

These values are exact for the exercised fixture paths, not a proof that every possible execution stays below them.
Fixtures do not cover arbitrary malformed or maximum-sized responses, every lifecycle sequence, or future compiler and
dependency changes. The regression suite proves that the configured paths do not overflow the selected stack; source
review and the margin protect the paths that were not measured.

## What overflow looks like

The stack occupies the bottom of linear memory and grows downwards, so overflowing it runs off address zero and traps as
an ordinary out-of-bounds access. It does not quietly corrupt the static data sitting above it.

A trap is terminal for the instance wherever it surfaces, in `render` or in a delivery callback (a fetch response, a
socket or mesh message). The host logs it and tears the slot down. It has to: a trap is a non-local exit, so the guest's
frames never run their epilogues and `__stack_pointer` keeps the value it held when the trap fired. Re-driving a trapped
instance would hand it a permanently smaller stack.

Fuel exhaustion is the exception. `render` treats it as recoverable and retries, so a widget that overruns its
instruction budget keeps its instance — along with whatever stack the trap consumed. The strike counter resets on any
successful render, so repeated overruns ratchet the usable stack down across an instance's life without a bound.

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
