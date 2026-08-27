# Widget Asset Cache & Cross-Host GC

How the on-disk widget asset cache is laid out, and how orphaned entries remain safe if several host versions run during
an SDK transition.

For why cache-backed renderer assets survive dormancy, see [`renderer-assets.md`](renderer-assets.md). For the
major-versioned host socket, see [`process-model.md`](process-model.md) and [`sdk-versioning.md`](sdk-versioning.md).

## Cache layout

The cache is a two-level store under `/mnt/data/bmc/widget-cache/`:

- **Bucket** — one directory per widget instance, named by an opaque **token**. The host owns the bucket; the widget
  never sees the token.
- **Entries** — inside a bucket, the widget stores blobs under its *own* keys (for the image widget, the resized
  artifact keyed by `(url, w, h, sizing)`). The host never interprets these inner keys.

The token is `<uuid>-<placement_tag>`, minted compositor-side and delivered on the widget handshake as a
`WidgetIdentity { token }` (`bmc-widget-protocol`). `uuid` is the persisted per-instance `WidgetId`; `placement_tag`
encodes the placement extent (`full`, `2x1`, …) so a resize is a new bucket but a move/position change is not. The host
receives the token in the initial configure and curries its `DiskCache` (`bmc-wasm-runtime/src/disk_cache.rs`) with it;
the wasm guest only gets put/get/evict-by-tag.

A bucket survives plain dormancy and host restart (the cache is the re-hydration source for a dormant widget's freed
texture). It dies only when **garbage-collected** — which is what this document covers.

## Why a GC, and why it can't read the config

Buckets are never deleted at widget-drop: no signal is guaranteed (config edited while the device is off, app killed
mid-shutdown, external write). Without reclamation they accumulate — confirmed in practice: swapping a device's
`config.scenes` to a different widget set leaves the removed instances' `…-full/` buckets behind. (`DiskCache::sweep`
only trims blobs *within* one bucket; it is not the cross-bucket reconcile.)

The obvious design — "list the buckets of every widget in `config.scenes`, delete the rest" — is **wrong here**. A host
owns only the slots connected to its SDK-major socket, not the whole config. The default system starts one active SDK
host, but an incompatible SDK transition can configure old and new hosts as separate compositor commands. Either host
would treat the other's live buckets as orphans. The live set therefore has to be assembled from what each running host
declares.

The filesystem is the only thing every host shares (going through the compositor is deliberately avoided), so liveness
is published there.

## Mechanism

Each host owns one **GC-root file** in a sibling directory:

```
/mnt/data/bmc/widget-gc-roots/sdk-v<major>
```

- **Name** — keyed by SDK major, which keeps concurrent hosts distinct using the same scheme as the
  `wasm-host-sdk-v<major>` socket.
- **Contents** — the cache tokens the host currently holds, one per line.
- **mtime is the heartbeat** — rewriting the file (on the periodic tick, and whenever a slot is added) bumps its mtime.

`reconcile` then:

1. walks the GC-roots directory; a root whose mtime is older than **2× the period** is a dead host's leftover and is
   pruned;
2. unions the tokens of the surviving (live) roots into the keep-set;
3. removes every `widget-cache/<token>` bucket not in the keep-set.

Shape mirrors `cleanup_stale_files` (`bmc-nix/.../copy_files.rs`): walk, test membership, remove gracefully (a
`NotFound` from a peer host winning the same delete is not an error).

**Safety belt** — if *zero* live roots are found, reconcile skips the bucket sweep entirely. A correct host writes its
own fresh root before reconciling, so an empty set means the liveness picture is missing, not that everything is
orphaned; declining to delete is the safe failure.

## Cadence and lifecycle

Run inside the host loop (`bmc-wasm-host/src/main_loop.rs`):

| When              | Action                                                            |
| ----------------- | ----------------------------------------------------------------- |
| Host startup      | publish the (possibly empty) root — **no reconcile** (see below)  |
| Slot inserted     | republish, so a new token is protected before a peer's next sweep |
| Every `GC_PERIOD` | republish (also picks up teardowns), then reconcile               |

The period defaults to 30 min — override with `BMC_WIDGET_GC_PERIOD_SECS` (seconds) for testing — and the stale
threshold is 2× it (tolerating one missed heartbeat). The `poll(2)` timeout is capped by the next tick so an
otherwise-idle host still wakes to it.

**No reconcile at process startup.** A freshly started host has not loaded its widgets yet, so its tokens aren't
published — sweeping then would delete the very buckets it is about to re-use on restart, defeating the cache. Startup
only *publishes*; the first sweep lands one period in. Teardown is deliberately lazy (the root is rebuilt on the next
tick, not on each teardown), which over-claims a removed widget's bucket for up to one period — harmless on abundant
flash, and it protects a bucket through a transient thin restart/reload gap.

A host that exits does **not** remove its own root; mtime staleness reclaims it within 2× the period, and that same
mechanism covers crashes (which a clean-exit hook would not).

## Code map

| File                                 | Role                                                               |
| ------------------------------------ | ------------------------------------------------------------------ |
| `bmc-wasm-host/src/cache_gc.rs`      | root-file write, reconcile, staleness; unit tests                  |
| `bmc-wasm-host/src/main_loop.rs`     | wiring: publish on startup/insert, tick publish + reconcile        |
| `bmc-wasm-host/src/slot.rs`          | `WidgetSlot::cache_token`, the per-slot token enumerated for roots |
| `bmc-wasm-runtime/src/disk_cache.rs` | the per-bucket blob store (`DiskCache`)                            |

## Testing on a real runtime

The GC runs only in the actual `bmc-wasm-host` process — i.e. on the VM or a device, never in the testbed/capture (which
drive `WasmWidgetRuntime` directly and never enter `run_loop`).

Publishing is immediate, so it can be checked without waiting:

```
ls -la /mnt/data/bmc/widget-gc-roots/ && cat /mnt/data/bmc/widget-gc-roots/sdk-v0
```

Reclamation, pruning, and the cross-host union only fire on the tick. To exercise them:

- **Orphan reclaim** — `mkdir /mnt/data/bmc/widget-cache/deadbeef-orphan`, wait a tick, watch it vanish (the
  `widget asset cache GC` debug line carries the stats).
- **Cross-host union / stale** — only one SDK major exists today, so fake a peer: a fresh `sdk-v1` root with a token
  keeps that token's bucket; an old-mtime `sdk-v1` root is pruned and its bucket reclaimed.
- **No-startup-wipe** — restart the host and confirm `widget-cache/` buckets survive immediately, proving the fresh host
  republishes before its first sweep.

The default 30-minute period makes live iteration slow, so set `BMC_WIDGET_GC_PERIOD_SECS` (e.g. `20`) in the
environment that launches `bmc-openwrt`; the supervised host inherits it from the compositor.

## Open items

- `WIDGET_CACHE_DIR` (`/mnt/data/bmc/widget-cache`) and `WIDGET_CACHE_BUCKET_MAX_BYTES` are provisional constants in
  `bmc-wasm-thin-protocol`, and the cache assumes `/mnt/data` is mounted and writable before the host starts — all
  deliberate team decisions to confirm.
