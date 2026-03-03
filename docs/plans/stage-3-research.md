# BDK-252 Stage 3: Research Findings

nixpkgs pin: `ece6e266caf1effab32eceef0403b797b4330373` (nixpkgs-unstable)
Mesa version (our custom): 25.1.1
Mesa version (upstream nixpkgs): 25.3.1 (but irrelevant — we use our own)
libgbm version (upstream nixpkgs): 25.1.0 (separate derivation)
libglvnd version: 1.7.0

Source repos for this research:
- `~/src/mesa` — Mesa 25.1.1 (tag `mesa-25.1.1`)
- `~/src/nixpkgs` — nixpkgs at `ece6e266ca`

---

## Task 1.1: addDriverRunpath

**Source:** `~/src/nixpkgs/pkgs/by-name/ad/addDriverRunpath/`

`addDriverRunpath` defines:
```nix
driverLink = "/run/opengl-driver" + lib.optionalString stdenv.hostPlatform.isi686 "-32";
```

The setup hook (`setup-hook.sh`) provides a shell function that
**prepends** `/run/opengl-driver/lib` to any ELF file's RUNPATH:
```bash
patchelf --set-rpath "/run/opengl-driver/lib:$origRpath" "$file"
```

**Who uses it:** Only `libglvnd` uses `addDriverRunpath` directly:
- As a `nativeBuildInput` (makes the `addDriverRunpath` shell function
  available)
- In `postFixup`: `addDriverRunpath $out/lib/libGLX.so` (only GLX, not
  EGL)
- Via string interpolation for `DEFAULT_EGL_VENDOR_CONFIG_DIRS`

**Mesa itself does NOT use `addDriverRunpath`.** The upstream
`default.nix` has no reference to it. The `/run/opengl-driver` path
enters the Mesa ecosystem only through libglvnd and libgbm.

---

## Task 1.2: libglvnd EGL vendor config path

**Source:** `~/src/nixpkgs/pkgs/by-name/li/libglvnd/package.nix`

libglvnd compiles the EGL vendor JSON search path into the binary via
C compiler flags:
```nix
env.NIX_CFLAGS_COMPILE = toString [
  "-UDEFAULT_EGL_VENDOR_CONFIG_DIRS"
  "-DDEFAULT_EGL_VENDOR_CONFIG_DIRS=\"${addDriverRunpath.driverLink}/share/glvnd/egl_vendor.d:/etc/glvnd/egl_vendor.d:/usr/share/glvnd/egl_vendor.d\""
];
```

This resolves to:
```
/run/opengl-driver/share/glvnd/egl_vendor.d:/etc/glvnd/egl_vendor.d:/usr/share/glvnd/egl_vendor.d
```

At runtime, libglvnd's `libEGL.so` searches these directories for
`*.json` vendor files (like `50_mesa.json`). Mesa's `postFixup` rewrites
`50_mesa.json` to use an absolute Nix store path:
```json
{"file_format_version": "1.0.0", "ICD": {"library_path": "/nix/store/...-mesa-.../lib/libEGL_mesa.so.0"}}
```

**The `__EGL_VENDOR_LIBRARY_FILENAMES` env var** overrides this
entirely — when set, libglvnd loads ONLY the specified JSON file(s)
and ignores `DEFAULT_EGL_VENDOR_CONFIG_DIRS`. This is what `wrapNixGL`
currently uses.

### Key finding: libEGL.so does NOT have /run/opengl-driver in RUNPATH

The upstream `postFixup` comment says explicitly:
> "Note that libEGL does not need it because it uses driver config
> files which should contain absolute paths to libraries."

Only `libGLX.so` gets the RUNPATH treatment. Since we are Wayland-only
(no X11/GLX), the libGLX RUNPATH is irrelevant.

### Impact on Stage 3

To make EGL self-contained, we need libglvnd to find `50_mesa.json`
without `/run/opengl-driver`. Options:

**Option A: Rebuild libglvnd** with a custom
`DEFAULT_EGL_VENDOR_CONFIG_DIRS` that includes `$mesa/share/glvnd/egl_vendor.d/`.
Problem: circular dependency — libglvnd would need Mesa's store path
at build time, but Mesa depends on libglvnd.

**Option B: Use `__EGL_VENDOR_LIBRARY_FILENAMES` env var.**
Not allowed per Stage 3 design (no env vars).

**Option C: Symlink/copy Mesa's `50_mesa.json` into a libglvnd-known
path.** Same circular dependency.

**Option D: Rebuild libglvnd with `$out/share/glvnd/egl_vendor.d/` in
the search path, then install Mesa's `50_mesa.json` there (or symlink
it).** This works if we create a combined package or use a postFixup
step. Since the JSON contains an absolute path to `libEGL_mesa.so`,
the JSON can live anywhere as long as libglvnd finds it.

**Option E: Build libglvnd as part of Mesa** (`glvnd` meson option).
Mesa already builds `libEGL_mesa.so` — if we also built the dispatch
layer... No, this is not how it works. Mesa builds the vendor
implementation, not the dispatch layer.

**Recommended approach:** Override libglvnd to include
`$out/share/glvnd/egl_vendor.d/` in `DEFAULT_EGL_VENDOR_CONFIG_DIRS`,
then in Mesa's postFixup, **also** install `50_mesa.json` into the
libglvnd output. Wait — that doesn't work either (can't write to
another package's output).

**Actually the simplest approach:** Override libglvnd to add
Mesa's `$out/share/glvnd/egl_vendor.d/` to the search path. But
libglvnd doesn't know Mesa's store path... unless we pass it as an
argument. This creates a **dependency loop**: Mesa depends on libglvnd
(build input), libglvnd depends on Mesa's path (compile-time flag).

**Breaking the loop:** Use Nix's `overrideAttrs` or a fixed-output
path. OR: make libglvnd search its OWN
`$out/share/glvnd/egl_vendor.d/` and then create a wrapper package
that symlinks Mesa's JSON there.

**Simplest correct approach:**
1. Override libglvnd to search `$out/share/glvnd/egl_vendor.d/` (its
   own output) as the FIRST entry in `DEFAULT_EGL_VENDOR_CONFIG_DIRS`.
2. In a post-step, symlink or copy `50_mesa.json` from Mesa into the
   libglvnd output.

But this still has the circular dep. The path forward is:
1. Rebuild libglvnd with a generic self-referencing search path.
2. Create a combined "mesa-gl" package that takes the libglvnd output,
   copies it, and adds Mesa's `50_mesa.json` into the copy's
   `share/glvnd/egl_vendor.d/`.

This is essentially **Option D from the stage-3 plan (Task 2.2,
Option C)** — a `mesa-complete` or `libglvnd-with-mesa` package.

---

## Task 1.3: Mesa DRI driver search path

**Source:** `~/src/mesa/meson.build:113-116`,
`~/src/mesa/src/gbm/backends/dri/meson.build`

### Critical finding: DRI driver search is NOT relevant for Wayland/GBM

In Mesa 25.x, the runtime driver loading for Wayland/EGL/GBM works as:

```
App → libEGL.so (libglvnd dispatch) → libEGL_mesa.so (Mesa EGL)
    → GBM device creation → libgbm.so
    → searches gbm-backends-path for dri_gbm.so
    → dri_gbm.so links statically with libgallium_dri (contains the driver)
```

The `dri/*.so` files (e.g., `etnaviv_dri.so`, `stm_dri.so`) in
`dri_drivers_path` are for **GLX/X11 use only**. They are loaded by
the X server's DRI loader or by Mesa's GLX frontend. For EGL+GBM
(Wayland), the driver code is statically linked into `dri_gbm.so`.

**Evidence from `~/src/mesa/src/gbm/backends/dri/meson.build:16`:**
```meson
link_with : [libloader, libgallium_dri],
```
`dri_gbm.so` directly links `libgallium_dri` — no runtime dlopen of
`*_dri.so` files.

### Mesa's `dri-drivers-path` meson option

```meson
# ~/src/mesa/meson.build:113-116
dri_drivers_path = get_option('dri-drivers-path')
if dri_drivers_path == ''
  dri_drivers_path = join_paths(get_option('prefix'), get_option('libdir'), 'dri')
endif
```

Default: `$prefix/$libdir/dri` → in Nix, `$out/lib/dri`.

This is the **installation** path, not a runtime search path. Our
custom Mesa at `nix/pkgs/mesa/package.nix` does not set
`dri-drivers-path`, so it defaults to `$out/lib/dri`. The `dri/*.so`
files are installed there.

### Is `LIBGL_DRIVERS_PATH` used?

**No.** `LIBGL_DRIVERS_PATH` does not appear in Mesa's C source code.
It only appears in CI YAML scripts. In modern Mesa (25.x), the DRI
loader code (`src/loader/loader.c`) provides `loader_open_driver_lib()`
which takes explicit search path variables and defaults — the only
caller for GBM is `backend.c` which uses `GBM_BACKENDS_PATH` and
`DEFAULT_BACKENDS_PATH`.

### Conclusion for Stage 3

**No DRI path changes needed.** The `dri/*.so` files are irrelevant
for our Wayland/EGL/GBM use case. The driver code is linked into
`dri_gbm.so` at build time.

---

## Task 1.4: libgbm `gbm-backends-path`

**Source:** `~/src/nixpkgs/pkgs/development/libraries/mesa/gbm.nix`

The standalone `libgbm` (in nixpkgs) sets:
```nix
(lib.mesonOption "gbm-backends-path" "${libglvnd.driverLink}/lib/gbm")
```

Which resolves to `/run/opengl-driver/lib/gbm`. This is compiled into
`libgbm.so` as `DEFAULT_BACKENDS_PATH`.

At runtime, `libgbm.so` searches:
1. `GBM_BACKENDS_PATH` env var (if set, for non-setuid processes)
2. `DEFAULT_BACKENDS_PATH` compiled-in default

**Source:** `~/src/mesa/src/gbm/main/backend.c:52-55`:
```c
static const char *backend_search_path_vars[] = {
   "GBM_BACKENDS_PATH",
   NULL
};
```

And `~/src/mesa/src/gbm/main/backend.c:111-114`:
```c
void *lib = loader_open_driver_lib(name, BACKEND_LIB_SUFFIX,
                                    backend_search_path_vars,
                                    DEFAULT_BACKENDS_PATH,
                                    warn_on_fail);
```

`DEFAULT_BACKENDS_PATH` is defined from the meson build:
`~/src/mesa/src/gbm/meson.build:14`:
```meson
args_gbm = [
  '-DDEFAULT_BACKENDS_PATH="@0@"'.format(gbm_backends_path),
]
```

### Our custom Mesa uses `libgbm-external = true`

`nix/pkgs/mesa/package.nix:134`:
```nix
(lib.mesonBool "libgbm-external" true)
```

This means our Mesa links against the **standalone `libgbm`** from
nixpkgs, which has `/run/opengl-driver/lib/gbm` compiled in.

### What `dri_gbm.so` is and where it lives

`dri_gbm.so` is the GBM backend (built as part of Mesa, not libgbm).
It is installed to `$mesa/lib/gbm/dri_gbm.so`:
```meson
# ~/src/mesa/src/gbm/backends/dri/meson.build:19-20
install : true,
install_dir: join_paths(get_option('libdir'), 'gbm'),
```

So our Mesa puts `dri_gbm.so` at `$mesa/lib/gbm/dri_gbm.so`. But the
standalone `libgbm.so` looks for it at `/run/opengl-driver/lib/gbm/`.

### Fix: set `libgbm-external = false`

If we build libgbm inside Mesa (`libgbm-external = false`), then:
- The `gbm-backends-path` defaults to `$out/lib/gbm` (from
  `meson.build:118-121`)
- `libgbm.so` and `dri_gbm.so` both end up in `$mesa/lib/` and
  `$mesa/lib/gbm/` respectively
- `libgbm.so` will search `$mesa/lib/gbm/` by default → finds
  `dri_gbm.so` → self-contained

**Alternative:** Override the standalone libgbm with a custom
`gbm-backends-path` pointing to `$mesa/lib/gbm`. But this creates a
dependency from libgbm → mesa, which doesn't exist normally.

**Recommended: `libgbm-external = false`.** This is the cleanest
approach — it eliminates the `/run/opengl-driver` dependency for GBM
entirely.

---

## Task 1.5: libglvnd RUNPATH on libEGL

**Source:** `~/src/nixpkgs/pkgs/by-name/li/libglvnd/package.nix:84-89`

```nix
# Set RUNPATH so that libGLX can find driver libraries in /run/opengl-driver(-32)/lib.
# Note that libEGL does not need it because it uses driver config files which should
# contain absolute paths to libraries.
postFixup = ''
  addDriverRunpath $out/lib/libGLX.so
'';
```

**Only `libGLX.so`** gets `/run/opengl-driver/lib` in its RUNPATH.
`libEGL.so` does NOT.

Since we are Wayland-only (no GLX), the `addDriverRunpath` on
`libGLX.so` is irrelevant. `libEGL.so` uses vendor JSON config files
(with absolute paths) instead of RUNPATH-based discovery.

**Conclusion:** No RUNPATH issue on `libEGL.so`. The only
`/run/opengl-driver` references that matter for us are:
1. `DEFAULT_EGL_VENDOR_CONFIG_DIRS` compiled into libglvnd (Task 1.2)
2. `gbm-backends-path` compiled into libgbm (Task 1.4)

---

## Summary: Where `/run/opengl-driver` enters the build

| Component | How injected | Affects us? | Fix |
|-----------|-------------|-------------|-----|
| `addDriverRunpath` setup hook | Defines the string `/run/opengl-driver` | No (not used by our Mesa) | N/A |
| `libglvnd` `DEFAULT_EGL_VENDOR_CONFIG_DIRS` | `-D` compiler flag | **Yes** — libglvnd can't find `50_mesa.json` | Rebuild libglvnd or create combined package |
| `libglvnd` `libGLX.so` RUNPATH | `patchelf` in postFixup | No (Wayland-only, no GLX) | N/A |
| `libgbm` `gbm-backends-path` | Meson flag | **Yes** — libgbm can't find `dri_gbm.so` | Set `libgbm-external = false` in Mesa |
| Mesa `dri/*.so` search path | Not compiled in; install path only | No (DRI drivers not used in EGL/GBM path) | N/A |
| Mesa `50_mesa.json` | Already uses absolute `$out/lib/` path | Correct already | N/A |
| Mesa `passthru.driverLink` | `inherit (libglvnd) driverLink` | Propagates stale string | Fix passthru |

## Recommended implementation approach

### 1. libgbm: Set `libgbm-external = false` in Mesa

In `nix/pkgs/mesa/package.nix`:
- Change `(lib.mesonBool "libgbm-external" true)` to `false`
- Remove `libgbm` from `buildInputs`
- The `gbm-backends-path` will default to `$out/lib/gbm` (self-contained)

### 2. libglvnd: Create a custom libglvnd overlay

Override libglvnd to:
- Replace `DEFAULT_EGL_VENDOR_CONFIG_DIRS` first entry with
  `$out/share/glvnd/egl_vendor.d/` (self-referencing)
- In a post-build step, copy/symlink Mesa's `50_mesa.json` into the
  libglvnd output's `share/glvnd/egl_vendor.d/`

**Problem:** libglvnd is built before Mesa, and we can't write to
libglvnd's output from Mesa's build.

**Solution:** Create a `libglvnd-mesa` wrapper derivation:
```nix
libglvnd-mesa = pkgs.runCommand "libglvnd-mesa" { } ''
  cp -r --no-preserve=mode ${customLibglvnd} $out
  mkdir -p $out/share/glvnd/egl_vendor.d
  cp ${mesa}/share/glvnd/egl_vendor.d/50_mesa.json $out/share/glvnd/egl_vendor.d/
'';
```

Where `customLibglvnd` is libglvnd rebuilt with
`DEFAULT_EGL_VENDOR_CONFIG_DIRS` = `$out/share/glvnd/egl_vendor.d/`
(plus FHS fallbacks if desired).

Applications link against `libglvnd-mesa` instead of plain `libglvnd`.

### 3. Mesa passthru: Fix `driverLink`

Replace `inherit (libglvnd) driverLink;` with something that points to
Mesa's own output (e.g., `driverLink = mesa;` or just remove it).

### 4. rustflags.nix: Add Mesa to ARM rpath (temporary)

Add `mesa` to `waylandRuntimeDeps` so the compositor binary has Mesa's
store path in its rpath. This is a temporary measure replaced by
per-binary patchelf in Stage 4.

---

## Open questions for implementation

1. **Does `libgbm-external = false` require additional meson flags?**
   Need to test. The `gbm` option is already enabled. May need to
   remove conflicting flags or adjust build inputs.

2. **libglvnd-mesa wrapper: does patchelf need updating?** If
   `libglvnd-mesa` is a copy of libglvnd with Mesa's JSON added, the
   binaries inside still have the original libglvnd's RPATH. Since
   `libEGL.so` has no `/run/opengl-driver` in its RPATH (only
   `libGLX.so` does), this should be fine for our Wayland-only case.

3. **Cross-compilation of modified Mesa with `libgbm-external = false`:**
   Need to verify that building libgbm inside Mesa works for ARM
   cross-compilation. The standalone `libgbm` derivation has minimal
   deps (just `libdrm`), so merging it into Mesa should not add much.

4. **libglvnd rebuild vs. wrapper package:** Rebuilding libglvnd with
   a custom `DEFAULT_EGL_VENDOR_CONFIG_DIRS` is cleaner but touches
   more of the dependency graph. A wrapper package is more isolated
   but adds a derivation. Given that we already overlay Mesa, adding
   a libglvnd overlay seems natural.
