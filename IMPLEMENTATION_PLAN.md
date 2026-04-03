# BDK-331 Implementation Plan

## Review Validation

- Still applicable on the current branch: `C1`, `C2`, `C4`, `C6`, `I1`, `I2`, `I4`, `I8`, `I10`, `I11`.
- Still applicable with narrower scope: `C7`. `SphereRenderer` and `BitmapRegistry` still own GPU-side resources without
  matching cleanup. The original `DoubleBufferState` concern is less urgent in the current tree because production
  widgets now go through `DoubleBufferedEglState`, which destroys buffers in `Drop`.
- Still worth fixing, but the original rationale is stale: `I6`. The workspace-wide Clippy suppressions are still broad,
  but `ii-net` and `ii-net-drv` are still path/workspace crates on this branch, so the first step is an audit, not a
  blind removal.

## Stage 1: Harden Runtime File and Decode Boundaries

**Goal**: Close the filesystem escape in KV storage and add explicit bounds to host-side image decoding.

**Success Criteria**: `host_kv_set`, `host_kv_get`, and `host_kv_delete` reject unsafe keys before touching cache or
disk; KV persistence never escapes the configured base directory; `host_decode_image` enforces a documented pixel/byte
budget and fails fast on oversized images.

**Tests**: Add unit tests for the KV key validator; add regression tests that exercise set/get/delete against a temp KV
directory and prove traversal keys do not create or read files outside the sandbox; add decode-image tests covering a
normal small image and an image that exceeds the configured budget.

**Status**: Done (C1 and I4 fixed: KV keys are validated before cache/disk use, and `host_decode_image` now enforces a
decoded pixel budget with helper coverage.)

## Stage 2: Remove Tracked Secrets and Replace the Secret-Seeding Flow

**Goal**: Remove committed credentials from tracked files and switch examples/test tooling to explicit KV-based
configuration that stays out of git.

**Success Criteria**: The committed Home Assistant token, Launch Library token, and media-control secrets file are
removed from tracked sources; Home Assistant and SpaceX examples read credentials from KV or another local-only
mechanism instead of hard-coded constants; media-control no longer relies on a tracked `secrets.ini`; testbed/capture
seeding uses fixture-header KV, capture config KV, and/or a gitignored local secrets template. Secret rotation is
tracked as an operational follow-up outside the repo.

**Tests**: Add focused config-loading tests for example widgets; add a smoke path for testbed/capture KV seeding without
tracked secrets; add ignore rules or a repository check so local secret files are not accidentally recommitted.

**Status**: Done (tracked secrets removed from sources; examples read KV; media-control uses a gitignored local
`secrets.ini` plus example template for local testbed seeding.)

## Stage 3: Contain the WASM Host Networking Surface

**Goal**: Fix the SDK event ownership bug, cap background resource creation per runtime, and make insecure TLS an
explicit opt-in.

**Success Criteria**: `mdns`, `ssdp`, and `udp_broadcast` event handlers take ownership of host buffers before borrowing
from them; the runtime enforces per-runtime limits for spawned/background resources and returns stable failure codes
when limits are exceeded; TLS verification is enabled by default and any insecure/self-signed path is exposed through an
explicitly named API used only by the call sites that require it.

**Tests**: Add SDK unit tests around event decoding helpers; add runtime tests that saturate the per-runtime limits and
verify no extra state/thread-backed resource is created; add TLS tests for secure default behavior plus the explicit
insecure override path.

**Status**: Done (C4 fixed in mdns/ssdp/udp_broadcast. I1 fixed with per-runtime resource caps. I2 fixed by making
verified TLS the default and adding an explicit insecure TLS API for self-signed LAN devices.)

## Stage 4: Finish Cleanup for Dead Code, XML Reuse, and GPU/Error Paths

**Goal**: Remove leftover dead code, stop repeated XML reparsing, and close the remaining GPU/shader cleanup gaps.

**Success Criteria**: The unused legacy `src/bin/capture.rs` file is removed and the declared `capture` binary continues
to build from `src/bin/capture/main.rs`; XML work is cached so `host_xml_parse` performs the expensive parse/index step
once per document; `SphereRenderer` and bitmap image handles are released from the owning renderer on teardown;
`widgets/wasm` frees shaders/programs on all early-return error paths, not only on the happy path.

**Tests**: Add regression tests for XML lookup behavior after the caching refactor; run the capture binary build path
after removing the dead file; add focused teardown/error-path tests where practical and verify the affected crates still
pass their existing test suites.

**Status**: Done (C6 removed the dead capture binary. I8 added XML query memoization. I10 fixed shader/program cleanup
on error. C7 fixed by tearing down sphere and bitmap GPU resources from `FemtoVgRenderer::drop` while the GL context is
still alive.)

## Stage 5: Restore Tooling Consistency

**Goal**: Centralize capture size definitions and narrow Clippy suppressions to the crates that actually need them.

**Success Criteria**: Capture size names and dimensions come from one shared source of truth used by `run_all`, `run`,
and config validation; the broad workspace-level Clippy allows are removed or reduced after auditing the actual
offenders; CI-equivalent linting remains green.

**Tests**: Add unit tests for shared size parsing/name mapping helpers; run
`cargo clippy --workspace --tests -- -D warnings` after moving or narrowing the suppressions; run capture-related
tests/builds that depend on the shared size metadata.

**Status**: Done (I11 centralized capture size metadata. I6 narrowed lint scope by removing the workspace-wide
suppressions and keeping only the crate/local exceptions that are still justified.)
