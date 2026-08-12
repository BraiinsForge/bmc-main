# SDK Version Matching

Ensures widgets compiled against one SDK version cannot silently break on a mismatched host.

## Problem

No version checking exists between WASM widgets and the host runtime. A widget compiled against a different SDK version
can silently produce garbage rendering, crash on missing host functions, or misinterpret the binary tree format.

## Design

### Mechanism: exported function

The SDK defines a `#[no_mangle] pub extern "C" fn __bmc_sdk_init() -> u64` that returns the version packed into a single
`u64` (`major | minor << 16 | patch << 32`). Any widget depending on `bmc-wasm-sdk` automatically exports this function.
The host calls it after instantiation but before `init`/`render` and rejects on mismatch.

Why exported function over a WASM custom section: `#[link_section]` statics from dependency rlibs get stripped by
wasm-ld in debug builds (no LTO). Only release builds with `lto = true` preserve them. Exported functions survive in all
build profiles because wasm-ld always preserves `#[no_mangle]` exports from dependencies.

### Version scheme: semver packed into u64

Semver `major.minor.patch` as 3 x `u16` packed into a `u64`. Starting at `0.1.0` to match current crate versions.

**Matching rule:** major must match exactly. Minor/patch are informational (logged for diagnostics). This keeps the door
open for backwards-compatible minor bumps later.

### Where the version lives

The canonical constant is in `bmc-wasm-protocol` (the shared wire-format crate used by both SDK and host). The SDK's
exported function references it — any widget depending on `bmc-wasm-sdk` gets the version export automatically. No
widget code changes needed.

Non-Rust toolchains would need to export `__bmc_sdk_init() -> u64` manually (same packing scheme).

### Host-side check

```
Engine + Module → FemtoVgRenderer → HostState → Store → Linker → instantiate → check_sdk_version() → render/init
```

The check runs after instantiation (since it calls the export) but before any widget code (`init`/`render`).

### Error cases

| Case                              | Behavior                                               |
| --------------------------------- | ------------------------------------------------------ |
| Missing export                    | Reject — error guides the author to export the version |
| Major version mismatch            | Reject — error shows both versions                     |
| Minor/patch differ, major matches | Accept — log the version for diagnostics               |

### Retiring a host import

Removing a host function is not the same as removing its import. wasmi resolves every import at instantiation, so a
widget importing `env.host_foo` fails to load outright against a host that no longer registers that name — and a widget
already on a device cannot be recompiled by the host that is rejecting it.

A retired host function therefore keeps its `Linker` registration as an inert stub: same name, same signature, ignoring
its arguments and returning whatever value the SDK wrapper already mapped to "unavailable". Deployed widgets keep
loading and silently take their fallback path. That silence is why the retirement is a host-side behavioural change and
takes a minor bump — the version is the only signal a reader gets.

If a retired function has no such sentinel in its return type, the retirement is not backwards compatible at all and
needs a major bump instead of a stub.

An inert stub may be deleted at the next **major** bump, and not before. That is the point where `check_sdk_version`
begins rejecting every widget that could have imported it, so the registration becomes dead weight rather than a
compatibility shim.

`env.host_bitmap_sample` is the first of these: retired in 0.3.0, droppable at 1.0.0. Its contract — instantiation plus
the sentinel return — is pinned by `bmc-wasm-runtime/tests/bitmap_sample_abi.rs`.

### Key files

| File                                       | Role                                                                             |
| ------------------------------------------ | -------------------------------------------------------------------------------- |
| `bmc-wasm-runtime/protocol/src/version.rs` | `SDK_VERSION` constant, export name, pack/unpack helpers                         |
| `bmc-wasm-runtime/sdk/src/lib.rs`          | `#[no_mangle] __bmc_sdk_init()` — auto-exported from every widget                |
| `bmc-wasm-runtime/src/runtime/backend.rs`  | `check_sdk_version()` — calls export after instantiation; `sdk_version()` getter |
| `bmc-wasm-runtime/src/runtime/imports/`    | `Linker` registrations for the `env.host_*` surface, including inert stubs       |
| `bmc-wasm-runtime/src/bin/testbed/main.rs` | Prints version to CLI, shows in window title (including on hot-reload)           |

## Phase 2: multi-version hosting (future)

The long-term plan is volta-style version management: the widget manifest (outside WASM) declares the SDK version it was
compiled against, and the device loads the matching host binding for that version.

The exported version serves as ground truth — even if the manifest is wrong, the host can verify the binary matches what
it claims.

### Approaches to evaluate for phase 2

- **Versioned modules in protocol + runtime** — `protocol/src/v1/`, `v2/` for format-specific types;
  `runtime/src/host_functions/v1.rs`, `v2.rs` for Linker registrations; `runtime/src/tree/v1.rs` for version-specific
  deserializers. Least duplication, shared types stay at crate root.
- **Separate versioned crates** — `bmc-wasm-protocol-v1`, `-v2` as independent crates. Full isolation but high
  duplication and more Cargo.toml management.
- **Additive evolution** — avoid breaking changes; new node types and host functions are additive, old widgets keep
  working. Only bump major on truly incompatible changes. Minimizes multi-version complexity but constrains protocol
  design.
