# BDK-334: Media Remote Control Widget POC

## Context

POC widget for controlling media playback on LAN devices from the Braiins Deck. **Control only** — no casting/streaming.
The Deck discovers devices and sends commands / reads state. Three protocols: **UPnP/DLNA** (HTTP-based), **Google
Cast** (`TCP+TLS`, `JSON` over thin `protobuf`), and **Kodi** (HTTP JSON-RPC 2.0).

Design philosophy: **generic host primitives, all protocol logic in WASM** — same as the Home Assistant WebSocket demo.
The host provides `TCP`, `TLS`, `HTTP` methods, `mDNS`, `SSDP`, `KV`, `HTTP listener`. The WASM widget handles
`SOAP/XML`, `CastV2` framing, `JSON` commands, UI.

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
│   └── Kodi: HTTP JSON-RPC 2.0 (via host_fetch)
│       └── JSON responses parsed via host_json_*
├── Discovery
│   ├── mDNS browse (_googlecast._tcp, _upnp._tcp, _xbmc-jsonrpc-h._tcp)
│   └── SSDP M-SEARCH (urn:schemas-upnp-org:device:MediaRenderer:1)
├── Unified MediaState
│   ├── track: title, artist, album
│   ├── art:   bitmap_id + palette (accent colors)
│   ├── playback: state, position, duration
│   ├── volume: level, muted
│   └── actions: TransportActions (can_play/pause/seek/next/previous)
├── MediaController trait — unified dispatch (protocol.rs)
│   ├── CastAdapter   (zero-sized, delegates to cast::*)
│   ├── KodiAdapter   (zero-sized, delegates to kodi::*)
│   └── UpnpAdapter   (carries UpnpDevice for SOAP calls)
└── UI (tree API + canvas)
    ├── Device picker (discovery + device list)
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
├── host_ssdp_search → device description XML parsing
├── host_kv_set / host_kv_get / host_kv_delete
├── host_http_listen / host_http_respond
└── [existing] host_ws_*, host_json_*, host_log, host_register_bitmap
```

### State machine

```rust
enum WidgetState {
    Discovering,                      // device picker
    Connected(MediaState),
    Disconnected(MediaState),         // reconnecting
}
```

### Protocol dispatch

Single thread-local `CONTROLLER: RefCell<Option<Box<dyn MediaController>>>`. All commands go through
`with_controller(|c| c.method())`. Adding protocol N+1: create module, write adapter (~20 lines), add one arm in
`connect_to_device()` + mDNS service type.

Status callbacks remain per-protocol (registered once at connect time) — UPnP uses callback-chain polling across
multiple SOAP responses, Cast receives push broadcasts, Kodi aggregates two-phase polling results.

---

## Implementation history

All stages complete. UI reference image at `research/ui-example-1.png`.

| Stage | What                           | Key details                                                                                                                                               |
| ----- | ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0     | Research & architecture        | Protocol survey, architecture design                                                                                                                      |
| 1     | Host function extensions       | `FetchRequest` builder (1a), TLS sockets with `rustls`/`NoCertVerifier` (1b), XML parsing via `roxmltree` (1c), runtime bitmap + `host_decode_image` (1d) |
| 2     | UPnP/DLNA controller + UI      | SOAP/XML protocol, full transport controls, responsive layout (Full/Large/Medium/Small), album art, seek/volume bars, disconnect detection                |
| 3     | Google Cast controller         | CastV2 over TLS, prost-encoded framing, heartbeat, receiver/media session state machine, push+pull status                                                 |
| 4     | Host discovery primitives      | mDNS via `mdns-sd` (4a), KV persistence — file-backed per-widget store (4b), HTTP listener (4c)                                                           |
| 5     | Device discovery + picker UI   | mDNS browse 3 service types, SSDP M-SEARCH with device description XML, device picker, auto-reconnect via KV, per-tile KV isolation                       |
| 6     | Kodi + `MediaController` trait | HTTP JSON-RPC 2.0 with Basic Auth, two-phase polling, `MediaController` trait unifying dispatch across all 3 protocols                                    |

---

## Known issues

### Kodi credentials hardcoded

The Kodi controller uses hardcoded `kodi:kodi` Basic Auth (`KODI_PASSWORD` constant in `kodi.rs`). Needs
user-configurable credentials: widget settings UI input, pass to `kodi::connect()`, dynamic `Authorization` header.
Should allow empty password for Kodi instances with auth disabled.

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

### Test server limitations (not widget bugs)

- **Volume doesn't affect audio** — `ffplay` doesn't accept runtime volume commands. SOAP responses correctly track
  volume state.
- **Next/Previous no-ops** — single-track renderer has no playlist. Widget queries `GetCurrentTransportActions` and
  disables skip buttons when unavailable, but the Python server doesn't implement this action.

---

## Testing setup

### Built-in Python UPnP renderer

The test script at `tools/dlna-push.py` — no external dependencies (stdlib only, Python 3.10+). Uses `ffplay` for audio
playback (falls back to `mpv`, then `afplay`).

```bash
make serve            # Pop/rock playlist
make serve-music      # Music examples
make serve-classical  # Classical examples
make serve-radio      # Internet radio streams
make spawn            # Empty renderer (no content)
make push URL=...     # Push URL to running renderer
make status           # Check renderer state
```

The renderer binds to `0.0.0.0:49494` (configurable via `PORT`), registers via mDNS (`dns-sd -R`) and sends SSDP
announcements, handles AVTransport + RenderingControl SOAP actions.

### Google Cast

No emulator available. Test against real Chromecast hardware.

### Kodi

Enable "Allow remote control via HTTP" in Settings → Services → Web server (default port 8080, default creds
`kodi:kodi`).

### Running the widget

```bash
cd bmc-wasm-runtime
make run EXAMPLE=media-control
```

### Debug layout outlines

Toggle colored outlines around every layout node to diagnose spacing issues. Either set `DEBUG_LAYOUT=1` env var at
startup, or click the **Debug layout** button in the testbed stats panel at runtime.

---

## Future exploration

### Additional protocols

| Protocol            | Transport                                 | Discovery                              | Complexity | Notes                                                                                                                                                                                      |
| ------------------- | ----------------------------------------- | -------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Roku ECP**        | HTTP (`POST /keypress/*`, `GET /query/*`) | `_roku._tcp` / SSDP                    | Low        | Every Roku device. Pure REST.                                                                                                                                                              |
| **Jellyfin / Emby** | HTTP REST + token auth                    | `_jellyfin._tcp` / `_emby._tcp` / SSDP | Low-medium | Jellyfin is open-source fork of Emby (closed-source since 2018). Session control APIs are ~90% identical — one implementation covers both. Jellyfin has better docs and growing community. |
| **Spotify Connect** | HTTPS REST (Web API)                      | Implicit (user's active device)        | Medium     | OAuth PKCE auth dance is the hard part. Endpoints themselves are trivial REST. Enormous user base.                                                                                         |
| **MPD**             | Plain-text TCP (line-based key-value)     | Manual config                          | Medium     | Needs `host_tcp_connect` (plain TCP, not TLS). Hugely popular in audiophile/Linux world.                                                                                                   |
| **Plex**            | HTTP REST + auth token                    | GDM (local) / MyPlex (cloud)           | Medium     | Similar to Jellyfin but with Plex-specific auth.                                                                                                                                           |
| **Volumio**         | WebSocket + REST                          | `_volumio._tcp`                        | Low        | Niche audiophile (Raspberry Pi).                                                                                                                                                           |

**Recommended order:** Roku → Jellyfin/Emby → Spotify.

### Accent-tinted background from album art

Implemented via `host_bitmap_sample` (samples average RGBA from a bitmap region) + OkLCH color adjustment in the
`color!` macro. The widget samples the loaded album art bitmap and uses `color!(sampled, lightness: 0.18, chroma: 0.04)`
to produce a dark-tinted accent background.

### Shader escape hatch

`host_shader_compile(src) / host_shader_run(shader_id, inputs)` for GPU-accelerated compute. Worth exploring separately.

### Componentisation

Extract reusable UI fragment helpers (controls row, progress row, album art, track info) to reduce size-variant
duplication once visual design stabilises.
