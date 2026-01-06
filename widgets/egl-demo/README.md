# egl-demo

## How to run on Deck

### Build
```bash
nix develop .#armv7-glibc-release --command cargo build -p bmc-widget-egl-demo --release
```

### Copy to Deck
```bash
export DECK_IP= # Add your Deck's IP address

scp target/armv7-unknown-linux-gnueabihf/release/bmc-widget-egl-demo root@$DECK_IP:/tmp/egl-demo
```


```bash
# XDG runtime directory for Wayland socket
export XDG_RUNTIME_DIR=/tmp/run
export WAYLAND_DISPLAY=wayland-0

# Library paths - all armv7 glibc libs from Nix store
export LD_LIBRARY_PATH=$(find /nix/store -maxdepth 3 -type d -name "lib" -path "*armv7l*gnueabihf*" 2>/dev/null | tr '\n' ':')

# Mesa environment (for GPU rendering)
export GBM_BACKENDS_PATH=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/lib/gbm
export LIBGL_DRIVERS_PATH=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/lib/dri
export __EGL_VENDOR_LIBRARY_FILENAMES=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/share/glvnd/egl_vendor.d/50_mesa.json

# Find glibc linker
LINKER=$(find /nix/store -name "ld-linux-armhf.so.3" 2>/dev/null | head -1)

# Run widget
RUST_LOG=info $LINKER --library-path "$LD_LIBRARY_PATH" /tmp/egl-demo
```