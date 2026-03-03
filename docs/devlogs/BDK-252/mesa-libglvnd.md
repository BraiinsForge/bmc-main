# Mesa and libglvnd strategy for per-app Nix closures

Related: BDK-252 Stage 1, BDK-308 (dev-shell-improvements)

## Problem

Nixpkgs Mesa is patched to search for DRI drivers at
`/run/opengl-driver/lib/dri/` instead of its own `$out/lib/dri/`. This
is the NixOS convention — the system-wide OpenGL driver is symlinked
there by the `hardware.graphics` NixOS module.

On our OpenWrt target there is no NixOS managing `/run/opengl-driver`.
Currently, `launch.sh` works around this by setting environment
variables (`LIBGL_DRIVERS_PATH`, `GBM_BACKENDS_PATH`,
`__EGL_VENDOR_LIBRARY_FILENAMES`).

## Decision: per-app Mesa

Each application (compositor, widgets, future third-party packages)
carries its own Mesa in its Nix closure. Rationale:

1. **Low ABI coupling between processes.** Wayland compositor and clients
   communicate via the Wayland socket protocol and DMA-BUF file
   descriptors. There is no shared Mesa library surface between
   processes. Different Mesa versions usually interoperate by
   negotiating common format/modifier sets via `linux-dmabuf-v1`.

2. **Public APIs are stable, but mixed runtime pieces are risky.** EGL,
   GLESv2, and Vulkan are Khronos-standardized ABIs, but real failures
   still happen when a process mixes loader/vendor/driver pieces from
   different builds (for example, libGL from one closure and DRI
   drivers from another). Each app closure must therefore keep
   libglvnd, Mesa loader libraries, DRI drivers, and their dependent
   libraries aligned.

3. **Kernel DRM UAPI is backwards-compatible, not feature-identical.**
   Open-source Mesa drivers are generally tolerant across kernel
   versions, but newer Mesa can still require newer kernel capabilities
   or trigger old-kernel bugs. Tight lockstep is most visible with
   proprietary NVIDIA; open-source stacks are usually more forgiving
   but still need testing on the oldest supported kernel.

4. **Aligns with Nix extensibility architecture.** Third-party
   packages (BDK-212 concept) bring their own closures. Forcing a
   single system-wide Mesa would require stripping Mesa from
   third-party closures and injecting our own — fragile and
   constraining.

5. **Disk space is manageable.** Mesa ARM build is ~20-40 MB. Multiple
   versions on flash storage are acceptable for our use case.

The alternative — single system-wide Mesa — was evaluated. It is
simpler but creates coupling: all apps must be compatible with the same
Mesa version, upgrades affect everything at once, and third-party
packages need special handling. The per-app approach avoids this.

References:
- https://discourse.nixos.org/t/help-understanding-the-libgl-abi-problem-and-possible-solutions/42022
- https://github.com/NixOS/nixpkgs/issues/31189 (libGL ABI problem)
- https://github.com/flatpak/flatpak/issues/3673 (Flatpak Mesa host/runtime mismatch)
- https://github.com/nix-community/nixGL (nixGL wrapper approach)

## Implementation: Mesa overlay

Create a Nix overlay that builds Mesa with self-contained driver paths
instead of the NixOS `/run/opengl-driver` indirection.

### What needs to change

Mesa's Nixpkgs derivation patches the DRI driver search path to
`/run/opengl-driver/lib/dri/`. We will build Mesa without this patch so
it uses its own `$out/lib/dri/` as the default search path. This is the
cleanest approach — Mesa already knows where its own drivers are at
build time.

### Paths to handle

| Path | Owned by | Purpose |
|------|----------|---------|
| `LIBGL_DRIVERS_PATH` | Mesa | DRI driver search (`*.so` in `lib/dri/`) |
| `GBM_BACKENDS_PATH` | Mesa (libgbm) | GBM backend (`gbm_*.so`) |
| `__EGL_VENDOR_LIBRARY_FILENAMES` | libglvnd | EGL vendor JSON (`50_mesa.json`) |

Avoid partial host/runtime mixing. A closure must not combine host
`/usr/lib*/dri` artifacts with store-provided libGL/libEGL or vice
versa.

`LIBGL_DRIVERS_PATH` and `GBM_BACKENDS_PATH` resolve
automatically (Mesa's own `$out/lib/dri/` and `$out/lib/`).

`__EGL_VENDOR_LIBRARY_FILENAMES` is a libglvnd concern — libglvnd
searches for vendor JSON files in a compiled-in path
(`/usr/share/glvnd/egl_vendor.d/` or the Nix store equivalent). If our
overlay builds libglvnd alongside Mesa, this should resolve
automatically. If not, a wrapper or overlay for libglvnd is needed.

`LIBVA_DRIVERS_PATH` is not applicable — the Vivante GC400 on the
STM32MP1 has no video decode engine, so VA-API is not used.

### Overlay sketch

```nix
final: prev: {
  mesa = prev.mesa.overrideAttrs (old: {
    # Remove the NixOS-specific /run/opengl-driver patch
    patches = builtins.filter
      (p: !(builtins.isString p && builtins.match ".*opengl-driver.*" p != null))
      (old.patches or []);
  });
}
```

The exact filter condition needs to be verified against the current
Nixpkgs Mesa derivation — the patch may be applied via meson flags
rather than a patch file.

### workspace.nix changes

BDK-308 already added rpath flags for the dev shell. For production ARM
builds, `workspace.nix` needs to:

1. Use the Mesa overlay (so the closure carries self-contained Mesa).
2. Ensure rpath entries for Mesa's `lib/` and `lib/dri/` are set on
   all GUI binaries.
3. Verify that `mkWidgetPackage.nix` also uses the overlaid Mesa.
