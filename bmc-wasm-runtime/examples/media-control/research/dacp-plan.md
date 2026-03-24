# DACP Media Control + mDNS Discovery + Host Primitives

## Status: All stages implemented ✅

All host primitives (mDNS, KV, HTTP listener), DMAP parser, DACP protocol, and device discovery are implemented and
integrated. See `research/plan.md` Stages 4-5 for the consolidated status.

## Key finding: DACP is dead on modern macOS

Apple Music on modern macOS (Ventura+) no longer advertises `_touch-able._tcp` via mDNS. The DACP protocol
(`_touch-able._tcp`, port 3689, DMAP binary responses) was the old iTunes Remote protocol. Apple replaced it with
AirPlay 2, which uses a completely different stack (encrypted, proprietary, no public documentation).

The DACP implementation in `src/dacp.rs` is correct per the protocol spec but effectively untestable without legacy
iTunes or a third-party DACP server.

## Implementation summary

| Stage | Component           | Files                                                         | Status          |
| ----- | ------------------- | ------------------------------------------------------------- | --------------- |
| 1     | mDNS host primitive | `runtime_wasmi.rs`, `host_api.rs`, `sdk/src/mdns.rs`          | ✅              |
| 2     | KV persistence      | `runtime_wasmi.rs`, `host_api.rs`, `sdk/src/kv.rs`            | ✅              |
| 3     | HTTP listener       | `runtime_wasmi.rs`, `host_api.rs`, `sdk/src/http_listener.rs` | ✅              |
| 4     | DMAP parser         | `src/dmap.rs`                                                 | ✅              |
| 4     | DACP protocol       | `src/dacp.rs`                                                 | ✅ (untestable) |
| 5     | Device discovery    | `src/lib.rs` (mDNS browse, picker UI)                         | ✅              |

## Detailed design

The original detailed design for each stage (host function signatures, SDK APIs, background thread patterns, DMAP
content codes, DACP pairing flow, etc.) is preserved below for reference.

---

### mDNS — Host functions

- `host_mdns_browse(svc_types)` — browse multiple service types, receive Found/Removed events as JSON
- `host_mdns_stop(browse_id)` — stop a browse session
- `host_mdns_register(svc_type, name, port, txt)` — register a service with TXT records
- `host_mdns_unregister(reg_id)` — unregister

Uses `mdns-sd` crate. Background thread per browse session with `mpsc` channel for event delivery. SDK provides
`MdnsBrowse`, `MdnsRegistration` structs.

### KV — Host functions

- `host_kv_set(key, value)` — persist key-value pair
- `host_kv_get(key, out_buf) -> len` — two-call pattern read
- `host_kv_delete(key)` — remove key

File-backed per-widget storage. In-memory cache for fast reads.

### HTTP listener — Host functions

- `host_http_listen(port) -> listener_id` — start listening (port 0 for ephemeral)
- `host_http_respond(request_id, status, headers, body)` — send response
- `host_http_close_listener(listener_id)` — stop listening
- `host_http_get_port(listener_id) -> port` — get actual bound port

Background thread with `TcpListener`, per-request response channels.

### DMAP parser

Binary TLV format: `[4-byte ASCII tag][4-byte BE u32 length][data]`

```rust
pub enum DmapValue<'a> { U8(u8), U16(u16), U32(u32), U64(u64), Str(&'a str), Data(&'a [u8]), Container(Vec<DmapNode<'a>>) }
pub fn parse(data: &[u8]) -> Vec<DmapNode<'_>>
pub fn find(nodes, tag) -> Option<&DmapNode>
pub fn find_u32(nodes, tag) -> Option<u32>
pub fn find_str(nodes, tag) -> Option<&str>
```

### DACP protocol

**Pairing flow:**

1. Generate GUID + 4-digit PIN
2. Start HTTP listener + register `_touch-remote._tcp` via mDNS
3. Music.app connects: `GET /pair?pairingcode=...`
4. Verify PIN, return DMAP pairing response, persist GUID
5. Auto-transition to login

**Session:**

- Login: `GET /login?pairing-guid=0x<GUID>` → session ID
- Long-poll: `GET /ctrl-int/1/playstatusupdate?session-id=<id>&revision-number=<rev>`
- Art: `GET /ctrl-int/1/nowplayingartwork?mw=1024&mh=576&session-id=<id>`

**Commands:**

- `playpause`, `nextitem`, `previtem`
- `setproperty?dacp.playingtime=<ms>`, `setproperty?dmcp.volume=<0-100>`
