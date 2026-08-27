# Image Widget Format Testing

The image widget accepts a dozen source formats, and which of them a build actually decodes is decided by Cargo features
rather than by any code path — so the failure mode is silent and only visible on a device. `deck image-formats` drives
every format through a real Deck and reports which ones decoded.

Run it when touching the `image` feature list in the workspace `Cargo.toml`, the decode paths in
`bmc-wasm-runtime/src/runtime/imports/render/assets.rs` or `bmc-render/src/gpu/bitmap.rs`, or the widget's own
`max_source_pixels` sniff.

```sh
# every enabled decoder, plus the pixel-budget and fetch-cap cases
nix run .#deck -- image-formats --device 192.168.1.2

# one case, leaving it on screen
nix run .#deck -- image-formats --device 192.168.1.2 --only webp --keep-config

# put the previous config back
nix run .#deck -- image-formats --device 192.168.1.2 --restore
```

`--dwell-seconds` sets both the scene cycling duration on the device and the wait, so a shorter run stays coherent.

## The corpus

Fixtures live in `bmc-tui/fixtures/image-formats/` and are served to the device over HTTP by a local server the run
starts. The device fetches them exactly as it would any other URL, so the real fetch path stays under test — what
changes is that the bytes come from this repository rather than a public host, which keeps a DNS filter, a rate limit or
an upstream re-encode from turning into what looks like a decoder regression. See that directory's `README.md` for what
each file exercises and `ATTRIBUTION.md` for its licence.

`CASES` in `bmc-tui/bmc_tui/procedures/image_formats.py` is the only definition: served path, expected magic bytes and
expected outcome per entry, plus an optional builder for the cases that are synthesised rather than stored. Add formats
there.

The run pins the build, verifies the fixtures, backs the device config up, writes one fullscreen scene per case,
restarts the compositor, cycles through them, then reads the wasm host log.

## The budgets being tested

| limit             | value                          | where                                                              |
| ----------------- | ------------------------------ | ------------------------------------------------------------------ |
| fetch cap         | 10 MiB                         | `MAX_FETCH_BODY_BYTES`, `bmc-wasm-runtime/.../background/fetch.rs` |
| pixel budget      | 4,194,304 px (4.0 Mpx)         | `MAX_DECODE_IMAGE_PIXELS`, `bmc-render/src/lib.rs`                 |
| allocation budget | 24 MiB                         | `MAX_DECODE_IMAGE_ALLOC_BYTES`, same file                          |
| JPEG headroom     | 64x the pixel budget (256 Mpx) | `JPEG_SCALE_HEADROOM`, `widgets-wasm/image/src/lib.rs`             |

The fetch cap comes first and dominates: a body over it is refused before any decoder runs. `over-pixel-budget.png`
(3000x3000) and `over-fetch-cap.bmp` (12 MB) therefore sit on opposite sides of it deliberately — one body cannot test
both limits, and conflating them is why an earlier corpus never exercised the pixel budget at all.

Those two are built per run rather than stored: they probe a size limit, so only their length and pixel count matter.
`flat_png` and `flat_bmp` in `image_formats.py` produce them and the asset server serves them on the same paths.

`ImageReader::decode` calls `limits.reserve(decoder.total_bytes())`, and `total_bytes()` is computed in the decoder's
*native* colour type. High-precision formats therefore hit the allocation budget at a lower pixel count than the 8-bit
ones: roughly 2.1 Mpx for HDR (`Rgb32F`) and 3.1 Mpx for farbfeld (`Rgba16`), against the full 4.0 Mpx for 8-bit.

## How an outcome is decided

Two profiling-gated lines in `host_decode_image` carry the verdict, both on the `mesh::profile` target:

| line                                           | emitted                                   | carries                 |
| ---------------------------------------------- | ----------------------------------------- | ----------------------- |
| `host_image_probe WxH px=N data_len=…`         | for every source, before any budget check | what the image measures |
| `host_decode_image WxH data_len=… decode_us=…` | only when a decode actually ran           | cost of that decode     |

A `decode` case must produce both. A `reject-size` case must produce the probe and *not* the decode — the widget applies
its own `max_source_pixels` check and returns `ErrorKind::TooLarge` silently, so the missing second line is the only
evidence that the rejection happened rather than the image quietly failing to arrive. A `reject-body` case produces
neither, and is judged on the fetch status alone.

That status is a `FetchOutcome` wire value, not only an HTTP code: `0` is a network failure, `100`–`599` an origin
reply, and `1000` a body the host refused for exceeding the fetch cap. The harness reports the three separately, because
flattening them hides a refusal behind what reads as an unreachable device.

`vmrss_delta_kb` is a process-wide RSS difference sampled either side of the decode — **not** a peak, and not
attributable to the decode alone. It is an order-of-magnitude hint; a real peak needs isolated runs with high-water
sampling.

A decoder that is not compiled in fails earlier still, at `with_guessed_format`, and logs `host_decode_image probe:` —
that is exactly the regression this widget shipped with when the decoder set was narrowed to PNG/JPEG.

## What guards the evidence

Every one of these exists because the failure it prevents produces a confident, wrong report rather than an obvious
break.

**Fixtures are verified through the serving URL before anything is pushed**, magic bytes and all, so a corrupt or
mis-served file cannot read as a decoder regression. Their served lengths must also be distinct: results are correlated
by body length, so a repeat would make a probe or decode impossible to attribute. Hard fail.

**The build behind the numbers is named.** The decoders live in the independently packaged `bmc-wasm-host`, so checking
the widget alone says nothing about the decoder build. The run builds `.#deck-packages-debug.core.pkg.wasmHost`,
compares that store path with the running host, and warns before measuring a different build.

**The profiling build is proven from the running binary**, by finding a literal that only the instrumented build
contains. Checking the log instead would be worthless: a log outlives the build that wrote it, so a debug-then-release
redeploy leaves stale `mesh::profile` lines that read as proof of a build no longer installed. Hard fail.

**Results are read through a `Device.log_window`**, which stamps the log with a nonce on open and captures on close, so
the window is bounded at both ends. Without a lower bound the harness would attribute a previous run's lines to this
one; without an upper bound it would sweep in whatever happened afterwards. If the stamp is missing on close — rotation,
truncation — it refuses to report at all rather than risk stale evidence. Hard fail.

**The binary's store path is compared before and after**, so a redeploy mid-run cannot blend two builds into one table.
Hard fail.

## Gaps

**DDS is deliberately absent.** It is a DirectX texture container, not anything a URL serves, and `image` can decode it
but not encode it, so no round-trip test can cover it; see the `image` entry in the workspace `Cargo.toml`.

**ICO is deliberately absent.** It is a Windows icon container rather than an image format — the same call as DDS.
`image` also decodes one only when its embedded PNG is RGBA, so a perfectly ordinary icon can fail to decode.

**TGA is deliberately absent.** It has no leading magic bytes, so `guess_format` can never select it and the feature
stays off; see the `image` entry in the workspace `Cargo.toml`.
