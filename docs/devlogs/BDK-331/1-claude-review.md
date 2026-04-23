# Branch Review: `jku/BDK-331/regression-testing`

**Base:** `6ec5da78dd1305c4544208bb313f5ef699226672` **97 commits, 373 files, ~55k lines added / ~7k removed**
**Reviewed:** 2026-04-03 **Updated:** 2026-04-11

---

## Critical Issues (Must Fix)

### C1. KV Store Path Traversal -- WASM Sandbox Escape

`bmc-wasm-runtime/src/runtime_wasmi.rs` ~L1259-1364

`host_kv_set`/`host_kv_get`/`host_kv_delete` join WASM-provided keys directly onto the filesystem path without
sanitizing `../` sequences. A malicious widget can read/write/delete arbitrary files on the embedded device.

**Fix:** Reject keys containing `/`, `\`, or `..`.

### C2. Committed Secrets (3 locations)

- `bmc-wasm-runtime/examples/media-control/secrets.ini` -- Emby API key, Kodi credentials
- `bmc-wasm-runtime/examples/spacex-launch/src/lib.rs:20` -- Launch Library 2 API token
- `bmc-wasm-runtime/examples/home-assistant/src/lib.rs:16-17` -- Home Assistant JWT (expires 2036)

All should be removed from tracking, rotated, and replaced with KV store lookups.

### C4. Use-After-Free Pattern in SDK Event Handlers

`bmc-wasm-runtime/sdk/src/mdns.rs`, `ssdp.rs`, `udp_broadcast.rs`

A `&str` is created from raw WASM memory *before* `Vec::from_raw_parts` takes ownership, producing a reference with no
backing Rust owner. Works by accident (scope ordering) but is fragile. Contrast with `ws.rs`/`socket.rs` which do it
correctly (construct `Vec` first, then borrow from it).

### C6. Dead File: Old Monolithic `capture.rs` (1,654 lines)

`bmc-wasm-runtime/src/bin/capture.rs`

The entire old capture binary still exists alongside the new `capture/main.rs`. Not referenced by `Cargo.toml`. Delete
it.

### C7. GPU Resource Leaks -- Remaining Scope Is Narrower In Current Tree

- `SphereRenderer` (`gpu/sphere.rs`) -- owns GL program, VBO, FBO, texture, and a FemtoVG image handle; still needs
  explicit teardown from its owning renderer
- `BitmapRegistry` (`gpu/bitmap.rs`) -- FemtoVG `ImageId` handles still need owner-driven cleanup
- `DoubleBufferState` concern is weaker than in the original review because production widget paths now go through
  `DoubleBufferedEglState`, which destroys export buffers in `Drop`; the remaining risk is manual `DoubleBufferState`
  use outside that wrapper

---

## Important Issues (Should Fix)

### I1. Unbounded Thread Spawning from WASM Host Functions

`runtime_wasmi.rs`

Every `host_fetch`, `host_ws_connect`, `host_tcp_connect`, `host_mdns_browse`, etc. spawns a thread with no per-runtime
cap. A malicious widget can exhaust system threads/memory. Add per-runtime connection limits.

Update 2026-04-11: resolved on the current branch. The runtime now enforces per-runtime caps and the SDK surfaces
rejected starts as `None` instead of registering dead callback entries for id `0`.

### I2. `NoCertVerifier` Skips ALL TLS Validation

`runtime_wasmi.rs` ~L3646-3686

Applies to all TLS connections from any widget, not just Chromecast. Make opt-in per connection.

### I4. `host_decode_image` -- No Size Limit on Decoded Output

`runtime_wasmi.rs` ~L530-583

A small compressed image could decompress to hundreds of MB of RGBA. Add max pixel budget.

Update 2026-04-11: resolved on the current branch. `host_decode_image` now applies both a pixel cap and a decoder
allocation cap before `decode()`, so high-bit-depth images are rejected before they allocate oversized intermediate
buffers.

### I6. Workspace-Level Lint Suppressions Too Broad

`Cargo.toml`

`result_large_err`, `io_other_error`, `large_enum_variant` are suppressed workspace-wide. These were intended for
vendored crates (now git deps). The ~20k lines of new `bmc-wasm-runtime` code also gets these lints silenced. Scope to
the specific crates.

### I8. XML Re-Parsed on Every Query

`runtime_wasmi.rs` ~L1648-1728

`roxmltree::Document::parse()` called on every `host_xml_get_str`/`host_xml_get_f64`. 10 field queries = 10 full parses.
Consider parsing once and extracting to a map.

### I10. EGL Shader Leak on Error Path

`widgets/wasm/src/egl.rs` ~L234

If fragment shader compilation fails, the already-created vertex shader is never deleted.

### I11. Size Name Definitions Duplicated in 3 Places

Capture infrastructure: `run_all.rs`, `run.rs`, `capture_config.rs` all define size-to-dimension mappings independently.
Define once.
