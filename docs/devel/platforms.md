# Supported Platforms

BDK-495 makes one `bmc-openwrt` binary choose platform behavior at runtime. The current source of truth is
`bmc-platform`: it parses the BOS platform string, maps that platform to a product, and expands the product into a
`HardwareProfile`.

## Platform Detection

`bmc-openwrt` defaults to automatic detection:

```text
--hardware-profile auto
```

With `auto`, `bmc_platform::BmcInfo::load` reads `/etc/bos_platform`, trims the file, and parses the value as
`BosPlatform`. `Manager::platform()` then returns the detected platform. If detection was not available and no override
was provided, `bmc-openwrt` fails loudly through a `BUG:` expectation rather than silently assuming Deck hardware.

For development and recovery, `bmc-openwrt` accepts these product-code overrides:

```text
--hardware-profile BMC100
--hardware-profile BMM100
--hardware-profile BMM101
--hardware-profile BFM100
```

The override maps to the representative `BosPlatform`, then follows the same product/profile path as autodetection.
`bmc-mock` uses the same parser, but its `auto` value falls back to `BMC100`.

## Platform And Product Mapping

`BosPlatform` is the detected BOS/platform identity. `Product` is the hardware profile key used by the application. They
are one-to-one today, but they remain separate types so future platform strings can resolve to products with extra
runtime identification if needed.

| `/etc/bos_platform`    | `BosPlatform` | Product  | CLI override |
| ---------------------- | ------------- | -------- | ------------ |
| `stm32mp157c-ii3-bmc1` | `Bmc1`        | `BMC100` | `BMC100`     |
| `stm32mp157c-ii1-am2`  | `Am2`         | `BMM100` | `BMM100`     |
| `stm32mp157c-ii2-bmm1` | `Bmm1`        | `BMM101` | `BMM101`     |
| `stm32mp157c-ii4-bfm1` | `Bfm1`        | `BFM100` | `BFM100`     |

Only `Bmc1` currently maps to an upgrade-index platform. Upgrade checks for the BMM/BFM platforms report that no upgrade
asset exists.

## Hardware Profiles

`HardwareProfile::for_product` contains the per-product display profile, optional slot grid, optional LED strip, and
platform paths. Hardware that is not present is modeled as `None`, not as a boolean.

| Product  | Logical display | Shape       | Scanout | Touch  | Slot grid | LED strip       |
| -------- | --------------- | ----------- | ------- | ------ | --------- | --------------- |
| `BMC100` | `1280x480`      | rectangular | 270 deg | 0 deg  | `4x2`     | APA102, 10 LEDs |
| `BMM100` | `320x240`       | rectangular | 0 deg   | 0 deg  | none      | none            |
| `BMM101` | `480x320`       | rectangular | 0 deg   | 0 deg  | none      | none            |
| `BFM100` | `480x480`       | round       | 90 deg  | 90 deg | none      | none            |

Display details:

| Product  | Advertised mode | Visible area   | Logical display | Seam overlap |
| -------- | --------------- | -------------- | --------------- | ------------ |
| `BMC100` | `600x1280`      | `0,0 480x1280` | `1280x480`      | `4px`        |
| `BMM100` | `320x240`       | `0,0 320x240`  | `320x240`       | `0px`        |
| `BMM101` | `480x320`       | `0,0 480x320`  | `480x320`       | `0px`        |
| `BFM100` | `480x480`       | `0,0 480x480`  | `480x480`       | `0px`        |

Definitions:

- Advertised mode is the DRM mode dimensions before visible-area crop and scanout transform.
- Visible area is the subrectangle of the advertised mode that is actually visible.
- Logical display size is the visible area after the scanout transform's axis swap. The compositor, widgets, and display
  capability code use logical dimensions.
- Scanout transform is the rotation applied when the composed logical buffer is presented to the panel.
- DPI is part of the model, but every current platform reports `dpi = 1`. Treat it as an advisory placeholder until
  panel active-area data is available.

`BMC100` is the only current product whose logical display differs from the advertised mode: it crops the `600x1280`
mode to a `480x1280` visible area, then the 270 degree scanout transform exposes it to the compositor as `1280x480`.

## Capabilities

`HardwareProfile::capabilities()` projects the profile into the smaller `HardwareCapabilities` value used by `bmc` core:

```rust
pub struct HardwareCapabilities {
    pub display: DisplayInfo,
    pub slot_grid: Option<SlotGrid>,
}
```

`display` is the logical display information delivered to widgets. `slot_grid` controls combined-scene support. If
`slot_grid` is `None`, combined scenes are not supported on that hardware.

The public gRPC `HardwareCapabilities` message is intentionally narrower today:

```proto
message HardwareCapabilities {
  bool combined_scenes_supported = 1;
}
```

The backend sets that boolean from `caps.slot_grid.is_some()`. The frontend uses it to hide or redirect the
combined-scene editor, and scene-management RPCs also reject combined-scene operations with `FailedPrecondition` when no
slot grid is available.

## Platform Differences In Current Behavior

`BMC100` keeps the original Deck behavior: fullscreen scenes, combined scenes on the 4x2 slot grid, and LED effects are
available.

`BMM100`, `BMM101`, and `BFM100` are fullscreen-only in the current UI/API surface. They have no slot grid, so combined
scenes are filtered out during startup/cycling and rejected by scene-management RPCs. They also have no LED strip
profile, so the OpenWrt LED driver is disabled for those products.

Widget manifest matching uses the active platform capabilities:

- Fullscreen placement derives a viewport descriptor from the active display: shape, logical width, logical height, and
  DPI.
- Slot-span placement is only meaningful when a slot grid is present. The only current slot descriptors are the BMC100
  spans: `1x1` -> `317x238`, `2x1` -> `638x238`, and `2x2` -> `638x480`, all rectangular with `dpi = 1`.
- A widget is supported only if one of its `supported_viewports` constraints matches the derived descriptor.

The scene-management gRPC API and frontend still expose the legacy `WidgetSize` labels: `Small`, `Medium`, `Large`, and
`Full`. The backend maps those labels to internal placement (`SlotSpan` or `Fullscreen`) and computes the reported
`supported_sizes` from the manifest's `supported_viewports`. This is a compatibility surface for the current editor and
widget layout model, not the hardware model itself.
