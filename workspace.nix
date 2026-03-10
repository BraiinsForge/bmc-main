# Workspace config for Rust builds. Defines commonDeps as single source
# of truth for dependency definitions shared with devShells.
{ self, pkgs }:
let lib = pkgs.lib; in
let
  rustflags = import ./nix/rustflags.nix { inherit lib; };
  inherit (rustflags) X11RuntimeDeps waylandRuntimeDeps makeRustflagsEnv;

  mkIndex = import ./nix/mkIndex.nix { inherit pkgs lib; };
  mkTarball = import ./nix/mkTarball.nix { inherit pkgs lib mkIndex; };

  # Fix for linux-pam cross-compilation issue in nixpkgs-unstable
  # The man output fails to build for ARMv7 glibc targets
  fixedArmv7Pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.extend (final: prev: {
    linux-pam = prev.linux-pam.overrideAttrs (old: {
      outputs = lib.filter (o: o != "man") (old.outputs or [ "out" ]);
    });
  });

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
    } // makeRustflagsEnv {
      runtimePackages = waylandRuntimeDeps fixedArmv7Pkgs;
      rustCrossTarget = "armv7-unknown-linux-gnueabihf";
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

  bmc = {
    lib = import ./nix/lib.nix { inherit pkgs lib; };
    crates = import ./nix/crates.nix { inherit (pkgs.ii.rust) defineCrate; };
    workspaces = {
      full = workspace;
      minimal = workspaceMinimal;
    };
    profiles = import ./nix/profiles.nix {
      inherit (bmc) workspaces;
      inherit pkgs fixedArmv7Pkgs;
    };
  };

  # All widget definitions for building
  widgets = {
    digital-clock = {
      crate = bmc.crates.widget-digital-clock;
      features = [ "standalone" ];
    };
    flip-clock = {
      crate = bmc.crates.widget-flip-clock;
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
    value = bmc.profiles."${archProfile.arch}-${archProfile.profile}".buildCrate bmc.crates.${crate.def} { };
  }));

  # Individual widget packages per arch/profile
  widgetTuples = lib.cartesianProduct {
    archProfile = glibcArchProfiles;
    widget = lib.mapAttrsToList (name: def: { inherit name; } // def) widgets;
  };

  widgetPackages = builtins.listToAttrs (lib.forEach widgetTuples ({ archProfile, widget }: {
    name = "widget-${widget.name}-${archProfile.arch}-${archProfile.profile}";
    value = bmc.lib.mkWidgetPackage {
      inherit (widget) name crate;
      features = widget.features or [ ];
      profile = bmc.profiles."${archProfile.arch}-${archProfile.profile}";
    };
  }));

  # Combined widget packages per arch/profile
  combinedWidgetPackages = builtins.listToAttrs (lib.forEach glibcArchProfiles ({ arch, profile }: {
    name = "widgets-${arch}-${profile}";
    value = bmc.lib.mkAllWidgets {
      inherit widgets;
      profile = bmc.profiles."${arch}-${profile}";
    };
  }));

  specialPackages = {
    workspace-deps = bmc.profiles.fast.deps;
    inherit (bmc.profiles.fast) build clippy test nextest;
  };

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

  # Native activation package (hooks run on build host during init tarball build)
  nativeCorePackage = bmc.lib.mkCorePackage {
    bmc-hook-merge-files = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-merge-files { };
    bmc-hook-file-symlinks = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-file-symlinks { };
    bmc-hook-activation-resolver = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-activation-resolver { };
  };

  armv7PackageDefs = import ./nix/packages.nix { inherit bmc fixedArmv7Pkgs; };

  initArtifacts = import ./nix/init-artifacts.nix {
    inherit self pkgs lib mkIndex mkTarball;
    packages = armv7PackageDefs;
    bmc-nix-cli = bmc.profiles.fast.buildCrate bmc.crates.bmc-nix-cli { };
    hooksOverridePath = "${nativeCorePackage}/hooks";
  };

in
{
  inherit commonDeps bmc;
  packages = cratePackages // widgetPackages // combinedWidgetPackages // nativeWidgetPackages // specialPackages // initArtifacts // {
    inherit bmc-video-play-armv7;
    bmc-mock = bmc.profiles.fast.buildCrate bmc.crates.bmc-mock { };
    bmc-nix-cli = bmc.profiles.fast.buildCrate bmc.crates.bmc-nix-cli { };
    bmc-hook-merge-files = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-merge-files { };
    bmc-hook-file-symlinks = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-file-symlinks { };
    bmc-hook-activation-resolver = bmc.profiles.fast.buildCrate bmc.crates.bmc-hook-activation-resolver { };

    # Native widgets combined - use with bmc-mock --widgets-path ./result/lib/bmc-widgets
    widgets = bmc.lib.mkAllWidgets { inherit widgets; profile = bmc.profiles.fast; };
  };
  devShells = pkgs.ii.lib.mapAttrValues (profile: profile.shell) bmc.profiles;
}
