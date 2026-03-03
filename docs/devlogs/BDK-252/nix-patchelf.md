# Patchelf-based rpath patching for widget binaries

**Status: Superseded by autoPatchelfHook (BDK-353)**

This document describes the original custom patchelf approach (nonguix-
inspired patchelf-plan). After investigating three approaches in
BDK-353, `autoPatchelfHook` + `runtimeDependencies` was chosen instead:
- Zero custom code (27 lines vs 99 lines)
- Automatic DT_NEEDED resolution (bonus: auto-resolves libgcc_s.so.1)
- `runtimeDependencies` handles dlopen'd libraries via rpath
- Cross-compilation works with `armv7Pkgs.autoPatchelfHook`

Implementation: `nix/autopatchelf-binaries.nix`
Deleted: `nix/patchelf-binaries.nix`, `docs/plans/stage-4-patchelf.md`

The problem analysis and goal sections below remain accurate — only the
solution changed.

---

Related: BDK-252 Stage 4, `mesa-libglvnd.md`, `implementation-plan.md`

## Problem

Widget binaries are built with Nix for ARMv7 (glibc, dynamically
linked). They need to find their shared library dependencies at
runtime -- Mesa, libglvnd, wayland, libxkbcommon, etc. Currently two
mechanisms handle this, both with drawbacks:

### 1. Wrapper scripts (`wrapNixGL` / `makeWrapper`)

The `wrapNixGL` mechanism (from nix-lib) and the `wrapWithLibs` /
`makeWrapper` pattern in `mkWidgetPackage` generate shell wrapper
scripts that set environment variables before exec-ing the real binary:

```bash
#!/nix/store/...-bash/bin/bash
export LIBGL_DRIVERS_PATH=/nix/store/...-mesa/lib/dri
export GBM_BACKENDS_PATH=/nix/store/...-mesa/lib
export __EGL_VENDOR_LIBRARY_FILENAMES=/nix/store/...-mesa/share/glvnd/egl_vendor.d/50_mesa.json
export LD_LIBRARY_PATH=/nix/store/...-wayland/lib:/nix/store/...-mesa/lib:...
exec /nix/store/...-widget/.widget-wrapped "$@"
```

Problems:
- Extra process (bash) for every widget launch
- Fragile: `LD_LIBRARY_PATH` is a global override, can leak into child
  processes or conflict with other libraries
- Cannot inspect the real binary's dependencies with `ldd` or
  `patchelf --print-rpath` without unwrapping

### 2. RUSTFLAGS rpath (`-C link-args=-Wl,-rpath,...`)

The `CARGO_TARGET_*_RUSTFLAGS` approach bakes rpath into binaries at
compile time:

```nix
CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_RUSTFLAGS =
  "-C link-args=-Wl,-rpath,${lib.makeLibraryPath [ mesa wayland ... ]}";
```

Problems:
- All-or-nothing: every binary in the crate gets the same rpath,
  even binaries that do not need Mesa or wayland
- Bloats the rpath of simple binaries with unnecessary entries
- Harder to reason about per-binary dependencies

### Goal

Replace both mechanisms with **post-build patchelf patching**: after
compilation, use `patchelf` to set per-binary rpath and interpreter.
Each binary gets exactly the library paths it needs -- no wrapper
scripts, no environment variables, no global RUSTFLAGS rpath.

---

## Inspiration: nonguix patchelf-plan

The [nonguix](https://gitlab.com/nonguix/nonguix) project for GNU Guix
provides a
[binary build system](https://gitlab.com/nonguix/nonguix/-/blob/master/nonguix/build/binary-build-system.scm)
that uses a declarative "patchelf-plan" to patch pre-built binaries.
The key ideas:

### Data structure

Each entry is `(binary runpath-list)`:

```scheme
(patchelf-plan
  `(("bin/my-app"    ("mesa" "wayland" ("nss" "/lib/nss")))
    ("bin/my-tool"   ("glib"))))
```

- `"mesa"` resolves to `<mesa-output>/lib` (default `/lib` suffix)
- `("nss" "/lib/nss")` resolves to `<nss-output>/lib/nss` (custom suffix)
- Package names are looked up in the derivation's `inputs` and `outputs`

### Patchelf phase

The phase runs before install and for each entry:

1. Resolves each runpath name to a store path + suffix
2. Detects whether the binary is 64-bit or 32-bit
3. Sets the ELF interpreter to the correct glibc dynamic linker
   (`ld-linux.so` for 64-bit, `ld-linux-armhf.so` for 32-bit)
4. Sets the rpath to the resolved library paths (colon-separated)

### Design principles

- **Declarative**: the plan is data, not imperative code
- **Per-binary granularity**: different binaries get different rpaths
- **Automatic interpreter selection**: based on ELF bitness
- **Package references resolved from build context**: names map to
  actual store paths from the derivation's dependency graph

---

## Proposed Nix implementation

### Data structure: `patchelfPlan`

A list of attrsets, each specifying a binary and its rpath
dependencies:

```nix
patchelfPlan = [
  {
    binary = "bin/bmc-widget-digital-clock";
    rpath = [ mesa libglvnd wayland libxkbcommon ];
  }
  {
    binary = "bin/bmc-compositor";
    rpath = [
      mesa
      libglvnd
      wayland
      libxkbcommon
      libinput
      seatd
      udev
      libdrm
      libgbm
    ];
  }
];
```

Each `rpath` entry can be:
- A **derivation** -- appends `/lib` automatically
- An **attrset** `{ pkg = mesa; path = "/lib/dri"; }` -- uses a
  custom subdirectory

This mirrors nonguix's string-vs-tuple distinction, adapted to Nix's
type system.

### Function signature

```nix
# patchelfBinaries :: {
#   drv         : derivation   -- the built package to patch
#   plan        : list         -- patchelf plan (see above)
#   stdenv      : derivation   -- provides patchelf + interpreter
#   extraRpath  : list         -- extra rpath entries for all binaries (optional)
# } -> derivation
patchelfBinaries = { drv, plan, stdenv, extraRpath ? [] }: ...
```

Returns a new derivation that copies `drv` and patches each binary
according to the plan.

### Integration points

Two possible integration strategies:

**Option A: postInstall phase in buildCrate.nix**

Add patchelf logic to the existing `buildCrate` function in nix-lib.
The build profile would accept a `patchelfPlan` parameter:

```nix
armv7-glibc-release = workspace.mkBuildProfile {
  suffix = "armv7";
  rustProfile = "release";
  rustCrossTarget = "armv7-unknown-linux-gnueabihf";
  build_pkgs = fixedArmv7Pkgs;
  patchelfPlan = [
    { binary = "bin/bmc-widget-digital-clock"; rpath = with fixedArmv7Pkgs; [ mesa libglvnd wayland ]; }
  ];
};
```

Pros: single build step, no extra derivation.
Cons: couples patchelf logic into the generic `buildCrate` function;
the plan applies to ALL crates built with this profile, but different
crates need different rpaths.

**Option B: separate post-build function (recommended)**

Apply `patchelfBinaries` after `buildCrate`, as a wrapper derivation.
This is the approach used in `mkWidgetPackage`:

```nix
mkWidgetPackage = { name, crate, profile, features ? [], patchelfPlan ? [] }:
  let
    binary = profile.buildCrate crate { inherit features; };
    patched = if patchelfPlan == [] then binary
              else patchelfBinaries {
                drv = binary;
                plan = patchelfPlan;
                inherit (profile) stdenv;
              };
  in
  pkgs.runCommand "bmc-widget-${name}" { } ''
    mkdir -p $out/lib/bmc-widgets/${name}/bin
    cp ${patched}/bin/* $out/lib/bmc-widgets/${name}/bin/
    ...
  '';
```

Pros: per-binary granularity, does not pollute `buildCrate`, can be
applied selectively.
Cons: extra derivation in the build graph (but it is fast -- just
`cp` + `patchelf`).

**Recommendation: Option B.** The patchelf plan is specific to the
application layer (widgets, compositor), not to the generic crate
build system. Keeping it separate is cleaner and more flexible.

### Interpreter handling

For cross-compiled ARMv7 binaries, the ELF interpreter must point to
the correct glibc's `ld-linux-armhf.so.3`. In Nix cross-compilation:

```nix
interpreter = "${stdenv.cc.libc}/lib/ld-linux-armhf.so.3";
```

Where `stdenv` is the cross stdenv for `armv7l-hf-multiplatform`.
The function detects the interpreter from the target's libc
automatically -- no manual path needed:

```nix
interpreter = "${stdenv.cc.libc}/lib/${stdenv.cc.libc.libName or "ld-linux-armhf.so.3"}";
```

Or more robustly, read the existing interpreter from the binary and
only replace the prefix:

```bash
old_interp=$(patchelf --print-interpreter "$binary")
base_interp=$(basename "$old_interp")
patchelf --set-interpreter "${libc}/lib/${base_interp}" "$binary"
```

This handles both ARM (`ld-linux-armhf.so.3`) and x86
(`ld-linux-x86-64.so.2`) without conditional logic.

---

## Implementation sketch

```nix
# nix/patchelf-binaries.nix
#
# Patch ELF binaries with per-binary rpath and interpreter.
# Replaces wrapper scripts and global RUSTFLAGS rpath.
{ lib, patchelf, stdenv }:

{ drv, plan, extraRpath ? [] }:

let
  # Resolve a single rpath entry to a store path string.
  # Entry is either a derivation (-> $out/lib) or an attrset
  # { pkg = <drv>; path = "/lib/dri"; }.
  resolveRpathEntry = entry:
    if builtins.isAttrs entry && entry ? pkg
    then "${lib.getLib entry.pkg}${entry.path}"
    else "${lib.getLib entry}/lib";

  # Resolve a full rpath list to a colon-separated string.
  resolveRpath = entries:
    let
      allEntries = entries ++ extraRpath;
      paths = map resolveRpathEntry allEntries;
    in
    lib.concatStringsSep ":" paths;

  # The interpreter from the cross-compilation libc.
  libc = stdenv.cc.libc;

  # Generate a patchelf command for one plan entry.
  mkPatchCmd = { binary, rpath ? [] }:
    let
      rpathStr = resolveRpath rpath;
    in
    ''
      echo "Patching ${binary}..."
      if [ ! -f "$out/${binary}" ]; then
        echo "ERROR: binary not found: $out/${binary}" >&2
        exit 1
      fi

      # Read existing interpreter basename to handle ARM/x86 automatically
      old_interp=$(${patchelf}/bin/patchelf --print-interpreter "$out/${binary}" 2>/dev/null || true)
      if [ -n "$old_interp" ]; then
        base_interp=$(basename "$old_interp")
        ${patchelf}/bin/patchelf \
          --set-interpreter "${libc}/lib/$base_interp" \
          --set-rpath "${rpathStr}" \
          "$out/${binary}"
      else
        # Static binary or no interpreter -- only set rpath
        ${patchelf}/bin/patchelf \
          --set-rpath "${rpathStr}" \
          "$out/${binary}"
      fi
    '';

  patchCommands = lib.concatStringsSep "\n" (map mkPatchCmd plan);

in
stdenv.mkDerivation {
  name = "${drv.name}-patched";
  src = drv;

  nativeBuildInputs = [ patchelf ];

  dontUnpack = true;
  dontBuild = true;
  dontPatchELF = true;  # Prevent fixupPhase from undoing our changes
  dontStrip = true;      # Already stripped by buildCrate

  installPhase = ''
    cp -r --no-preserve=mode $src $out

    ${patchCommands}
  '';
}
```

### Rpath entry resolution examples

```nix
# Simple derivation -> $out/lib
resolveRpathEntry mesa
# => "/nix/store/...-mesa-24.3.1/lib"

# Custom path for DRI drivers
resolveRpathEntry { pkg = mesa; path = "/lib/dri"; }
# => "/nix/store/...-mesa-24.3.1/lib/dri"

# libglvnd
resolveRpathEntry libglvnd
# => "/nix/store/...-libglvnd-1.7.0/lib"
```

---

## Usage examples

### In workspace.nix: widget package with patchelf

```nix
# Import the patchelf function
patchelfBinaries = import ./nix/patchelf-binaries.nix {
  inherit lib;
  inherit (pkgs) patchelf;
  stdenv = fixedArmv7Pkgs.stdenv;
};

# Common rpath dependencies for GL widgets
widgetGlRpath = with fixedArmv7Pkgs; [
  mesa
  libglvnd
  wayland
  libxkbcommon
  fontconfig
  freetype
];

mkWidgetPackage = { name, crate, profile, features ? [] }:
  let
    binary = profile.buildCrate crate { inherit features; };
    patched = patchelfBinaries {
      drv = binary;
      plan = [
        {
          binary = "bin/bmc-widget-${name}";
          rpath = widgetGlRpath;
        }
      ];
    };
    widgetSrc = ./widgets + "/${name}";
  in
  pkgs.runCommand "bmc-widget-${name}" { } ''
    mkdir -p $out/lib/bmc-widgets/${name}/bin
    cp ${widgetSrc}/manifest.json $out/lib/bmc-widgets/${name}/
    cp ${patched}/bin/* $out/lib/bmc-widgets/${name}/bin/
    if [ -d "${widgetSrc}/assets" ]; then
      cp -r ${widgetSrc}/assets $out/lib/bmc-widgets/${name}/
    fi
  '';
```

### Per-binary rpath for a multi-binary package

```nix
compositorPatched = patchelfBinaries {
  drv = profile.buildCrate crates.bmc-compositor { };
  plan = [
    {
      binary = "bin/bmc-compositor";
      rpath = with fixedArmv7Pkgs; [
        mesa
        { pkg = mesa; path = "/lib/dri"; }
        libglvnd
        wayland
        libxkbcommon
        libinput
        seatd
        udev
        libdrm
        libgbm
      ];
    }
    {
      binary = "bin/bmc-tool";
      rpath = with fixedArmv7Pkgs; [
        wayland
        libxkbcommon
      ];
    }
  ];
};
```

### Verification

After building, verify the patched binary:

```bash
nix build .#widgets-armv7-glibc-release

# Check interpreter points to Nix store glibc
patchelf --print-interpreter result/lib/bmc-widgets/digital-clock/bin/bmc-widget-digital-clock
# /nix/store/...-glibc-2.40/lib/ld-linux-armhf.so.3

# Check rpath contains the expected store paths
patchelf --print-rpath result/lib/bmc-widgets/digital-clock/bin/bmc-widget-digital-clock
# /nix/store/...-mesa-.../lib:/nix/store/...-wayland-.../lib:...

# Verify the binary finds all libraries
# (requires qemu-user or ARM device)
ldd result/lib/bmc-widgets/digital-clock/bin/bmc-widget-digital-clock
```

---

## Mesa DRI special handling

Even with rpath pointing to `$mesa/lib`, Mesa's DRI drivers at
`$mesa/lib/dri/*.so` are not found via the standard ELF rpath
mechanism. DRI drivers are loaded by Mesa's internal loader, which
searches:

1. `$LIBGL_DRIVERS_PATH` environment variable
2. A compiled-in default path (nixpkgs patches this to
   `/run/opengl-driver/lib/dri/`)

This means **patchelf alone is not sufficient for GL binaries with
stock nixpkgs Mesa**. Two paths forward:

### Path A: patchelf + minimal wrapper (before Mesa overlay)

Use patchelf for all standard shared libraries (wayland, libxkbcommon,
libglvnd, etc.) but keep a minimal wrapper or env var for DRI:

```nix
# Only the DRI path needs an env var -- everything else is rpath
makeWrapper "$bin" "$out/bin/wrapper" \
  --set LIBGL_DRIVERS_PATH "${mesa}/lib/dri"
```

This is a stepping stone -- most of the wrapper complexity is gone,
only a single env var remains.

### Path B: patchelf alone (after Mesa overlay -- Stage 3)

Once Mesa is built with the self-contained overlay (see
`mesa-libglvnd.md`), the DRI loader uses `$out/lib/dri/` as its
default search path instead of `/run/opengl-driver/lib/dri/`. At that
point:

- The binary's rpath includes `$mesa/lib` (for `libEGL_mesa.so`,
  `libgbm.so`, etc.)
- Mesa's internal DRI loader finds `$mesa/lib/dri/` automatically
  (because the overlay removed the `/run/opengl-driver` patch)
- No environment variables needed

**This is the target state.** Patchelf fully replaces both `wrapNixGL`
and RUSTFLAGS rpath.

### What about `__EGL_VENDOR_LIBRARY_FILENAMES`?

This is a libglvnd concern, not ELF rpath. libglvnd searches for
vendor JSON files (`50_mesa.json`) in a compiled-in path. The Mesa
overlay should also ensure libglvnd finds the correct vendor JSON
from `$mesa/share/glvnd/egl_vendor.d/`. If not, a similar overlay
for libglvnd is needed (or a single remaining env var).

---

## Migration path

```
Current state (Stage 1):
  wrapNixGL wrapper scripts set LD_LIBRARY_PATH + Mesa env vars
  RUSTFLAGS rpath for native x86 builds
  |
  v
Stage 3: Mesa overlay (mesa-libglvnd.md)
  Build Mesa with self-contained DRI paths
  Remove /run/opengl-driver dependency
  |
  v
Stage 4: patchelf-binaries function (this document)
  1. Add nix/patchelf-binaries.nix
  2. Define patchelfPlan per widget/compositor
  3. Apply in mkWidgetPackage after buildCrate
  4. Verify: patchelf --print-rpath shows correct store paths
  5. Verify: binaries run on device without env vars
  |
  v
Cleanup:
  1. Remove wrapNixGL = true from build profiles
  2. Remove wrapWithLibs / makeWrapper from mkWidgetPackage
  3. Remove CARGO_TARGET_*_RUSTFLAGS rpath entries from workspace.nix
     (keep only -L native= for link-time library search, not rpath)
  4. Remove Mesa env var passing from bmc/src/widget/spawner.rs
     (LinkerConfig fields already removed in Stage 1)
  5. Remove runtimeLibs list from workspace.nix
```

### What can be done before Stage 3

For **non-GL binaries** (hypothetical tools that only need wayland,
libxkbcommon, etc. but no Mesa/EGL), patchelf works immediately. No
Mesa overlay needed.

For **GL binaries**, either:
- Use patchelf for non-Mesa deps + minimal wrapper for
  `LIBGL_DRIVERS_PATH` only (Path A above)
- Wait for Stage 3 Mesa overlay, then apply patchelf for everything

The recommended approach is to implement the `patchelfBinaries`
function now (it is generic) and start using it as part of Stage 4
when the Mesa overlay is ready.

---

## Open questions

1. **`lib.getLib` vs `lib.getOutput "lib"`** -- which is the correct
   way to get the lib output for split-output packages? Need to verify
   that `lib.getLib mesa` returns the lib output, not the main output.
   For packages without a separate lib output, `lib.getLib` should
   fall back to `$out`.

2. **Shared objects in the patched binary** -- `cp -r --no-preserve=mode`
   copies all files from the original derivation. If the derivation
   contains symlinks to shared objects (e.g., `libfoo.so -> libfoo.so.1`),
   does the copy preserve them correctly? Need to verify.

3. **Interaction with `autoPatchelfHook`** -- nixpkgs has
   `autoPatchelfHook` which automatically patches ELF binaries during
   `fixupPhase`. Could we use it instead of a custom function? It
   auto-discovers needed libraries from `buildInputs`. However, it
   patches ALL binaries uniformly -- no per-binary granularity. And it
   is designed for packages that declare their deps via `buildInputs`,
   not for Cargo/Rust builds where deps come from the workspace config.
   Likely not suitable, but worth investigating.

4. **Stripping** -- Nix's `fixupPhase` normally strips binaries. Our
   custom derivation uses `dontBuild = true` but still runs
   `installPhase`. Need to verify that the patched binary is not
   accidentally stripped again (or ensure stripping already happened
   in the original `buildCrate` derivation).

5. **Reproducibility** -- patchelf modifies binaries in-place. The
   patched derivation has a different store hash than the original.
   This is expected and correct, but means the patched derivation
   depends on both the original binary AND all the rpath dependencies.
   `nix why-depends` should show this clearly.

6. **Native x86 builds** -- the current `fast` profile (used for
   `bmc-mock` and x86 widgets in dev) uses RUSTFLAGS rpath. Should
   patchelf also be used for x86 dev builds, or is RUSTFLAGS rpath
   acceptable there? Patchelf would be more consistent, but RUSTFLAGS
   rpath is simpler for dev and does not require Mesa overlay work
   (NixOS/nixGL handles Mesa on x86 developer machines).

---

## Resolution (BDK-353)

Open question 3 ("Interaction with autoPatchelfHook") turned out to
be the answer. The key insight missed in the original analysis:
`autoPatchelfHook` supports a `runtimeDependencies` attribute that
adds `/lib` paths to rpath for ALL dynamic executables, regardless of
DT_NEEDED. This solves the dlopen problem without per-binary
granularity — which was acceptable because all widget binaries need
the same GL runtime deps, and the compositor needs a superset.

Answers to remaining open questions:
1. `lib.getLib` — used in `autopatchelf-binaries.nix`, works correctly
   for split-output packages.
2. Shared object symlinks — not relevant; `overrideAttrs` modifies the
   original derivation in-place rather than copying.
3. autoPatchelfHook — adopted as the solution. See above.
4. Stripping — not an issue; `autoPatchelfHook` runs in
   `postFixupHooks` after stripping, and modifies the original
   derivation (no separate copy).
5. Reproducibility — same as original: the derivation hash changes
   because the hook modifies binaries. `nix why-depends` shows the
   runtimeDependencies in the closure.
6. Native x86 builds — RUSTFLAGS rpath kept for x86 dev builds.
   autoPatchelfHook only applied to ARM cross-compiled builds.
