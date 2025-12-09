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

- [ ] Socket creation and cleanup works
- [ ] Process spawns with correct env vars
- [ ] Init/ready handshake works
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

- [ ] `WidgetClient` connects and handles IPC protocol
- [ ] Unit tests for client connection and message handling
- [ ] Example usage documented

### Status: Not Started

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

- [ ] Clock widget runs as standalone process
- [ ] IPC handshake works
- [ ] Settings updates work
- [ ] All sizes render correctly
- [ ] Nix package builds

### Dependencies

- Stage 3 (IPC Message Types)
- Stage 6 (Process Spawner) - for integration testing
- Stage 7 (Widget Client SDK) - for IPC client

### Status: Not Started

---

## Stage 9: IPC Integration in Deck Application

### Goal
Integrate widget process management into the main Deck application.

### Scope
- Add widget registry to application startup
- Replace built-in clock with spawned clock widget
- Handle IPC messages from widgets
- Forward actions to appropriate controllers

### Integration Points

1. **Startup**
   - Platform-specific crate (`bmc-openwrt` or `bmc-mock`) creates `WidgetDiscovery` implementation
   - Initialize `WidgetRegistry` with the discovery implementation
   - Log available widgets

2. **Scene Loading**
   - For each widget instance in config:
     - Look up widget in registry
     - Spawn widget process
     - Track in `WidgetManager`

3. **Action Handling**
   - Receive `Action` messages from widgets
   - Route to appropriate controller:
     - `play_sound` / `stop_sound` → `SoundController`
     - `led` / `stop_led` → `LedController`

4. **Settings Broadcasting**
   - When system setting changes:
     - Find widgets subscribed to that setting
     - Send `settings_update` to each

### WidgetManager

`WidgetManager` is generic over its dependencies (registry, spawner) to allow mocking in tests:
- Takes `WidgetRegistry` and `ProcessSpawner` as constructor parameters
- All platform-specific behavior is injected, not hardcoded

**API:**
- `WidgetManager::new(registry, spawner)` - Create with injected dependencies
- `WidgetManager::spawn_widget(widget_uid, instance_id, size, params, settings)` - Spawn a widget instance
- `WidgetManager::stop_widget(instance_id)` - Stop a widget instance
- `WidgetManager::broadcast_settings_update(update)` - Send settings update to subscribed widgets
- `WidgetManager::handle_widget_messages()` - Stream of messages from all widgets

### Test Cases

1. **Registry Integration**
   - Registry loads on startup
   - Widgets discoverable

2. **Widget Spawning**
   - Spawn widget from config
   - Handle spawn failures gracefully

3. **Action Routing**
   - Sound actions forwarded correctly
   - LED actions forwarded correctly

4. **Settings Broadcast**
   - Subscribed widgets receive updates
   - Non-subscribed widgets don't receive

### Success Criteria

- [ ] Clock widget spawns from Deck application
- [ ] Actions route to controllers
- [ ] Settings broadcast works
- [ ] Graceful handling of widget crashes

### Dependencies

- Stage 5 (Widget Registry)
- Stage 6 (Process Spawner)
- Stage 8 (Clock Widget)

### Status: Not Started

---

## Stage 10: Configuration Migration

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

### Dependencies

- Stage 8 (Clock Widget) - for initial UID mapping
- Stage 9 (Deck Integration) - for runtime testing

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
