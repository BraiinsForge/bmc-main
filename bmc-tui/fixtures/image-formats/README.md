# Image-format fixtures

Served to the device by the `deck image-formats` harness through the local asset server, so a run depends on nothing
outside the repository.

Licence and credit: [`ATTRIBUTION.md`](ATTRIBUTION.md).

## Files

- `blue-marble.png` — the fetched source; every other fixture is generated from it
- `blue-marble.{bmp,ff,gif,hdr,jpg,pnm,qoi,tiff,webp}` — one enabled decoder each

## The two size-limit cases are not stored here

`over-pixel-budget.png` (3000x3000) and `over-fetch-cap.bmp` (12 MB) are built per run by `flat_png` and `flat_bmp` in
`bmc_tui/procedures/image_formats.py`, and served on those paths by the asset server.

They probe a size limit rather than a decoder, so nothing about their content matters — only their length and pixel
count. Storing them cost 16 MB of LFS payload for flat colour that `zlib` and `struct` reproduce exactly.

They still sit on opposite sides of the cap deliberately. A body over the cap never reaches a decoder, so one oversized
file cannot test both limits — conflating them is why an earlier corpus never exercised the pixel budget.

## How these were made

`blue-marble.png` is the linked rendition rescaled to 640x320 with Lanczos3. Every other file is that image re-encoded
with the `image` crate, one file per enabled decoder, converted to whatever colour type each encoder accepts (farbfeld
16-bit RGBA, Radiance Rgb32F, the rest 8-bit RGB).

640x320 is chosen, not incidental: the widget viewport is 480x480, so the source still arrives larger on one axis and
exercises the downscale path, while every derived fixture costs a quarter of what a 1280px source would.

There is no generator committed: the corpus only changes when the enabled decoder set does, and regenerating it then is
a deliberate act, not routine maintenance.
