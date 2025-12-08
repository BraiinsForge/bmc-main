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

## Stage 1: Protocol Trait Definition

### Goal
Create the `bmc-ipc` crate with the abstract `Protocol` trait that defines how messages are encoded/decoded. This establishes the foundation for pluggable protocols.

### Scope
- Create new `bmc-ipc` crate
- Define `Protocol` trait for message encoding/decoding
- Define error types
- Add unit tests with a mock protocol implementation

### Files to Create

```
bmc-ipc/
  Cargo.toml
  src/
    lib.rs
    protocol.rs      # Protocol trait definition
    error.rs         # Error types
```

### Protocol Trait

The `Protocol` trait defines two methods:
- `encode` - Serialize a message to bytes
- `decode` - Deserialize bytes to a message

Framing (splitting byte streams into discrete messages) is handled separately at the transport layer, not in the protocol trait. This keeps the trait focused on serialization/deserialization.

### Test Cases

1. **Mock Protocol**
   - Create a simple mock protocol for testing
   - Verify encode/decode round-trip works
   - Verify error handling on invalid input

### Success Criteria

- [ ] `bmc-ipc` crate created and added to workspace
- [ ] `Protocol` trait defined with encode/decode methods
- [ ] `ProtocolError` type defined
- [ ] Unit tests pass with mock protocol

### Dependencies

- `serde` for Serialize/DeserializeOwned traits
- `thiserror` for error types

---

## Stage 2: JsonLineProtocol Implementation

### Goal
Implement the newline-delimited JSON protocol as the default `Protocol` implementation.

### Scope
- Implement `JsonLineProtocol` struct
- Add unit tests for JSON encoding/decoding

### Files to Modify/Create

```
bmc-ipc/
  src/
    lib.rs           # Export JsonLineProtocol
    protocol.rs      # Add JsonLineProtocol implementation
```

### Test Cases

1. **Encoding**
   - Verify valid JSON structure

2. **Decoding**
   - Parse valid JSON
   - Error on malformed JSON
   - Error on empty input

3. **Round-trip**
   - Encode then decode simple structs
   - Verify data integrity preserved

### Success Criteria

- [ ] `JsonLineProtocol` implements `Protocol` trait
- [ ] Unit tests pass for encoding/decoding

### Dependencies

- `serde_json` for JSON handling

---

## Stage 3: Shared Widget Types

### Goal
Define shared types used by both the Deck application and widgets.

### Scope
- Create new `bmc-shared/widget-types` crate
- Define shared data types (SizeInfo, Localization, ActionPayload, etc.)
- Add serialization tests

### Files to Create

```
bmc-shared/widget-types/
  Cargo.toml
  src/
    lib.rs
    size.rs          # SizeType, SizeInfo
    settings.rs      # Localization
    actions.rs       # ActionPayload (PlaySound, Led, etc.)
```

### Types to Define

- `SizeType` - enum (Small, Medium, Large, Full)
- `SizeInfo` - name, width, height
- `Localization` - dateFormat, timeFormat, numberFormat, temperatureUnit, firstDayOfWeek
- `ActionPayload` - PlaySound, StopSound, Led, StopLed

### Test Cases

1. **Serialization**
   - Each type serializes to expected JSON format
   - JSON field names match spec (camelCase)

2. **Deserialization**
   - Parse valid JSON for each type
   - Error on missing required fields

### Success Criteria

- [ ] `bmc-shared/widget-types` crate created and added to workspace
- [ ] All shared types implemented with serde derives
- [ ] Unit tests pass for serialization/deserialization

---

## Stage 4: IPC Message Types

### Goal
Define message types for IPC communication, using shared types from `bmc-shared/widget-types`.

### Scope
- Define message enums and structs in `bmc-ipc`
- Add serialization tests to verify JSON format matches spec

### Files to Create

```
bmc-ipc/
  src/
    lib.rs              # Export messages module
    messages/
      mod.rs            # Message enums (AppMessage, WidgetMessage)
      app_to_widget.rs  # Init, SettingsUpdate, Shutdown
      widget_to_app.rs  # Ready, Error, Action
```

### Message Types

**Application to Widget (`AppMessage`):**
- `Init` - size, params, settings
- `SettingsUpdate` - key, value
- `Shutdown`

**Widget to Application (`WidgetMessage`):**
- `Ready`
- `Error` - message, recoverable
- `Action` - name, payload (uses `ActionPayload` from `bmc-shared/widget-types`)

### Test Cases

1. **Message Serialization**
   - Each message type serializes to expected JSON format
   - JSON output matches examples in `widget-ipc-protocol.md`

2. **Message Deserialization**
   - Parse valid JSON for each message type
   - Error on unknown message type
   - Error on missing required fields

3. **Round-trip with JsonLineProtocol**
   - Encode then decode each message type
   - Verify all fields preserved

### Success Criteria

- [ ] All message types from IPC protocol spec implemented
- [ ] JSON output matches examples in `widget-ipc-protocol.md`
- [ ] Unit tests pass for serialization/deserialization

### Notes

- Depends on `bmc-shared/widget-types` for shared types
- No runtime behavior yet - pure data types
- No changes to existing `bmc` or `bmc-display` crates

---

## Stage 5: Widget Manifest Parser

*To be defined after Stage 4 review*

---

## Stage 6: Widget Registry

*To be defined after Stage 4 review*

---

## Stage 7: Widget Process Spawner

*To be defined after Stage 4 review*

---

## Stage 8: Digital Clock Widget Extraction

*To be defined after Stage 4 review*

---

## Stage 9: IPC Integration in Deck Application

*To be defined after Stage 4 review*

---

## Stage 10: Configuration Migration

*To be defined after Stage 4 review*

---

## Stage 11: Remaining Widget Extraction

*To be defined after Stage 4 review*

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
