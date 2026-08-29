# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

# Originally derived from
# https://github.com/NixOS/nixpkgs/blob/nixpkgs-unstable/pkgs/development/libraries/mesa/default.nix,
# trimmed for a wayland-only Vivante GC400 build (no X11, no Vulkan,
# no LLVM/OpenCL, no software pipes, no video codecs).
#
# The nixpkgs original is distributed under the MIT license:
#
# Copyright (c) 2003-2026 Eelco Dolstra and the Nixpkgs/NixOS contributors
#
# Permission is hereby granted, free of charge, to any person obtaining
# a copy of this software and associated documentation files (the
# "Software"), to deal in the Software without restriction, including
# without limitation the rights to use, copy, modify, merge, publish,
# distribute, sublicense, and/or sell copies of the Software, and to
# permit persons to whom the Software is furnished to do so, subject to
# the following conditions:
#
# The above copyright notice and this permission notice shall be
# included in all copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
# EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
# MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
# NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS
# BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN
# ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
# CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
# SOFTWARE.
{ lib
, bison
, buildPackages
, expat
, fetchCrate
, fetchFromGitLab
, flex
, jdupes
, libdisplay-info
, libdrm
, libunwind
, llvmPackages
, lm_sensors
, spirv-tools
, meson
, ninja
, pkg-config
, python3Packages
, runCommand
, rust-bindgen
, rust-cbindgen
, rustPlatform
, rustc
, stdenv
, udev
, wayland
, wayland-protocols
, wayland-scanner
, zlib
, zstd
, galliumDrivers ? [
    "etnaviv"
  ]
, eglPlatforms ? [
    "wayland"
  ]
, mesa
, mesa-gl-headers
, makeSetupHook
,
}:

let
  # `llvmpipe` is Mesa's software rasterizer,
  # used by headless CI tests that need real GL without a GPU.
  #
  # It compiles SPIR-V via LLVM, so the LLVM stack
  # only comes along on builds that select it.
  needsLlvm = lib.elem "llvmpipe" galliumDrivers;

  # Mesa ≥26 fetches Rust dependencies via meson's wrap mechanism, looking
  # for `<pname>-<version>.tar.gz` files in `MESON_PACKAGE_CACHE_DIR`.
  # We mirror nixpkgs' approach: build the cache from `wraps.json`.
  rustDeps = lib.importJSON ./wraps.json;

  fetchDep =
    dep:
    fetchCrate {
      inherit (dep) pname version hash;
      unpack = false;
    };

  toCommand = dep: "ln -s ${dep} $out/${dep.pname}-${dep.version}.tar.gz";

  packageCacheCommand = lib.pipe rustDeps [
    (map fetchDep)
    (map toCommand)
    (lib.concatStringsSep "\n")
  ];

  packageCache = runCommand "mesa-rust-package-cache" { } ''
    mkdir -p $out
    ${packageCacheCommand}
  '';

  needNativeCLC = !stdenv.buildPlatform.canExecute stdenv.hostPlatform;

  common = import ./common.nix { inherit lib fetchFromGitLab; };
in
stdenv.mkDerivation {
  inherit (common)
    pname
    version
    src
    meta
    ;

  patches = [
    ./opencl.patch
  ];

  postPatch = ''
    patchShebangs .

    for header in ${toString mesa-gl-headers.headers}; do
      if ! diff -q $header ${mesa-gl-headers}/$header; then
        echo "File $header does not match between mesa and mesa-gl-headers, please update mesa-gl-headers first!"
        exit 42
      fi
    done
  '';

  # Keep build-ids so drivers can use them for caching, etc.
  # Also some drivers segfault without this.
  separateDebugInfo = true;

  # Strip symbol tables from libraries to reduce size.
  stripAllList = [ "lib" ];
  __structuredAttrs = true;

  env.MESON_PACKAGE_CACHE_DIR = packageCache;

  # llvmpipe's build invokes `llvm-config` from `$PATH`.
  # The default Mesa build doesn't pull LLVM in,
  # so put it on PATH only when we actually build llvmpipe.
  preConfigure = lib.optionalString needsLlvm ''
    PATH=${lib.getDev llvmPackages.libllvm}/bin:$PATH
  '';

  # Minimal mesa for Vivante GC400 on the Deck:
  #   - one gallium driver (etnaviv)
  #   - one EGL platform (wayland)
  #   - everything else explicitly disabled to keep the closure small,
  #     drop unrelated codegen, and avoid pulling in LLVM/Vulkan/OpenCL/etc.
  # Options that no longer exist in mesa ≥26 are omitted (gallium-nine,
  # gallium-xa, xlib-lease, glx, gallium-vdpau, gallium-va, gallium-opencl,
  # llvm, install-mesa-clc, install-precomp-compiler, tools).
  mesonFlags = [
    "--sysconfdir=/etc"

    # Size optimization
    "--buildtype=release"
    "--optimization=s"

    # What to build
    (lib.mesonOption "platforms" (lib.concatStringsSep "," eglPlatforms))
    (lib.mesonOption "gallium-drivers" (lib.concatStringsSep "," galliumDrivers))
    (lib.mesonOption "vulkan-drivers" "")
    (lib.mesonOption "vulkan-layers" "")

    (lib.mesonEnable "glvnd" false)
    (lib.mesonEnable "gbm" true)
    (lib.mesonBool "libgbm-external" false)

    (lib.mesonBool "teflon" false) # TensorFlow frontend
    (lib.mesonBool "amdgpu-virtio" false) # AMD virtio native context
    (lib.mesonBool "gallium-rusticl" false) # OpenCL frontend
    (lib.mesonBool "gallium-extra-hud" false) # extra HUD sensors

    # X11 paths — auto-enabled by meson but irrelevant for our wayland-only build.
    (lib.mesonEnable "xlib-lease" false)
    (lib.mesonEnable "glx" false)

    # Video acceleration API — none of the drivers it requires are built.
    (lib.mesonEnable "gallium-va" false)

    # LLVM is only needed for software pipes / OpenCL / radeonsi — not for etnaviv.
    (lib.mesonEnable "llvm" needsLlvm)
  ] ++ lib.optionals needsLlvm [
    # Mesa locates Clang's runtime libs to drive its SPIR-V → CPU compile path
    # for llvmpipe / OpenCL.
    #
    # Without this, the build picks up Clang from a path outside the closure
    # and the produced llvmpipe driver fails to load.
    (lib.mesonOption "clang-libdir" "${lib.getLib llvmPackages.clang-unwrapped}/lib")
  ] ++ [

    # Default to all freedreno kernel mode drivers. Ignored when freedreno
    # is not being built (we only build etnaviv).
    (lib.mesonOption "freedreno-kmds" "msm,kgsl,virtio,wsl")

    (lib.mesonEnable "intel-rt" stdenv.hostPlatform.isx86_64)

    # auto_features wants these on; we don't.
    (lib.mesonEnable "gallium-mediafoundation" false) # Windows
    (lib.mesonEnable "android-libbacktrace" false)
    (lib.mesonEnable "microsoft-clc" false) # Windows (OpenCL on D3D12)
    (lib.mesonEnable "valgrind" false)
  ];

  strictDeps = true;

  buildInputs = [
    expat
    libdisplay-info
    libdrm
    libunwind
    lm_sensors
    spirv-tools
    udev
    wayland
    wayland-protocols
    zlib
    zstd
  ] ++ lib.optionals needsLlvm [
    llvmPackages.libllvm
  ];

  depsBuildBuild = [
    pkg-config
    buildPackages.stdenv.cc
  ];

  nativeBuildInputs = [
    meson
    pkg-config
    ninja
    bison
    flex
    python3Packages.python
    python3Packages.packaging
    python3Packages.pycparser
    python3Packages.mako
    python3Packages.pyyaml
    jdupes
    rustc
    rust-bindgen
    rust-cbindgen
    rustPlatform.bindgenHook
    wayland-scanner
  ]
  ++ lib.optionals needNativeCLC [
    # `or null` to not break eval with `attribute missing` on darwin to linux cross
    (buildPackages.mesa.cross_tools or null)
  ];

  disallowedRequisites = lib.optional
    (
      needNativeCLC && buildPackages.mesa ? cross_tools
    )
    buildPackages.mesa.cross_tools;

  doCheck = false;

  postFixup = ''
    # and in Vulkan layer manifests
    for js in $out/share/vulkan/{im,ex}plicit_layer.d/*.json; do
      substituteInPlace "$js" --replace '"libVkLayer_' '"'"$out/lib/libVkLayer_"
    done

    # remove DRI pkg-config file, provided by dri-pkgconfig-stub
    rm -f $out/lib/pkgconfig/dri.pc

    # remove headers moved to mesa-gl-headers
    for header in ${toString mesa-gl-headers.headers}; do
      rm -f $out/$header
    done

    # clean up after removing stuff
    find $out -type d -empty -delete

    # Don't depend on build python
    patchShebangs --host --update $out/bin/*

    # NAR doesn't support hard links, so convert them to symlinks to save space.
    jdupes --hard-links --link-soft --recurse "$out"
  '';

  passthru = {
    driverLink = throw "mesa.driverLink is disabled — glvnd is off";
    providesEglLoader = true;
    inherit
      eglPlatforms
      galliumDrivers;

    # for compatibility
    drivers = lib.warn "`mesa.drivers` is deprecated, use `mesa` instead" mesa;

    llvmpipeHook = makeSetupHook
      {
        name = "llvmpipe-hook";
        substitutions.mesa = mesa;
      } ./llvmpipe-hook.sh;
  };
}
