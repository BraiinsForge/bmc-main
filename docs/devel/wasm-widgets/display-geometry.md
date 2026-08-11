# WASM Widget Display Geometry

Widgets receive geometry from the host. Do not hardcode product dimensions in widget code: read the viewport assigned to
the widget and the logical display it lives on.

## Viewport Versus Display

There are two related rectangles:

- `WidgetViewport` is the drawable area assigned to this widget instance. A fullscreen widget gets the active display
  size. A BMC100 combined-scene widget gets one of the slot-span viewports.
- `DisplayInfo` is the whole logical display. It lets a widget know whether it is running on a rectangular or round
  display and what the full panel size is.

For fullscreen widgets, viewport and display dimensions are usually the same. For combined-scene widgets on BMC100, the
viewport is smaller than the display.

Current platform viewports:

| Platform | Placement  | Viewport shape | Viewport size |
| -------- | ---------- | -------------- | ------------- |
| `BMC100` | fullscreen | rectangular    | `1280x480`    |
| `BMC100` | slot `1x1` | rectangular    | `317x238`     |
| `BMC100` | slot `2x1` | rectangular    | `638x238`     |
| `BMC100` | slot `2x2` | rectangular    | `638x480`     |
| `BMM100` | fullscreen | rectangular    | `320x240`     |
| `BMM101` | fullscreen | rectangular    | `480x320`     |
| `BFM100` | fullscreen | round          | `480x480`     |

Each platform reports its real display density (`BMC100` 217, `BMM100` 141, `BMM101` 165, `BFM100` 229). Treat DPI as
advisory.

## SDK APIs

Use `widget_viewport()` for the actual render target:

```rust
use bmc_wasm_sdk::{render_ui, widget_viewport};

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let viewport = widget_viewport();

    render_ui(
        viewport.width,
        viewport.height,
        // build UI for viewport.shape here
    );
}
```

Use `display_info()` when layout depends on the whole display:

```rust
use bmc_wasm_sdk::{DisplayShape, display_info};

let display = display_info();
let on_round_panel = matches!(display.shape, DisplayShape::Round);
```

`ViewportShape` and `DisplayShape` are separate enums even though both currently have `Rectangular` and `Round`. The
viewport shape describes the widget's render region; the display shape describes the physical/logical display. They can
grow independently.

## Compatibility Size Variant Fallback

The current widget design still has layout code that branches on the old Deck variants: `Small`, `Medium`, `Large`, and
`Full`. For that reason the SDK keeps `widget_size()` and `SizeVariant` as a compatibility API.

`widget_size()` returns the real viewport width and height, plus a `SizeVariant` classified from those dimensions:

```rust
use bmc_wasm_sdk::{SizeVariant, widget_size};

let size = widget_size();
match size.variant {
    SizeVariant::Small => { /* compact layout */ }
    SizeVariant::Medium => { /* wide compact layout */ }
    SizeVariant::Large => { /* large layout */ }
    SizeVariant::Full => { /* Deck fullscreen layout */ }
}
```

The classifier compares the actual viewport to the canonical BMC100 dimensions:

| Variant  | Canonical size |
| -------- | -------------- |
| `Small`  | `317x238`      |
| `Medium` | `638x238`      |
| `Large`  | `638x480`      |
| `Full`   | `1280x480`     |

It chooses the closest canonical size by normalized width/height distance. If two variants are equally close, the
larger-area variant wins so non-BMC100 fullscreen displays do not collapse to the most compact layout.

This fallback is only a layout compatibility shim. New widget code should use `widget_viewport()` and `display_info()`
as the source of truth. Do not use `SizeVariant` to identify the hardware platform, and do not assume a `Large` variant
means the viewport is exactly `638x480`.

## Delivery Path

The host builds geometry before the widget process starts:

1. `bmc` derives a viewport from scene placement and active `HardwareCapabilities`.

2. The coordinator registers a `WidgetInitialConfig` containing viewport width, viewport height, viewport shape, display
   width, display height, display shape, and display DPI.

3. The compositor emits the initial Wayland batch:

   ```text
   configure(width, height, viewport_shape)
   display_info(width, height, shape, dpi)
   params(json)
   setting events...
   configure_done
   ```

4. `bmc-wasm-host` converts that initial config into `WasmWidgetRuntime` geometry.

5. The WASM SDK reads cached host values through imports: `host_widget_size`, `host_widget_viewport_shape`,
   `host_display_size`, and `host_display_shape_dpi`.

The geometry is immutable for a runtime instance. A change that alters viewport size or shape respawns the widget.
Parameter and system-setting updates are delivered in place.

## Testbed

The WASM testbed uses the same geometry model. The platform catalog is `bmc-wasm-runtime/src/platform_catalog.rs` and
covers `BMC100`, `BMM100`, `BMM101`, and `BFM100`. It names the viewport geometries a widget may occupy on each
platform; the display size, shape and DPI behind them come from `bmc_platform::HardwareProfile`, the same source the
device uses.

Run a widget against a specific platform with:

```bash
just wasm::dev <widget-name> "--platform BMM101"
```

Use `params-demo` when checking geometry plumbing. It renders the current viewport and display values on screen.
