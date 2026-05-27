# BMC to BMM Port Analysis

Date: 2026-05-27

Scope: identify where the current BMC application assumes the Braiins Deck display geometry, widget size grid, and LED
strip hardware while preparing a port to BMM.

## Display Geometry

The core scene model is the main source of the current `1280x480` logical display assumption.

- `bmc/src/scene.rs` defines the canonical widget grid as `2` rows by `4` columns.
- `bmc/src/scene.rs` maps semantic widget sizes to fixed dimensions:
  - `small`: `317x238`
  - `medium`: `638x238`
  - `large`: `638x480`
  - `full`: `1280x480`
- `bmc/src/widget/coordinator.rs` converts `WidgetSize` into compositor placement and `WidgetInitialConfig`, so those
  dimensions become the runtime configure sizes seen by widgets.
- `bmc-grpc/proto/web/scene_management.proto` exposes only semantic sizes (`small`, `medium`, `large`, `full`), not
  display dimensions or a layout profile.

The compositor is partly dynamic through DRM, but still has BMC-specific panel assumptions.

- `bmc-openwrt/src/compositor/render/drm_output.rs` reads the DRM mode, but crops a reported `600` pixel physical width
  down to visible width `480`.
- `bmc-openwrt/src/compositor/render/drm_output.rs` always reports logical size as `(height, width)`, assuming a
  portrait physical panel rotated into landscape.
- `bmc-openwrt/src/compositor/egl_compositor.rs` hardcodes headless physical size as `480x1280`, which yields logical
  `1280x480`.
- `bmc-openwrt/src/compositor/scene_renderer.rs` assumes physical `480x1280`, logical `1280x480`, and the current
  rotation transform model.
- `bmc-openwrt/src/compositor/egl_compositor.rs` documents touch calibration as tied to the current panel and
  render-side inverse rotation.

## Widget Size Presets

The same `2x4` grid and semantic size spans are duplicated in the frontend.

- `frontend/src/pages/workspace/Display/fn/const.ts` hardcodes a `2x4` occupancy map and size spans:
  - `small`: `1x1`
  - `medium`: `1x2`
  - `large`: `2x2`
  - `full`: `2x4`
- `frontend/src/pages/workspace/Display/fn/fn.ts` uses those spans in occupancy, insertion, and drag/drop logic.
- `frontend/src/pages/workspace/Display/components/CombinedSceneView/CombinedSceneView.scss` hardcodes `repeat(4)`
  columns and `repeat(2)` rows.
- `frontend/src/pages/workspace/Display/DisplayCombined.scss` caps the display editor at `1280px`.

The WASM tooling repeats the BMC geometry.

- `bmc-wasm-host/src/main.rs` hardcodes the shared render scratch maximum as `1280x480`.
- `bmc-wasm-runtime/src/capture_config.rs` hardcodes capture presets:
  - `full`: `1280x480`
  - `large`: `638x480`
  - `medium`: `638x238`
  - `small`: `317x238`
- `bmc-wasm-runtime/src/bin/testbed/main.rs` repeats the same tile sizes for the hosted testbed.

Widget implementations vary.

- `widgets/digital-clock/ui/main.slint` uses semantic `WidgetSize` to choose fixed font sizes, paddings, and layout
  widths.
- `widgets/flip-clock/src/layout.rs` is more viewport-aware, but its tests lock in the current supported viewport set.
- `widgets/flip-clock/src/main.rs` standalone mode defaults to `640x480`.

## Hardware Drivers

The OpenWrt binary wires concrete BMC hardware directly.

- `bmc-openwrt/src/main.rs` hardcodes the display backlight path as `/sys/class/backlight/display-bl`.
- `bmc-openwrt/src/main.rs` always constructs the APA102 LED driver on `/dev/spidev0.0`.
- `bmc-openwrt/src/main.rs` contains BMC board-specific Wi-Fi sysfs paths and hubbed/hubless detection.
- `bmc-openwrt/src/button_driver.rs` maps only the `reset` uevent button to `ButtonId::Reset`.

LED support is present across backend, UI, widget protocol, and WASM runtime.

- `bmc-led/src/config.rs` hardcodes LED count as `10`.
- `bmc-led/src/apa102_spi/platform_led_driver.rs` opens/configures SPI, uses APA102 LEDs, and uses BGR pixel ordering.
- `bmc/src/startup.rs` always initializes `LedController`.
- `bmc-grpc/proto/web/configuration.proto` exposes LED settings unconditionally.
- `frontend/src/pages/workspace/Settings/components/SectionSoundAndLight/SectionSoundAndLight.tsx` renders LED
  notification controls unconditionally.
- `bmc-widget-protocol/protocol/deck-widget-v1.xml` exposes LED requests to widgets.
- `bmc-wasm-runtime/src/runtime/imports/led.rs` exposes LED imports to WASM widgets, although `RuntimeConfig` can set
  `led_command_sender` to `None`.

If `/dev/spidev0.0` is missing, the APA102 worker logs an error and exits, but the application still has LED settings
and command paths. For BMM, this should be represented as an explicit hardware capability rather than a missing-device
side effect.

## Minimal Multi-Hardware Specification

The first BMM port should keep one OpenWrt binary and select hardware behavior at runtime. The compositor should be the
authority for display geometry and display capabilities, while BMC core and widgets consume hardware-neutral capability
data.

### Hardware Profile

Add the hardware model in `bmc-platform`, selected from the detected platform. The source of truth is
`/etc/bos_platform`, read directly by `bmc_platform::BmcInfo::load` (`fs::read_to_string` + `BmcPlatform::from_str`, no
shell). Detection is exposed on the `BmcManager` trait as `platform() -> BmcPlatform`, so bmc-openwrt and bmc-mock
implement it differently: bmc-openwrt returns the detected value; bmc-mock returns a config-selected one.

For development and recovery, bmc-openwrt accepts an override such as
`--hardware-profile BMC100|BMM100|BMM101|BFM100|auto`. `auto` uses the detected platform; if `/etc/bos_platform` cannot
be read and no override is given, the fallback is the Deck (`Bmc1`) with a warning, preserving current Deck behavior.

The profile should contain at least:

- hardware identity: `BMC100` for Deck, `BMM100`, `BMM101` for BMM, `BFM100`, or later product codes
- logical display size
- physical display size
- display shape: `rectangular` or `round`
- display DPI
- rotation and crop behavior
- touch coordinate transform policy
- widget slot grid: columns and rows, such as `4x2`, or disabled
- LED strip support: APA102 or disabled
- platform paths for backlight, buttons, Wi-Fi, scanout node, render node, and SPI device when present

The `BMC100` Deck profile should describe the current `1280x480` logical rectangular display, advertised `600x1280`
mode, visible area `0,0 480x1280`, `270 deg` transform, the `4x2` slot grid, and APA102 LED strip. The `BMM100` profile
should describe its `160x480` rectangular display with `0 deg` transform, no slot grid, and LED strip disabled. The
`BMM101` BMM profile should describe its `320x480` rectangular display with `0 deg` transform, no slot grid, and LED
strip disabled. The `BFM100` profile should describe the initial round display: `480x480` with `90 deg` transform, no
slot grid, and LED strip disabled.

The first implementation keeps an explicit per-product table in `bmc-platform`, mapping each `/etc/bos_platform` string
(via `BmcPlatform` and `BmcPlatform::product()`) to a hardware profile:

| BOS platform           | Product  | Logical display | Advertised mode | Visible area   | Transform | Display shape | Slot grid | LED strip  |
| ---------------------- | -------- | --------------- | --------------- | -------------- | --------- | ------------- | --------- | ---------- |
| `stm32mp157c-ii3-bmc1` | `BMC100` | `1280x480`      | `600x1280`      | `0,0 480x1280` | `270 deg` | rectangular   | `4x2`     | APA102, 10 |
| `stm32mp157c-ii1-am2`  | `BMM100` | `160x480`       | `160x480`       | `0,0 160x480`  | `0 deg`   | rectangular   | none      | none       |
| `stm32mp157c-ii2-bmm1` | `BMM101` | `320x480`       | `320x480`       | `0,0 320x480`  | `0 deg`   | rectangular   | none      | none       |
| `stm32mp157c-ii4-bfm1` | `BFM100` | `480x480`       | `480x480`       | `0,0 480x480`  | `90 deg`  | round         | none      | none       |

Unknown platform strings fail loudly (`BmcPlatform::from_str` errors); the only fallback is the Deck when
`/etc/bos_platform` is unreadable and no override is set. This avoids silently running Deck hardware assumptions on a
new product.

### Compositor Capabilities

Expose a hardware-neutral capability snapshot from the compositor trait in `bmc`, instead of letting every subsystem
read profile internals. The capability and profile types both live in `bmc-platform`.

The snapshot should include:

- full logical display width and height
- display shape
- display DPI
- widget slot-grid capability

LED-strip details stay on the hardware profile (used to construct the LED driver); the capability snapshot omits them,
since nothing downstream of the compositor consumes LED capability.

`bmc-platform` owns the per-product table and geometry constants, while `bmc` core gates scenes and frontend APIs
through the compositor interface and bmc-openwrt only detects the platform.

### Widget Geometry Model

Widgets should not receive or depend on product-specific semantic size variants. The widget-facing model should be
geometry-first:

- widget viewport width and height
- full display width and height
- display shape: `rectangular` or `round`
- display DPI

The existing widget configure path already sends the widget viewport. Extend the initial configure batch with display
information owned by the compositor:

- `configure(width, height)` continues to mean widget viewport
- `display_info(display_width, display_height, shape, dpi)` describes the full target display
- `params`, `setting`, and `configure_done` keep their current roles

If the wire protocol keeps the existing `size_type` field for compatibility, new SDK code should ignore it. Do not add a
new semantic size variant to the protocol.

The WASM SDK should expose APIs shaped like:

```rust
pub struct WidgetViewport {
    pub width: u32,
    pub height: u32,
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

pub fn widget_viewport() -> WidgetViewport;
pub fn display_info() -> DisplayInfo;
```

Widgets should branch on display properties and actual dimensions:

```rust
let viewport = widget_viewport();
let display = display_info();

match display.shape {
    DisplayShape::Round => render_round(viewport, display),
    DisplayShape::Rectangular => render_rectangular(viewport, display),
}
```

`SizeVariant` should become a legacy SDK convenience for existing Deck widgets. It may continue mapping dimensions to
`Small`, `Medium`, `Large`, and `Full` inside the compatibility API, but new widgets should not use it and the protocol
should not promote it as a hardware abstraction.

### WASM Host Display Limits

`bmc-wasm-host` currently initializes its render scratch buffer from Deck constants before any widget connects. That
should stop being a process-startup constant.

There should be no environment-variable or CLI side channel for display geometry. The host should learn widget viewport
and display information through the Wayland initial state from the compositor, then initialize or resize scratch
resources from that data.

### Slot Grid Capability

Combined scenes should be derived from a product-agnostic slot grid. Hardware that supports combined scenes exposes the
number of widget slots as `width x height`; hardware that does not support combined scenes exposes no slot grid.

Expose this through a small backend capability API so the frontend does not need to infer hardware from display
dimensions or product names.

The web API should expose data equivalent to:

```proto
message HardwareCapabilities {
  bool combined_scenes_supported = 1;
}
```

The Rust representation carries the richer per-platform capabilities (display resolution, shape, DPI, slot grid, LED
strip), but the wire API stays minimal for the first slice: the frontend only needs to know whether combined scenes are
available. Current `BMC100` supports combined scenes; hardware with no slot grid reports
`combined_scenes_supported = false`.

Backend behavior:

- reject `AddCombinedScene` when combined scenes are disabled
- reject combined-scene widget add, update, move, and remove operations when disabled
- prevent disabled combined scenes from becoming the active rendered scene
- tolerate existing config entries enough to boot, but do not activate them on unsupported hardware

Frontend behavior:

- fetch display/scene capabilities from the backend
- hide the "Combined Scene" add menu item when disabled
- block or redirect the combined-scene editor route on unsupported hardware
- keep the existing `4x2` combined editor unchanged for `BMC100`

Frontend hiding is not sufficient on its own. The backend must enforce the same capability because the RPC API can be
called directly.

### LED Strip Capability

The no-LED-strip BMM case should be represented as an explicit hardware capability, not as a missing `/dev/spidev0.0`
side effect.

Minimal behavior:

- Deck profile creates the APA102 driver as today
- BMM profile creates a disabled/no-op LED capability
- backend APIs and UI can initially remain visible if this is outside the first implementation slice, but the capability
  should exist so later UI/API gating has a real source of truth

### First Implementation Slice

The first useful slice is:

1. add hardware profile selection in `bmc-openwrt`
2. expose compositor capabilities to `bmc`
3. gate combined scenes in backend scene-management behavior
4. gate combined scenes in the frontend
5. add widget display information to the compositor-to-widget initial state
6. pass display information through `bmc-widget`, `bmc-wasm-host`, and `bmc-wasm-runtime`
7. add geometry-first SDK APIs and mark `SizeVariant` as legacy compatibility
8. make selected widgets branch on viewport, display shape, and DPI

### Verification Targets

The implementation should be considered complete for this slice when:

- `BMC100` still reports `1280x480`, `rectangular`, and combined-scene support enabled
- `BMM100` reports `160x480`, `rectangular`, and combined-scene support disabled
- `BMM101` reports `320x480`, `rectangular`, and combined-scene support disabled
- `BFM100` reports `480x480`, `round`, and combined-scene support disabled
- `bmc-wasm-host` no longer contains Deck display max constants as the active source of truth
- widgets can read viewport and display information without using `SizeVariant`
- existing widgets using `widget_size()` still work through the compatibility layer
- the frontend does not show the combined-scene add path on BMM
- the backend rejects combined-scene creation and mutation on BMM
- missing APA102 hardware on BMM is represented by profile capability, not by a failed SPI open

## Highest-Risk Files

The highest-risk first files for BMM are:

- `bmc/src/scene.rs`
- `bmc/src/widget/coordinator.rs`
- `bmc-openwrt/src/compositor/render/drm_output.rs`
- `bmc-openwrt/src/compositor/scene_renderer.rs`
- `bmc-openwrt/src/compositor/egl_compositor.rs`
- `bmc-wasm-host/src/main.rs`
- `bmc-wasm-runtime/src/capture_config.rs`
- `frontend/src/pages/workspace/Display/fn/const.ts`
- `frontend/src/pages/workspace/Display/components/CombinedSceneView/CombinedSceneView.scss`

For the no-LED-strip BMM case, the first useful split is probably a null/no-op LED capability in the backend plus
capability-aware UI/API behavior. The current system can survive a missing SPI device, but it does not accurately
communicate that LED control is unavailable.
