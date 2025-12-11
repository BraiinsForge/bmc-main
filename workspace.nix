# Workspace config for Rust builds. Receives commonDeps from flake.nix
# to share dependency definitions with devShells.
{ self, pkgs, commonDeps }:
let lib = pkgs.lib; in
let
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

  workspace = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    # packages that can be executed during compilation (from commonDeps)
    nativeDeps = _pkgs: commonDeps.buildDeps;
    # packages that will be cross-compiled for target arch
    targetDeps = _build_pkgs: [
      # openssl.dev
    ];
    # environment variables (from commonDeps)
    env = commonDeps.env // { };
  };

  build-profiles = with workspace; {
    # fast profile (no cross compilation, non-portable binaries)
    fast = mkBuildProfile {
      minimal_deps = false;
      rustProfile = "fast";
    };
    armv7-release = mkBuildProfile {
      suffix = "armv7";
      minimal_deps = true;
      rustProfile = "release";
      rustCrossTarget = "armv7-unknown-linux-musleabihf";
      build_pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
    };
    armv7-debug = mkBuildProfile {
      suffix = "armv7";
      minimal_deps = false;
      rustProfile = "dev";
      rustCrossTarget = "armv7-unknown-linux-musleabihf";
      build_pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
    };
  };

  # use each profile to build each crate
  allTuples = lib.cartesianProduct
    ({
      # NOTE: Update README.md when changing these sets!
      arch = [
        "armv7"
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

  # Build all widgets and combine into a single output
  allWidgets = pkgs.symlinkJoin {
    name = "bmc-widgets";
    paths = lib.mapAttrsToList
      (name: widget:
        mkWidgetPackage {
          inherit name;
          inherit (widget) crate;
          features = widget.features or [ ];
          wrapWithLibs = widget.wrapWithLibs or false;
          profile = build-profiles.fast;
        }
      )
      widgets;
  };

in
{
  packages = packages // specialPackages // {
    inherit bmc-video-play-armv7;
    bmc-mock = build-profiles.fast.buildCrate crates.bmc-mock { };

    # Individual widget packages
    widget-digital-clock = mkWidgetPackage {
      name = "digital-clock";
      crate = crates.widget-digital-clock;
      features = [ "standalone" ];
      wrapWithLibs = true;
      profile = build-profiles.fast;
    };

    # All widgets combined - use with bmc-mock --widgets-path ./result/lib/bmc-widgets
    widgets = allWidgets;
  };
  devShells = pkgs.ii.lib.mapAttrValues (profile: profile.shell) build-profiles;
}
