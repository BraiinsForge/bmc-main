Braiins clock

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
`<subcommand> --help`) for its procedures and options. For the lower-level scripts, see `docs/nix-device-scripts.md`
(deploy arbitrary Nix packages, the compositor, widgets, and so on).
