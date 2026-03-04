# BDK-334: Media Remote Control Widget POC

## Context

POC widget for controlling media playback on LAN devices from the Braiins Deck. **Control only** — no casting/streaming.
The Deck discovers devices and sends commands / reads state. Two protocols: **UPnP/DLNA** (HTTP-based) and **Google
Cast** (`TCP+TLS`, `JSON` over thin `protobuf`).

Design philosophy: **generic host primitives, all protocol logic in WASM** — same as the Home Assistant WebSocket demo.
The host provides `TCP`, `TLS`, `HTTP` methods, `mDNS`, `KV`, `HTTP listener`. The WASM widget handles `SOAP/XML`,
`CastV2` framing, `JSON` commands, UI.

Visual goal: YouTube Music style — large album art, accent-tinted dark background (colors extracted from the artwork),
clean playback controls. Icons from Carbon Design System (already available locally).

---

## Architecture

```
Widget (WASM)
├── Protocols (all in WASM)
│   ├── UPnP: SOAP over HTTP POST (via host_fetch)
│   │   └── XML responses parsed via host_xml_*
│   └── Cast: prost-encoded CastMessage over host_tls_connect
│       └── JSON payloads parsed via host_json_*
├── Color extraction
│   └── palette_extract on RGBA pixels returned by host_decode_image
├── Discovery
│   └── mDNS browse (_googlecast._tcp, _upnp._tcp)
│       └── UPnP devices register via mDNS as bridge (no native SSDP)
├── Unified MediaState
│   ├── track: title, artist, album
│   ├── art:   bitmap_id + palette (accent colors)
│   ├── playback: state, position, duration
│   ├── volume: level, muted
│   └── actions: TransportActions (can_play/pause/seek/next/previous)
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

## Stage 4 — Host primitives for Discovery ✅

Three new host primitives, each following the established socket pattern.

### 4a. mDNS host primitive ✅

**Crate:** `mdns-sd`. Browse multiple service types, receive Found/Removed events as JSON. Register services with TXT
records. SDK: `mdns_browse()`, `mdns_register()`, `MdnsBrowse`, `MdnsRegistration`.

### 4b. KV persistence ✅

Per-widget file-backed key-value store. Synchronous host functions. SDK: `kv_set()`, `kv_get()`, `kv_get_string()`,
`kv_delete()`. Two-call pattern for reads. Used for device preferences and auto-reconnect.

### 4c. HTTP listener ✅

Inbound HTTP server primitive. Background thread with `TcpListener`, per-request response channels. SDK:
`http_listen()`, `HttpRequest.respond()`, `HttpListener.close()`.

---

## Stage 5 — Device discovery + picker UI ✅

### State machine

```rust
enum WidgetState {
    Discovering,                      // device picker
    Connected(MediaState),
    Disconnected(MediaState),         // reconnecting
}
```

### mDNS discovery

Browses `_googlecast._tcp`, `_upnp._tcp` simultaneously. UPnP devices register themselves via mDNS as a bridge (native
UPnP uses SSDP which we don't implement). Deduplicates by `service_name`. Extracts display names from TXT records.

### Device picker UI

- **Empty state:** "Searching for devices..." with animated squiggle loader, discovery log (non-Small sizes)
- **Device list:** Scrollable list of discovered devices with protocol icon + name. Discovery log on right (non-Small).
- **Responsive:** Full/Large = 24px title, 16px names; Medium/Small = compact; Small hides discovery log.

### Connection flow

- UPnP → create `UpnpDevice`, store in thread-local, transition to `Disconnected`, kick off SOAP polling
- Cast → `cast::connect()`, transition to `Disconnected`

### Device switcher

Connected/disconnected UI has a switcher button (protocol icon + device name). Tapping disconnects and returns to
picker. Discovery log shows live mDNS events while browsing.

### Auto-reconnect

Persists last connected device via `kv::set("last_device", service_name)`. On startup, if the stored device appears in
discovery results, auto-connects.

---

## Stage 6 — Kodi protocol + unified protocol dispatch

### Context

The media control widget has 3 protocols (UPnP, Cast, Kodi) but every command dispatches through
`match proto { Cast => ..., Kodi => ..., Upnp => ... }` in ~7 places across lib.rs. Adding each new protocol multiplies
these dispatch sites. The `MediaController` trait in protocol.rs was designed to solve this but was never wired up. This
stage adds Kodi AND unifies all dispatch behind the trait, so future protocols require edits in only one place.

### Design

#### Redesigned `MediaController` trait (`protocol.rs`)

Replace the current unused trait with what lib.rs actually needs:

```rust
pub trait MediaController {
    // Lifecycle
    fn disconnect(&self);
    fn is_alive(&self) -> bool;
    fn is_connected(&self) -> bool;
    fn tick(&self, delta_ms: u32);

    // Commands (fire-and-forget)
    fn play(&self);
    fn pause(&self);
    fn next(&self);
    fn previous(&self);
    fn seek(&self, position_secs: u32, duration_secs: u32);
    fn set_volume(&self, level: f32);   // 0.0–1.0
    fn set_mute(&self, muted: bool);

    // Protocol metadata
    fn poll_interval_playing(&self) -> u32;
    fn poll_interval_idle(&self) -> u32;
    fn protocol_name(&self) -> &'static str;
    fn protocol_icon(&self) -> &'static Icon;
}
```

Key decisions:

- **`&self` not `&mut self`** — mutations happen inside protocol modules through their own thread-locals (Cast, Kodi) or
  are stateless HTTP (UPnP)
- **`seek(position_secs, duration_secs)`** — caller always has both; Cast/UPnP use position, Kodi computes percentage
  from both
- **No poll/status methods** — each protocol pushes status updates through its own callback (registered once at connect
  time); no per-poll-type methods
- **No `connect()`** — construction is protocol-specific (different args); trait starts after connection

#### Adapter structs in `lib.rs`

Three thin adapters, each ~20 lines, implementing the trait by delegating to module functions:

```rust
struct CastAdapter;                        // zero-sized, delegates to cast::*
struct KodiAdapter;                        // zero-sized, delegates to kodi::*
struct UpnpAdapter { device: UpnpDevice }  // carries device ref for SOAP calls
```

Why in lib.rs (not in protocol modules): UPnP commands need callback functions (`on_command_response`, `on_volume_set`,
`on_mute_set`) that live in lib.rs. Putting adapters here avoids callback-plumbing indirection.

#### Thread-local changes in `lib.rs`

**Remove:**

- `PROTOCOL: Cell<ActiveProtocol>` — replaced by controller
- `DEVICE: RefCell<Option<UpnpDevice>>` — moved into `UpnpAdapter`
- `ActiveProtocol` enum — no longer needed

**Add:**

- `CONTROLLER: RefCell<Option<Box<dyn MediaController>>>` — the single dispatch point

**Helper:**

```rust
fn with_controller(f: impl FnOnce(&dyn MediaController)) {
    CONTROLLER.with(|c| {
        if let Some(ctrl) = c.borrow().as_deref() { f(ctrl); }
    });
}
```

#### Dispatch unification in `lib.rs`

Every `match proto { Cast => ..., Kodi => ..., Upnp => ... }` block becomes a single trait method call:

| Current (7 dispatch sites)                | After                                                                 |
| ----------------------------------------- | --------------------------------------------------------------------- |
| `disconnect_and_return_to_picker()` match | `with_controller(\|c\| c.disconnect())`                               |
| `render()` tick block (3 separate ifs)    | Single `with_controller` block: `c.tick()`, `c.is_alive()`, intervals |
| Volume bar touch match                    | `with_controller(\|c\| c.set_volume(frac))`                           |
| Click handler match → N handle\_\*\_click | Single `handle_media_click(ctrl, index)`                              |
| `seek_to_fraction()` match                | `with_controller(\|c\| c.seek(pos, dur))`                             |
| Device picker icon match                  | `c.protocol_icon()` from stored controller                            |

**Removed functions:** `handle_cast_click`, `adjust_cast_volume`, `handle_kodi_click`, `adjust_kodi_volume` — replaced
by unified `handle_media_click` and `adjust_volume_by_delta`.

#### What stays protocol-specific

Status callbacks remain per-protocol (registered once in `connect_to_device`):

- `on_cast_status(status: &CastMediaStatus)` — maps Cast status → MediaState
- `on_kodi_status(status: &KodiMediaStatus)` — maps Kodi status → MediaState (new)
- `on_position_info`, `on_volume`, etc. — UPnP's callback-chain polling

This is pragmatic: unifying status callbacks would require UPnP to aggregate multiple SOAP responses internally — a
deeper refactor deferred to a future stage.

#### UPnP adapter specifics

- `tick()` — no-op (polling driven by response callback chains, as before)
- `is_alive()` — always returns `true` (UPnP manages alive state through `record_failure()`/`reset_failures()` in lib.rs
  response callbacks)

#### `connect_to_device()` — the ONE place that knows about protocols

```rust
fn connect_to_device(device: &DiscoveredDevice) {
    // ... persist selection, set device name ...
    let controller: Box<dyn MediaController> = match device.protocol {
        DiscoveredProtocol::Cast => {
            cast::connect(&device.host, device.port, on_cast_status);
            Box::new(CastAdapter)
        }
        DiscoveredProtocol::Kodi => {
            kodi::connect(&device.host, device.port, on_kodi_status);
            Box::new(KodiAdapter)
        }
        DiscoveredProtocol::Upnp => {
            let adapter = UpnpAdapter { device: make_upnp_device(device) };
            // Kick off initial UPnP polls...
            Box::new(adapter)
        }
    };
    CONTROLLER.set(Some(controller));
    STATE.set(Disconnected(MediaState::default()));
}
```

Adding protocol N+1: create module, write adapter (~20 lines), add one arm here + mDNS service type.

### Files

| File              | Action                                           | Lines changed (est.) |
| ----------------- | ------------------------------------------------ | -------------------- |
| `src/kodi.rs`     | Already created in prior stage                   | —                    |
| `src/icons.rs`    | Already edited (PROTO_KODI) in prior stage       | —                    |
| `src/protocol.rs` | Rewrite `MediaController` trait, remove unused   | ~40                  |
| `src/lib.rs`      | Adapters, unified dispatch, remove N-way matches | ~200 net             |

### Verification

```bash
cd /Users/kubijo/dev/braiins/forge/bmc-main
make validate-wasm    # clippy + tests + all WASM examples
```

Test against real Kodi: enable "Allow remote control via HTTP" in Settings → Services → Web server (default port 8080).

**Status:** Not Started

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

**Note:** The widget now queries `GetCurrentTransportActions` and disables skip buttons when the renderer reports
Next/Previous unavailable. The Python server likely doesn't implement this action — the default is all-capable until
told otherwise, so buttons remain enabled against the test server.

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

### Additional protocols

Candidates for multi-protocol expansion, ordered by complexity and reach:

| Protocol            | Transport                                 | Discovery                              | Complexity | Notes                                                                                                                                                                                      |
| ------------------- | ----------------------------------------- | -------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Kodi**            | HTTP JSON-RPC (POST/WebSocket)            | `_xbmc-jsonrpc-h._tcp`                 | Low        | Full transport, library, metadata, art URLs. All host primitives exist.                                                                                                                    |
| **Roku ECP**        | HTTP (`POST /keypress/*`, `GET /query/*`) | `_roku._tcp` / SSDP                    | Low        | Every Roku device. Pure REST.                                                                                                                                                              |
| **Jellyfin / Emby** | HTTP REST + token auth                    | `_jellyfin._tcp` / `_emby._tcp` / SSDP | Low-medium | Jellyfin is open-source fork of Emby (closed-source since 2018). Session control APIs are ~90% identical — one implementation covers both. Jellyfin has better docs and growing community. |
| **Spotify Connect** | HTTPS REST (Web API)                      | Implicit (user's active device)        | Medium     | OAuth PKCE auth dance is the hard part. Endpoints themselves are trivial REST. Enormous user base.                                                                                         |
| **MPD**             | Plain-text TCP (line-based key-value)     | Manual config                          | Medium     | Needs `host_tcp_connect` (plain TCP, not TLS). Hugely popular in audiophile/Linux world.                                                                                                   |
| **Plex**            | HTTP REST + auth token                    | GDM (local) / MyPlex (cloud)           | Medium     | Similar to Jellyfin but with Plex-specific auth.                                                                                                                                           |
| **Volumio**         | WebSocket + REST                          | `_volumio._tcp`                        | Low        | Niche audiophile (Raspberry Pi).                                                                                                                                                           |

**Recommended order:** Kodi → Roku → Jellyfin/Emby → Spotify.

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
