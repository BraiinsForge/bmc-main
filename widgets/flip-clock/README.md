# Flip-Clock Widget

GPU-accelerated clock widget with split-flap animation for the Braiins Deck.

## Overview

The flip-clock widget is a Wayland client that uses OpenGL ES for GPU-accelerated rendering with zero-copy buffer
sharing via DMA-BUF. It displays time in HH:MM:SS format with a classic split-flap display animation.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    flip-clock widget                         │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────────┐  │
│  │ Time Logic   │  │ Animation    │  │ Wayland Client    │  │
│  │              │  │ State        │  │                   │  │
│  │ - Get time   │  │ - Flip angle │  │ - wl_surface      │  │
│  │ - Detect     │  │ - Easing     │  │ - frame callback  │  │
│  │   changes    │  │              │  │ - linux-dmabuf    │  │
│  └──────┬───────┘  └──────┬───────┘  └─────────┬─────────┘  │
│         │                 │                    │            │
│         └─────────────────┼────────────────────┘            │
│                           │                                  │
│                    ┌──────▼───────┐                         │
│                    │ OpenGL ES    │                         │
│                    │ Renderer     │                         │
│                    │              │                         │
│                    │ - glow       │                         │
│                    │ - Shaders    │                         │
│                    │ - Textures   │                         │
│                    └──────┬───────┘                         │
│                           │                                  │
│                    ┌──────▼───────┐                         │
│                    │ EGL + GBM    │                         │
│                    │              │                         │
│                    │ - Context    │                         │
│                    │ - DMA-BUF    │                         │
│                    └──────────────┘                         │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

## Features

- **Two animation modes**:
  - **2D Flat** (`--mode flat`): Classic split-flap animation with textured digits
  - **3D Extruded** (`--mode extruded`, default): 3D geometry with perspective and lighting
- **GPU acceleration**: OpenGL ES 2.0 via Vivante GC400
- **Zero-copy rendering**: DMA-BUF buffer sharing with compositor
- **High-quality typography**: ab_glyph font rendering with Braiins Deck Sans Bold
- **Smooth animation**: 60fps with easing

## Technology Stack

| Component | Technology | Purpose | |-----------|-----------|---------| | Rendering | OpenGL ES 2.0 (glow) |
GPU-accelerated graphics | | EGL Context | smithay EGL wrapper | Cross-platform EGL initialization | | Buffer Management
| GBM (Generic Buffer Manager) | DMA-BUF allocation and export | | Wayland Protocol | wayland-client | Compositor
communication | | Font Rendering | ab_glyph | Digit texture generation | | 3D Tessellation | lyon | Font outline to 3D
mesh conversion | | Time Handling | chrono, chrono-tz | Timezone-aware time display |

## Building

```bash
# For ARMv7 device (glibc required for Wayland/EGL)
nix develop .#armv7-glibc-release --command cargo build -p bmc-widget-flip-clock --release
```

Binary location: `target/armv7-unknown-linux-gnueabihf/release/bmc-widget-flip-clock`

## Usage

```bash
# 3D extruded mode (default)
./bmc-widget-flip-clock

# 2D flat split-flap mode
./bmc-widget-flip-clock --mode flat

# Show help
./bmc-widget-flip-clock --help
```

## Running on Device

The widget requires the compositor to be running and needs proper environment setup for glibc binaries on the musl-based
OpenWRT system.

### Environment Setup

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

# Run widget (3D mode)
RUST_LOG=info $LINKER --library-path "$LD_LIBRARY_PATH" /tmp/bmc-widget-flip-clock

# Run widget (2D mode)
RUST_LOG=info $LINKER --library-path "$LD_LIBRARY_PATH" /tmp/bmc-widget-flip-clock --mode flat
```

### Prerequisites

The compositor must be running before starting the widget:

```bash
# Start compositor first (see bmc-compositor/README.md)
$LINKER --library-path "$LD_LIBRARY_PATH" /tmp/bmc-compositor-egl
```

## Architecture Details

### Rendering Pipeline

1. **Wayland Client**: Connects to compositor, creates surface
2. **EGL Context**: Initialized on GPU device (`/dev/dri/renderD128`)
3. **GBM Buffers**: Allocated with `RENDERING | LINEAR` flags
4. **EGLImage**: Binds GBM buffer to OpenGL texture
5. **Framebuffer**: Renders to EGLImage-backed texture
6. **DMA-BUF Export**: Exports GBM buffer as file descriptor
7. **Wayland Buffer**: Creates `wl_buffer` from DMA-BUF
8. **Compositor**: Displays buffer with zero-copy

### Animation Modes

#### 2D Flat Mode

- Digit split horizontally at center
- Top half flips down around center hinge
- Uses textured quads with perspective transform
- Shows old digit's top → new digit's bottom on flap

#### 3D Extruded Mode

- Font outlines tessellated with lyon
- Geometry extruded into 3D with depth
- Lit with directional light showing edge faces
- Whole digit rotates around X axis (180°)
