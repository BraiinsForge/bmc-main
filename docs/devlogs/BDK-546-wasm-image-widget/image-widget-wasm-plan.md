# BDK-546 — WASM image widget: analysis & port plan

Port the released Slint image widget (`RemoteImageWidget`: fetch an image from an operator-configured URL on a refresh
interval, display it fitted to the widget viewport) to the WASM widget runtime, under epic Widgets NG (BDK-403).

This is a throwaway scaffolding doc for the branch; it is not intended to survive into the final MR.

Scope note: beyond porting the widget, this MR also builds a reusable **widget asset-lifecycle layer** (flash-backed
storage, asset re-hydration, dormant/wake lifecycle hooks); the image widget is its first consumer. See the dedicated
section below.

## Resource budget (measured on device)

- **RAM is the scarce resource: ~240 MB total** (`MemTotal 246 MB`, ~100 MB available), shared across the compositor and
  every widget process. The etnaviv/GC400 GPU has **no dedicated VRAM** — textures are allocated from this same pool via
  GBM, so every texture byte is a RAM byte. A full 12 MP source decode (~48 MB RGBA) is ~20 % of total RAM. This is what
  the whole design protects.
- **Flash is abundant and wear-leveled.** 3.22 GiB **eMMC** (hardware wear-leveling). `/overlay` is small (116 M f2fs,
  ~45 M free) = OpenWRT config — wrong place for a cache. The writable data area is **`/mnt/data` (2.8 G ext4, ~1.4 G
  free**, shares the partition with `/nix`). MB-scale per-refresh writes are negligible for both space and endurance.
- **CPU: dual-core Cortex-A7 (STM32MP157C).** A second core exists for a background decode thread; no cooperative
  work-scheduler is needed up front.

## Prior art

- **Slint image widget** (stable-25.11): gRPC `RemoteImageWidget { url, refresh_duration_sec }`, task
  `bmc/src/widget_tasks/remote_image.rs`. HTTP GET with `deck_image_width/height` query hints, decode PNG/JPEG via the
  `image` crate, **CPU-resize** (`FilterType::Triangle`) + black letterbox. States: Initial / Loading /
  ConfigurationError / LoadingError / UnexpectedError / LoadingSuccess + a stale overlay with retry; later gained a fill
  toggle and URL templating. (Distinct from the Slint `RemoteWidget`, which renders server-side and returns an image URL
  — not this ticket.)
- **media-control (WASM)** `bmc-wasm-runtime/examples/media-control/`: existing fetch → `BitmapSlot.set()` (register +
  auto-evict previous) → `Draw::bitmap_id` aspect-aware *contain* letterbox → `evict()`; placeholder icon for
  loading/empty; `bitmap_sample` for accent tint. It sidesteps large sources (album art is small; YouTube URLs are
  rewritten to a small `mqdefault` server variant). The reusable plumbing exists; handling a large operator-supplied
  image is the new work.

## Binding runtime constraints (verified)

- **No streaming anywhere today.** Fetch buffers the whole body in RAM (ureq `read_to_vec()`,
  `runtime/background/fetch.rs:60`); decode produces a full `DynamicImage` (`runtime/imports/render/assets.rs`); femtovg
  needs the full RGBA to upload (`gpu/bitmap.rs:231`).
- **Decode caps reject large sources:** `MAX_DECODE_IMAGE_PIXELS = 4_194_304` (≈2048²), `max_alloc = 24 MB`
  (`assets.rs`). A 12 MP photo is rejected today.
- **Resident cost per bitmap:** full RGBA kept in CPU forever (`StoredBitmap.pixels`, used only by `bitmap_sample`,
  never freed) **plus** the GL texture (`gpu/bitmap.rs:19`) — both in the shared 240 MB pool.
- **Draw-time GPU scaling aliases on downscale** — bitmaps register with `ImageFlags::empty()` (no `GENERATE_MIPMAPS`,
  `gpu/renderer.rs:805`). Addressed by the exact-dimensions contract, not by on-device quality.
- **Registered bitmaps survive dormancy; only the render target (export buffer) is freed.** Verified: `apply_lifecycle`
  (`bmc-wasm-host/src/slot.rs:548`) only allocates/frees the slot's render target on a lifecycle transition — it does
  not touch renderer assets. Bitmaps are evicted **only** on teardown (`shutdown`, `slot.rs:712` →
  `evict_renderer_assets`) and lost on restart. So a `BitmapId` survives dormant→visible but dies on teardown/restart.
  (Corrects an earlier overstatement; flagged by František.) Note: textures staying resident through dormancy means a
  dormant image widget holds its viewport-sized texture in the 240 MB pool — addressed by the asset-lifecycle layer
  below.
- **Guest lifecycle surface today:** the guest exports `init` / `render(delta_ms)` / `on_params_update` /
  `on_system_update` / `on_touch` / `unload` — there is **no `on_dormant`/`on_wake` hook**. A dormant slot is not
  rendered (`main_loop.rs:378` gates on `needs_render`; `slot.rs:589` confirms render ∉ dormant), and the wasm linear
  memory persists across dormancy. So the guest cannot self-evict while dormant, and must not hold the asset bytes (they
  would stay resident and defeat the reclaim) — re-hydration must come from the host-side flash store.
- **FBO machinery present** (`gpu/renderer.rs:123` `create_render_target` / `begin_frame_to_image` /
  `set_render_target`), used by the drop-shadow path. Its perf cost was per-frame shadows on moving clock hands (usage
  removed; impl remains) — a one-shot FBO downscale per image load is amortized and viable.
- **Flash write precedent:** the KV store is file-per-key under `kv_store_path` (`runtime/imports/data.rs`). Its read
  path loads the whole value into an in-memory `kv_cache`, so it must NOT be used for image blobs — the image cache is a
  dedicated, mmap-friendly store.

## Approach

Two memory axes, handled separately. The scarce resource is RAM; flash is free.

1. **Server-side sizing (primary large-source strategy).** Send the viewport size as a query hint; cooperative servers
   return a pre-sized image and the device never decodes a giant one (Slint + media-control precedent). No new deps.
2. **Async image job (architecture).** A host job off the render thread (background thread on the 2nd core): on a cache
   miss, fetch to a transient temp → decode + downscale with bounded memory → write the **downscaled** result to the
   `/mnt/data` cache → register the small bitmap; on wake/restart (cache hit), skip the fetch and load straight from the
   cached downscaled artifact. It signals `started | failed(error) | done(BitmapId)`; the widget renders a placeholder
   while working and swaps on `done`. Model on the existing background-fetch + completion-callback infra.
3. **Bounded decode — formats limited to PNG + JPEG** (no libvips):
   - **JPEG:** `jpeg-decoder` `.scale()` — DCT shrink-on-load (1/8, 1/4, 1/2, 1). Pure Rust, ARM-viable; an added
     explicit dep (the in-tree `zune-jpeg` 0.4 has no scaled-decode API).
   - **PNG:** `png` crate incremental rows (`Reader::next_row`) → a ~20-line box-downscale accumulator. Bounded memory,
     naturally pausable.
   - No general all-format shrink-on-load lib is embedded-viable (only libvips, native/heavy, excluded by limiting
     inputs to PNG/JPEG).
4. **Resident bounding.** Keep only the viewport-sized texture (one-shot CPU thumbnail or one-shot FBO downscale); drop
   the full buffer and the retained CPU `pixels` copy when `bitmap_sample` is unused.
5. **Reject** non-PNG/JPEG, or oversized when the server ignores the hint and the bounded on-device path cannot apply →
   error/placeholder state.
6. **Quality contract:** exact dimensions → no on-device scaling → sharp.

**Config:** fully manifest-driven (`url`, refresh interval, later fill mode / templating); extend the
manifest/capability schema where it falls short, rather than reviving the gRPC `RemoteImageWidget`.

## Flash image cache

> Key & eviction here are **superseded by *Cache identity & GC*** below — the key is the widget instance
> (`uuid + extent`), not `(URL, size, fit)`, and eviction is a periodic reconcile sweep, not per-change.

Mandatory because bitmaps are lost on teardown and on restart (they survive plain dormancy today), and re-fetching from
the network is slow and fails offline.

- **Location:** a dedicated cache dir on `/mnt/data`, per-widget namespace; reads via `memmap2`. Separate from `kv`
  (which RAM-caches values).
- **Content:** the **downscaled** result only, keyed on **(URL, target size, fit-mode)**. A size change evicts that
  entry and triggers a fresh download + resize (the original is not kept). The original is transient — it exists only
  during the first decode (RAM `Cursor`, or stream-to-temp + `memmap2` for a huge source) and is then dropped.
- **Blob format:** raw RGBA — on wake, `mmap` → `create_image`, zero decode on the hot path. Resident footprint on load
  is one viewport-sized texture either way (the irreducible floor for displaying the image), so format is not a
  flash-economy choice; raw wins on wake cost. PNG-encoded is a fine alternative if files should be smaller.
- **Eviction:** flash is abundant, so this is tidiness not pressure — a modest global size cap + LRU over the cache dir;
  automatic eviction on URL/size/params change (per-widget); plus an operator-facing "clear image cache" (stale cached
  art is a support scenario).
- **Dormancy re-hydration source.** The cache also backs the asset-lifecycle layer (next section): a dormant widget's
  texture is freed and re-uploaded from the cache on wake, reclaiming the viewport-sized RAM it would otherwise hold.

## Cache identity & GC

(Supersedes the `(URL, size, fit)` keying in *Flash image cache* and *Refresh* above. Shaped by the MR review — the
keying went through `{uuid, extent}` → opaque token, and the GC argument was sharpened against a resource-keyed
alternative.)

**Two levels: a host-owned per-instance *bucket*, with widget-chosen keys inside.** The cache is not keyed on the
resource. Each widget instance gets a bucket — a namespace the host owns — and inside it the widget stores entries under
its *own* keys, which only the widget knows (for the image, the resized artifact's identity `(url, w, h)`). The host
never interprets those inner keys. Concerns split cleanly: the widget owns *what* it caches and how it names it; the
host owns the bucket and its lifecycle.

**The bucket id is the widget instance, not the URL.** It already exists as `WidgetId(Uuid)` (`scene.rs:103`): minted
per placed widget, persisted in `config.scenes`, stable across restart/move/resize, fresh on clone/add. A
delete-then-add mints a new id and the old one leaves the config, so the persisted scene config is the authoritative
live set. The bucket key is `uuid + extent`: `uuid` covers remove (gone from config → swept), move (same id → reused),
and type-swap via delete+add; `extent` — the `Fullscreen | Span{cols,rows}` placement span (`WidgetPlacement`),
DPI/resolution-stable, not raw pixels — covers resize: a new extent is a new bucket. Position is deliberately excluded;
it never changes the artifact.

**Compositor-minted, opaque, host-agnostic, delivered on the handshake.** Only the compositor knows scene + placement +
instance, so it mints the id once and ships it on the existing `deck_widget_surface_v1` handshake (the `initial`
configure that already carries `display`/viewport, `slot.rs:159`). It is delivered as an *opaque token* — a
`WidgetIdentity { token: String }` serde envelope in `bmc-widget-protocol`, the token being `uuid-<extent-tag>` minted
compositor-side. The widget treats it as a black box; uuid/extent never appear as typed protocol concepts, so a future
round-display / combined-scene extent model reshapes only how the id is minted and touches no widget-facing contract.
The protocol is widget-facing — nothing about the wasm host appears in it. The **wasm guest never sees the token**: the
wasm host curries its `DiskCache` with the bucket id and exposes only put/get/evict-by-tag to the guest. (Review settled
this: an earlier `{uuid, extent}` JSON, host-named `host_context` event was reworked to this opaque, de-host-ified
token.)

**The generic store.** `DiskCache` (`bmc-wasm-runtime/src/disk_cache.rs`) is content-agnostic:
`put(key, saved_at, metadata, bytes)` / `get → { saved_at, metadata, bytes }` / `evict` / `sweep` / `trim`, on-disk
`[saved_at u64 | meta_len u32 | metadata | bytes]`, `mmap`'d reads. It owns only the on-disk format, a first-class
`saved_at` (UTC epoch, for age/freshness), an opaque caller `metadata` blob, the byte-cap LRU, and the sweep — no
width/height/pixels (those were an image-specific leak, removed). The image layer keeps its bitmap concerns to itself:
RGBA in `bytes`, `(width, height)` + a url hash in `metadata`. The same store serves a sound or runtime-generated
artifact unchanged.

**GC = a periodic reconcile of buckets against the live set — not signals, not LRU-first.** Don't depend on cleanup at
widget-drop: nothing guarantees the message arrives (config edited while off, app killed mid-shutdown, external write).
A low-frequency reconcile deletes every bucket not in the live set — the whole bucket at once. Signal-free and
deterministic (not only-under-pressure), and generic: it drops a bucket without re-deriving or understanding the
widget's inner keys. A resource-keyed cache could not — the collector would have to replay every widget type's key
derivation from params (re-coupling to the internals the keying hides) and still couldn't see keys computed at runtime,
leaving only LRU-by-age. Mechanics model `cleanup_stale_files` (`bmc-nix/.../copy_files.rs:66` — walk + membership +
graceful remove). The per-bucket LRU byte cap is the between-sweep backstop, not the primary reclamation.

**What the "live set" is — superseded in review.** The first sketch derived it from `config.scenes` in a host-side
`JobScheduler` cron + startup reconcile. Rejected: the cache is shared by **several hosts at once, one per SDK major**
(a protocol break keeps old and new hosts alive until widgets upgrade), so no single config view is the whole live set.
The implemented design is filesystem-based — each running host publishes the tokens it holds to a per-major GC-root file
(mtime = heartbeat), and the reconcile, run inside each host loop, keeps the union across all live hosts. Full design +
on-device testing: **[`asset-cache-gc.md`](../../devel/wasm-host/asset-cache-gc.md)**.

Orphan accumulation is confirmed, not hypothetical: swapping a device's `config.scenes` from two image widgets to a
different set left the removed instances' `…-full/` buckets behind on `/mnt/data/bmc/widget-cache/`, since nothing
reconciles bucket dirs yet. (`DiskCache::sweep` exists but only sweeps blobs *within* one bucket against live tags — it
is the per-bucket layer, not this cross-bucket reconcile, and has no production caller either.)

**Cache-at-decode, restore-on-wake.** `register_rgba` (`bitmap.rs:133`) keeps only `{image_id, w, h}` and drops the RGBA
after upload, so at dormancy the host has no pixels. The widget writes its bucket after decode; on wake it re-registers
from the bucket (`mmap` → upload, no fetch/decode) under its own tag. The freshness rule (saved_at age vs the refresh
param) decides whether wake/refresh re-fetches or serves the cached entry.

**Built so far (committed):** the `on_dormant`/`on_wake` lifecycle hooks; the generic `DiskCache`; the opaque
`WidgetIdentity` handshake (compositor mint + event + `InitialState` plumbing); per-instance bucket currying + the guest
put/get/evict-by-tag API; write-at-decode; restore-on-wake with the sizing-aware decode; host-driven dormant texture
eviction; the fit/fill sizing mode; observability on the profiling channel; and the cross-host asset-cache GC (per-host
GC-root files + union reconcile — [`asset-cache-gc.md`](../../devel/wasm-host/asset-cache-gc.md)).

**Remaining:** the core cache work is complete; `GuestId` retirement (below) is the only tail-end follow-up.

**Followup (tail end).** `GuestId(u32)` (`host_api.rs:216`) is a *separate*, process-local auto-increment id that
namespaces in-memory asset tags (audio prefix evictions, `namespaced_tag`) — minted fresh per process, unrelated to the
persisted bucket token. Once the stable `WidgetIdentity` bucket id is plumbed, revisit whether `GuestId` collapses into
it (one stable per-instance id serving both in-memory namespacing and the flash bucket) or stays independent. Deferred
to the tail of this work so it does not entangle the step-3 wiring.

**Raised in review (MR !367).** The provisional constants (`WIDGET_CACHE_BUCKET_MAX_BYTES`, `WIDGET_CACHE_DIR` + the
`/mnt/data`-mounted dependency), the collected observability evidence (`observability-verification.md`), and the live
cross-host GC validation were posted as review notes on !367 (`317006`–`317008`).

## Sizing modes (fit / fill)

Operator-facing `sizing` enum param (`contain` default | `cover`), wired through to the **decode**, not just render:

- **Decode is sizing-aware.** `contain` scales-to-fit + letterboxes (today's path); `cover` scales so the constraining
  dimension fills and crops the source to the viewport → a viewport-sized artifact. Crop the *source* first, then resize
  (not `resize_to_fill`'s resize-then-crop), so a pathological tiny extreme-aspect source can't upscale into a giant
  transient.
- **`sizing` joins the cache identity.** The two modes produce different viewport-sized blobs, so the identity
  distinguishes them; a `sizing` change re-decodes (`on_params_update` while visible, a `try_restore` identity miss on
  wake).
- **Render draws the `cover` blob 1:1** (pre-cropped); `contain` keeps its letterbox rect.
- **Memory:** no regression — the cover blob is viewport-sized (the design's texture floor), same decode caps, CPU ≈
  contain. The render-only `cover` that shipped first was wrong: it downscaled-to-fit then upscaled-to-fill (a double
  resample), mangling a source that would have been sharp.

## Refresh / freshness semantics

The refresh period is the cached image's **TTL**, with one freshness rule applied everywhere: *if the cached entry's age
≥ the refresh period, re-fetch; else show it and wait.* Applied (a) on the poll tick while visible, (b) on wake after
re-hydrating from cache, and (c) on restart.

- **Timestamp is an absolute UTC instant** (epoch millis), host-stamped into the cache entry — **timezone-invariant**,
  following the existing `next_alarm` convention ("UTC milliseconds since the Unix epoch; pair with the timezone field")
  and how the clock widget reads `SystemTime::now()` and applies the TZ only at render. The display-TZ setting (an IANA
  string) **never enters** the freshness math; the image widget shows no time, so it never touches TZ at all. Stored in
  the cache (not wasm memory) so it survives dormancy and restart.
- **Visible** refresh rides the host poll timer (monotonic, TZ-safe already). **Dormant:** `on_dormant` pauses the poll
  (no off-screen fetching). **Wake:** re-hydrate the texture from cache, then apply the freshness rule — refresh now if
  past TTL, else resume the poll. **Bucket** is the instance (`uuid+extent`, per *Cache identity & GC*); within it the
  widget's entry tag is `(url, w, h)`. A refresh re-fetches and overwrites that tag; a URL change is a new tag (the
  widget drops the old one on the params change), all inside the same bucket. An LRU eviction or a bucket sweep while
  dormant turns the wake re-hydrate into a cache-miss → full fetch.
- Caveat: wall-clock UTC can jump (NTP, unset RTC before sync); for a coarse TTL that at worst triggers an early/late
  refresh, never corruption — consistent with how the clock/alarm features already trust wall-clock UTC.

## Widget asset-lifecycle layer (in scope — built here, reusable)

This MR builds a reusable SDK layer for heavy-asset widgets, not a one-off for images. The image widget is its first
consumer; a sound or other heavy-asset widget later reuses whichever parts it needs. Four pieces:

- **Lifecycle hooks.** New guest exports `on_dormant()` / `on_wake()`, fired by the host at the visible↔dormant
  transitions — boundary ticks like `unload`, not during dormancy, so the no-code-while-dormant invariant holds.
  `on_dormant` lets a widget pause its own activity (polls, timers, transient wasm buffers); `on_wake` re-hydrates and
  resumes.
- **Flash-backed blob store (`DiskCache`).** The generic store behind the buckets (host-managed, `memmap2`-backed, LRU'd
  on `/mnt/data`; distinct from `kv`), built and committed: `put(key, saved_at, metadata, bytes)` /
  `get(key) → {saved_at, metadata, bytes}` / `evict(key)` / `sweep(live)` / `trim()`. Content-agnostic — it owns a
  first-class `saved_at` (UTC epoch, tz-invariant) for age/freshness, an opaque per-entry `metadata` blob the caller
  defines, and the opaque `bytes`; it knows nothing of images. The image layer puts RGBA in `bytes` and
  `(width, height)`
  - a url hash in `metadata`; a sound or runtime-generated artifact caches the same way with its own metadata.
- **Asset re-hydration.** Register a bitmap/audio/mesh against a *cache key* so the host can re-upload it from the blob
  store on wake — the bytes never enter wasm memory.
- **Bounded decode/resize producer (image-specific).** jpeg-decoder/png → downscaled artifact → blob store; the first
  concrete asset producer. Other widgets bring their own.

**Eviction is host-driven; hooks carry intent.** The host owns the GPU eviction on →Dormant — it already frees the
render target in `apply_lifecycle` and evicts the slot's cached-asset textures there too, with correct GL-context
ordering. The guest's `on_dormant` does only non-GPU work; `on_wake` triggers re-hydration (host re-uploads, context
current because a frame is imminent). This split keeps the guest out of fragile GL-teardown ordering.

**Efficiency beyond the texture.** `on_dormant` lets a polling widget stop fetching while off-screen — a CPU/network/RAM
win across widgets, not just images. (Confirm whether the host currently keeps a dormant slot's `register_poll` firing;
if so, this is the clean fix.)

**Tuning deferred to the real runtime.** Wake re-upload sync-vs-async and dormant-eviction timing (immediate vs a short
grace period to avoid thrash during fast scene cycling) are feel decisions — ship sane defaults (async-on-miss,
grace-period eviction), make them tunable, adjust on-device.

## Hermetic replay & fixtures

The regression replay is fully hermetic — the fixture carries all internal and external state — so every new state
source the image widget introduces has to slot in.

- **Binary fetch bodies: supported.** The unified fixture format (`bmc-wasm-runtime/src/unified_fixture.rs`) has a
  `FixtureBody::Base64` variant (`"b64"`); `from_bytes` falls back to base64 for non-text payloads. So image bytes embed
  as base64 and replay through the fetch stub like weather's JSON.
- **Clock: host-injected, already hermetic.** The guest's `SystemTime::now()` is a host import (`host_get_system_time` →
  UTC `unix_secs: i64`); replay drives wall + monotonic time via the runtime's injected clock
  (`set_time(system_time, monotonic_ms)`, read host-side via `n()`). **Rule:** the cache layer must stamp entries and
  compute TTL freshness from the injected clock (UTC `unix_secs` / `n()`), never
  `std::time::SystemTime::now()`/`Instant::now()`. This is also exactly the TZ-invariant UTC instant the freshness
  design needs.
- **Cache-lifecycle replay is blocked by the testbed (out of scope).** The dormant→wake cache cycle (evict + re-hydrate)
  can only be driven by `→dormant`/`→visible` transitions, and the testbed has no mechanism to fire them — so a
  redirectable cache backing and lifecycle fixture events are moot until that lands (its own work, not this MR). The
  recorded fixtures cover decode + render for both sizing modes; the cache lifecycle is verified on VM/device instead
  (see `observability-verification.md`).

## Staging (full parity; large-source support lands by MR end)

- **Stage 0 — widget fundamentals (no host change).** `widgets-wasm/image/` reusing media-control plumbing: manifest
  `url` + refresh; fetch; register via `BitmapSlot`; `Draw::bitmap_id` fit/letterbox; loading/error/stale states;
  evict-on-change; send the viewport-size query hint. Works for sources within the 2048² cap; offline tests.
- **Stage 1 — async job + resident bounding.** Move decode+register into the async job (`started/failed/done`); bound
  resident memory (CPU thumbnail or one-shot FBO); drop the retained CPU copy.
- **Stage 2 — flash blob store + bounded large-source decode.** The blob store (`/mnt/data`, per-widget namespace,
  `memmap2`, LRU + manual eviction) and the image producer: bounded decode from the cached/temp file (jpeg-decoder
  scale, png-rows + accumulator); cache-backed load on a hit; revisit the decode cap.
- **Stage 3 — asset-lifecycle layer (MR final state).** `on_dormant`/`on_wake` guest hooks; host-driven eviction of the
  slot's cached-asset textures on →Dormant; re-hydration from the blob store on wake (bytes never enter wasm). The image
  widget opts in as the first consumer.
- **Stage 4 — parity features (done).** Fill/crop toggle = the `fit`/`fill` sizing mode (cover-aware decode); URL
  templating and the server-side sizing hint = `{{width}}/{{height}}`; the exact-dimensions no-scale path = the decode
  using the source as-is when it already fits the viewport (`resize_rgba_to_fit`'s pass-through).
- **On-device manual reload.** A gesture to re-fetch on demand, not just on the refresh timer. UX undecided —
  candidates: long-press → circular loader with a "Reload" label, or an Android-style pull-to-reload. Reuses the
  existing loading/error/stale states; design the gesture + loading feedback before building.

## Critical files

- New: `widgets-wasm/image/**` (scaffold from `widgets-wasm/weather/` + `bmc-wasm-runtime/examples/media-control/`); a
  host flash blob-store module (`/mnt/data`, `memmap2`, LRU); SDK modules for the blob store + asset re-hydration.
- SDK: `bmc-wasm-runtime/sdk/src/lib.rs` (new `on_dormant`/`on_wake` guest hooks), `.../sdk/src/host.rs` (async
  image-job, blob-store, and re-hydration APIs).
- Host: `bmc-render/src/gpu/bitmap.rs` (downscale-on-register; drop CPU copy), `bmc-render/src/gpu/renderer.rs`
  (one-shot FBO downscale if chosen), `bmc-wasm-runtime/src/runtime/imports/render/assets.rs` (decode-cap policy;
  jpeg-decoder + png-incremental paths; cache-keyed re-hydration), `bmc-wasm-runtime/src/runtime/background/fetch.rs` +
  `imports/network.rs` (async job completion; fetch-to-cache-file), `bmc-wasm-host/src/slot.rs` (`apply_lifecycle` fires
  the guest hooks + host-driven asset eviction on →Dormant, with GL-context ordering),
  `bmc-wasm-runtime/src/runtime/imports/data.rs` (blob-store host API alongside `kv`).
- New deps: `jpeg-decoder`, `memmap2`.

## Verification

- `just wasm::gen image`, build; `just wasm::run image` against test URLs including a deliberately large source; eyeball
  fit/letterbox + downscale quality at each size in the running instance.
- Memory: instrument `BitmapRegistry`/`StoredBitmap` to confirm resident ≈ viewport size and that peak is bounded on the
  jpeg-decoder / png-incremental paths.
- Offline tests: stub the fetched image bytes in capture fixtures. `just validate`; `just validate-wasm`;
  `just wasm::verify-all` against committed, human-blessed baselines.

## Observability & measurement (its own commit, not the consumer's)

Lands as a **separate commit** from the image-widget consumer, so the cache is measurable on its own terms rather than
entangled with widget logic. Today the only runtime signal is `tracing` → the host log (`/var/log/bmc/*.log`); there is
no metrics pipe and no RSS/memory-reading precedent, so this instrumentation is all new.

**The signals.** Structured `tracing` events under one `target: "widget_cache"`, greppable in the host log:

- cache seams — `put{tag, bytes, meta_len}`, `get` hit/miss `{tag, age_ms}`, `evict`, `trim{evicted, freed}`, and the GC
  reconcile `{buckets_dropped}`
- `DiskCache::stats()` (new) — bucket entry count + total bytes, logged on transitions; the flash footprint and the
  basis for the per-bucket cap review
- `BitmapRegistry` resident-bytes accounting (new) — logged at register/evict and **at the dormant/wake boundary**, so
  the RAM-reclaim premise is measured, not assumed
- freshness — each wake/poll decision (serve-cached vs refresh, age vs TTL)

**Samply profile tracks: dropped.** The earlier plan to merge cache-size / mem-size counter tracks and cache-event
markers into `just wasm::profile`'s `profile.json.gz` is superseded. Observability landed on the profiling channel
instead (`mesh::profile` + ii-stopwatch, surfaced by a `--profile debug` build and verified on VM + device), and the
desktop testbed wouldn't exercise those tracks fully anyway.

**VM vs device.**

- **VM (`bmc-virt`)** — the *logic*, CI-able: drive dormant/wake + refresh via fixtures/events and grep the
  `widget_cache` log for the miss→put→restore→GC sequence. The VM's memory/GPU model is not the GC400, so its reclaim
  numbers are not representative — assert behaviour here, not byte budgets.
- **Device (real Deck)** — the *numbers*: the resident-bytes + `DiskCache::stats()` logs and the samply tracks against
  the 240 MB budget, plus a new `/proc/self/statm` RSS sample around decode and across dormancy to confirm peak-bounded
  decode and the reclaim. This is what justifies `WIDGET_CACHE_BUCKET_MAX_BYTES`.

## Open implementation items (map before building the host stages 1–3)

- The existing async/background-job + completion-delivery infra and host threading/handoff to the single wasm instance —
  to model the image job on.
- ureq 3.x streaming body→file/reader API (fetch → cache file).
- Host→guest hook invocation: `apply_lifecycle` (`slot.rs:548`) does not currently have the renderer in scope, so wiring
  it to fire `on_dormant`/`on_wake` and to evict the slot's assets needs the renderer + GL context threaded in,
  sequenced before the render-target free. (Mechanism is decided — explicit hooks; this is the wiring.)
- Whether the host currently keeps a dormant slot's `register_poll` fetches firing — if so, `on_dormant` should pause
  them.
- Deferred to on-device tuning: wake re-upload sync-vs-async, and dormant-eviction timing (immediate vs grace period).
- The host's designated writable app-data path under `/mnt/data` for the blob store.
