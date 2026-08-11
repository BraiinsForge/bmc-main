# Deck Device Operations

Initialising, deploying to, and iterating on a Deck from a dev host. The store-init, package-deploy, and firmware-flash
flows live in the `deck` harness (`nix run .#deck`); a single shell script remains for fast native-binary iteration.

## Prerequisites

- SSH access as root to the device
- The device has a `/mnt/data` partition available

## deck init — First-time Nix store setup

`nix run .#deck -- init` initializes a fresh device with the Nix store. This is a **one-time** operation for devices
that have never had Nix installed.

It refuses to overwrite a populated `/nix` or `/mnt/data/nix`, bind-mounts `/mnt/data/nix` at `/nix`, builds and streams
the `init-tarball-armv7` flake output (a minimal Nix store with the `nix-store` binary and the core profile), extracts
it on the device, and activates the initial profile generation.

```sh
nix run .#deck -- init --device 192.168.1.2
```

Add `--dry-run` to probe the device and build the tarball without mutating anything. After init the device has a working
`/nix/store` and a `nix-store` binary at `/run/current-profile/bin/nix-store`, which `deck deploy` relies on.

To reinitialize a device, first clean up the existing store (the abort hint prints this command):

```sh
ssh root@192.168.1.2 'umount /nix 2>/dev/null; rm -rf /mnt/data/nix /nix'
```

## deck deploy — Deploy packages to an initialized device

`nix run .#deck -- deploy` is the primary means of deployment throughout day-to-day development. This kind of deployment
always builds Nix packages, though. So for faster iteration on a single component, like a native widget or compositor,
you might prefer `nix-cargo-deploy.sh`.

It builds Nix flake packages and copies their entire closures to the device's `/nix/store` using `nix copy`, then
installs them into the bmc profile via one `bmc-nix-cli` call. The device must already be initialized with `deck init`.

With no `--packages`, it deploys `core` plus every widget (discovered from the nix-owned `category` metadata). Pass
`--packages` with bare names (e.g. `core widget-image`) that expand to `.#deck-packages.*`, or fully-qualified flake
URIs (anything with `#`, e.g. `.#armv7-nixpkgs.strace`) used as-is. Index packages (exposed under `deck-packages`) are
auto-detected and their `.pkg` output is built; raw nixpkgs derivations (e.g. `armv7-nixpkgs`) are built directly.

```sh
# core plus every widget
nix run .#deck -- deploy --device 192.168.1.2

# a specific set of Deck packages (bare names expand to .#deck-packages.*)
nix run .#deck -- deploy --device 192.168.1.2 --packages core widget-clock

# a package from nixpkgs
nix run .#deck -- deploy --device 192.168.1.2 --packages '.#armv7-nixpkgs.strace'
```

The deployed packages are installed into the bmc profile and activated immediately. Executables are available under
`/run/current-profile/bin/` for core and nixpkgs packages. Widget packages (native and wasm) are installed under
`/run/current-profile/lib/bmc-widgets/<name>/`. Changed widgets reload during activation without bmc-tui restarting the
compositor. Core changes still let the service orchestrator restart the compositor when its executable dependencies
change.

`--profile {release,debug}` selects the build (default `release`). `debug` deploys the parallel
`.#deck-packages-debug.*` set — the same package names built with the `profiling` feature on the compositor and the wasm
host — so the device surfaces the `mesh::profile` channel: ii-stopwatch timing, MemProbe RSS, and the asset-cache
write/evict/restore events. Use it when you need to observe or measure on-device, then redeploy `release` to drop the
overhead.

```sh
nix run .#deck -- deploy --device 192.168.1.2 --profile debug
```

Run `nix run .#deck -- <init|deploy|sysupgrade|upgrade-e2e> --help` for the full option set of each procedure.

## nix-cargo-deploy.sh — Fast impure deploy of cargo-built native binaries

This is for fast iteration over a given component of the system. Use `deck deploy` unless you're just making simple
fixes that you expect to work in the produced version. This will be faster, especially if you are doing a change in a
leaf crate.

IMPORTANT NOTE: This script is currently doing a dirty replacement inside of the profile. That means that the changes
are lost right after creating a new generation, ie. deploying through `nix run .#deck`.

Copies a locally cargo-built native binary directly to the device for fast development iteration. The binary is uploaded
to `/mnt/data/tmp/cargo-deploy/`, and the existing profile entry under `/run/current-profile/` is repointed to that
staged binary via symlink. Dynamic linker and rpath dependencies are copied over with `nix copy`.

This script handles only native binaries (compositor and native widgets). **Wasm widgets are never deployed through this
script** — use `deck deploy` for wasm widgets.

This is not suitable for deploying a brand new widget package directly. It only redeploys targets that already exist in
the profile. Deploy the widget package first via `deck deploy`, then iterate with `nix-cargo-deploy.sh`.

The device must already have the packages deployed via `deck deploy` (so the target paths exist).

```sh
# Execute in a dev shell such as nix develop ".#armv7-glibc-release"

# Deploy the compositor (bmc-openwrt, built for armv7-unknown-linux-gnueabihf)
./scripts/nix-cargo-deploy.sh compositor 192.168.1.2

# Deploy a native widget by name (built for armv7-unknown-linux-gnueabihf)
./scripts/nix-cargo-deploy.sh widget flip-clock 192.168.1.2
DEVICE_IP=192.168.1.2 ./scripts/nix-cargo-deploy.sh widget flip-clock
```

In case you need to add extra cargo flags, such as a --features flag, use `CARGO_EXTRA_FLAGS` environment variable.

The script resolves Cargo `target_directory` dynamically (so `CARGO_TARGET_DIR` and Cargo config overrides are honored).
With default settings, binaries are expected at these standard cross-compilation output paths:

| Command         | Local binary path                                                |
| --------------- | ---------------------------------------------------------------- |
| `compositor`    | `target/armv7-unknown-linux-gnueabihf/release/bmc-openwrt`       |
| `widget <name>` | `target/armv7-unknown-linux-gnueabihf/release/bmc-widget-<name>` |

The script will build these binaries itself through `cargo build`. The script assumes you're in a dev shell, such as
".#armv7-glibc-release". As a one-liner, use
`nix develop ".#armv7-glibc-release" -c ./scripts/nix-cargo-deploy.sh compositor 192.168.1.2` for example.

### Frontend assets for the compositor

The compositor binary (`bmc-openwrt`) has `/run/current-profile/www/bmc` baked in as the default frontend path (via
`BMC_WEB_FRONTEND_DIR` at build time). For the compositor to serve the UI, that path must exist on the device; otherwise
`nix-cargo-deploy.sh compositor` will warn.

The dev-only `.#deck-packages.bmc-frontend` package wraps the frontend build so that its contents end up under
`<profile>/www/bmc/`. It is marked `category = "dev"` and is NOT included in the init tarball. Use `deck deploy` for
deployment.

## Example: Running bmc-openwrt on the device

The compositor is ran through `/etc/init.d/bmc-compositor`, as long as you have deployed the whole `core` package, it
should be restarted automatically.

In case you're running nix-cargo-deploy.sh, stop the service via `/etc/init.d/bmc-compositor stop`, then use
`start-compositor bmc-openwrt` to start the compositor in a dirty way, without the service. The service always refers to
the core package in `/nix/store`, so it will not pick up compositor changes through `nix-cargo-deploy.sh`

## Inspecting the device — logs, cache, scene config

### Logs

App logs live under `/var/log/bmc/`. **Sort by mtime first (`ls -lat /var/log/bmc/`)** — a stale legacy file sits next
to the live one and looks just as authoritative:

| File                               | Source                                                                 |
| ---------------------------------- | ---------------------------------------------------------------------- |
| `bmc.log`                          | compositor (`bmc-openwrt`)                                             |
| `run-bmc-wasm-host-sdk-v0.log`     | **live** wasm host — widget + asset-cache events; rotated (`.1.gz`, …) |
| `widgets.log`, `bmc-wasm-thin.log` | widget / thin-client host                                              |
| `bmc-wasm-host.log`                | **stale legacy** — predates the versioned host log; do not grep this   |

On a `--profile debug` build the asset-cache observability rides the `mesh::profile` target in the live wasm-host log:

```sh
ssh root@192.168.1.2 'grep -E "cache write|dormant eviction|cache restore" \
  /var/log/bmc/run-bmc-wasm-host-sdk-v0.log'
```

### Asset cache

Per-instance flash buckets live at `/mnt/data/bmc/widget-cache/<widget-uuid>-<extent>/<tag>.blob` — one per widget
instance (the image widget writes `image.blob`). Their presence proves write-at-decode ran even when logging is off.

### Scene config and the dormancy prerequisite

The active scene set is `/etc/bmc/config.json` — scenes plus `scene_cycling`, `accounts`, and display settings.
(Firmware from before the config migration reads `/etc/bmc_config.json` instead; the first boot of current firmware
copies it to the new path and keeps the original for downgrade safety, after which only `/etc/bmc/config.json` matters.)
There is no `jq` on the device, so edit locally and push back, preserving the other top-level fields:

```sh
ssh root@192.168.1.2 'cat /etc/bmc/config.json' > cfg.json
jq --slurpfile s bmc-virt/data/configs/image-cache.json '.scenes = $s[0].scenes' cfg.json > cfg.new.json
ssh root@192.168.1.2 'cp /etc/bmc/config.json /etc/bmc/config.json.bak; cat > /etc/bmc/config.json' < cfg.new.json
ssh root@192.168.1.2 'killall bmc-openwrt'   # procd respawns, reloads config at startup
```

Edit only while the app is down (or restart right after): `bmc-openwrt` holds the config in memory and rewrites the file
on save, clobbering manual edits made while it runs.

Every config carries a top-level `"version"` field (`1` for the current schema). A config **without** it is read as the
legacy pre-migration schema; if its widgets are already in the current (`widget_type_id`) shape — as any config written
by a recent firmware is — the legacy parse fails and the boot path resets to platform defaults. This only bites when
flashing current firmware onto a device whose config predates the `version` field (a transitional dev situation; the
`bmc-virt/data/configs/*.json` samples are stamped, so `bmc-virt push` / `just run --config` are fine). The reset is
**not** destructive — the original is copied to `/etc/bmc/config.json.bcp` first, and the pre-migration
`/etc/bmc_config.json` is kept untouched — but to keep the config live, add `"version": 1` to the file (any top-level
position) before flashing or before the reboot that loads the new firmware.

To exercise a widget's dormant/wake path (e.g. the image cache's RAM reclaim), the config needs **≥4 enabled scenes**:
the compositor keeps the active scene `Visible` and both cycle neighbours `Prepared`, so only a non-neighbour scene
reaches `Dormant`. With 2–3 enabled scenes nothing ever goes dormant, and the evict/restore path never fires.
