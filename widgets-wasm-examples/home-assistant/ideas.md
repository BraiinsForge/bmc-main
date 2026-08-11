# Home Assistant Widget — Idea

## Context

This isn't meant to be production-grade any time soon. HA integration is overkill for most users — it's really for
homebrew devs who enjoy tinkering. The main value is:

- An interesting example widget for the SDK
- Stress-testing the SDK's networking and rendering capabilities
- Validating the host API design (especially the WebSocket gap)

Direct protocol widgets (e.g. Chromecast via `rust_cast`) are still worth exploring separately — they're simpler,
self-contained, and don't require an HA instance.

## The Problem

Building device-specific WASM widgets (Chromecast, UPnP, Zigbee lights...) means duplicating discovery, connection
management, protocol handling, and UI rendering for every device type.

## The Insight

Home Assistant already speaks all these protocols and exposes everything as a unified **entity model** via its WebSocket
API. A Chromecast is just `media_player.living_room`. A UPnP speaker is `media_player.bedroom_speaker`. A Hue bulb is
`light.kitchen`.

**One generic WASM widget can control any device** — it only cares about the HA entity domain, not the underlying
protocol.

## How It Works

The widget connects to HA's WebSocket API, subscribes to entity state changes, and renders UI based on the entity
**domain** — not the underlying protocol:

- `light.*` → toggle, brightness slider, color picker
- `climate.*` → current/target temp, mode selector
- `media_player.*` → play/pause, volume, now-playing info (works for Chromecast, UPnP, Sonos, anything)
- `switch.*` → simple toggle
- `cover.*` → open/close/stop, position slider
- `lock.*` → lock/unlock

When the user taps "play" on a media player, the widget calls `media_player.media_play_pause`. HA translates that to
whatever protocol the actual device speaks. **The widget code is identical regardless of device.**

For truly generic behavior, the widget can query HA's service registry to discover what services an entity supports and
build UI controls dynamically — no widget update needed for new HA integrations.

## What the Host Provides

The host firmware provides **generic primitives only** — no HA-specific logic:

- Rendering (existing draw commands)
- Input (existing touch/button handling)
- **WebSocket bridge** (new — persistent connection with auth/reconnection)
- Fetch (existing HTTP for one-off requests)

HA integration logic lives entirely in the WASM widget.

## Configuration

Users don't configure the widget directly:

1. One-time setup: point device at HA instance + provide auth token
2. Configure in HA which entities appear on the device (custom integration, dashboard definition, or just a list of
   entity IDs)
3. Widget renders dynamically based on entity state and metadata

No widget-specific config syntax. Reconfigure without reflashing.

## What's Needed

- ~~**WebSocket host API** — persistent bidirectional connection~~ (done — `ws!()` macro + `Ws`/`WsEvent`)
- **Entity renderer** — domain-based UI rendering logic in the WASM widget (basic entity list done, domain-specific
  controls pending)
- HA auth token storage on device

## Open Questions

- Offline behavior — cache last-known state? Which controls remain functional?
- Large entity lists on small screen — pagination, tabs, swipe between pages?
- Image loading for album art, device icons — caching strategy?
- Should the widget auto-discover entities or only show explicitly configured ones?
