# bmc-openwrt

Set your device IP:

```bash
export DECK_IP=10.0.0.25
```

## Prerequisites

https://braiins.atlassian.net/wiki/spaces/Nix/pages/1055326210/Installing+dynamically+linked+Nix+binaries+to+BMM+BMC#Preparing-and-mounting-Nix-store-on-the-miner

## Build Widgets

```bash
nix build .#widgets-armv7-release -o result-widgets

scp -r result-widgets/lib/bmc-widgets root@$DECK_IP:/tmp/
```

## Build Main Binary

```bash
nix develop .#armv7-glibc-release --command cargo build -p bmc-openwrt --release

scp target/armv7-unknown-linux-gnueabihf/release/bmc-openwrt root@$DECK_IP:/tmp/bmc-openwrt
```

## Run on Device

```bash
# XDG runtime directory for Wayland socket
export XDG_RUNTIME_DIR=/tmp/run
mkdir -p $XDG_RUNTIME_DIR

/tmp/bmc-openwrt --widgets-path /tmp/bmc-widgets
```
