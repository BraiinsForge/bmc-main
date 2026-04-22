# Vivante damage-tracking regression

**Ticket**: BDK-389 · Companion to [`../combined-scene-analysis.md`](../combined-scene-analysis.md).

Captures why the compositor's `frame.clear(...)` and KMS `FB_DAMAGE_CLIPS` paths added in
`4b2ad451 "bmc-openwrt: Plumb output damage to page flips"` are currently disabled for the Deck's Vivante GC400.

## Symptom

On the Deck HW (Vivante GC400, Etnaviv Mesa 25.1.1, DSI 480×1280) with `4b2ad451` deployed, the wasm `hello-widget`
showcase rendered two distinct defects together:

1. **First-use-of-glyph missing from short layout runs.** The first occurrence of a given glyph in a layout run came out
   transparent / button-bg-coloured, subsequent occurrences in the same frame rendered fine:

   | Rendered     | Expected     | Missing               |
   | ------------ | ------------ | --------------------- |
   | `Second y`   | `Secondary`  | `a`, `r`              |
   | `Te t ry 0`  | `Tertiary 0` | `r` (first), `i`, `a` |
   | `Anima ions` | `Animations` | `t`                   |
   | (blank)      | `Primary`    | whole string          |

   Long paragraphs (`Rich Text`, `Lorem ipsum …`) rendered perfectly.

2. **~300 ms pipeline stalls every ~5 s**, visible as ~11 Hz choppiness (widget commits ~91 ms apart; the stalls were on
   top of that).

Not reproducible in the `bmc-virt` QEMU VM — VM's virgl / llvmpipe path tolerates the operations that break on Etnaviv.

## Diagnosis

Bisect across `origin/master..jku/BDK-389/combined-scene` landed on `4b2ad451` as the first-bad commit. The two
behavioural changes in that commit each drive one of the symptoms independently:

- **`frame.clear(BACKGROUND_COLOR, &damage_rects)` before widget composite** (in `scene_renderer.rs::render_scene`) —
  issues a `glClear` on the output FB immediately before the compose loop that samples widget DMA-BUF textures via
  `render_texture_from_to`. On Etnaviv this disturbs sampler coherency against the widget textures for the duration of
  the frame; sparse atlas reads (short-layout glyphs) see stale/empty texels for cells that were newly uploaded by the
  widget's own GL context in the previous moment. Dense sampling (paragraphs, icons, shape fills) refreshes the sampler
  by volume and works. Cross-process sync via an EGL fence would be the proper producer-side solution but
  `wp_linux_drm_syncobj_v1` isn't wired through this widget or compositor yet.

- **`PlaneDamageClips::from_damage(...)` on every atomic commit** (in `render/drm_output.rs::page_flip`) — attaches
  per-rect damage blobs to the KMS atomic commit. Etnaviv's scanout path periodically stalls 300+ ms consuming these
  hints. Setting `damage_clips: None` returns to master's behaviour and removes the stalls.

Confirmed independently: with both the clear and the damage-clips disabled, both symptoms vanish and the output
framebuffer exactly matches master.

### Evidence that narrowed it

- `glReadPixels` dump of the output FB on the broken build shows the missing-glyph pattern already baked in *before* KMS
  scanout — corruption enters at or before composition, not in the display path.
- Forcing full-output damage on `FB_DAMAGE_CLIPS` doesn't fix text — rules out "KMS drops pixels it wasn't told about".
- Animation regions render fresh every frame on the broken build — rules out "output FB carrying stale first-frame
  pixels".
- The widget's own DMA-BUF is correct: the identical widget binary renders cleanly on master. Problem is not on the
  producer side, it's Etnaviv's response to the specific operation sequence 4b2ad451 introduced in the compositor.

## Current state

The compositor is patched to keep the `4b2ad451` damage-tracking *infrastructure* (`OutputDamage` enum, tracker fields
in `CompositorState`, `output_damage` argument on `render_scene`, per-widget damage accumulation in `damage_rects`) but
disables the two paths that misfire on Etnaviv:

- `scene_renderer.rs` — the `frame.clear` call is removed; the rest of `render_scene` unchanged.
- `render/drm_output.rs` — `damage_clips: None` unconditionally; `damage` argument retained on `page_flip` so callers
  don't have to change.

Comments in both files point back here.

## Re-enabling the feature properly

Both paths can be re-enabled in principle — Vivante is a tile-based renderer and would benefit from proper partial
damage — but each needs a different fix:

1. **Clear-before-composite sampler disturbance**: either (a) wire EGL fence sync on the widget producer side so the
   compositor samples against a fenced DMA-BUF guaranteed-complete in its own GL context, or (b) replace the
   pre-composite `frame.clear` with a post-composite scissor-clear that only touches pixels not covered by any widget
   (safe on Etnaviv because it's after all sampling).
2. **KMS `FB_DAMAGE_CLIPS` stalls**: characterise which Etnaviv / kernel versions handle the hint well, gate at a GPU
   probe, or report and track upstream fix.

Until then, running at master-equivalent "no clear / no damage clips" costs nothing on the Deck's current scene (widgets
cover the full output, so skipping the clear is a no-op) and avoids both defects.

## Reproduction recipe

1. Cross-compile + deploy compositor:
   `nix develop .#armv7-glibc-release --command ./scripts/nix-cargo-deploy.sh compositor <device-ip>`.
2. Ensure `/mnt/data/bmc-widgets/wasm/bin/bmc-widget-wasm` is a fresh armv7 build
   (`cargo build --release --target armv7-unknown-linux-gnueabihf -p bmc-widget-wasm` in the cross shell + `scp`).
3. Push `hello_widget.wasm` to `/mnt/data/wasm/hello_widget.wasm` on-device, and a fullscreen scene config pointing the
   wasm widget at that path into `/etc/bmc_config.json`.
4. Launch `bmc-openwrt --widgets-path /mnt/data/bmc-widgets` with a fresh `XDG_RUNTIME_DIR`.

With `4b2ad451` as-is: button labels on the showcase scene drop first-use glyphs and the compositor exhibits ~300 ms
periodic stalls. With the current patch: text renders cleanly and framerate matches master.
