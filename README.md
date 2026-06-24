Braiins clock

## Setting up repository

Install git-lfs through package manager of your choice and run

```
git lfs install
```

to initialize the binary files with proper contents.

## Development Environment

Enter the dev shell for local development (Rust, frontend, GUI):

```shell
nix develop
```

This provides:

- Rust toolchain (from rust-toolchain.toml)
- Protobuf compiler
- Node.js + Yarn for frontend
- GUI libraries for Slint/display development (X11, Wayland, OpenGL)
- FHS-compatible environment for node_modules binaries

For ARM cross-compilation:

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

Or run the one-line wasm validation recipe:

```shell
just validate-wasm
```

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
