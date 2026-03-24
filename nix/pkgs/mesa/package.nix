# from https://github.com/NixOS/nixpkgs/blob/nixpkgs-unstable/pkgs/development/libraries/mesa/default.nix
{ lib
, bison
, buildPackages
, directx-headers
, elfutils
, expat
, fetchCrate
, fetchFromGitLab
, file
, flex
, glslang
, spirv-tools
, intltool
, jdupes
, libdrm
, libpng
, libunwind
, lm_sensors
, meson
, ninja
, pkg-config
, python3Packages
, rust-bindgen
, rust-cbindgen
, rustPlatform
, rustc
, stdenv
, udev
, wayland
, wayland-protocols
, wayland-scanner
, xcbutilkeysyms
, libx11
, libxcb
, libxext
, libxfixes
, libxrandr
, libxshmfence
, libxxf86vm
, xorgproto
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
  rustDeps = [
    {
      pname = "paste";
      version = "1.0.14";
      hash = "sha256-+J1h7New5MEclUBvwDQtTYJCHKKqAEOeQkuKy+g0vEc=";
    }
    {
      pname = "proc-macro2";
      version = "1.0.86";
      hash = "sha256-9fYAlWRGVIwPp8OKX7Id84Kjt8OoN2cANJ/D9ZOUUZE=";
    }
    {
      pname = "quote";
      version = "1.0.33";
      hash = "sha256-VWRCZJO0/DJbNu0/V9TLaqlwMot65YjInWT9VWg57DY=";
    }
    {
      pname = "syn";
      version = "2.0.68";
      hash = "sha256-nGLBbxR0DFBpsXMngXdegTm/o13FBS6QsM7TwxHXbgQ=";
    }
    {
      pname = "unicode-ident";
      version = "1.0.12";
      hash = "sha256-KX8NqYYw6+rGsoR9mdZx8eT1HIPEUUyxErdk2H/Rlj8=";
    }
  ];

  copyRustDep = dep: ''
    cp -R --no-preserve=mode,ownership ${fetchCrate dep} subprojects/${dep.pname}-${dep.version}
    cp -R subprojects/packagefiles/${dep.pname}/* subprojects/${dep.pname}-${dep.version}/
  '';

  copyRustDeps = lib.concatStringsSep "\n" (builtins.map copyRustDep rustDeps);

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

    ${copyRustDeps}
  '';

  # Keep build-ids so drivers can use them for caching, etc.
  # Also some drivers segfault without this.
  separateDebugInfo = true;
  __structuredAttrs = true;

  mesonFlags = [
    "--sysconfdir=/etc"

    # What to build
    (lib.mesonOption "platforms" (lib.concatStringsSep "," eglPlatforms))
    (lib.mesonOption "gallium-drivers" (lib.concatStringsSep "," galliumDrivers))
    (lib.mesonOption "vulkan-drivers" "")
    (lib.mesonOption "vulkan-layers" "")

    (lib.mesonEnable "glvnd" false)
    (lib.mesonEnable "gbm" true)
    (lib.mesonBool "libgbm-external" false)

    (lib.mesonBool "gallium-nine" false) # Direct3D9 in Wine, largely supplanted by DXVK

    # Only used by xf86-video-vmware, which has more features than VMWare's KMS driver,
    # so we're keeping it for now. Should be removed when that's no longer the case.
    # See: https://github.com/NixOS/nixpkgs/pull/392492
    (lib.mesonEnable "gallium-xa" false)

    (lib.mesonBool "teflon" false) # TensorFlow frontend

    (lib.mesonEnable "xlib-lease" false)
    (lib.mesonEnable "glx" false)
    (lib.mesonEnable "gallium-vdpau" false)
    (lib.mesonEnable "gallium-va" false)

    (lib.mesonEnable "llvm" false)
    (lib.mesonEnable "gallium-opencl" false)

    # Enable all freedreno kernel mode drivers. (For example, virtio can be
    # used with a virtio-gpu device supporting drm native context.) This option
    # is ignored when freedreno is not being built.
    (lib.mesonOption "freedreno-kmds" "msm,kgsl,virtio,wsl")

    # Enable Intel RT stuff when available
    (lib.mesonEnable "intel-rt" stdenv.hostPlatform.isx86_64)

    # Rusticl, new OpenCL frontend
    (lib.mesonBool "gallium-rusticl" false)
    #(lib.mesonOption "gallium-rusticl-enable-drivers" "auto")

    # meson auto_features enables this, but we do not want it
    (lib.mesonEnable "android-libbacktrace" false)
    (lib.mesonEnable "microsoft-clc" false) # Only relevant on Windows (OpenCL 1.2 API on top of D3D12)

    # Enable more sensors in gallium-hud
    (lib.mesonBool "gallium-extra-hud" false)

    (lib.mesonOption "tools" "")
    (lib.mesonBool "install-mesa-clc" false)
    (lib.mesonBool "install-precomp-compiler" false)
    (lib.mesonEnable "valgrind" false)
  ];

  strictDeps = true;

  buildInputs =
    [
      directx-headers
      elfutils
      expat
      spirv-tools
      libdrm
      libpng
      libunwind
      libx11
      libxcb
      libxext
      libxfixes
      libxrandr
      libxshmfence
      libxxf86vm
      lm_sensors
      python3Packages.python # for shebang
      udev
      wayland
      wayland-protocols
      xcbutilkeysyms
      xorgproto
      zstd
    ];

  depsBuildBuild = [
    pkg-config
    buildPackages.stdenv.cc
  ];

  nativeBuildInputs = [
    meson
    pkg-config
    ninja
    intltool
    bison
    flex
    file
    python3Packages.python
    python3Packages.packaging
    python3Packages.pycparser
    python3Packages.mako
    python3Packages.ply
    python3Packages.pyyaml
    jdupes
    # Use bin output from glslang to not propagate the dev output at
    # the build time with the host glslang.
    (lib.getBin glslang)
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
