Braiins clock

## Build frontend

```
nix build -L .#frontend
```

## Run mock with built frontend assets

```
cargo run --bin bmc-mock -- --address=0.0.0.0:6070 --www-path=./result
```

## Run bmc-openwrt on control board

```shell
cd bmc-openwrt/
nix develop .#armv7-release

export MINER_IP=192.168.1.2
cargo run # or 'cargo run -- <ARGS>'
# terminate it by Ctrl+C
```

## Speed up Slint compilation during development

We use by default resource embedding strategy `EmbedResourcesKind::EmbedForSoftwareRenderer` which is slow and generates
huge Rust file (~50MB).\
We can switch to `EmbedResourcesKind::EmbedFiles` during development in VSCode/RustRover. This
strategy generates small Rust file (~820KB).

### RustRover

- open `bmc-display/Cargo.toml` in RustRover
- activate cargo feature `slint-embed-files` by clicking on the checkbox
- if you have custom run configurations (e.g. manual clippy check), you need to apply it there as well
- this fixes indexing, which otherwise cannot find generated objects

### VSCode

- create `.vscode/settings.json` with following content
  ```json
  {
    "rust-analyzer.cargo.features": [
        "bmc-display/slint-embed-files"
    ]
  }
  ```
- restart rust-analyzer server
- if you have custom run configurations (e.g. manual clippy check), you need to apply it there as well
