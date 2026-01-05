# Widget System Refactor Plan

This document outlines the refactoring of BMC from a monolithic Slint application to a compositor-based multi-process architecture with plugin widgets.

## Overview

### Current Architecture
- Single Slint application rendering all widgets
- Widgets are enum variants compiled into the binary
- Centralized DisplayController managing all state
- Single event loop for all UI updates

### Target Architecture
- Main Deck application embeds a Wayland compositor
- Each widget is a separate Wayland client application
- Widgets are installable packages (Nix packages)
- Communication via custom Wayland protocol extension
- Widget registry for discovery and management

---

## 1. Widget Registry & Discovery

### Purpose
Discover and track available widgets installed on the system.

### Description
The widget registry scans `/usr/lib/bmc-widgets/official/` and `/usr/lib/bmc-widgets/third-party/` directories on startup and watches for changes. Each subdirectory is a widget package (symlink to Nix store). When a new widget is installed or removed, the registry automatically updates without requiring application restart.

On startup and when changes are detected:
1. Scan both `official/` and `third-party/` directories for widget subdirectories
2. Load and parse `manifest.json` from each widget
3. Validate manifest schema and verify binary exists and is executable
4. Register valid widgets, skip invalid ones with warning log
5. Handle duplicate UIDs by keeping first found

The registry is a local service that tracks widgets installed on the device. It provides:
- Lookup by widget UID for spawning instances
- List of all available widgets for UI with full metadata:
  - Widget UID, name, description, version
  - Author information (name, URL)
  - Supported sizes
  - Parameter schema for rendering configuration forms in frontend
- Version information for compatibility checks

See [Widget Manifest](widget-manifest.md) for full manifest schema specification.

### Widget Directory Structure

```
/usr/lib/bmc-widgets/
├── official/                           # Preinstalled widgets
│   ├── clock/                          -> /nix/store/<hash>-clock/lib/bmc-widgets/clock/
│   │   ├── manifest.json
│   │   ├── bin/
│   │   │   └── clock
│   │   └── assets/
│   │       ├── icon.png
│   │       ├── preview-small.png
│   │       ├── preview-medium.png
│   │       ├── preview-large.png
│   │       └── preview-full.png
│   ├── ticker-btc/                     -> /nix/store/<hash>-ticker-btc/...
│   └── ...
└── third-party/                        # User-installed widgets (cleared on factory reset)
    ├── weather/                        -> /nix/store/<hash>-weather/...
    └── ...
```

This separation enables easy factory reset by removing the entire `third-party` directory.

### Error Handling
- Invalid manifest: Log warning, skip widget
- Missing binary: Log warning, skip widget
- Duplicate UID: Log warning, keep first found

---

## 2. Widget Installation

### Purpose
Allow users to install and update widgets via web UI without technical knowledge.

### How Widgets Are Distributed
- **Preinstalled widgets** - A limited set of core widgets are preinstalled on the device as part of the firmware image
- **Official widget store** - Additional official widgets are available for on-demand installation from the Braiins widget store (a curated list of verified widgets)
- **Third-party widgets** - Distributed as Nix flakes hosted on GitHub (or other Git hosting). The flake defines how to build the widget binary for ARMv7. Since the device is embedded and cannot compile, third-party widgets must have pre-built binaries available in a binary cache (e.g., Cachix or developer's own cache)

### Installation Flow (Web UI)

To be defined later 

---

## 3. Widget Instance Configuration

### Purpose
Store and manage user-created widget instances within scenes.

### Operations
- **Create** - Validate params against widget's schema, assign unique instance ID (UUID v4) to distinguish multiple instances of the same widget
- **Update params** - Validate new params, restart widget with new configuration
- **Resize** - Validate new size is supported by widget, verify widget fits into scene layout (for combined scenes), restart widget with new size
- **Move** - Update position within scene
- **Delete** - Stop widget process, remove from config

### Persistence
Widget instance configurations work the same as in the current system, but with widget UIDs instead of built-in widget type enums:

- **Storage location**: `/etc/bmc/config.json` - the same file used for all BMC settings (scenes, alarms, localization, etc.)
- **Save behavior**: Configuration is saved immediately after every change (create, update, resize, move, delete)
- **Load behavior**: On application startup, all widget instances are loaded from config and spawned according to their scene assignments

---

## 4. Widget Process Management

### Purpose
Spawn, monitor, and terminate widget processes.

### Spawn Sequence
1. Look up widget in registry
2. Spawn binary with environment variable `WAYLAND_DISPLAY` pointing to compositor socket
3. Widget connects to Wayland compositor
4. Compositor identifies widget by PID (from socket credentials)
5. Compositor sends configuration via Wayland protocol extension (size, params, settings)
6. Widget acknowledges configuration and starts rendering
7. Mark widget as ready

### Lifecycle Operations
- **Start** - Spawn process for instance
- **Stop** - Send shutdown via Wayland protocol, wait for exit, kill if timeout

### Crash Handling
- Monitor process exit
- Log error with exit code
- After N failures, mark widget as failed

### Cleanup
- On widget stop: Compositor unregisters widget surface
- On application exit: Stop all widgets gracefully

---

## 5. Widget Communication (Wayland Protocol Extension)

### Purpose
Bidirectional communication between compositor and widget processes via custom Wayland protocol.

### Protocol: `bmc_widget_v1`

Communication happens through a Wayland protocol extension that provides:

**Compositor → Widget:**
- `configure` - Widget size and type
- `params` - Widget instance parameters (JSON)
- `setting` - System setting update
- `visibility` - Widget visible/hidden state
- `shutdown` - Graceful shutdown request

**Widget → Compositor:**
- `ack_configure` - Acknowledge configuration
- `error` - Report error condition
- `action` - Request system action (sound, LED)

### Action Handling
When widget sends action request:
- `play_sound` → Forward to SoundController
- `stop_sound` → Forward to SoundController
- `led` → Forward to LedController
- `stop_led` → Forward to LedController

### Settings Broadcast
When system setting changes:
1. Get list of widgets subscribed to that setting (from manifest)
2. Send `setting` event to each subscribed widget via Wayland protocol

---

## 6. Compositor / Display Management

### Purpose
Compose widget surfaces into scenes and handle display output.

### Surface Management
- Accept Wayland connections from widget processes
- Track surfaces by instance ID
- Position surfaces according to scene layout
- Handle surface creation/destruction

### Scene Rendering
- Render active scene's widgets at correct positions
- Pre-render adjacent scenes for smooth transitions
- Handle scene transitions (slide left/right)

### Grid Layout

| Size | Grid Cells | Pixels |
|------|-----------|--------|
| small | 1x1 | 317x238 |
| medium | 2x1 | 638x238 |
| large | 2x2 | 638x480 |
| full | 4x2 | 1280x480 |

### Gesture Handling
- Horizontal swipe - trigger scene transitions (left/right)
- Vertical swipe - show control screen (brightness, volume, etc.)
- Forward touch events to widgets when not gesturing

### Overlay Management
Overlays rendered above all widgets:
- Alarm screen (blocks interaction)
- Upgrade screen (blocks interaction)
- WiFi status indicator (non-blocking)
- Control center

### Night Mode
When night mode activates/deactivates:
1. Broadcast `settings_update` with `nightMode` to subscribed widgets
2. Adjust compositor brightness if applicable

---

## 7. Built-in Widgets Extraction

### Purpose
Convert existing compiled-in widgets to standalone widget packages.

### Widgets to Extract

| Current Widget | Widget Name | Description |
|----------------|-------------|-------------|
| Clock | clock | Analog/digital clock |
| TickerBtc | ticker-btc | Bitcoin price ticker |
| BlockHeight | block-height | Blockchain block height |
| BraiinsPool | braiins-pool | Mining pool stats |
| RemoteImage | remote-image | HTTP image display |
| BlockchainData | blockchain-data | Difficulty/hashrate charts |

Each widget will have a unique UID (UUID v4) generated during creation.

### Extraction Process (per widget)
1. Create new crate
2. Move Slint UI files
3. Move data polling logic
4. Create manifest.json
5. Create binary entry point with IPC connection
6. Package as Nix derivation
7. Test standalone

### Shared Library
Create `bmc-widget-sdk` crate with:
- Wayland protocol client implementation
- Common utilities
- Slint helpers

---

## 8. Configuration Migration

### Purpose
Automatically migrate existing configurations to new format.

### Detection
Check config version or presence of old widget format

### Migration Map

| Old Widget | New widget | Param Mapping |
|------------|------------|---------------|
| Clock | clock | Direct mapping |
| TickerBtc | ticker-btc | Direct mapping |
| BlockHeight | block-height | Direct mapping |
| BraiinsPool | braiins-pool | Direct mapping |
| RemoteImage | remote-image | Direct mapping |
| BlockchainData | blockchain-data | Direct mapping |
| RemoteWidget | Keep as-is | Special handling |

Widget UIDs (UUIDs) will be resolved from widget names during migration.

### Handling Missing Widgets
If migrated config references a widget that's not installed:
- Log warning
- Keep widget in config but mark as "unavailable"
- Show placeholder in UI
- Don't spawn process

### Backup
Before migration:
- Copy config to backup file with timestamp
- Log migration event

---

## 9. Frontend API Changes

### Purpose
Update gRPC API to support standalone widgets.

### New Endpoints

#### Widget Discovery
- **ListWidgets** - Return all available widgets with summary info
- **GetWidget** - Return full widget details including param schema

#### Widget Instance Management
- **AddWidget** - Now takes widget UID instead of widget type
- **UpdateWidget** - Support partial param updates
- **Widget status** - Include running/error/unavailable state

### Validation
- On AddWidget: Validate params against widget's schema
- On UpdateWidget: Validate partial params
- Return detailed errors for validation failures

---

## Open Questions
1. **Widget updates** - How to handle widget version upgrades?
2. **Resource limits** - Should widgets have memory/CPU limits?


---

## Related Documents

- [Widget Manifest](widget-manifest.md) - Manifest schema specification
- [Widget IPC Protocol](widget-ipc-protocol.md) - Legacy IPC protocol (outdated, replaced by Wayland protocol extension)
- [Widget Refactor Implementation](widget-refactor-implementation.md) - Detailed implementation plan
