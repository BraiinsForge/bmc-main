# bmc-gallery

Every `*.scene.rs` in the repo, rendered through bmc-render's femtovg pipeline onto
[rs-gallery](https://github.com/kubijo/rs-gallery) stages. Excluded from the cargo workspace with its own `Cargo.lock`,
so the framework's egui version stays clear of the workspace's.

```sh
just gallery::run                                        # the window
just gallery::hot                                        # the window, reloading scenes as you edit
just gallery::check                                      # type-check scenes + launcher, no GL

just gallery::preview 'animation::Easing Curves' easing  # one scene at its declared knobs
just gallery::knobs 'overlays::Settings Tray'            # what a recipe can set
just gallery::capture-init 'overlays::Settings Tray'     # a recipe with those knobs filled in
just gallery::capture                                    # every shot in capture.toml
```

`preview` shows a scene as it declares itself; setting knobs is what `capture` is for. Both write under `.tmp/`, taking
a name rather than a path.

Scene keys are `<file-stem>::<Title Case fn>` — quote them. Add `GALLERY_SOFTWARE_GL=1` on a machine with no GPU.

A capture leaves `capture.json` beside the images; a loop should assert `complete` and `shots.length == requested`
rather than counting files, since a run that stops early still writes the report.

## Capture cannot see GL state

Captures prove what a scene draws, not that the kit left the GL context as it found it — capture rebinds its framebuffer
per shot, so a binding we fail to restore is invisible there while the window renders solid white. That has hidden two
bugs already.

**Any change to render targets, framebuffer binding, or renderer GL state needs one `just gallery::run` before it counts
as verified.**

## Adding a scene

Put a `*.scene.rs` beside the code it exercises; the glob in `gallery.toml` finds it. Import `bmc_gallery::prelude::*`
and stage through `ctx.node_stage` (an SDK tree) or `ctx.custom_stage` (straight onto the renderer) — the `_input`
variants take the pointer and the wheel, for widgets that hit-test themselves.

Register assets *inside* the stage closure: the renderer is built on the first stage draw, so `ensure_registered` above
it panics in a process that has drawn nothing yet — which is what a headless render of that scene is.

`custom_stage` returns whether it is still moving. Answer from something checkable: a renderer handed no clock cannot
animate, one handed `Instant::now()` can.
