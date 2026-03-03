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

The factory index (`miniminer-factory.json`) is NOT built by Nix — it
aggregates tarballs across multiple nixpkgs versions and is produced
by external tooling (CI).

---

## File Structure

All Nix build files live under `nix/`. Each file exports a single
function, named after the file:

```
nix/
├── workspace.nix          # Rust crates, build profiles, devShells
├── mkWidgetPackage.nix    # Build a widget crate into a package derivation
├── mkIndex.nix            # Package list → miniminer-index.json
├── mkTarball.nix          # Package list + bmc-nix CLI → .tar.gz + metadata
└── artifacts.nix          # Package list, index, and tarball definitions
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
    min_bos_version = "26.01";
    min_bmc_version = "1.0.0";
    upgrade_strategy = "reboot"; # "reboot" or false
    install_strategy = false;
    # cache = "default";    # optional — defaults to first entry in caches
  }
  {
    pkg = <derivation>;
    name = "digital-clock";
    version = "1.0.0";
    category = "widget";
    description = "Digital clock widget";
    min_bos_version = "26.01";
    min_bmc_version = "1.0.0";
    upgrade_strategy = false;
    install_strategy = false;
  }
]
```

The `pkg` field is an actual Nix derivation. The function extracts
the store path from it. Everything else is explicit metadata — it is
not derived from the package contents.

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
The future `deck-lib` library flake will expose a **third-party** API
with a different signature (e.g., `src`-based) for external consumers.
See [Third-Party Flake Convention](#third-party-flake-convention-future).

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
                    #     min_bos_version; min_bmc_version;
                    #     upgrade_strategy; install_strategy; } ]
, caches ? []       # [ { name; cache_url; cache_key; } ]
, indexes ? []      # [ "https://..." ] — federated index URLs
, commit ? ""       # git commit hash for provenance field
}:
# Output: derivation producing $out/miniminer-index.json
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
      "min_bos_version": "...",
      "min_bmc_version": "...",
      "category": "...",
      "description": "...",
      "upgrade_strategy": "...",
      "install_strategy": false
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
#   $out/miniminer-nix-<bos_version>.tar.gz
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
  "tarball_name": "miniminer-nix-26.01.tar.gz"
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
      min_bos_version = "26.01";
      min_bmc_version = "1.0.0";
      upgrade_strategy = "reboot";
      install_strategy = false;
    }
    {
      pkg = nix;
      name = "nix";
      version = nix.version;
      category = "core";
      description = "Nix package manager";
      min_bos_version = "26.01";
      min_bmc_version = "1.0.0";
      upgrade_strategy = "reboot";
      install_strategy = false;
    }
    {
      pkg = digital-clock;
      name = "digital-clock";
      version = "1.0.0";
      category = "widget";
      description = "Digital clock widget";
      min_bos_version = "26.01";
      min_bmc_version = "1.0.0";
      upgrade_strategy = false;
      install_strategy = false;
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
    # nix build .#index    → result/miniminer-index.json
    # nix build .#tarball  → result/miniminer-nix-26.01.tar.gz
    #                        result/metadata.json
  };
}
```

---

## Scalability: Index Generation Without All Packages

The current design requires all package derivations to be available
when `mkIndex` runs, since it extracts store paths from them. This
works well at our current scale, but may not scale indefinitely — if
the number of packages grows large, building all of them on a single
machine may become infeasible due to disk space or build time.

### Mitigation 1: Nested indexes

The index format already supports the `indexes` field — a list of
URLs pointing to other index files (see
[nix-concepts.md](nix-concepts.md#index-structure)). This allows
splitting the package set across multiple independent builds:

```nix
# Team A builds their packages and index
indexA = mkIndex {
  packages = teamAPackages;
  caches = [ ... ];
};

# Team B builds their packages and index
indexB = mkIndex {
  packages = teamBPackages;
  caches = [ ... ];
};

# Top-level index references the sub-indexes, only needs its own packages
topIndex = mkIndex {
  packages = corePackages;  # only core, not everything
  indexes = [
    "https://cache-a.example.com/v1/miniminer-index.json"
    "https://cache-b.example.com/v1/miniminer-index.json"
  ];
  caches = [ ... ];
};
```

Each sub-index is built independently (potentially on different CI
machines or in different repositories), and the top-level index just
references them. The device-side resolver already handles federated
indexes — it fetches and merges all referenced indexes.

### Mitigation 2: External index generation

If nested indexes are still insufficient (e.g., a single sub-index
itself contains too many packages for one machine), index generation
can move outside of Nix entirely. Each package build would output its
own metadata (store path, version, etc.) as a small JSON file — similar
to how `mkTarball` outputs `metadata.json`. An external tool (CI
script, dedicated service) would then collect these metadata files and
assemble the final index without needing the actual package
derivations present.

This is a future escape hatch — the current Nix-based approach should
be used until it demonstrably hits limits.

---

## Third-Party Flake Convention (future)

Third-party developers (community widget authors, etc.) need a
standard way to expose their packages so that our build pipeline can
consume them. This section proposes a convention for flake outputs.
It does not need to be implemented now.

### `deckPackages` output

Third-party flakes expose a `deckPackages` output as a **function**
that receives the consumer's package sets. This ensures all packages
are built against the same nixpkgs, guaranteeing shared library versions.

```nix
# Example third-party flake (community-widgets/flake.nix)
{
  inputs = {
    # Library flake provides mkWidgetPackage and other helpers
    deck-lib.url = "github:braiins/deck-lib";
  };

  outputs = { self, deck-lib, ... }: {
    deckPackages = { pkgs, armv7Pkgs, lib }: {
      hashrate-widget = {
        pkg = deck-lib.lib.mkWidgetPackage {
          name = "hashrate-widget";
          src = ./hashrate-widget;
          # ... widget-specific build options
        };
        name = "hashrate-widget";
        version = "1.2.0";
        category = "widget";
        description = "Widget showing current hashrate statistics";
        min_bos_version = "26.01";
        min_bmc_version = "1.0.0";
        upgrade_strategy = false;
        install_strategy = false;
      };
      pool-stats-widget = {
        pkg = armv7Pkgs.callPackage ./pool-stats { };
        name = "pool-stats-widget";
        version = "0.5.0";
        category = "widget";
        description = "Mining pool statistics display";
        min_bos_version = "26.01";
        min_bmc_version = "1.0.0";
        upgrade_strategy = false;
        install_strategy = false;
      };
    };
  };
}
```

### Function arguments

| Argument | Description |
|----------|-------------|
| `pkgs` | Build host package set (x86_64-linux) |
| `armv7Pkgs` | Cross-compilation target package set (ARMv7) |
| `lib` | nixpkgs `lib` for utility functions |

The third party uses `armv7Pkgs` for all target binaries. `pkgs` is
available for build-time tools (code generators, etc.).

Note that `deckPackages` itself does not provide build helpers — it
is purely the interface for exposing packages to consumers. Build
helpers (like widget packaging utilities) are provided separately
through a **library flake** maintained by us. Third parties add this
library flake as an input and use its functions to build widget
packages more easily. They then expose the resulting derivations
through `deckPackages`. The two mechanisms are complementary:

- **Library flake** — provides `mkWidgetPackage` and similar helpers
  for building packages that conform to the expected layout
- **`deckPackages`** — the standardized interface for exposing
  built packages to consumers

### Return value

An attrset keyed by package name. Each value has the same format as
an entry in the package list (see
[Package List Format](#package-list-format)). The `pkg` field is an
actual Nix derivation built against the provided `armv7Pkgs`.

### Consumer-side usage

```nix
# In artifacts.nix
let
  communityPkgs = inputs.community-widgets.deckPackages {
    inherit pkgs armv7Pkgs lib;
  };

  packageList = corePackages ++ lib.attrValues communityPkgs;
in
  ...
```

### Why a function, not a plain attrset

If `deckPackages` were a plain attrset, the third-party flake would
evaluate its packages against its own nixpkgs pin. This could produce
ARMv7 binaries linked against a different glibc or with incompatible
store paths. By accepting `armv7Pkgs` as an argument, the consumer
controls which nixpkgs is used, ensuring all packages share the same
toolchain and closure.

---

## Open Questions

- **Package version source of truth** — versions are currently
  specified in the package list in flake.nix. Whether to read them
  from Cargo.toml or widget manifest.json instead is left for later.

- **`mkTarball` nativeBuildInputs** — confirm that `nix`, `gzip`, and
  `bmc-nix-cli` are sufficient as build-time inputs, and that no
  additional tools (e.g., `sqlite` for DB inspection) are needed.
