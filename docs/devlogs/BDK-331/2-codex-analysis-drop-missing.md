# Analysis: MR 242 note 288041 (`Drop` missing for GPU helpers)

**Reviewed:** 2026-04-13
**Branch:** `jku/BDK-331/regression-testing`
**HEAD:** `d614d80ef06d42ea63e0c767b25ed1d418cc2fa8`
**Note:** <https://gitlab.ii.zone/bos/bmc-main/-/merge_requests/242#note_288041>

## Reviewer note

František Boháček asked whether the GPU-side helpers should implement `Drop`
so they release unmanaged resources automatically:

- `SphereRenderer` in `bmc-wasm-runtime/src/gpu/sphere.rs`
- `BitmapRegistry` in `bmc-wasm-runtime/src/gpu/bitmap.rs`

The concrete concern was leaked GL/FemtoVG resources:

- `SphereRenderer`: GL program, VBO, FBO, texture, FemtoVG image handle
- `BitmapRegistry`: FemtoVG `ImageId` handles

## Current codebase status

The underlying leak concern is addressed in the current branch, but not by
adding `Drop` directly to `SphereRenderer` or `BitmapRegistry`.

### 1. Cleanup now happens in the owner: `FemtoVgRenderer`

`FemtoVgRenderer` is the only owner of both helper objects:

- `bitmap_registry: BitmapRegistry`
- `sphere: Option<SphereRenderer>`

Current teardown lives in
`bmc-wasm-runtime/src/gpu/renderer.rs:131-137`:

```rust
impl Drop for FemtoVgRenderer {
    fn drop(&mut self) {
        if let Some(sphere) = self.sphere.take() {
            sphere.destroy(&self.gl, &mut self.canvas);
        }
        self.bitmap_registry.clear(&mut self.canvas);
    }
}
```

This means the renderer now performs deterministic cleanup while both the GL
context and FemtoVG canvas are still alive.

### 2. `SphereRenderer` cleanup exists and is complete for owned resources

`bmc-wasm-runtime/src/gpu/sphere.rs:346-359` now contains an explicit teardown
method:

- `canvas.delete_image(self.image_id)`
- `gl.delete_vertex_array(...)`
- `gl.delete_buffer(self.vbo)`
- `gl.delete_framebuffer(self.fbo)`
- `gl.delete_texture(self.fbo_texture)`
- `gl.delete_program(self.program)`

Important ownership detail:

- `SphereRenderer::texture` is **not owned** by `SphereRenderer`
- `bmc-wasm-runtime/src/gpu/sphere.rs:235-239` documents it as
  "borrowed from femtovg, not owned"
- that texture comes from
  `bmc-wasm-runtime/src/gpu/renderer.rs:498-503` via
  `self.canvas.get_native_texture(image_id)`

So the original note slightly overstated current ownership. The sphere renderer
owns the offscreen FBO texture (`fbo_texture`) and its FemtoVG image wrapper
(`image_id`), but not the source texture borrowed from the bitmap image.

### 3. `BitmapRegistry` cleanup exists for all registered images

`bmc-wasm-runtime/src/gpu/bitmap.rs:99-104` now drains the registry and deletes
every registered FemtoVG image:

```rust
pub fn clear(&mut self, canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>) {
    for bitmap in self.bitmaps.drain().map(|(_, bitmap)| bitmap) {
        canvas.delete_image(bitmap.image_id);
    }
}
```

Because `FemtoVgRenderer::drop()` calls `bitmap_registry.clear(...)`, these
image handles are no longer leaked on renderer teardown.

## Was the reviewer comment correct?

Yes, for the original review point the comment was materially correct:

- the code previously lacked teardown for the sphere renderer's unmanaged GL
  resources
- the bitmap registry previously retained FemtoVG image handles for the runtime
  lifetime with no cleanup on renderer destruction

That gap was fixed later by commit `cdf1e2c2`
(`wasm: Harden runtime boundaries and cleanup paths #BDK-331`), which added:

- `FemtoVgRenderer::drop()`
- `SphereRenderer::destroy(...)`
- `BitmapRegistry::clear(...)`

## Should `SphereRenderer` and `BitmapRegistry` themselves implement `Drop`?

Not necessarily, and in the current design it would be awkward.

The reason is that cleanup requires external state that those helper structs do
not own:

- `SphereRenderer` needs both `&glow::Context` and `&mut Canvas<OpenGl>`
- `BitmapRegistry` needs `&mut Canvas<OpenGl>`

Direct `Drop` on those types would require one of these less attractive designs:

- storing raw GL/canvas pointers inside the helpers
- duplicating ownership of rendering context state
- introducing more complex lifetime coupling between helper objects and the
  renderer

Given the current ownership model, owner-driven teardown in
`FemtoVgRenderer::drop()` is the simpler and safer solution.

## Conclusion

The GitLab note is **stale relative to the current branch**:

- the leak it reports has been fixed
- the fix is implemented at the owning renderer level rather than as direct
  `Drop` impls on the helper types

Recommended MR response:

- explain that the original leak concern was valid
- point to `FemtoVgRenderer::drop()`, `SphereRenderer::destroy(...)`, and
  `BitmapRegistry::clear(...)`
- clarify that direct `Drop` on the helper structs is not required under the
  current ownership model
- resolve the note unless the reviewer explicitly prefers an architectural
  refactor toward self-contained RAII wrappers

## Follow-up doc note

`docs/devlogs/BDK-331/1-claude-review.md` still describes `C7` as remaining
work. That is now outdated on `HEAD` and should be updated to mark the GPU
cleanup issue resolved by `cdf1e2c2`.
