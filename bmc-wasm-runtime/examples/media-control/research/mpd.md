# MPD Protocol Support for Media-Control Widget

## Context

Adding MPD (Music Player Daemon) protocol as the 5th media protocol in the media-control widget. MPD uses plain TCP
(port 6600, line-based text protocol), which the host runtime doesn't support yet — only TLS sockets exist
(`host_tls_connect` for Cast). OwnTone server at `nas.lan:6600` is the test target. The `mpd` / `mpd_protocol` crates
are all socket-coupled and won't work in WASM; the protocol is simple enough to implement by hand (~150 lines of line
parsing).

---

## Stage 1: `host_tcp_connect` — Plain TCP Host Primitive

**Goal**: Add plain TCP socket support, reusing all existing socket infrastructure.

### Files to modify

**`bmc-wasm-runtime/src/runtime_wasmi.rs`**:

- Add `tcp_background_thread(socket_id, host, port, event_tx, write_rx)` — copy of `tls_background_thread` (line ~2928)
  with all rustls code removed. Just `TcpStream::connect` → `set_read_timeout(50ms)` → `Connected` event → same
  drain-writes/read loop
- Add `host_tcp_connect` linker binding (next to `host_tls_connect` at line ~842) — identical structure, spawns
  `tcp_background_thread` instead

**`bmc-wasm-runtime/sdk/src/socket.rs`**:

- Add `extern fn host_tcp_connect(host_ptr, host_len, port) -> u32` to the extern block
- Add `pub fn tcp_connect(host, port, callback) -> Socket` — twin of `tls_connect`, calls `host_tcp_connect`

### What stays unchanged

- `SocketEvent`, `SocketOutbound`, `ActiveSocket` types — shared between TLS and TCP
- `host_socket_write`, `host_socket_close` — protocol-agnostic
- `deliver_socket_events()` — doesn't care about transport type
- `Socket` struct, `__on_socket_event` dispatch, callback registry

**Status**: Not Started

---

## Stage 2: MPD Protocol Module

**Goal**: Create `mpd.rs` with connection state machine, line parser, idle-mode push updates.

### New file: `examples/media-control/src/mpd.rs`

**State machine**:

```
Connecting → AwaitingBanner → Ready → Closed
```

- `AwaitingBanner`: wait for `OK MPD x.y.z\n`
- `Ready`: normal operation — either idle-waiting or processing a response

**Thread-local state** (same pattern as `cast.rs`, `kodi.rs`):

```rust
struct MpdState {
    socket: Socket,
    phase: Phase,
    recv_buf: String,           // accumulate partial lines
    pending: Option<Pending>,   // what command we're waiting on
    response: Vec<(String, String)>,  // key-value pairs of current response
    idle_active: bool,
    queued_command: Option<QueuedCommand>,  // command waiting for noidle to complete
    on_status: StatusCallback,
    host: String,
    port: u16,
    volume_before_mute: u32,   // for mute emulation
    ms_since_activity: u32,    // liveness heartbeat
}
```

**Line parser**: Append `SocketEvent::Data` to `recv_buf`, split on `\n`, process complete lines:

- `OK MPD ...` → banner, transition to Ready, send initial `status\n`
- `key: value` → push to `response` vec
- `OK` → dispatch accumulated response based on `pending`, clear response
- `ACK [...]` → log error, clear response

**Command sequencing**: MPD is strictly one-command-at-a-time. `pending: Option<Pending>` tracks what we sent:

```rust
enum Pending { Status, CurrentSong, Idle, Fire }
```

After `Status` response → send `currentsong\n`. After `CurrentSong` → fire status callback, send
`idle player mixer options\n`. After `Idle` response (change notification) → send `status\n` (cycle restarts).

**User commands while idle**: When play/pause/next/etc. arrive while `idle_active`:

1. Store command in `queued_command`
2. Send `noidle\n`
3. On idle response: send queued command
4. On command response: send `status\n` (re-enters the status→currentsong→idle cycle)

**Mute emulation**: MPD has no mute. Store `volume_before_mute`. On mute: `setvol 0`. On unmute: restore. Report muted
when volume == 0.

**Liveness**: `tick()` increments `ms_since_activity`. If >30s with no socket events, send a `status\n` as heartbeat
(breaks out of idle, gets fresh state).

**Public API** (matches other protocol modules):

- `connect(host, port, on_status)`, `disconnect()`, `is_alive()`, `tick(delta_ms)`
- `play()`, `pause()`, `stop()`, `next()`, `previous()`
- `seek(position_secs: f64)` — sends `seekcur <secs>`
- `set_volume(level: u32)` — 0-100, sends `setvol <level>`
- `set_mute(muted: bool)` — emulated via volume

**Status callback payload**:

```rust
pub struct MpdMediaStatus {
    pub state: &'static str,        // "play" / "pause" / "stop"
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub elapsed_secs: f64,
    pub duration_secs: f64,
    pub volume: u32,
}
```

**Album art**: Skip for now. Can add later via MPD's `albumart`/`readpicture` commands or OwnTone's HTTP API.

**Status**: Not Started

---

## Stage 3: Widget Integration

**Goal**: Wire MPD into discovery, connection, and UI.

### Modify: `examples/media-control/src/lib.rs`

1. Add `mod mpd;`
2. Add `Mpd` variant to `DiscoveredProtocol` enum (after `Emby`, before `Upnp` — or at end)
3. Add `"_mpd._tcp"` to the `mdns_browse` service types array in `init()`
4. Add mDNS handler branch in `on_mdns_event` for `_mpd._tcp` service type
5. Add `DiscoveredProtocol::Mpd` arm in `connect_to_device()`:
   ```
   DiscoveredProtocol::Mpd => {
       mpd::connect(&device.host, device.port, on_mpd_status);
       Box::new(MpdAdapter)
   }
   ```
6. Add `MpdAdapter` struct (zero-sized) + `impl MediaController`:
   - `set_volume`: multiply 0.0-1.0 by 100
   - `seek`: pass position_secs as f64
   - `poll_interval_playing/idle`: 30_000 (idle mode handles updates)
   - `protocol_name`: `"MPD"`
7. Add `on_mpd_status` callback — same pattern as `on_kodi_status`:
   - Map `"play"/"pause"/"stop"` to `TransportState`
   - Volume: `status.volume * 10` (0-100 → permille)
   - Build `TrackMeta` from title/artist/album fields
8. Add all match arms: `proto_label` ("MPD"), `proto_icon` (`icons::PROTO_MPD`)

### Modify: `examples/media-control/src/icons.rs`

Add:

```rust
pub const PROTO_MPD: Icon = include_icon!("assets/icons/proto-mpd.svg");
```

The SVG file already exists (unstaged `proto-mpd.svg`).

**Status**: Not Started

---

## Verification

1. `make validate-wasm` — formatting + clippy + checks
2. `make run EXAMPLE=media-control` — testbed with OwnTone at `nas.lan:6600`
3. Verify: MPD device appears in device picker via mDNS `_mpd._tcp`
4. Verify: connect shows track info, playback controls work (play/pause/next/prev/seek/volume)
5. Verify: idle mode delivers push updates (change track in OwnTone web UI → widget updates)
6. Verify: mute toggle works (volume goes to 0 and back)
