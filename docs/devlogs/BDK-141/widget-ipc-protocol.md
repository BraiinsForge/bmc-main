# Widget IPC Protocol Specification

This document defines the IPC protocol for communication between the main Deck application and widget instances.

## Overview

Each widget instance communicates with the main Deck application via a dedicated Unix socket. The application creates the socket before spawning the widget, and the widget connects to it on startup.

## Socket

- Path: `/run/bmc/widgets/<instance-id>.sock`
- Type: Unix domain socket (SOCK_STREAM)
- Creator: Deck application (before spawning widget)
- The socket path is passed to the widget via `BMC_IPC_SOCKET` environment variable

## Message Format

Messages are newline-delimited JSON. Each message is a single JSON object followed by a newline character (`\n`).

```
{"type": "message_type", ...fields}\n
```

## Connection Lifecycle

1. Application creates socket at `/run/bmc/widgets/<instance-id>.sock`
2. Application spawns widget process with `BMC_IPC_SOCKET` environment variable
3. Widget connects to the socket
4. Application sends `init` message with configuration
5. Widget sends `ready` message when initialized
6. Bidirectional communication continues until shutdown
7. Application sends `shutdown` message before terminating widget
8. Application cleans up socket after widget exits

## Application to Widget Messages

### `init`

Sent immediately after widget connects. Contains full configuration for the widget instance.

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `"init"` |
| `size` | SizeInfo | Current widget size |
| `params` | object | Widget instance configuration parameters |
| `settings` | object | Current values of subscribed system settings (only settings declared in manifest are included) |

**Note:** When a widget subscribes to `localization`, the entire localization object with all format preferences is sent (not individual fields). See [Available Settings](widget-manifest.md#settings) for details.

```json
{
  "type": "init",
  "size": {
    "name": "large",
    "width": 638,
    "height": 480
  },
  "params": {
    "style": "modern",
    "showSeconds": false
  },
  "settings": {
    "localization": {
      "dateFormat": "DdMmYyyyDot",
      "timeFormat": "Hour24",
      "numberFormat": "SpaceGroupCommaDecimal",
      "temperatureUnit": "Celsius",
      "firstDayOfWeek": "Monday"
    },
    "timezone": "Europe/Prague",
    "nightMode": false
  }
}
```

### `settings_update`

Sent when a subscribed system setting changes. The entire category is sent, not individual fields.

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `"settings_update"` |
| `key` | string | Setting category name (`localization`, `timezone`, or `nightMode`) |
| `value` | any | New setting value (object for `localization`, primitive for others) |

Example - night mode change:
```json
{
  "type": "settings_update",
  "key": "nightMode",
  "value": true
}
```

Example - localization change (entire object is sent when any field changes):
```json
{
  "type": "settings_update",
  "key": "localization",
  "value": {
    "dateFormat": "DdMmYyyyDot",
    "timeFormat": "Hour24",
    "numberFormat": "SpaceGroupCommaDecimal",
    "temperatureUnit": "Celsius",
    "firstDayOfWeek": "Monday"
  }
}
```

### `shutdown`

Sent before application terminates the widget process. Widget should clean up and exit gracefully.

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `"shutdown"` |

```json
{
  "type": "shutdown"
}
```

## Widget to Application Messages

### `ready`

Sent by widget after initialization is complete and it is ready to display.

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `"ready"` |

```json
{
  "type": "ready"
}
```

### `error`

Sent when widget encounters an error.

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `"error"` |
| `message` | string | Error description |
| `recoverable` | boolean | Whether widget can continue operating |

```json
{
  "type": "error",
  "message": "Failed to fetch weather data",
  "recoverable": true
}
```

### `action`

Sent to request system actions. The application executes the action on behalf of the widget.

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `"action"` |
| `name` | string | Action name |
| `payload` | object | Action-specific parameters |

#### Available Actions

##### `play_sound`

Play a predefined sound effect.

| Payload Field | Type | Description |
|---------------|------|-------------|
| `sound` | string | Sound ID (available via internal API) |

```json
{
  "type": "action",
  "name": "play_sound",
  "payload": {
    "sound": "confirmation"
  }
}
```

##### `stop_sound`

Stop any currently playing sound.

| Payload Field | Type | Description |
|---------------|------|-------------|
| (none) | | |

```json
{
  "type": "action",
  "name": "stop_sound",
  "payload": {}
}
```

##### `led`

Control LED effect.

| Payload Field | Type | Description |
|---------------|------|-------------|
| `effect` | string | LED effect name (see available effects below) |
| `color` | object | RGB color with `r`, `g`, `b` values (0-255) |
| `duration` | number | Optional duration in milliseconds (omit for persistent) |

Available effects:
- `chase`
- `knight_rider`
- `scan`
- `snake`
- `breathe`
- `solid`

```json
{
  "type": "action",
  "name": "led",
  "payload": {
    "effect": "breathe",
    "color": { "r": 255, "g": 0, "b": 0 },
    "duration": 5000
  }
}
```

##### `stop_led`

Stop current LED effect and return to default state.

| Payload Field | Type | Description |
|---------------|------|-------------|
| (none) | | |

```json
{
  "type": "action",
  "name": "stop_led",
  "payload": {}
}
```

## Error Handling

- If widget fails to connect within 5 seconds of spawn, application should terminate it
- If widget sends malformed JSON, application logs error and ignores message
- If socket connection is lost unexpectedly, application should restart the widget

## SizeInfo Object

Used in the `init` message to provide the widget's size type and pixel dimensions.

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Size type (`small`, `medium`, `large`, `full`) |
| `width` | integer | Width in pixels |
| `height` | integer | Height in pixels |

See [Size Dimensions](widget-manifest.md#size-dimensions) for the standard size values.
