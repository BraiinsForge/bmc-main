# Workspace config for Rust builds. Defines commonDeps as single source
# of truth for dependency definitions shared with devShells.
{ self, pkgs }:
let lib = pkgs.lib; in
let
  rustflags = import ./nix/rustflags.nix { inherit lib; };
  inherit (rustflags) X11RuntimeDeps waylandRuntimeDeps makeRustflagsEnv;

  widgetLib = import ./nix/widget.nix {
    inherit pkgs lib;
  };
  inherit (widgetLib) mkWidgetPackage mkAllWidgets;

  mkIndex = import ./nix/mkIndex.nix { inherit pkgs lib; };
  mkTarball = import ./nix/mkTarball.nix { inherit pkgs lib mkIndex; };
  mkCorePackage = import ./nix/pkgs/core/package.nix { inherit pkgs lib; };

  # Fix for linux-pam cross-compilation issue in nixpkgs-unstable
  # The man output fails to build for ARMv7 glibc targets
  fixedArmv7Pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.extend (final: prev: {
    linux-pam = prev.linux-pam.overrideAttrs (old: {
      outputs = lib.filter (o: o != "man") (old.outputs or [ "out" ]);
    });
  });

  crates = with pkgs.ii.rust; {
    bmc-mock = defineCrate {
      path = "./bmc-mock";
      packageName = "bmc-mock";
    };
    bmc-openwrt = defineCrate {
      path = "./bmc-openwrt";
      packageName = "bmc-openwrt";
    };
    bmc-nix-cli = defineCrate {
      path = "./bmc-nix";
      packageName = "bmc-nix";
      binName = "bmc-nix-cli";
    };
    bmc-hook-merge-files = defineCrate {
      path = "./bmc-nix";
      packageName = "bmc-nix";
      binName = "bmc-hook-merge-files";
    };
    bmc-hook-file-symlinks = defineCrate {
      path = "./bmc-nix";
      packageName = "bmc-nix";
      binName = "bmc-hook-file-symlinks";
    };
    bmc-hook-activation-resolver = defineCrate {
      path = "./bmc-nix";
      packageName = "bmc-nix";
      binName = "bmc-hook-activation-resolver";
    };
    widget-digital-clock = defineCrate {
      path = "./widgets/digital-clock";
      packageName = "bmc-widget-digital-clock";
    };
    widget-flip-clock = defineCrate {
      path = "./widgets/flip-clock";
      packageName = "bmc-widget-flip-clock";
    };
  };

  # Shared deps used by both package builds and devShells.
  # Single source of truth to keep build derivations and dev environments in sync.
  commonDeps = {
    # Rust build-time deps (protoc for protobufs, diffutils for cargo)
    buildDeps = with pkgs; [ protobuf diffutils pkg-config ];

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
    } // makeRustflagsEnv {
      runtimePackages = commonDeps.guiRuntimeDeps;
      rustCrossTarget = pkgs.stdenv.hostPlatform.rust.rustcTarget;
    };

    guiBuildDeps = with pkgs; [
      fontconfig
      freetype
    ];

    # Runtime libs for GUI/display development (Slint, winit backends)
    guiRuntimeDeps = with pkgs; (waylandRuntimeDeps pkgs) ++ (X11RuntimeDeps pkgs) ++ [
      libxkbcommon
      mesa
    ];

    # Node.js tooling for frontend builds
    frontendDeps = with pkgs; [ nodejs yarn ];
  };

  # Minimal workspace config for musl profiles (bmc-openwrt, statically linked)
  workspaceMinimal = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    nativeDeps = _pkgs: commonDeps.buildDeps;
    # minimal deps for static linking
    targetDeps = _build_pkgs: [
      # openssl.dev
    ];
    env = {
      FONTCONFIG_FILE = commonDeps.env.FONTCONFIG_FILE;
    } // makeRustflagsEnv {
      runtimePackages = waylandRuntimeDeps fixedArmv7Pkgs;
      rustCrossTarget = "armv7-unknown-linux-gnueabihf";
    };
  };

  # Full workspace config for glibc profiles (widgets, compositor)
  workspace = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    nativeDeps = _pkgs: commonDeps.buildDeps ++ commonDeps.guiBuildDeps;
    # packages that will be cross-compiled for target arch
    targetDeps = build_pkgs: with build_pkgs; [
      # Compositor dependencies (require dynamic linking)
      wayland
      libxkbcommon
      libinput
      seatd
      udev
      libdrm
      mesa
      libgbm
      libGL
    ];
    env = commonDeps.env;
  };

  build-profiles = {
    # fast profile (no cross compilation, non-portable binaries)
    fast = workspace.mkBuildProfile {
      minimal_deps = false;
      rustProfile = "fast";
    };
    # musl profiles for bmc-openwrt (statically linked)
    armv7-release = workspaceMinimal.mkBuildProfile {
      suffix = "armv7";
      minimal_deps = true;
      rustProfile = "release";
      rustCrossTarget = "armv7-unknown-linux-musleabihf";
      build_pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
    };
    armv7-debug = workspaceMinimal.mkBuildProfile {
      suffix = "armv7";
      minimal_deps = false;
      rustProfile = "dev";
      rustCrossTarget = "armv7-unknown-linux-musleabihf";
      build_pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
    };
    # glibc profiles for widgets/compositor (dynamically linked)
    armv7-glibc-release = workspace.mkBuildProfile {
      suffix = "armv7";
      minimal_deps = true;
      rustProfile = "release";
      rustCrossTarget = "armv7-unknown-linux-gnueabihf";
      build_pkgs = fixedArmv7Pkgs;
      wrapNixGL = true;
    };
    armv7-glibc-debug = workspace.mkBuildProfile {
      suffix = "armv7";
      minimal_deps = false;
      rustProfile = "dev";
      rustCrossTarget = "armv7-unknown-linux-gnueabihf";
      build_pkgs = fixedArmv7Pkgs;
      wrapNixGL = true;
    };
  };

  # All widget definitions for building
  widgets = {
    digital-clock = {
      crate = crates.widget-digital-clock;
      features = [ "standalone" ];
    };
    flip-clock = {
      crate = crates.widget-flip-clock;
      features = [ "standalone" ];
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
    value = build-profiles."${archProfile.arch}-${archProfile.profile}".buildCrate crates.${crate.def} { };
  }));

  # Individual widget packages per arch/profile
  widgetTuples = lib.cartesianProduct {
    archProfile = glibcArchProfiles;
    widget = lib.mapAttrsToList (name: def: { inherit name; } // def) widgets;
  };

  widgetPackages = builtins.listToAttrs (lib.forEach widgetTuples ({ archProfile, widget }: {
    name = "widget-${widget.name}-${archProfile.arch}-${archProfile.profile}";
    value = mkWidgetPackage {
      inherit (widget) name crate;
      features = widget.features or [ ];
      profile = build-profiles."${archProfile.arch}-${archProfile.profile}";
    };
  }));

  # Combined widget packages per arch/profile
  combinedWidgetPackages = builtins.listToAttrs (lib.forEach glibcArchProfiles ({ arch, profile }: {
    name = "widgets-${arch}-${profile}";
    value = mkAllWidgets {
      inherit widgets;
      profile = build-profiles."${arch}-${profile}";
    };
  }));

  specialPackages = {
    workspace-deps = build-profiles.fast.deps;
    inherit (build-profiles.fast) build clippy test nextest;
  };

  armv7lPkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
  bmc-video-play-armv7 = armv7lPkgs.callPackage ./bmc-video/package.nix { };

  # Native individual widget packages (for bmc-mock)
  nativeWidgetPackages = builtins.listToAttrs (lib.mapAttrsToList
    (name: widget: {
      name = "widget-${name}";
      value = mkWidgetPackage {
        inherit name;
        inherit (widget) crate;
        features = widget.features or [ ];
        profile = build-profiles.fast;
      };
    })
    widgets);

  # Native activation package (hooks run on build host during init tarball build)
  nativeCorePackage = mkCorePackage {
    bmc-hook-merge-files = build-profiles.fast.buildCrate crates.bmc-hook-merge-files { };
    bmc-hook-file-symlinks = build-profiles.fast.buildCrate crates.bmc-hook-file-symlinks { };
    bmc-hook-activation-resolver = build-profiles.fast.buildCrate crates.bmc-hook-activation-resolver { };
  };

  # ARM packages for the init tarball
  armv7Packages = {
    nix = fixedArmv7Pkgs.nix;
    core = mkCorePackage {
      bmc-openwrt = build-profiles.armv7-glibc-release.buildCrate crates.bmc-openwrt { };
      bmc-hook-merge-files = build-profiles.armv7-glibc-release.buildCrate crates.bmc-hook-merge-files { };
      bmc-hook-file-symlinks = build-profiles.armv7-glibc-release.buildCrate crates.bmc-hook-file-symlinks { };
      bmc-hook-activation-resolver = build-profiles.armv7-glibc-release.buildCrate crates.bmc-hook-activation-resolver { };
    };
    digital-clock = mkWidgetPackage {
      name = "digital-clock";
      crate = crates.widget-digital-clock;
      profile = build-profiles.armv7-glibc-release;
      features = [ "standalone" ];
    };
    flip-clock = mkWidgetPackage {
      name = "flip-clock";
      crate = crates.widget-flip-clock;
      profile = build-profiles.armv7-glibc-release;
      features = [ "standalone" ];
    };
  };

  armv7PackageDefs = import ./nix/packages.nix { inherit armv7Packages; };

  initArtifacts = import ./nix/init-artifacts.nix {
    inherit self pkgs lib mkIndex mkTarball;
    packages = armv7PackageDefs;
    bmc-nix-cli = build-profiles.fast.buildCrate crates.bmc-nix-cli { };
    hooksOverridePath = "${nativeCorePackage}/hooks";
  };

in
{
  inherit commonDeps build-profiles crates;
  packages = cratePackages // widgetPackages // combinedWidgetPackages // nativeWidgetPackages // specialPackages // initArtifacts // {
    inherit bmc-video-play-armv7;
    bmc-mock = build-profiles.fast.buildCrate crates.bmc-mock { };
    bmc-nix-cli = build-profiles.fast.buildCrate crates.bmc-nix-cli { };
    bmc-hook-merge-files = build-profiles.fast.buildCrate crates.bmc-hook-merge-files { };
    bmc-hook-file-symlinks = build-profiles.fast.buildCrate crates.bmc-hook-file-symlinks { };
    bmc-hook-activation-resolver = build-profiles.fast.buildCrate crates.bmc-hook-activation-resolver { };

    # Native widgets combined - use with bmc-mock --widgets-path ./result/lib/bmc-widgets
    widgets = mkAllWidgets { inherit widgets; profile = build-profiles.fast; };
  };
  devShells = pkgs.ii.lib.mapAttrValues (profile: profile.shell) build-profiles;
}
