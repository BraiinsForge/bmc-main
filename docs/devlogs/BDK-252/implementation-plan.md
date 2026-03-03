# Prerequisites

The full concept and details are captured in docs/devlogs/BDK-212/nix,
all of the files there are important for this implementation plan.

BDK-308 (dev-shell-improvements) already delivered rpath-based linking
for the dev shell. This plan builds on that — the remaining work is
making production builds and Mesa/libglvnd self-contained.

# Goal

The goal is to, as fast as possible, get to a state where we do not need
launch.sh script and custom initialization of Nix store.

1. launch.sh

We need to change how the widgets are compiled, making sure they get
the rpath they need, instead of using the impure `LD_LIBRARY_PATH`.
Part of this work has already been done as part of BDK-308.

The libglvnd/mesa environment variables have to be solved as well, and
the decision is to use per-app Mesa, documented separately in
`mesa-libglvnd.md`.

2. custom initialization (deploy corinthia)

For initialization, we need to be able to build the initial tarball with Nix.
As a prerequisite, we need to be able to build and activate profiles. This
will be necessary to make the tarball (during nix build we build the profile),
and activation is necessary for actually using the profile.

# Out of scope for v1

The following are documented in the concept but deferred past the
initial firmware with Nix support:

- Two-phase activation rollback (Finding A1 from MR !212 review) —
  v1 treats activation failure as a fatal error requiring manual
  intervention or factory reset.
- Store corruption recovery (hash verification + re-download of
  corrupted store paths).
- Tarball signature/hash verification for the manual Stage 2 bootstrap
  path. Verification is enforced in Stage 7 production flows.
- Third-party server support and package browsing UI.
- Custom server list management and package/server blacklisting.

# Risks

1. **Mesa/libglvnd recompilation** — Nixpkgs Mesa is patched to use
   `/run/opengl-driver`. We need a custom Mesa overlay that makes
   DRI driver discovery self-contained. See `mesa-libglvnd.md`.
2. **bmc-nix-initializer static linking** — The initializer runs
   before the Nix store exists, so it must be statically linked with
   all dependencies (Slint, DRM, WiFi, HTTP+TLS). Binary size and
   long-term maintenance are concerns. Consider whether LED
   patterns + serial console could replace the Slint UI.
3. **Cross-repo coordination** — Stages 5 and 7 require changes in
   bos-packages, bos-main, and openwrt repositories.

# Stage 1: Prerequisites

This is mainly to ease out the development of the compositor and widgets.
The tasks here can be done potentially in parallel.

1. Rework workspace.nix to produce binaries that run by themselves,
   without using impure environment variables. BDK-308 handled the dev
   shell rpath; this extends it to production ARM builds and applies the
   Mesa overlay so DRI drivers are found via rpath, not
   `/run/opengl-driver`. See `mesa-libglvnd.md`.
2. Implement profile building and activation in bmc-nix and bare bones
   bmc-nix-cli:
   - `types.rs` — serde types for PackageIndex, Manifest, ServerRegistry
   - `profile.rs` — `build_profile()`, `build_symlink_tree()`
   - `activation.rs` — `activate_profile()`, topological sort of
     activation scripts
   - `hooks.rs` — `run_hooks()`, lexicographic execution
   - 3 built-in hooks as `[[bin]]` targets: hook-merge-files,
     hook-file-symlinks, hook-activation-resolver
   - `bmc-nix-cli` with `build-profile` subcommand (used by mkTarball)
3. Implement scripts for deploying to the Deck during development, i.e.
   when building in the shell we need to copy the library closure.
   Concretely: a shell script or Nix derivation that computes the
   runtime closure of a built binary and nix copies it to the device.

## Success criteria

- The compositor binary built with `nix build` runs on the device
  without `launch.sh`, `LD_LIBRARY_PATH`, or any environment variable
  overrides. Only requirement: `/nix` bind mount exists.
- `bmc-nix-cli build-profile --index <path> --profile-dir <path>`
  produces a valid profile generation with correct symlink tree.
- Dev deploy script copies a binary + its closure to the device in a
  single command.

## Testing

- Unit tests for `build_profile()`, `activate_profile()`,
  `build_symlink_tree()` using temp directories.
- `bmc-nix-cli build-profile` integration test with a fixture index
  JSON and pre-built store paths.
- Manual test: run compositor on device from Nix-built profile.

# Stage 2: Store initialization tarball

Build the Nix derivations that produce the initial store tarball for
device provisioning.

1. `mkIndex.nix` — takes package list, caches, indexes, commit hash;
   outputs `miniminer-index.json`.
2. `artifacts.nix` — central package list definition (single source of
   truth for both mkIndex and mkTarball).
3. `mkTarball.nix` — takes packages + bmc-nix-cli; generates temp
   index, invokes `bmc-nix-cli build-profile` in the sandbox, captures
   closure via `pkgs.closureInfo`, populates Nix DB via
   `nix-store --load-db`, creates gzipped tarball + `metadata.json`.
4. `mkWidgetPackage.nix` — standard builder for widget packages with
   correct directory layout (`lib/bmc-widgets/<name>/`).

## Success criteria

Clean store initialization on a Deck that has no Nix:
1. Bind mount `/mnt/data/nix` to `/nix`
2. Extract tarball to `/` on the Deck
3. Activate generation 1 of
   `/nix/var/nix/gcroots/profiles/bmc` by running the generation's
   self-contained activation entrypoint:
   `/nix/var/nix/gcroots/profiles/bmc/bmc-1-link/activation/entrypoint`
4. Compositor and widgets start correctly.

Stage 2 is a trusted/manual bootstrap path for development and
controlled provisioning. Tarball integrity verification is intentionally
deferred to Stage 7.

## Testing

- `nix build .#tarball` succeeds and produces a valid tarball.
- Tarball contents: correct Nix DB, all store paths present, profile
  generation directory with symlink tree.
- Manual test: extract on a clean device, run the activation
  entrypoint, verify services start.

# Stage 3: Implement rest of bmc-nix

Stage 1 implemented profile building and activation. This stage adds
the runtime operations needed for upgrades and lifecycle management.

Modules to implement:
- `index.rs` — `fetch_indexes()`, `fetch_and_merge_indexes()`,
  `resolve_new_package()`, `resolve_installed_package()`
- `store.rs` — `copy_store_paths()` (NAR download from binary cache),
  `init_store()`
- `manifest.rs` — manifest read/write, merge with existing manifest
- `upgrade.rs` — `compute_upgrade_plan()`, `apply_profile_change()`
- `gc.rs` — `cleanup_generations()`, `collect_garbage()`

## Success criteria

- Given a running device with an existing profile and a binary cache
  serving updated packages, `bmc-nix` can compute an upgrade plan,
  download NARs, build a new generation, and activate it.
- GC removes old generations according to gc.json policy.

## Testing

- Unit tests per module with mock HTTP responses (index JSON, NAR
  downloads).
- Integration test: full upgrade cycle using a local nix-serve
  instance or fixture files.

# Stage 4: Deployment indexes and binary cache

Moved before self-initialization because the initializer needs to fetch
indexes and the upgrade flow needs a binary cache to download from.

1. Binary cache setup — decide on Attic, Cachix, or self-hosted
   nix-serve. Configure CI to upload built closures.
2. Index publishing — CI pipeline produces `miniminer-index.json` and
   uploads it alongside the binary cache.
3. Factory index assembly — external tooling or CI job that collects
   `metadata.json` from multiple tarball builds (one per BOS version)
   and produces `miniminer-factory.json`.

## Success criteria

- `curl <server>/miniminer-index.json` returns a valid index.
- `nix copy --to <cache-url>` or equivalent uploads packages.
- Factory index lists tarballs for at least one BOS version.

## Testing

- Smoke test: fetch index, resolve a package, download its NAR from
  the cache, verify hash.

# Stage 5: Self-initialization

The device can bootstrap its own Nix store on first boot or after
factory reset, without manual tarball extraction.

1. New crate: `bmc-nix-initializer`
   - Statically linked (musl + all deps bundled) — runs before Nix
     store exists.
   - Boot flow: check store → check WiFi → start AP mode if needed →
     fetch factory index → download tarball → extract → activate.
   - UI: evaluate Slint (DRM + static link) vs LED patterns + serial
     console. See Risk 2 above.
2. OpenWrt service for `bmc-nix-initializer` — runs on every boot,
   checks for store presence and `/tmp/nix-activated` sentinel.

**Depends on:** Stage 4 (factory index and binary cache must be
available for the initializer to fetch from).

## Success criteria

- A device with no Nix store boots, connects to WiFi (or starts AP),
  downloads the tarball, and starts the compositor — all without manual
  intervention.

## Testing

- Manual test on a factory-reset device.
- `bmc-mock` simulation with `--with-nix-mock` flag (mocked HTTP
  responses for factory index and tarball download).

# Stage 6: Upgrade in bmc-upgrade and frontend

Integrate the Nix upgrade flow into the existing upgrade system.

1. Extend `SystemUpgradeState` with new variants: `CopyingPackages`,
   `BuildingProfile`, `ActivatingProfile`.
2. Add `SystemUpgradeError` variants for Nix-specific failures.
3. Implement combined upgrade flow in `system_upgrade.rs`: fetch
   indexes → compute plan → copy packages → build profile → if BOS
   upgrade needed, set pending activation marker and run sysupgrade
   (reboot, activation deferred to `nix-activator` on next boot) →
   otherwise activate immediately.
4. gRPC proto changes:
   - `CheckForUpgradeResponse`: add `package_updates` and
     `bos_upgrade_required` fields.
   - New `NixProgress` message with `CopyingPackages` / `BuildingProfile` /
     `ActivatingProfile` phases for streaming progress.
   - New `PackageUpdate` message: `name`, `old_version`, `new_version`,
     `category`.
5. Frontend: update `SectionUpgrade.tsx` to show multi-phase progress
   and package-level detail.

**Post-reboot re-validation:** After a BOS upgrade + reboot, the
`nix-activator` service must re-check BOS compatibility before
activating the pending profile. If BOS is still old (upgrade failed),
retry or fall back to previous generation.

## Success criteria

- User triggers upgrade from frontend → sees package list and progress
  → upgrade completes without reboot (Nix-only) or with one reboot
  (BOS + Nix).
- Auto-upgrade via `bmc-scheduler` triggers the same combined flow.

## Testing

- Unit tests for `compute_upgrade_plan()` with various scenarios
  (Nix-only, BOS+Nix, no update available).
- `bmc-mock` with mock binary cache for frontend development.
- Manual test on device: trigger upgrade, verify new generation active.

# Stage 7: OpenWrt services for new firmware

Cross-repo work to produce firmware that uses Nix for the application
layer instead of shipping BMC as an OpenWrt package.

**Repositories affected:** bos-packages, bos-main, openwrt.

1. Remove BMC package from bos-packages — replace with
   `bmc-nix-initializer` package.
2. Create 4 OpenWrt services:
   - `bmc-nix-initializer` — store bootstrap on boot (from Stage 5).
   - `nix-factory-reset` — runs in `boot()`, checks for
     `/mnt/data/NIX_FACTORY_RESET` marker, deletes `/mnt/data/nix`.
     Must run before any Nix service.
   - `nix-activator` — activates BMC profile on every boot. Handles
     post-BOS-upgrade activation.
   - `nix-service-applier` — restarts services after activation. Waits
     for activation PID to finish, then stops/starts/restarts services
     based on diff of old/new service files.
3. COMMAND file in firmware archive — downloads, verifies
   (hash/signature), and extracts initial tarball during BOS sysupgrade
   (before first boot with Nix).
4. Update bos-main flake.nix to consume `bmc-nix-initializer` from
   bmc-main flake instead of `bmc-openwrt`.

## Success criteria

- Firmware built from bos-main boots, initializes Nix store (via
  COMMAND or initializer), activates profile, starts all services.
- Factory reset (via marker file) wipes store and reinitializes on
  next boot.
- Service restart after upgrade does not kill the activation process.

## Testing

- End-to-end: build firmware, flash to device, verify full boot
  sequence.
- Factory reset: set marker, reboot, verify clean re-initialization.
- COMMAND negative test: tampered tarball is rejected before extraction.

# Stage 8: Interface for users to install

Now we're finished with the Nix implementation itself. But users cannot
use it to download packages such as widgets. So we need to make a frontend
for it.

Presumably this stage will be implemented only later, not for the
initial firmware with Nix support.
