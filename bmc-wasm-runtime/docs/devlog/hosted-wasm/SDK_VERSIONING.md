# SDK Version Matching

Ensures widgets compiled against one SDK version cannot silently break on a mismatched host.

## Problem

No version checking exists between WASM widgets and the host runtime. A widget compiled against a different SDK version
can silently produce garbage rendering, crash on missing host functions, or misinterpret the binary tree format.

## Design

### Mechanism: exported function

The SDK defines a `#[no_mangle] pub extern "C" fn __bmc_sdk_version() -> u64` that returns the version packed into a
single `u64` (`major | minor << 16 | patch << 32`). Any widget depending on `bmc-wasm-sdk` automatically exports this
function. The host calls it after instantiation but before `init`/`render` and rejects on mismatch.

Why exported function over a WASM custom section: `#[link_section]` statics from dependency rlibs get stripped by
wasm-ld in debug builds (no LTO). Only release builds with `lto = true` preserve them. Exported functions survive in all
build profiles because wasm-ld always preserves `#[no_mangle]` exports from dependencies.

### Version scheme: semver packed into u64

Semver `major.minor.patch` as 3 x `u16` packed into a `u64`. Starting at `0.1.0` to match current crate versions.

**Matching rule:** major must match exactly. Minor/patch are informational (logged for diagnostics). This keeps the door
open for backwards-compatible minor bumps later.

### Where the version lives

The canonical constant is in `bmc-wasm-protocol` (the shared `no_std` crate used by both SDK and host). The SDK's
exported function references it — any widget depending on `bmc-wasm-sdk` gets the version export automatically. No
widget code changes needed.

Non-Rust toolchains would need to export `__bmc_sdk_version() -> u64` manually (same packing scheme).

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

### Key files

| File                      | Role                                                                             |
| ------------------------- | -------------------------------------------------------------------------------- |
| `protocol/src/version.rs` | `SDK_VERSION` constant, export name, pack/unpack helpers                         |
| `sdk/src/lib.rs`          | `#[no_mangle] __bmc_sdk_version()` — auto-exported from every widget             |
| `src/runtime.rs`          | `check_sdk_version()` — calls export after instantiation; `sdk_version()` getter |
| `src/bin/testbed.rs`      | Prints version to CLI, shows in window title (including on hot-reload)           |

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
