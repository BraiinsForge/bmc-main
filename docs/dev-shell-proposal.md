# DevShell Improvements Proposal & Implementation Guide

## Problem Statement

The current Nix development environment has several gaps that make it difficult to work on the project without
system-level dependencies:

1. **Default devShell is too minimal** - only provides `rustToolchain`, missing protobuf and workspace env vars
2. **bmc-mock-display/shell.nix is orphaned** - uses impure `<nixpkgs>` channel, has separate Rust toolchain, not
   integrated with flake
3. **Missing runtime libraries** - `libfontconfig.so.1` needed by Slint for font enumeration (loaded via dlopen)
4. **No frontend devShell** - developers need global node/yarn, biome/sass may need native libs
5. **No documentation** - unclear which shell to use for what

## Current State

```
flake.nix devShells:
├── default        → just rustToolchain (broken for most builds)
├── fast           → workspace shell, local native builds
├── armv7-release  → ARM cross-compile release
└── armv7-debug    → ARM cross-compile debug

bmc-mock-display/shell.nix  → standalone, impure, X11/Wayland/GL deps
```

The workspace shells (`fast`, `armv7-*`) do include `protobuf` and `diffutils` via `nativeDeps`, plus `FONTCONFIG_FILE`
env var. But they don't include:

- The fontconfig *library* for runtime dlopen
- X11/Wayland/GL libraries for GUI development
- Node.js tooling for frontend

## Proposed DevShell Structure

Simplified to avoid unnecessary complexity:

| Shell           | Purpose                      | Contents                                    |
|-----------------|------------------------------|---------------------------------------------|
| `default`       | Full development environment | Everything combined (Rust + frontend + GUI) |
| `armv7-release` | Keep as-is                   | ARM cross-compile release                   |
| `armv7-debug`   | Keep as-is                   | ARM cross-compile debug                     |

The `fast` shell becomes redundant - `default` covers all local development needs. Separate `display`/`frontend` shells
add complexity without clear benefit.

## Proposed Changes

### 1. Create combined `full` devShell as default

Current:

```nix
{
    default = pkgs.mkShell { packages = [ pkgs.ii.rustToolchain ]; };
}
```

Proposed - a combined shell using the shared `commonDeps`:

```nix
{
    full = (pkgs.buildFHSEnv {
      name = "bmc-full-env";
      targetPkgs = pkgs: with pkgs; [
        ii.rustToolchain
      ]
      ++ commonDeps.buildDeps
      ++ commonDeps.frontendDeps
      ++ commonDeps.fhsLibs
      ++ commonDeps.guiDeps;
    
      runScript = "bash";
      profile = ''
        export FONTCONFIG_FILE=${commonDeps.env.FONTCONFIG_FILE}
      '';
    }).env;
    
    default = full;
}
```

This uses `buildFHSEnv` to solve the ELF interpreter problem for frontend binaries while also providing all Rust and GUI
dependencies.

### 2. Why buildFHSEnv? (ELF Interpreter Problem)

Frontend tooling (biome, sass-embedded, etc.) ships prebuilt native binaries that expect a standard FHS layout:

```
$ patchelf --print-interpreter frontend/node_modules/sass-embedded-linux-x64/dart-sass/src/dart
/lib64/ld-linux-x86-64.so.2
```

This path doesn't exist on NixOS/Guix systems. The `yarn-files.nix` derivation handles this for CI builds using
`autoPatchelfHook`, but devShells need a different solution.

**Binary Analysis:**

| Binary                                  | Type            | Dependencies                                                      |
|-----------------------------------------|-----------------|-------------------------------------------------------------------|
| `@biomejs/cli-linux-x64/biome`          | Dynamic (glibc) | libgcc_s, libpthread, libm, libdl, libc                           |
| `sass-embedded-linux-x64/.../dart`      | Dynamic (glibc) | libdl, libpthread, libm, libc                                     |
| `sass-embedded-linux-musl-x64/.../dart` | Dynamic (musl)  | libc.musl-x86_64.so.1 (ignored via autoPatchelfIgnoreMissingDeps) |
| `@bufbuild/buf-linux-x64/bin/buf`       | Static          | None                                                              |
| `@esbuild/linux-x64/bin/esbuild`        | Static          | None                                                              |

`buildFHSEnv` creates a shell with standard Linux paths (`/lib64/ld-linux-x86-64.so.2`, etc.), solving this without
manual patching.

### 3. Deduplicate dependencies with workspace.nix

Currently `workspace.nix` defines build deps inline:

```nix
{
    nativeDeps = pkgs: with pkgs; [ protobuf diffutils ];
    env = { FONTCONFIG_FILE = pkgs.makeFontsConf { }; };
}
```

**Solution:** Define deps in `flake.nix`, pass to `workspace.nix` as parameter.

```nix
# flake.nix
let
  # Shared dependency definitions
  commonDeps = {
    # Build-time deps for Rust compilation
    buildDeps = with pkgs; [ protobuf diffutils ];

    # Environment variables
    env = {
      FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ pkgs.corefonts ]; };
    };

    # GUI libs for Slint/display work
    guiDeps = with pkgs; [
      fontconfig
      xorg.libX11 xorg.libXcursor xorg.libXrandr xorg.libXi
      xorg.libXinerama xorg.libXext xorg.libXft xorg.libXrender xorg.libxcb
      wayland wayland-protocols libxkbcommon
      libGL vulkan-loader mesa
    ];

    # Frontend tooling
    frontendDeps = with pkgs; [ nodejs yarn ];

    # Libs for FHS compat (node_modules binaries)
    fhsLibs = with pkgs; [ stdenv.cc.cc.lib glibc ];
  };

  # Pass to workspace.nix
  workspace = import ./workspace.nix { inherit self pkgs commonDeps; };
in {}
```

```nix
# workspace.nix - receives commonDeps, uses for nativeDeps/env
{ self, pkgs, commonDeps }:
let
  workspace = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    nativeDeps = pkgs: commonDeps.buildDeps;
    env = commonDeps.env;
    # ...
  };
in {}
```

This ensures build derivations and devShells stay in sync - single source of truth for deps.

### 4. Delete bmc-mock-display/shell.nix

After migration, remove the standalone file.

### 5. Update documentation

Add usage instructions to README or a dedicated doc explaining:

- How to enter the dev shell (`nix develop`)
- What the shell provides (Rust, frontend, GUI libs)
- ARM cross-compile shells (`nix develop .#armv7-release`)

### 6. Add devcontainer support

Provide devcontainer configs for VS Code / GitHub Codespaces users. Uses Alpine base with Nix feature (not a Nix-based
image, which doesn't work well with devcontainers).

**.devcontainer/devcontainer.json** (full development environment):

```json
{
    "name": "BMC Development",
    "image": "mcr.microsoft.com/devcontainers/base:alpine",
    "features": {
        "ghcr.io/devcontainers/features/nix:1": {
            "flakes": true,
            "extraNixConfig": "experimental-features = nix-command flakes"
        }
    },
    "postCreateCommand": "nix develop --command bash -c 'cd frontend && yarn install'",
    "postStartCommand": "nix develop",
    "customizations": {
        "vscode": {
            "extensions": [
                "jnoortheen.nix-ide",
                "rust-lang.rust-analyzer",
                "biomejs.biome",
                "esbenp.prettier-vscode"
            ]
        }
    }
}
```

Since `default` is now the `full` shell with everything, one devcontainer config suffices. GUI rendering won't work in
containers (no GPU/display), but Rust compilation and frontend builds work fine.

## Migration Path

1. Extract common deps into shared definition (avoid duplication with workspace.nix)
2. Add `full` devShell using `buildFHSEnv` (becomes default)
3. Remove `fast` shell (redundant with full)
4. Test in Docker container (see Testing section)
5. Add devcontainer config
6. Add testing script to `scripts/`
7. Delete `bmc-mock-display/shell.nix`
8. Document usage

## Resolved Questions

- **What exact native libs does biome/sass need?**
    - biome: glibc basics (libgcc_s, libpthread, libm, libdl, libc)
    - sass-embedded: same glibc basics (uses bundled Dart VM)
    - The musl variant is ignored (already handled in yarn-files.nix)
    - **Real problem**: ELF interpreter `/lib64/ld-linux-x86-64.so.2` missing on NixOS/Guix → solved with `buildFHSEnv`

- **Shell structure?**
    - Simplified: `full` (default) + `armv7-*` (cross-compile) only
    - Separate `fast`/`display`/`frontend` shells add complexity without clear benefit

- **Duplication with workspace.nix?**
    - Extract common deps into shared definition, use in both places

## Testing

Testing script location: `scripts/test-devshell.sh`

Use a clean `nixos/nix` Docker container to verify the shell provides all necessary dependencies:

```bash
#!/usr/bin/env bash
# scripts/test-devshell.sh - Test devShell in clean Docker environment

set -euo pipefail

docker run -it --rm -v "$(pwd)":/workspace -w /workspace nixos/nix bash -c '
  # Enable flakes
  mkdir -p ~/.config/nix
  echo "experimental-features = nix-command flakes" > ~/.config/nix/nix.conf

  echo "=== Testing default (full) devShell ==="

  nix develop --command bash -c "
    set -e
    echo \"--- Rust build ---\"
    cargo build -p bmc-shared-utils

    echo \"--- Frontend (tests FHS compat for node binaries) ---\"
    cd frontend && yarn install && yarn build
  "

  echo "=== All tests passed ==="
'
```

Note: GUI rendering won't work in Docker (no display), but all compilation and frontend tooling should work.

## Breaking Changes Analysis

| Change                                          | Impact                           | Risk                 |
|-------------------------------------------------|----------------------------------|----------------------|
| `workspace.nix` signature (+commonDeps param)   | Only imported from flake.nix     | None - internal      |
| Remove `fast` shell                             | Anyone using `.#fast` explicitly | Low - not documented |
| `default` shell changes (mkShell → buildFHSEnv) | Different shell environment      | Low - additive       |
| Packages (`bmc-mock`, `frontend`, etc.)         | Unchanged                        | None                 |
| `armv7-*` shells                                | Unchanged                        | None                 |

**External consumers (bos-main):** Safe. bos-main only uses packages via flake input:

```nix
inputs.bmc-main.packages.${localSystem}."bmc-openwrt-${arch}-release"
inputs.bmc-main.packages.${localSystem}.frontend
```

These packages are unchanged by this proposal.

---

## Implementation Instructions

> **IMPORTANT:** When implementing this proposal, ASK THE USER before making any choices not explicitly
> decided in this document. Do not guess or make assumptions. If something is unclear or has multiple
> valid approaches, stop and ask.

### Step-by-step implementation

1. **Modify `flake.nix`:**
    - Add `commonDeps` definition in the `let` block (see section 3 for exact structure)
    - Pass `commonDeps` to workspace.nix import: `{ inherit self pkgs commonDeps; }`
    - Replace current `default` devShell with `buildFHSEnv`-based `full` shell (see section 1)
    - Remove `fast` from devShells export (keep `armv7-*`)

2. **Modify `workspace.nix`:**
    - Update function signature: `{ self, pkgs, commonDeps }:`
    - Replace inline `nativeDeps` with `commonDeps.buildDeps`
    - Replace inline `env` with `commonDeps.env`

3. **Create `scripts/test-devshell.sh`:**
    - Copy the script from Testing section
    - Make executable: `chmod +x scripts/test-devshell.sh`

4. **Create `.devcontainer/devcontainer.json`:**
    - Copy the JSON from section 6

5. **Delete `bmc-mock-display/shell.nix`**

6. **Update `README.md`:**
    - Document how to use `nix develop`
    - Document ARM cross-compile shells

7. **Test:**
    - Run `scripts/test-devshell.sh`
    - Verify Docker test passes

### Decisions already made (do not re-ask)

- Use `buildFHSEnv` for the full shell (solves ELF interpreter problem)
- Define `commonDeps` in `flake.nix`, pass to `workspace.nix`
- Shell structure: `full` (default) + `armv7-*` only (no separate fast/display/frontend)
- Testing script goes in `scripts/`
- Single devcontainer config using default shell
- CI integration is out of scope

### If unsure, ASK about

- Exact Nix syntax if the examples don't compile
- Whether to include additional packages not listed in `commonDeps`
- Any errors encountered during testing
- README documentation wording/structure
- Anything not explicitly covered in this document
