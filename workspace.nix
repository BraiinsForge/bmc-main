# Workspace config for Rust builds. Receives commonDeps from flake.nix
# to share dependency definitions with devShells.
{ self, pkgs, commonDeps }:
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
  };

  # Build a widget package with the correct directory structure
  mkWidgetPackage = { name, crate, profile, features ? [ ], wrapWithLibs ? false }:
    let
      binary = profile.buildCrate crate { inherit features; };
      widgetSrc = ./widgets + "/${name}";
      runtimeLibs = with pkgs; [
        wayland
        libxkbcommon
        xorg.libX11
        xorg.libXcursor
        xorg.libXi
        xorg.libXrandr
        vulkan-loader
        libGL
      ];
    in
    pkgs.runCommand "bmc-widget-${name}"
      {
        nativeBuildInputs = lib.optionals wrapWithLibs [ pkgs.makeWrapper ];
      } ''
      mkdir -p $out/lib/bmc-widgets/${name}/bin
      cp ${widgetSrc}/manifest.json $out/lib/bmc-widgets/${name}/
      if ${if wrapWithLibs then "true" else "false"}; then
        for bin in ${binary}/bin/*; do
          makeWrapper "$bin" "$out/lib/bmc-widgets/${name}/bin/$(basename $bin)" \
            --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
        done
      else
        cp ${binary}/bin/* $out/lib/bmc-widgets/${name}/bin/
      fi
      if [ -d "${widgetSrc}/assets" ]; then
        cp -r ${widgetSrc}/assets $out/lib/bmc-widgets/${name}/
      fi
    '';

  # Minimal workspace config for musl profiles (bmc-openwrt, statically linked)
  workspaceMinimal = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    # packages that can be executed during compilation (from commonDeps)
    nativeDeps = _pkgs: commonDeps.buildDeps;
    # minimal deps for static linking
    targetDeps = _build_pkgs: [
      # openssl.dev
    ];
    env = {
      FONTCONFIG_FILE = pkgs.makeFontsConf {
        fontDirectories = [ pkgs.corefonts ];
      };
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
    # environment variables (from commonDeps, with fontconfig in LD_LIBRARY_PATH
    # for Slint build script which dlopen's libfontconfig.so.1 during compilation)
    env = commonDeps.env // {
      LD_LIBRARY_PATH = lib.makeLibraryPath [
        pkgs.libgcc
        pkgs.fontconfig
      ];
    };
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
      wrapWithLibs = true;
    };
  };

  # Build all widgets for a given profile and combine into a single output
  # wrapLibs: whether to wrap binaries with LD_LIBRARY_PATH (only for x86, not cross-compiled)
  mkAllWidgets = { profile, wrapLibs ? false }: pkgs.symlinkJoin {
    name = "bmc-widgets";
    paths = lib.mapAttrsToList
      (name: widget:
        mkWidgetPackage {
          inherit name profile;
          inherit (widget) crate;
          features = widget.features or [ ];
          # Only wrap with libs if requested AND the widget wants it
          wrapWithLibs = wrapLibs && (widget.wrapWithLibs or false);
        }
      )
      widgets;
  };

  # x86 widgets (for bmc-mock) - need library wrapper for Nix environment
  allWidgets = mkAllWidgets { profile = build-profiles.fast; wrapLibs = true; };

  # ARM widgets (glibc, dynamically linked) - compatible with system Wayland libs
  allWidgetsArmv7Release = mkAllWidgets { profile = build-profiles.armv7-glibc-release; };
  allWidgetsArmv7Debug = mkAllWidgets { profile = build-profiles.armv7-glibc-debug; };

in
{
  packages = packages // specialPackages // {
    inherit bmc-video-play-armv7;
    bmc-mock = build-profiles.fast.buildCrate crates.bmc-mock { };

    # Individual widget packages (x86)
    widget-digital-clock = mkWidgetPackage {
      name = "digital-clock";
      crate = crates.widget-digital-clock;
      features = [ "standalone" ];
      wrapWithLibs = true;
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
