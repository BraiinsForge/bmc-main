# Nix Device Scripts

Helper scripts for initializing and deploying packages to the Deck. All scripts accept the device IP either as an
argument or via the `DEVICE_IP` environment variable.

## Prerequisites

- SSH access as root to the device
- The device has a `/mnt/data` partition available

## nix-init.sh — First-time Nix store setup

Initializes a fresh device with the Nix store. This is a **one-time** operation for devices that have never had Nix
installed.

The script:

1. Checks that `/nix` and `/mnt/data/nix` are empty (refuses to overwrite)
2. Creates `/mnt/data/nix` and bind-mounts it to `/nix`
3. Builds the `init-tarball-armv7` flake output (contains a minimal Nix store with `nix-store` binary and the core
   profile)
4. Streams and extracts the tarball on the device
5. Activates the initial profile

```sh
# With positional argument
./scripts/nix-init.sh 192.168.1.2

# With environment variable
DEVICE_IP=192.168.1.2 ./scripts/nix-init.sh
```

After this, the device has a working `/nix/store` and a `nix-store` binary at `/run/current-profile/bin/nix-store`,
which is required by `nix-deploy.sh`.

To reinitialize a device, first clean up the existing store:

```sh
ssh root@192.168.1.2 'umount /nix 2>/dev/null; rm -rf /mnt/data/nix /nix'
```

## nix-deploy.sh — Deploy packages to an initialized device

This is the primary means of deployment throughout day-to-day development. This kind of deployment always builds Nix
packages, though. So for faster iteration on a single component, like a native widget or compositor, you might prefer
`nix-cargo-deploy.sh`

Builds a Nix flake package and copies its entire closure to the device's `/nix/store` using `nix copy`, then installs it
into the bmc profile via `bmc-nix-cli`. The device must already be initialized with `nix-init.sh`.

The first argument is a full flake URI. Index packages (exposed under `deck-packages`) are auto-detected and their
`.pkg` output is built. Raw nixpkgs derivations (e.g. `armv7-nixpkgs`) are built directly.

```sh
# To deploy our Deck packages
./scripts/nix-deploy.sh '.#deck-packages.core' 192.168.1.2
./scripts/nix-deploy.sh '.#deck-packages.digital-clock' 192.168.1.2
# To deploy packages from nixpkgs
./scripts/nix-deploy.sh '.#armv7-nixpkgs.strace' 192.168.1.2
./scripts/nix-deploy.sh '.#armv7-nixpkgs.file' 192.168.1.2
```

The script prints the `/nix/store/...` path of the deployed package. You can then run binaries from that path on the
device.

### nixpkgs

Nixpkgs is exposed as pkgs, the armv7 packages are exposed as "armv7-nixpkgs". So you can for example do
`./scripts/nix-deploy.sh .#armv7-nixpkgs.strace` to deploy the strace package.

The deployed packages are installed into the bmc profile and activated immediately. Executables are available under
`/run/current-profile/bin/`.

## nix-cargo-deploy.sh — Fast impure deploy of cargo-built binaries

This is for fast iteration over a given component of the system. Use nix-deploy.sh unless you're just making simple
fixes that you expect to work in the produced version. This will be faster, especially if you are doing a change in a
leaf crate.

Copies a locally cargo-built binary directly to the device, replacing the file in-place under `/run/current-profile/`.
This skips the full Nix rebuild and is meant for fast development iteration. All the dependencies are copied over as
well.

NOTE: that this is not suitable for deploying a new widget. It only replaces currently existing widgets. NOTE: only
deploys native widgets, not WASM widgets.

The device must already have the packages deployed via `nix-deploy.sh` (so the target paths exist).

```sh
# Execute in a dev shell such as nix develop ".#armv7-glibc-release"

# Deploy the compositor (bmc-openwrt, built for armv7-unknown-linux-gnueabihf)
./scripts/nix-cargo-deploy.sh compositor 192.168.1.2

# Deploy a widget by name (built for armv7-unknown-linux-gnueabihf)
./scripts/nix-cargo-deploy.sh widget digital-clock 192.168.1.2
DEVICE_IP=192.168.1.2 ./scripts/nix-cargo-deploy.sh widget flip-clock
```

In case you need to add extra cargo flags, such as a --features flag, use `CARGO_EXTRA_FLAGS` environment variable.

The script expects binaries at the standard cargo cross-compilation output paths:

| Command         | Local binary path                                                |
| --------------- | ---------------------------------------------------------------- |
| `compositor`    | `target/armv7-unknown-linux-gnueabihf/release/bmc-openwrt`       |
| `widget <name>` | `target/armv7-unknown-linux-gnueabihf/release/bmc-widget-<name>` |

The script will build these binaries itself through `cargo build`. The script assumes you're in a dev shell, such as
".#armv7-glibc-release". As a one-liner, use
`nix develop ".#armv7-glibc-release" -c ./scripts/nix-cargo-deploy.sh compositor 192.168.1.2` for example.

## Example: Running bmc-openwrt on the device

After deploying, use the `start-compositor` wrapper to launch `bmc-openwrt`. This wrapper (deployed as part of the core
profile at `/run/current-profile/bin/start-compositor`) sets up the `XDG_RUNTIME_DIR` needed by the Wayland compositor:

```sh
ssh root@192.168.1.2

# start-compositor creates a temporary XDG_RUNTIME_DIR and execs its arguments
start-compositor bmc-openwrt
```

It ensures the Wayland compositor embedded in `bmc-openwrt` can create its socket under a valid runtime directory.
