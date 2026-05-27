# BMC to BMM Port Specification

Date: 2026-05-27

Scope: define the first implementation slice for running one OpenWrt binary on multiple hardware platforms, starting
with `BMC100` Deck, `BMM100`, `BMM101` BMM, and `BFM100`.

This spec is based on the analysis in `docs/devlogs/BDK-495/bmc-to-bmm-port-analysis.md`.

## Goals

- keep one `bmc-openwrt` binary that selects hardware behavior at runtime
- make display geometry and hardware capabilities product-agnostic
- replace stored config use of Deck-specific widget size variants with product-agnostic widget placement; keep the
  existing Deck-specific scene-management gRPC/frontend `WidgetSize` compatibility surface in this slice
- make `manifest.json` advertise supported widget viewport constraints by viewport shape, width range, height range, and
  DPI range
- expose widget display data as geometry, not product-specific size variants
- support `BMC100` Deck behavior unchanged
- support `BMM100` and `BMM101` BMM platforms with rectangular display information, no combined scene grid, and no LED
  strip
- represent `BFM100` as the only round display platform in the initial platform table
- make unsupported features explicit through capabilities instead of relying on missing device nodes or frontend-only
  hiding

## Non-Goals

- do not redesign combined scenes for BMM
- do not make the combined scene editor support arbitrary grids in this slice
- do not remove `SizeVariant` from the legacy widget SDK (`widget_size()`) in this slice; replacing stored placement
  values is separate from the SDK compatibility shim
- do not require exact DPI values in `manifest.json`; widgets may use DPI ranges
- do not redesign LED settings UI in this slice unless required by backend capability handling
- do not add product-specific widget APIs

## Widget Placement and Manifest Constraints

There are two separate concepts that should not share the same enum:

- scene placement: where the configured widget is placed in the scene
- manifest viewport constraint: which viewport shape, viewport dimensions, and DPI range the widget binary supports

Scene config should stop using `small`, `medium`, `large`, and `full` as stored size values. Those names are
Deck-specific labels for concrete placements. Store placement instead:

```rust
pub enum WidgetPlacement {
    Fullscreen,
    SlotSpan(SlotSpan),
}

pub struct SlotSpan {
    pub columns: u32,
    pub rows: u32,
}
```

The scene-management gRPC API keeps its existing Deck-specific `WidgetSize` fields in this slice and maps them to
placement internally. If the wire API is made geometry-first later, put shared display enums/messages in a shared proto
module, for example `web/shared.proto`, so scene-management and hardware service messages do not declare colliding
symbols in the same package. Equivalent future proto shape:

```proto
// web/shared.proto
enum DisplayShape {
  DISPLAY_SHAPE_UNSPECIFIED = 0;
  DISPLAY_SHAPE_RECTANGULAR = 1;
  DISPLAY_SHAPE_ROUND = 2;
}

message WidgetViewportConstraint {
  DisplayShape display_type = 1;
  uint32 min_width = 2;
  uint32 max_width = 3;
  uint32 min_height = 4;
  uint32 max_height = 5;
  uint32 min_dpi = 6;
  uint32 max_dpi = 7;
}

message WidgetPlacement {
  oneof kind {
    FullscreenPlacement fullscreen = 1;
    SlotSpan slot_span = 2;
  }
}

message FullscreenPlacement {}

message SlotSpan {
  uint32 columns = 1;
  uint32 rows = 2;
}
```

Legacy config migration should be deterministic:

| Legacy size | Placement                           |
| ----------- | ----------------------------------- |
| `small`     | `slot_span { columns: 1, rows: 1 }` |
| `medium`    | `slot_span { columns: 2, rows: 1 }` |
| `large`     | `slot_span { columns: 2, rows: 2 }` |
| `full`      | `fullscreen {}`                     |

Frontend labels may remain Deck-friendly for `BMC100`:

| Placement       | BMC100 user label |
| --------------- | ----------------- |
| `slot_span 1x1` | Small             |
| `slot_span 2x1` | Medium            |
| `slot_span 2x2` | Large             |
| `fullscreen`    | Fullscreen        |

On platforms without `slot_grid`, only `fullscreen` placement is offered for now.

`manifest.json` should replace `sizes` with supported viewport constraints. A viewport constraint describes the viewport
shape, viewport width range, viewport height range, and DPI range the widget author supports; it is not a placement.

Recommended JSON shape:

```json
{
  "supported_viewports": [
    {
      "type": "rectangular",
      "min_width": 160,
      "max_width": 1280,
      "min_height": 238,
      "max_height": 480,
      "min_dpi": 1,
      "max_dpi": 65535
    },
    {
      "type": "round",
      "min_width": 480,
      "max_width": 480,
      "min_height": 480,
      "max_height": 480,
      "min_dpi": 1,
      "max_dpi": 65535
    }
  ]
}
```

The example above describes viewport families with inclusive ranges. A widget that can render across the rectangular
product family should not enumerate every known product resolution. Exact viewport support is still representable by
setting equal min and max values, but that should be reserved for widgets that genuinely require one concrete
resolution.

The JSON field is named `type` for manifest readability. The Rust struct in `bmc-widget-manifest` names the field
`viewport_shape` (with `#[serde(rename = "type")]`) so the type names reflect the semantic. The protobuf field stays
`display_type` to avoid a frontend-visible rename; it carries the same enum values.

The backend derives an actual viewport descriptor from active hardware capabilities and scene placement:

- `fullscreen` maps to active display shape and logical display size
- `slot_span` maps through the active slot-grid layout to a concrete widget viewport size and active display shape
- when `slot_grid` is absent, `slot_span` placement is invalid
- a widget can be added only if its manifest contains at least one constraint matching the derived descriptor

A constraint matches a derived descriptor when the display type is equal and all descriptor values are inside their
inclusive ranges: `min_width <= width <= max_width`, `min_height <= height <= max_height`, and
`min_dpi <= dpi <= max_dpi`.

For this slice, `slot_span` is not an arbitrary rectangle. Only the existing `BMC100` spans are valid:

| Slot span | Derived descriptor          |
| --------- | --------------------------- |
| `1x1`     | `rectangular 317x238 dpi=1` |
| `2x1`     | `rectangular 638x238 dpi=1` |
| `2x2`     | `rectangular 638x480 dpi=1` |

Reject other slot spans, including `1x2`, `3x1`, and `4x2`. Future arbitrary-grid support can widen this table later.

Add-time validation is not enough because stored scenes can become unsupported when the active platform changes. Scene
startup, cycling, and activation must revalidate the active viewport descriptor against the widget manifest constraints:

- unsupported fullscreen scenes are excluded from widget spawning
- unsupported fullscreen scenes are excluded from scene cycling
- unsupported fullscreen scenes cannot become the active scene
- if the stored config contains only unsupported fullscreen and unsupported combined scenes, start with no active scene
  and no spawned widgets

For initial `BMC100` compatibility, old manifest `sizes` map to exact rectangular viewport constraints with
`min_* == max_*` and `min_dpi == max_dpi == 1`:

| Legacy manifest size | Constraint                                |
| -------------------- | ----------------------------------------- |
| `small`              | `rectangular width=317 height=238 dpi=1`  |
| `medium`             | `rectangular width=638 height=238 dpi=1`  |
| `large`              | `rectangular width=638 height=480 dpi=1`  |
| `full`               | `rectangular width=1280 height=480 dpi=1` |

New manifests should use `supported_viewports`. During migration, accepting old `sizes` is allowed only as a
compatibility parser path that normalizes into exact `supported_viewports` constraints. The generated web data may keep
exposing supported Deck size labels while the scene-management gRPC API remains compatibility-first; the backend derives
those labels from `supported_viewports`.

Manifest validation rules:

- `supported_viewports` must be non-empty after compatibility normalization
- provided min and max values for width, height, and DPI must be nonzero; omitted bounds are open-ended
- each viewport constraint must satisfy `min_width <= max_width`, `min_height <= max_height`, and `min_dpi <= max_dpi`
  when both bounds are present
- each viewport constraint must use a concrete viewport shape, not `DISPLAY_SHAPE_UNSPECIFIED`
- duplicate viewport constraints are invalid; viewport shape and all six min/max fields define identity
- a manifest must not provide both legacy `sizes` and new `supported_viewports`
- invalid legacy `sizes` values remain invalid; compatibility normalization is only for recognized legacy values

## Platform Detection

`bmc-openwrt` reads the BOS platform directly from `/etc/bos_platform`. `bmc_platform::BmcInfo::load` already does this
(`fs::read_to_string` + `BosPlatform::from_str`), with no shell. Detection is exposed on the `BmcManager` trait as
`platform() -> BosPlatform`, so it has a different implementation per binary: bmc-openwrt returns the detected value;
bmc-mock returns a config-selected one.

`/etc/bos_platform` holds the string bos-main builds as `${mpu}-${build}-${iface}` (see `braiins-os-plus/firmware.nix`):

| `/etc/bos_platform`    | `BosPlatform` | `Product` |
| ---------------------- | ------------- | --------- |
| `stm32mp157c-ii3-bmc1` | `Bmc1`        | `Bmc100`  |
| `stm32mp157c-ii1-am2`  | `Am2`         | `Bmm100`  |
| `stm32mp157c-ii2-bmm1` | `Bmm1`        | `Bmm101`  |
| `stm32mp157c-ii4-bfm1` | `Bfm1`        | `Bfm100`  |

`BosPlatform::from_str` parses the string into the platform; `BosPlatform::product()` maps the platform to its
`Product`. The mapping is 1:1 today but a future platform may resolve to several products (e.g. via an EEPROM read), so
platform and product stay distinct types.

For development and recovery, `bmc-openwrt` accepts an override:

```text
--hardware-profile BMC100|BMM100|BMM101|BFM100|auto
```

`auto` uses the detected platform. The operator-facing codes are products; the override maps each to the representative
platform. If `/etc/bos_platform` cannot be read and no override is given, bmc-openwrt fails loudly instead of defaulting
to a possibly wrong hardware profile.

## Platform Table

The first implementation uses an explicit per-product table in `bmc-platform`.

| BOS platform           | Product  | Logical display | Advertised mode | Visible area   | Scanout transform | Display shape | Slot grid | LED strip       |
| ---------------------- | -------- | --------------- | --------------- | -------------- | ----------------- | ------------- | --------- | --------------- |
| `stm32mp157c-ii3-bmc1` | `BMC100` | `1280x480`      | `600x1280`      | `0,0 480x1280` | `270 deg`         | rectangular   | `4x2`     | APA102, 10 LEDs |
| `stm32mp157c-ii1-am2`  | `BMM100` | `320x240`       | `320x240`       | `0,0 320x240`  | `0 deg`           | rectangular   | none      | none            |
| `stm32mp157c-ii2-bmm1` | `BMM101` | `480x320`       | `480x320`       | `0,0 480x320`  | `0 deg`           | rectangular   | none      | none            |
| `stm32mp157c-ii4-bfm1` | `BFM100` | `480x480`       | `480x480`       | `0,0 480x480`  | `90 deg`          | round         | none      | none            |

The `BMC100` row represents current behavior:

- logical display resolution: `1280x480`
- advertised mode: `600x1280`
- visible area: `0,0 480x1280`
- scanout transform: `270 deg`
- display shape: rectangular
- slot grid: `4x2`
- combined scenes: enabled
- LED strip: APA102 on the current SPI path, 10 LEDs

The `BMM100` row represents BMM behavior:

- logical display resolution: `320x240`
- advertised mode: `320x240`
- visible area: `0,0 320x240`
- scanout transform: `0 deg`
- display shape: rectangular
- slot grid: absent
- combined scenes: disabled
- LED strip: absent

The `BMM101` row represents BMM behavior:

- logical display resolution: `480x320`
- advertised mode: `480x320`
- visible area: `0,0 480x320`
- scanout transform: `0 deg`
- display shape: rectangular
- slot grid: absent
- combined scenes: disabled
- LED strip: absent

The `BFM100` row represents the only round display in the initial table:

- logical display resolution: `480x480`
- advertised mode: `480x480`
- visible area: `0,0 480x480`
- scanout transform: `90 deg`
- display shape: round
- slot grid: absent
- combined scenes: disabled
- LED strip: absent

## Rust Hardware Model

The model lives in `bmc-platform` (alongside the existing `BosPlatform`), shared by both binaries and the compositor.
`BosPlatform` is the detected platform; `Product` is the unit the profile table keys on.

Recommended Rust shape:

```rust
pub struct HardwareProfile {
    pub product: Product,
    pub display: DisplayProfile,
    pub slot_grid: Option<SlotGrid>,
    pub led_strip: Option<LedStripProfile>,
    pub paths: PlatformPaths,
}

pub enum Product {
    Bmc100,
    Bmm100,
    Bmm101,
    Bfm100,
}

pub struct DisplayProfile {
    pub logical_width: u32,
    pub logical_height: u32,
    pub advertised_width: u32,
    pub advertised_height: u32,
    pub shape: DisplayShape,
    pub dpi: u32,
    pub scanout_transform: DisplayTransform,
    pub touch_transform: TouchTransform,
    pub visible_area: VisibleArea,
    pub seam_overlap_px: i32,
}

pub enum DisplayShape {
    Rectangular,
    Round,
}

pub enum DisplayTransform {
    Deg0,
    Deg90,
    Deg270,
}

pub enum TouchTransform {
    Deg0,
    Deg90,
    Deg270,
}

pub struct VisibleArea {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct SlotGrid {
    pub columns: usize,
    pub rows: usize,
}

pub struct LedStripProfile {
    pub kind: LedStripKind,
    pub device: PathBuf,
    pub led_count: usize,
}

pub enum LedStripKind {
    Apa102,
}

pub struct PlatformPaths {
    pub backlight: Option<PathBuf>,
    pub scanout_node: PathBuf,
    pub render_node: PathBuf,
}
```

Use `Option` consistently to represent hardware that is absent from a platform. When hardware exists, describe the
concrete hardware type and parameters instead of a generic availability flag.

`touch_transform` is the residual per-panel touch rotation described in the Touch Behavior section. `seam_overlap_px` is
the pixel overlap applied to neighboring scenes during a scene-swipe transition; it hides the edge-sampling gap the
GC400 GPU leaves at the seam under rotated scanout. `BMC100` uses `4`; platforms without rotated scanout use `0`.

## Display Semantics

All widget-facing display dimensions should be logical display dimensions after the visible-area crop. Advertised mode
and scanout transform are internal compositor/hardware data.

Definitions:

- advertised mode: the mode dimensions reported to the compositor before visible-area crop and before scanout transform
- visible area: the rectangular subregion of the advertised mode that is actually visible
- logical display size: the visible area after the scanout transform's axis swap is applied; used by the compositor,
  widgets, and frontend capabilities
- scanout transform: the transform relating logical and panel coordinate spaces; its axis swap determines the logical
  display size, and the same transform rotates the composed buffer at scanout
- widget viewport: the drawable rectangle assigned to one widget
- display shape: the visible display shape, currently `rectangular` or `round`
- DPI: display density for layout decisions; exact values are intentionally deferred until panel active-area data is
  available

Initial platform display values:

| BOS platform | Advertised mode | Visible area   | Logical display | Scanout transform | Shape       |
| ------------ | --------------- | -------------- | --------------- | ----------------- | ----------- |
| `BMC100`     | `600x1280`      | `0,0 480x1280` | `1280x480`      | `270 deg`         | rectangular |
| `BMM100`     | `320x240`       | `0,0 320x240`  | `320x240`       | `0 deg`           | rectangular |
| `BMM101`     | `480x320`       | `0,0 480x320`  | `480x320`       | `0 deg`           | rectangular |
| `BFM100`     | `480x480`       | `0,0 480x480`  | `480x480`       | `90 deg`          | round       |

The display pipeline is: read advertised mode, apply the visible-area crop in advertised-mode coordinate space, compute
the logical display size as the visible area with the scanout transform's axis swap applied, render widgets in that
logical coordinate space, then rotate the composed buffer by the scanout transform when presenting it to scanout. The
transform appears in two distinct roles: a dimension swap that derives the logical size, and the pixel rotation at
present that drives the panel. `BMC100` is the only initial platform whose logical display differs from the advertised
mode because it crops the advertised width from `600` to `480` to get a visible area of `480x1280`, then the `270 deg`
axis swap exposes that as logical `1280x480`.

DPI stays in the capability model. Until panel active areas are known, all initial platforms should report `dpi = 1` as
an explicit fake value. Widgets should treat DPI as advisory during this slice and must not rely on exact DPI for
correctness.

## Compositor Capability Source

The compositor should be the authority that exposes hardware-neutral display and feature capabilities to `bmc` core.

Add shared capability types in `bmc-platform` (next to `BosPlatform`), so higher layers do not depend on
OpenWrt-specific profile internals. These are the *domain* capabilities the compositor hands to `bmc` core for display
sizing and slot-grid gating; they are distinct from the gRPC `web::HardwareCapabilities`, which is a minimal projection
carrying only `combined_scenes_supported` (see the gRPC section). The same short name is used for two layers on purpose,
but only the bool crosses the wire:

```rust
pub struct HardwareCapabilities {
    pub display: DisplayInfo,
    pub slot_grid: Option<SlotGrid>,
}

pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub shape: DisplayShape,
    pub dpi: u32,
}

pub enum DisplayShape {
    Rectangular,
    Round,
}

pub struct SlotGrid {
    pub columns: usize,
    pub rows: usize,
}
```

`None` means no slot grid or no LED strip. The capability and profile types share the `bmc-platform` crate, so
`SlotGrid` and `DisplayShape` are literally the same types on both boundaries. LED strip details (kind, device, count)
stay on the profile only; the capability snapshot intentionally omits them, since nothing downstream of the compositor
consumes LED capability.

Extend the compositor trait with a capability accessor (the type is the `bmc_platform` one, brought into scope with
`use`):

```rust
fn hardware_capabilities(&self) -> HardwareCapabilities;
```

Both binaries build it from `HardwareProfile::for_product(manager.platform().product()).capabilities()`; the mock's
product follows its configured platform.

## OpenWrt Startup Flow

The startup flow should be:

1. parse command-line arguments
2. derive the hardware profile from `manager.platform().product()` (CLI override feeds `platform()`)
3. construct the compositor with the selected display profile
4. construct hardware drivers from selected profile capabilities and paths
5. pass the compositor into `bmc` entrypoint as today

The profile should reach all places that currently contain Deck constants:

- advertised mode, visible-area crop, and scanout transform
- headless compositor display dimensions
- scene renderer physical/logical dimensions
- touch transform policy
- LED driver construction
- compositor-to-widget Wayland initial state used by `bmc-wasm-host` for scratch sizing

## Widget Protocol

The widget protocol should continue to send widget viewport dimensions in the initial configure batch.

Fullscreen widget viewport sizing is platform-specific. Ownership stays in `bmc::Coordinator`, which already computes
widget position, size, and `WidgetInitialConfig.width/height` before calling `register_widget`. For a fullscreen scene,
the coordinator/scene-layout path should derive the viewport from `compositor.hardware_capabilities().display` and
register exactly one widget at `(0, 0)` with viewport size equal to the active platform logical display size:

| Platform | Fullscreen widget viewport |
| -------- | -------------------------- |
| `BMC100` | `1280x480`                 |
| `BMM100` | `320x240`                  |
| `BMM101` | `480x320`                  |
| `BFM100` | `480x480`                  |

Combined scenes remain available only when `slot_grid` is present. On platforms without a slot grid, no slot-span widget
viewport should be spawned.

Add display information to the same initial batch, carry the widget's viewport shape on `configure`, and drop the
obsolete `size_type` argument (the SDK becomes geometry-first; the legacy `widget_size()` shim is computed from
`width`/`height`):

```text
configure(width, height, viewport_shape)
display_info(width, height, shape, dpi)
params(...)
setting(...)
configure_done()
```

The `display_info` event arg names drop the redundant `display_` prefix (the event itself already says it's about the
display).

The Wayland XML declares two enums:

- `display_shape` — referenced by `display_info.shape` (panel shape).
- `viewport_shape` — referenced by `configure.viewport_shape` (the widget's render-region shape).

Both enums declare the same two variants today (`rectangular`, `round`) but are kept separate so viewport shapes can
grow independently of panel shapes in future versions of the protocol. The generated Rust types from wayland-scanner
produce two distinct enums.

Protocol requirements:

- add `display_info` to the current development protocol without a version bump
- always send `display_info` before `configure_done`
- populate `display_info` from the active compositor hardware profile
- extend `configure` with a 4th `viewport_shape` argument; populate it from the per-widget `viewport_shape` stored in
  scene config (`bmc::scene::Widget.viewport_shape`)
- the backend `scene_management` gRPC service fills `Widget.viewport_shape` from `caps.display.shape` on widget
  add/update; the frontend does not see or send the field
- old clients do not consume `display_info` and the new `configure` argument; the protocol changes are not
  version-bumped during this development stage

## WASM Runtime and SDK

The new SDK model should be geometry-first.

Expose:

```rust
pub struct WidgetViewport {
    pub width: u32,
    pub height: u32,
    pub shape: ViewportShape,
}

pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub shape: DisplayShape,
    pub dpi: u32,
}

pub enum DisplayShape {
    Rectangular,
    Round,
}

pub enum ViewportShape {
    Rectangular,
    Round,
}

pub fn widget_viewport() -> WidgetViewport;
pub fn display_info() -> DisplayInfo;
```

Two enums keep panel and viewport vocabularies separated so they can diverge in the future even though today they share
the same variants.

Widgets that need to adapt to a round render region should branch on `viewport_shape` (their own region); widgets that
need to mask the panel as a whole should branch on `display.shape`:

```rust
let viewport = widget_viewport();
let display = display_info();

match viewport.shape {
    ViewportShape::Round => render_round(viewport, display),
    ViewportShape::Rectangular => render_rectangular(viewport, display),
}
```

`SizeVariant` should become legacy compatibility for existing widgets. It may continue mapping dimensions to `Small`,
`Medium`, `Large`, and `Full` inside the old `widget_size()` API, but new widgets should not depend on it and the new
protocol additions should not encode it.

Legacy `SizeVariant` classification must be deterministic for non-BMC fullscreen dimensions. Exact BMC100 matches keep
their current variants. Unknown dimensions should map to the closest BMC100 variant by normalized distance:

```text
distance = abs(width - variant_width) / variant_width
         + abs(height - variant_height) / variant_height
```

If two variants tie, choose the variant with the larger area. Required compatibility mappings:

| Viewport   | Legacy variant |
| ---------- | -------------- |
| `1280x480` | `Full`         |
| `638x480`  | `Large`        |
| `638x238`  | `Medium`       |
| `317x238`  | `Small`        |
| `320x240`  | `Small`        |
| `480x320`  | `Large`        |
| `480x480`  | `Large`        |

## WASM Host Display Maximum

`bmc-wasm-host` may keep a fixed Deck-maximum render scratch buffer for this slice. `BMM100`, `BMM101`, and `BFM100`
viewports fit inside `1280x480`, and smaller platforms render into a sub-region.

There should be no environment-variable or CLI side channel for active display geometry. The source of truth for widget
viewport and display info remains the Wayland initial state from the compositor via
`DeckWidgetSurfaceClient::connect_with_fd`; the fixed scratch size is only a capacity limit, not the active viewport.

## Web Capability API

Expose a small product-agnostic capability API to the frontend. Add a new hardware capabilities service rather than
overloading scene management or frontend constants.

Wire shape:

```proto
service HardwareService {
  rpc GetHardwareCapabilities(google.protobuf.Empty)
      returns (HardwareCapabilities);
}

message HardwareCapabilities {
  bool combined_scenes_supported = 1;
}
```

The frontend needs only `combined_scenes_supported`. Display geometry, slot-grid dimensions, and LED strip stay
backend-internal; the backend derives the boolean from the active platform's slot grid.

Backend slot-grid semantics (not on the wire):

- columns is the number of columns, rows the number of rows
- `4x2` means four columns and two rows; `BMC100` uses `4x2`
- a platform with no slot grid reports `combined_scenes_supported = false`
- valid positions satisfy `0 <= col < columns` and `0 <= row < rows`

The backend models the slot grid as `Option<SlotGrid>`, which keeps "no grid" distinct from a malformed zero-sized grid.

## Scene Support Enforcement

Combined scene support should be derived from slot grid capability, not from product names. Fullscreen scene support
should be derived from the active fullscreen viewport descriptor and the widget manifest constraints.

Minimal `BMC100` behavior:

- `4x2` slot grid enables combined scenes
- existing `1x1`, `2x1`, `2x2`, and fullscreen viewport dimensions remain unchanged
- frontend may keep showing the user labels Small, Medium, Large, and Fullscreen for these placements
- existing frontend combined editor remains fixed to `4x2`

Minimal `BMM100` behavior:

- no slot grid disables combined scenes
- frontend hides the combined scene add path
- backend rejects combined scene creation and mutation
- existing combined scene config entries are tolerated enough to boot, but not spawned, cycled, or activated

Minimal `BMM101` behavior:

- no slot grid disables combined scenes
- frontend hides the combined scene add path
- backend rejects combined scene creation and mutation
- existing combined scene config entries are tolerated enough to boot, but not spawned, cycled, or activated

Minimal `BFM100` behavior:

- no slot grid disables combined scenes
- frontend hides the combined scene add path
- backend rejects combined scene creation and mutation
- widgets receive `480x480`, shape `round`, and the platform DPI through `display_info()`

Backend enforcement:

- reject `AddCombinedScene` when slot grid is absent
- reject add, update, move, and remove operations for widgets inside combined scenes when slot grid is absent
- exclude unsupported combined scenes from startup widget spawning
- exclude unsupported combined scenes from scene cycling
- prevent unsupported combined scenes from active-scene selection
- exclude unsupported fullscreen scenes from startup widget spawning, scene cycling, and active-scene selection when
  their widget manifest constraints do not match the active platform's fullscreen viewport descriptor
- if the stored config contains only unsupported combined scenes and unsupported fullscreen scenes, start with no active
  scene and no spawned widgets rather than spawning unsupported widget sizes
- keep `GetScenes` returning stored scenes so user config does not silently disappear

Frontend behavior:

- load hardware capabilities before rendering display scene actions
- hide "Combined Scene" when `slot_grid` is absent
- block or redirect the combined editor route when `slot_grid` is absent
- keep existing `4x2` drag/drop logic for `BMC100`

This slice does not need to compute widget pixel sizes from arbitrary slot grids. If a later product has a non-`4x2`
grid, define that as a separate frontend/backend layout project.

## LED Strip Capability

LED strip support should be a hardware capability.

Minimal behavior:

- `BMC100` exposes `LedStrip { kind: APA102, led_count: 10 }` and creates the APA102 driver as today
- `BMM100`, `BMM101`, and `BFM100` create a disabled LED loop instead of the APA102 worker
- the disabled LED loop receives LED commands and ignores them
- startup may still construct `LedController`, register LED settings/test paths, and expose widget/WASM LED request
  paths in this slice
- widget and WASM LED requests on platforms without `led_strip` must not fail the widget; they should be consumed by the
  disabled LED loop
- frontend LED settings can be gated in a later slice, but they should have a real capability source

## Touch Behavior

The hardware profile carries an explicit touch transform policy per platform.

The GT911 controller reports its `ABS_X` / `ABS_Y` axes already aligned with the logical landscape orientation the
widget tree paints into, so a touch sample is scaled directly against the logical display dimensions. The profile's
`touch_transform` then applies any residual per-panel rotation on top of that scaling, and is the identity (`Deg0`) for
every platform whose touch axes already match the logical orientation:

| Platform | Touch transform |
| -------- | --------------- |
| `BMC100` | `Deg0`          |
| `BMM100` | `Deg0`          |
| `BMM101` | `Deg0`          |
| `BFM100` | `Deg90`         |

`BMC100` uses the identity even though its scanout transform is `270 deg`: the touch controller is landscape-native, so
touch must not be rotated to match scanout. The `BFM100` `Deg90` value is the round-panel default and has not yet been
confirmed on hardware.

For this slice:

- widgets continue receiving rectangular coordinate space
- round display shape is metadata for layout, not a different coordinate system
- touch coordinates are transformed into logical display coordinates
- all initial platforms have GT911 touch input advertised as a normal Linux input device
- clipping touches outside the `BFM100` round visible area is not required for the first slice

## Test Plan

Add focused tests around capability and data-flow boundaries:

- hardware profile mapping: `BMC100` maps to advertised mode `600x1280`, visible area `0,0 480x1280`, logical
  `1280x480`, `4x2`, APA102 with 10 LEDs
- hardware profile mapping: `BMM100` maps to rectangular `320x240`, transform `0 deg`, absent slot grid, and absent LED
  strip
- hardware profile mapping: `BMM101` maps to rectangular `480x320`, transform `0 deg`, absent slot grid, and absent LED
  strip
- hardware profile mapping: `BFM100` maps to round `480x480`, transform `90 deg`, absent slot grid, and absent LED strip
- platform detection: nonzero `bos_platform` command fails profile detection
- platform detection: unknown platform fails unless explicit development fallback is enabled
- compositor capabilities expose the selected profile
- manifest matching accepts a derived viewport descriptor inside an inclusive `supported_viewports` range without
  requiring an exact product-resolution entry
- scene management rejects combined scene creation when slot grid is absent
- scene management does not spawn, cycle, or activate combined scenes when slot grid is absent
- scene management does not spawn, cycle, or activate fullscreen scenes whose manifest does not support the active
  platform fullscreen viewport
- scene management starts with no active scene when all stored scenes are unsupported for the active platform
- widget protocol initial state includes display info before `configure_done` and a `viewport_shape` argument on the
  `configure` event
- scene_management stamps `bmc::scene::Widget.viewport_shape` from `caps.display.shape` on widget add/update; the
  frontend does not see the field
- fullscreen scenes register one widget at the active platform logical display size
- WASM SDK exposes `widget_viewport()` (carrying viewport shape) and `display_info()` (carrying panel shape)
- existing `widget_size()` users still work through compatibility behavior
- legacy closest-variant mapping returns `Small` for `320x240` and `Large` for `480x320` and `480x480`
- `bmc-wasm-host` keeps its fixed Deck-maximum scratch allocation while taking active widget viewport/display geometry
  from Wayland initial state
- frontend hides combined scene creation when slot grid is absent
- frontend blocks or redirects the combined editor route when slot grid is absent

## Verification Targets

The slice is complete when:

- `BMC100` preserves current Deck behavior
- `BMM100` reports logical display `320x240`, rectangular shape, transform `0 deg`, absent slot grid, and absent LED
  strip
- `BMM101` reports logical display `480x320`, rectangular shape, transform `0 deg`, absent slot grid, and absent LED
  strip
- `BFM100` reports logical display `480x480`, round shape, transform `90 deg`, absent slot grid, and absent LED strip
- `bmc-wasm-host` no longer uses Deck constants as the active viewport source of truth; the fixed scratch allocation may
  remain Deck-maximum sized
- widgets can read viewport and display information without `SizeVariant`
- old widgets using `widget_size()` still run
- fullscreen widgets on `BMM100` receive a `320x240` viewport
- fullscreen widgets on `BMM101` receive a `480x320` viewport
- fullscreen widgets on `BFM100` receive a `480x480` viewport
- frontend combined-scene controls are hidden on `BMM101`
- backend combined-scene RPCs are rejected on `BMM101`
- `bmc-openwrt` does not attempt to open the APA102 SPI device on `BMM100`
- `bmc-openwrt` does not attempt to open the APA102 SPI device on `BMM101`
- `bmc-openwrt` does not attempt to open the APA102 SPI device on `BFM100`

## Defrerred Decisions

- exact DPI values for `BMC100`, `BMM100`, `BMM101`, and `BFM100`
