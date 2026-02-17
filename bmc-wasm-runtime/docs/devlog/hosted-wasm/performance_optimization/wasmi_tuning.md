# wasmi Interpreter Tuning Guide

Actionable plan for optimizing wasmi 1.0 execution performance in a WASM widget runtime. Assumes the host application
calls a WASM `render()` function every frame (16–33ms budget), with fuel metering for safety.

**Runtime performance (per-frame execution speed) is the primary goal.** Startup time is secondary — widgets load once
and render thousands of frames. Every microsecond saved per frame compounds across the lifetime of the widget.

## Background

After optimizing the host side (render loop, caching, allocations), **wasmi itself is ~76% of remaining CPU time**
(inclusive, samply profiling). The register-based engine (shipped in wasmi 0.32, included in 1.0) is already active —
there is no backend switch to flip. What remains is configuration tuning, reducing work the interpreter does per frame,
and evaluating whether a different runtime is warranted.

## 1. Reduce interpreter work per frame

These target steady-state execution — the hot path that runs every frame.

### 1a. Benchmark fuel overhead ✅ (desktop — needs armv7)

Fuel metering instruments every bytecode sequence with fuel-check instructions. wasmi integrates this into its
register-based translator (cheaper than wasmtime's approach), but on ARM with tight budgets, even small overhead
matters.

**Desktop result:** \<1% difference (1.8ms with fuel vs 1.8ms without, within noise). Fuel overhead is negligible on
x86. **Still needs benchmarking on armv7** where the CPU is weaker and cache effects may differ.

Decision: keep fuel enabled for safety. If armv7 shows >5%, consider making it configurable.

### 1b. Tune stack caching ✅ (applied, needs armv7 validation)

wasmi caches execution stacks for reuse across calls. Default is 2. Applied `set_max_cached_stacks(4)`.

```
config.set_max_cached_stacks(4);
```

**Desktop result:** \<1% difference (within noise). Applied anyway — negligible memory cost, may help on armv7.

### 1c. Minimize host–guest boundary crossings

Each host function call from WASM goes through wasmi's call machinery (register save/restore, fuel accounting, type
checking). Profile with `analyze_profile.py` to check if host function trampolines appear in hot functions.

Mitigations:

- **Batch host calls:** If widgets do many small host queries per frame, combine into fewer calls with larger payloads
- **Move work to WASM or to host:** A host function that does trivial work (return a constant, read a flag) pays
  disproportionate overhead relative to its useful work. Either inline it in the WASM side or batch it with other calls

### 1d. Profile and optimize the Wasm code itself

The widget's Rust code compiles to Wasm instructions that wasmi interprets. Inefficient Wasm (excessive copies,
unnecessary allocations, unoptimized loops) directly translates to more interpreter iterations. **If the widget is doing
unnecessary work, no amount of interpreter tuning helps.**

```bash
# Check instruction counts and code structure
wasm-opt --metrics input.wasm

# Optimize the Wasm binary itself (binaryen)
wasm-opt -O3 -o optimized.wasm input.wasm
```

Also ensure release builds are properly optimized:

```toml
[profile.release]
opt-level = "z"     # or "s" — smaller code = fewer instructions to interpret
lto = true
codegen-units = 1
strip = true
```

Smaller Wasm = fewer instructions = faster interpretation. `opt-level = "z"` often produces faster-interpreting code
than `opt-level = 3` because the size-optimized output avoids loop unrolling and inlining that helps native CPUs but
just means more bytecode for an interpreter to step through.

## 2. Configuration tuning (secondary)

These primarily affect startup and module loading. Worth doing but not the main win.

### 2a. Disable unused Wasm proposals ✅ (applied)

wasmi 1.0 enables many Wasm proposals by default. Disabled everything not used by Rust-compiled widgets.

Note: `wasm_simd()` and `wasm_relaxed_simd()` are behind a compile-time feature gate in wasmi 1.0.9 and are not
available unless the `simd` feature is enabled (it isn't in our build).

```
config.wasm_tail_call(false);
config.wasm_multi_memory(false);
config.wasm_memory64(false);
config.wasm_extended_const(false);
config.wasm_custom_page_sizes(false);
config.wasm_wide_arithmetic(false);

// Keep enabled: bulk_memory, reference_types, mutable_globals, sign_extension,
// saturating_float_to_int, multi_value (used by standard Rust-compiled Wasm)
```

### 2b. Try `Lazy` compilation mode

Default is `LazyTranslation` (validate eagerly, translate on first call). Full `Lazy` mode defers both validation and
translation, giving up to 27x faster startup:

```
config.compilation_mode(wasmi::CompilationMode::Lazy);
```

**Trade-off:** A malformed function that's never called won't be detected. Safe for first-party widgets compiled from
Rust. Third-party widgets would need validation at submission time.

## 3. Alternative runtimes (major change)

Only pursue this if the above tuning is insufficient to meet the frame budget on the target device.

| Runtime              | Type                 | armv7        | Speedup vs wasmi | Integration effort |
| -------------------- | -------------------- | ------------ | ---------------- | ------------------ |
| **wasm3**            | C interpreter        | Yes          | ~comparable      | Moderate (C FFI)   |
| **WAMR interpreter** | C interpreter        | Yes          | ~1.5-2x          | Moderate (C FFI)   |
| **WAMR AOT**         | Ahead-of-time native | Yes          | ~10-50x          | High               |
| **wasmtime**         | JIT (Cranelift)      | aarch64 only | ~10-100x         | N/A for armv7      |

**WAMR AOT** is the only path to dramatically faster execution on armv7. It requires:

1. Compiling each `.wasm` to a native `.aot` binary using `wamrc` during build or widget submission
2. Integrating WAMR's C runtime via FFI (replacing wasmi entirely)
3. Maintaining host function bindings in C or via `wamr-rust-sdk`

This is a large architectural change — only if profiling on the real device shows the interpreter is the bottleneck and
the frame budget cannot be met otherwise.

## 4. Measurement plan

Before making changes, establish baselines on the **target device** (armv7):

```bash
# Internal timing (frame breakdown)
make run EXAMPLE=hello-widget ARGS="--perf-report=reports/04-wasmi-baseline/perf.json --perf-frames=600"

# CPU profile
make profile
python3 tools/analyze_profile.py profile.json.gz
```

After each change, re-run the same benchmark and compare with `compare_reports.py`. Focus on:

- `avg_wasm_us` — direct measure of interpreter time per frame (**primary metric**)
- `avg_frame_us` — end-to-end frame time
- `p95_frame_us` — tail latency (sensitive to GC pauses and translation stalls)

## 5. Priority order

1. **Benchmark fuel overhead** — most likely free performance, quantify the cost on armv7
2. **Tune stack caching** — reduces per-call allocation overhead in the hot loop
3. **Profile Wasm code itself** — the widget might be doing unnecessary work
4. **Optimize Wasm binary** — `wasm-opt -O3`, `opt-level = "z"`, LTO
5. **Minimize host–guest crossings** — batch calls if trampolines show up hot in profiles
6. **Disable unused Wasm proposals** — cheap win for startup, minor for runtime
7. **`Lazy` compilation mode** — startup only
8. **WAMR AOT** — nuclear option if nothing else meets the frame budget
