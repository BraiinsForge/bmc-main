# BDK-334: Media Remote Control Widget POC

## Context

POC widget for controlling media playback on LAN devices from the Braiins Deck. **Control only** — no casting/streaming.
The Deck discovers devices and sends commands / reads state. Three protocols for multi-protocol demo: **UPnP/DLNA**
(HTTP-based), **Google Cast** (`TCP+TLS`, `JSON` over thin `protobuf`), and **DACP** (HTTP + binary DMAP).

Design philosophy: **generic host primitives, all protocol logic in WASM** — same as the Home Assistant WebSocket demo.
The host provides `TCP`, `TLS`, `HTTP` methods, `mDNS`, `KV`, `HTTP listener`. The WASM widget handles `SOAP/XML`,
`CastV2` framing, `DMAP` parsing, `JSON` commands, UI.

Visual goal: YouTube Music style — large album art, accent-tinted dark background (colors extracted from the artwork),
clean playback controls. Icons from Carbon Design System (already available locally).

---

## Architecture

```
Widget (WASM)
├── Protocols (all in WASM)
│   ├── UPnP: SOAP over HTTP POST (via host_fetch)
│   │   └── XML responses parsed via host_xml_*
│   ├── Cast: prost-encoded CastMessage over host_tls_connect
│   │   └── JSON payloads parsed via host_json_*
│   └── DACP: HTTP GET/POST (via host_fetch)
│       └── Binary DMAP responses parsed in WASM
├── Color extraction
│   └── palette_extract on RGBA pixels returned by host_decode_image
├── Discovery
│   └── mDNS browse (_googlecast._tcp, _touch-able._tcp, _upnp._tcp)
│       └── UPnP devices register via mDNS as bridge (no native SSDP)
├── Unified MediaState
│   ├── track: title, artist, album
│   ├── art:   bitmap_id + palette (accent colors)
│   ├── playback: state, position, duration
│   └── volume: level, muted
└── UI (tree API + canvas)
    ├── Device picker (mDNS discovery + device list)
    ├── Album art + accent-tinted background
    ├── Playback controls (prev/play-pause/next)
    ├── Progress bar + seek
    ├── Volume control
    └── Device switcher button

Host (native, generic primitives)
├── host_fetch — POST/PUT/DELETE/GET with headers + body
├── host_tls_connect(host, port) → socket events
├── host_socket_write / host_socket_close
├── host_xml_parse / host_xml_get_* / host_xml_free
├── host_decode_image → returns RGBA pixels to WASM
├── host_mdns_browse / host_mdns_register
├── host_kv_set / host_kv_get / host_kv_delete
├── host_http_listen / host_http_respond
└── [existing] host_ws_*, host_json_*, host_log, host_register_bitmap
```

---

## Stage 0 — Research archive ✅

Plan saved to `research/plan.md`. UI reference image at `research/ui-example-1.png`.

---

## Stage 1 — Host function extensions ✅

All four sub-stages implemented and passing `make validate-wasm`.

### 1a. Extend `host_fetch` with HTTP method + body ✅

SDK got `FetchRequest` builder with `get()`, `post()`, `put()`, `delete()`, `.headers()`, `.body()`, `.send()`,
`.send_after()`. Backward-compatible. Host side splits ureq v3 `WithBody` vs `WithoutBody`. All existing widgets compile
unchanged.

### 1b. TCP+TLS socket host functions ✅

Uses `rustls` with `NoCertVerifier` for self-signed certs (Chromecast). Background thread per connection with mpsc
channel for event delivery. SDK provides `tls_connect()`, `Socket` (write/close), `SocketEvent` enum.

### 1c. Host-side XML parsing ✅

Uses `roxmltree`. Simplified XPath: `//local_name` for text content, `//local_name/@attr` for attributes.
Namespace-agnostic matching. `xml_docs: HashMap<u32, String>` stores raw XML, re-parsed per query.

### 1d. Runtime bitmap + host_decode_image ✅

`Draw::bitmap_id(x, y, w, h, id)` for pre-registered bitmap IDs. `host::decode_image()` uses two-call pattern.
`host::register_bitmap()` exposed as public SDK fn.

---

## Stage 2 — UPnP/DLNA controller + UI ✅

All protocol logic in WASM. SOAP over HTTP POST. XML responses parsed via `XmlDoc`. Full UI with album art, transport
controls, progress bar, volume, seek, disconnect detection.

**Files:** `src/lib.rs`, `src/upnp.rs`, `src/protocol.rs`, `src/icons.rs`

**Features:** Play, Pause, Stop, Next, Previous, GetTransportInfo, GetPositionInfo, GetVolume, SetVolume, GetMute,
SetMute. Poll every 1s (playing) / 3s (idle). Disconnect detection (3 failures threshold). Button clicks (index-based),
touch interactions (key-based `TouchHit` with `frac_x()`), drag support for volume/seek bars, album art aspect ratio,
responsive layout (Full/Large/Medium/Small).

---

## Stage 3 — Google Cast controller ✅

Composition approach — prost-build on Chromium's `cast_channel.proto` for the routing envelope, JSON payloads via
`host_json_*`, host TLS socket for transport.

**Files:** `src/cast.rs`, `build.rs`, `proto/cast_channel.proto`

**Features:** TLS connect, CastV2 framing (4-byte BE length prefix), heartbeat (PING/PONG every 5s), receiver status,
media session management, play/pause/next/prev/seek/volume. Status updates via push (`MEDIA_STATUS` broadcasts) + pull
(`GET_STATUS`).

---

## Stage 4 — Host primitives for DACP + Discovery ✅

Three new host primitives, each following the established socket pattern.

### 4a. mDNS host primitive ✅

**Crate:** `mdns-sd`. Browse multiple service types, receive Found/Removed events as JSON. Register services with TXT
records. SDK: `mdns_browse()`, `mdns_register()`, `MdnsBrowse`, `MdnsRegistration`.

### 4b. KV persistence ✅

Per-widget file-backed key-value store. Synchronous host functions. SDK: `kv_set()`, `kv_get()`, `kv_get_string()`,
`kv_delete()`. Two-call pattern for reads. Used for DACP pairing GUIDs, device preferences.

### 4c. HTTP listener ✅

Inbound HTTP for DACP pairing flow. Background thread with `TcpListener`, per-request response channels. SDK:
`http_listen()`, `HttpRequest.respond()`, `HttpListener.close()`.

### 4d. DMAP parser + DACP protocol ✅

**Files:** `src/dmap.rs`, `src/dacp.rs`

DMAP binary TLV parser (~150 lines). DACP protocol module with pairing flow (mDNS registration as `_touch-remote._tcp`,
HTTP listener for pairing callback, PIN verification, GUID persistence), session management (login, long-poll status
updates), playback commands (playpause, nextitem, previtem, setproperty).

**Note:** DACP (`_touch-able._tcp`) is effectively dead on modern macOS — Apple Music no longer advertises it. The
implementation is correct but untestable without legacy iTunes. AirPlay (the actual modern protocol) uses a completely
different stack.

---

## Stage 5 — Device discovery + picker UI ✅

### State machine

```rust
enum WidgetState {
    Discovering,                                       // device picker
    Pairing { device: DiscoveredDevice, pin: String }, // DACP pairing
    Connected(MediaState),
    Disconnected(MediaState),                          // reconnecting
}
```

### mDNS discovery

Browses `_googlecast._tcp`, `_touch-able._tcp`, `_upnp._tcp` simultaneously. UPnP devices register themselves via mDNS
as a bridge (native UPnP uses SSDP which we don't implement). Deduplicates by `service_name`. Extracts display names
from TXT records.

### Device picker UI

- **Empty state:** "Searching for devices..." with animated squiggle loader, discovery log (non-Small sizes)
- **Device list:** Scrollable list of discovered devices with protocol icon + name. Discovery log on right (non-Small).
- **Responsive:** Full/Large = 24px title, 16px names; Medium/Small = compact; Small hides discovery log.

### Connection flow

- UPnP → create `UpnpDevice`, store in thread-local, transition to `Disconnected`, kick off SOAP polling
- Cast → `cast::connect()`, transition to `Disconnected`
- DACP with stored GUID → `dacp::connect()`, transition to `Disconnected`
- DACP without GUID → `dacp::start_pairing()`, transition to `Pairing`

### Device switcher

Connected/disconnected UI has a switcher button (protocol icon + device name). Tapping disconnects and returns to
picker. Discovery log shows live mDNS events while browsing.

### Auto-reconnect

Persists last connected device via `kv::set("last_device", service_name)`. On startup, if the stored device appears in
discovery results, auto-connects.

---

## Known issues

### `request_frame_after()` broken by animation-only loop

**Priority: High — blocks Cast heartbeat, progress updates, and any periodic WASM logic.**

The tree processing code unconditionally overrides `animation_only_frame = true` whenever the tree has active animations
and no user interaction. This overrides `request_frame_after()` which WASM code uses for periodic timers. The result is
that WASM render is never called again after the initial burst — heartbeats never fire, progress freezes.

**Proposed fix:** Add `deferred_wasm_render_ms` countdown to `HostState` that counts down on animation frames and forces
a full WASM render when expired.

### Volume setting glitchy (Cast)

Stale in-flight responses overwrite optimistic updates. The 4-tile testbed makes it worse (4 independent connections to
same device). Needs proper debounce or single-connection mode for testing.

### Volume bar fill broken in small widget layout

The flex:1.0 layout produces a different actual width than the computed fill width. The oversized-background approach
works for full/compact but breaks for stacked.

### Volume doesn't affect audio (Python test server)

`ffplay` doesn't accept runtime volume commands. The UPnP SOAP responses correctly track volume state, but the actual
audio output doesn't change. This is a limitation of the test server, not the widget.

### Next/Previous no-ops (Python test server)

The Python UPnP renderer is single-track — it has no playlist, so Next/Previous actions have nothing to skip to. Real
DLNA renderers connected to media servers with queues would support them.

---

## Testing setup

### Built-in Python UPnP renderer

The test script at `tools/dlna-push.py` includes a fully built-in UPnP/DLNA renderer — no external dependencies like
`gmrender-resurrect`. Uses `ffplay` for audio playback (falls back to `mpv`, then `afplay`).

```bash
# Start renderer with example content
make serve          # Pop/rock playlist
make serve-music    # Music examples
make serve-classical # Classical examples
make serve-radio    # Internet radio streams

# Push a URL to running renderer
make push URL=https://example.com/stream.mp3

# Spawn empty renderer (no content)
make spawn

# Check renderer status
make status
```

The renderer:

- Binds to `0.0.0.0` on port 49494 (configurable via `PORT`)
- Registers via mDNS as `_upnp._tcp` using `dns-sd -R` so the widget discovers it
- Handles AVTransport + RenderingControl SOAP actions
- Sends SSDP announcements on startup
- Plays audio via `ffplay -nodisp -autoexit -loglevel quiet`

### Google Cast

No emulator available. Test against real Chromecast hardware or build a mock Cast server (Rust, ~300-500 lines, fakes
port 8009 TLS + CastV2 protocol).

### Running the widget

```bash
cd bmc-wasm-runtime
make run EXAMPLE=media-control ARGS="--address 0.0.0.0:6070"
```

---

## Future exploration

### Accent-tinted background from album art

1. `decode_image(bytes)` → RGBA pixels
2. `palette_extract` in WASM → 3 dominant colors
3. Dark-tint dominant: HSL — saturation ×0.35, lightness →0.12
4. Use as background color

### Shader escape hatch

`host_shader_compile(src) / host_shader_run(shader_id, inputs)` for GPU-accelerated compute. Worth exploring separately.

### Componentisation

Extract reusable UI fragment helpers (controls row, progress row, album art, track info) to reduce size-variant
duplication once visual design stabilises.
