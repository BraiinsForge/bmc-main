# Capture System Extensions Plan

Follow-up extensions to the base capture system (BDK-331).

## Status Summary

| Extension                   | Status      | Notes                                                 |
| --------------------------- | ----------- | ----------------------------------------------------- |
| Unified data encoding       | **Done**    | `{ "text": ... }` / `{ "b64": ... }` format           |
| KV variant matrix           | **Done**    | `[kv]`, `[[variants]]`, `--variant`, merge logic      |
| Fetch fixture recording     | **Done**    | `--record-fixtures` records via `FetchObserver`       |
| Fetch fixture replay        | **Done**    | `FetchInterceptor` serves from fixtures dir           |
| Event fixture replay        | **Done**    | Timestamped event stream, stub channels               |
| Event fixture recording     | **Done**    | `record_events` flag, `take_recorded_events()`        |
| Clean observer architecture | **Done**    | `RuntimeConfig` with hooks, no internal fixture state |
| Fetch cycle detection       | **Done**    | Auto-stops recording when widget re-polls same URL    |
| `record_timeout` config     | **Done**    | Configurable wall-clock cap for recording mode        |
| GLES 2.0 context in capture | **Done**    | Sphere shader `#version 100`, GLES context request    |
| Calendar fixtures           | **Done**    | 4 iCal feeds + synthetic test events                  |
| ISS Position fixtures       | **Done**    | Position API + TLE API                                |
| SpaceX Launch fixtures      | **Done**    | Launch Library 2 API                                  |
| Regression compare script   | **Done**    | `tools/regression_compare.py` (odiff-based)           |
| Home Assistant fixtures     | **Done**    | WebSocket auth + 63 entities + state events           |
| Media Control fixtures      | **Next**    | Needs live DLNA/Cast/Kodi for SSDP/mDNS/WS            |
| Capture orchestrator        | **Done**    | `tools/capture_run.py` — all widgets, all variants    |
| Regression comparison test  | **Next**    | End-to-end pipeline validation                        |
| Interaction/click testing   | Not started | Click events in fixture timeline                      |

## Architecture

### RuntimeConfig (single constructor)

All runtime configuration is passed via `RuntimeConfig` before `init()` runs:

```rust
pub struct RuntimeConfig {
    pub fuel_per_frame: u64,
    pub prefs: FormatPreferences,
    pub kv_store_path: Option<PathBuf>,
    pub fetch_interceptor: Option<FetchInterceptor>,
    pub fetch_observer: Option<FetchObserver>,
    pub record_events: bool,
    pub event_fixtures: Vec<FixtureEvent>,
}
```

The runtime provides generic facilities (interceptor/observer hooks). All fixture logic lives in the capture binary —
the runtime has no knowledge of fixtures, recording directories, or fixture file formats.

### Hook types

- **`FetchInterceptor`**: `Box<dyn Fn(&str, &str) -> Option<(u32, Vec<u8>)>>` — intercepts fetch requests by
  `(method, url)`. Returns `Some((status, body))` to serve from fixtures, `None` to proceed to network.
- **`FetchObserver`**: `Box<dyn Fn(&str, u32, &[u8])>` — notified when any fetch response is delivered, with
  `(method_url_key, status, body)`. Used for recording and cycle detection.

### Fetch cycle detection

During `--record-fixtures`, the observer tracks seen URLs in a `HashSet`. When a URL repeats (widget re-polling), a
shared `AtomicBool` flag triggers recording settlement. This means widgets that poll continuously (ISS, SpaceX) finish
recording as soon as all unique URLs have been fetched, instead of waiting for the full wall-clock timeout.

### Fixture file format

`examples/<widget>/fixtures/fetch_responses.json`:

```json
{
  "fetches": {
    "GET https://api.example.com/data": { "status": 200, "body": { "text": "{\"items\": []}" } },
    "GET https://api.example.com/image": { "status": 200, "body": { "b64": "iVBORw0K..." } }
  },
  "events": [
    { "at_ms": 0,   "type": "ssdp_found",   "search_id": 1, "data": { "text": "..." } },
    { "at_ms": 16,  "type": "ws_open",       "ws_id": 1 },
    { "at_ms": 32,  "type": "ws_message",    "ws_id": 1, "data": { "text": "..." } },
    { "at_ms": 300, "type": "ws_close",      "ws_id": 1, "code": 1000 }
  ]
}
```

Flat format (fetches-only, no `"fetches"` wrapper) also supported for backward compatibility.

### Event types

| type               | fields                           | maps to                                   |
| ------------------ | -------------------------------- | ----------------------------------------- |
| `ssdp_found`       | `search_id`, `data`              | `SsdpEvent::Found(json)`                  |
| `ssdp_removed`     | `search_id`, `data`              | `SsdpEvent::Removed(usn)`                 |
| `mdns_found`       | `browse_id`, `data`              | `MdnsEvent::Found(json)`                  |
| `mdns_removed`     | `browse_id`, `data`              | `MdnsEvent::Removed(name)`                |
| `ws_open`          | `ws_id`                          | `WsEvent::Open`                           |
| `ws_message`       | `ws_id`, `data`                  | `WsEvent::Message(bytes)`                 |
| `ws_close`         | `ws_id`, `code`                  | `WsEvent::Close(code)`                    |
| `socket_connected` | `socket_id`                      | `SocketEvent::Connected`                  |
| `socket_data`      | `socket_id`, `data`              | `SocketEvent::Data(bytes)`                |
| `socket_closed`    | `socket_id`, `code`              | `SocketEvent::Closed(code)`               |
| `udp_response`     | `broadcast_id`, `data`, `source` | `UdpBroadcastEvent::Response(data, addr)` |

### capture.toml

```toml
# All fields optional — sensible defaults used when omitted.

time = "2026-03-10T18:00:00"    # Start time for deterministic rendering
settle_delay = 5                 # Extra frames after I/O settles
timeout = 300                    # Settlement timeout in frames (~5s virtual)
record_timeout = 30              # Wall-clock cap for --record-fixtures (seconds, default: 120)
# duration = 2.0                 # Seconds to capture after ready (mutually exclusive with frames)
# capture_every = 3              # Capture every Nth frame during duration mode
# frames = [1, 30, 60]           # Explicit frame numbers to capture

[kv]
ical_url = "fixtures/sample.ics"

[[variants]]
name = "dark"
kv = { theme = "dark" }

[[variants]]
name = "light"
kv = { theme = "light" }
```

## Remaining Work

### 1. Media Control fixtures (needs live services)

The widget uses SSDP discovery, mDNS browsing, WebSocket control, and HTTP fetches. Recording requires at least one of:

- A DLNA/UPnP media renderer on the network
- A Chromecast device
- A Kodi instance
- A Jellyfin/Emby server

All protocol events (SSDP, mDNS, WS, UDP broadcast, TCP socket) are recorded. Discovery events include the search/browse
ID so replay routes them to the correct handler.

### 2. Regression comparison pipeline

End-to-end test of the comparison workflow:

- Run `capture_run.py` to produce `captures/*/current/` for all widgets
- Copy to `captures/*/baselines/`
- Make a small widget change, re-capture
- Run `regression_compare.py` to verify it detects the diff

### 3. Interaction / click testing

Extend the fixture event timeline with click events:

```json
{ "at_ms": 500, "type": "click", "x": 640, "y": 240 }
```

This would allow testing interactive widgets (session picker, settings toggles) by replaying a scripted interaction
sequence. The capture binary would inject click events into the runtime at the specified timestamps.

Not yet designed — needs:

- `FixtureEventKind::Click { x, y }` variant
- Injection via `runtime.handle_click(x, y)` at the right time
- Possibly `FixtureEventKind::Scroll`, `FixtureEventKind::LongPress` for other gestures
