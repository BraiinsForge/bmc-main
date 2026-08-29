# Copyright (C) 2025  Braiins Systems s.r.o.
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

# Workspace config for Rust builds. Defines commonDeps as single source
# of truth for dependency definitions shared with devShells.
{ self, pkgs }:
let lib = pkgs.lib; in
let
  rustflags = import ./nix/rustflags.nix { inherit lib; };
  inherit (rustflags) makeRustflagsEnv;
  # Overlay used by every package set that targets the OpenWRT appliance image — HW (armv7)
  # and VM (x86, aarch64). The image runs mdev rather than a udev daemon, so libinput must
  # be linked against libudev-zero rather than systemd's libudev.
  #
  # `compositorUdev` carries the matching libudev.so.1 into the runtime closure without
  # rebinding `udev` globally, which would pull unrelated packages (lvm2, btrfs-progs)
  # onto libudev-zero APIs they do not support.
  applianceOverlay = final: prev:
    let
      # nixpkgs ships libudev-zero 1.0.3, whose ID_INPUT_* tag synthesis in set_properties_from_evdev
      # checks EV_REL before EV_ABS — any device carrying both (e.g. QEMU's virtio-tablet-pci,
      # with REL_WHEEL + ABS_X/ABS_Y + BTN_TOUCH) falls through the REL branch and never reaches
      # the TOUCHSCREEN tag. libinput then refuses the device as "not tagged as supported input device".
      #
      # Upstream fix bbeb7ad5 ("Fixes incorrect detection of touchpads (#66)") swaps the branch
      # order so EV_ABS is checked first. Never released, so we pin to the fix commit directly
      # until a >=1.0.4 tag lands and nixpkgs picks it up.
      libudevZero = (prev."libudev-zero").overrideAttrs (_: {
        version = "unstable-2024-04-17";
        src = prev.fetchFromGitHub {
          owner = "illiliti";
          repo = "libudev-zero";
          rev = "bbeb7ad51c1edb7ab3cf63f30a21e9bb383b7994";
          hash = "sha256-hQoLnKpT/cnGyUl56DnHjZ0nfenLPI9EvmOejqEPxfc=";
        };
      });
    in
    {
      libinput = prev.libinput.override {
        udev = libudevZero;
        wacomSupport = false;
      };
      compositorUdev = libudevZero;
    };

  # Resolve the libudev provider for a compositor runtime closure:
  # appliance package sets expose `compositorUdev` (→ libudev-zero); the
  # native dev shell / bmc-mock closure falls back to `pkgs.udev`, since
  # libinput on a plain dev host still expects systemd udev.
  compositorUdev = pkgs: pkgs.compositorUdev or pkgs.udev;

  # Fix for linux-pam cross-compilation issue in nixpkgs-unstable
  # The man output fails to build for ARMv7 glibc targets
  armv7Pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.extend (final: prev:
    # Guard: only apply to cross-compiled (ARM target) packages, not to
    # build-host packages that share this overlay via splicing.
    lib.optionalAttrs (prev.stdenv.hostPlatform != prev.stdenv.buildPlatform) (
      (mesaOverlay { }) final prev
      // applianceOverlay final prev
      // {
        linux-pam = prev.linux-pam.overrideAttrs (old: {
          outputs = lib.filter (o: o != "man") (old.outputs or [ "out" ]);
        });
      }
    ));

  # Custom mesa overlay (glvnd disabled) so EGL works without NixOS's
  # /run/opengl-driver. galliumDrivers selects which GPU backends to build:
  #   armv7 (Deck hardware):   [ "etnaviv" ]
  #   x86/aarch64 (VM / dev):  [ "etnaviv" "virgl" "softpipe" ]
  mesaOverlay = { galliumDrivers ? [ "etnaviv" ] }: final: prev: {
    mesa = prev.callPackage ./nix/pkgs/mesa/package.nix {
      inherit galliumDrivers;
    };
    # dri-pkgconfig-stub references mesa.driverLink which our mesa disables (no glvnd).
    # Point it at mesa directly.
    dri-pkgconfig-stub = prev.writeTextFile {
      name = "dri-pkgconfig-stub";
      destination = "/lib/pkgconfig/dri.pc";
      text = ''
        dridriverdir=${final.mesa}/lib/dri

        Name: dri
        Version: ${final.mesa.version}
        Description: Graphics driver path stub (custom mesa, no glvnd)
      '';
    };
    libva = prev.libva.overrideAttrs {
      mesonFlags = lib.optionals prev.stdenv.hostPlatform.isLinux [
        "-Ddriverdir=${final.mesa}/lib/dri"
      ];
    };
    # Break infinite recursion mesa -> libdisplay-info ->
    # -> v4l-utils -> qt5compat -> .. -> libva-minimal
    v4l-utils = prev.v4l-utils.override {
      withGUI = false;
    };
    libva-minimal = prev.libva-minimal.overrideAttrs {
      mesonFlags = lib.optionals prev.stdenv.hostPlatform.isLinux [
        "-Ddriverdir=${final.mesa}/lib/dri"
      ];
    };
    libvdpau = prev.libvdpau.overrideAttrs {
      mesonFlags = lib.optionals prev.stdenv.hostPlatform.isLinux [
        "-Dmoduledir=${final.mesa}/lib/vdpau"
      ];
    };
  };
  vmGalliumDrivers = [ "etnaviv" "virgl" "softpipe" ];
  x86Pkgs = pkgs.extend (lib.composeExtensions
    (mesaOverlay { galliumDrivers = vmGalliumDrivers; })
    applianceOverlay);

  # Narrow pkgs overlay for the `ci` build profile: just Mesa with
  # llvmpipe so headless GL tests can run in the Nix sandbox. Doesn't
  # pull in `applianceOverlay`, so we don't drag compositor-runtime
  # mutations through the test env.
  ciPkgs = pkgs.extend (mesaOverlay { galliumDrivers = [ "llvmpipe" ]; });
  aarch64Pkgs = pkgs.pkgsCross.aarch64-multiplatform.extend (final: prev:
    # Same cross-splicing guard as armv7Pkgs — libinput and the
    # compositorUdev marker only belong on the cross-target side.
    lib.optionalAttrs (prev.stdenv.hostPlatform != prev.stdenv.buildPlatform) (
      (mesaOverlay { galliumDrivers = vmGalliumDrivers; }) final prev
      // applianceOverlay final prev
    ));

  # musl set for the minimal workspace. Needs applianceOverlay so
  # compositorUdev resolves to libudev-zero; without it the raw set falls
  # back to pkgs.udev, which on pkgsMusl is systemd — whose closure does
  # not build for armv7-musl. No mesaOverlay: the minimal targetDeps
  # carry no mesa.
  armv7MuslPkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsMusl.extend
    (final: prev:
      # Same cross-splicing guard as armv7Pkgs.
      lib.optionalAttrs (prev.stdenv.hostPlatform != prev.stdenv.buildPlatform)
        (applianceOverlay final prev));

  # Shared deps used by both package builds and devShells.
  # Single source of truth to keep build derivations and dev environments in sync.
  commonDeps = {
    # Rust build-time deps (protoc for protobufs, diffutils for cargo)
    buildDeps = pkgs: with pkgs; [
      protobuf
      diffutils
      pkg-config
    ];

    # Env vars needed by Slint for font rendering and runtime linking
    env = {
      LD_LIBRARY_PATH = "${lib.makeLibraryPath [
        pkgs.libgcc
        # rodio (bmc-wasm-runtime's `audio` feature) dynlinks libasound.so.2 at
        # exec time; nextest invokes test binaries with --list and fails to
        # start without alsa on the loader path.
        pkgs.alsa-lib
      ]}";
    };

    guiBuildDeps = pkgs: with pkgs; [
      fontconfig
      freetype
    ];

    guiTargetDeps = pkgs: with pkgs; [
      # Compositor dependencies (require dynamic linking)
      wayland
      libxkbcommon
      libinput
      (compositorUdev pkgs)
      libdrm
      mesa
      alsa-lib
    ];

    # Node.js tooling for frontend builds
    frontendDeps = pkgs: with pkgs; [ nodejs yarn ];
  };

  # Minimal workspace config for musl profiles (bmc-nix-cli, statically linked)
  workspaceMinimal = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    # No cargo-timings charts in $out: crate outputs become deck packages,
    # and identical chart paths across packages conflict in the profile union.
    timings = false;
    nativeDeps = _pkgs: (commonDeps.buildDeps _pkgs);
    # Workspace-deps step compiles ALL Cargo.lock entries, including
    # crates from glibc-only binaries (compositor, widgets). Provide
    # the system libraries their build.rs scripts need via pkg-config.
    # libinput is excluded — its crate loads it via dlopen at runtime,
    # not link-time pkg-config, so the deps step builds without it.
    targetDeps = pkgs: with pkgs; [
      wayland
      libxkbcommon
      (compositorUdev pkgs)
      libdrm
      alsa-lib
    ];
  };

  mkNativeWorkspace = attrs: pkgs.ii.rust.mkWorkspaceConfig ({
    src = ./.;
    timings = false;
    nativeDeps = _pkgs: (commonDeps.buildDeps _pkgs) ++ (commonDeps.guiBuildDeps _pkgs);
    # packages that will be cross-compiled for target arch
    targetDeps = commonDeps.guiTargetDeps;
    env = commonDeps.env;
  } // attrs);

  # Full workspace config for glibc profiles (widgets, compositor)
  workspace = mkNativeWorkspace { };

  workspaceGallery = mkNativeWorkspace {
    workspacePath = "bmc-gallery";
  };

  # Build a --remap-path-prefix flag for a ${storeDir}/<hash>-<name>
  # path. The target replaces the 32-char hash with a fixed filler,
  # preserving path length/shape so debuginfo layout stays predictable
  # while stripping the runtime dependency on the volatile hash.
  #
  # Hash is always 32 chars (base32 cryptographic digest); its offset
  # is storeDir + the "/" separator.
  mkStorePathRemapFlag = storePath:
    let
      hashOffset = builtins.stringLength builtins.storeDir + 1;
      hash = builtins.substring hashOffset 32 storePath;
      remapped = builtins.replaceStrings [ hash ] [ "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" ] storePath;
    in
    "--remap-path-prefix=${storePath}=${remapped}";

  # Hide the fenix-generated toolchain's volatile hash from produced
  # .wasm blobs. Every wasm workspace must reuse these flags, so that
  # neither the remap nor the stack reservation is silently lost.
  # The repository cargo config carries the same stack size for builds that
  # run outside nix; `wasm-stack-size` checks they agree.
  wasmRemapFlags = mkStorePathRemapFlag "${pkgs.ii.rust.toolchain}";
  wasmStackSize = 64 * 1024;
  wasmRustFlags = "${wasmRemapFlags} -C link-arg=-zstack-size=${toString wasmStackSize}";

  mkWasmWorkspace = workspacePath: pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    inherit workspacePath;
    nativeDeps = _pkgs: (commonDeps.buildDeps _pkgs);
    env = {
      CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS = wasmRustFlags;
    };
  };

  workspaceWasmExamples = mkWasmWorkspace "widgets-wasm-examples";
  workspaceWasmWidgets = mkWasmWorkspace "widgets-wasm";

  # Roots that host wasm widget crates.
  # Each is a cargo workspace whose immediate subdirectories
  # are individual widget crates (each with a manifest.json).
  #
  # Mirrors `bmc-wasm-runtime/tools/widget_root.py`.
  # Adding a root means updating both.
  wasmWidgetRoots = [
    {
      workspaceName = "wasmExamples";
      src = ./widgets-wasm-examples;
      workspace = workspaceWasmExamples;
    }
    {
      workspaceName = "wasmWidgets";
      src = ./widgets-wasm;
      workspace = workspaceWasmWidgets;
    }
  ];

  # Filesystem-derived catalog of every wasm widget across every root.
  # Single source of truth for downstream consumers (crates.nix, packages.nix,
  # checks.nix, wasm-widgets.nix).
  #
  # A widget is anything with `Cargo.toml` in a subdir of one of the roots;
  # per-entry flags drive which downstream consumer picks it up:
  #   - `isShippable` (has manifest.json): nix/packages.nix deck packages
  #   - `hasCaptureConfig`: nix/wasm-regression.nix regression reports
  #
  # Regression-only crates (e.g. stress-test) have no manifest.
  wasmWidgetCatalog =
    let
      mkEntry = root: name:
        let
          crateDir = root.src + "/${name}";
          manifestPath = crateDir + "/manifest.json";
          captureConfigPath = crateDir + "/capture/config.toml";
          isShippable = builtins.pathExists manifestPath;
        in
        lib.nameValuePair name {
          inherit name isShippable;
          inherit (root) workspaceName workspace;
          src = crateDir;
          manifest = if isShippable then manifestPath else null;
          wasmFile = (builtins.replaceStrings [ "-" ] [ "_" ] name) + ".wasm";
          hasCaptureConfig = builtins.pathExists captureConfigPath;
        };
      discover = root: lib.filter
        (n: builtins.pathExists (root.src + "/${n}/Cargo.toml"))
        (lib.attrNames (builtins.readDir root.src));
    in
    lib.listToAttrs (lib.concatMap
      (root: map (mkEntry root) (discover root))
      wasmWidgetRoots);

  bmc = {
    armv7-nixpkgs = armv7Pkgs;
    lib = import ./nix/lib.nix { inherit pkgs lib armv7Pkgs; };
    crates = import ./nix/crates.nix {
      inherit lib wasmWidgetCatalog;
      inherit (pkgs.ii.rust) defineCrate;
    };
    workspaces = {
      full = workspace;
      gallery = workspaceGallery;
      minimal = workspaceMinimal;
      wasmExamples = workspaceWasmExamples;
      wasmWidgets = workspaceWasmWidgets;
    };
    profiles = import ./nix/profiles.nix {
      inherit (bmc) workspaces;
      inherit rustflags pkgs armv7Pkgs armv7MuslPkgs x86Pkgs aarch64Pkgs ciPkgs;
    };
  };

  frontend = import ./frontend { inherit self pkgs; };

  # UI/alarm sounds baked into bmc-openwrt as BMC_SOUNDS_DIR and overlaid
  # into the bmc-virt guest image. Filtered to the mp3 payload so the store
  # path is content-addressed on the sounds alone.
  sounds = builtins.path {
    name = "bmc-sounds";
    path = ./assets/sounds;
    filter = path: type: type == "directory" || pkgs.lib.hasSuffix ".mp3" path;
  };

  # Runtime deps for dlopen'd libraries, split by widget type.
  # Functions accepting pkgs — resolved at the point of use.
  deps = {
    frontend = frontend.build;
    inherit sounds;
    widgetRuntimeDeps = {
      # Native GPU widgets: smithay/EGL dlopen's the full GL stack.
      # libgbm is part of mesa's lib/ output, no separate entry needed.
      native = pkgs: with pkgs; [
        wayland
        libxkbcommon
        libdrm
        mesa
        libGL
      ];
    };
    # Compositor: native widget deps + compositor-specific input libs.
    compositorRuntimeDeps = pkgs:
      (deps.widgetRuntimeDeps.native pkgs) ++ (with pkgs; [
        libinput
        (compositorUdev pkgs)
      ]);
  };

  # All widget definitions for building
  widgets = {
    flip-clock = {
      crate = bmc.crates.widget-flip-clock;
      features = [ "standalone" ];
      runtimeDepsKind = "native";
    };
  };

  # Build profile sets for the cartesian product
  # NOTE: Update README.md when changing these sets!
  glibcArchProfiles = [
    { arch = "armv7-glibc"; profile = "release"; }
    { arch = "armv7-glibc"; profile = "debug"; }
  ];

  # use each profile to build each crate
  crateTuples = lib.cartesianProduct {
    archProfile = glibcArchProfiles;
    crate = [
      { def = "bmc-openwrt"; }
    ];
  };

  cratePackages = builtins.listToAttrs (lib.forEach crateTuples ({ archProfile, crate }: {
    name = "${crate.def}-${archProfile.arch}-${archProfile.profile}";
    value = bmc.lib.autopatchelfBinaries {
      drv = bmc.profiles."${archProfile.arch}-${archProfile.profile}".buildCrate bmc.crates.${crate.def} { };
      runtimeDeps = deps.compositorRuntimeDeps armv7Pkgs;
    };
  }));

  # Individual widget packages per arch/profile
  widgetPackages = builtins.listToAttrs (lib.concatLists (lib.forEach glibcArchProfiles ({ arch, profile }:
    lib.mapAttrsToList
      (name: def: {
        name = "widget-${name}-${arch}-${profile}";
        value = bmc.lib.mkWidgetPackage {
          inherit name;
          inherit (def) crate;
          runtimeDeps = deps.widgetRuntimeDeps.${def.runtimeDepsKind};
          features = def.features or [ ];
          profile = bmc.profiles."${arch}-${profile}";
        };
      })
      widgets
  )));

  # Combined widget packages per arch/profile: native widgets joined
  # with wasm widgets so both land under lib/bmc-widgets/<name>/.
  combinedWidgetPackages = builtins.listToAttrs (lib.forEach glibcArchProfiles ({ arch, profile }: {
    name = "widgets-${arch}-${profile}";
    value = pkgs.symlinkJoin {
      name = "bmc-widgets-${arch}-${profile}";
      paths = [
        (bmc.lib.mkAllWidgets {
          inherit widgets;
          runtimeDeps = deps.widgetRuntimeDeps.native;
          profile = bmc.profiles."${arch}-${profile}";
        })
        (mkAllWasmWidgets {
          profile = bmc.profiles."${arch}-${profile}";
          # Mirror mkOpenwrt: profiling on for non-release builds.
          hostFeatures = lib.optionals (profile != "release") [ "profiling" ];
        })
      ];
    };
  }));

  armv7MuslDeps = bmc.profiles.armv7-musl-release.deps.overrideAttrs (old: {
    # Cargo reports "cannot produce cdylib" for these fixtures:
    # armv7-unknown-linux-musleabihf does not support that crate type.
    # TODO(BDK-734): let nix-lib accept dependency-build Cargo flags instead of rewriting this phase.
    buildPhase =
      let
        rewritten = builtins.replaceStrings
          [ " --all-targets" ]
          [ " --workspace --all-targets --exclude bmc-wasm-assets-linker-widget --exclude bmc-wasm-assets-macro-widget" ]
          old.buildPhase;
      in
      assert pkgs.lib.assertMsg (rewritten != old.buildPhase)
        "armv7MuslDeps: nix-lib build phase no longer contains --all-targets";
      rewritten;
  });

  specialPackages = {
    gallery-deps = bmc.profiles.gallery.deps;
    workspace-deps = bmc.profiles.fast.deps;
    workspace-deps-wasm-widgets = bmc.profiles.wasm-widgets-debug.deps;
    inherit (bmc.profiles.ci) build clippy test nextest;
    workspace-deps-armv7 = bmc.profiles.armv7-glibc-release.deps;
    workspace-deps-armv7-musl = armv7MuslDeps;
    nextest-armv7 = bmc.profiles.armv7-glibc-release.nextest;
  } // (
    # bmc-openwrt + bmc-virt helper packages for x86_64 and aarch64.
    # bmc-openwrt needs autopatchelf for compositor runtime deps (dlopen'd).
    let
      archConfigs = {
        x86_64 = { runtimePkgs = x86Pkgs; profiles = [ "release" "debug" "rr" ]; };
        aarch64 = { runtimePkgs = aarch64Pkgs; profiles = [ "release" "debug" ]; };
      };
      mkOpenwrt = arch: profile: cfg: {
        name = "bmc-openwrt-${arch}-${profile}";
        value = bmc.lib.autopatchelfBinaries {
          drv = bmc.profiles."${arch}-${profile}".buildCrate bmc.crates.bmc-openwrt {
            features = lib.optionals (profile != "release") [ "profiling" ];
          };
          runtimeDeps = deps.compositorRuntimeDeps cfg.runtimePkgs;
        };
      };
      mkVirtCrate = arch: profile: crate: {
        name = "${crate}-${arch}-${profile}";
        value = bmc.profiles."${arch}-${profile}".buildCrate bmc.crates.${crate} { };
      };
    in
    builtins.listToAttrs (lib.concatLists (lib.mapAttrsToList
      (arch: cfg:
        (map (p: mkOpenwrt arch p cfg) cfg.profiles)
          ++ [
          (mkVirtCrate arch "debug" "bmc-virt-leds")
          {
            name = "bmc-virt-relay-${arch}-debug";
            value = bmc.lib.autopatchelfBinaries {
              drv = bmc.profiles."${arch}-debug".buildCrate bmc.crates.bmc-virt-relay { };
              runtimeDeps = [ cfg.runtimePkgs.wayland ];
            };
          }
          # Combined widget package for the VM: native widgets joined
          # with wasm widgets, cross-compiled for the guest arch.
          {
            name = "widgets-${arch}";
            value =
              let
                wasmWidgetPackage = mkAllWasmWidgets {
                  profile = bmc.profiles."${arch}-debug";
                  # VM is always debug → mirror mkOpenwrt and turn profiling on.
                  hostFeatures = [ "profiling" ];
                };
              in
              pkgs.symlinkJoin {
                name = "bmc-widgets-${arch}";
                paths = [
                  (bmc.lib.mkAllWidgets {
                    inherit widgets;
                    runtimeDeps = deps.widgetRuntimeDeps.native;
                    profile = bmc.profiles."${arch}-debug";
                  })
                  wasmWidgetPackage
                ];
                passthru.host = wasmWidgetPackage.host;
              };
          }
        ]
      )
      archConfigs))
  );

  armv7lPkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
  bmc-video-play-armv7 = armv7lPkgs.callPackage ./bmc-video/package.nix { };

  # Native individual widget packages (for bmc-mock)
  nativeWidgetPackages = builtins.listToAttrs (lib.mapAttrsToList
    (name: widget: {
      name = "widget-${name}";
      value = bmc.lib.mkWidgetPackage {
        inherit name;
        inherit (widget) crate;
        features = widget.features or [ ];
        profile = bmc.profiles.fast;
      };
    })
    widgets);

  # Native hooks package (hooks run on build host during init tarball build).
  # Build bmc-nix once and symlink each hook binary out of it.
  nativeBmcNix = bmc.profiles.fast.buildCrate bmc.crates.bmc-nix { };
  nativeWasmAssets = bmc.profiles.fast.buildCrate bmc.crates.wasm-assets { };
  selectNativeBmcNixBin = bmc.lib.selectBmcNixBin { inherit pkgs; bmcNix = nativeBmcNix; };
  nativeHooksPackage = bmc.lib.mkPackage {
    name = "native-hooks";
    hooks = [
      { prefix = "001"; bin = selectNativeBmcNixBin "bmc-hook-merge-files"; }
      { prefix = "002"; bin = selectNativeBmcNixBin "bmc-hook-file-symlinks"; }
      { prefix = "099"; bin = selectNativeBmcNixBin "bmc-hook-activation-resolver"; }
    ];
  };

  # wasm-widgets.nix is parametric in the host profile: re-import it per
  # profile to cross-compile the bmc-wasm-host for every consumer
  # arch (armv7 deck + x86_64/aarch64 VM).
  wasmWidgetsFor = profile: hostFeatures: import ./nix/wasm-widgets.nix {
    inherit pkgs profile hostFeatures;
    wasmAssets = nativeWasmAssets;
    widgetCatalog = wasmWidgetCatalog;
    wasmReleaseProfiles = {
      wasmExamples = bmc.profiles.wasm-examples-release;
      wasmWidgets = bmc.profiles.wasm-widgets-release;
    };
    crates = bmc.crates;
    autopatchelfBinaries = bmc.lib.autopatchelfBinaries;
    inherit (deps) widgetRuntimeDeps;
  };

  wasmWidgetsModule = wasmWidgetsFor bmc.profiles.armv7-glibc-release [ ];

  # Build every shippable wasm widget (has manifest.json) from `profile`, joined
  # into a single lib/bmc-widgets/<name>/ tree (same shape as mkAllWidgets).
  # Regression-only crates without a manifest are excluded.
  mkAllWasmWidgets = { profile, hostFeatures ? [ ] }:
    let
      m = wasmWidgetsFor profile hostFeatures;
      shippable = lib.filterAttrs (_: w: w.isShippable) wasmWidgetCatalog;
      widgetPackages = lib.mapAttrsToList
        (name: entry: m.mkWasmWidget {
          inherit name;
          wrapperMode = "baked";
          inherit (entry) wasmFile manifest;
          wasmDir = m.wasmWidgets.${name};
          thin = m.thin;
        })
        shippable;
    in
    pkgs.symlinkJoin {
      name = "bmc-wasm-widgets";
      paths = widgetPackages;
      passthru = {
        inherit (m) host;
        wrapperModes = map (package: package.wrapperMode) widgetPackages;
        representativeWrapper = builtins.head widgetPackages;
      };
    };

  bakedWasmWidgetsForTest = mkAllWasmWidgets {
    profile = bmc.profiles.armv7-glibc-release;
  };

  armv7PackageDefs = import ./nix/packages.nix {
    inherit bmc armv7Pkgs deps;
    inherit wasmWidgetCatalog;
    profile = bmc.profiles.armv7-glibc-release;
    openwrtFeatures = [ ];
    inherit (wasmWidgetsModule) wasmWidgets thin host wasmLauncher mkWasmWidget;
  };

  # Same package set, built debug with the `profiling` feature on the
  # compositor and the wasm host — exposed as `deck-packages-debug` so a
  # `deck deploy --profile debug` surfaces the mesh::profile timing/memory
  # channel on the device. Package names are identical to the release set;
  # only the build profile differs.
  wasmWidgetsModuleDebug = wasmWidgetsFor bmc.profiles.armv7-glibc-debug [ "profiling" ];
  armv7PackageDefsDebug = import ./nix/packages.nix {
    inherit bmc armv7Pkgs deps;
    inherit wasmWidgetCatalog;
    profile = bmc.profiles.armv7-glibc-debug;
    openwrtFeatures = [ "profiling" ];
    inherit (wasmWidgetsModuleDebug) wasmWidgets thin host wasmLauncher mkWasmWidget;
  };

  initArtifacts = import ./nix/init-artifacts.nix {
    inherit self pkgs lib;
    inherit (bmc.lib) mkIndex mkTarball mkPackageFeed;
    packages = armv7PackageDefs;
    bmc-nix-cli = bmc.profiles.fast.buildCrate bmc.crates.bmc-nix-cli { };
    hooksOverridePath = "${nativeHooksPackage}/hooks";
  };

in
{
  inherit commonDeps bmc deps makeRustflagsEnv wasmWidgetCatalog wasmStackSize;
  wasmWrapperTestPackages = {
    baked = bakedWasmWidgetsForTest.representativeWrapper;
    bakedModes = bakedWasmWidgetsForTest.wrapperModes;
    host = bakedWasmWidgetsForTest.host;
    inherit (wasmWidgetsModule) mkWasmLauncher;
  };
  inherit (initArtifacts) mkInitArtifacts;
  inherit (wasmWidgetsModule) wasmExamples wasmWidgetsBundle wasmWidgets;
  checks = frontend.checks;
  # Nested attrset of cross-built deck packages.
  #
  # Lives in `legacyPackages` because `packages.<system>` rejects nested attrsets
  # in the flake schema, while `legacyPackages.<system>` permits them.
  #
  # Scripts and docs reference `.#deck-packages.<name>` which Nix resolves
  # through the `legacyPackages` chain.
  legacyPackages = {
    deck-packages = armv7PackageDefs;
    deck-packages-debug = armv7PackageDefsDebug;
  };
  packages = cratePackages // widgetPackages // combinedWidgetPackages // nativeWidgetPackages // specialPackages // initArtifacts.packages // {
    inherit bmc-video-play-armv7;
    bmc-mock = bmc.profiles.fast.buildCrate bmc.crates.bmc-mock { };
    bmc-nix-cli = bmc.profiles.fast.buildCrate bmc.crates.bmc-nix-cli { };
    bmc-nix-cli-armv7-release =
      let package = bmc.profiles.armv7-musl-release.buildCrate bmc.crates.bmc-nix-cli { };
      in package.overrideAttrs (old: {
        cargoArtifacts = old.cargoArtifacts.overrideAttrs (_: {
          cargoArtifacts = armv7MuslDeps;
        });
      });
    gallery-build = bmc.profiles.gallery.build;
    gallery-clippy = bmc.profiles.gallery.clippy;
    gallery-test = bmc.profiles.gallery.test;

    # Native widgets joined with wasm widgets, so bmc-mock sees the full catalog
    # under lib/bmc-widgets/<name>/.
    # Use with bmc-mock --widgets-path ./result/lib/bmc-widgets.
    widgets = pkgs.symlinkJoin {
      name = "bmc-widgets-native";
      paths = [
        (bmc.lib.mkAllWidgets {
          inherit widgets;
          runtimeDeps = deps.widgetRuntimeDeps.native;
          profile = bmc.profiles.fast;
        })
        (mkAllWasmWidgets { profile = bmc.profiles.fast; })
      ];
    };

    frontend = frontend.build;
    yarnFiles = frontend.yarnFiles;
    inherit sounds;
  };
  devShells =
    let
      # ARM glibc RUSTFLAGS for dev shells only — patchelf handles builds.
      # Use all runtime deps (compositor is superset of widget deps).
      armv7GlibcShellEnv = makeRustflagsEnv {
        runtimePackages = deps.compositorRuntimeDeps armv7Pkgs;
        rustCrossTarget = "armv7-unknown-linux-gnueabihf";
      };
      shells = pkgs.ii.lib.mapAttrValues (profile: profile.shell) bmc.profiles;
    in
    shells
    // lib.mapAttrs'
      (name: shell: {
        name = name;
        value = shell.overrideAttrs (prev: { env = (prev.env or { }) // armv7GlibcShellEnv; });
      })
      (lib.filterAttrs (name: _: lib.hasPrefix "armv7-glibc" name) shells);
}
