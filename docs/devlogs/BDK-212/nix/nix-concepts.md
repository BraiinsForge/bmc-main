# Miniminer Nix-Based Upgrade System

## Overview

This document describes a Nix-based upgrade workflow for the Miniminer, enabling decoupled release management between
the Braiins OS (BOS) base system and application layer (display software, widgets, custom packages).

The system provides:

- Independent versioning and release of applications without requiring BOS updates
- Transparent user experience (appears as standard firmware update, without the need to wait for reboots in some cases)
- Automatic dependency resolution across multiple servers
- Easy rollback and factory reset capabilities
- Package manager behavior with garbage collection

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
3. **Custom Profile Management** - Symlink-based profile management with generation-based rollback (not using
   `nix profile`)

**Rationale:**

- Flakes provide reproducible builds/packages with `flake.lock`
- Custom profile management provides generation-based rollback via atomic symlink replacement. The nix profile utility
  doesn't work well with storepaths (not possible to update all packages at once)
- NAR/narinfo is the standard binary cache format - works out of the box with Nix tooling
- All dependencies are resolved automatically by Nix

---

## Binary Cache Structure

Binary caches use the standard Nix format with NAR archives and narinfo metadata files.

### Server Layout

```
https://cache.braiins.com/
├── nix-cache-info                  # Cache metadata
├── nix-package-index.v1.json       # Root index (package list)
├── nix-package-feed.v1.json        # Per-firmware entries (init tarballs, release indexes)
├── <hash>.narinfo                  # Package metadata files
└── nar/
    ├── <hash>.nar.xz               # Compressed NAR archives
    └── <hash>.nar.zst
```

### Flake Lockfiles

There is a single flake.lock file that everything else stems from. This allows us to build all the widgets against the
same version of nixpkgs.

For ease of use, all widgets are kept in a single repository with the flake, but in future, this can be expanded also to
other repositories.

The build process produces both the packages and the index that gets published to the server. This allows to capture the
whole closure, copying only the resulting `nix-package-index.v1.json` to the server's store.

### Dependencies

Each application in the Nix cache comes with all its runtime dependencies (libraries, drivers, etc.). When an
application is downloaded, all of its dependencies are fetched automatically.

Some dependencies are *propagated* — they are pulled into the profile's symlink tree alongside the application itself
when the profile is built. This is the Nix mechanism for making shared libraries and other resources visible to all
packages in the profile.

**Important:** BOS is NOT a Nix dependency. Applications do not depend on BOS through Nix. Compatibility checks are
handled by custom checker packages (see "Checker Packages").

---

## Supplementary Metadata Index

Since the Nix cache itself doesn't include application-specific metadata (like categories or upgrade hints), we maintain
a lightweight supplementary index. This lists the latest available versions of packages.

### Index Structure

**Location:** `<https://<server>>/nix-package-index.v1.json`

```json
{
  "version": 1,
  "provenance": {
    "commit": "a1b2c3d4e5f6789..."
  },
  "indexes": [
    "https://other-server.example.com",
    "https://community-cache.example.com"
  ],
  "caches": [
    {
      "name": "default",
      "cache_url": "https://cache.braiins.com",
      "cache_key": "..."
    }
  ],
  "packages": [
    {
      "name": "miniminer-display",
      "version": "2.1.0",
      "store_path": "/nix/store/abc123def456-bmc-2.1.0",
      "category": "core",
      "description": "Main display application for the Deck",
      "upgrade_strategy": "reboot",
      "install_strategy": null
    },
    {
      "name": "hashrate-widget",
      "version": "1.2.0",
      "store_path": "/nix/store/xyz789ghi012-hashrate-widget-1.2.0",
      "category": "widget",
      "description": "Widget showing current hashrate statistics",
      "upgrade_strategy": null,
      "install_strategy": null
    }
  ]
}
```

It is expected that the server will ensure that the indexes do not conflict between each other. Such as, there shouldn't
be a package with the same version listed twice. A package could be listed twice with differing versions. The device
should then choose latest by default in user's UI. Specific version might be used by other software.

**Key fields:**

- `version` - Version of the index itself
- `provenance` - Build provenance information:
  - `commit` - Git commit hash from which the index was built
- `caches` - List of binary caches:
  - `name` - Cache identifier referenced by packages
  - `cache_url` - URL of the binary cache
  - `cache_key` - Public key for signature verification
- `indexes` - List of URLs pointing to other index pages (for federated package discovery)
- `packages` - Array of available packages (a package name can appear multiple times with different versions):
  - `name` - Package name
  - `version` - Package version
  - `cache` - Optional cache identifier for this package. Informational only — store paths are realised through the
    substituters configured on the device, not by matching this field against `caches[]`.
  - `store_path` - Nix store path, realised on-device via `nix-store --realise` from the configured substituters
  - `category` - Package category (display, widget, etc.)
  - `description` - Human-readable package description
  - `upgrade_strategy` - This is the strategy in order to completely update the package. When reboot, the user is asked
    to reboot
  - `install_strategy` - This is the strategy in order to completely install the package.

### Package feed structure

The feed maps each BOS version to that firmware's release artifacts: the initialization tarball and, optionally, the
firmware's own package index. Initialization selects the entry's tarball; firmware upgrades follow the entry's
`index_url`. Braiins should ensure there is always an entry for the latest firmware to prevent users not getting Nix
initialized.

**Location:** `<https://<server>>/nix-package-feed.v1.json`

```json
{
  "version": 1,
  "entries": [
    {
      "bos_version": "2026-03-04-0-8436f26b-26.02",
      "download_url": "https://cache.braiins.com/v1/nix-2026-03-04-0-8436f26b-26.02.tar.gz",
      "profile_path": "/nix/var/nix/gcroots/profiles/bmc",
      "index_url": "https://cache.braiins.com/v1/2026-03-04-0-8436f26b-26.02/nix-package-index.v1.json"
    }
  ]
}
```

**Key fields:**

- `version` - Version of the package feed itself
- `entries` - One entry per BOS version:
  - `bos_version` - Full BOS version string from `/etc/bos_version` (e.g., "2026-03-04-0-8436f26b-26.02")
  - `download_url` - URL of the `.tar.gz` archive containing the initial Nix store and profile
  - `profile_path` - Path of the initial profile inside the tarball
  - `index_url` - Optional exact URL of that firmware's `nix-package-index.v1.json`; required when the entry is used for
    firmware-scoped index resolution

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
  "factory": {
      "id": "braiins_server",
      "base_url": "https://cache.braiins.com",
      "known_public_key": "cache.braiins.com:AAAAB3NzaC1...",
      "priority": 1,
      "enabled": true
  },
  "servers": [
    {
      "id": "braiins_server",
      "feed_url": "https://cache.braiins.com/nix-package-feed.v1.json",
      "known_public_key": "cache.braiins.com:AAAAB3NzaC1...",
      "priority": 1,
      "enabled": true
    },
    {
      "id": "app_a_server",
      "index_url": "https://apps-cache.braiins.com/nix-package-index.v1.json",
      "known_public_key": "apps-cache.braiins.com:BBBBB4NzaC2...",
      "priority": 2,
      "enabled": true,
      "required": false
    }
  ]
}
```

**Key fields:**

- `factory` - Server entry for the package feed (used for initialization/reset):
  - `id` - Unique server identifier
  - `base_url` - Base URL for the factory server; the client appends /nix-package-feed.v1.json to fetch the feed.
  - `known_public_key` - Public key for signature verification
  - `priority` - Resolution priority (lower = higher priority)
  - `enabled` - Whether this server is active
- `servers` - List of server entries for package indexes. Each entry links its content by exactly one of:
  - `feed_url` - Exact URL of the server's `nix-package-feed.v1.json`; the index is resolved per firmware through the
    feed entry's `index_url`
  - `index_url` - Exact URL of the server's `nix-package-index.v1.json`, fetched directly
  - `id` - Unique server identifier
  - `known_public_key` - Already known public key for signature verification. The server can also offer new keys with
    new cache servers. The keys here are already trusted.
  - `priority` - Conflict resolution priority (lower = higher priority)
  - `enabled` - Whether this server is being actively used for fetching packages
  - `required` - Optional; defaults to true. A required server's fetch or feed-resolution failure aborts the upgrade; an
    optional server degrades with a warning.

The packages will be resolved by order of the priority to prevent shadowing of the official packages. Server priorities
must be unique to avoid ambiguous resolution.

Currently the `id` field is going to be used by `installed_from`. It should be a unique identifier of the server, but it
should be static. It can be a generated UUID. This value drives conflict resolution when selecting updates (see Conflict
Resolution).

### Installed Packages Registry

Package versions are tracked in individual profiles in a `manifest` file. The file is a json that depicts all the
packages versions used to build the profile. The profile generations are identified by their profile number they have in
their name.

Rollbacks are performed by switching to a lower generation.

---

## Checker Packages

Some of the dependencies cannot be constrained by Nix itself. Nix itself is concerned with dependencies such as
libraries necessary for a binary to run. But it is not constrained by runtime dependencies, such as wayland protocol
versions or kernel drivers.

Compatibility is enforced by dedicated checker packages rather than metadata fields in the package index. These checkers
are small executables shipped via Nix and run before install/upgrade decisions. They can validate BOS/BMC compatibility,
device model constraints, or other system requirements and report whether a full BOS upgrade is required or the
operation should be blocked.

The exact checker list and result format will be defined alongside the upgrade orchestration that calls them. The key
contract is that the upgrade flow runs all relevant checker packages before copying store paths or switching profiles.

Examples include:

- Check that the compositor supports given wayland protocol version
- Check version of firmwae to see if the version is new enough

The exact implementation of this is deferred for later, but it is expected packages, such as BMC core package, can
provide files stating what they support. The checker package can then use these to check if something is supported.

It's important that everything should be checked against the next, target, version. Not against currently active
version.

---

## Conflict Resolution

Conflict resolution decides which package versions are selected when multiple servers publish the same package name. The
algorithm is:

1. If a package is already installed and the server listed in `installed_from` publishes any entry for it, consider only
   that server's entries. Other servers are consulted only when the origin server lists no entry for the package at all.
2. **Filter out downgrades.** Within the considered entries, discard any entry for a package whose version is strictly
   lower than the currently installed version. Upgrades never downgrade an installed package — this holds regardless of
   pin state or server priority. Entries with the *same* version as the installed one are kept, because their
   `store_path` may differ (e.g. a rebuild against a new toolchain or dependency) and picking up that new store path is
   a legitimate upgrade. If this filter leaves no candidates for an installed package, keep the installed version as-is
   — an origin stuck on lower versions reports the package stale rather than migrating to another server.
3. Keep only versions allowed by the package's pin constraint (see the manifest `pinned` field).
4. From what remains, choose the latest version.
5. If multiple packages remain after version selection, resolve by server priority (lower number wins). Priorities must
   be unique.
6. If multiple packages still remain, fail explicitly. This is a server-side publishing error and should not be resolved
   on-device.

This conflict resolution is independent of file-level conflicts in the profile merge (see Installation flow).

---

## File Conflict Resolution

File conflicts happen when multiple selected packages provide the same path in the profile merge (for example, two
packages ship `bin/widget-runner`). Resolution is based on the priority of the server listed in each package's
`installed_from`:

1. Prefer the file from the package whose `installed_from` server has the higher priority (lower number wins).
2. If priorities are equal, choose one based on the lexicographical order of the package names (including version) and
   continue.
3. Always log a warning when a file conflict is detected, even if it is resolved by priority. This should be treated as
   an abnormal situation.

**Example:** `bin/widget-runner` is provided by packages from `braiins_server` (priority 1) and `community_server`
(priority 3). The file from `braiins_server` is selected and a warning is logged. If both servers had priority 1, one
file would be chosen arbitrarily and a warning would still be logged.

---

## Installation/Upgrade Workflow

There is a single button for upgrade of the whole system, including all the widgets and the Braiins OS. The user should
be told what the upgrade is going to need (ie. BOS upgrade takes longer due to reboot)

**Upgrade** means bumping versions of already-installed packages. It is not possible to upgrade individual packages —
upgrading always updates all components of the system (BOS + all applications).

**Install** means adding a new package that wasn't previously installed. Users can install individual packages without
upgrading existing ones, as long as compatibility checks pass via checker packages (see "Checker Packages").

### Complete Upgrade Flow Diagram

```
┌──────────────────────────────────────────────────────────────────┐
│ 1. User Selection & BOS Check                                    │
│    - System resolves each server's nix-package-index.v1.json     │
│    - User browses and selects applications in web UI             │
│    - Run compatibility checker packages                          │
│    - If incompatibility detected: offer only full upgrade        │
└───────────────────────────────┬──────────────────────────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────────────┐
│ 2. Realise Store Paths (nix-store --realise)                     │
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
│ 4. BOS Upgrade (if required)                                     │
│    - Handled by bmc-upgrade, outside of Nix functions            │
│    - Triggers reboot; activation flag left on filesystem         │
└───────────────────────────────┬──────────────────────────────────┘
                                │
                                ▼
┌──────────────────────────────────────────────────────────────────┐
│ 5. Activation & Atomic Profile Switch                            │
│    - Atomically replace current profile symlink                  │
│    - Execute activation scripts in alphanumerical order          │
│    - Previous generation remains on disk for rollback            │
└──────────────────────────────────────────────────────────────────┘
```

### Phase 1: User Selection & BOS Check

1. System resolves each enabled server's `nix-package-index.v1.json` — fetched directly from the entry's `index_url`,
   or, for feed-linked servers, from the `index_url` of the feed entry matching the target firmware
2. User browses available applications in web UI
3. User selects applications to install or update
4. For each selected app, run compatibility checker packages. If a newer BOS is necessary, full upgrade has to be
   performed (see "Checker Packages").
5. User clicks "Install Selected" or "Upgrade System & Install Selected"

### Phase 2: Realise store paths from index using nix-store --realise

The store paths are collected from the indexes. Then, they're realised through `nix-store --realise`, which fetches any
missing paths and their dependencies from the configured substituters (binary caches).

If the store paths are not available, the whole process is aborted.

### Phase 3: Profile construction

From all the collected packages, a profile is built. This means that all the files from the packages are taken and
symlinked together to a single folder.

Afterwards a manifest of the profile is computed to know the packages that went inside of the profile for upgrades. This
manifest is saved inside of the profile itself.

A final activation script is always computed for the profile, executing all the individual activation scripts in
alphanumerical order. See Phase 5 for detailed description of the activation itself.

#### Hooks

During the build of the profile it's possible we will need to build specialized scripts or unify a couple of files into
one. For example it could be feasible to combine all individual widget json files into a bigger json file with all the
widgets listed in one place.

The activation script creation could also be just another hook that will sort the activation scripts alphabetically and
build a singular script that calls the individual activation scripts.

The hooks can be added by any package, inside of `hooks/` subdirectory. These hooks are executed in order based on the
lexicographical order of the filenames. Similarly to the activation scripts, they will get paths of the new profile as
environment variable. But they will not get the old profile. The hooks should never depend on the currently active
profile. Only activation can depend on currently active profile.

For cross compilation bootstrap purposes, there will be a hook override path. This will then be used sometimes to refer
to hooks out of the target profile. Specifically when the init tarball is being made, we can be compiling it on x86_64,
the hooks in the profile will be for aarch64, not x86_64. So we need to override the hooks path with a custom path with
x86_64 binaries.

#### Manifest

The manifest kept inside of the profile should be similarly structured as the indexes the packages are downloaded from.
Each package has its name, store_path, version, description, where it came from and who/what has installed it specified.

However, there needs to be also additional information stating what has actually installed this package. This can then
be used as part of deciding when a package might be removed - what/who installed it can also decide on when to remove
it, so we might need to ask the given system if to keep the package on upgrades or if to upgrade it.

The usual value of "installed_by" is going to be "user", where the user is the one who has installed it and only the
user can remove it, ie. the user has installed a widget. We keep that widget as long as it exists or the user has
removed it.

Each package can also be pinned to a semver version constraint that limits which versions an upgrade may move it to. The
constraint is a string such as `1.2.3` (exactly that version), `^1.2` (compatible `1.x`), `~1.2.3` (within the minor),
`1.2.x` (any patch of `1.2`), or a range like `>=1.2, <2.0.0`. A bare, fully specified version (`1.2.3`) means exactly
that version; every other form follows `semver` range semantics. `null` (or an omitted field) means the package is
unpinned and may upgrade to any version.

This could allow for partial upgrades if we wanted to support those, since all information necessary to build a new
profile is kept. Also the upgrades will be performed thanks to them, looking at the packages based on the names.

We also need to keep where the package has came from, from what index. This helps us decide during update what package
to choose if there are multiple with the same name.

```
{
  "packages": {
    "miniminer-display": {
      "version": "2.1.0",
      "store_path": "/nix/store/abc123def456-bmc-2.1.0",
      "category": "core",
      "description": "Main display application for the Deck",
      "installed_by": "system",
      "installed_from": "braiins_server",
      "pinned": null
    },
    "hashrate-widget": {
      "version": "1.2.0",
      "store_path": "/nix/store/xyz789ghi012-hashrate-widget-1.2.0",
      "category": "widget",
      "description": "Widget showing current hashrate statistics",
      "installed_by": "user",
      "installed_from": "braiins_server",
      "pinned": "^1.2.0"
    }
  }
}
```

**Key fields:**

- `packages` - Installed packages with their metadata:
  - `version` - Installed version
  - `store_path` - Nix store path of the installed package
  - `category` - Package category (core, widget, etc.)
  - `description` - Human-readable package description
  - `installed_by` - What initiated the installation (e.g., "system", "user")
  - `installed_from` - What server from servers.json configuration this package is installed from
  - `pinned` - Semver version constraint the package is pinned to (e.g. `1.2.3`, `^1.2`, `~1.2.3`, `1.2.x`,
    `>=1.2, <2`); a bare full version means exactly that version. `null` or an absent field means unpinned (any version)

### Phase 4: BOS Upgrade

Compatibility checks are handled by checker packages. When a checker signals incompatibility, the user has to first
upgrade (see "Checker Packages"). During the upgrade, both BOS and the application parts managed through Nix are
upgraded to latest version.

The BOS upgrade is handled by `bmc-upgrade`. It triggers a reboot. Unlike a normal package upgrade, the Nix side of a
firmware upgrade is not resolved against whatever the servers currently advertise — instead, it is scoped to the
incoming firmware version. The firmware tarball ships `bmc-nix-cli` and `servers.json.default`, and the CLI is invoked
with `--firmware <incoming version>`: each feed-linked server's package feed is fetched and the entry matching that
version selects the **feed-resolved index** — the exact index the firmware release was built and tested against. This
guarantees that the set of applications activated alongside the new BOS matches what the firmware was validated with,
independent of what any server's live index currently contains.

Concretely:

- Every firmware tarball includes `bmc-nix-cli` and `servers.json.default`; every firmware release publishes a feed
  entry whose `index_url` names that release's `nix-package-index.v1.json`.
- During a firmware upgrade, `bmc-nix-cli` is invoked from the tarball with `--firmware`, and the feed-resolved index
  for that version is the only index used to resolve store paths and build the new profile. A resolution failure aborts
  keep-current; there is no fallback to another index.
- The no-downgrade filter from "Conflict Resolution" still applies against the currently installed versions, so a
  firmware upgrade will not roll an installed application back either.
- After the firmware upgrade completes and the device is back to normal operation, subsequent upgrades (application
  layer only) resume using the configured servers as usual.

The new Nix profile is built before the BOS upgrade but not yet activated. A flag is written to the filesystem to
indicate a pending profile activation. After the reboot, a boot service detects this flag and runs the profile
activation (Phase 5).

**Important:**

- BOS upgrade happens BEFORE the profile activation (step 5), but triggers a reboot
- Profile activation runs after boot via a service that checks the pending activation flag
- BOS is NOT a Nix dependency — compatibility is checked by checker packages (see "Checker Packages")
- The feed-resolved index for the incoming firmware is the only index consulted for the firmware-upgrade Nix step

**User experience:** User selected "Install Miniminer Display v2.1.0" and sees installation progress.

#### BOS Downgrade

On some platforms (currently BMM101) the user is allowed to downgrade BOS to an older firmware. This case reuses the
same mechanism as a firmware upgrade — the feed keeps its entry for the older firmware, so the `--firmware` scope
resolves to that release's own index — but with a different resolution outcome: the older release's index will typically
advertise lower application versions than what is currently installed.

The no-downgrade filter from "Conflict Resolution" still applies here without any special case. Entries in the older
release's index whose version is lower than the currently installed version are discarded, and the installed version is
kept as-is. In practice this means:

- Installed applications keep their current, newer versions across a BOS downgrade.
- The older release's index effectively acts as a floor / compatibility hint only for packages that would otherwise be
  missing; for anything already installed at a higher version, its entries are thrown away.
- No Nix-level rollback of application packages happens as a side effect of the BOS downgrade. If the user also wants to
  downgrade an application, that is a separate action (profile rollback or an explicit install of a specific version).

If a checker package indicates the newer application versions are not compatible with the older BOS, the downgrade is
blocked in the same way an upgrade would be — via the checker mechanism, not via silent version rewriting.

### Phase 5: Activation, atomic profile switch

Each package can ship with multiple activation scripts. The activation scripts should:

1. Get the system to the state where the package can be used
2. Run the newly made services

For example, when upgrading the main BMC compositor, first, the bmc service is put to place in /etc/init.d. Then the BMC
is (re)started.

### Activation structure

```
core/
  activation/
    scripts/
      50-write-boundary
      60-bmc-service
      60-bmc-start
      zzz-link-current
```

The activation scripts run in alphanumerical order. It is expected that 50-write-boundary is the separator for side
effects. Everything before it should do only checks and fail if something is not as expected. The whole activation will
be aborted when that happens.

The activation scripts run with these environment variables:

```
PROFILE_OLD_GENERATION=/path/to/old/generation
PROFILE_NEW_GENERATION=/path/to/new/generation
```

These allow for example to see what services have been changed and restart them. The paths in these variables can be
just symlinks.

The activation scripts aren't specified further, they can be shell scripts, they can be binaries... It depends on our
specific needs. It is expected that if some heavy calculation has to be performed, we will put that inside of the hooks
and the activation scripts will perform only lighter operations.

### Final Activation

The final activation script is computed from the individual ones during build of the profile based on the alphanumerical
order - Phase 3.

Part of the activation performs an atomic switch to the new application generation. (to the new profile)

The replacement is done only after all activation scripts that perform checks succeed. The individual checks depend on
the nature of the packages. This is done thanks to the 'write-boundary' service. Everything before this service does not
have any side effects, only checks are performed. All services that do have side effects need to run after
'write-boundary'. Thanks to this the device does not end up in an inconsistent state when the checks fail.

---

## Configuration Preservation

User configurations survive upgrades through:

**Profile Generations** - Previous generations preserved on disk for rollback.

**Conffiles** - OpenWrt normally removes non-marked files on the disk. To mitigate that there will be a service with
activation script that marks such files as config files that should be backed up before sysupgrade.

---

## Service configuration in activation scripts

Since home-manager / NixOS have modules that extend each other, it's possible to for example say that a module wants a
configuration file to be put to a given place, such as:

```
xdg.configFile."fish/config.fish".text = "my contents";
```

Saying to create a file ~/.config/fish/config.fish with given content. Then the module that's responsible for
`xdg.configFile` can pick this up and create all the files that were specified.

In our system, this is not possible, though. We do not have the module system to extend options like this.

There will be two ways of solving this, first off, a simpler way, the activation services can expand environment
variables. And then the 'consumers' will use these environment variables.

Secondly, it is possible to solve this from within files that the packages can put inside of the profile with needed
configuration. These could be for example json files with necessary configuration.

In practice, both approaches can be combined, the first one for simpler services, such as specifying where config files
reside that shouldn't be removed. The second for more complex ones.

---

## Custom Profile Management

Instead of using `nix profile`, the system uses a custom Rust-based profile management implementation. This provides
more control over the symlink structure, avoids using Nix evaluations.

### Overview

The custom profile manager:

- Builds unified symlink trees from multiple Nix store paths
- Manages generation directories for instant rollback
- Performs atomic profile switches via symlink replacement
- Tracks installed packages in profile's generation manifest

### Profile Structure

```
/nix/var/nix/gcroots/profiles/
├── bmc/                           # Application profiles
│   ├── 1-link/                    # Generation 1 (factory)
│   │   ├── bin/
│   │   │   ├── miniminer-display -> /nix/store/xxx-miniminer-display-2.0.0/bin/miniminer-display
│   │   │   └── widget-runner -> /nix/store/yyy-widgets-1.0.0/bin/widget-runner
│   │   ├── lib/
│   │   │   └── ... -> symlinks to store paths
│   │   └── share/
│   │       └── ... -> symlinks to store paths
│   ├── 2-link/                    # Generation 2
│   ├── 3-link/                    # Generation 3 (current)
│   └── current -> 3-link          # Atomic symlink to active generation
```

### Installation flow

When installing or updating packages:

1. **Collect store paths** - Gather all store paths for packages to be included in the profile
2. **Build unified symlink tree** - Walk each store path and merge all files/directories under standard paths (bin, lib,
   share, etc.), creating symlinks pointing to the actual files in the Nix store
3. **Handle file conflicts** - If two packages provide the same file, apply conflict resolution (priority-based or
   error)
4. **Run hooks** - Special scripts that operate on the profile, producing new files a) **Resolve activation scripts** -
   Based on the alphanumerical order.
5. **Create manifest** - Generate a `manifest` file in the profile that captures all package versions installed,
   including their store paths
6. **Activation** - Run activation scripts and as part of that, switch the currently active profile generation for new
   one

### Atomic Symlink Replacement

The switch to a new generation is atomic at the filesystem level. This ensures the system is never in an inconsistent
state — the profile either points to the old generation or the new one, never to a partially-built state.

See the Installation/Upgrade Workflow section for the full installation flow (phases 3 and 5).

### Generation Management

- **Creation** - Each install/update/remove operation creates a new generation
- **Retention** - Configurable number of generations kept (default: 2 + factory)
- **Protection** - Factory generation (generation 1) is never garbage-collected
- **Cleanup** - Old generations can be removed to free disk space (see Garbage Collection)

---

## Rollback Mechanism

The custom profile management maintains generation directories on disk. Each generation is a complete symlink tree that
can be switched to instantly.

Only rollbacks to previous profile generations existing on the disk are possible. Similarly to switch to a new profile
generation, the activation of the older generation is ran.

**Available operations:**

- List available generations (stored in `/nix/var/nix/gcroots/profiles/`)
- Rollback to previous generation (instant symlink switch)
- Rollback to specific generation by number

---

## Initialization

The devices now in production do not contain Nix yet. Apart from that the way we initialize the Nix store is also
important for factory resets - we need to guarantee that even if the device gets bricked, the factory reset actually
resets the /nix/store and all the state.

Because of this, an initializer binary will be maintained for already existing devices. For new devices, the initial
/nix/store version will be flashed in the factory.

### Factory initialization

In the factory, we will flash the partitions with the desired /nix/store, /nix/var/nix and so on. The profile itself can
also be activated already on the flashed partitions, or it could be activated on boot.

The scripts for the factory will have to be adapted and new images made.

### First OpenWrt firmware with Nix

When we will be making the first firmware with Nix support, we should ideally ensure that Nix store is initialized prior
to the upgrade.

For that, we will mark the firmware as a major version, so devices cannot skip it.

The image's COMMAND will contain the initialization procedure in the image check part. It will download the initial nix
store and extract it to the root partition. It will also contain the initial profile.

Then on the next boot, activation of the profile will be ran. The fallback initializer will detect that the store is
initialized already and be skipped.

### Fallback (factory reset or if something goes wrong)

There will be a fallback service that checks for initialization and in case of issues, it will reinitialize the
/nix/store. We do not have source of /nix/store on the device, so it needs to be downloaded.

Since the device might not be connected to the WiFi, a small static compiled program will have to be maintained forever.
This program will allow for basic WiFi configuration and download of the initial /nix/store tarball.

Among other things, the initial tarball should also contain the initial profile. This way we can just call the
activation script from it, not relying on the scripts or programs already available on the system to be able to build
and activate the profile.

The `factory` field in `/etc/nix-upgrade/servers.json` names the server whose package feed lists the available tarballs.

The initial tarball is selected based on the bos version saved in `/etc/bos_version`, matched against the feed's
entries. In case there is no entry for a given version, the service will upgrade BOS itself. The latest version always
has to have a feed entry, otherwise this would break.

The service needs to communicate to the user what's happening through multiple states and progress bars so that the user
knows it's not stuck.

#### Tarball integrity and TLS

Store initialization runs during sysupgrade, after the device has already downloaded the firmware over TLS. The
initializer therefore keeps certificate validation enabled and relies on the same system-clock and certificate-trust
prerequisites as the firmware download.

Factory tarballs must also be cryptographically signed. After downloading the tarball and before extraction, the
initializer verifies the signature against the `known_public_key` from `servers.json`. If verification fails, the
tarball is rejected. The signature provides content authentication independently of the TLS transport.

## Factory Reset

During a factory reset, a file is created to completely remove the /nix/store and all its state on next boot. This is
then respected by the initializer that will remove the /nix/store prior anything starts from it.

This path is taken, because it might be impossible to remove the /nix/store during operation, due to files being used by
currently running processes.

---

## Garbage Collection

**Requirements:**

On the Deck, there should always be enough space for the next upgrade even in case it changes every derivation - This
can happen on glibc or compiler changes. This guarantees that we're always able to install new version of the software
even if everything changed. Afterwards the previous versions could be garbage collected.

It will have to be calculated how much space is taken by currently used packages and try to garbage collect, according
to some other given constraints, enough space for the next upgrades.

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

**Key fields:**

- `keep_generations` - Minimum number of generations to keep
- `keep_days` - Keep all generations newer than this (days)
- `min_free_space` - Try to guarantee this much free space
- `protected_generations` - Generations that are never removed (e.g., factory)

**Operations:**

- **Remove package** - Uninstall a package from the current profile (rebuilds profile without it, keeps shared deps in
  store)
- **Delete old generations** - Remove generation directories older than configured threshold
- **Nix store garbage collection** - Run `nix-collect-garbage` to remove unreachable store paths

---

## User Experience

- The BOS and app upgrades run after each other.
- The user does not see that both are updated at the same time.
- The user cannot choose upgrading only one or the other.

### Web UI for Package Management

Users interact through a web UI to:

- Browse and select applications to install/update
- View changelogs and manage installed applications
- User always needs to upgrade all installed applications in bulk

### Version Display

- BMC version shown to user (the main core packages version, user friendly, ie. 26.01)
- Individual application version shown to user (e.g., "Miniminer Display v2.1.0")
- BOS version hidden (available in advanced settings, stored in `/etc/bos_version`)

---

## Signature Verification

NAR archives are signed using Ed25519 keys (in narinfo `Sig` field). Trusted keys configured in `/etc/nix/nix.conf`:

```
trusted-public-keys = cache.braiins.com:AAAAB3NzaC1... apps-cache.braiins.com:BBBBB4NzaC2...
```

Nix refuses packages with invalid/missing signatures. Multiple signatures supported for multi-party trust.

The user should be the one to authorize the new cache servers. The indexes offer new cache servers. The main one from
Braiins will be authorized automatically, but when user adds more indexes, they should be asked to verify that they want
to authorize the server.

---

## Error Handling

### Potential Failure Points

If the upgrade fails in any step, the previous profile is kept in use.

| Failure              | Detection        | Recovery              |
| -------------------- | ---------------- | --------------------- |
| Network failure      | Incomplete NAR   | Retry, resume partial |
| BOS upgrade fails    | Install error    | Show error, abort     |
| Insufficient storage | Pre-flight check | Run GC first          |
| Activation failure   | Non-zero exit    | Auto rollback         |
| Invalid signature    | Nix verification | Reject package        |
| Corrupt download     | NarHash mismatch | Re-download           |

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

## Examples

Here are some examples on how the hooks can be utilized.

### File merger hook

Sometimes we will need to merge multiple files into a single file, for example for OpenWrt's Conffiles. To do that there
will be a specialized hook.

The packages will provide a folder `merge-files/path/to/file`. Then, the hook will take the files from there and create
a new file under the profile: `path/to/file`. Specifically the hook should look at the files in a lexicographical order,
so then we can prepend priority to the files, ie. `000-`, `050-`, `100-`.

### File symlinker hook

We might need to create an activation script that will symlink files out of the profile. To do that, there are at least
two options. Either each package is going to provide an activation script with the symlinks, but that might lead to
quite a lot of activation scripts that need to be executed on each boot, or a hook could be made so that we have only
one activation script.

With the hook approach, the packages could provide `file-symlinks/` with json file definitions. These definitions will
tell us what file to take out of the profile and where to put it to. There might be multiple conflicting files, so the
hook has to decide based on the priority which file to choose. Additionally it could also respect the priorities of the
configured servers.

```
{
   "priority": 10,
   "from": "my/test",
   "to": "/etc/test"
}
```

The hook will create a single activation script that will symlink all of the files. This allows us to keep the possibly
harder logic in Rust and use activation scripts only for simple

The produced script will reside at `activation/file-symlinks` and `activation/file-symlinks.json`, stating when it
should run.
