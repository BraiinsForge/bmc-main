# BMC Compositor

A minimal Wayland compositor for displaying widget surfaces on the Braiins Deck.

## Architecture

The compositor uses smithay with a DRM/KMS backend for direct framebuffer access. It does not require a seat daemon (seatd/logind) - it opens DRM devices directly, which requires root privileges.

```
BMC Compositor (owns display via DRM/KMS)
    |
    +-- Wayland socket (/run/wayland-0)
    |
    +-- Widget processes (Wayland clients)
            - digital-clock
            - ticker
            - etc.
```

## Building

### For ARMv7 (device)

```bash
nix develop .#armv7-glibc-release --command cargo build -p bmc-compositor --release
```

Binary location: `target/armv7-unknown-linux-gnueabihf/release/bmc-compositor`

### For x86 (check only)

The compositor requires DRM/KMS which is only available on the ARM device. For development, you can check compilation:

```bash
nix develop --command cargo check -p bmc-compositor
```

Note: Running the compositor on x86 requires a winit backend (not yet implemented).

## Device Setup

### Prerequisites

The device must have the following Nix store paths available. These are typically deployed as part of the BMC system image.

### Required Environment Variables

Before running the compositor, set the following environment variables:

```bash
# Library path - include all armv7 Nix store libraries
export LD_LIBRARY_PATH=$(ls -d /nix/store/*armv7*/lib 2>/dev/null | tr '\n' ':')

# Mesa GBM backend path
export GBM_BACKENDS_PATH=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/lib/gbm

# Mesa DRI drivers path
export LIBGL_DRIVERS_PATH=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/lib/dri
```

**Note:** The exact Nix store paths may vary. Find the correct paths with:

```bash
# Find GBM backend
find /nix/store -name "dri_gbm.so" 2>/dev/null | grep armv7 | head -1

# Find DRI drivers
find /nix/store -path "*armv7*/lib/dri" -type d 2>/dev/null | head -1
```

### Running the Compositor

The binary requires the glibc dynamic linker from the Nix store:

```bash
# Find the glibc linker
LINKER=$(ls /nix/store/*glibc*armv7*/lib/ld-linux-armhf.so.3 2>/dev/null | head -1)

# Run with debug logging
RUST_LOG=debug $LINKER --library-path "$LD_LIBRARY_PATH" /tmp/bmc-compositor
```

### XDG Runtime Directory

The Wayland socket requires XDG_RUNTIME_DIR to be set:

```bash
export XDG_RUNTIME_DIR=/tmp/run
mkdir -p $XDG_RUNTIME_DIR
```

### Convenience Script

Create `/tmp/run-compositor.sh` on the device:

```bash
#!/bin/sh

# Set up XDG runtime directory for Wayland socket
export XDG_RUNTIME_DIR=/tmp/run
mkdir -p $XDG_RUNTIME_DIR

# Set up library paths
export LD_LIBRARY_PATH=$(ls -d /nix/store/*armv7*/lib 2>/dev/null | tr '\n' ':')
export GBM_BACKENDS_PATH=$(dirname $(find /nix/store -name "dri_gbm.so" 2>/dev/null | grep armv7 | head -1))
export LIBGL_DRIVERS_PATH=$(find /nix/store -path "*armv7*/lib/dri" -type d 2>/dev/null | head -1)

# Find glibc linker
LINKER=$(ls /nix/store/*glibc*armv7*/lib/ld-linux-armhf.so.3 2>/dev/null | head -1)

# Run compositor
exec $LINKER --library-path "$LD_LIBRARY_PATH" /tmp/bmc-compositor "$@"
```

Then run:

```bash
chmod +x /tmp/run-compositor.sh
RUST_LOG=info /tmp/run-compositor.sh
```

## DRM Device Selection

The compositor automatically scans for DRM devices and selects the first one with connectors:

- `/dev/dri/card0` - Often a render-only device (no connectors)
- `/dev/dri/card1` - Usually the display controller with DSI connector

The Braiins Deck uses a DSI panel connected to card1.

## Troubleshooting

### "Failed to create libseat session"

The compositor uses direct DRM access, not libseat. If you see this error, you're running an old version. Rebuild.

### "No such file or directory" for libgbm.so.1

Add the mesa-libgbm path to LD_LIBRARY_PATH:

```bash
find /nix/store -name "libgbm.so.1" 2>/dev/null | grep armv7
# Add the directory containing this file to LD_LIBRARY_PATH
```

### "cannot open shared object file: dri_gbm.so"

Set GBM_BACKENDS_PATH to the directory containing dri_gbm.so:

```bash
export GBM_BACKENDS_PATH=/nix/store/...-mesa-armv7l-.../lib/gbm
```

### "Failed to restore previous state" errors

These are harmless warnings from smithay during cleanup. They occur when the compositor exits without having fully configured a display mode.

### Permission denied on /dev/dri/cardX

Run as root or ensure the user is in the `video` group:

```bash
ls -la /dev/dri/
# Should show: crw------- root root
# Run as root or add user to video group
```

## Display Information

The Braiins Deck display:
- Physical panel: 600x1280 (portrait)
- Displayed as: 1280x480 (landscape, rotated 90° CCW by compositor)
- Interface: DSI (MIPI Display Serial Interface)
- DRM connector type: DSI

## Features

- **Software rendering**: Uses DRM dumb buffers for CPU-based compositing
- **90° rotation**: Rotates widget content to match landscape display orientation
- **Frame callbacks**: Proper Wayland frame synchronization for smooth animations
- **VBlank sync**: Tear-free display with proper page flipping

## Running a Widget

Once the compositor is running, connect a Wayland widget client:

```bash
# On the device, in another terminal
export WAYLAND_DISPLAY=wayland-0
export XDG_RUNTIME_DIR=/tmp/run

# Run your widget (e.g., digital-clock)
/path/to/widget-binary
```

The compositor will display the widget content with 90° rotation applied.

## Development Status

- [x] DRM device discovery and initialization
- [x] Direct DRM access (no seat daemon required)
- [x] Display mode configuration
- [x] Framebuffer setup (double-buffered dumb buffers)
- [x] Wayland socket creation
- [x] Surface compositing (SHM buffer support)
- [x] Frame callback support
- [x] 90° display rotation
- [x] Widget animation demo
- [ ] Touch input forwarding
- [ ] DMA-BUF support (zero-copy)
- [ ] Multi-widget compositing
