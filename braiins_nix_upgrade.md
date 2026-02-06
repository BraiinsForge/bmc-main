# Miniminer Nix-Based Upgrade System

## Overview

This document describes a Nix-based upgrade workflow for the Miniminer, enabling decoupled release management between the Braiins OS (BOS) base system and application layer (display software, widgets, custom packages).

The system provides:

* Independent versioning and release of applications without requiring BOS updates
* Transparent user experience (appears as standard firmware update, without the need to wait for reboots in some cases)
* Automatic dependency resolution across multiple servers
* Easy rollback and factory reset capabilities
* Package manager behavior with garbage collection

---

## Architecture

### Example servers

```
┌────────────────────────────────────────────────────────────────────────────┐
│             Example servers (binary cache - Attic for example)             │
├───────────────────┬────────────────────────┬───────────────────────────────┤
│   BOS Server      │   APP A Server (Forge) │      APP B Server (Community) │
├───────────────────┼────────────────────────┼───────────────────────────────┤
│ - OpenWrt base    │ - Display app          │ - Community widgets           │
│ - System libs     │ - Widgets              │ - Widget deps                 │
│ - Core deps       │ - Widget deps          │ - Third-party packages        │
│ - Braiins widgets │ - ...                  │ - ...                         │
└───────────────────┴────────────────────────┴───────────────────────────────┘
                                       │
                                       ▼
                              ┌─────────────────┐
                              │   Miniminer     │
                              │   (Target)      │
                              │  STM32MP157C    │
                              │    OpenWrt      │
                              └─────────────────┘
```

### Nix Strategy: Flakes + Binary Cache

After analyzing available options, the recommended approach combines:

1. **Nix Flakes** - For reproducible, locked dependency management
2. **Binary Cache with NAR/narinfo** - Standard Nix binary cache format (nix-serve, Attic, Cachix, etc.)
3. **Custom Profile Management** - Symlink-based profile management with generation-based rollback (not using `nix profile`)

**Rationale:**

* Flakes provide reproducible builds/packages with `flake.lock`
* Custom profile management provides generation-based rollback via
  atomic symlink replacement. The nix profile utility doesn't work
  well with storepaths (not possible to update all packages at once)
* NAR/narinfo is the standard binary cache format - works out of the box with Nix tooling
* All dependencies are resolved automatically by Nix

---

## Binary Cache Structure

Binary caches use the standard Nix format with NAR archives and narinfo metadata files.

### Server Layout

```
https://cache.braiins.com/
├── nix-cache-info                    # Cache metadata
├── v1/
│   └── miniminer-index.json         # Root index (package list)
├── <hash>.narinfo                    # Package metadata files
└── nar/
    ├── <hash>.nar.xz                # Compressed NAR archives
    └── <hash>.nar.zst
```

### Flake Lockfiles

There is a single flake.lock file that everything else stems from. This allows us to build all the widgets
against the same version of nixpkgs.

For ease of use, all widgets are kept in a single repository with the flake, but in future, this can be expanded
also to other repositories.

The build process produces both the packages and the index that gets
published to the server. This allows to capture the whole closure,
copying only the resulting index.json to the server's store.

### Dependencies

Each application in the Nix cache comes with all its runtime dependencies (libraries, drivers, etc.). When an application is downloaded, all of its dependencies are fetched automatically.

Some dependencies are *propagated* — they are pulled into the profile's symlink tree alongside the application itself when the profile is built. This is the Nix mechanism for making shared libraries and other resources visible to all packages in the profile.

**Important:** BOS is NOT a Nix dependency. Applications do not depend on BOS through Nix. BOS compatibility is checked separately via `min_bos_version` in the package index.

---

## Supplementary Metadata Index

Since the Nix cache itself doesn't include application-specific
metadata (like `min_bos_version`), we maintain a lightweight
supplementary index. This lists the latest available versions of
packages.

### Index Structure

**Location:** `<https://<server>>/v1/miniminer-index.json`

```json
{
  "version": 1,
  "provenance": {
    "commit": "a1b2c3d4e5f6789..."
  },
  "indexes": [
    "https://other-server.example.com/miniminer-index.json",
    "https://community-cache.example.com/miniminer-index.json"
  ],
  "packages": {
    "miniminer-display": {
      "latest": "2.1.0",
      "store_path": "/nix/store/abc123def456-bmc-2.1.0",
      "min_bos_version": "26.01",
      "category": "core",
      "description": "Main display application for the Deck"
    },
    "hashrate-widget": {
      "latest": "1.2.0",
      "store_path": "/nix/store/xyz789ghi012-hashrate-widget-1.2.0",
      "min_bos_version": "25.10",
      "category": "widget",
      "description": "Widget showing current hashrate statistics"
    }
  }
}
```

It is expected that the server will ensure that the indexes do not
conflict between each other. Such as, there shouldn't be a package
with the same version listed twice. A package could be listed twice
with differing versions. The device should then choose latest.

**Key fields:**

* `version` - Version of the index itself
* `provenance` - Build provenance information:
  * `commit` - Git commit hash from which the index was built
* `indexes` - List of URLs pointing to other index pages (for federated package discovery)
* `packages` - Available packages with their metadata:
  * `version` - Latest available version
  * `store_path` - Nix store path for direct `nix copy` from binary cache
  * `min_bos_version` - Minimum BOS version required (YY.MM format, e.g., "26.01")
  * `category` - Package category (display, widget, etc.)
  * `description` - Human-readable package description

---

## Device-Side Configuration

### Binary Cache Configuration

**Location:** `/etc/nix/nix.conf`

```
substituters = https://cache.braiins.com https://apps-cache.braiins.com
trusted-public-keys = cache.braiins.com:AAAAB3NzaC1... apps-cache.braiins.com:BBBBB4NzaC2...
```

### Server Registry

Additional metadata about servers (priority, enabled state).

**Location:** `/etc/nix-upgrade/servers.json`

```json
{
  "servers": [
    {
      "id": "bos_server",
      "type": "bos",
      "cache_url": "https://cache.braiins.com",
      "index_url": "https://cache.braiins.com/miniminer-index.json",
      "public_key": "cache.braiins.com:AAAAB3NzaC1...",
      "priority": 1,
      "enabled": true
    },
    {
      "id": "app_a_server",
      "type": "application",
      "cache_url": "https://apps-cache.braiins.com",
      "index_url": "https://apps-cache.braiins.com/miniminer-index.json",
      "public_key": "apps-cache.braiins.com:BBBBB4NzaC2...",
      "priority": 2,
      "enabled": true
    }
  ]
}
```

The packages will be resolved by order of the priority to prevent
shadowing of the official packages.

### Installed Packages Registry

Package versions are tracked in individual profiles in a `manifest`
file. The file is a json that depicts all the packages versions used
to build the profile. The profile generations are identified by their
profile number they have in their name.

Rollbacks are performed by switching to a lower generation.

---

## Installation/Upgrade Workflow

There is a single button for upgrade of the whole system, including
all the widgets and the Braiins OS. The user should be told what the
upgrade is going to need (ie. BOS upgrade takes longer due to reboot)

**Upgrade** means bumping versions of already-installed packages. It is
not possible to upgrade individual packages — upgrading always updates
all components of the system (BOS + all applications).

**Install** means adding a new package that wasn't previously
installed. Users can install individual packages without upgrading
existing ones, as long as the current BOS satisfies the package's
`min_bos_version`.

### Complete Upgrade Flow Diagram


```
┌──────────────────────────────────────────────────────────────────┐
│ 1. User Selection & BOS Check                                    │
│    - System fetches miniminer-index.json from all servers        │
│    - User browses and selects applications in web UI             │
│    - Check min_bos_version against current BOS                   │
│    - If newer BOS needed: offer only full upgrade                │
└───────────────────────────────┬──────────────────────────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────────────┐
│ 2. Copy Store Paths (nix copy)                                   │
│    - Collect store paths from the index                          │
│    - Fetch packages and all dependencies from binary cache       │
│    - If store paths not available: abort                         │
└───────────────────────────────┬──────────────────────────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────────────┐
│ 3. Profile Construction                                          │
│    - Symlink all package files into a single profile directory   │
│    - Compute profile manifest (packages, versions, store paths)  │
│    - Compute final activation script from individual scripts     │
└───────────────────────────────┬──────────────────────────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────────────┐
│ 4. BOS Upgrade (if min_bos_version not satisfied)                │
│    - BOS upgrade happens BEFORE the profile switch               │
└───────────────────────────────┬──────────────────────────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────────────┐
│ 5. Activation & Atomic Profile Switch                            │
│    - Atomically replace current profile symlink                  │
│    - Execute activation scripts in dependency order              │
│    - Previous generation remains on disk for rollback            │
└──────────────────────────────────────────────────────────────────┘
```

### Phase 1: User Selection & BOS Check

1. System fetches `miniminer-index.json` from all enabled servers
2. User browses available applications in web UI
3. User selects applications to install or update
4. For each selected app, check `min_bos_version` against current BOS (`/etc/bos-version`). If newer is necessary, full upgrade has to be performed
5. User clicks "Install Selected" or "Upgrade System & Install Selected"

### Phase 2: Copy store paths from index using nix-copy

The store paths are collected from the indexes. Then, they're fetched
through nix-copy. Nix queries the binary cache for the computed store
path.

If the store paths are not available, the whole process is aborted.

### Phase 3: Profile construction

From all the collected packages, a profile is built. This means that
all the files from the packages are taken and symlinked together to a
single folder.

Apart from that, a manifest of the profile is computed to know the
packages that went inside of the profile.

And lastly, a final activation script is computed for the profile,
executing all the individual activation scripts in computed order.
See Phase 5 for detailed description of the activation itself.

#### Manifest

The manifest kept inside of the profile should be similarly structured
as the indexes the packages are downloaded from. Each package has its
name, store_path, version and description specified.

This could allow for partial upgrades if we wanted to support those,
since all information necessary to build a new profile is kept. Also
the upgrades will be performed thanks to them, looking at the packages
based on the names.

### Phase 4: BOS Upgrade

Packages will have a minimum BOS version requirement. When this requirement is not satisfied, the user has to first upgrade.
During the upgrade, both BOS and the application parts managed through Nix are upgraded to latest version.

**Important:**

* BOS upgrade happens BEFORE the profile switch (step 5)
* BOS is NOT a Nix dependency — it's checked separately via `min_bos_version`

**User experience:** User selected "Install Miniminer Display v2.1.0" and sees installation progress.

### Phase 5: Activation, atomic profile switch

Each package can ship with multiple activation scripts. The activation scripts should:
1. Get the system to the state where the package can be ran
2. Actually run what's necessary automatically

For example, when upgrading the main BMC compositor, first, the bmc service is put to place in /etc/init.d.
Then the BMC is (re)started.

### Activation structure

```
core/
  activation/
    bmc-service.json
    bmc-service.sh
    bmc-start.sh
    bmc-start.json
```

Each activation script has its own json. That specifies relations to other services.
```
{
  "before": [ "serviceRestarts" ], // The activation always runs before serviceRestarts
  "after": [ "writeBoundary" ] // The service always runs after writeBoundary
}
```

The activation scripts run with these environment variables:
```
BMC_OLD_GENERATION=/path/to/old/generation
BMC_NEW_GENERATION=/path/to/new/generation
```
These allow for example to restart services.

### Final Activation

The final activation script is computed from the individual ones
during build of the profile based on the given constraints (after,
before) - Phase 3.

Part of the activation performs an atomic switch to the new
application version. (to the new profile)

The replacement is done only after all activation scripts that perform
checks succeed. The individual checks depend on the nature of the
packages.

---

## Configuration Preservation

User configurations survive upgrades through:

**Profile Generations** - Previous generations preserved on disk for rollback

TODO OpenWrt configuration

---

## Custom Profile Management

Instead of using `nix profile`, the system uses a custom Rust-based
profile management implementation. This provides more control over the
symlink structure, avoids using Nix evaluations.

### Overview

The custom profile manager:

* Builds unified symlink trees from multiple Nix store paths
* Manages generation directories for instant rollback
* Performs atomic profile switches via symlink replacement
* Tracks installed packages in profile's generation manifest

### Profile Structure

```
/nix/var/nix/gcroots/profiles/
├── bmc/                           # Application profiles
│   ├── bmc-1/                     # Generation 1 (factory)
│   │   ├── bin/
│   │   │   ├── miniminer-display -> /nix/store/xxx-miniminer-display-2.0.0/bin/miniminer-display
│   │   │   └── widget-runner -> /nix/store/yyy-widgets-1.0.0/bin/widget-runner
│   │   ├── lib/
│   │   │   └── ... -> symlinks to store paths
│   │   └── share/
│   │       └── ... -> symlinks to store paths
│   ├── bmc-2/                     # Generation 2
│   ├── bmc-3/                     # Generation 3 (current)
│   └── current -> bmc-3           # Atomic symlink to active generation
```

### Installation flow

When installing or updating packages:

1. **Collect store paths** - Gather all store paths for packages to be included in the profile
2. **Build unified symlink tree** - Walk each store path and merge all files/directories under standard paths (bin, lib, share, etc.), creating symlinks pointing to the actual files in the Nix store
3. **Handle conflicts** - If two packages provide the same file, apply conflict resolution (priority-based or error)
4. **Resolve activation scripts** - Based on dependency rules between activation scripts, determine the correct order and content of activation scripts, and add them to the profile
5. **Create manifest** - Generate a `manifest` file in the profile that captures all package versions installed, including their store paths
6. **Atomic switch** - Replace the `current` symlink to point to the new generation directory
7. **Activation** - Run activation scripts

### Atomic Symlink Replacement

The switch to a new generation is atomic at the filesystem level.
This ensures the system is never in an inconsistent state — the profile either points to the old generation or the new one, never to a partially-built state.

See the Installation/Upgrade Workflow section for the full installation flow (phases 3 and 5).

### Generation Management

* **Creation** - Each install/update/remove operation creates a new generation
* **Retention** - Configurable number of generations kept (default: 2 + factory)
* **Protection** - Factory generation (generation 1) is never garbage-collected
* **Cleanup** - Old generations can be removed to free disk space (see Garbage Collection)

---

## Rollback Mechanism

The custom profile management maintains generation directories on
disk. Each generation is a complete symlink tree that can be switched
to instantly.

Only rollbacks to previous profile generations existing on the disk
are possible. Similarly to switch to a new profile generation, the
activation of the older generation is ran.

**Available operations:**
* List available generations (stored in `/nix/var/nix/gcroots/profiles/`)
* Rollback to previous generation (instant symlink switch)
* Rollback to specific generation by number

---

## Factory Reset

TODO

---

## Garbage Collection

**Requirements:**

On the Deck, there should always be enough space for the whole world -
in case all the derivations got changed, like due to a change in the
compiler or glibc. This guarantees that we're always able to install
new version of the software even if everything changed. Afterwards the
previous versions could be garbage collected.

It will have to be calculated how much space is taken by currently
used packages and try to garbage collect, according to some other
given constraints, enough space for.

**Proposed Configuration:** `/etc/nix-upgrade/gc.json`

Possible options
```json
{
  "keep_generations": 2,
  "keep_days": 60,
  "min_free_space": "100M",
  "protected_generations": [1]
}
```

* `keep_generations` - Minimum number of generations to keep
* `keep_days` - Remove generations older than this (days)
* `min_free_space` - Try to guarantee this much free space
* `protected_generations` - Generations that are never removed (e.g., factory)

**Operations:**

* **Remove package** - Uninstall a package from the current profile (rebuilds profile without it, keeps shared deps in store)
* **Delete old generations** - Remove generation directories older than configured threshold
* **Nix store garbage collection** - Run `nix-collect-garbage` to remove unreachable store paths

---

## User Experience

* The BOS and app upgrades run after each other.
* The user does not see that both are updated at the same time.
* The user cannot choose upgrading only one or the other.

### Web UI for Package Management

Users interact through a web UI to:

* Browse and select applications to install/update
* View changelogs and manage installed applications
* User always needs to upgrade all installed applications in bulk

### Version Display

* BMC version shown to user (the main core packages version, user friendly, ie. 26.01)
* Individual application version shown to user (e.g., "Miniminer Display v2.1.0")
* BOS version hidden (available in advanced settings, stored in `/etc/bos-version`)

---

## Signature Verification

NAR archives are signed using Ed25519 keys (in narinfo `Sig` field). Trusted keys configured in `/etc/nix/nix.conf`:

```
trusted-public-keys = cache.braiins.com:AAAAB3NzaC1... apps-cache.braiins.com:BBBBB4NzaC2...
```

Nix refuses packages with invalid/missing signatures. Multiple signatures supported for multi-party trust.

---

## Error Handling

### Potential Failure Points

If the upgrade fails in any step, the previous profile is kept in use.

| Failure | Detection | Recovery |
| --- | --- | --- |
| Network failure  | Incomplete NAR  | Retry, resume partial  |
| BOS upgrade fails  | Install error  | Show error, abort  |
| Insufficient storage  | Pre-flight check  | Run GC first  |
| Activation failure  | Non-zero exit  | Auto rollback  |
| Invalid signature  | Nix verification  | Reject package  |
| Corrupt download  | NarHash mismatch  | Re-download  |

---

## Future Features

* **Custom server list** - UI for adding third-party servers with key management
* **Package/server blacklist** - Warn users about suspicious packages or domains

---

## Summary

This Nix-based upgrade system provides:

1. **Decoupled Releases** - Applications and BOS can be released independently
2. **User Transparency** - Complex operations hidden behind simple UI
3. **Reliable Upgrades** - Atomic operations with automatic rollback
4. **Flexible Package Management** - Per-widget installation and updates
5. **Configuration Preservation** - User settings survive upgrades
6. **Multi-Server Support** - Dependencies resolved across organizations
7. **Reproducibility** - Flakes ensure consistent builds via lockfiles
8. **Security** - Built-in signature verification via narinfo
