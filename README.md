# bmc-main

## Getting started

Software in this repository runs on the [Braiins Deck](https://braiinsforge.com/hardware/braiins-deck) with firmware
26.09. Update your Deck through its normal software upgrade flow before deploying a development build.

### Official widgets

The following official widgets are available:

- Bitcoin Mining Data
- Block Height
- Braiins Pool
- Clock
- Financial Ticker List
- Fleet Management
- Formula 1
- Halving Countdown
- Image
- ISS Position
- Mining Info (Mining, Geek, Network, and Info Overload views)
- Nameday
- Picture of the Day (NASA Astronomy Picture of the Day)
- Random Facts
- SpaceX Launch
- Ticker - Single
- Weather

See the [widget documentation](docs/stories/widgets/README.md) for features, configuration, and supported display sizes.

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
system package manager: Fontconfig, FreeType, Wayland, libxkbcommon, Mesa/OpenGL/EGL, ALSA, libinput, udev, and libdrm.
Package names differ between distributions. Building for and deploying to the device via `nix run .#deck -- deploy` does
not need these libraries.

Root SSH access is available by default. Log in as `root` using the password you configured in the Deck's web UI.

### Set up the repository

After cloning the repository, initialize Git LFS and enter the development shell:

```shell
git lfs install
git lfs pull
nix develop
```

Run the remaining commands from this development shell.

For deployment, set `DEVICE_IP` in this shell to your Deck's actual IP address. Find the address in the settings tray by
swiping down from the top of the Deck's screen, then replace `<your-deck-ip-address>` below with that address:

```shell
DEVICE_IP="<your-deck-ip-address>"
```

If your Deck has a password set, you can install your SSH public key to avoid repeated password prompts during
deployment:

```shell
ssh-copy-id root@"$DEVICE_IP"
```

### Widget development

Widgets are written in Rust with [`bmc-wasm-sdk`](bmc-wasm-runtime/sdk/README.md) and compiled for
`wasm32-unknown-unknown`. A widget builds a declarative UI tree; the host handles layout, text shaping, rendering, and
animations. The SDK also provides host services for fetching data, storing state, handling touch input, and controlling
LEDs.

Start with the [widget developer guide](docs/devel/wasm-widgets/README.md) and
[best practices](docs/devel/wasm-widgets/best-practices.md). Browse the SDK API and lifecycle documentation locally:

```shell
just wasm::docs
```

#### Create and configure a widget

Use the [production widgets](widgets-wasm/) as references: [Clock](widgets-wasm/clock/) for layout and settings,
[Weather](widgets-wasm/weather/) for fetching and displaying data, and [Braiins Pool](widgets-wasm/braiins-pool/) for
credentials. Add a Rust `cdylib` crate to the widget workspace and its `Cargo.toml` member list. Use the SDK path
dependency and build configuration from an existing production widget.

Each widget has a `manifest.json` declaring its unique identity, name, icon, supported viewports, and configuration. The
[manifest schema](bmc-widget-manifest/manifest.schema.json) provides the field reference and editor validation. Declare
widget-specific choices as [params](docs/devel/wasm-widgets/params.md), use
[credential slots](docs/devel/wasm-widgets/credentials.md) for secrets, and read device-wide values such as timezone and
night mode from [system settings](docs/devel/wasm-widgets/system-settings.md). After changing params or credential
declarations, regenerate the typed accessors:

```shell
just wasm::gen my-widget
```

Replace `my-widget` with your widget's directory name. Design and check each viewport declared in the manifest; the
[display geometry guide](docs/devel/wasm-widgets/display-geometry.md) covers sizes and rectangular or round displays.

#### Preview and iterate

The WASM widget testbed provides a device-free development loop. Start a hot-reloading preview of Clock:

```shell
just wasm::dev clock
```

To build the widget in release mode and preview it once:

```shell
just wasm::run clock
```

Both commands build the widget and launch the desktop testbed. Replace `clock` with your widget's directory name to
preview it. Use the testbed's viewport selection, Params panel, and System panel to exercise layout and settings.

#### Test and deploy

Keep pure logic testable on the host and check rendering across the supported viewports. Run the repository checks with
`just validate`. For widgets with recorded fixtures and baselines, run the visual regression check:

```shell
just wasm::verify my-widget
```

The [regression testing guide](docs/devel/wasm-widgets/regression-testing.md) explains how to add capture configuration,
record fixtures, and review or update baselines. Visual capture requires a working EGL/GPU environment. Use
`just wasm::size my-widget` to inspect the binary size and `just wasm::profile my-widget` to investigate performance.

Nix discovers widget crates with manifests in the widget workspaces and exposes them as
`deck-packages.widget-<directory-name>`. Ensure new files are tracked by Git so Nix includes them. After the
[initial deployment](#deploy-to-a-deck), deploy an individual widget with:

```shell
nix run .#deck -- deploy --device "$DEVICE_IP" --packages widget-my-widget
```

Replace `my-widget` with your widget's directory name. Verify the result on the device as well as in the testbed. See
the [`bmc-wasm-runtime` README](bmc-wasm-runtime/README.md) for the full tooling reference.

### Deploy to a Deck

On a Deck running firmware 26.09, deploy the packages built from your checkout to the address set in `DEVICE_IP`:

```shell
nix run .#deck -- deploy --device "$DEVICE_IP"
```

**Current limitation:** `deck deploy` clears the Deck's upgrade servers, so the device no longer upgrades automatically
after deployment. To restore the default upgrade servers and allow automatic upgrades again, run:

```shell
nix run .#deck -- register-server --device "$DEVICE_IP"
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

## Deployment during development

`nix run .#deck` is the harness for deploying packages and flashing firmware to a device — run it with `--help` (and
`<subcommand> --help`) for its procedures and options. For details see `docs/deployment.md`.
