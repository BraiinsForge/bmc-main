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

# Library paths - all armv7 glibc libs from Nix store (proper ordering is critical)
export LD_LIBRARY_PATH=$(find /nix/store -maxdepth 3 -type d -name "lib" -path "*armv7l*gnueabihf*" 2>/dev/null | tr '\n' ':')

# Mesa environment (use specific version for stability)
export GBM_BACKENDS_PATH=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/lib/gbm
export LIBGL_DRIVERS_PATH=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/lib/dri
export __EGL_VENDOR_LIBRARY_FILENAMES=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/share/glvnd/egl_vendor.d/50_mesa.json

# Find glibc linker
LINKER=$(find /nix/store -name "ld-linux-armhf.so.3" 2>/dev/null | head -1)

$LINKER --library-path "$LD_LIBRARY_PATH" /tmp/bmc-openwrt --widgets-path /tmp/bmc-widgets --widget-linker "$LINKER" --widget-library-path "$LD_LIBRARY_PATH" --widget-gbm-backends-path "$MESA_PATH/lib/gbm" --widget-libgl-drivers-path "$MESA_PATH/lib/dri" --widget-egl-vendor-library "$MESA_PATH/share/glvnd/egl_vendor.d/50_mesa.json"
```
