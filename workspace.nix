# Workspace config for Rust builds. Defines commonDeps as single source
# of truth for dependency definitions shared with devShells.
{ self, pkgs }:
let lib = pkgs.lib; in
let
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
    widget-digital-clock = defineCrate {
      path = "./widgets/digital-clock";
      packageName = "bmc-widget-digital-clock";
    };
    widget-flip-clock = defineCrate {
      path = "./widgets/flip-clock";
      packageName = "bmc-widget-flip-clock";
    };
  };

  X11RuntimeDeps = pkgs: with pkgs; [
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    xorg.libXinerama
    xorg.libXext
    xorg.libXft
    xorg.libXrender
    xorg.libxcb
    vulkan-loader
    libGL
  ];

  waylandRuntimeDeps = pkgs: with pkgs; [
    wayland
    libxkbcommon
    vulkan-loader
    libGL
  ];

  allRuntimeDeps = pkgs: ((X11RuntimeDeps pkgs) ++ (waylandRuntimeDeps pkgs));

  # Add rpath to produced binaries
  makeRpathLinkArgument = { packages }:
    "-C link-args=-Wl,-rpath,${lib.makeLibraryPath packages}";

  # Create RUSTFLAGS for runtime dlopen of libraries in 'runtimePackages'
  makeRustflagsEnv = { runtimePackages, rustCrossTarget }:
    let
      target = lib.toUpper (builtins.replaceStrings [ "-" ] [ "_" ] rustCrossTarget);
      value = makeRpathLinkArgument { packages = runtimePackages; };
    in
    {
      "CARGO_TARGET_${target}_RUSTFLAGS" = value;
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

  # Build a widget package with the correct directory structure
  mkWidgetPackage = { name, crate, profile, features ? [ ], runtimeDeps ? waylandRuntimeDeps }:
    let
      rustCrossTarget =
        if profile ? rustCrossTarget
        then profile.rustCrossTarget
        else pkgs.stdenv.hostPlatform.rust.rustcTarget;
      runtimePackages =
        if builtins.isFunction runtimeDeps
        then runtimeDeps (profile.build_pkgs or pkgs)
        else runtimeDeps;
      binary = profile.buildCrate crate {
        inherit features;
        env = makeRustflagsEnv { inherit runtimePackages rustCrossTarget; };
      };
      widgetSrc = ./widgets + "/${name}";
    in
    pkgs.runCommand "bmc-widget-${name}" { } ''
      mkdir -p $out/lib/bmc-widgets/${name}/bin
      cp ${widgetSrc}/manifest.json $out/lib/bmc-widgets/${name}/
      cp ${binary}/bin/* $out/lib/bmc-widgets/${name}/bin/
      if [ -d "${widgetSrc}/assets" ]; then
        cp -r ${widgetSrc}/assets $out/lib/bmc-widgets/${name}/
      fi
    '';

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
    };
    armv7-glibc-debug = workspace.mkBuildProfile {
      suffix = "armv7";
      minimal_deps = false;
      rustProfile = "dev";
      rustCrossTarget = "armv7-unknown-linux-gnueabihf";
      build_pkgs = fixedArmv7Pkgs;
    };
  };

  # use each profile to build each crate
  allTuples = lib.cartesianProduct
    ({
      # NOTE: Update README.md when changing these sets!
      arch = [
        "armv7-glibc"
      ];
      profile = [
        "release"
        "debug"
      ];
      crate = [
        { def = "bmc-openwrt"; }
      ];
    });

  packages = builtins.listToAttrs (lib.forEach allTuples ({ arch, profile, crate }: {
    name = "${crate.def}-${arch}-${profile}";
    value = build-profiles."${arch}-${profile}".buildCrate crates.${crate.def} { };
  }));

  specialPackages = {
    workspace-deps = build-profiles.fast.deps;
    inherit (build-profiles.fast) build clippy test nextest;
  };

  armv7lPkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
  bmc-video-play-armv7 = armv7lPkgs.callPackage ./bmc-video/package.nix { };

  # All widget definitions for building
  widgets = {
    digital-clock = {
      crate = crates.widget-digital-clock;
      features = [ "standalone" ];
      runtimeDeps = waylandRuntimeDeps;
    };
    flip-clock = {
      crate = crates.widget-flip-clock;
      features = [ "standalone" ];
      runtimeDeps = waylandRuntimeDeps;
    };
  };

  # Build all widgets for a given profile and combine into a single output
  mkAllWidgets = { profile, runtimeDeps ? waylandRuntimeDeps }: pkgs.symlinkJoin {
    name = "bmc-widgets";
    paths = lib.mapAttrsToList
      (name: widget:
        mkWidgetPackage {
          inherit name profile;
          inherit (widget) crate;
          features = widget.features or [ ];
          runtimeDeps = widget.runtimeDeps or runtimeDeps;
        }
      )
      widgets;
  };

  # x86 widgets (for bmc-mock) - need library wrapper for Nix environment
  allWidgets = mkAllWidgets { profile = build-profiles.fast; runtimeDeps = allRuntimeDeps; };

  # ARM widgets (glibc, dynamically linked) - compatible with system Wayland libs
  allWidgetsArmv7Release = mkAllWidgets { profile = build-profiles.armv7-glibc-release; };
  allWidgetsArmv7Debug = mkAllWidgets { profile = build-profiles.armv7-glibc-debug; };

in
{
  inherit commonDeps;
  packages = packages // specialPackages // {
    inherit bmc-video-play-armv7;
    bmc-mock = build-profiles.fast.buildCrate crates.bmc-mock { };

    # Individual widget packages (x86)
    widget-digital-clock = mkWidgetPackage {
      name = "digital-clock";
      crate = crates.widget-digital-clock;
      features = [ "standalone" ];
      runtimeDeps = allRuntimeDeps;
      profile = build-profiles.fast;
    };
    widget-flip-clock = mkWidgetPackage {
      name = "flip-clock";
      crate = crates.widget-flip-clock;
      features = [ "standalone" ];
      runtimeDeps = allRuntimeDeps;
      profile = build-profiles.fast;
    };

    # All widgets combined - use with bmc-mock --widgets-path ./result/lib/bmc-widgets
    widgets = allWidgets;

    # ARM widget packages
    widgets-armv7-release = allWidgetsArmv7Release;
    widgets-armv7-debug = allWidgetsArmv7Debug;
  };
  devShells = pkgs.ii.lib.mapAttrValues (profile: profile.shell) build-profiles;
}
