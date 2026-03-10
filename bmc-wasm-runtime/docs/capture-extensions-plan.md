# Capture System Extensions Plan

Follow-up extensions to the base capture system (BDK-331).

## 1. Unified Data Encoding in Fixtures

All fixture data uses a nested object with a single key indicating encoding:

```json
{ "text": "plain string content" }
{ "b64": "base64encodeddata==" }
```

### Fetch fixtures (updated format)

```json
{
  "GET https://api.example.com/data": { "status": 200, "body": { "text": "{\"items\": []}" } },
  "GET https://api.example.com/image": { "status": 200, "body": { "b64": "iVBORw0K..." } }
}
```

Replaces the current `body_base64` field. Loader should accept both formats during migration (check for `body_base64` as
fallback).

## 2. Event-Based Fixture Replay (SSDP, mDNS, WebSocket)

### Problem

Widgets like media-control depend on discovery protocols (SSDP, mDNS) and persistent connections (WebSocket) that the
current fetch-only fixture system can't replay. Without this, those widgets can't be capture-tested at all.

### Design

The fixture file grows a timestamped event stream alongside the existing fetch map:

```json
{
  "fetches": {
    "GET https://jellyfin.local/...": { "status": 200, "body": { "text": "..." } }
  },
  "events": [
    { "at_ms": 0,   "type": "ssdp_found",   "data": { "text": "{\"location\": \"...\"}" } },
    { "at_ms": 16,  "type": "ws_open",       "ws_id": 1 },
    { "at_ms": 32,  "type": "ws_message",    "ws_id": 1, "data": { "text": "{\"playing\": true}" } },
    { "at_ms": 48,  "type": "ws_message",    "ws_id": 1, "data": { "b64": "AQIDBAU=" } },
    { "at_ms": 100, "type": "mdns_found",    "data": { "text": "{\"name\": \"kodi._http._tcp\"}" } },
    { "at_ms": 200, "type": "ssdp_removed",  "data": { "text": "uuid:device-1" } },
    { "at_ms": 300, "type": "ws_close",      "ws_id": 1, "code": 1000 }
  ]
}
```

### Event types

| type               | fields              | maps to                                   |
| ------------------ | ------------------- | ----------------------------------------- |
| `ssdp_found`       | `data`              | `SsdpEvent::Found(json)`                  |
| `ssdp_removed`     | `data`              | `SsdpEvent::Removed(usn)`                 |
| `mdns_found`       | `data`              | `MdnsEvent::Found(json)`                  |
| `mdns_removed`     | `data`              | `MdnsEvent::Removed(name)`                |
| `ws_open`          | `ws_id`             | `WsEvent::Open`                           |
| `ws_message`       | `ws_id`, `data`     | `WsEvent::Message(bytes)`                 |
| `ws_close`         | `ws_id`, `code`     | `WsEvent::Close(code)`                    |
| `socket_connected` | `socket_id`         | `SocketEvent::Connected`                  |
| `socket_data`      | `socket_id`, `data` | `SocketEvent::Data(bytes)`                |
| `socket_closed`    | `socket_id`, `code` | `SocketEvent::Closed(code)`               |
| `udp_response`     | `data`, `source`    | `UdpBroadcastEvent::Response(data, addr)` |

### Replay mechanism

In the capture frame loop, before `deliver_*` calls:

```rust
// Drain all events whose at_ms <= current monotonic_ms
while let Some(event) = fixture_events.peek() {
    if event.at_ms > monotonic_ms { break; }
    let event = fixture_events.next();
    match event.type {
        "ssdp_found" => inject into ssdp_searches event_rx,
        "ws_message" => inject into websockets event_rx,
        // ...
    }
}
```

### Intercept outbound calls

When fixtures are loaded, `host_ws_connect`, `host_ssdp_search`, `host_mdns_browse`, etc. should create stub entries in
their respective HashMaps with synthetic channels — the fixture replay feeds events into the rx side. No real network
connections are made.

### Recording

When `--record-fixtures` is active, wrap each `deliver_*` method to capture events with their `monotonic_ms` timestamp.
After the capture loop, write the combined fixture file.

## 3. KV Variant Matrix

### Problem

Widgets have user-configurable state (theme, locale, skin, connection settings) stored in the KV store. Capture needs to
test multiple combinations to catch regressions across all variants.

### capture.toml format

```toml
# Default KV values applied to all variants (and to the single capture when no variants defined)
[kv]
ical_url = "fixtures/sample.ics"

# When variants are defined, each is captured separately
[[variants]]
name = "dark-theme"
kv = { theme = "dark" }

[[variants]]
name = "light-theme"
kv = { theme = "light" }

[[variants]]
name = "compact"
kv = { theme = "dark", layout = "compact" }
```

### Merge semantics

Per-variant `kv` merges on top of the top-level `[kv]`. Variant keys override top-level keys.

### Output structure

```
captures/<widget>/<variant>/<size>/frame_0001.png
```

When no `[[variants]]` defined, no variant subdirectory:

```
captures/<widget>/<size>/frame_0001.png
```

### Makefile integration

The capture binary handles variant iteration internally — one invocation per size, the binary loops over variants.
Alternatively, the binary accepts `--variant=<name>` and the Makefile loops. The latter is simpler (one size + one
variant per invocation, same pattern as sizes).

### CLI

```
capture <wasm> --size=1280x480 --output=captures/calendar/dark-theme/full
    --variant=dark-theme
```

The `--variant` flag selects which variant's KV to apply. The Makefile generates the full matrix:

```makefile
@for entry in $(SIZES); do \
    for variant in $$(capture --list-variants $(WASM_FILE)); do \
        xvfb-run -a cargo run --features capture --bin capture -- \
            $(WASM_FILE) --size=$$dim --variant=$$variant \
            --output=$(CAPTURE_DIR)/current/$$variant/$$name; \
    done \
done
```

## 4. Regression Compare Script

Use nix shebang for system deps:

```python
#!/usr/bin/env nix
#!nix shell nixpkgs#python312 nixpkgs#odiff
#!nix --command python3
```

## Implementation Order

1. **Unified data encoding** — update `FixtureResponse` and loader, backward-compat with `body_base64`
2. **KV variant matrix** — `[kv]` and `[[variants]]` in capture.toml, `--variant` CLI flag
3. **Event fixture replay** — stub channels, replay loop, intercept outbound calls
4. **Event fixture recording** — wrap deliver\_\* methods, write combined fixture file
