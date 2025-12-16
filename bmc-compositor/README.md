# BMC Compositor

A minimal Wayland compositor for displaying widget surfaces on the Braiins Deck.

## Architecture

The compositor uses smithay with a DRM/KMS backend for direct framebuffer access. It does not require a seat daemon (seatd/logind) - it opens DRM devices directly, which requires root privileges.

Two rendering backends are available:

1. **Software Renderer** (`bmc-compositor`) - CPU-based rendering with DRM dumb buffers
2. **EGL Renderer** (`bmc-compositor-egl`) - GPU-accelerated rendering via OpenGL ES

### Hardware Architecture

The Braiins Deck uses an STM32MP157 SoC with a **split GPU/display architecture**:

- **GPU**: Vivante GC400 (etnaviv driver) - `/dev/dri/card0` and `/dev/dri/renderD128`
- **Display**: STM32 LTDC display controller - `/dev/dri/card1`

This means rendering happens on one device (GPU) and scanout on another (display controller).

```
BMC Compositor (owns display via DRM/KMS)
    |
    +-- GPU (etnaviv/renderD128) -- EGL/OpenGL ES rendering
    |
    +-- Display (stm32-ltdc/card1) -- DRM/KMS scanout
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
# Software renderer
nix develop .#armv7-glibc-release --command cargo build -p bmc-compositor --release

# EGL renderer (GPU-accelerated)
nix develop .#armv7-glibc-release --command cargo build -p bmc-compositor --release --bin bmc-compositor-egl
```

Binary locations:
- `target/armv7-unknown-linux-gnueabihf/release/bmc-compositor`
- `target/armv7-unknown-linux-gnueabihf/release/bmc-compositor-egl`

### For x86 (check only)

The compositor requires DRM/KMS which is only available on the ARM device. For development, you can check compilation:

```bash
nix develop --command cargo check -p bmc-compositor
```

Note: Running the compositor on x86 requires a winit backend (not yet implemented).

## Device Setup

### Why glibc on a musl System?

The device runs OpenWRT with musl libc, but the Nix-built Mesa drivers and compositor use glibc. To run glibc binaries on a musl system, we explicitly invoke the glibc dynamic linker.

### Prerequisites

The device must have the following Nix store paths available (deployed as part of the BMC system image):

- glibc (provides the dynamic linker)
- Mesa (provides libgbm, libEGL, libGLESv2, DRI drivers)
- libdrm
- libinput, libudev (for input handling)

### Required Environment Variables

```bash
# XDG runtime directory (for Wayland socket)
export XDG_RUNTIME_DIR=/tmp/run
mkdir -p $XDG_RUNTIME_DIR

# Library path - ALL armv7 Nix store libraries (use find for proper ordering)
export LD_LIBRARY_PATH=$(find /nix/store -maxdepth 3 -type d -name "lib" -path "*armv7l*gnueabihf*" 2>/dev/null | tr '\n' ':')

# Mesa GBM backend (required for GBM device creation)
export GBM_BACKENDS_PATH=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/lib/gbm

# Mesa DRI drivers (required for hardware acceleration)
export LIBGL_DRIVERS_PATH=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/lib/dri

# EGL vendor library (required for EGL initialization) - CRITICAL FOR EGL RENDERER
export __EGL_VENDOR_LIBRARY_FILENAMES=/nix/store/scrdcg9kms4qbw062xiw54hchx3j1zpr-mesa-armv7l-unknown-linux-gnueabihf-25.1.6/share/glvnd/egl_vendor.d/50_mesa.json
```

**Note:** The Mesa path `scrdcg9kms4qbw062xiw54hchx3j1zpr` is the current hash. If it changes, find the correct one with:
```bash
find /nix/store -name 'dri_gbm.so' 2>/dev/null | grep armv7
```

### Finding the Correct Paths

The exact Nix store paths vary. Find them with:

```bash
# Find Mesa store path
MESA_PATH=$(find /nix/store -name "libgbm.so.1" 2>/dev/null | grep armv7 | head -1 | xargs dirname | xargs dirname)
echo "Mesa path: $MESA_PATH"

# Verify paths exist
ls -la $MESA_PATH/lib/gbm/           # Should contain dri_gbm.so
ls -la $MESA_PATH/lib/dri/           # Should contain etnaviv_dri.so
ls -la $MESA_PATH/share/glvnd/egl_vendor.d/  # Should contain 50_mesa.json

# Find glibc linker
LINKER=$(ls /nix/store/*glibc*armv7*/lib/ld-linux-armhf.so.3 2>/dev/null | head -1)
echo "Linker: $LINKER"
```

### Running the Software Renderer

```bash
# Find the glibc linker
LINKER=$(ls /nix/store/*glibc*armv7*/lib/ld-linux-armhf.so.3 2>/dev/null | head -1)

# Run with debug logging
RUST_LOG=debug $LINKER --library-path "$LD_LIBRARY_PATH" /tmp/bmc-compositor
```

### Running the EGL Renderer

The EGL renderer requires additional environment variables for GPU initialization:

```bash
# All the standard variables plus EGL vendor
export __EGL_VENDOR_LIBRARY_FILENAMES=$MESA_PATH/share/glvnd/egl_vendor.d/50_mesa.json

# Optional: override device paths (defaults shown)
export BMC_GPU_DEVICE=/dev/dri/renderD128    # GPU render node
export BMC_DISPLAY_DEVICE=/dev/dri/card1      # Display controller

# Run
RUST_LOG=debug $LINKER --library-path "$LD_LIBRARY_PATH" /tmp/bmc-compositor-egl
```

### Complete Setup Script

Create `/tmp/run-compositor.sh` on the device:

```bash
#!/bin/sh
# BMC Compositor launcher script
# Usage: ./run-compositor.sh [sw|egl]

MODE="${1:-sw}"

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

# Select binary
if [ "$MODE" = "egl" ]; then
    BINARY=/tmp/bmc-compositor-egl
    echo "Starting EGL compositor (GPU-accelerated)..."
else
    BINARY=/tmp/bmc-compositor
    echo "Starting software compositor..."
fi

# Run
exec $LINKER --library-path "$LD_LIBRARY_PATH" $BINARY
```

Usage:

```bash
chmod +x /tmp/run-compositor.sh

# Software renderer
RUST_LOG=info /tmp/run-compositor.sh sw

# EGL renderer
RUST_LOG=info /tmp/run-compositor.sh egl
```

## DRM Device Selection

### Software Renderer

The software renderer automatically scans for DRM devices and selects the first one with connectors:

- `/dev/dri/card0` - GPU (etnaviv) - render-only, no connectors
- `/dev/dri/card1` - Display (stm32-ltdc) - has DSI connector

### EGL Renderer

The EGL renderer uses both devices:

| Device | Path | Driver | Purpose |
|--------|------|--------|---------|
| GPU | `/dev/dri/renderD128` | etnaviv | EGL context, OpenGL ES rendering |
| Display | `/dev/dri/card1` | stm32-ltdc | DRM/KMS scanout |

Override with environment variables:

```bash
export BMC_GPU_DEVICE=/dev/dri/renderD128
export BMC_DISPLAY_DEVICE=/dev/dri/card1
```

The Braiins Deck uses a DSI panel connected to card1.

## Troubleshooting

### Common Issues

#### "No such file or directory" for libraries

Missing library path. Ensure `LD_LIBRARY_PATH` includes all armv7 libs:

```bash
export LD_LIBRARY_PATH=$(ls -d /nix/store/*armv7*/lib 2>/dev/null | tr '\n' ':')
```

#### "cannot open shared object file: dri_gbm.so"

GBM backend path not set:

```bash
export GBM_BACKENDS_PATH=$MESA_PATH/lib/gbm
```

#### Permission denied on /dev/dri/cardX

Run as root or ensure the user is in the `video` group:

```bash
ls -la /dev/dri/
# Run as root
```

#### "Failed to restore previous state" errors

Harmless warnings from smithay during cleanup. Occur when the compositor exits without having fully configured a display mode.

### EGL-Specific Issues

#### "Failed to create EGL display" or empty EGL extensions

The EGL vendor library is not configured. Set:

```bash
export __EGL_VENDOR_LIBRARY_FILENAMES=$MESA_PATH/share/glvnd/egl_vendor.d/50_mesa.json
```

Verify the file exists and contains valid JSON pointing to `libEGL_mesa.so.0`.

#### "Failed to allocate GBM buffer"

This occurs when trying to allocate buffers that can be shared between the GPU and display devices. Possible causes:

1. **Format mismatch**: The GPU and display don't support the same buffer format
2. **Memory constraints**: Not enough contiguous memory for the buffer
3. **Driver limitations**: etnaviv/stm32-ltdc DMA-BUF interop issues

Debug with:

```bash
# Check supported formats on each device
cat /sys/kernel/debug/dri/0/state  # GPU
cat /sys/kernel/debug/dri/1/state  # Display
```

#### EGL initializes but rendering fails

Check OpenGL ES capabilities:

```bash
# Should show: OpenGL ES 2.0 Mesa, Vivante GC400
RUST_LOG=debug /tmp/run-compositor.sh egl 2>&1 | grep -E "(GL Version|GL Renderer)"
```

### Diagnostic Commands

Run these on the device to verify the graphics stack:

```bash
# List DRM devices
ls -la /dev/dri/

# Check GPU driver
cat /sys/class/drm/card0/device/driver/module/description

# Check display driver  
cat /sys/class/drm/card1/device/driver/module/description

# List DRM connectors
cat /sys/class/drm/card1-DSI-1/status

# Check Mesa is loaded
ls $MESA_PATH/lib/dri/etnaviv_dri.so
```

## Display Information

The Braiins Deck display:
- Physical panel: 600x1280 (portrait)
- Displayed as: 1280x480 (landscape, rotated 90° CCW by compositor)
- Interface: DSI (MIPI Display Serial Interface)
- DRM connector type: DSI

## Features

### Software Renderer (`bmc-compositor`)

- **CPU rendering**: Uses DRM dumb buffers for software compositing
- **Simple setup**: No GPU driver requirements
- **90° rotation**: Rotates widget content to match landscape display orientation
- **Frame callbacks**: Proper Wayland frame synchronization
- **VBlank sync**: Tear-free display with page flipping

### EGL Renderer (`bmc-compositor-egl`)

- **GPU acceleration**: OpenGL ES 2.0 via Vivante GC400
- **Split architecture**: Renders on GPU, scans out on display controller
- **Dumb buffer + PRIME export**: Standard approach for split GPU/display systems
- **Zero-copy rendering**: GPU renders directly to display-owned buffers
- **Double buffering**: Smooth frame presentation
- **Frame callbacks**: Proper Wayland frame synchronization
- **VBlank sync**: Tear-free display with page flipping

#### How Buffer Sharing Works

The STM32MP157 has a split GPU/display architecture where the display controller (stm32-ltdc)
cannot import buffers from the GPU. The solution uses the reverse direction:

1. **Allocate dumb buffer on display** (stm32-ltdc via CMA)
2. **PRIME export** the buffer as DMA-BUF file descriptor
3. **Import into GPU** via EGL for OpenGL ES rendering
4. **GPU renders** to the display-owned buffer
5. **Display scans out** directly (buffer already in display memory)

This is the standard approach for embedded systems with separate GPU and display controllers.

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

### Software Renderer

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
- [ ] Multi-widget compositing

### EGL Renderer

- [x] Split GPU/display architecture support
- [x] EGL display creation on GPU device
- [x] OpenGL ES 2.0 context initialization
- [x] GlesRenderer setup
- [x] DRM surface on display device
- [x] Wayland socket creation
- [x] Double-buffered rendering
- [x] Dumb buffer allocation on display (CMA-backed)
- [x] PRIME export from display to GPU
- [x] DMA-BUF import into EGL for rendering
- [x] Widget display (digital-clock working as Wayland client)
- [ ] SHM buffer compositing
- [ ] Touch input forwarding
- [ ] Multi-widget compositing
