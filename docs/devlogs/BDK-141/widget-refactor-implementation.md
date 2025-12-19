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

## Stage 8: Digital Clock Widget Extraction

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

## Stage 9: Add Widget System (Alongside Existing)

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

## Stage 10: Wayland Compositor Integration

### Goal
Integrate a Wayland compositor into the `bmc` application that directly composites widget processes, replacing the monolithic Slint display.

### Background

The `jan/coordinator-refactor` branch contains a working PoC demonstrating the compositor client approach. This stage evolves that into a full compositor embedded in `bmc`.

### Architecture

BMC **is** the Wayland compositor. It owns the display directly via DRM/KMS and composites widget surfaces.

```
+-------------------------------------------------------------+
|                      BMC Application                        |
|  +-------------------------------------------------------+  |
|  |              Wayland Compositor (smithay)             |  |
|  |   - DRM/KMS backend for direct display output         |  |
|  |   - Composites widget surfaces onto framebuffer       |  |
|  |   - Handles input events (touch, pointer)             |  |
|  |   - Implements wl_compositor, wl_shm, xdg_shell       |  |
|  +-------------------------------------------------------+  |
|         |                |                    |             |
|  +-------------+  +-------------+  +---------------------+  |
|  | SceneManager|  |WidgetManager|  | TransitionController|  |
|  +-------------+  +-------------+  +---------------------+  |
+-------------------------------------------------------------+
          | Wayland protocol (wl_shm, wl_surface, xdg_shell)
          v
+----------+  +----------+  +----------+  +----------+
|  Clock   |  |  Ticker  |  |  Pool    |  |  Image   |
|  Widget  |  |  Widget  |  |  Widget  |  |  Widget  |
| (client) |  | (client) |  | (client) |  | (client) |
+----------+  +----------+  +----------+  +----------+
```

### Key Design Points

- **BMC is the compositor**: No Weston, no intermediate layer
- **Widgets are Wayland clients**: They connect to BMC's Wayland socket
- **Direct framebuffer access**: DRM/KMS for display, no GPU compositing needed
- **Software rendering**: Widgets use Slint SoftwareRenderer, BMC composites in software
- **Frame callback optimization**: Only send frame callbacks to visible surfaces. When a widget is not displayed (e.g., on a different scene), the compositor should stop sending frame callbacks. This causes the Wayland client (Slint widget) to automatically stop rendering, saving CPU. The Wayland protocol is designed this way - clients wait for frame callbacks before rendering, so withholding callbacks naturally throttles hidden widgets to 0 FPS.

### Scope

This stage is divided into sub-stages for incremental progress:

---

### Stage 10.1: Minimal Compositor with Widget Display

**Goal**: BMC acts as a Wayland compositor, spawns one widget, and displays it.

**Scope**:
- Add `smithay` compositor library to `bmc`
- Initialize DRM/KMS backend for direct framebuffer access
- Create Wayland socket for widget connections
- Spawn digital-clock widget as Wayland client
- Composite widget surface to display

**Dependencies**:
- `smithay` for compositor infrastructure
- `calloop` for event loop integration

**Success Criteria**:
- [ ] BMC initializes DRM/KMS display
- [ ] Wayland socket created for clients
- [ ] Widget spawned with `WAYLAND_DISPLAY` pointing to BMC
- [ ] Widget surface composited to display (Slint handles Wayland client automatically)
- [ ] Works on ARMv7 device

### Status: In Progress

### Implementation Notes (Stage 10.1)

This section documents the current state of Stage 10.1 implementation for future continuation.

#### What Has Been Done

1. **Compositor Crate Structure Created** (`bmc-compositor/`)
   - `Cargo.toml` - Dependencies configured (smithay 0.7.0, calloop, drm, gbm, libseat, udev, etc.)
   - `src/lib.rs` - Exports `drm_backend` and `state` modules
   - `src/main.rs` - Entry point with logging initialization and DRM backend setup skeleton
   - `src/drm_backend.rs` - DRM/KMS backend skeleton with LibSeat session initialization
   - `src/state.rs` - Wayland compositor state with protocol handlers

2. **Fixed `CompositorHandler::client_compositor_state`**
   - Changed parameter from `ClientId` to `Client` (smithay 0.7.0 API)
   - Implemented proper `ClientState` struct with `CompositorClientState`
   - Added `client.get_data::<ClientState>()` pattern for per-client state tracking

3. **Fixed `smithay-drm-extras` Dependency Conflict**
   - System has `libdisplay-info-0.3.0` but crate expected `< 0.3.0`
   - Fixed by setting `default-features = false` in `Cargo.toml` to disable `display_info` feature
   - Change in root `Cargo.toml`: `smithay-drm-extras = { version = "0.1.0", default-features = false }`

4. **Fixed All Compilation Errors** (Compositor now builds cleanly)
   - Added `BufferHandler` implementation for `Compositor`
   - Imported `Resource` trait for `surface.id()` method
   - Added `SelectionHandler` implementation (required by `DataDeviceHandler`)
   - Fixed `OFlags` import (use `smithay::reexports::rustix::fs::OFlags`)
   - Fixed `DrmDeviceFd::new()` call (needs `.into()` for `DeviceFd` conversion)
   - Fixed `Serial` type import (use `smithay::utils::Serial`)
   - Added `udev` crate dependency for GPU discovery

5. **Code Passes Clippy and Formatting**
   - All clippy warnings resolved
   - Code formatted with `nix fmt`

#### Current State

The compositor crate **compiles successfully** with `nix develop --command cargo clippy -p bmc-compositor -- -D warnings`.

The following protocol handlers are implemented in `state.rs`:
- `CompositorHandler` - Surface creation and commits
- `ShmHandler` - Shared memory buffers
- `BufferHandler` - Buffer lifecycle
- `SeatHandler` - Input handling (keyboard, pointer, touch)
- `XdgShellHandler` - Window management
- `SelectionHandler` - Copy/paste support
- `DataDeviceHandler` - Drag and drop

#### Next Steps to Complete Stage 10.1

1. **Complete DRM device initialization in `drm_backend.rs`:**
   - Find and configure connectors
   - Set up CRTC (display controller)
   - Configure display mode (1280x480 for the Braiins Deck display)
   - Initialize framebuffer

2. **Switch to software renderer:**
   - Replace `GlowRenderer` with `PixmanRenderer` for CPU-based rendering
   - ARMv7 target doesn't have GPU, so software rendering is required
   - May need to add `renderer_pixman` feature to smithay in `Cargo.toml`

3. **Create Wayland display socket:**
   - Initialize `wayland_server::Display`
   - Create socket for widget clients to connect
   - Set up client connection handling with `ClientState::default()`

4. **Implement basic render loop:**
   - Handle surface commits
   - Composite surfaces to framebuffer
   - Use vsync for smooth rendering

5. **Spawn widget as Wayland client:**
   - Set `WAYLAND_DISPLAY` environment variable when spawning widget
   - Verify widget connects and renders

#### Key Files Reference

| File | Purpose |
|------|---------|
| `bmc-compositor/src/state.rs` | Compositor state and Wayland protocol handlers |
| `bmc-compositor/src/drm_backend.rs` | DRM/KMS display backend |
| `bmc-compositor/src/main.rs` | Entry point |
| `Cargo.toml` (root) | Smithay dependency configuration |

#### Smithay Documentation References

- [CompositorHandler trait](https://smithay.github.io/smithay/smithay/wayland/compositor/trait.CompositorHandler.html)
- [Anvil reference compositor](https://github.com/Smithay/smithay/tree/master/anvil) - Use as implementation reference
- [smithay-drm-extras](https://crates.io/crates/smithay-drm-extras) - DRM utilities

#### Build Commands

```bash
# Native x86 development (requires nix develop for system dependencies)
nix develop --command cargo check -p bmc-compositor
nix develop --command cargo clippy -p bmc-compositor -- -D warnings

# ARMv7 cross-compilation (glibc, NOT musl - compositor requires dynamic linking)
nix develop .#armv7-glibc-release --command cargo check -p bmc-compositor
```

#### ARMv7 Cross-Compilation Notes

- **Must use glibc profile**, not musl - compositor dependencies (libinput, libseat, etc.) don't support static linking
- Changed `libseat` → `seatd` in `workspace.nix` `targetDeps` for cross-compilation compatibility
- First build of cross-compilation environment takes significant time (building ARM toolchain)
- The compositor will be dynamically linked, similar to widgets

#### EGL/GPU Rendering Attempts and Findings

The STM32MP157 SoC has a split GPU/display architecture:
- **GPU**: Vivante GC400 (etnaviv driver) - `/dev/dri/card0` and `/dev/dri/renderD128`
- **Display**: STM32 LTDC display controller - `/dev/dri/card1`
- **Panel**: MIPI-DSI, Sitronix ST7703, 600x1280 @ 63Hz (only 480x1280 visible)

**What Works**:
1. EGL initialization on GPU (etnaviv) via GBM - ✓
2. OpenGL ES 2.0 context creation - ✓
3. GLES renderer creation (Vivante GC400) - ✓
4. GBM buffer allocation on GPU with empty flags - ✓
5. DMA-BUF export from GPU buffers - ✓

**What Fails (with detailed logs)**:

1. **GBM device creation on display (stm32-ltdc)** - ❌
   ```
   Failed to create display GBM device: Device is not a GBM compatible device
   ```
   - stm32-ltdc is a simple display controller without GBM support
   - GBM requires GPU-like buffer management which LTDC doesn't have

2. **PRIME import from GPU to display** - ❌
   ```
   GBM buffer object allocated on GPU
   DMA-BUF exported: 1 planes, format DrmFormat { code: DrmFourcc(XR24), modifier: Linear }
   Importing DMA-BUF: fd=19, stride=1920, format=DrmFourcc(XR24), size=480x1280
   ERROR: Failed to import DMA-BUF as GEM handle (PRIME import)
   ```
   - stm32-ltdc's DRM driver doesn't implement `drm_gem_prime_import`
   - The LTDC is a display-only device with no buffer management capability
   - It can only scanout buffers allocated on itself (dumb buffers)

3. **GBM buffer allocation with SCANOUT flag on GPU** - ❌
   ```
   GBM allocation failed: Os { code: 22, kind: InvalidInput, message: "Invalid argument" }, 
   size=480x1280, format=Xrgb8888
   ```
   - `/dev/dri/renderD128` is a render-only node
   - Render nodes don't support SCANOUT flag (that's for display)
   - etnaviv render node only supports rendering operations

4. **GBM buffer allocation with RENDERING flag on GPU** - ❌
   ```
   GBM allocation failed: Os { code: 22, kind: InvalidInput, message: "Invalid argument" }, 
   size=480x1280, format=Argb8888
   ```
   - Same EINVAL error with RENDERING flag
   - The flag itself may not be the issue

5. **Smithay GbmAllocator with various flags** - ❌
   ```
   GBM allocation failed: Os { code: 22, kind: InvalidInput, message: "Invalid argument" }
   ```
   - Tried: `GbmBufferFlags::SCANOUT`, `GbmBufferFlags::RENDERING`, `GbmBufferFlags::empty()`
   - Smithay's allocator may add internal flags or modifiers

6. **Raw gbm_bo_create with empty flags** - ✓ (partial success)
   ```
   GBM buffer object allocated on GPU
   DMA-BUF exported: 1 planes, format DrmFormat { code: DrmFourcc(XR24), modifier: Linear }
   ```
   - Buffer allocation works!
   - DMA-BUF export works!
   - But then PRIME import to display fails (see #2 above)

**Approaches Tried**:

| Approach | Allocation | Export | Import | Scanout | Notes |
|----------|------------|--------|--------|---------|-------|
| GPU GBM → PRIME import to display | ✓ | ✓ | ❌ | - | stm32-ltdc can't PRIME import |
| Display GBM → import to GPU | ❌ | - | - | - | stm32-ltdc has no GBM support |
| Smithay GbmAllocator (SCANOUT) | ❌ | - | - | - | EINVAL on render node |
| Smithay GbmAllocator (RENDERING) | ❌ | - | - | - | EINVAL on render node |
| Smithay GbmAllocator (empty) | ❌ | - | - | - | EINVAL |
| Raw gbm_bo_create (empty flags) | ✓ | ✓ | ❌ | - | Works but can't import |

**Root Cause Analysis**:

The stm32-ltdc display controller is a **simple framebuffer scanout engine**:
- It reads pixels from memory and sends them to the display
- It does NOT have:
  - GBM support (no GPU-style buffer management)
  - PRIME import capability (can't import foreign buffers)
  - Any buffer allocation beyond dumb buffers
- It only supports:
  - Dumb buffers (allocated via `DRM_IOCTL_MODE_CREATE_DUMB`)
  - CMA (Contiguous Memory Allocator) backed memory
  - Direct scanout of its own allocated buffers

The Vivante GC400 GPU (etnaviv):
- Full GBM and EGL support
- Can allocate buffers and render to them
- Can export buffers as DMA-BUF
- **But**: its buffers can't be imported by stm32-ltdc for scanout

**This is a fundamental hardware limitation** - the two devices can't share buffers in the GPU→display direction.

**Working Solution: Dumb Buffer with PRIME Export**

The standard approach for split GPU/display embedded systems works:

1. **Allocate dumb buffer on display device** (stm32-ltdc)
   - Uses `DRM_IOCTL_MODE_CREATE_DUMB` - CMA-backed, scanout-capable
2. **PRIME export as DMA-BUF**
   - `drm_prime_handle_to_fd()` exports the buffer as a file descriptor
3. **Import DMA-BUF into GPU's EGL context**
   - Build `Dmabuf` descriptor with the exported fd
   - GPU can now render to this buffer via OpenGL ES
4. **Create framebuffer on display device**
   - `add_framebuffer()` registers the dumb buffer for scanout
5. **GPU renders, display scans out**
   - Buffer is already on the display device - no copy needed!

**Implementation** (in `bmc-compositor/src/render_egl.rs`):

```rust
fn allocate_buffer(&mut self) -> Result<RenderBuffer> {
    // Step 1: Create dumb buffer on display device (CMA-backed)
    let dumb_buffer = self.display_drm.create_dumb_buffer(
        (self.width, self.height),
        DrmFourcc::Xrgb8888,
        32,
    )?;

    // Step 2: PRIME export as DMA-BUF
    let dmabuf_fd = self.display_drm
        .buffer_to_prime_fd(dumb_buffer.handle(), 0)?;

    // Step 3: Build Dmabuf descriptor for GPU import
    let mut builder = Dmabuf::builder(
        (self.width as i32, self.height as i32).into(),
        DrmFourcc::Xrgb8888,
        DrmModifier::Linear,
        DmabufFlags::empty(),
    );
    builder.add_plane(dmabuf_fd, 0, 0, dumb_buffer.pitch());
    let dmabuf = builder.build()?;

    // Step 4: Create framebuffer on display device
    let fb = self.display_drm.add_framebuffer(&dumb_buffer, 24, 32)?;

    Ok(RenderBuffer { dumb_buffer, dmabuf, fb })
}
```

**Why This Works**:
- stm32-ltdc supports PRIME *export* (just not import)
- The buffer is allocated on display memory (CMA), so scanout works directly
- GPU imports the buffer via DMA-BUF and renders to it
- No CPU copy needed - true zero-copy GPU-accelerated rendering

**Verified Working**:
- Compositor displays dark background with GPU rendering
- Digital clock widget connects as Wayland client and displays correctly
- Full pipeline: Widget → Wayland → Compositor → GPU render → Display scanout

**Environment Variables for Running on Device**:
```bash
export LD_LIBRARY_PATH=$(find /nix/store -maxdepth 3 -type d -name "lib" -path "*armv7l*gnueabihf*" 2>/dev/null | tr '\n' ':')
export GBM_BACKENDS_PATH=/nix/store/.../mesa-.../lib/gbm
export LIBGL_DRIVERS_PATH=/nix/store/.../mesa-.../lib/dri
export __EGL_VENDOR_LIBRARY_FILENAMES=/nix/store/.../mesa-.../share/glvnd/egl_vendor.d/50_mesa.json
export GLIBC_LD=/nix/store/.../glibc-.../lib/ld-linux-armhf.so.3

# Run compositor
$GLIBC_LD --library-path $LD_LIBRARY_PATH /path/to/bmc-compositor-egl
```

---

### Stage 10.2: Grid Layout and Multiple Widgets

**Goal**: Position multiple widgets in a grid layout.

**Scope**:
- Implement grid layout calculation (4x2 grid, 1280x480 display)
- Spawn multiple widgets for a scene
- Position widget surfaces according to grid
- Handle widget surface commits

**Grid Dimensions**:
| Size | Grid Cells | Pixels |
|------|-----------|--------|
| small | 1x1 | 320x240 |
| medium | 2x1 | 640x240 |
| large | 2x2 | 640x480 |
| full | 4x2 | 1280x480 |

**Success Criteria**:
- [ ] Multiple widgets spawned
- [ ] Widgets positioned correctly in grid
- [ ] All widgets visible simultaneously
- [ ] No rendering artifacts

### Status: Not Started

---

### Stage 10.3: Scene Transitions

**Goal**: Implement smooth horizontal scrolling between scenes.

**Scope**:
- Pre-spawn widgets for adjacent scenes
- Animate surface positions for horizontal scroll
- Use vsync for smooth 60fps transitions
- Implement gesture detection for navigation

**Success Criteria**:
- [ ] Smooth 60fps horizontal scroll
- [ ] Touch gesture triggers transition
- [ ] Scene carousel loops correctly

### Status: Not Started

---

## Stage 11: Configuration Migration

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

### Widget Task Data Flow (Current)

```
Clock task (bmc/src/widget_tasks/clock.rs)
  → display_controller.update_clock_widget()
    → Slint IndexMapModel update
      → UI reactive render
```

### Widget Task Data Flow (Target)

```
Clock widget process (standalone binary)
  → IPC socket write (action/ready/error)
    → Deck application IPC handler
      → Forward to appropriate controller

Deck application
  → IPC socket write (init/settings_update/shutdown)
    → Clock widget process receives
      → Updates internal state and renders
```
