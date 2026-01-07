# Widget System Refactor - Implementation Plan

This document provides a detailed, stage-by-stage implementation plan for refactoring the BMC widget system from a monolithic Slint application to a compositor-based multi-process architecture.

## Prerequisites

Before starting, ensure familiarity with:
- [Widget System Refactor Plan](widget-system-refactor-plan.md) - Architecture overview
- [Widget Manifest Specification](widget-manifest.md) - Manifest schema
- [Widget IPC Protocol](widget-ipc-protocol.md) - Communication protocol

## Code Style

- Prefer composition over inheritance
- Use traits for abstraction and testability
- Keep modules focused with single responsibility
- Avoid global state - pass dependencies explicitly as parameters
- Write code that is easy to test in isolation

## Commit Requirements

Each commit must:
- Pass `cargo clippy`
- Pass `nix fmt`
- Pass all existing and new tests
- Follow commit message format defined in [CLAUDE.md](../../../CLAUDE.md#commit-message-format)

## Current Architecture Summary

- Single Slint application rendering all widgets
- Widgets defined as enum variants in `WidgetKind` (`bmc-display/src/data.rs`)
- `DisplayController` manages all widget state centrally
- Widget data polling via async tasks in `bmc/src/widget_tasks/`
- Configuration persisted to `/etc/bmc/config.json`

---

## Stage 1: IPC Crate Setup

### Goal
Create the `bmc-ipc` crate for IPC communication between the application and widgets.

### Scope
- Create new `bmc-ipc` crate
- Define error types

### Files to Create

```
bmc-ipc/
  Cargo.toml
  src/
    lib.rs
```

### Dependencies

- `serde` for serialization traits
- `thiserror` for error types

### Status: Complete

---

## Stage 2: JsonLinesCodec Implementation

### Goal
Implement a generic JSON Lines codec that combines newline framing with JSON encoding/decoding using `tokio_util::codec`.

### Scope
- Implement generic `JsonLinesCodec<Dec, Enc>` struct using `tokio_util::codec::{Encoder, Decoder}`
- `Dec` - type to decode from incoming messages
- `Enc` - type to encode for outgoing messages
- Use `LinesCodec` for newline framing
- Use `serde_json` for JSON encoding/decoding
- Add `CodecError` for error handling
- Add unit tests

### Test Cases

1. **Encoding**
   - Verify messages encode to JSON with newline delimiter

2. **Decoding**
   - Parse valid JSON lines
   - Handle incomplete messages (buffering)
   - Error on malformed JSON

### Success Criteria

- [x] `JsonLinesCodec<Dec, Enc>` is generic over decode/encode types
- [x] `CodecError` type defined
- [x] Unit tests pass for encoding/decoding

### Dependencies

- `serde_json` for JSON handling
- `tokio-util` with codec feature
- `bytes` for BytesMut

### Status: Complete

---

## Stage 3: IPC Message Types

### Goal
Define all types and messages for IPC communication in the `bmc-ipc` crate.

### Scope
- Add types and messages to `bmc-ipc` crate
- Add serialization tests to verify JSON format matches spec

### Data Types

- `SizeType` - enum (Small, Medium, Large, Full)
- `SizeInfo` - size type, width, height
- `Localization` - dateFormat, timeFormat, numberFormat, temperatureUnit, firstDayOfWeek
- `ActionPayload` - PlaySound, StopSound, Led, StopLed

### Message Types

**Application to Widget (`AppMessage`):**
- `Init` - size, params, settings
- `SettingsUpdate` - key, value
- `Shutdown`

**Widget to Application (`WidgetMessage`):**
- `Ready`
- `Error` - message, recoverable
- `Action` - name, payload

### Test Cases

1. **Serialization**
   - Each type serializes to expected JSON format
   - JSON field names match spec (camelCase)
   - JSON output matches examples in `widget-ipc-protocol.md`

2. **Deserialization**
   - Parse valid JSON for each type
   - Error on missing required fields

### Success Criteria

- [x] All data types implemented with serde derives
- [x] All message types from IPC protocol spec implemented
- [x] JSON output matches examples in `widget-ipc-protocol.md`
- [x] Unit tests pass for serialization/deserialization

### Notes

- No runtime behavior yet - pure data types
- No changes to existing `bmc` or `bmc-display` crates

### Status: Complete

---

## Stage 4: Widget Manifest Parser

### Goal
Parse and validate widget manifest files (`manifest.json`) according to the specification.

### Scope
- Create new `bmc-widget` crate for widget-related types and abstractions
- Define manifest data types
- Implement manifest parsing and validation
- Add comprehensive tests

### Data Types

- `Manifest` - uid, version, name, description, author, binary, settings, sizes, params
- `Author` - name, url
- `SettingKey` - enum (Localization, Timezone, NightMode)
- `ParamDefinition` - name, param_type, description, default, enum_values, min, max
- `ParamType` - enum (String, Boolean, Number, Array)

### Validation Rules

1. `uid` must be valid UUID v4
2. `version` must be valid semver
3. `name` max 50 characters
4. `description` max 200 characters
5. `sizes` must have at least one entry
6. `params` default values must match declared type
7. `params` enum values must be provided if using enum constraint

### Test Cases

1. **Valid Manifest**
   - Parse complete manifest with all fields
   - Parse minimal manifest (required fields only)

2. **Invalid Manifest**
   - Missing required fields
   - Invalid UUID format
   - Invalid semver
   - Empty sizes array
   - Type mismatch in param defaults

3. **Param Validation**
   - String with enum constraint
   - Number with min/max
   - Boolean default

### Success Criteria

- [x] All manifest types defined with serde derives
- [x] `Manifest::from_reader()` parses JSON
- [x] `FromStr` trait implemented for `Manifest`
- [x] Validation errors are descriptive
- [x] Unit tests cover all validation rules (7 tests)

### Dependencies

- `uuid` for UUID parsing
- `semver` for version parsing

### Status: Complete

---

## Stage 5: Widget Registry

### Goal
Discover and track available widgets installed on the system.

### Scope
- Add `WidgetDiscovery` trait to `bmc-widget` crate for platform abstraction
- Add `PathDiscovery` struct to `bmc-widget` crate for filesystem-based discovery
- Add `WidgetRegistry` struct to `bmc-widget` crate as a pure data container
- Provide lookup and listing APIs
- Handle invalid/duplicate widgets gracefully

### Platform Abstraction

The `bmc-widget` crate defines a `WidgetDiscovery` trait that abstracts widget discovery:

- `WidgetDiscovery::discover()` - Async method returning `Vec<WidgetInfo>`

Discovery implementations return loaded `WidgetInfo` directly, handling all loading logic internally.

**PathDiscovery (in bmc-widget):**
- Reusable filesystem-based discovery
- Takes a list of paths to scan
- Handles manifest parsing, binary validation, and error logging

**Platform usage:**

- **bmc-openwrt:** Uses `PathDiscovery::new(vec!["/usr/lib/bmc-widgets/official/", "/usr/lib/bmc-widgets/third-party/"])`
- **bmc-mock:** Uses `PathDiscovery::new(paths)` where paths come from CLI argument `--widgets-path`

### Widget Directory Structure

All widgets (both platforms) follow the same structure when built by Nix:

```
<widget-name>/
  manifest.json
  bin/
    <binary-name>
  assets/
    icon.png
    preview-*.png
```

### Widget Loading

`PathDiscovery` handles all loading internally:
1. Scan each configured path for subdirectories
2. Read `manifest.json` from each subdirectory
3. Parse and validate manifest
4. Resolve binary path relative to widget directory
5. Verify binary exists and is executable
6. Return valid widgets, log warnings for invalid ones

### API

- `WidgetDiscovery` trait - Platform-agnostic discovery interface
- `PathDiscovery::new(paths)` - Create filesystem-based discovery
- `PathDiscovery::discover()` - Scan and return all valid widgets
- `WidgetRegistry::new(widgets)` - Create registry from widget iterator
- `WidgetRegistry::get(uid)` - Get widget by UID
- `WidgetRegistry::list()` - List all available widgets
- `WidgetRegistry::supports_size(uid, size)` - Check if widget supports a given size
- `WidgetInfo` - Contains manifest, widget directory path, and absolute binary path

### Error Handling

- Invalid manifest: Log warning, skip widget, continue scanning
- Missing binary: Log warning, skip widget
- Non-executable binary: Log warning, skip widget
- Duplicate UID: Log warning, keep first found
- Non-existent scan path: Log warning, continue with other paths

### Test Cases

1. **PathDiscovery (filesystem tests)**
   - Empty directory returns no widgets
   - Non-existent path handled gracefully
   - Single valid widget discovered
   - Multiple widgets from one path
   - Multiple scan paths combined
   - Invalid manifest skipped
   - Missing binary skipped
   - Non-executable binary skipped
   - Files in scan directory ignored (only directories scanned)
   - Widget paths correctly populated

2. **WidgetRegistry (unit tests with mock data)**
   - Empty registry
   - Single widget lookup
   - Multiple widgets
   - Duplicate UID keeps first
   - Get non-existent widget returns None
   - Size support check
   - List all widgets

3. **Error Cases**
   - Directory doesn't exist
   - Invalid manifest JSON
   - Missing binary file
   - Non-executable binary

### Success Criteria

- [x] `WidgetDiscovery` trait returns `Vec<WidgetInfo>` directly
- [x] `PathDiscovery` scans directories correctly
- [x] `WidgetRegistry` accepts widgets directly (decoupled from discovery)
- [x] Invalid widgets logged and skipped
- [x] Duplicate UIDs handled
- [x] Unit tests for registry with mock `WidgetInfo`
- [x] Integration tests for `PathDiscovery` with temp directories

### Status: Complete

---

## Stage 6: Widget Process Spawner

### Goal
Spawn and manage widget processes with IPC communication.

### Scope
- Define `ProcessSpawner` trait for spawning widget processes
- Define `WidgetConnection` trait for IPC communication
- Implement `UnixSpawner` and `UnixConnection` using Unix sockets

### Trait Abstractions

**ProcessSpawner trait:**
- `spawn(widget_info, instance_id, init_msg)` - Spawn a new widget process, returns `WidgetConnection`

**WidgetConnection trait:**
- `send(msg)` - Send message to widget
- `recv()` - Receive message from widget

**UnixSpawner:**
- Configurable connection and handshake timeouts
- No generics - uses hardcoded codec type

**UnixConnection:**
- Wraps `Framed<UnixStream, JsonLinesCodec<WidgetMessage, AppMessage>>`
- Cleans up socket file on drop
- No generics - codec type is fixed

### Spawn Sequence

1. Create socket directory if needed
2. Remove stale socket if exists
3. Bind Unix socket at `<socket_dir>/<instance-id>.sock`
4. Spawn binary with `BMC_IPC_SOCKET` env var
5. Accept connection on socket (configurable timeout, default 5s)
6. Send `init` message
7. Wait for `ready` message (configurable timeout, default 5s)
8. Return `UnixConnection` handle

### Shutdown

- Process is killed on drop via `kill_on_drop(true)`
- Socket file is cleaned up in `Drop` implementation

### Message Framing

Uses `JsonLinesCodec<WidgetMessage, AppMessage>`:
- Decodes `WidgetMessage` from widget
- Encodes `AppMessage` to widget
- Newline-delimited JSON format

### Test Cases

1. **Spawn**
   - Successfully spawn mock widget
   - Handle spawn timeout
   - Handle connection refused

2. **Communication**
   - Send/receive messages
   - Handle malformed JSON

3. **Lifecycle**
   - Graceful shutdown
   - Force kill on timeout
   - Detect crash

### Success Criteria

- [x] Socket creation and cleanup works
- [x] Process spawns with correct env vars
- [x] Init/ready handshake works
- [ ] Graceful and forced shutdown works
- [ ] Integration tests with mock widget binary

### Status: In Progress

---

## Stage 7: Widget Client SDK

### Goal
Provide client-side IPC infrastructure for widgets to communicate with the main application.

### Scope
- Implement `WidgetClient` for socket connection and message handling
- Provide helpers for common widget patterns
- Keep in `bmc-widget` crate (widget SDK)

### Crate Organization

The `bmc-widget` crate serves as the widget SDK containing:
- `manifest.rs` - Widget manifest parsing (already implemented)
- `client.rs` - New: IPC client for widgets

### WidgetClient

**Purpose:** Connect to the main application and handle IPC protocol.

**API:**
- `WidgetClient::connect()` - Read `BMC_IPC_SOCKET` env var, connect to socket
- `WidgetClient::recv() -> AppMessage` - Receive command from application
- `WidgetClient::send(WidgetMessage)` - Send response to application
- `WidgetClient::send_ready()` - Convenience method to send Ready message
- `WidgetClient::send_error(msg, recoverable)` - Convenience method to send Error message

### Connect Sequence (Widget Perspective)

1. Read `BMC_IPC_SOCKET` from environment
2. Connect to Unix socket
3. Wait for `Init` message from application
4. Initialize widget state from init params (size, params, settings)
5. Send `Ready` message
6. Enter main loop handling `SettingsUpdate` and `Shutdown` messages

### Error Handling

- Missing `BMC_IPC_SOCKET` env var → exit with error
- Connection failure → exit with error
- Unexpected message → send `Error` message with `recoverable: false`
- Recoverable errors (e.g., network fetch failed) → send `Error` with `recoverable: true`

### Test Cases

1. **Connection**
   - Connect to socket from env var
   - Handle missing env var
   - Handle connection failure

2. **Protocol**
   - Receive Init message
   - Send Ready message
   - Receive SettingsUpdate message
   - Receive Shutdown message

3. **Error Handling**
   - Send Error message (recoverable)
   - Send Error message (non-recoverable)

### Success Criteria

- [x] `WidgetClient` connects and handles IPC protocol
- [x] Unit tests for client connection and message handling
- [x] Example usage documented

### Status: Complete

---

## Stage 8: Wayland Compositor Core

### Goal

Create a standalone Wayland compositor that can display widget surfaces on the embedded device using EGL/GPU rendering.

### Background

The `bmc-compositor` crate contains a working Wayland compositor using Smithay. It runs on ARMv7 with the STM32MP157 SoC's split GPU/display architecture (Vivante GC400 GPU + STM32 LTDC display controller).

### What Was Implemented

1. **Compositor Crate Structure** (`bmc-compositor/`)
   - Smithay-based Wayland server with protocol handlers
   - DRM/KMS backend for direct display output
   - EGL rendering with split GPU/display architecture
   - Frame callback handling for widget synchronization

2. **Protocol Handlers** (`bmc-compositor/src/state.rs`)
   - `CompositorHandler` - Surface creation and commits
   - `ShmHandler` - Shared memory buffers
   - `BufferHandler` - Buffer lifecycle
   - `SeatHandler` - Input handling (keyboard, pointer, touch)
   - `XdgShellHandler` - Window management
   - `SelectionHandler` - Copy/paste support
   - `DataDeviceHandler` - Drag and drop

3. **EGL Rendering** (`bmc-compositor/src/render_egl.rs`)
   - Dumb buffer allocation on display device (CMA-backed)
   - PRIME export as DMA-BUF
   - GPU import and rendering via OpenGL ES
   - Zero-copy buffer sharing between GPU and display
   - 90° rotation for portrait panel in landscape mode

4. **DRM Backend** (`bmc-compositor/src/drm_backend.rs`)
   - Device discovery via udev
   - Display mode configuration (480x1280 physical, 1280x480 logical)
   - CRTC and connector selection
   - Page flip handling

### Key Files

| File | Purpose |
|------|---------|
| `bmc-compositor/src/state.rs` | Wayland protocol state and handlers |
| `bmc-compositor/src/render_egl.rs` | EGL render pipeline |
| `bmc-compositor/src/drm_backend.rs` | DRM device management |
| `bmc-compositor/src/main_egl.rs` | EGL compositor entry point |
| `bmc-compositor/src/main.rs` | Software renderer entry point |

### Build Commands

```bash
# ARMv7 cross-compilation (must use glibc, not musl)
nix develop .#armv7-glibc-release --command cargo build -p bmc-compositor --bin bmc-compositor-egl --release
```

### Hardware Notes

- **GPU**: Vivante GC400 (etnaviv driver) - `/dev/dri/renderD128`
- **Display**: STM32 LTDC controller - `/dev/dri/card1`
- **Panel**: MIPI-DSI, 600x1280 @ 63Hz (480x1280 visible)
- **Buffer sharing**: Display allocates dumb buffers, PRIME exports to GPU for rendering

### Success Criteria

- [x] Compositor builds for ARMv7
- [x] DRM/KMS display initialization works
- [x] Wayland socket created for clients
- [x] Widget surfaces composited to display
- [x] Frame callbacks sent to widgets
- [x] Works on actual device

### Status: Complete

---

## Stage 9: Digital Clock Widget Extraction

### Goal
Extract the digital clock widget as the first standalone widget to validate the architecture.

### Scope
- Create new `digital-clock` crate
- Move clock Slint UI
- Implement IPC client
- Create manifest.json
- Package as Nix derivation

### Why Clock First

- Simplest widget with minimal dependencies
- No external data fetching
- Good test case for IPC and rendering

### Crate Structure

```
bmc-widget-clock/
  Cargo.toml
  src/
    main.rs          # Entry point, IPC connection
    clock.rs         # Clock logic
  ui/
    clock.slint      # Moved from bmc-display
  assets/
    icon.png
    preview-small.png
    preview-medium.png
    preview-large.png
    preview-full.png
  manifest.json
```

### IPC Client Behavior

1. Read `BMC_IPC_SOCKET` environment variable
2. Connect to the socket
3. Receive `init` message with size, params, settings
4. Initialize Slint UI
5. Send `ready` message
6. Run event loop, handle `settings_update` messages

### Manifest

```json
{
  "uid": "550e8400-e29b-41d4-a716-446655440000",
  "version": "1.0.0",
  "name": "Clock",
  "description": "Analog and digital clock display",
  "author": {
    "name": "Braiins",
    "url": "https://braiins.com"
  },
  "binary": "bin/clock",
  "settings": ["localization", "timezone", "nightMode"],
  "sizes": ["small", "medium", "large", "full"],
  "params": {
    "style": {
      "name": "Clock Style",
      "type": "string",
      "description": "Visual style of the clock",
      "default": "digital",
      "enum": {
        "digital": "Digital",
        "analog": "Analog"
      }
    },
    "showSeconds": {
      "name": "Show Seconds",
      "type": "boolean",
      "description": "Display seconds on the clock",
      "default": false
    }
  }
}
```

### Test Cases

1. **Standalone Execution**
   - Widget starts with mock IPC
   - Handles init message correctly
   - Sends ready message

2. **Settings Updates**
   - Responds to timezone change
   - Responds to night mode change
   - Responds to localization change

3. **Rendering**
   - Correct size rendering
   - Style parameter works
   - Time updates correctly

### Success Criteria

- [x] Clock widget runs as standalone process
- [x] IPC handshake works
- [x] Settings updates work
- [x] All sizes render correctly
- [x] Nix package builds

### Dependencies

- Stage 3 (IPC Message Types)
- Stage 6 (Process Spawner) - for integration testing
- Stage 7 (Widget Client SDK) - for IPC client

### Status: Complete

---

## Stage 10: Add Widget System (Alongside Existing)

### Goal
Add the new multi-process widget system alongside the existing monolithic Slint display. Both systems run in parallel during this stage.

### Scope
- Add `--widgets-path` CLI argument to bmc-mock
- Add `WidgetRegistry` initialization to startup
- Create `WidgetManager` for spawning and managing widget processes
- Wire up settings broadcasting to widgets
- Wire up action routing from widgets to controllers
- Existing monolithic display continues to work unchanged

---

### Step 1: Add CLI Argument

**File:** `bmc-mock/src/cli.rs`

- Add `--widgets-path` argument (optional `PathBuf`)
- When provided, enables the new widget system

---

### Step 2: Add WidgetRegistry Initialization

**File:** `bmc-mock/src/main.rs`

- If `--widgets-path` provided:
  - Create `PathDiscovery` with the path
  - Call `discovery.discover()` to find widgets
  - Create `WidgetRegistry` from discovered widgets
  - Log available widgets
- Pass optional `WidgetRegistry` to bmc entry

---

### Step 3: Update bmc Entry Point

**File:** `bmc/src/entry.rs`

- Add optional `WidgetRegistry` parameter
- Pass it to `App::init()`

---

### Step 4: Create WidgetManager

**File:** `bmc/src/widget_manager.rs` (new)

Responsibilities:
- Hold `WidgetRegistry` and `ProcessSpawner`
- Track running widget instances by instance ID
- Spawn widget processes with IPC sockets
- Stop widget processes
- Broadcast settings updates to subscribed widgets
- Route widget actions to controllers

**API:**
- [x] `init(widgets_paths)` - Create with paths, does discovery internally
- [x] `spawn_widget(widget_uid, instance_id, init_msg)` - Spawn a widget
- [x] `stop_widget(instance_id)` - Stop a widget
- [ ] `stop_all()` - Stop all widgets
- [x] `broadcast_settings(update)` - Send to all connected widgets
- [x] `send_message(instance_id, msg)` - Send message to specific widget
- [ ] `poll_messages()` - Check for incoming messages from widgets

---

### Step 5: Integrate WidgetManager into App

**File:** `bmc/src/startup.rs`

- Add optional `WidgetManager` field to `App` struct
- If `WidgetRegistry` provided in init:
  - Create `UnixSpawner` with socket directory
  - Create `WidgetManager` with registry and spawner
- Keep existing `WidgetTasks` unchanged (both systems run in parallel)

---

### Step 6: Wire Up Settings Broadcasting

**File:** `bmc/src/system_manager.rs` (or new file)

- When localization settings change, call `widget_manager.broadcast_settings_update()`
- When timezone changes, call `widget_manager.broadcast_settings_update()`
- When night mode changes, call `widget_manager.broadcast_settings_update()`

---

### Step 7: Wire Up Action Routing

**File:** `bmc/src/widget_manager.rs` or `bmc/src/startup.rs`

- Spawn task to poll widget messages
- When `WidgetMessage::Action` received:
  - `play_sound` → `SoundController::play()`
  - `stop_sound` → `SoundController::stop()`
  - `led` → `LedController::set_effect()`
  - `stop_led` → `LedController::stop()`

---

### Test Cases

1. **Without --widgets-path**
   - bmc-mock starts normally with existing behavior
   - No widget processes spawned

2. **With --widgets-path**
   - WidgetRegistry initialized
   - Available widgets logged
   - Both old and new systems run

3. **Widget Spawning**
   - Can manually spawn widget via WidgetManager
   - IPC handshake completes (init → ready)

4. **Settings Broadcast**
   - Timezone change reaches widget
   - Localization change reaches widget
   - Night mode change reaches widget

5. **Action Routing**
   - Widget sound action triggers SoundController
   - Widget LED action triggers LedController

### Success Criteria

- [x] `--widgets-path` argument added to bmc-mock
- [x] WidgetRegistry loads widgets from path
- [x] WidgetManager can spawn widget processes
- [x] IPC handshake works (init → ready)
- [ ] Settings updates propagate to widgets
- [ ] Actions from widgets route to controllers


### Status: In Progress

---

## Stage 11: Compositor Trait Abstraction

### Goal

Define a compositor interface trait in `bmc` that abstracts away the implementation details, allowing different backends (EGL for ARMv7, mock/no-op for x86 development).

### Scope

- Define `Compositor` trait in `bmc` crate
- Define minimal supporting types for widget display configuration
- Trait must support async operations and be thread-safe
- No implementation yet - just the interface

### Files to Create/Modify

- `bmc/src/compositor.rs` (new) - Trait definition and types
- `bmc/src/lib.rs` - Export compositor module

### Trait Design

The `Compositor` trait provides the interface between the main application and the compositor:

| Method | Description |
|--------|-------------|
| `start()` | Start the compositor, returns the Wayland display socket name |
| `wayland_display()` | Get the Wayland display socket name for widgets |
| `register_widget()` | Register a widget with instance_id and position (before spawning) |
| `unregister_widget()` | Unregister a widget when its process stops |
| `set_active_scene()` | Set which widgets are visible and their positions |
| `broadcast_setting()` | Send a setting update to all connected widgets |
| `action_receiver()` | Get a channel receiver for widget actions (sound, LED requests) |
| `event_receiver()` | Get a channel receiver for compositor events (widget ready, disconnected) |
| `shutdown()` | Shutdown the compositor |

### Usage

The compositor is passed to `Coordinator` as `Arc<dyn Compositor>`. This allows:
- Shared ownership between Coordinator, App, and other components
- Thread-safe access from async tasks
- Runtime polymorphism for different backends (EGL on ARMv7, mock on x86)

### Supporting Types

| Type | Description |
|------|-------------|
| `SceneLayout` | List of widget placements with instance_id, position (x,y), size (w,h), and visibility |
| `WidgetAction` | Action request from a widget: instance_id and action payload (reuses `ActionPayload` from bmc-ipc) |
| `CompositorEvent` | Event from compositor: `WidgetReady { instance_id }`, `WidgetDisconnected { instance_id }` |
| `CompositorError` | Error enum for compositor operations |

### Threading Model

The compositor implementation runs in a **separate thread** (not a tokio task). This is because:

1. **Smithay uses calloop**: The `bmc-compositor` code uses smithay's calloop event loop, which is synchronous and blocking. Calloop is designed for Wayland compositors and integrates with DRM, libinput, and other low-level Linux APIs.

2. **Incompatible with tokio**: Calloop's blocking event loop cannot run inside tokio's cooperative async runtime without blocking the executor.

3. **Clean separation**: The compositor thread handles rendering and Wayland protocol; the main tokio runtime handles business logic and web services. Communication via channels is simple and the message rate is low (scene changes, widget registrations).

Implementations should use `std::thread::spawn` or `tokio::task::spawn_blocking` to run the compositor loop.

### Success Criteria

- [x] `Compositor` trait defined with async methods
- [x] Supporting types defined (minimal set)
- [x] Trait is `Send + Sync` for thread-safe access
- [x] Error types defined

### Status: Complete

---

## Stage 12: Wayland Protocol Extension for Widget Communication

### Goal

Implement a Wayland protocol extension (`deck_widget_v1`) for compositor-widget communication. This replaces the need for separate JSON IPC sockets.

### Background

A Wayland protocol extension:
- Uses a single connection for all communication (rendering + messages)
- Provides type-safe, versioned messages
- Generates client/server bindings automatically via `wayland-scanner`
- Is the standard approach for Wayland compositors

### Protocol Structure

**`deck_widget_manager_v1`** (global singleton):
- Bound by widgets to access BMC functionality
- Provides `get_widget_surface` request

**`deck_widget_surface_v1`** (per-surface):

| Direction | Message | Description |
|-----------|---------|-------------|
| Compositor → Widget | `setting` | System setting update (timezone, localization, night mode) |
| Compositor → Widget | `shutdown` | Graceful shutdown request |
| Widget → Compositor | `request_action` | Request system action (sound, LED) |

Note: Initial configuration (size, params, settings) is passed via environment variables. Widget readiness is implicit when the first buffer is committed (standard Wayland behavior).

### Widget Configuration via Environment Variables

Widgets receive all initial configuration via environment variables set by the coordinator:

| Variable | Purpose | Example |
|----------|---------|---------|
| `WAYLAND_DISPLAY` | Wayland compositor socket | `wayland-bmc` |
| `XDG_RUNTIME_DIR` | Runtime directory | `/run` |
| `DECK_INSTANCE_ID` | Unique instance identifier | `clock-abc123` |
| `DECK_SIZE_TYPE` | Widget size category | `small`, `medium`, `large`, `full` |
| `DECK_WIDTH` | Widget width in pixels | `320` |
| `DECK_HEIGHT` | Widget height in pixels | `240` |
| `DECK_PARAMS` | Widget-specific parameters (JSON) | `{"style":"digital"}` |
| `DECK_TIMEZONE` | Current timezone | `Europe/Prague` |
| `DECK_NIGHT_MODE` | Night mode state | `0` or `1` |
| `DECK_LOCALIZATION` | Localization settings (JSON) | `{"dateFormat":"DD.MM.YYYY",...}` |

This approach:
- Simplifies the protocol (no configure/params events needed for initial setup)
- Widget has complete configuration at startup (size, params, settings)
- No PID matching or identification handshake required
- Compositor only needs to handle runtime changes (settings, shutdown)

### Widget Identification

Widget passes its instance ID when binding to the protocol:

1. Coordinator spawns widget with environment variables (size, params, settings, instance_id)
2. Widget reads configuration from environment
3. Widget connects to Wayland, binds `deck_widget_manager_v1`
4. Widget calls `get_widget_surface(surface, instance_id)`
5. Compositor uses instance_id to look up position for rendering

### Protocol Versioning

Wayland protocols use interface versioning for backward compatibility (this follows the standard Wayland versioning scheme):

- Each interface has a `version` attribute (starts at 1)
- New requests/events can only be added at the end of the interface
- New enum entries can only be added at the end of enums
- Requests/events have a `since` attribute indicating minimum version required

**Adding new features:**

1. **New setting type**: Add entry to `setting_type` enum, increment interface version, add `since="2"` to the new entry. Old widgets ignore unknown setting types.

2. **New action type**: Add entry to `action_type` enum, increment interface version. Old compositor ignores unknown action types.

3. **New event** (e.g., `reconfigure`): Add at end of interface with `since="2"`. Compositor only sends to widgets that bound version 2+.

4. **New request** (e.g., `request_focus`): Add at end of interface with `since="2"`. Widget checks bound version before calling.

**Version negotiation**: When widget binds to `deck_widget_manager_v1`, it specifies the maximum version it supports. Compositor uses the minimum of (advertised version, requested version). Both sides only use features available in the negotiated version.

---

### Sub-stage 12.1: Protocol XML Definition

#### Goal
Define the BMC widget protocol in Wayland XML format.

#### File to Create
- `bmc-widget-protocol/protocol/bmc-widget-v1.xml`

#### Protocol Elements

**`deck_widget_manager_v1`** (global):

| Element | Type | Required | Description |
|---------|------|----------|-------------|
| `destroy` | request | Yes | Standard cleanup, allows unbinding from global |
| `get_widget_surface` | request | Yes | Associates protocol with a `wl_surface`, includes `instance_id` for compositor to identify widget |
| `error` enum | enum | Yes | Error codes for invalid operations |

**`deck_widget_surface_v1`** (per-surface):

| Element | Type | Required | Description |
|---------|------|----------|-------------|
| `destroy` | request | Yes | Standard cleanup |
| `request_action` | request | Yes | Core feature - sound/LED control |
| `setting` | event | Yes | Core feature - settings updates (timezone, localization, night mode) |
| `shutdown` | event | Yes | Graceful termination signal |
| `setting_type` enum | enum | Yes | Needed for `setting` event |
| `action_type` enum | enum | Yes | Needed for `request_action` |

Note: Initial configuration (size, params, settings) is passed via environment variables. Widget readiness is detected by first buffer commit (standard Wayland behavior).

#### Protocol Definition

```xml
<?xml version="1.0" encoding="UTF-8"?>
<protocol name="deck_widget_v1">
  <copyright>
    Copyright (C) 2025 Braiins Systems s.r.o.
    SPDX-License-Identifier: MIT
  </copyright>

  <interface name="deck_widget_manager_v1" version="1">
    <description summary="BMC widget manager">
      Global interface for BMC widget management. Widgets bind to this
      interface to register themselves with the compositor.
    </description>

    <request name="destroy" type="destructor">
      <description summary="destroy the manager">
        Destroy the widget manager. Does not affect existing widget surfaces.
      </description>
    </request>

    <request name="get_widget_surface">
      <description summary="create a widget surface">
        Create a deck_widget_surface_v1 for the given wl_surface.
        The instance_id must match the DECK_INSTANCE_ID environment variable
        and is used by the compositor to identify this widget for positioning.
        The surface must not already have a role.
      </description>
      <arg name="id" type="new_id" interface="deck_widget_surface_v1"/>
      <arg name="surface" type="object" interface="wl_surface"/>
      <arg name="instance_id" type="string" summary="widget instance identifier"/>
    </request>

    <enum name="error">
      <entry name="role" value="0" summary="surface already has a role"/>
      <entry name="invalid_instance" value="1" summary="unknown instance_id"/>
    </enum>
  </interface>

  <interface name="deck_widget_surface_v1" version="1">
    <description summary="BMC widget surface">
      Interface for a widget surface. Initial configuration (size, params,
      settings) is received via environment variables. This protocol handles
      runtime communication: settings updates, shutdown, and action requests.
      Widget readiness is detected when the first buffer is committed.
    </description>

    <!-- Requests (Widget → Compositor) -->

    <request name="destroy" type="destructor">
      <description summary="destroy the widget surface">
        Destroy the widget surface role. The wl_surface remains valid.
      </description>
    </request>

    <request name="request_action">
      <description summary="request a system action">
        Request the compositor to perform a system action (sound, LED).
        The payload is JSON-encoded action data.
      </description>
      <arg name="action_type" type="uint" enum="action_type"/>
      <arg name="payload" type="string" summary="JSON payload"/>
    </request>

    <!-- Events (Compositor → Widget) -->

    <event name="setting">
      <description summary="system setting update">
        Sent when a system setting changes at runtime.
      </description>
      <arg name="setting_type" type="uint" enum="setting_type"/>
      <arg name="value" type="string" summary="JSON-encoded value"/>
    </event>

    <event name="shutdown">
      <description summary="request graceful shutdown">
        Sent when the widget should shut down gracefully.
        Widget should clean up and exit.
      </description>
    </event>

    <!-- Enums -->

    <enum name="setting_type">
      <entry name="timezone" value="0" summary="timezone setting"/>
      <entry name="localization" value="1" summary="localization settings"/>
      <entry name="night_mode" value="2" summary="night mode state"/>
    </enum>

    <enum name="action_type">
      <entry name="play_sound" value="0" summary="play a sound"/>
      <entry name="stop_sound" value="1" summary="stop sound"/>
      <entry name="led" value="2" summary="set LED effect"/>
      <entry name="stop_led" value="3" summary="stop LED effect"/>
    </enum>
  </interface>
</protocol>
```

#### Success Criteria
- [x] XML validates with `wayland-scanner --strict`
- [x] All messages documented
- [x] Enums cover all needed values

---

### Sub-stage 12.2: Protocol Crate Setup

#### Goal
Create a Rust crate that generates client and server bindings from the protocol XML.

#### Files to Create

```
bmc-widget-protocol/
  Cargo.toml
  build.rs
  protocol/
    bmc-widget-v1.xml
  src/
    lib.rs
```

#### Cargo.toml Dependencies

- `wayland-scanner` - Build dependency for code generation
- `wayland-client` - Client-side bindings (for widgets)
- `wayland-server` - Server-side bindings (for compositor)
- `wayland-backend` - Shared backend types
- `wayland-protocols` - Core protocol types (for `wl_surface`)

#### Code Generation

The `build.rs` uses `wayland-scanner` procedural macros at build time.
The `lib.rs` exposes two modules:
- `client` - For widgets to use
- `server` - For compositor to use

Both modules use `wayland_scanner::generate_client_code!` and `wayland_scanner::generate_server_code!` macros.

#### Success Criteria
- [x] Crate compiles
- [x] Client bindings generated
- [x] Server bindings generated
- [x] Types are re-exported cleanly

---

### Sub-stage 12.3: Compositor Protocol Implementation

#### Goal
Implement the protocol handlers in the compositor (bmc-openwrt).

#### Files to Modify
- `bmc-openwrt/src/compositor/` - Add protocol handling

#### Implementation Steps

1. **Register the global**: Add `deck_widget_manager_v1` to compositor's global list
2. **Handle bind**: When widget binds, create manager instance
3. **Handle get_widget_surface**: Associate `deck_widget_surface_v1` with `wl_surface`, extract `instance_id`
4. **Match instance_id**: Look up registered widget by instance_id, associate surface with position
5. **Detect readiness**: When widget commits first buffer, mark as ready
6. **Handle request_action**: Forward to action channel
7. **Send setting**: When settings change, broadcast to all widgets
8. **Send shutdown**: When widget should terminate
9. **Handle disconnect**: Detect client disconnect, notify coordinator

#### Protocol State

The compositor needs to track:
- Pending widget registrations (by instance_id, from coordinator)
- Active widget surfaces (by instance_id, after widget connects)

#### Success Criteria
- [ ] Global advertised to clients
- [ ] Widget surfaces created correctly
- [ ] Instance ID matching works
- [ ] Actions forwarded to channel
- [ ] Settings broadcast works

#### Status: Not Started

---

### Sub-stage 12.4: Widget Client Library Update

#### Goal
Update `bmc-widget` crate to provide a Wayland-based client API.

#### Files to Modify
- `bmc-widget/Cargo.toml` - Add `bmc-widget-protocol` dependency
- `bmc-widget/src/lib.rs` - Export new client module
- `bmc-widget/src/wayland_client.rs` (new) - Wayland client implementation
- `bmc-widget/src/env.rs` (new) - Environment variable helpers

#### New API

The `bmc-widget` crate provides helpers for widgets:

**Environment variable helpers:**

| Function | Description |
|----------|-------------|
| `read_instance_id()` | Read `DECK_INSTANCE_ID` from environment |
| `read_size()` | Read `DECK_SIZE_TYPE`, `DECK_WIDTH`, `DECK_HEIGHT` from environment |
| `read_params<T>()` | Read and parse `DECK_PARAMS` as JSON |
| `read_settings()` | Read `DECK_TIMEZONE`, `DECK_NIGHT_MODE`, `DECK_LOCALIZATION` from environment |

**Wayland protocol helpers:**

| Function/Method | Description |
|-----------------|-------------|
| `bind_widget_manager()` | Bind to `deck_widget_manager_v1` global |
| `get_widget_surface()` | Create widget surface with instance_id |
| `request_action()` | Request sound/LED action |

Widgets handle events (`setting`, `shutdown`) via standard Wayland event dispatch. Widget readiness is implicit when the first buffer is committed.

**Note on Slint widgets**: Slint manages its own Wayland connection internally and doesn't expose it. Slint widgets will need to create a separate Wayland connection to bind to `deck_widget_manager_v1` and handle our protocol events (`setting`, `shutdown`). This means running two event loops or integrating the protocol connection into Slint's event loop via a timer or file descriptor watch.

#### Status: Not Started

#### Success Criteria
- [ ] Environment variable helpers work (size, params, settings)
- [ ] Widget can bind to `deck_widget_manager_v1`
- [ ] Widget surface created with instance_id
- [ ] Actions can be requested
- [ ] Setting events received
- [ ] Shutdown event handled

---

### Sub-stage 12.5: Widget Integration (digital-clock)

#### Goal
Migrate the digital-clock widget to use environment variables and Wayland protocol.

#### Files to Modify
- `widgets/digital-clock/Cargo.toml` - Update dependencies
- `widgets/digital-clock/src/main.rs` - Use new client API

#### Migration Steps

1. Remove `BMC_IPC_SOCKET` environment variable usage and JSON IPC client
2. Read size, params, and initial settings from environment variables
3. Initialize Slint UI at correct size with initial settings
4. Create separate Wayland connection for BMC protocol
5. Bind to `deck_widget_manager_v1`, call `get_widget_surface(surface, instance_id)`
6. Commit first buffer (compositor detects widget is ready)
7. Handle `setting` events for runtime timezone/localization/night_mode changes
8. Handle `shutdown` event for graceful termination

#### Testing

1. Spawn digital-clock with correct environment variables
2. Verify widget reads size from environment and renders correctly
3. Verify widget connects to compositor and calls `set_ready()`
4. Change timezone, verify setting event received and UI updates
5. Verify shutdown works

#### Status: Not Started

#### Success Criteria
- [ ] digital-clock reads config from environment variables
- [ ] digital-clock uses Wayland protocol for runtime events
- [ ] No JSON IPC socket used
- [ ] All settings updates work
- [ ] Graceful shutdown works
- [ ] Widget renders correctly at all sizes

---

### Overall Success Criteria

- [ ] Protocol XML validates with wayland-scanner
- [ ] Compositor implements protocol handlers
- [ ] Widget client library works (env helpers + protocol)
- [ ] digital-clock uses environment variables + Wayland protocol
- [ ] JSON IPC removed

### Dependencies

- Stage 11 (Compositor Trait Abstraction)

### Status: Not Started

---

## Stage 13: EGL Compositor Implementation in bmc-openwrt

### Goal

Implement `Compositor` trait for ARMv7 using the existing `bmc-compositor` crate code. The compositor runs in a dedicated thread within the `bmc-openwrt` process.

### Background

The `bmc-compositor` crate (Stage 8) contains a working Wayland compositor POC. This stage integrates it into `bmc-openwrt` as a library, implementing the `Compositor` trait from Stage 11 and the Wayland protocol extension from Stage 12.

### Scope

- Create `EglCompositor` struct in `bmc-openwrt` that implements `Compositor` trait
- Run compositor event loop in a dedicated thread (uses calloop, not tokio)
- Communicate between async main thread and compositor thread via channels
- Implement `deck_widget_v1` protocol handlers from Stage 12
- Reuse rendering code from `bmc-compositor` crate
- Only EGL renderer for now (no software fallback needed)

### Architecture

The main application runs on tokio async runtime. The compositor runs in a separate thread with its own calloop event loop. Communication happens via channels:

- **Main → Compositor**: Commands (register widget, set scene, broadcast setting, shutdown)
- **Compositor → Main**: Events (widget ready, widget disconnected, actions)

### Files to Create/Modify

**New files:**
- `bmc-openwrt/src/compositor/mod.rs` - Module exports
- `bmc-openwrt/src/compositor/egl_compositor.rs` - `EglCompositor` implementation
- `bmc-openwrt/src/compositor/commands.rs` - Command/event types for thread communication
- `bmc-openwrt/src/compositor/protocol.rs` - `deck_widget_v1` protocol handlers

**Modified files:**
- `bmc-openwrt/src/lib.rs` - Export compositor module
- `bmc-openwrt/src/main.rs` - Initialize compositor and pass to App
- `bmc-openwrt/Cargo.toml` - Add dependencies on `bmc-compositor` and `bmc-widget-protocol`

### Widget Spawn and Registration Flow

1. Coordinator determines widget size from scene configuration (small/medium/large/full)
2. Coordinator calculates pixel dimensions from grid position
3. Coordinator calls `compositor.register_widget(instance_id, position)` to register expected widget
4. Coordinator spawns widget with environment variables:
   - `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR` - Wayland connection
   - `DECK_INSTANCE_ID` - Widget instance identifier
   - `DECK_SIZE_TYPE` - Size category (small/medium/large/full)
   - `DECK_WIDTH`, `DECK_HEIGHT` - Pixel dimensions
   - `DECK_PARAMS` - JSON widget parameters
   - `DECK_TIMEZONE`, `DECK_NIGHT_MODE`, `DECK_LOCALIZATION` - Initial settings
5. Widget reads configuration from environment, initializes UI with correct size and settings
6. Widget connects to Wayland, binds `deck_widget_manager_v1`
7. Widget calls `get_widget_surface(surface, instance_id)`
8. Compositor matches `instance_id` to registered widget, associates surface with position
9. Widget commits first buffer (compositor detects widget is ready)
10. Compositor starts sending frame callbacks, widget is now visible

### Widget Disconnect Detection

The compositor detects widget disconnects via Smithay's client lifecycle callbacks:

1. **Client disconnect**: When a Wayland client disconnects (exit, crash, or connection close), Smithay invokes the `ClientData::disconnected()` callback
2. **Surface cleanup**: The compositor's `CompositorHandler::destroyed()` is called for each surface owned by the client
3. **Instance lookup**: Compositor looks up instance_id from the destroyed surface
4. **Event dispatch**: Compositor sends `WidgetDisconnected { instance_id, reason }` event to coordinator via channel
5. **Logging**: Coordinator logs the disconnect with widget details (instance_id, widget UID, exit reason if available)

### Error Handling

Widget crashes must be detected and handled:

- Compositor detects client disconnect (Wayland client gone)
- Compositor sends `WidgetDisconnected` event to main app via channel
- Coordinator receives event and logs the crash
- Coordinator may attempt to respawn the widget (configurable retry policy)
- After N failed attempts, coordinator marks widget as failed and logs error

### Third-Party Client Support (xdg_toplevel)

The compositor must support both our custom `deck_widget_surface_v1` protocol and standard `xdg_toplevel` for third-party Wayland clients (e.g., Slint applications using winit backend).

#### Why Both Protocols

- **deck_widget_surface_v1**: Our widgets use this protocol. Widget sends `instance_id` explicitly via `get_widget_surface()`.
- **xdg_toplevel**: Standard desktop window protocol. Third-party apps (including Slint with winit) use this by default. No `instance_id` mechanism exists.

A `wl_surface` can only have **one role** - either `deck_widget_surface_v1` or `xdg_toplevel`, never both.

#### Registration Flow for Third-Party Clients

Since `xdg_toplevel` clients cannot send an `instance_id`, the compositor identifies them by PID:

1. Coordinator spawns third-party app, knows the PID
2. Coordinator calls `compositor.register_widget(instance_id, position, pid)` with the process PID
3. Third-party app connects to Wayland, creates `xdg_toplevel`
4. Compositor extracts client PID from Wayland connection (`Client::credentials()`)
5. Compositor matches PID to registered widget, associates surface with position
6. Compositor sends `configure` event telling client its size
7. Client renders and commits buffer

#### xdg_toplevel Protocol Requirements

For `xdg_toplevel` clients, the compositor must implement the configure sequence:

1. Send `xdg_toplevel.configure(width, height, states)` - tells client what size to use
2. Send `xdg_surface.configure(serial)` - marks end of configure sequence
3. Client renders at that size
4. Client sends `ack_configure(serial)` - acknowledges the configure

Without this sequence, third-party clients won't draw - they wait for the compositor to tell them their size.

#### What xdg_toplevel Features to Implement

| Feature | Required | Notes |
|---------|----------|-------|
| `configure` sequence | Yes | Client won't render without it |
| `set_title` / `set_app_id` | Receive only | Useful for logging, can ignore |
| `move` / `resize` requests | No | Compositor controls positioning |
| `set_min_size` / `set_max_size` | No | Can ignore |
| `set_fullscreen` / `set_maximized` | No | Can ignore |

#### Compositor State for Both Protocols

```rust
pub struct CompositorState {
    // Widget positions (both protocols use this)
    widget_positions: HashMap<InstanceId, Position>,

    // For deck_widget_surface_v1 clients (instance_id from protocol)
    deck_widgets: HashMap<InstanceId, WlSurface>,

    // For xdg_toplevel clients (instance_id from PID lookup)
    pid_to_instance: HashMap<u32, InstanceId>,
    xdg_surfaces: HashMap<InstanceId, XdgToplevel>,
}
```

#### Rendering

Both surface types feed into the same rendering pipeline. The compositor:
1. Iterates all surfaces with committed content
2. Looks up position by `instance_id` (from protocol for deck widgets, from PID for xdg)
3. Renders at the assigned position

### Success Criteria

- [ ] `EglCompositor` implements `Compositor` trait
- [ ] Compositor runs in dedicated thread with calloop
- [ ] `deck_widget_v1` protocol handlers work
- [ ] `xdg_shell` protocol handlers work (for third-party clients)
- [ ] Third-party clients identified by PID matching
- [ ] xdg_toplevel configure sequence implemented
- [ ] Commands flow from main thread to compositor (register, set scene, broadcast setting, shutdown)
- [ ] Events flow from compositor to main thread (widget ready, widget disconnected)
- [ ] Actions flow from compositor to main thread (sound, LED requests)
- [ ] Instance ID based widget identification works
- [ ] Widget surfaces display correctly (both deck_widget and xdg_toplevel)
- [ ] Widget disconnects detected and reported
- [ ] Graceful shutdown works

### Dependencies

- Stage 8 (Wayland Compositor Core) - reuses `bmc-compositor` code
- Stage 11 (Compositor Trait) - implements the trait
- Stage 12 (Wayland Protocol Extension) - implements protocol handlers

### Status: Not Started

---

## Stage 14: Coordinator-Compositor Integration

### Goal

Connect the `WidgetCoordinator` to the compositor so spawned widgets can render to the display.

### Scope

- Add compositor reference to `Coordinator`
- Pass all configuration via environment variables to spawned widgets
- Register widgets with compositor before spawning (position only)
- Unregister widgets when stopped
- Handle widget crash events from compositor

### Files to Modify

- `bmc/src/widget/coordinator.rs` - Add compositor, update spawn flow
- `bmc/src/widget/manager.rs` - Build environment variables for spawning
- `bmc/src/widget/spawner.rs` - Pass environment variables to child process
- `bmc/src/startup.rs` - Wire compositor to coordinator
- `bmc/src/entry.rs` - Accept compositor parameter

### Widget Spawn Flow

1. Coordinator calculates widget position and size from grid layout
2. Coordinator calls `compositor.register_widget(instance_id, position)` - registers expected widget
3. Coordinator spawns widget process with environment variables:
   - `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR` - Wayland connection
   - `DECK_INSTANCE_ID` - Widget instance identifier
   - `DECK_SIZE_TYPE` - Size category
   - `DECK_WIDTH`, `DECK_HEIGHT` - Pixel dimensions
   - `DECK_PARAMS` - JSON widget parameters
   - `DECK_TIMEZONE`, `DECK_NIGHT_MODE`, `DECK_LOCALIZATION` - Initial settings
4. Widget reads configuration from environment, initializes UI with correct size and settings
5. Widget connects to Wayland, calls `get_widget_surface(surface, instance_id)`
6. Compositor matches instance_id to registered widget
7. Widget commits first buffer (compositor detects widget is ready)
8. Compositor starts rendering widget at registered position

### Widget Crash Handling

1. Compositor detects widget disconnect (Wayland client gone)
2. Compositor sends `WidgetDisconnected { instance_id, reason }` event
3. Coordinator receives event via channel
4. Coordinator logs the crash with widget details
5. Coordinator checks retry policy (e.g., max 3 retries with backoff)
6. If retries remaining: respawn widget with same environment variables
7. If retries exhausted: mark widget as failed, notify user via logs/UI

### Success Criteria

- [ ] Coordinator holds compositor reference
- [ ] Widgets spawned with all config via environment variables
- [ ] Widget registration includes position
- [ ] Widget receives correct size via `DECK_WIDTH`/`DECK_HEIGHT` env vars
- [ ] Widget surfaces appear on display at correct position
- [ ] Widget crashes detected and handled with retry
- [ ] Widget unregistration on process stop

### Dependencies

- Stage 11 (Compositor Trait)
- Stage 12 (Wayland Protocol Extension)
- Stage 13 (EGL Compositor Implementation)

### Status: Not Started

---

## Stage 15: Scene Management

### Goal

Implement scene switching while keeping all widgets running. The compositor controls visibility via frame callbacks and widget positioning.

### Scope

- Scene switching updates compositor's active layout via `set_active_scene()`
- Hidden widgets stop receiving frame callbacks (standard Wayland pattern)
- Visible widgets are positioned according to scene layout
- All widget processes remain running across scene switches

### Scene Layout System

The display is a 4×2 grid (1280×480 pixels):

| Column 0 | Column 1 | Column 2 | Column 3 |
|----------|----------|----------|----------|
| (0,0) | (1,0) | (2,0) | (3,0) |
| (0,1) | (1,1) | (2,1) | (3,1) |

Each cell is 320×240 pixels.

Widget sizes map to grid cells:

| Size | Grid Cells | Pixels |
|------|------------|--------|
| Small | 1×1 | 320×240 |
| Medium | 2×1 | 640×240 |
| Large | 2×2 | 640×480 |
| Full | 4×2 | 1280×480 |

### Scene Switching Flow

1. User triggers scene switch (gesture/button)
2. Coordinator builds `SceneLayout` from scene configuration
3. Coordinator calls `compositor.set_active_scene(layout)`
4. Compositor updates internal state (marks widgets visible/hidden, updates positions)
5. On next frame: render visible widgets, send frame callbacks only to visible widgets
6. Hidden widgets naturally pause (waiting for callback that won't come)

### Visibility Signaling

The standard Wayland mechanism for visibility is **frame callbacks**:

1. **Widget becomes hidden**: Compositor stops sending frame callbacks. Widget naturally pauses (blocks waiting for callback that won't come).

2. **Widget becomes visible**: Compositor resumes sending frame callbacks. Widget receives callback and resumes rendering.

This is the standard Wayland pattern - compositors should avoid signaling frame callbacks if the surface is not visible. The `wl_surface.enter`/`wl_surface.leave` events are for output tracking, not visibility.

### Success Criteria

- [ ] `Coordinator.set_active_scene()` sends layout to compositor
- [ ] Compositor positions widgets according to layout
- [ ] Only visible widgets receive frame callbacks
- [ ] Hidden widgets pause rendering (power savings)
- [ ] Scene transitions are smooth
- [ ] All widget processes remain running across scene switches

### Dependencies

- Stage 13 (EGL Compositor Implementation)
- Stage 14 (Coordinator-Compositor Integration)

### Status: Not Started

---

## Stage 16: Settings Broadcasting to Widgets

### Goal

Propagate system settings changes (timezone, localization, night mode) to all running widgets via the Wayland protocol extension.

### Scope

- When settings change, coordinator sends update to compositor
- Compositor broadcasts `setting` event to all widgets via `deck_widget_surface_v1`
- Widgets update their state without restart

### Settings Types

| Setting | When Changed | Example |
|---------|--------------|---------|
| Timezone | User changes timezone | `Europe/Prague` → `America/New_York` |
| Localization | User changes date/time format | 24h → 12h format |
| Night Mode | Automatic or manual toggle | Brightness/color adjustment |

### Integration Points

Settings changes originate from:
- `SystemManager` - timezone updates
- `LocalizationConfig` - format preferences
- `NightModeController` - night mode state

Each needs to call `coordinator.broadcast_settings_update()` when settings change.

### Files to Modify

- `bmc/src/startup.rs` - Wire settings change handlers to coordinator
- `bmc/src/widget/coordinator.rs` - Add broadcast method

### Success Criteria

- [ ] Timezone changes propagate to widgets
- [ ] Localization changes propagate to widgets
- [ ] Night mode changes propagate to widgets
- [ ] Widgets update display without restart

### Dependencies

- Stage 12 (Wayland Protocol Extension)

### Status: Not Started

---

## Stage 17: Action Routing (Sound/LED)

### Goal

Route widget action requests to hardware controllers.

### Scope

- Spawn action handler task in background
- Receive actions from compositor's action channel (via Wayland protocol)
- Route to appropriate controller
- Handle errors gracefully (log but don't crash)

### Action Flow

1. Widget sends `action` request via `deck_widget_surface_v1` Wayland protocol
2. Compositor forwards action to main app via action channel
3. ActionHandler receives action from channel
4. ActionHandler routes to appropriate controller based on action type

### Action Types

| Action | Controller Method |
|--------|-------------------|
| `PlaySound` | `SoundController::play()` |
| `StopSound` | `SoundController::stop()` |
| `Led` | `LedController::set_effect()` |
| `StopLed` | `LedController::stop()` |

### Files to Create/Modify

- `bmc/src/widget/action_handler.rs` (new) - Action routing logic
- `bmc/src/startup.rs` - Spawn action handler task with controller references

### Success Criteria

- [ ] ActionHandler spawned as background task
- [ ] Sound actions trigger SoundController
- [ ] LED actions trigger LedController
- [ ] Errors logged but don't crash handler

### Dependencies

- Stage 12 (Wayland Protocol Extension)

### Status: Not Started

---

## Stage 18: Configuration Migration

### Goal
Migrate existing widget configurations to use widget UIDs.

### Scope
- Detect old configuration format
- Map old widget types to new widget UIDs
- Preserve all widget instance settings
- Create backup before migration

### Migration Map

| Old Widget Type | New Widget UID | Notes |
|-----------------|----------------|-------|
| `Clock` | `550e8400-...` | Direct mapping |
| `TickerBtc` | (TBD) | Future extraction |
| `BlockHeight` | (TBD) | Future extraction |
| `BraiinsPool` | (TBD) | Future extraction |
| `RemoteImage` | (TBD) | Future extraction |
| `BlockchainData` | (TBD) | Future extraction |
| `RemoteWidget` | Keep as-is | Special handling |

### Migration Process

1. Check config version or format
2. If old format detected:
   - Create backup: `/etc/bmc/config.json.backup.<timestamp>`
   - Log migration start
   - For each scene:
     - For each widget:
       - Map widget type to UID
       - Preserve params and position
   - Update config version
   - Save migrated config
   - Log migration complete

### Handling Missing Widgets

If widget UID not found in registry:
- Keep widget in config
- Mark as `unavailable: true`
- Log warning
- Show placeholder in UI (future stage)

### Test Cases

1. **Migration**
   - Old config migrates correctly
   - All widget params preserved
   - Backup created

2. **Already Migrated**
   - New format config unchanged
   - No duplicate migration

3. **Missing Widgets**
   - Handled gracefully
   - Marked unavailable

### Success Criteria

- [ ] Migration runs on startup if needed
- [ ] Backup created before migration
- [ ] All existing configs migrate successfully
- [ ] Missing widgets handled gracefully


### Status: Not Started

---

## Stage 19: Scene Transition Animation

### Goal

Implement smooth swipe-based scene transitions with animated visual feedback.

### Background

Scene switching should feel fluid and responsive. When the user swipes, both the current and next scene should be visible during the transition, sliding in the direction of the swipe.

### Approach: Compositor-Side Scene Rendering

The cleanest approach is to handle transitions entirely in the compositor. Widgets are unaware of transitions - they just render normally. The compositor:

1. Renders each scene's widgets to an offscreen framebuffer
2. During swipe: composites both framebuffers with offset based on finger position
3. After swipe completes: updates active scene, stops rendering old scene

This keeps widget code simple and allows smooth 60fps animations.

### Transition Types

| Type | Description | Use Case |
|------|-------------|----------|
| **Swipe** | Scenes slide horizontally following finger | User-initiated navigation |
| **Fade** | Cross-fade between scenes | Programmatic scene change, timeout |

### Compositor State for Transitions

The compositor needs to track:

| Field | Type | Description |
|-------|------|-------------|
| `transition_state` | enum | `None`, `Swiping`, `Settling`, or `Fading` |
| `transition_type` | enum | `Swipe` or `Fade` |
| `from_scene` | SceneId | Scene being transitioned away from |
| `to_scene` | SceneId | Scene being transitioned to |
| `progress` | f32 | 0.0 = from_scene fully visible, 1.0 = to_scene fully visible |
| `direction` | enum | `Left` or `Right` (for swipe only) |
| `velocity` | f32 | For momentum-based settling after touch up |

### Rendering Pipeline

**Normal (no transition):**
1. Render visible widgets directly to display framebuffer

**During swipe:**
1. Render `from_scene` widgets to offscreen buffer A
2. Render `to_scene` widgets to offscreen buffer B
3. Composite both buffers to display:
   - Buffer A offset by `progress * screen_width` in swipe direction
   - Buffer B follows behind

**During fade:**
1. Render `from_scene` widgets to offscreen buffer A
2. Render `to_scene` widgets to offscreen buffer B
3. Composite both buffers to display with alpha blending:
   - Buffer A with alpha = `1.0 - progress`
   - Buffer B with alpha = `progress`

**After transition:**
1. Update `active_scene` to the final scene
2. Return to normal rendering
3. Hidden scene's widgets stop receiving frame callbacks

### Touch Input Integration

Touch events flow through libinput → compositor:

1. **Touch down**: Record start position, prepare transition if near edge or gesture detected
2. **Touch move**: Update `progress` based on finger delta, re-render
3. **Touch up**: Calculate velocity, start settling animation or snap back

### Settling Animation

When finger lifts:
- If `progress > 0.5` or velocity is high enough → animate to completion
- If `progress < 0.5` and low velocity → animate back to start (cancel)

Use easing function (e.g., ease-out) for smooth deceleration.

### Files to Modify

- `bmc-openwrt/src/compositor/state.rs` - Add `TransitionState`, scene framebuffers
- `bmc-openwrt/src/compositor/render_egl.rs` - Offscreen rendering, compositing
- `bmc-openwrt/src/compositor/input.rs` (new) - Touch event handling
- `bmc-openwrt/src/compositor/egl_compositor.rs` - Integrate touch events into event loop

### Framebuffer Management

Options for offscreen buffers:
1. **Two persistent buffers** - Always allocated, swap roles as needed
2. **On-demand allocation** - Allocate when transition starts, free when done
3. **Texture caching** - Keep recently-used scene renders cached

Option 1 is simplest for memory-constrained embedded device with known scene count.

### Success Criteria

- [ ] Touch input events received from libinput
- [ ] Swipe gesture detection works
- [ ] Scenes render to offscreen framebuffers
- [ ] Both scenes visible during transition
- [ ] Smooth 60fps animation during swipe
- [ ] Momentum-based settling animation
- [ ] Snap-back on cancelled swipe
- [ ] Frame callbacks correctly managed (only active scene receives them after transition)

### Dependencies

- Stage 13 (EGL Compositor Implementation)
- Stage 15 (Scene Management)

### Status: Not Started

---

## Appendix: Current Code References

### Key Files in Current Architecture

| File | Purpose |
|------|---------|
| `bmc-display/src/data.rs` | Widget/Scene data structures, `WidgetKind` enum |
| `bmc-display/src/display_controller.rs` | Central UI state manager |
| `bmc-display/src/display_controller/state.rs` | Scene/widget state methods |
| `bmc/src/widget_tasks.rs` | Widget task spawning/lifecycle |
| `bmc/src/widget_tasks/clock.rs` | Clock widget async task |
| `bmc/src/config.rs` | Configuration persistence |
| `bmc/src/web/grpc/scene_management.rs` | Scene management gRPC API |
| `bmc-display/ui/widgets/clock.slint` | Clock widget Slint UI |

### Key Files in New Architecture

| File | Purpose |
|------|---------|
| `bmc-compositor/src/state.rs` | Wayland compositor state and protocol handlers (POC) |
| `bmc-compositor/src/render_egl.rs` | EGL render pipeline for ARMv7 (POC) |
| `bmc/src/compositor.rs` | Compositor trait definition (Stage 11) |
| `bmc/src/widget/coordinator.rs` | Widget lifecycle orchestration |
| `bmc/src/widget/manager.rs` | Widget process spawning |
| `bmc/src/widget/action_handler.rs` | Widget action routing (Stage 17) |
| `bmc-widget-protocol/` | Wayland protocol extension (Stage 12) |
| `bmc-openwrt/src/compositor/` | EglCompositor implementation (Stage 13) |
| `widgets/digital-clock/` | Standalone digital clock widget |
