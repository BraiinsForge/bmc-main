# DevShell Improvements

**Ticket:** BDK-308

## Problem

The Nix development environment had several gaps that made it difficult to work on the project without system-level
dependencies:

1. **Default devShell was too minimal** — only provided `rustToolchain`, missing protobuf and workspace env vars
2. **bmc-mock-display/shell.nix was orphaned** — used impure `<nixpkgs>` channel, had a separate Rust toolchain, not
   integrated with the flake
3. **Missing runtime libraries** — `libfontconfig.so.1` needed by Slint for font enumeration (loaded via dlopen)
4. **No frontend devShell** — developers needed global node/yarn; biome/sass ship prebuilt native binaries that don't
   work on NixOS out of the box
5. **No documentation** — unclear which shell to use for what

## Previous State

```
flake.nix devShells:
├── default        → just rustToolchain (broken for most builds)
├── fast           → workspace shell, local native builds
├── armv7-release  → ARM cross-compile shell
├── armv7-debug    → ARM cross-compile shell
└── frontend       → mkShell with nodejs/yarn + LD_LIBRARY_PATH for libgcc

bmc-mock-display/shell.nix  → standalone, impure, X11/Wayland/GL deps
```

The workspace shells (`fast`, `armv7-*`) included `protobuf` and `diffutils` via `nativeDeps`, plus the
`FONTCONFIG_FILE` env var. But they did not include:

- The fontconfig *library* for runtime dlopen
- X11/Wayland/GL libraries for GUI development
- Node.js tooling for frontend

Dependencies were defined separately in `workspace.nix` and `flake.nix`, with no shared definitions.

## New State

```
flake.nix devShells:
├── default (= full) → complete dev environment: Rust + frontend + GUI + ARMv7 cross-compiler
├── local            → same as full but without the ARMv7 cross-compiler
├── fast             → workspace build shell (from workspace.nix, for CI)
├── armv7-release    → ARM cross-compile shell (from workspace.nix)
└── armv7-debug      → ARM cross-compile shell (from workspace.nix)
```

- `bmc-mock-display/shell.nix` was deleted.
- The standalone `frontend` shell was removed — frontend deps are part of the default shell.
- Dependencies are defined once in `flake.nix` as `commonDeps` and shared with `workspace.nix`, keeping build
  derivations and devShells in sync.

## Key Decisions

### Minimal LD_LIBRARY_PATH + RUSTFLAGS rpath

The dev shell uses a plain `mkShell` with a two-pronged approach for native library dependencies:

- **Minimal `LD_LIBRARY_PATH`** — only `libgcc` is exposed via `LD_LIBRARY_PATH`. This is the minimum needed for
  vendored npm binaries (biome, sass-embedded) that are dynamically linked against glibc.
- **RUSTFLAGS rpath for GUI libs** — instead of putting GUI libraries (X11, Wayland, fontconfig, GL, etc.) on
  `LD_LIBRARY_PATH`, the shell sets `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS` with `-Wl,-rpath,<paths>`. This
  bakes the library search paths directly into compiled Rust binaries, so they can find their dlopen dependencies
  (e.g. Slint loading `libfontconfig.so.1`) at runtime without any environment variable.

This keeps the shell simple and the built binaries self-contained with respect to their runtime library dependencies.

### Build deps vs runtime deps split

GUI dependencies were split into two groups:

- **`guiBuildDeps`** — libraries needed at compile/link time: `fontconfig`, `freetype`
- **`guiRuntimeDeps`** — libraries needed at runtime via dlopen: X11, Wayland, GL, vulkan, fontconfig, etc.

The runtime deps are referenced by the RUSTFLAGS rpath but are not added to `LD_LIBRARY_PATH`.

### ARM cross-compilation in the dev shell

The `full` shell includes the ARMv7 cross-compiler and sets `CC_armv7_unknown_linux_musleabihf` and
`CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER` env vars. This means developers can cross-compile for ARM directly
from their dev shell without entering a separate shell.

The `local` shell is available for developers who don't need the ARM toolchain.

### Frontend Binary Analysis

These are the vendored npm binaries that motivated the `LD_LIBRARY_PATH` setup:

| Binary                                  | Type            | Dependencies                                                      |
|-----------------------------------------|-----------------|-------------------------------------------------------------------|
| `@biomejs/cli-linux-x64/biome`          | Dynamic (glibc) | libgcc_s, libpthread, libm, libdl, libc                           |
| `sass-embedded-linux-x64/.../dart`      | Dynamic (glibc) | libdl, libpthread, libm, libc                                     |
| `sass-embedded-linux-musl-x64/.../dart` | Dynamic (musl)  | libc.musl-x86_64.so.1 (ignored via autoPatchelfIgnoreMissingDeps) |
| `@bufbuild/buf-linux-x64/bin/buf`       | Static          | None                                                              |
| `@esbuild/linux-x64/bin/esbuild`        | Static          | None                                                              |

The glibc-linked binaries only need basic libc/libgcc — the minimal `LD_LIBRARY_PATH` with `libgcc` is sufficient.
The musl variant and static binaries need no special handling.

## What Changed

| Area                          | Before                                     | After                                              |
|-------------------------------|--------------------------------------------|----------------------------------------------------|
| Default devShell              | `mkShell` with only `rustToolchain`        | `mkShell` with all deps + ARMv7 cross-compiler     |
| GUI runtime libs              | Not available in any shell                 | Baked into binaries via RUSTFLAGS rpath             |
| `LD_LIBRARY_PATH`             | Not set (or large in `frontend` shell)     | Minimal: only `libgcc`                             |
| `fast` / `frontend` shells    | Existed as separate shells                 | Removed from flake devShells (fast still in workspace.nix) |
| `bmc-mock-display/shell.nix`  | Standalone impure shell.nix                | Deleted                                            |
| Dependency definitions        | Duplicated between flake.nix/workspace.nix | Shared via `commonDeps`                            |
| `workspace.nix` signature     | `{ self, pkgs }`                           | `{ self, pkgs, commonDeps }`                       |
| Frontend tooling              | Separate `frontend` shell                  | Part of the default shell                          |
| ARM cross-compilation         | Only via workspace.nix build profiles      | Also available directly in the `full` dev shell    |
| Devcontainer                  | None                                       | `.devcontainer/devcontainer.json` added            |
| Fonts                         | Only `corefonts`                           | `corefonts` + `font-awesome_6`                     |

## External Consumers

**bos-main** is safe. It only consumes packages via flake input:

```nix
inputs.bmc-main.packages.${localSystem}."bmc-openwrt-${arch}-release"
inputs.bmc-main.packages.${localSystem}.frontend
```

These packages are unchanged.
