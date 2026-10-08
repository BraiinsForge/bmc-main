# Display Scanout

How `bmc-openwrt` turns a composited frame into the buffer the panel scans out, and where the BMM panels need more than
a page flip: a red/blue swap into RGB565 and an optional panel color adjustment.

## Pipeline

The compositor renders every frame into an `XRGB8888` intermediate in natural RGB order. What happens next depends on
the product's `DisplayProfile::pixel_format` (see [Supported Platforms](platforms.md)):

| Pixel format | Products           | Scanout                                                                 |
| ------------ | ------------------ | ----------------------------------------------------------------------- |
| `Xrgb8888`   | `BMC100`, `BFM100` | The intermediate is page-flipped directly.                              |
| `Bgr565`     | `BMM100`, `BMM101` | `ScanoutSwizzler` samples the intermediate into an `RG16` buffer first. |

Screen captures read the intermediate, never the scanout buffer. They show widget design colors in natural RGB order on
every product, without the swap or the panel adjustment. The gallery shows the same, since it renders through
`bmc-render` without the compositor.

## The BGR565 swizzle

The ST7365P panel on BMM products expects red and blue swapped (`B<<11 | G<<5 | R`). Its DRM plane advertises only
`RG16` and `XR24`, so the swap cannot be expressed through the fourcc: the buffer stays tagged `RG16` and the swap lives
in the pixels.

`ScanoutSwizzler` (`bmc-openwrt/src/compositor/render/scanout_swizzle.rs`) is a full-frame GPU pass with a custom
texture shader. It writes `.bgr` of each sampled pixel into a double-buffered `RG16` scanout buffer, and that buffer is
page-flipped instead of the intermediate.

## Panel color adjustment

`DisplayProfile::color_adjustment` brightens midtones on panels where the design's secondary grays read too dark. On
`BMM101`, Gray 60 (`#6F6F6F`) is hard to read at desk distance, while Gray 30 (`#C6C6C6`) is comfortable. Widgets keep
the design colors; the swizzle pass applies the adjustment at scanout.

The curve runs along two line segments through `(shadow_floor, shadow_floor)`, `(input_anchor, output_anchor)` and
`(1, 1)`, and scales all three channels by one gain, which keeps hue and saturation. The rising segment is evaluated at
the pixel's luma, the falling one at its brightest channel, its peak, and the smaller gain wins. A gray has equal luma
and peak, so grays follow the curve exactly. Below the shadow floor the gain is 1, and white and black stay put. The
segment coefficients are computed once at startup, which keeps the per-pixel work to a few arithmetic operations.

Luma drives the rise because a dark tinted fill looks as dark as its luma, not its peak. Lifted by its peak, the dark
red under a falling badge in `ticker-list` brightened far more than its pink text, and the badge lost a third of its
contrast. The falling segment stays on the peak because it caps the gain where the peak reaches 1, so bright accents
never clip.

| Parameter       | Value     | Why                                                                |
| --------------- | --------- | ------------------------------------------------------------------ |
| `shadow_floor`  | `38/255`  | Keeps the 15% ticker trend fills (`#42BE65`, `#FA4D56`) unchanged. |
| `input_anchor`  | `111/255` | Gray 60.                                                           |
| `output_anchor` | `198/255` | Gray 30.                                                           |

Fills whose luma stays below the floor keep their color: the ticker trend fills, the red and darker green badge fills,
and the 30% falling-chart fill in `ticker-list`'s `BMM101` frame, whose luma near the line is about 36/255. A brighter
fill such as the green `#0E3F25` badge is lifted, but its `#34C06A` text gains more luminance, so the contrast still
rises.

`BMM101` was tuned on its panel. `BMM100` shares the same values until its own panel is evaluated. Products without the
swizzle pass have no adjustment: `SceneRenderer` refuses a profile that sets one on an `Xrgb8888` product rather than
dropping it silently.

Several alternatives were tried and rejected. A global lightness boost lifted the faint ticker fills and washed out
saturated accents, turning the weather sun pale. A perceptual Oklab mapping cost 61–143 ms per full-frame pass on the
GC400. Fading the gain out with saturation also kept the badges but added more than twice the luma rise's cost, and an
8-bit lookup table broke the gray ramp. On the GC400 a full-frame swizzle pass takes about 2.8 ms without the curve, 5.7
ms with both segments on the peak, and 6.9 ms with the luma rise.

## Verifying shader changes

The tests in `scanout_swizzle.rs` compile the shader with and without the adjustment on Mesa's llvmpipe, draw probe
colors through it and check them against the curve in Rust. They need headless EGL: run them in the `ci` dev shell
(`nix develop .#ci`), whose `BMC_REQUIRE_HEADLESS_EGL` turns a missing EGL into a failure instead of a silent skip.

The GC400 compiles shaders at draw time. A shader that passes those tests can still fail there, and the failure leaves
the `RG16` scanout buffers black while the intermediate and screen captures look fine. Any change to
`scanout_swizzle.frag` or its uniforms needs a run on a BMM device: confirm the panel shows the expected colors, and
compare the pass timing with the adjustment enabled and disabled. Small shader differences show up in that timing:
splitting the curve into separate float uniforms or applying the unused alpha each cost 0.5–1 ms per frame.
