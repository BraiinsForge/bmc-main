# WASM Renderer Asset Lifecycle

WASM widgets register SVGs, bitmaps, and meshes under stable tags. The runtime keeps the tag-to-ID reservation separate
from the resident renderer payload, so a dormant widget can release restorable data without invalidating the IDs held in
guest state.

This document covers renderer assets. Audio uses the same package mechanism for static files, but remains eager and
resident because it can be used without a render target.

## Backing classes

The runtime records one backing class for every renderer registration:

| Backing    | Source                                                           | Dormant behavior               | Restore behavior                                      |
| ---------- | ---------------------------------------------------------------- | ------------------------------ | ----------------------------------------------------- |
| `Package`  | immutable file installed beside the widget                       | suspend the renderer payload   | load and verify the package file                      |
| `Cache`    | decoded RGBA in the widget instance's on-disk cache              | suspend the renderer payload   | mmap and upload a cache hit; tolerate a missing entry |
| `Volatile` | bytes supplied through a pointer into the widget's linear memory | keep the renderer payload live | no automatic restore source exists                    |

`Package` and `Cache` are restorable: the host owns a source outside WASM linear memory. `Volatile` preserves the SDK
0.2 pointer-registration contract. Automatically suspending a volatile asset would leave the widget holding an ID that
the host could not make usable again, so volatile assets stay resident until explicit eviction or runtime teardown.

The backing class describes the restore source, not whether the guest currently holds the encoded input. Registration
may transiently copy or decode input, but a package/cache restore does not move the complete source through WASM linear
memory.

## Static package assets

Widget source keeps using `include_svg!`, `include_bitmap!`, `include_mesh!`, `include_nine_patch!`, `include_skin!`,
and `include_audio!`. No widget-side packaging annotation or wake hook is required.

For a WASM build, each macro emits:

- a fixed `BMCREFV1` reference containing the asset kind and content-derived `PackageAssetId`;
- a framed payload record in the `bmc_assets_v1` custom section.

The reference is passed whole to the host import, which makes every used asset discoverable in the linked module's data
segments. A linker may discard a dependency's unaddressable custom section even though its reference remains live. The
extractor therefore also scans the target profile's dependency rlibs and selects only records matching references in the
linked module. A missing referenced record fails packaging; records from unused dependencies are ignored.

The Nix widget build runs `bmc-wasm-assets extract`. It verifies and deduplicates the selected records, writes payloads
under `lib/assets/v1/<kind>/<id>.asset`, and removes the custom section. Packaging fails if an extracted payload occurs
in any WASM data segment or if the stripped module retains an asset section. The generated wrapper passes the asset
directory to the thin process as `--asset-root`; the host verifies the reference kind and file digest whenever it loads
a package file.

The final package therefore contains the payload once as a file. The shipped WASM contains only the fixed reference and
guest-required metadata, not the full asset. Native storybook builds do not pass through this packaging step and retain
embedded data for their native registrars.

## Registration and IDs

Renderer tags are namespaced per widget instance. Registration and guest eviction imports prepend that namespace, so a
widget cannot register over or evict another widget's assets. A new registration creates a reservation and returns its
opaque `SvgId`, `BitmapId`, or `MeshId`.

Package and cache registration always creates or reuses a suspended reservation. It does not install the renderer
payload merely because the widget requested an ID. During rendering, the host restores a suspended SVG, bitmap, or mesh
when its ID reaches the corresponding renderer draw. Submitted and cached trees use the same draw-time path, without a
separate asset-reference walk.

Volatile registration remains allowed and resident while the slot is dormant for compatibility.

A suspended reservation still owns its ID. Drawing it cannot alias another asset: renderer lookup reports no resident
payload until restoration fills that exact reservation. Destructive eviction removes the reservation and its runtime
backing association, then releases its ID for reuse. Each runtime refuses draw IDs it does not currently own, preventing
a stale tree from drawing an ID reallocated to another widget. Host-reserved SVG IDs are exempt so widgets can use
built-in renderer icons without owning ledger entries for them. A widget remains responsible for not using its own
evicted IDs. Eviction does not delete immutable package files or cache blobs; cache deletion is a separate cache API.

Bitmap and SVG registries reuse released IDs before extending their allocation high-water marks. SVG widget IDs remain
below the host-reserved range. Mesh IDs are one-based storage indices; eviction releases the vacant index and
invalidates atlas slots that cached the old mesh. Pending mesh reservations use the same hole-reuse policy before GPU
initialization. This keeps repeated registration and eviction bounded by the maximum number of simultaneously live
assets.

## Sleep and wake

Slots start dormant. The normal renderable states are `Prepared`, `Entering`, `Visible`, and `Leaving`.

On a renderable-to-dormant edge, the host:

1. invokes `on_sleep` while current assets remain usable;
2. suspends every package- and cache-backed SVG, bitmap, and mesh;
3. leaves volatile assets resident;
4. releases the slot's render target.

Widgets do not need to evict restorable assets in `on_sleep`. `Slot::evict()` and `evict_all()` remain explicit,
destructive operations for assets the widget no longer intends to reuse.

On a dormant-to-renderable edge, the host:

1. invokes `on_wake`;
2. forces the first render into the new render target;
3. restores each package/cache ID when rendering reaches a draw that uses it;
4. installs each demanded payload into its existing reservation before issuing that draw.

A package failure is fatal when a draw first demands the asset: the host preserves the suspended reservation and stops
the widget after `on_wake` has run. A cache miss is recoverable. The reservation remains suspended and the widget can
follow its normal source-reconstruction path. Assets that no draw uses stay suspended across wake and subsequent frames.

Two opposite edges can arrive before renderer delivery:

- `SleepThenWake` invokes both hooks in order without suspending payloads that were already resident. Package/cache
  reservations first created by `on_sleep` stay suspended until a draw uses them.
- `WakeThenSleep` keeps the final dormant mutation policy, invokes both hooks, and performs no render. Package/cache
  registration may reserve but does not upload; volatile registration retains its compatibility behavior.

There is no render between either pair of coalesced hooks.

## Cache-backed image widget

`BitmapSlot::set_fit` copies the fetched encoded image into a host decode job. The worker scales it, writes the decoded
RGBA and dimensions to the per-instance cache, and only then reports the result to the runtime. A successful write makes
the bitmap cache-backed.

If the result arrives while the widget is active, the host uploads the decoded pixels immediately and returns the bitmap
ID through `__on_image_ready`. If it arrives while dormant, the host keeps only the suspended reservation and drops the
guest's pending callback entry through `__on_image_dropped`. The image widget's `on_wake` cache check preserves the same
ID on a hit and refetches on a miss.

If the cache write fails while active, the decoded bitmap can remain volatile for that session. If it fails while
dormant, the host does not create an unrestorable resident bitmap.

The cache layout, lifetime, and cross-host garbage collection are documented in
[`asset-cache-gc.md`](asset-cache-gc.md).

## Host-loop GPU scope

The host stages CPU-side delivery readiness for every slot. It enters the cross-process GPU lock and completion-fence
scope only when staged lifecycle or delivery work can access the renderer. Dormant reservations alone therefore add no
GPU lock or fence to an idle loop iteration.

## Observability

Profiling builds log per-kind suspension/restoration counts and renderer resident-byte deltas. These values prove that
the runtime removed owned renderer payloads, but they are not process-RSS measurements:

- bitmap bytes are decoded `width × height × 4` payload estimates;
- mesh bytes are nominal parsed buffer and texture sizes;
- SVG bytes count path-command storage and exclude tessellation caches, map/vector overhead, and shared renderer
  capacity.

Dropping a payload also drops its asset-owned allocations, including SVG path caches, but the allocator or shared GPU
driver may retain capacity. Device free-memory and process RSS can therefore stay flat even when the renderer registry
correctly becomes non-resident.

## Code map

| Area                     | Files                                                                         |
| ------------------------ | ----------------------------------------------------------------------------- |
| asset macros             | `bmc-render/macros/src/lib.rs`                                                |
| package format/extractor | `bmc-wasm-assets/`                                                            |
| package installation     | `nix/wasm-widgets.nix`                                                        |
| runtime backing ledger   | `bmc-wasm-runtime/src/renderer_assets.rs`                                     |
| registration/restoration | `bmc-wasm-runtime/src/runtime/imports/render/assets.rs`, `runtime/backend.rs` |
| renderer reservations    | `bmc-render/src/gpu/svg.rs`, `bitmap.rs`, `mesh.rs`                           |
| lifecycle/GPU delivery   | `bmc-wasm-host/src/main_loop.rs`, `bmc-wasm-runtime/src/runtime/delivery.rs`  |
