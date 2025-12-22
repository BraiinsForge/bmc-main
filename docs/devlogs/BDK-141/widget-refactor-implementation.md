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

Define a compositor interface trait in `bmc` that abstracts away the implementation details, allowing different backends (EGL for ARMv7, mock for x86 development).

### Scope

- Define `CompositorHandle` trait in `bmc` crate
- Define supporting types for widget display configuration
- No implementation yet - just the interface

### Files to Create/Modify

- `bmc/src/compositor.rs` (new) - Trait definition
- `bmc/src/lib.rs` - Export compositor module

### Trait Design

The `CompositorHandle` trait provides the interface between the main application and the compositor:

| Method | Description |
|--------|-------------|
| `start()` | Start the compositor (creates Wayland socket, initializes display) |
| `wayland_display()` | Get the Wayland display socket path for widgets |
| `register_widget()` | Register a widget with position and size configuration |
| `unregister_widget()` | Unregister a widget when its process stops |
| `set_active_scene()` | Set which widgets are visible and where |
| `action_receiver()` | Get channel receiver for widget actions (sound, LED) |
| `shutdown()` | Shutdown the compositor |

### Supporting Types

- `WidgetDisplayConfig` - Position (x, y) and size (width, height) for a widget
- `SceneLayout` - List of widgets with their display configurations
- `WidgetAction` - Action request from a widget (instance_id + action payload)

### Success Criteria

- [ ] `CompositorHandle` trait defined with all necessary methods
- [ ] Supporting types defined
- [ ] Trait is `Send + Sync` for thread-safe access
- [ ] No implementation yet - just the interface

### Status: Not Started

---

## Stage 12: EGL Compositor Implementation in bmc-openwrt

### Goal

Implement `CompositorHandle` trait for ARMv7 using the existing bmc-compositor code. The compositor runs in a dedicated thread within the bmc-openwrt process.

### Scope

- Create `EglCompositor` struct that implements `CompositorHandle`
- Run compositor event loop in a dedicated thread
- Communicate between main thread and compositor via channels
- Reuse existing code from `bmc-compositor` crate

### Files to Create/Modify

- `bmc-openwrt/src/compositor.rs` (new) - EglCompositor implementation
- `bmc-openwrt/src/main.rs` - Initialize and pass compositor to App

### Implementation Approach

The `EglCompositor` wraps the compositor thread and provides channel-based communication:

| Component | Purpose |
|-----------|---------|
| `compositor_thread` | Handle to the compositor thread |
| `command_tx` | Channel to send commands to compositor |
| `action_rx` | Channel to receive widget actions |
| `wayland_display` | Path to Wayland socket |

Commands sent to compositor:
- `RegisterWidget` - Register widget with position/size
- `UnregisterWidget` - Remove widget
- `SetActiveScene` - Update visible widgets
- `Shutdown` - Stop compositor

### Key Integration Points

- PID matching for widget identification (compositor reads socket credentials)
- Scene layout determines widget visibility and positioning
- Frame callbacks only sent to visible widgets

### Success Criteria

- [ ] EglCompositor implements CompositorHandle
- [ ] Compositor runs in dedicated thread
- [ ] Widget registration/unregistration works
- [ ] Action channel receives widget requests
- [ ] Scene layout updates work

### Status: Not Started

---

## Stage 13: Coordinator-Compositor Integration

### Goal

Connect the WidgetCoordinator to the compositor so spawned widgets can render.

### Scope

- Add compositor reference to Coordinator
- Pass WAYLAND_DISPLAY to spawned widgets
- Register widgets with compositor before spawning
- Unregister widgets when stopped

### Files to Modify

- `bmc/src/widget/coordinator.rs` - Add compositor reference
- `bmc/src/widget/manager.rs` - Pass WAYLAND_DISPLAY to spawned widgets
- `bmc/src/entry.rs` - Wire up compositor to app

### Widget Spawn Flow

1. Calculate widget position from scene layout
2. Register widget with compositor (reserves position)
3. Spawn widget process with `WAYLAND_DISPLAY` env var
4. Widget connects to compositor's Wayland socket
5. Compositor matches by PID
6. Widget renders to its surface

### Success Criteria

- [ ] Coordinator holds compositor reference
- [ ] Widgets spawned with WAYLAND_DISPLAY env var
- [ ] Widget registration happens before process spawn
- [ ] Widget unregistration on process stop

### Status: Not Started

---

## Stage 14: Scene Management

### Goal

Implement scene switching while keeping all widgets running. Compositor controls visibility via frame callbacks.

### Scope

- Scene switching updates compositor's active layout
- Hidden widgets stop receiving frame callbacks
- Compositor sends "out of scope" signal to hidden widgets
- Widgets resume when visible again

### Frame Callback Optimization

- Only visible widgets receive frame callbacks
- Hidden widgets receive visibility event and pause rendering
- Standard Wayland pattern - clients wait for frame callbacks
- Saves CPU when widgets are not displayed

### Success Criteria

- [ ] Scene switching works
- [ ] All widgets remain running across scene changes
- [ ] Hidden widgets stop rendering (no frame callbacks)
- [ ] Widgets resume rendering when visible again

### Status: Not Started

---

## Stage 15: Action Routing (Sound/LED)

### Goal

Route widget action requests to hardware controllers.

### Scope

- Spawn action handler task in background
- Receive actions from compositor's action channel
- Route to appropriate controller

### Action Types

| Action | Controller |
|--------|------------|
| `PlaySound` | SoundController |
| `StopSound` | SoundController |
| `Led` | LedController |
| `StopLed` | LedController |

### Files to Modify

- `bmc/src/entry.rs` - Spawn action handler task
- `bmc/src/widget/action_handler.rs` (new) - Process widget actions

### Success Criteria

- [ ] Action handler task runs in background
- [ ] Sound actions trigger SoundController
- [ ] LED actions trigger LedController

### Status: Not Started

---

## Stage 16: Wayland Protocol Extension for Widget Communication

### Goal

Replace the custom JSON-over-Unix-socket IPC with a proper Wayland protocol extension (`bmc_widget_v1`) for compositor-widget communication.

### Background

The JSON IPC is non-standard and requires widget developers to implement custom parsing. A Wayland protocol extension:
- Uses a single protocol for all communication
- Provides type-safe, versioned messages
- Generates client bindings automatically via `wayland-scanner`
- Is the standard approach for Wayland compositors

### Protocol Structure

**`bmc_widget_manager_v1`** (global singleton):
- Bound by widgets to access BMC functionality
- Provides `get_widget_surface` request

**`bmc_widget_surface_v1`** (per-surface):

| Direction | Message | Description |
|-----------|---------|-------------|
| Compositor → Widget | `configure` | Widget size and type |
| Compositor → Widget | `params` | JSON widget parameters |
| Compositor → Widget | `setting` | System setting update |
| Compositor → Widget | `visibility` | Widget visible/hidden |
| Compositor → Widget | `shutdown` | Graceful shutdown |
| Widget → Compositor | `ack_configure` | Acknowledge configure |
| Widget → Compositor | `error` | Report error |
| Widget → Compositor | `action` | Request system action |

### Widget Identification

PID matching via Wayland socket credentials:
1. BMC spawns widget, records PID
2. Widget connects to Wayland
3. Compositor reads client PID from socket
4. Compositor matches PID to pending spawn

### Implementation Stages

| Sub-stage | Description |
|-----------|-------------|
| 16.1 | Protocol XML definition |
| 16.2 | Protocol crate setup (`bmc-widget-protocol`) |
| 16.3 | Compositor protocol implementation |
| 16.4 | Widget client library |
| 16.5 | Widget integration (digital-clock) |
| 16.6 | Migration and cleanup |

### Success Criteria

- [ ] Protocol XML validates with wayland-scanner
- [ ] Compositor implements protocol handlers
- [ ] Widget client library works
- [ ] digital-clock uses Wayland protocol
- [ ] JSON IPC deprecated

### Status: Not Started

---

## Stage 17: Configuration Migration

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

### Key Files in New Architecture

| File | Purpose |
|------|---------|
| `bmc-compositor/src/state.rs` | Wayland compositor state and protocol handlers |
| `bmc-compositor/src/render_egl.rs` | EGL render pipeline for ARMv7 |
| `bmc/src/compositor.rs` | CompositorHandle trait definition |
| `bmc/src/widget/coordinator.rs` | Widget lifecycle orchestration |
| `bmc/src/widget/manager.rs` | Widget process spawning |
| `bmc-openwrt/src/compositor.rs` | EglCompositor implementation |
| `widgets/digital-clock/` | Standalone digital clock widget |
