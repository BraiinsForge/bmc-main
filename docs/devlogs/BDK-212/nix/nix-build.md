# Nix Build Infrastructure

## Overview

This document describes the Nix derivations that bridge between
building individual packages (Rust crates, widgets, etc.) and
producing the artifacts needed by the device-side upgrade system
described in [nix-concepts.md](nix-concepts.md).

The build infrastructure produces three kinds of artifacts:

1. **Packages** — individual derivations (widgets, bmc-openwrt, etc.)
2. **Index** (`miniminer-index.json`) — metadata listing all available
   packages and their store paths
3. **Tarball** (`.tar.gz`) — initial Nix store snapshot for device
   initialization, containing packages and a pre-built profile

The factory index (`miniminer-factory.json`) can be built either by
Nix (`mkFactoryIndex.nix`, for local testing with placeholder URLs)
or by CI tooling (`scripts/build-factory-index.sh`, assembling real
download URLs from tarball `metadata.json` files).

---

## File Structure

All Nix build files live under `nix/`. Each file exports a single
function, named after the file:

```
nix/
├── workspace.nix          # Rust crates, build profiles, devShells
├── mkWidgetPackage.nix    # Build a widget crate into a package derivation
├── mkIndex.nix            # Package list → index.json
├── mkTarball.nix          # Package list + bmc-nix CLI → .tar.gz + metadata
├── mkFactoryIndex.nix     # Tarball entries → miniminer-factory.json
└── init-artifacts.nix     # Package list, index, tarball, and factory index
```

`flake.nix` imports these files and passes shared arguments (`pkgs`,
`commonDeps`, etc.). `artifacts.nix` is the single source of truth
for what packages are released and how they're assembled into the
index and tarball.

---

## Data Flow

```
   Any build process (workspace profiles, mkWidgetPackage, callPackage, etc.)
        │
        │  produces package derivations
        ▼
   [ { pkg = <drv>; name; version; category; ... } ]
        │
        ├──────────────────────────────────┐
        ▼                                  ▼
   ┌──────────────┐                ┌──────────────────┐
   │ mkIndex       │→ index.json   │ mkTarball         │→ .tar.gz
   └──────────────┘                │                   │  + metadata.json
                                   │ 1. generates      │
                                   │    temp index     │
                                   │ 2. invokes        │
                                   │    bmc-nix CLI    │
                                   │    to build       │
                                   │    profile        │
                                   │ 3. captures       │
                                   │    closure        │
                                   └──────────────────┘
                                           │
                                           ▼
                                   (external tooling collects
                                    metadata.json from multiple
                                    versions → factory.json)
```

The **package list** is defined once and feeds both `mkIndex` and
`mkTarball`. Adding a package to the system means adding one entry
to the list.

---

## Package List Format

The central input to `mkIndex` and `mkTarball` is a list of package
entries. Each entry pairs a Nix derivation with its metadata:

```nix
[
  {
    pkg = <derivation>;       # the built package (any valid Nix derivation)
    name = "miniminer-display";
    version = "2.1.0";
    category = "core";        # "core", "widget", etc.
    description = "Main display application";
    upgrade_strategy = "reboot"; # "reboot" or null
    install_strategy = null;
    # cache = "default";    # optional — defaults to first entry in caches
  }
  {
    pkg = <derivation>;
    name = "digital-clock";
    version = "1.0.0";
    category = "widget";
    description = "Digital clock widget";
    upgrade_strategy = null;
    install_strategy = null;
  }
]
```

The `pkg` field is an actual Nix derivation. The function extracts
the store path from it. Everything else is explicit metadata — it is
not derived from the package contents.
Compatibility checks are handled by checker packages; see
"Checker Packages" in [nix-concepts.md](nix-concepts.md).

How packages are built (via `mkWidgetPackage`, workspace profiles,
`callPackage`, etc.) is irrelevant to the index/tarball pipeline.

The optional `cache` field selects which binary cache hosts this
package. If omitted, `mkIndex` defaults it to the first entry in
`caches` (or `"default"` when `caches` is empty).

---

## Functions

### workspace.nix

**Existing file, moved from repo root to `nix/`.**

Defines Rust crate builds, cross-compilation profiles, and
development shells. See [the current workspace.nix](../../../../workspace.nix)
for the full implementation.

Responsibilities:
- Crate definitions via `pkgs.ii.rust.defineCrate`
- Build profiles: `fast`, `armv7-release`, `armv7-debug`,
  `armv7-glibc-release`, `armv7-glibc-debug`
- Development shells for each profile
- Direct package builds (bmc-mock, bmc-openwrt, etc.)

```nix
# nix/workspace.nix
{ self, pkgs, commonDeps }:
# Returns: { packages, devShells }
```

No changes to this file's API are needed for the index/tarball
pipeline. It just produces derivations that go into the package list.

---

### mkWidgetPackage.nix

**Extracted from workspace.nix.**

Builds a single widget Rust crate into a package derivation with the
standard directory layout expected by the device.

Note: The `mkWidgetPackage` defined here is the **internal** API, used
within our monorepo (takes `crate` + `profile` from the workspace).

```nix
# nix/mkWidgetPackage.nix
{ pkgs, lib }:
{ name              # widget name (e.g. "digital-clock")
, crate             # crate definition from workspace
, profile           # build profile to use
, features ? []     # cargo features to enable
, wrapWithLibs ? false  # wrap binary with LD_LIBRARY_PATH
}:
# Output: derivation with structure:
#   $out/lib/bmc-widgets/<name>/bin/<binary>
#   $out/lib/bmc-widgets/<name>/manifest.json
#   $out/lib/bmc-widgets/<name>/assets/  (if present)
```

This is one way to produce a package derivation. The resulting
derivation can be used as a `pkg` entry in the package list.

---

### mkIndex.nix

Builds a `miniminer-index.json` from a list of package entries.

```nix
# nix/mkIndex.nix
{ pkgs, lib }:
{ packages          # [ { pkg; name; version; category; description;
                    #     upgrade_strategy; install_strategy; } ]
, caches ? []       # [ { name; cache_url; cache_key; } ]
, indexes ? []      # [ "https://..." ] — federated index URLs
, commit ? ""       # git commit hash for provenance field
}:
# Output: derivation producing $out/index.json
```

**Implementation approach:**

Uses `pkgs.runCommand` (or `pkgs.writeText` with `builtins.toJSON`).
For each entry in `packages`, extracts the store path from `pkg` and
combines it with the metadata to form the JSON structure defined in
[nix-concepts.md](nix-concepts.md#index-structure).

The output follows this schema:

```json
{
  "version": 1,
  "provenance": { "commit": "<git-hash>" },
  "indexes": [],
  "caches": [
    { "name": "default", "cache_url": "...", "cache_key": "..." }
  ],
  "packages": [
    {
      "name": "...",
      "version": "...",
      "cache": "default",
      "store_path": "/nix/store/<hash>-<name>-<version>",
      "category": "...",
      "description": "...",
      "upgrade_strategy": "...",
      "install_strategy": null
    }
  ]
}
```

This is a lightweight derivation — pure JSON generation, no
compilation or special tooling.

---

### mkTarball.nix

Builds an initial Nix store tarball for device initialization.

```nix
# nix/mkTarball.nix
{ pkgs, lib, mkIndex }:
{ packages          # same format as mkIndex
, bmc-nix-cli       # derivation of the bmc-nix CLI tool
, bos_version       # "26.01"
, profile_path ? "/nix/var/nix/gcroots/profiles/bmc"
, extraFiles ? null  # optional derivation whose contents are overlaid
                     # into the tarball root (e.g. /etc/nix/nix.conf)
}:
# Output: derivation producing:
#   $out/nix-<bos_version>.tar.gz
#   $out/metadata.json
```

**Implementation approach:**

This is the most involved derivation. Steps inside the build:

1. **Generate temporary index** — call `mkIndex` with the same
   `packages` to produce an `index.json`. This gives the `bmc-nix`
   CLI the input it needs.

2. **Build profile** — invoke `bmc-nix-cli build-profile` with the
   generated index. See [bmc-nix-cli.md](bmc-nix-cli.md) for the
   full CLI specification. This produces the symlink-based profile
   directory (symlink tree, hooks, manifest) as described in
   [nix-concepts.md](nix-concepts.md#custom-profile-management).
   Use `--generation 1`, and point `--profile-dir` into the tarball
   root (e.g., `$rootDir${profile_path}`) so files land inside the
   archive.

   **Note:** The profile is NOT activated during the tarball build.
   Activation scripts reference absolute system paths (`/etc/init.d/`,
   etc.) that don't exist under the build sandbox. The `current`
   symlink is also not set. On first boot, the `bmc-nix-initializer`
   detects there is no active profile and activates the latest
   generation using the `bmc-nix` library directly. See
   [nix-openwrt-services.md](nix-openwrt-services.md#bmc-nix-initializer).

3. **Capture closure** — use `pkgs.closureInfo` to compute the full
   runtime closure of all packages. This is a standard nixpkgs
   utility (`pkgs/build-support/closure-info.nix`) that uses
   `exportReferencesGraph` under the hood. It produces:
   - `store-paths` — one store path per line, the complete closure
   - `registration` — nix-store database registration entries
     (suitable for `nix-store --load-db`)
   - `total-nar-size` — aggregate NAR size

   The `closureInfo` derivation is built separately and its outputs
   are referenced in the tarball build step:

   ```nix
   closureInfo = pkgs.closureInfo {
     rootPaths = map (p: p.pkg) packages;
   };
   ```

4. **Populate Nix DB** — use `nix-store --load-db` with a local
   store root to build the SQLite database from the registration
   data. This uses Nix's `local?root=` store backend to write into
   an isolated directory without touching the build machine's store:

   ```bash
   export NIX_REMOTE=local?root=$rootDir
   nix-store --load-db < ${closureInfo}/registration
   ```

   The `mkTarball` derivation needs `nix` and `gzip` in
   `nativeBuildInputs` for the `nix-store` command and tarball
   compression respectively.

   This produces `$rootDir/nix/var/nix/db/db.sqlite` — a fully
   populated store database. After extraction on the device, the
   store is immediately functional with no further initialization
   needed.

5. **Create tarball** — assemble a root directory and tar it up:
   - All store paths listed in `${closureInfo}/store-paths`
   - The populated Nix DB at `nix/var/nix/db/db.sqlite`
   - The built profile directory at `profile_path` — this lives
     outside `/nix/store` (it is a directory of symlinks pointing
     into the store, assembled by `bmc-nix-cli` in step 2) and is
     archived at its relative path within the tarball root
   - Extra files from `extraFiles` (if provided), overlaid at the
     root (e.g., `etc/nix/nix.conf`)
   - Compress with `gzip`

   Example usage of `extraFiles`:

   ```nix
   extraFiles = pkgs.writeTextDir "etc/nix/nix.conf" ''
     substituters = https://cache.braiins.com
     trusted-public-keys = cache.braiins.com:AAAAB3...
   '';
   ```

6. **Write metadata** — produce `metadata.json` alongside the
   tarball:

```json
{
  "bos_version": "26.01",
  "profile_path": "/nix/var/nix/gcroots/profiles/bmc",
  "tarball_name": "nix-26.01.tar.gz"
}
```

This metadata is consumed by external tooling to build the factory
index (`miniminer-factory.json`) across multiple versions. The
external tooling must supply the `download_url` field (not present
in `metadata.json`) when assembling `miniminer-factory.json`, since
it depends on where the tarball is ultimately published.

---

## artifacts.nix

The package list, index, and tarball definitions are extracted into
`nix/artifacts.nix` to keep `flake.nix` lean. This file is the single
source of truth for what gets released.

```nix
# nix/artifacts.nix
{ self, pkgs, lib, workspace, mkWidgetPackage, mkIndex, mkTarball
, armv7Pkgs  # pkgs.pkgsCross.armv7l-hf-multiplatform (glibc; widgets
             # use armv7-glibc-release profile — see workspace.nix)
}:
let
  # Build individual packages (all ARMv7 for the target device)
  bmc-app = workspace.packages.bmc-openwrt-armv7-release;
  nix = armv7Pkgs.nix;
  digital-clock = mkWidgetPackage {
    name = "digital-clock";
    crate = workspace.crates.widget-digital-clock;
    profile = workspace.build-profiles.armv7-glibc-release;
    features = [ "standalone" ];
  };

  # The package list — single source of truth
  packageList = [
    {
      pkg = bmc-app;
      name = "miniminer-display";
      version = "2.1.0";
      category = "core";
      description = "Main display application";
      upgrade_strategy = "reboot";
      install_strategy = null;
    }
    {
      pkg = nix;
      name = "nix";
      version = nix.version;
      category = "core";
      description = "Nix package manager";
      upgrade_strategy = "reboot";
      install_strategy = null;
    }
    {
      pkg = digital-clock;
      name = "digital-clock";
      version = "1.0.0";
      category = "widget";
      description = "Digital clock widget";
      upgrade_strategy = null;
      install_strategy = null;
    }
  ];

  index = mkIndex {
    packages = packageList;
    caches = [{
      name = "default";
      cache_url = "https://cache.braiins.com";
      cache_key = "cache.braiins.com:AAAAB3NzaC1...";
    }];
    commit = self.rev or "dirty";
  };

  tarball = mkTarball {
    packages = packageList;
    # bmc-nix-cli is a [[bin]] target of the bmc-nix crate;
    # workspace.nix needs to export it as a package
    bmc-nix-cli = workspace.packages.bmc-nix-cli;
    bos_version = "26.01";
    extraFiles = pkgs.writeTextDir "etc/nix/nix.conf" ''
      substituters = https://cache.braiins.com
      trusted-public-keys = cache.braiins.com:AAAAB3NzaC1...
    '';
  };
in
{
  inherit index tarball;
}
```

### flake.nix integration

`flake.nix` imports all the Nix files and passes them to
`artifacts.nix`:

```nix
# In flake.nix (target layout, once `nix/` directory exists)
let
  workspace = import ./nix/workspace.nix { inherit self pkgs commonDeps; };
  mkWidgetPackage = import ./nix/mkWidgetPackage.nix { inherit pkgs lib; };
  mkIndex = import ./nix/mkIndex.nix { inherit pkgs lib; };
  mkTarball = import ./nix/mkTarball.nix { inherit pkgs lib mkIndex; };
  armv7Pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform;
  artifacts = import ./nix/artifacts.nix {
    inherit self pkgs lib workspace mkWidgetPackage mkIndex mkTarball armv7Pkgs;
  };
in
{
  packages.x86_64-linux = workspace.packages // {
    inherit (artifacts) index tarball;
    # nix build .#index    → result/index.json
    # nix build .#tarball  → result/nix-26.01.tar.gz
    #                        result/metadata.json
  };
}
```

---

## Open Questions

- **`mkTarball` nativeBuildInputs** — confirm that `nix`, `gzip`, and
  `bmc-nix-cli` are sufficient as build-time inputs, and that no
  additional tools (e.g., `sqlite` for DB inspection) are needed.
