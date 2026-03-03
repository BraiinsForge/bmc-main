# Prerequisites

The full concept and details are captured in docs/devlogs/BDK-212/nix,
all of the files there are important for this implementation plan.

BDK-318 (dev-shell-improvements) already delivered rpath-based linking
for the dev shell. This plan builds on that — the remaining work is
making production builds and Mesa/libglvnd self-contained.

# Goal

The goal is to, as fast as possible, get to a state where we do not need
launch.sh script and custom initialization of Nix store.

1. launch.sh

We need to change how the widgets are compiled, making sure they get
the rpath they need, instead of using the impure `LD_LIBRARY_PATH`.
Part of this work has already been done as part of BDK-318.

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

- Store corruption recovery (hash verification + re-download of
  corrupted store paths).
- Third-party server support and package browsing UI.
- Custom server list management and individual package installation.
- Disk space pre-flight check before NAR downloads (run GC if needed).

# Risks

1. **Mesa/libglvnd recompilation** — Nixpkgs Mesa is patched to use
   `/run/opengl-driver`. A custom Mesa overlay for self-contained DRI
   discovery was implemented in Stage 3. Stage 4 then replaced wrapper
   scripts with `autoPatchelfHook` rpath (BDK-353). See
   `mesa-libglvnd.md` and `nix-patchelf.md`.
2. **Cross-repo coordination** — Stages 7 and 9 require changes in
   bos-packages, bos-main, and openwrt repositories.

# Stage 1: Prerequisites

This is mainly to ease out the development of the compositor and widgets.
The tasks here can be done potentially in parallel.

1. Rework workspace.nix to produce binaries that run by themselves,
   without using impure environment variables. BDK-318 handled the dev
   shell rpath; this extends it to production ARM builds. For Mesa/GL
   driver discovery, enable `wrapNixGL` on the ARM glibc build profiles
   so that `buildCrate` generates wrapper scripts with the correct Mesa
   environment variables. A custom Mesa overlay (see `mesa-libglvnd.md`)
   is deferred to a later stage.
2. Implement profile building and activation in bmc-nix and bare bones
   bmc-nix-cli:
   - `types.rs` — serde types for PackageIndex, Manifest, ServerRegistry
   - `profile.rs` — `build_profile()`, `build_symlink_tree()`,
     `activate_profile()`
   - `activation.rs` — topological sort and execution of activation
     scripts during profile build
   - `hooks.rs` — `run_hooks()`, lexicographic execution
   - 3 built-in hooks as `[[bin]]` targets: hook-merge-files,
     hook-activation-resolver
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
device provisioning. The initial profile contains the `nix` binary
itself (needed for runtime `nix copy` operations), `bmc-openwrt`, and
the core widgets. Config files (`servers.json`, `nix.conf`) are
included via `mkTarball`'s `extraFiles` parameter.

1. `mkIndex.nix` — takes package list, caches, indexes, commit hash;
   outputs `index.json`.
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
deferred to Stage 9.

## Testing

- `nix build .#init-tarball-armv7` succeeds and produces a valid tarball.
- Tarball contents: correct Nix DB, all store paths present, profile
  generation directory with symlink tree.
- Manual test: extract on a clean device, run the activation
  entrypoint, verify services start.

# Stage 3: Self-contained Mesa (no wrapper scripts)

Stage 1 uses `wrapNixGL` wrapper scripts to set Mesa environment
variables (`LIBGL_DRIVERS_PATH`, `GBM_BACKENDS_PATH`,
`__EGL_VENDOR_LIBRARY_FILENAMES`). This works but adds a shell wrapper
around every widget binary. Replace with a custom Mesa overlay that
makes DRI driver discovery self-contained — Mesa finds its own
`$out/lib/dri/` via compiled-in paths instead of environment variables.

1. Investigate the nixpkgs Mesa derivation to determine how
   `/run/opengl-driver` is injected (patch, mesonFlags, or postFixup).
   See `mesa-libglvnd.md` for the original analysis.
2. Record the exact nixpkgs revision from `flake.lock` and use it for
   all upstream lookups in this stage.
3. Modify the custom Mesa derivation at `nix/pkgs/mesa/package.nix`
   (and if needed, create a libglvnd overlay at
   `nix/overlays/mesa-self-contained.nix`) so Mesa's DRI loader uses
   `$out/lib/dri/` and EGL vendor JSON points to `$out/lib/`.
4. Add the overlay to `flake.nix` overlay chain.
5. Ensure Mesa is in compositor rpath via `rustflags.nix` (temporary,
   replaced by autoPatchelfHook in Stage 4).
6. Verify: no `/run/opengl-driver` references in the Mesa closure.

## Success criteria

- Mesa's DRI loader finds drivers at `$out/lib/dri/` without
  `LIBGL_DRIVERS_PATH`.
- EGL vendor JSON in Mesa output points to `$out/lib/`.
- No `/run/opengl-driver` references in the Mesa closure.
- No Mesa env vars are required (including `__EGL_VENDOR_LIBRARY_FILENAMES`).
- The compositor runs from the Nix store via `start-compositor` with Mesa
  env vars unset.

## Testing

- `grep -r "opengl-driver"` on the Mesa closure returns nothing.
- `cat $mesa/share/glvnd/egl_vendor.d/50_mesa.json` shows `$out/lib/`.
- Manual test on device:
  - deploy the compositor via `./scripts/nix-deploy.sh`
  - run `/run/current-profile/bin/start-compositor /nix/store/.../bin/bmc-openwrt`
    with `LIBGL_DRIVERS_PATH`, `GBM_BACKENDS_PATH`,
    `__EGL_VENDOR_LIBRARY_FILENAMES` unset
  - verify EGL initializes (process keeps running)

# Stage 4: autoPatchelfHook rpath

With Mesa self-contained (Stage 3), replace the RUSTFLAGS scripts with
nixpkgs' `autoPatchelfHook`. Each binary gets the rpath entries it
needs — no wrapper scripts, no environment variables, no global
RUSTFLAGS rpath.

Originally investigated three approaches (BDK-353):
- Option 1: Custom `patchelf` in postFixup (nonguix-inspired plan)
- Option 2: nixpkgs' `autoPatchelfHook` + `runtimeDependencies`
- Option 3: `build.rs` linking (ruled out: too many libs use dlopen)

Option 2 was chosen — zero custom code, automatic DT_NEEDED
resolution, and `runtimeDependencies` handles dlopen'd libraries.

Implementation: `nix/autopatchelf-binaries.nix` applies
`autoPatchelfHook` via `overrideAttrs` on `buildCrate` derivations.
The hook runs in `postFixupHooks` (after `--shrink-rpath`), so the
rpaths it sets are not stripped. Key details:
- Must use `armv7Pkgs.autoPatchelfHook` (not `pkgs.autoPatchelfHook`)
  so the hook propagates ARM bintools and sets the ARM interpreter.
- `runtimeDependencies` adds `/lib` paths to rpath for dlopen'd
  libraries (Mesa, wayland, libxkbcommon, etc.) regardless of
  DT_NEEDED.
- Also auto-resolves DT_NEEDED libraries (e.g. `libgcc_s.so.1`).

1. Implemented `nix/autopatchelf-binaries.nix` — applies
   `autoPatchelfHook` + `runtimeDependencies` via `overrideAttrs`.
2. Defined `widgetRuntimeDeps` and `compositorRuntimeDeps` in
   `workspace.nix` as ARM package lists for dlopen'd libraries.
3. Applied `autopatchelfBinaries` in `mkWidgetPackage`, `mkCorePackage`,
   and `cratePackages` for compositor and widget builds.
4. Removed `wrapNixGL = true` from ARM glibc profiles.
5. Removed ARM production RUSTFLAGS rpath from ARM cross targets.
6. Deleted `nix/patchelf-binaries.nix` and `docs/plans/stage-4-patchelf.md`.

## Success criteria

- Widget binaries run on device without any wrapper script or
  environment variable overrides. Only requirement: `/nix` bind mount.
- `patchelf --print-rpath` on widget binaries shows library store
  paths including dlopen'd dependencies.
- `patchelf --print-interpreter` shows the Nix store glibc interpreter.
- No `LD_LIBRARY_PATH` or `LIBGL_DRIVERS_PATH` references in build
  outputs.

## Testing

- `nix build .#widgets-armv7-glibc-release` succeeds.
- `patchelf --print-rpath` shows correct store paths.
- `grep -r LD_LIBRARY_PATH result/` returns nothing.
- Manual test: widget runs on device from bare store path, no env vars.

# Stage 5: Implement rest of bmc-nix

Stage 1 implemented profile building and activation. This stage adds
the runtime operations needed for upgrades and lifecycle management.

Modules to implement:
- `index.rs` — `fetch_indexes()`, `fetch_and_merge_indexes()` (with
  visited-set cycle detection for federated `indexes` URLs),
  `resolve_new_package()`, `resolve_installed_package()`
- `store.rs` — `copy_store_paths()` (NAR download from binary cache),
  `init_store()`
- `manifest.rs` — manifest read/write, merge with existing manifest,
  `compute_upgrade_plan()`
- `upgrade.rs` — `apply_profile_change()`
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

# Stage 6: Deployment indexes (COMPLETE)

Build the Nix derivations and tooling that produce final deployment indexes.
Some of the work should already be done from stage 2, finish it here.

1. `mkFactoryIndex.nix` or equivalent tooling that collects
   `metadata.json` from multiple tarball builds (one per BOS version)
   and produces `factory.json`.
2. Ensure `mkIndex.nix` (from Stage 2) produces a complete
   `miniminer-index.json` usable for runtime upgrades.

**Delivered:**

- Audited `mkIndex.nix` output — schema compliant (null, not false)
- `nix/mkFactoryIndex.nix` — generates `factory.json`
- `scripts/build-factory-index.sh` — CI helper assembling factory
  index from tarball `metadata.json` files
- `scripts/validate-nix-indexes.sh` — schema validation for both
  index types
- Wired `mkIndex`, `mkTarball`, `mkFactoryIndex` into `nix/lib.nix`
  (removed standalone imports from `workspace.nix`)
- `init-factory-index` flake output via `init-artifacts.nix`
- `FactoryIndex`/`FactoryTarball` types in `bmc-nix/src/types.rs`
- Round-trip integration tests (`bmc-nix/tests/index_roundtrip.rs`)
- Documentation fixes: `nix-build.md` (false→null, tarball naming),
  `nix-concepts.md` (.tar.xz→.tar.gz)

### Index build flow

1. `nix build .#init-index-armv7` → `result/index.json`
   (miniminer-index.json for runtime upgrades)

2. `nix build .#init-tarball-armv7` → `result/nix-26.02.tar.gz`
   + `result/metadata.json`

3. `nix build .#init-factory-index` → `result/factory.json`
   (for local testing with placeholder URL)

4. CI flow:
   a. Build tarball: `nix build .#init-tarball-armv7`
   b. Upload tarball to cache server
   c. Build factory index:
      `./scripts/build-factory-index.sh \
        --base-url https://cache.braiins.com/v1 \
        --metadata result/metadata.json \
        --output factory.json`
   d. Upload factory index to cache server

5. Validation:
   `./scripts/validate-nix-indexes.sh \
     --index result/index.json \
     --factory factory.json`

## Success criteria

- `nix build` produces a valid `miniminer-index.json`.
- Factory index can be built listing tarballs for at least one BOS
  version.

## Testing

- Validate index JSON schema against concept doc specification.
- Round-trip: build index, feed it to `bmc-nix` index parsing, verify
  all packages resolve correctly.

# Stage 7: Self-initialization

The device can bootstrap its own Nix store on first boot or after
factory reset, without manual tarball extraction.

1. New crate: `bmc-nix-init`
   - Statically linked (musl + all deps bundled) — runs before Nix
     store exists.
   - Boot flow: check store → check WiFi → start AP mode if needed →
     fetch factory index → download tarball → extract → activate.
   - UI: evaluate Slint (DRM + static link) vs LED patterns + serial
     console. See Risk 2 above.
2. OpenWrt service for `bmc-nix-init` — runs on every boot, checks for
   activation finished (/tmp/nix_activated) and `nix_init` U-Boot
   sentinel.

It should use the Slint files already available for the initial setup.
The difference will be that the device state will be left out in
SetupPending, it won't proceed further. The architecture should be
similar to `bmc`, `bmc-mock`, `bmc-openwrt`. The idea is to have
something to test on regular computer rather than just on the device.

The nix init uses the `bmc-nix` crate from earlier stages.

## Success criteria

- A device with no Nix store boots, connects to WiFi (or starts AP),
  downloads the tarball, and starts the compositor — all without manual
  intervention.

## Testing

- Manual test on a factory-reset device.
- `bmc-nix-init-mock` crate for mock testing

# Stage 8: Upgrade in bmc-upgrade and frontend

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
the activator logs the error and does not activate the new generation.
The previous generation remains active. Automatic rollback to the
previous generation is out of scope for v1 — the user sees an upgrade
failure and can retry.

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

# Stage 9: OpenWrt services managed by Nix, application...

Now we need to finish the Nix services.
1. Create generic activation service for OpenWrt services
3. Create 2 OpenWrt services:
   - `nix-activator` — activates BMC profile on every boot. Handles
     post-BOS-upgrade activation. Activation must be idempotent:
     re-running on an already-active generation must be a fast no-op
     with no side effects.
   - `nix-service-orchestrator` — restarts services after activation. Waits
     for activation PID to finish, then stops/starts/restarts services
     based on diff of old/new service files.

# Stage 10: OpenWrt services for new firmware

Cross-repo work to produce firmware that uses Nix for the application
layer instead of shipping BMC as an OpenWrt package.

**Repositories affected:** bos-packages, bos-main, openwrt.

1. Remove BMC package from bos-packages — replace with
   `bmc-nix-init` package.
2. Create 2 OpenWrt services:
   - `bmc-nix-init` — store bootstrap on boot (from Stage 7).
   - `nix-factory-reset` — runs in `boot()`, checks for
     `nix_init` U-Boot env marker, deletes `/mnt/data/nix`.
     Must run before any Nix service.
3. COMMAND file in firmware archive — downloads, verifies
   (hash/signature), and extracts initial tarball during BOS sysupgrade
   (before first boot with Nix).
4. Update bos-main flake.nix to consume `bmc-nix-initializer` from
   bmc-main flake instead of `bmc-openwrt`.
5. Ship `servers.json` and `factory.json` as part of the firmware
   image. The initializer reads these on first boot to know where to
   fetch the tarball from — without them it cannot initialize.

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


# Stage 11: Interface for users to install (not done in v1)

Now we're finished with the Nix implementation itself. But users cannot
use it to download packages such as widgets. So we need to make a frontend
for it.

Presumably this stage will be implemented only later, not for the
initial firmware with Nix support.
