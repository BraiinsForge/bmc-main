# Nix Device Scripts

Helper scripts for initializing and deploying packages to the Deck.
All scripts accept the device IP either as an argument or via the
`DEVICE_IP` environment variable.

## Prerequisites

- SSH access as root to the device
- The device has a `/mnt/data` partition available

## nix-init.sh — First-time Nix store setup

Initializes a fresh device with the Nix store. This is a **one-time** operation
for devices that have never had Nix installed.

The script:

1. Checks that `/nix` and `/mnt/data/nix` are empty (refuses to overwrite)
2. Creates `/mnt/data/nix` and bind-mounts it to `/nix`
3. Builds the `init-tarball-armv7` flake output (contains a minimal Nix store
   with `nix-store` binary and the core profile)
4. Streams and extracts the tarball on the device
5. Activates the initial profile

```sh
# With positional argument
./scripts/nix-init.sh 192.168.1.2

# With environment variable
DEVICE_IP=192.168.1.2 ./scripts/nix-init.sh
```

After this, the device has a working `/nix/store` and a `nix-store` binary at
`/run/current-profile/bin/nix-store`, which is required by `nix-deploy.sh`.

To reinitialize a device, first clean up the existing store:

```sh
ssh root@192.168.1.2 'umount /nix 2>/dev/null; rm -rf /mnt/data/nix /nix'
```

## nix-deploy.sh — Deploy packages to an initialized device

Builds a Nix flake package and copies its entire closure to the device's
`/nix/store` using `nix copy`. The device must already be initialized with
`nix-init.sh`.

```sh
# Deploy the main BMC binary
./scripts/nix-deploy.sh bmc-openwrt-armv7-release 192.168.1.2

# Deploy widgets
DEVICE_IP=192.168.1.2 ./scripts/nix-deploy.sh widgets-armv7-glibc-release
```

The script prints the `/nix/store/...` path of the deployed package. You can
then run binaries from that path on the device.

### nixpkgs

Nixpkgs is exposed as pkgs, the armv7 packages are exposed as "armv7-pkgs".
So you can for example do `./scripts/nix-deploy armv7-pkgs.strace` to deploy
the strace package.

The deployed packages are automatically installed to user's Nix profile.
So the executables should be directly available when you log in through ssh.

### Common packages

| Package name                    | Description                    |
|---------------------------------|--------------------------------|
| `bmc-openwrt-armv7-release`     | Main BMC binary (release)      |
| `bmc-openwrt-armv7-debug`       | Main BMC binary (debug)        |
| `widgets-armv7-glibc-release`   | All widgets bundle (release)   |
| `widgets-armv7-glibc-debug`     | All widgets bundle (debug)     |

## nix-cargo-deploy.sh — Fast impure deploy of cargo-built binaries

Copies a locally cargo-built binary directly to the device, replacing the file
in-place under `/run/current-profile/`. This skips the full Nix rebuild and is
meant for fast development iteration.

The device must already have the packages deployed via `nix-deploy.sh` (so the
target paths exist).

```sh
# Deploy the compositor (bmc-openwrt, built for armv7-unknown-linux-musleabihf)
./scripts/nix-cargo-deploy.sh compositor 192.168.1.2

# Deploy a widget by name (built for armv7-unknown-linux-gnueabihf)
./scripts/nix-cargo-deploy.sh widget digital-clock 192.168.1.2
DEVICE_IP=192.168.1.2 ./scripts/nix-cargo-deploy.sh widget flip-clock
```

The script expects binaries at the standard cargo cross-compilation output
paths:

| Command | Local binary path |
|---------|-------------------|
| `compositor` | `target/armv7-unknown-linux-musleabihf/release/bmc-openwrt` |
| `widget <name>` | `target/armv7-unknown-linux-gnueabihf/release/bmc-widget-<name>` |

Build in the appropriate nix develop shell first (`nix develop .#armv7-release`
for the compositor, `nix develop .#armv7-glibc-release` for widgets — see
`workspace.nix` for available shells).

## Example: Running bmc-openwrt on the device

After deploying, use the `start-compositor` wrapper to launch `bmc-openwrt`.
This wrapper (deployed as part of the core profile at
`/run/current-profile/bin/start-compositor`) sets up the `XDG_RUNTIME_DIR`
needed by the Wayland compositor:

```sh
ssh root@192.168.1.2

# start-compositor creates a temporary XDG_RUNTIME_DIR and execs its arguments
start-compositor bmc-openwrt
```

It ensures the Wayland compositor embedded in `bmc-openwrt` can create its
socket under a valid runtime directory.
