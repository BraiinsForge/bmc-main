# BDK-252 Stage 3: Self-contained Mesa — Implementation Plan

**Goal:** Replace the `wrapNixGL` wrapper scripts with a self-contained
Mesa build. Mesa provides `libEGL.so`, `libGLESv2.so`, and `libgbm.so`
directly (no libglvnd dispatch). GBM finds its backend at
`$out/lib/gbm/` via compiled-in path. No environment variables needed.

**Prerequisites:** Stage 1 and Stage 2 complete — `wrapNixGL = true`
on ARM glibc profiles, widgets build and run (with wrapper scripts),
init tarball works.

**Design decisions:**
- Per-app Mesa (each widget/compositor carries its own Mesa in its
  closure) — rationale in `mesa-libglvnd.md`
- **Disable libglvnd** — Mesa provides `libEGL.so` directly, no vendor
  JSON dispatch. Rationale in `mesa-libglvnd.md` § "Why no libglvnd"
- **Build libgbm inside Mesa** (`libgbm-external = false`) — eliminates
  the `/run/opengl-driver/lib/gbm` compiled-in path
- Modify the existing custom Mesa package at
  `nix/pkgs/mesa/package.nix`, not upstream nixpkgs Mesa
- The overlay chain in `flake.nix` already has a Mesa overlay slot at
  lines 37-39; no new overlay needed

---

## Research summary (completed)

Full findings in `docs/plans/stage-3-research.md`. Key results:

**Only 2 things inject `/run/opengl-driver` that affect our stack:**

1. **libgbm** (standalone, `gbm.nix`) — compiles
   `/run/opengl-driver/lib/gbm` as `gbm-backends-path`.
   Fix: `libgbm-external = false` → builds libgbm inside Mesa with
   default path `$out/lib/gbm`.

2. **libglvnd** — compiles `/run/opengl-driver/share/glvnd/egl_vendor.d/`
   as `DEFAULT_EGL_VENDOR_CONFIG_DIRS`.
   Fix: **disable glvnd entirely** → Mesa produces `libEGL.so` directly,
   no vendor JSON needed.

**Things that are NOT problems:**
- DRI `dri/*.so` search path — irrelevant for Wayland/EGL/GBM (driver
  is statically linked into `dri_gbm.so`)
- `libGLX.so` RUNPATH — irrelevant (Wayland-only, no GLX)
- Mesa itself — already uses `$out/lib/` paths in its own outputs

---

## Task 1: Modify `package.nix` — disable glvnd, internalize libgbm

**Goal:** Make Mesa self-contained: it provides `libEGL.so`,
`libGLESv2.so`, `libgbm.so`, and `dri_gbm.so` all from `$out/`.

**Changes to `nix/pkgs/mesa/package.nix`:**

### 1a. Disable glvnd

```nix
# was: (lib.mesonEnable "glvnd" true)
(lib.mesonEnable "glvnd" false)
```

Effects:
- Mesa builds `libEGL.so` (soname `libEGL.so.1`) instead of
  `libEGL_mesa.so.0`
- No `50_mesa.json` vendor JSON is produced
- No dependency on libglvnd

### 1b. Build libgbm inside Mesa

```nix
# was: (lib.mesonBool "libgbm-external" true)
(lib.mesonBool "libgbm-external" false)
```

Effects:
- Mesa builds `libgbm.so` itself
- `gbm-backends-path` defaults to `$out/lib/gbm` (from
  `meson.build:118-121`)
- `dri_gbm.so` is installed to `$out/lib/gbm/`

### 1c. Remove libglvnd, libgbm, libva-minimal, libvdpau from buildInputs

Remove `libglvnd`, `libgbm`, `libva-minimal`, and `libvdpau` from the
function arguments and `buildInputs` list. The latter two are safe to
remove because `gallium-va` and `gallium-vdpau` are both disabled.
Removing them also eliminates the packages that read `mesa.driverLink`
from the evaluation graph, allowing `driverLink` to be removed from
passthru without breaking the build.

### 1d. Remove or simplify postFixup

The current `postFixup` rewrites `50_mesa.json` vendor JSON paths.
With glvnd disabled, no vendor JSON is produced. Remove the EGL vendor
JSON rewriting block. Keep the Vulkan layer manifest rewriting (even
though we don't use Vulkan currently — it is harmless).

### 1e. Remove `driverLink` from passthru

The current passthru has `inherit (libglvnd) driverLink;` which
evaluates to `/run/opengl-driver`. Remove it — with glvnd disabled
and libgbm internal, there is no need for a `driverLink` indirection.

**Files:** `nix/pkgs/mesa/package.nix`

---

## Task 2: Update `workspace.nix` — replace `libGL` with `mesa`

**Goal:** Applications link against `libEGL.so` from Mesa directly.

**Changes:**

In `targetDeps` (line 83-94):
- Remove `libgbm` — now built inside Mesa
- Remove `libGL` — was libglvnd, no longer needed
- Keep `mesa` — provides `libEGL.so`, `libGLESv2.so`, `libgbm.so`

In `commonDeps.guiRuntimeDeps` (line 53-56):
- Remove `mesa` if it duplicates with waylandRuntimeDeps
- Ensure `mesa` is in the list (not `libGL`)

**Files:** `workspace.nix`

---

## Task 3: Update `rustflags.nix` — replace `libGL` with `mesa`

**Goal:** ARM rpath includes Mesa's store path.

**Changes to `waylandRuntimeDeps`:**
```nix
# was: libGL (= libglvnd)
# now: mesa (provides libEGL.so, libGLESv2.so, libgbm.so)
waylandRuntimeDeps = pkgs: with pkgs; [
  wayland
  libxkbcommon
  vulkan-loader
  mesa           # was: libGL
];
```

This is temporary — Stage 4 replaces RUSTFLAGS rpath with per-binary
patchelf patching.

**Files:** `nix/rustflags.nix`

---

## Task 4: Verify ARM cross-compilation

**Goal:** Confirm the modified Mesa builds for ARM.

```bash
nix build .#legacyPackages.x86_64-linux.armv7-pkgs.mesa
```

Check outputs:
```bash
ls result/lib/libEGL*      # should be libEGL.so, libEGL.so.1, libEGL.so.1.0.0
ls result/lib/libgbm*      # should be libgbm.so, libgbm.so.1, etc.
ls result/lib/gbm/          # should contain dri_gbm.so
ls result/lib/libGLESv2*    # should exist
```

Verify no glvnd artifacts:
```bash
ls result/share/glvnd/ 2>/dev/null  # should not exist or be empty
ls result/lib/libEGL_mesa* 2>/dev/null  # should not exist
```

**Files:** none (verification only)

---

## Task 5: Verify no `/run/opengl-driver` in closure

```bash
nix build .#legacyPackages.x86_64-linux.armv7-pkgs.mesa
nix path-info -r ./result > /tmp/mesa-closure.txt
while read -r path; do
  grep -rql "opengl-driver" "$path" 2>/dev/null && echo "FOUND: $path"
done < /tmp/mesa-closure.txt
```

No output = success.

Also check Mesa's own binary:
```bash
strings result/lib/libgbm.so | grep -i "opengl-driver"
# Should return nothing — gbm-backends-path should be $out/lib/gbm
```

---

## Task 6: Manual test on device

1. Build the compositor: `nix build .#bmc-openwrt-armv7-glibc-release`
2. Deploy to device with `./scripts/nix-deploy.sh`
3. On device, unset all Mesa env vars:
   ```bash
   unset LIBGL_DRIVERS_PATH GBM_BACKENDS_PATH __EGL_VENDOR_LIBRARY_FILENAMES
   ```
4. Run via `start-compositor`:
   ```bash
   /run/current-profile/bin/start-compositor /nix/store/.../bin/bmc-openwrt
   ```
5. Verify EGL initializes (process keeps running). Failure is a
   non-zero exit and logs like:
   ```
   MESA-LOADER: failed to open dri: ...
   Failed to initialize EGL context: Failed to create GBM device
   ```

**Note:** The `wrapNixGL` wrapper is still in place (removed in
Stage 4). This test verifies the wrapper is no longer _needed_.

**Fallback:** If `start-compositor` or the profile system is not yet
deployed on the test device, copy the compositor binary directly and
run it manually: `XDG_RUNTIME_DIR=/tmp /nix/store/.../bin/bmc-openwrt`
after unsetting Mesa env vars.

---

## Files to modify

| File | Action | Purpose |
|------|--------|---------|
| `nix/pkgs/mesa/package.nix` | Modify | Disable glvnd, `libgbm-external = false`, remove libglvnd/libgbm deps |
| `workspace.nix` | Modify | Replace `libGL`/`libgbm` with `mesa` in deps |
| `nix/rustflags.nix` | Modify | Replace `libGL` with `mesa` in `waylandRuntimeDeps` |

---

## Dependency graph

```
Task 1 (package.nix)
  ├→ Task 2 (workspace.nix)
  ├→ Task 3 (rustflags.nix)
  └→ Task 4 (ARM cross build)
       └→ Task 5 (closure verification)
            └→ Task 6 (device test)
```

---

## Risks

1. **`libgbm-external = false` build failures.** Setting this may
   require additional meson flags not in our minimal config. The
   standalone `libgbm` derivation only needs `libdrm`; building it
   inside Mesa should not add much, but needs verification.

2. **Cross-compilation of modified Mesa.** Removing libglvnd and
   internalizing libgbm changes the dependency graph. Test the ARM
   cross build early.

3. **Packages linking against `libGL` (= libglvnd).** Any Nix
   expression that references `libGL` in the ARM target gets libglvnd.
   We must ensure our widget/compositor builds use `mesa` instead.
   Other nixpkgs packages that depend on `libGL` are unaffected
   (the overlay only changes `mesa`, not `libGL` or `libglvnd`).

4. **Impact on Stage 4 patchelf plans.** With libgbm internal and
   glvnd disabled, `libgbm` and `libglvnd` should NOT appear as
   separate rpath entries. The `widgetGlRpath` list in
   `nix-patchelf.md` simplifies accordingly.

---

## Success criteria

- Mesa produces `libEGL.so` directly (not `libEGL_mesa.so`).
- `libgbm.so` has `$out/lib/gbm` as compiled-in backends path.
- `dri_gbm.so` exists at `$out/lib/gbm/dri_gbm.so`.
- No `/run/opengl-driver` references in the Mesa closure.
- No Mesa env vars required (`GBM_BACKENDS_PATH`,
  `__EGL_VENDOR_LIBRARY_FILENAMES`, `LIBGL_DRIVERS_PATH`).
- The compositor runs from the Nix store with Mesa env vars unset.

---

## Key reference files

| Document | Path | Purpose |
|----------|------|---------|
| Mesa strategy | `docs/devlogs/BDK-252/mesa-libglvnd.md` | Per-app Mesa, no-glvnd decision |
| Research findings | `docs/plans/stage-3-research.md` | Detailed investigation results |
| Patchelf design | `docs/devlogs/BDK-252/nix-patchelf.md` | Stage 4 dependency |
| Implementation plan | `docs/devlogs/BDK-252/implementation-plan.md` | Stage 3 high-level |
| Custom Mesa | `nix/pkgs/mesa/package.nix` | Derivation to modify |
| Custom Mesa common | `nix/pkgs/mesa/common.nix` | Version and source |
| Flake | `flake.nix` | Overlay chain |
| Workspace | `workspace.nix` | `fixedArmv7Pkgs`, `targetDeps` |
| Profiles | `nix/profiles.nix` | `wrapNixGL = true` (not removed until Stage 4) |
