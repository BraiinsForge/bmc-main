# bmc-main

## Getting started

Software in this repository can be used on the [Braiins Deck](https://braiinsforge.com/hardware/braiins-deck). It will
be used on all Decks starting from the upcoming release. The latest released firmware (26.02.1) can't run this software;
the manual firmware upgrade described below is required.

### Current status

This project is still being prepared for external use. Some features and supporting components are not yet available or
complete, and the setup may change as the public release matures.

The following widgets are currently supported:

- Image
- Block Height
- Halving Countdown
- Clock
- Weather
- Random Facts
- Nameday
- ISS Position
- SpaceX Launch
- Financial Ticker — single (including old BTC Ticker widget)
- Financial Ticker — list
- Formula 1

Not yet supported widgets:

- NASA Picture of the Day
- Braiins Pool
- Bitcoin Mining Data

Major user-facing gaps also remain:

- The first-boot and device initialization flow is not yet presented on the device display.
- Firmware upgrade progress and completion are not yet shown on the device display.
- Centralized account management is not yet available.
- Configuration forms do not yet support all controls needed by widgets with complex settings.

#### Runtime limitations

Current memory constraints mean that fewer widgets can remain active simultaneously than on release 26.02.1. This does
not change which widget types are supported. The rendering pipeline has improved since 26.02.1, but it is still being
optimized to reach higher frame rates.

Running the software on a physical Braiins Deck currently requires a compatible custom firmware image. The current
firmware image is available here:

```text
https://feeds.braiins-os.com/stm32mp157c-ii3-bmc1/firmware_2026-07-21-0-34648a21-26.07-rc_arm_cortex-a7_neon-vfpv4.tar
```

Only manual upgrades to this firmware are possible at the moment. See [Deploy to a Deck](#deploy-to-a-deck) for the
upgrade instructions.

### Architecture

The display stack is built around a Wayland compositor, with widgets running as independent Wayland clients. Official
widgets are WASM modules executed by the [`bmc-wasm-runtime`](bmc-wasm-runtime/README.md).

### Prerequisites

Install the following host tools:

- Git and Git LFS
- Nix with flakes enabled
- `mprocs` for widget workflows that run a simulator beside the testbed (`cargo install mprocs` from the development
  shell)

The `nix develop` shell provides the Rust toolchain, Protobuf compiler, `pkg-config`, Node.js, and Yarn. GPU libraries
from Nix are currently not supported, so running the widget testbed requires the native development libraries from your
system package manager: Fontconfig, FreeType, Wayland, libxkbcommon, Mesa/OpenGL/EGL, ALSA, libinput, seatd, udev, and
libdrm. Package names differ between distributions. Building for and deploying to the device via
`nix run .#deck -- deploy` does not need these libraries.

Deploying to a physical device additionally requires root SSH access and a `/mnt/data` partition on the device.

### Set up the repository

After cloning the repository, initialize Git LFS and enter the development shell:

```shell
git lfs install
git lfs pull
nix develop
```

Run commands in the remaining sections from this shell unless they invoke Nix directly.

### Run a widget in the testbed

The WASM widget testbed provides a device-free development loop. Start a hot-reloading preview of an example widget:

```shell
just wasm::dev hello-widget
```

To build the widget in release mode and preview it once:

```shell
just wasm::run hello-widget
```

Both commands build the widget for `wasm32-unknown-unknown` and launch the desktop testbed. See the
[`bmc-wasm-runtime` README](bmc-wasm-runtime/README.md) for the other widget development and regression-testing
commands.

### Deploy to a Deck

Flash the custom firmware image from the URL above to the device. Without Nix, SSH into the device and run the upgrade
there directly. To find the IP address of the Deck, open the settings tray. On 26.02 firmware, swipe up from the bottom
of the screen; on the newer firmware offered here, swipe down from the top instead.

```shell
ssh root@192.168.1.2
cd /tmp
wget https://feeds.braiins-os.com/stm32mp157c-ii3-bmc1/firmware_2026-07-21-0-34648a21-26.07-rc_arm_cortex-a7_neon-vfpv4.tar
sysupgrade ./firmware_2026-07-21-0-34648a21-26.07-rc_arm_cortex-a7_neon-vfpv4.tar
```

With Nix, download the image to your host and flash it with the `deck` harness, which validates the image and asks for
confirmation before flashing:

```shell
nix run .#deck -- sysupgrade \
  --device 192.168.1.2 \
  --image './firmware_2026-07-21-0-34648a21-26.07-rc_arm_cortex-a7_neon-vfpv4.tar'
```

Either way, allow approximately 10 minutes for the upgrade: during `sysupgrade`, the firmware downloads an
initialization tarball and uses it to populate `/nix/store` on the device. Do not interrupt the upgrade while this is in
progress. Once the device is back online, deploy the packages built from your checkout:

```shell
nix run .#deck -- deploy --device 192.168.1.2
```

Subsequent iterations also use `deck deploy`; see [`docs/deployment.md`](docs/deployment.md) for package selection,
debug profiles, and faster native-binary iteration. Note that if you make changes to the bmc-wasm-runtime, you should
redeploy all widgets, not just the one you have changed.

## Cross-compilation

For ARM cross-compilation, use one of the target-specific development shells:

```shell
nix develop .#armv7-glibc-release  # release builds
nix develop .#armv7-glibc-debug    # debug builds
```

## Build and validation

Run validation commands from the repository root. Use the root `justfile` for routine checks, because it wraps the
formatter, lint, tests, Python checks, wasm checks, and repo content checks in the same shape expected by CI.

Common builds:

```shell
# Build frontend
nix build -L .#frontend --print-out-paths --no-link

# Build OpenWRT binaries
nix build .#bmc-openwrt-armv7-glibc-release
nix build .#bmc-openwrt-armv7-glibc-debug

# Build deployable Deck packages
nix build .#deck-packages.core.pkg
nix build .#deck-packages.widget-clock.pkg
nix build .#deck-packages.widget-blockheight.pkg
nix build .#deck-packages.bmc-frontend.pkg

# Cargo builds inside a dev shell
cargo build
cargo build --release
```

Top-level workspace validation:

```shell
# Format the workspace
nix fmt

# Run workspace clippy
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings

# Run workspace tests
cargo test --workspace
```

Or run the one-line validation recipe:

```shell
just validate
```

Production widget workspace validation:

```shell
# Format the workspace
nix fmt

# Run wasm-target widget clippy
cargo clippy --manifest-path ./widgets-wasm/Cargo.toml --workspace --target wasm32-unknown-unknown

# Run production widget workspace tests
cargo test --manifest-path ./widgets-wasm/Cargo.toml --workspace
```

Both of these are covered by `just validate` above — the wasm gates run as part of it.

Other useful focused recipes:

```shell
just format
just clippy
just test bmc
just validate-full  # Full nix-driven check set matching the main CI stage; this is much heavier
```

Frontend commands live in `frontend/justfile`:

```shell
cd frontend
just validate  # format, lint, type-check, and tests
just build
just lint
just test
```

### Rust-analyzer

Using rust-analyzer in the widgets requires further configuration as the widgets use the wasm32 target. It should be
possible to work from the repository's root, supporting both the top-level workspace and the widgets-wasm workspace.

in .vscode/settings.json, you will need

```
{
    "rust-analyzer.linkedProjects": ["widgets-wasm/Cargo.toml", "./Cargo.toml"],
    "rust-analyzer.cargo.target": "wasm32-unknown-unknown"
}
```

## Build frontend

```
nix build -L .#frontend
```

## Run mock with built frontend assets

```
cargo run --bin bmc-mock -- --address=0.0.0.0:6070 --www-path=./result
```

## Build and run mock with widgets

Build all widgets:

```
nix build .#widgets -o result-widgets
```

Build frontend and run mock with widgets:

```
nix build -L .#frontend
cargo run --bin bmc-mock -- --address=0.0.0.0:6070 --www-path=./result --widgets-path=./result-widgets/lib/bmc-widgets
```

## Build widgets for OpenWRT device

Build ARM widgets (glibc, dynamically linked):

```
nix build .#widgets-armv7-glibc-release -o result-widgets-arm
```

## Run bmc-openwrt on control board

```shell
cd bmc-openwrt/
nix develop .#armv7-glibc-release

export MINER_IP=192.168.1.2
cargo run # or 'cargo run -- <ARGS>'
# terminate it by Ctrl+C
```

## Deployment during development

`nix run .#deck` is the harness for deploying packages and flashing firmware to a device — run it with `--help` (and
`<subcommand> --help`) for its procedures and options. For details see `docs/deployment.md`.
