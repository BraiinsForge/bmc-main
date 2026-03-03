# Mesa strategy for per-app Nix closures

Related: BDK-252 Stage 1, BDK-308 (dev-shell-improvements)

## Problem

Nixpkgs Mesa relies on two external packages that compile
`/run/opengl-driver` paths into their binaries:

1. **libgbm** (standalone) — sets `gbm-backends-path` to
   `/run/opengl-driver/lib/gbm`, so `libgbm.so` cannot find
   `dri_gbm.so` at runtime.
2. **libglvnd** — compiles `DEFAULT_EGL_VENDOR_CONFIG_DIRS` to search
   `/run/opengl-driver/share/glvnd/egl_vendor.d/` for EGL vendor JSON
   files (`50_mesa.json`).

On our OpenWrt target there is no NixOS managing `/run/opengl-driver`.
Currently, `wrapNixGL` wrapper scripts work around this by setting
environment variables (`GBM_BACKENDS_PATH`,
`__EGL_VENDOR_LIBRARY_FILENAMES`).

## Decision: per-app Mesa, no libglvnd

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
   drivers from another). Each app closure must therefore keep Mesa
   loader libraries, DRI drivers, and their dependent libraries aligned.

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

### Why no libglvnd

libglvnd is a vendor-neutral dispatch layer that allows multiple GL
implementations (Mesa, NVIDIA, etc.) to coexist. It provides
`libEGL.so` as a thin dispatcher that loads vendor-specific
`libEGL_mesa.so` via JSON config files.

We disable libglvnd because:

1. **Single vendor.** The target device has exactly one GPU (Vivante
   GC400). There will never be multiple GL vendors to dispatch between.

2. **Eliminates the EGL vendor JSON problem entirely.** With glvnd
   enabled, libglvnd needs to find `50_mesa.json` to discover
   `libEGL_mesa.so`. The compiled-in search path points to
   `/run/opengl-driver/share/glvnd/egl_vendor.d/` — a NixOS-managed
   path that does not exist on OpenWrt. Fixing this requires either
   rebuilding libglvnd with a custom search path (creating circular
   dependency issues) or maintaining a wrapper package. Disabling glvnd
   eliminates this entire problem.

3. **Mesa supports it natively.** When built with `-Dglvnd=disabled`,
   Mesa produces `libEGL.so` directly (not `libEGL_mesa.so`).
   Applications link against `libEGL.so` from Mesa — no dispatch layer,
   no vendor JSON, no environment variables.

4. **Simpler closure.** Each app's Nix closure contains Mesa (with
   `libEGL.so`, `libGLESv2.so`, `libgbm.so`, `dri_gbm.so`) and nothing
   else. No libglvnd in the dependency tree.

5. **Third-party packages can still use libglvnd.** Per-app Mesa means
   each package controls its own GL stack. If a third-party package
   needs libglvnd (e.g., for multi-vendor dispatch), it brings its own
   in its closure. Our decision does not constrain them.

References:
- https://discourse.nixos.org/t/help-understanding-the-libgl-abi-problem-and-possible-solutions/42022
- https://github.com/NixOS/nixpkgs/issues/31189 (libGL ABI problem)
- https://github.com/flatpak/flatpak/issues/3673 (Flatpak Mesa host/runtime mismatch)
- https://github.com/nix-community/nixGL (nixGL wrapper approach)

## Research findings

See `docs/plans/stage-3-research.md` for the full investigation. Key
results:

### Where `/run/opengl-driver` enters the build

| Component | How injected | Affects us? | Fix |
|-----------|-------------|-------------|-----|
| libglvnd `DEFAULT_EGL_VENDOR_CONFIG_DIRS` | `-D` compiler flag | **Eliminated** — glvnd disabled | N/A |
| libglvnd `libGLX.so` RUNPATH | `patchelf` in postFixup | No (Wayland-only) | N/A |
| libgbm `gbm-backends-path` | Meson flag | **Yes** | `libgbm-external = false` |
| Mesa `dri/*.so` search path | Install path only, not runtime | No | N/A |
| Mesa `50_mesa.json` | Absolute `$out/lib/` path | **Eliminated** — glvnd disabled | N/A |
| Mesa `passthru.driverLink` | `inherit (libglvnd) driverLink` | Stale string | Remove |

### DRI drivers are irrelevant for Wayland/GBM

The `dri/*.so` files (e.g., `etnaviv_dri.so`) are for GLX/X11 only.
In the EGL/GBM path, the gallium driver code is statically linked into
`dri_gbm.so` (the GBM backend). There is no runtime dlopen of
`*_dri.so` files. Therefore `LIBGL_DRIVERS_PATH` is not needed and the
`dri-drivers-path` meson option is irrelevant for our use case.

The EGL/GBM runtime chain:
```
App → libEGL.so (Mesa, direct) → GBM → libgbm.so → dri_gbm.so
                                                     (links libgallium_dri)
```

## Implementation: custom Mesa package

Modify the existing custom Mesa package at
`nix/pkgs/mesa/package.nix`. Two changes:

### 1. Disable glvnd

```nix
(lib.mesonEnable "glvnd" false)    # was: true
```

Mesa produces `libEGL.so` and `libGLESv2.so` directly. No vendor JSON,
no dispatch layer.

Remove:
- `libglvnd` from `buildInputs`
- `postFixup` block that rewrites `50_mesa.json` (no JSON produced)
- `inherit (libglvnd) driverLink` from `passthru`

### 2. Build libgbm inside Mesa

```nix
(lib.mesonBool "libgbm-external" false)    # was: true
```

Mesa builds `libgbm.so` itself. The `gbm-backends-path` defaults to
`$out/lib/gbm` (from `meson.build:118-121`), which is where Mesa
installs `dri_gbm.so`. Self-contained.

Remove:
- `libgbm` from `buildInputs`

### workspace.nix changes

Replace `libGL` (which is libglvnd) with `mesa` in runtime deps.
Applications link against `libEGL.so` from Mesa directly.

```nix
# In targetDeps and waylandRuntimeDeps:
# Remove: libGL (= libglvnd)
# Keep: mesa (provides libEGL.so, libGLESv2.so, libgbm.so)
```

### Impact on Stage 4 (patchelf)

With glvnd disabled and libgbm internal, the patchelf plan simplifies:
- No `libglvnd` in rpath entries
- No separate `libgbm` in rpath entries
- Mesa's `$out/lib` provides everything: `libEGL.so`, `libGLESv2.so`,
  `libgbm.so`
- `dri_gbm.so` at `$out/lib/gbm/` is found by libgbm automatically

The `widgetGlRpath` from `nix-patchelf.md` simplifies to:
```nix
widgetGlRpath = with fixedArmv7Pkgs; [
  mesa          # libEGL.so, libGLESv2.so, libgbm.so
  wayland
  libxkbcommon
  fontconfig
  freetype
];
```
