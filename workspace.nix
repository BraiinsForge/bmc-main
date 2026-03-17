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
  aarch64Pkgs = pkgs.pkgsCross.aarch64-multiplatform.extend (final: prev:
    # Same cross-splicing guard as armv7Pkgs — libinput and the
    # compositorUdev marker only belong on the cross-target side.
    lib.optionalAttrs (prev.stdenv.hostPlatform != prev.stdenv.buildPlatform) (
      (mesaOverlay { galliumDrivers = vmGalliumDrivers; }) final prev
      // applianceOverlay final prev
    ));

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
      FONTCONFIG_FILE = pkgs.writeText "fonts.conf" ''
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
        <fontconfig>
          <dir>${pkgs.corefonts}</dir>
        </fontconfig>
      '';

      LD_LIBRARY_PATH = "${lib.makeLibraryPath [
        pkgs.libgcc
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
      seatd
      (compositorUdev pkgs)
      libdrm
      mesa
      alsa-lib
    ];

    # Node.js tooling for frontend builds
    frontendDeps = pkgs: with pkgs; [ nodejs yarn ];
  };

  # Minimal workspace config for musl profiles (bmc-openwrt, statically linked)
  workspaceMinimal = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    nativeDeps = _pkgs: (commonDeps.buildDeps _pkgs);
    # Workspace-deps step compiles ALL Cargo.lock entries, including
    # crates from glibc-only binaries (compositor, widgets). Provide
    # the system libraries their build.rs scripts need via pkg-config.
    # libinput is excluded — it refuses to build for static platforms
    # and its crate uses dlopen at runtime, not link-time pkg-config.
    targetDeps = pkgs: with pkgs; [
      wayland
      libxkbcommon
      seatd
      (compositorUdev pkgs)
      libdrm
    ];
    env = {
      FONTCONFIG_FILE = commonDeps.env.FONTCONFIG_FILE;
    };
    includeExtraSrc = [
      "assets"
    ];
  };

  # Full workspace config for glibc profiles (widgets, compositor)
  workspace = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    nativeDeps = _pkgs: (commonDeps.buildDeps _pkgs) ++ (commonDeps.guiBuildDeps _pkgs);
    # packages that will be cross-compiled for target arch
    targetDeps = commonDeps.guiTargetDeps;
    env = commonDeps.env;
    includeExtraSrc = [
      "assets"
    ];
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
  # .wasm blobs. Apply to every wasm workspace — currently only
  # workspaceWasmExamples consumes it, but any new wasm target should
  # reuse this so the remap isn't silently lost.
  wasmRemapFlags = mkStorePathRemapFlag "${pkgs.ii.rust.toolchain}";

  workspaceWasmExamples = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    workspacePath = "bmc-wasm-runtime/examples";
    nativeDeps = _pkgs: (commonDeps.buildDeps _pkgs);
    env = {
      CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS = wasmRemapFlags;
    };
  };

  bmc = {
    armv7-pkgs = armv7Pkgs;
    lib = import ./nix/lib.nix { inherit pkgs lib armv7Pkgs; };
    crates = import ./nix/crates.nix { inherit (pkgs.ii.rust) defineCrate; };
    workspaces = {
      full = workspace;
      minimal = workspaceMinimal;
      wasmExamples = workspaceWasmExamples;
    };
    profiles = import ./nix/profiles.nix {
      inherit (bmc) workspaces;
      inherit pkgs armv7Pkgs x86Pkgs aarch64Pkgs;
    };
  };

  # Runtime deps for dlopen'd libraries, split by widget type.
  # Functions accepting pkgs — resolved at the point of use.
  deps = {
    widgetRuntimeDeps = {
      # Slint-based widgets: winit backend dlopen's wayland + xkbcommon.
      slint = pkgs: with pkgs; [
        wayland
        libxkbcommon
      ];
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
    # Compositor: native widget deps + compositor-specific input/seat libs.
    compositorRuntimeDeps = pkgs:
      (deps.widgetRuntimeDeps.native pkgs) ++ (with pkgs; [
        libinput
        seatd
        (compositorUdev pkgs)
      ]);
  };

  # All widget definitions for building
  widgets = {
    digital-clock = {
      crate = bmc.crates.widget-digital-clock;
      features = [ "standalone" ];
      runtimeDepsKind = "slint";
    };
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

  specialPackages = {
    workspace-deps = bmc.profiles.fast.deps;
    inherit (bmc.profiles.fast) build clippy test nextest;
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
            value = pkgs.symlinkJoin {
              name = "bmc-widgets-${arch}";
              paths = [
                (bmc.lib.mkAllWidgets {
                  inherit widgets;
                  runtimeDeps = deps.widgetRuntimeDeps.native;
                  profile = bmc.profiles."${arch}-debug";
                })
                (mkAllWasmWidgets {
                  profile = bmc.profiles."${arch}-debug";
                  # VM is always debug → mirror mkOpenwrt and turn profiling on.
                  hostFeatures = [ "profiling" ];
                })
              ];
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

  # Native hooks package (hooks run on build host during init tarball build)
  nativeHooksPackage = bmc.lib.mkPackage {
    name = "native-hooks";
    hooks = [
      { prefix = "001"; bin = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-merge-files { }; }
      { prefix = "002"; bin = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-file-symlinks { }; }
      { prefix = "099"; bin = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-activation-resolver { }; }
    ];
  };

  # wasm-widgets.nix is parametric in the host profile: re-import it per
  # profile to cross-compile the bmc-widget-wasm host for every consumer
  # arch (armv7 deck + x86_64/aarch64 VM).
  wasmWidgetsFor = profile: hostFeatures: import ./nix/wasm-widgets.nix {
    inherit pkgs profile hostFeatures;
    wasmReleaseProfile = bmc.profiles.wasm-release;
    crates = bmc.crates;
    autopatchelfBinaries = bmc.lib.autopatchelfBinaries;
    inherit (deps) widgetRuntimeDeps;
  };

  wasmWidgetsModule = wasmWidgetsFor bmc.profiles.armv7-glibc-release [ ];

  # Shared wasm widget catalog: name → { wasmFile, manifest }. Mirrors
  # the per-widget entries in nix/packages.nix; both consumers (deck
  # release tarball and combined widgets tree below) iterate it.
  wasmWidgets = {
    hello-widget = {
      wasmFile = "hello_widget.wasm";
      manifest = ./bmc-wasm-runtime/examples/hello-widget/manifest.json;
    };
    calendar = {
      wasmFile = "calendar.wasm";
      manifest = ./bmc-wasm-runtime/examples/calendar/manifest.json;
    };
    spacex-launch = {
      wasmFile = "spacex_launch.wasm";
      manifest = ./bmc-wasm-runtime/examples/spacex-launch/manifest.json;
    };
    iss-position = {
      wasmFile = "iss_position.wasm";
      manifest = ./bmc-wasm-runtime/examples/iss-position/manifest.json;
    };
    home-assistant = {
      wasmFile = "home_assistant.wasm";
      manifest = ./bmc-wasm-runtime/examples/home-assistant/manifest.json;
    };
    media-control = {
      wasmFile = "media_control.wasm";
      manifest = ./bmc-wasm-runtime/examples/media-control/manifest.json;
    };
    mesh-demo = {
      wasmFile = "mesh_demo.wasm";
      manifest = ./bmc-wasm-runtime/examples/mesh-demo/manifest.json;
    };
    metronome = {
      wasmFile = "metronome.wasm";
      manifest = ./bmc-wasm-runtime/examples/metronome/manifest.json;
    };
    pomodoro = {
      wasmFile = "pomodoro.wasm";
      manifest = ./bmc-wasm-runtime/examples/pomodoro/manifest.json;
    };
  };

  # Build all wasm widgets against `profile`'s host binary, joined into
  # a single lib/bmc-widgets/<name>/ tree (same shape as mkAllWidgets).
  mkAllWasmWidgets = { profile, hostFeatures ? [ ] }:
    let m = wasmWidgetsFor profile hostFeatures; in
    pkgs.symlinkJoin {
      name = "bmc-wasm-widgets";
      paths = lib.mapAttrsToList
        (name: w: m.mkWasmWidget {
          inherit name;
          inherit (w) wasmFile manifest;
          wasmDir = m.wasmExamples;
          host = m.host;
        })
        wasmWidgets;
    };

  armv7PackageDefs = import ./nix/packages.nix {
    inherit bmc armv7Pkgs deps;
    inherit (wasmWidgetsModule) wasmExamples host mkWasmWidget;
  };

  initArtifacts = import ./nix/init-artifacts.nix {
    inherit self pkgs lib;
    inherit (bmc.lib) mkIndex mkTarball mkFactoryIndex;
    packages = armv7PackageDefs;
    bmc-nix-cli = bmc.profiles.fast.buildCrate bmc.crates.bmc-nix-cli { };
    hooksOverridePath = "${nativeHooksPackage}/hooks";
  };

in
{
  inherit commonDeps bmc deps makeRustflagsEnv;
  inherit (wasmWidgetsModule) wasmExamples;
  packages = cratePackages // widgetPackages // combinedWidgetPackages // nativeWidgetPackages // specialPackages // initArtifacts // {
    deck-packages = armv7PackageDefs;
    armv7-nixpkgs = armv7Pkgs;
    inherit bmc-video-play-armv7;
    bmc-mock = bmc.profiles.fast.buildCrate bmc.crates.bmc-mock { };
    bmc-nix-init-mock = bmc.profiles.fast.buildCrate bmc.crates.bmc-nix-init-mock { };
    bmc-nix-init-armv7-release =
      bmc.profiles.armv7-musl-release.buildCrate bmc.crates.bmc-nix-init-openwrt { };
    bmc-nix-cli = bmc.profiles.fast.buildCrate bmc.crates.bmc-nix-cli { };
    bmc-hook-merge-files = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-merge-files { };
    bmc-hook-file-symlinks = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-file-symlinks { };
    bmc-hook-activation-resolver = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-activation-resolver { };
    bmc-activation-copy-files-armv7-glibc-release = bmc.profiles.armv7-glibc-release.buildCrate bmc.crates.bmc-activation-copy-files { };

    # Native widgets combined - use with bmc-mock --widgets-path ./result/lib/bmc-widgets
    widgets = bmc.lib.mkAllWidgets { inherit widgets; profile = bmc.profiles.fast; };
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
