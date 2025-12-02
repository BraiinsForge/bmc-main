# Widget Manifest Specification

This document defines the manifest schema for BMC widgets. The manifest describes a widget's metadata, capabilities, configuration schema, and assets.

## Overview

Each widget is distributed as a Nix package containing:

- `manifest.json` - Widget manifest (this specification)
- Binary executable - Wayland application
- Assets - Icons and preview images

The main Deck application scans widget directories, reads manifests, and presents available widgets to users. When a user creates a widget instance, the application spawns the binary as a Wayland client.

## Manifest Location

Widgets are installed into the Nix store and symlinked to a known location:

```
/nix/store/<hash>-<widget-name>/lib/bmc-widgets/<widget-name>/manifest.json
```

The system configuration symlinks installed widgets to a standard scan directory with separate subdirectories for official and third-party widgets:

```
/usr/lib/bmc-widgets/
  official/
    <widget-name>/ -> /nix/store/<hash>-<widget-name>/...
  third-party/
    <widget-name>/ -> /nix/store/<hash>-<widget-name>/...
```

This separation enables easy factory reset by removing the entire `third-party` directory.

The main Deck application scans both `/usr/lib/bmc-widgets/official/` and `/usr/lib/bmc-widgets/third-party/` to discover available widgets.

## Schema

The manifest is a JSON file with the following structure:

### Root Object

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `uid` | string | Yes | Unique widget identifier (UUID v4) |
| `version` | string | Yes | Widget version |
| `name` | string | Yes | Human-readable display name |
| `description` | string | Yes | Brief description of the widget |
| `author` | Author | No | Widget author information |
| `binary` | string | Yes | Path to executable relative to widget directory |
| `settings` | string[] | No | System settings the widget subscribes to |
| `sizes` | string[] | Yes | Supported widget size types |
| `params` | object | No | Schema for widget instance configuration parameters |

### Field Specifications

#### `uid`

Unique identifier for the widget.

- Type: `string`
- Format: UUID v4 (e.g., `550e8400-e29b-41d4-a716-446655440000`)
- Must be generated once when creating the widget and never changed

#### `version`

Semantic version of the widget.

- Type: `string`
- Format: Semantic versioning (MAJOR.MINOR.PATCH)
- Examples: `1.0.0`, `2.1.3`

#### `name`

Human-readable name displayed in the widget selection UI.

- Type: `string`
- Max length: 50 characters

#### `description`

Brief description of what the widget does, displayed in the widget selection UI.

- Type: `string`
- Max length: 200 characters

#### `author`

Information about the widget author or publisher.

- Type: `Author` object

##### Author Object

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Author or organization name |
| `url` | string | No | Website or repository URL |

#### `binary`

Path to the widget executable, relative to the widget directory.

- Type: `string`
- Format: Relative path
- The binary must be executable
- Examples: `bin/clock`, `bin/weather`

#### `settings`

Array of system setting keys that the widget subscribes to. The main Deck application sends the current values in the `init` message and sends `settings_update` messages via IPC when these settings change.

- Type: `string[]`
- Available setting keys:

| Key | Value Type | Description |
|-----|------------|-------------|
| `localization` | object | All localization/format preferences (see below) |
| `timezone` | string | IANA timezone identifier (e.g., `Europe/Prague`) |
| `nightMode` | boolean | Night mode active state |

Example:
```json
"settings": ["localization", "timezone", "nightMode"]
```

##### Localization Object

When a widget subscribes to `localization`, it receives an object containing all format preferences:

| Field | Type | Description |
|-------|------|-------------|
| `dateFormat` | string | Date format pattern |
| `timeFormat` | string | Time format (`12h` or `24h`) |
| `numberFormat` | string | Number format locale |
| `temperatureUnit` | string | Temperature unit (`celsius` or `fahrenheit`) |
| `firstDayOfWeek` | string | First day of the week (`monday` or `sunday`) |


#### `sizes`

Array of supported widget size types. At least one size must be specified. Widgets must support all sizes they declare. The actual pixel dimensions are sent to the widget in the `init` message.

- Type: `string[]`
- Min items: 1
- Available size types: `small`, `medium`, `large`, `full`

Example:
```json
"sizes": ["small", "medium", "large", "full"]
```

##### Size Dimensions

The Deck application sends pixel dimensions in the [`init` message](widget-ipc-protocol.md#init) based on the size type. See [SizeInfo Object](widget-ipc-protocol.md#sizeinfo-object) for the message format.

Dimensions for the Braiins Deck (1280x480 display, 4x2 grid):

| Size Type | Width | Height | Grid Cells |
|-----------|-------|--------|------------|
| `small` | 317 | 238 | 1x1 |
| `medium` | 638 | 238 | 2x1 |
| `large` | 638 | 480 | 2x2 |
| `full` | 1280 | 480 | 4x2 |

#### `params`

Object defining the configuration parameters that users can set per widget instance. Keys are parameter identifiers, values are parameter definitions.

- Type: `object`
- Keys: Parameter identifier (camelCase, alphanumeric)
- Values: `ParamDefinition` object

##### ParamDefinition Object

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Human-readable label |
| `type` | string | Yes | Parameter type |
| `description` | string | No | Help text for the parameter |
| `default` | any | Yes | Default value (must match type) |
| `enum` | object | No | Allowed values for string type |
| `min` | number | No | Minimum value for number type |
| `max` | number | No | Maximum value for number type |

##### Parameter Types

| Type | JSON Type | Description |
|------|-----------|-------------|
| `string` | string | Text value, optionally constrained by `enum` |
| `boolean` | boolean | True/false toggle |
| `number` | number | Numeric value, optionally constrained by `min`/`max` |
| `array` | array | Array of values |

##### Enum Object

For string parameters with a fixed set of allowed values. Keys are the actual values stored, values are human-readable labels.

- Type: `object`
- Keys: Actual value (stored in config)
- Values: Display label (shown in UI)

## Widget Directory Structure

```
/usr/lib/bmc-widgets/
  official/
    <widget-name>/
      manifest.json
      bin/
        <binary-name>
      assets/
        icon.png
        preview-small.png    # if small size supported
        preview-medium.png   # if medium size supported
        preview-large.png    # if large size supported
        preview-full.png     # if full size supported
  third-party/
    <widget-name>/
      ...
```

## Validation Rules

1. `uid` must be unique across all installed widgets
2. `binary` must point to an executable file
3. `sizes` must contain at least one entry
4. `params` default values must match their declared type
5. `params` enum values must be provided if parameter uses enum constraint
6. `assets/` directory must exist and contain `icon.png`
7. `assets/` directory must contain `preview-<size>.png` for all sizes declared in `sizes`

## Runtime Behavior

When a widget instance is created:

1. Application validates instance configuration against `params` schema
2. Application creates IPC socket at `/run/bmc/widgets/<instance-id>.sock`
3. Application spawns the binary with environment variable:
   - `BMC_IPC_SOCKET` - Path to the IPC socket
4. Widget connects to IPC socket and receives [`init` message](widget-ipc-protocol.md#init) with full configuration
5. Application sends [`settings_update` messages](widget-ipc-protocol.md#settings_update) when subscribed settings change
