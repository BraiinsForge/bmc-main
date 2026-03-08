# BDK-334: Media Remote Control Widget POC

## Context

POC widget for controlling media playback on LAN devices from the Braiins Deck. **Control only** — no casting/streaming.
The Deck discovers devices and sends commands / reads state. Four protocols: **UPnP/DLNA** (HTTP-based), **Google Cast**
(`TCP+TLS`, `JSON` over thin `protobuf`), **Kodi** (HTTP JSON-RPC 2.0), and **Jellyfin/Emby** (HTTP REST).

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
│   ├── Kodi: HTTP JSON-RPC 2.0 (via host_fetch)
│   │   └── JSON responses parsed via host_json_*
│   └── Jellyfin/Emby: HTTP REST + token auth (via host_fetch)
│       └── JSON responses parsed via host_json_*
├── Discovery
│   ├── mDNS browse (_googlecast._tcp, _upnp._tcp, _xbmc-jsonrpc-h._tcp, _jellyfin._tcp, _emby._tcp)
│   └── SSDP M-SEARCH (urn:schemas-upnp-org:device:MediaRenderer:1)
├── Unified MediaState
│   ├── track: title, artist, album
│   ├── art:   bitmap_id + palette (accent colors)
│   ├── playback: state, position, duration
│   ├── volume: level, muted
│   └── actions: TransportActions (can_play/pause/seek/next/previous)
├── MediaController trait — unified dispatch (protocol.rs)
│   ├── CastAdapter         (zero-sized, delegates to cast::*)
│   ├── KodiAdapter         (zero-sized, delegates to kodi::*)
│   ├── UpnpAdapter         (carries UpnpDevice for SOAP calls)
│   └── EmbyJellyfinAdapter (zero-sized, delegates to emby_jellyfin::*)
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
| 7     | Jellyfin / Emby                | HTTP REST with token auth, shared adapter for both forks, mDNS discovery, session picker modal for multi-session support, `SubTargets` protocol extension |

---

## Known issues

### Kodi credentials

Kodi credentials are now read from an ini config file (no longer hardcoded). May still need widget settings UI for
user-configurable credentials in the future.

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

### Jellyfin / Emby

Test against a running Jellyfin or Emby server on the LAN. Discovered via mDNS (`_jellyfin._tcp` / `_emby._tcp`).
Multi-session support — use the session picker modal to switch between active playback sessions.

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

| Protocol            | Transport                                 | Discovery                           | Complexity | Notes                                                                                              |
| ------------------- | ----------------------------------------- | ----------------------------------- | ---------- | -------------------------------------------------------------------------------------------------- |
| **Roku ECP**        | HTTP (`POST /keypress/*`, `GET /query/*`) | `_roku._tcp` / SSDP                 | Low        | Every Roku device. Pure REST.                                                                      |
| ~~Jellyfin / Emby~~ | ~~HTTP REST + token auth~~                | ~~`_jellyfin._tcp` / `_emby._tcp`~~ | ~~Done~~   | Implemented in `emby_jellyfin.rs` — shared adapter for both, session picker UI, mDNS discovery.    |
| **Spotify Connect** | HTTPS REST (Web API)                      | Implicit (user's active device)     | Medium     | OAuth PKCE auth dance is the hard part. Endpoints themselves are trivial REST. Enormous user base. |
| **MPD**             | Plain-text TCP (line-based key-value)     | Manual config                       | Medium     | Needs `host_tcp_connect` (plain TCP, not TLS). Hugely popular in audiophile/Linux world.           |
| **Plex**            | HTTP REST + auth token                    | GDM (local) / MyPlex (cloud)        | Medium     | Similar to Jellyfin but with Plex-specific auth.                                                   |
| **Volumio**         | WebSocket + REST                          | `_volumio._tcp`                     | Low        | Niche audiophile (Raspberry Pi).                                                                   |

**Recommended order:** Roku → Spotify.

### Accent-tinted background from album art

Implemented via `host_bitmap_sample` (samples average RGBA from a bitmap region) + OkLCH color adjustment in the
`color!` macro. The widget samples the loaded album art bitmap and uses `color!(sampled, lightness: 0.22, chroma: 0.06)`
to produce a dark-tinted accent background.

### 9-patch skinning (SDK-level feature)

Android-style [9-patch](https://developer.android.com/guide/topics/graphics/drawables#nine-patch) support as a global
SDK draw primitive. A 9-patch divides a bitmap into 9 regions (4 corners, 4 edges, 1 center) that scale independently —
corners stay fixed, edges stretch in one axis, center stretches both ways. This lets widgets use rich textured/gradient
UI elements (buttons, panels, cards, backgrounds) from a single bitmap without distortion.

**Why it fits this architecture:**

- Zero shader work — host slices + stretches with existing GPU bitmap drawing
- Widgets stay declarative — just `Draw::nine_patch(x, y, w, h, bitmap_id, insets)`
- Enables visual "skins" — a skin is a set of named 9-patch bitmaps for standard UI elements
- Album art background solved: host generates a blurred 9-patch from the art bitmap at registration time, widget draws
  it as a panel background. No per-frame blur shader needed.

**Protocol extension:**

```
Draw command 0x48: NinePatch
  [x: f32] [y: f32] [w: f32] [h: f32]
  [bitmap_id: u16]
  [left: u16] [top: u16] [right: u16] [bottom: u16]  // inset pixels from each edge
```

Insets define where the corners end and the stretchable regions begin. The host renderer slices the source bitmap into 9
quads and draws each with appropriate UV mapping. FemtoVG already supports sub-rect bitmap drawing, so the host
implementation is straightforward.

**Skin system (future):**

A skin could be a bundle (zip/tar) containing:

- `manifest.json` — maps semantic element names (`button`, `panel`, `slider_track`, etc.) to 9-patch entries
- `*.png` — source bitmaps
- Inset metadata (either embedded in PNG as Android-style 1px border, or declared in manifest)

Widgets reference elements by name (`Draw::skin("button", x, y, w, h)`), host resolves to the active skin's bitmap +
insets. Switching skins = loading a different bundle, zero widget code changes.

**Proof-of-concept test case: Winamp skin.** The classic `.wsz` format (renamed zip) contains all UI elements as BMP
sprite sheets with known fixed pixel layouts. A build-time converter script slices them into individual PNGs and emits a
manifest with insets. The media-control widget rendering a Winamp skin on the Deck display would be a compelling demo.

Resources:

- [Winamp Skin Museum](https://skins.webamp.org/) — 100k+ skins, browsable with preview
- [Webamp source](https://github.com/captbaritone/webamp) — JS reimplementation, skin parser is the definitive format
  reference
- [How Webamp loads skins](https://jordaneldredge.com/how-winamp2-js-loads-native-skins-in-your-browser/) — detailed
  walkthrough of sprite sheet slicing
- [Archive.org collection](https://archive.org/details/winampskins) — bulk download
- [WSZ format spec](http://wiki.winamp.com/wiki/WSZ_Files) — pixel coordinates for each UI element

**Host-integrated skinning design:**

The preferred approach is optional 9-patch skinning on host-side components (buttons, progress bars), not widget-side
reimplementation via touchable canvases. The widget passes skin overrides alongside existing component properties; when
absent, the host renders with default styles (fully backward compatible).

SDK types:

```rust
/// Parsed 9-patch element — bitmap ID + insets defining stretchable regions.
/// Created via `NinePatch::from_png()` which reads the Android-format 1px border
/// markers automatically — developers never specify insets manually.
struct NinePatch {
    bitmap_id: u16,
    left: u16,
    top: u16,
    right: u16,
    bottom: u16,
}

/// Optional skin override for a button.
/// Pressed state falls back to normal (with host-side darkening) when not provided —
/// never falls back to the default solid-color style once a skin is active.
struct ButtonSkin {
    normal: NinePatch,
    pressed: Option<NinePatch>, // None = darken normal 9-patch
    text_color: u32,            // 0 = use default for the style
}

/// Optional skin override for a progress/volume bar.
/// Same fallback rule: missing sub-elements inherit from their parent skin element,
/// not from the default unskinned rendering.
struct BarSkin {
    track_bg: NinePatch,        // background track
    track_fg: NinePatch,        // filled portion
    thumb: Option<NinePatch>,   // seek dot — None = circle (current default)
}

/// Per-widget skin config — built at init from decoded assets.
struct Skin {
    button: Option<ButtonSkin>,
    button_ghost: Option<ButtonSkin>,  // separate skin for ghost buttons
    bar: Option<BarSkin>,
    panel: Option<NinePatch>,          // layout background override
    frame: Option<NinePatch>,          // album art frame
}
```

**Fallback rule:** once a skin element is provided for a component, all states of that component stay within the skin.
For example, if `ButtonSkin.pressed` is `None`, the host darkens the `normal` 9-patch — it never falls back to the
default solid-color `fill_rect`. This avoids jarring visual mixing between skinned and unskinned states.

Widget usage:

```
// Buttons — skin is an optional override, style still controls semantics
button!("Play", icon: play_id, style: Ghost, size: Small, skin: skin.button_ghost)

// Progress bar — host component (not a canvas anymore)
progress_bar!(value: progress, width: w, height: h, skin: skin.bar)

// Layout panel background — 9-patch instead of solid color
row(props!(skin: skin.panel, padding: 16.0, gap: 8.0), [...])

// Direct 9-patch draw on canvas — for custom widget elements
canvas(props!(...), vec![
    Draw::nine_patch(0.0, 0.0, w, h, skin.frame),
])
```

Wire format extension for button (trailing optional payload):

```
[NODE_BUTTON][style:u8][size:u8][icon_id:u16][disabled:u8][label_len:u16][label...]
[has_skin:u8]
  if has_skin != 0:
    [normal_bitmap_id:u16][n_left:u16][n_top:u16][n_right:u16][n_bottom:u16]
    [has_pressed:u8]
      if has_pressed: [pressed_bitmap_id:u16][p_left:u16][p_top:u16][p_right:u16][p_bottom:u16]
    [text_color:u32]
```

Host rendering change: in `draw_button()`, check if skin data is present. If so, draw the 9-patch background instead of
`fill_rect` with the hardcoded style color. When pressed and no pressed 9-patch is provided, apply a darkening tint over
the normal 9-patch (same approach as the current brightness shift for solid colors). All other button logic (icon
positioning, text layout, ellipsis, scissoring, click detection) stays identical.

**SDK draw primitive** (opcode `0x48` — `DRAW_NINE_PATCH`):

```
[DRAW_NINE_PATCH][x:f32][y:f32][w:f32][h:f32][bitmap_id:u16][left:u16][top:u16][right:u16][bottom:u16]
```

Exposed in the SDK as `Draw::nine_patch()` for direct use in canvas draws. Widgets building custom interactive elements
via `touchable()` can use this directly without going through the component system. The host renderer slices the bitmap
into 9 quads with appropriate UV mapping — FemtoVG already supports sub-rect bitmap rendering, so this is
straightforward.

**SDK helper — `NinePatch::from_png()`:**

Parses the Android-format 1px black-pixel border that encodes stretchable regions, strips the border, registers the
inner bitmap via `host::register_bitmap()`, and returns a `NinePatch` with insets populated automatically. The developer
never touches pixel coordinates:

```
let skin = Skin {
    button: Some(ButtonSkin {
        normal: NinePatch::from_png(include_bytes!("skin/button.9.png")),
        pressed: Some(NinePatch::from_png(include_bytes!("skin/button_pressed.9.png"))),
        text_color: WHITE,
    }),
    ..Default::default()
};
```

The parsing logic is ~30 lines (scan top row + left column for black pixel runs). Reference crate:
[`nine_patch_drawable`](https://github.com/kanru/nine_patch_drawable) — but simple enough to implement inline in the SDK
without a dependency.

### Skin system — implementation status

**Extracted into `bmc-wasm-skin` crate** (`bmc-wasm-runtime/skin/`). Current state:

**Continuation prompt:**

> Run the testbed with the media-control widget and verify the Winamp-skinned transport buttons render correctly —
> beveled 9-patch frame with our SVG icons on top, icon colors from `skin.toml` (normal: `#bdced6`, pressed: `#4a5a6b`).
> The skin zip is at `examples/media-control/assets/skins/winamp.zip`, loaded via `include_skin!` at line 27 of
> `examples/media-control/src/lib.rs`. If the colors look wrong, the `wsz-to-skin` converter's fg sampling is unreliable
> (hardcoded pixel offset) — manually edit `skin.toml` inside the zip or improve the sampling logic in
> `skin/tools/src/main.rs`. After visual verification, continue with the "next tasks" below.

**Done:**

- `bmc-wasm-skin` crate — types (`NinePatch`, `NinePatchAsset`, `ButtonSkin`, `Skin`, `SkinAsset`, `SkinEntry`),
  registration (callback-based bitmap registrar), 9-patch parsing utility (`parse_nine_patch_insets`)
- `include_skin!("path.zip")` proc macro — reads zip at compile time, parses `.9.png` borders, reads `skin.toml`
  metadata (per-asset `color` field), emits `Skin` literal
- `include_nine_patch!` refactored to use shared `parse_nine_patch_insets()`
- `skin.toml` metadata format inside skin zips — maps asset names to properties (currently: `color = "#RRGGBB"`)
- `SkinEntry` — resolved asset with `nine_patch` + `color` from metadata
- `ButtonSkin` fields: `normal`, `pressed`, `text_color`, `pressed_text_color`, `opaque`
- Host button renderer: per-state text color, `opaque` flag skips icon/label rendering
- `wsz-to-skin` Rust CLI tool (`skin/tools/`) — extracts generic button frame from Winamp `.wsz`, clears center symbol,
  generates proper 9-patch with bevel-aware insets, samples fg colors, writes `skin.toml`
- Media-control widget: transport buttons skinned with Winamp button frame + our SVG icons, colors from `skin.toml`

**In progress / next:**

- Verify icon colors render correctly with the sampled values from `skin.toml`
- Improve `wsz-to-skin` fg color sampling (currently hardcoded offset, unreliable for some skins)
- Consider adding `BarSkin` for progress/volume bars (track bg, track fg, thumb)
- Consider `panel` / `frame` 9-patch assets for layout backgrounds and album art frames
- Extract more sprite sheets from `.wsz` (TITLEBAR.BMP, MAIN.BMP, POSBAR.BMP, etc.)

**File layout:**

```
bmc-wasm-runtime/
├── skin/                    # bmc-wasm-skin crate (types + parsing, zero deps, WASM-safe)
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── tools/               # wsz-to-skin converter (standalone binary crate)
│       ├── Cargo.toml
│       └── src/main.rs
├── sdk-macros/               # include_skin!, include_nine_patch! (build-time, depends on skin + image + zip + toml)
├── sdk/                      # re-exports skin types, init callback in begin_tree()
└── src/components/button.rs  # host renderer: 9-patch bg, per-state text color, opaque flag
```

**Skin zip format:**

```
my-skin.zip
├── button_normal.9.png    # 9-patch with 1px border (stretch markers for center only)
├── button_pressed.9.png
└── skin.toml              # per-asset metadata
```

```toml
[button_normal]
color = "#bdced6"   # fg/icon color for normal state (RRGGBB)

[button_pressed]
color = "#4a5a6b"   # fg/icon color for pressed state
```

**Widget usage:**

```
const SKIN: Skin = include_skin!("assets/skins/winamp.zip");

let normal = SKIN.get("button_normal");  // → SkinEntry { nine_patch, color }
let pressed = SKIN.get("button_pressed");
let skin = Some(ButtonSkin {
    normal: normal.nine_patch,
    pressed: Some(pressed.nine_patch),
    text_color: normal.color,
    pressed_text_color: pressed.color,
    opaque: false,
});
button!("", icon: play_id, style: Ghost, size: Small, skin: skin)
```

### Shader escape hatch

`host_shader_compile(src) / host_shader_run(shader_id, inputs)` for GPU-accelerated compute. Worth exploring separately.

### Componentisation

Extract reusable UI fragment helpers (controls row, progress row, album art, track info) to reduce size-variant
duplication once visual design stabilises.
