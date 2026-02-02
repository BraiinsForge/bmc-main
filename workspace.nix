{ self, pkgs }:
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
  };

  workspace = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    # packages that can be executed during compilation
    nativeDeps = pkgs: with pkgs; [
      protobuf
      diffutils
    ];
    # packages that will be cross-compiled for target arch
    targetDeps = build_pkgs: with build_pkgs; [
      # openssl.dev
    ];
    env = {
      FONTCONFIG_FILE = pkgs.makeFontsConf {
        fontDirectories = [ pkgs.corefonts ];
      };
    };
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

in
{
  packages = packages // specialPackages // {
    inherit bmc-video-play-armv7;
    bmc-mock = build-profiles.fast.buildCrate crates.bmc-mock { };
  };
  devShells = pkgs.ii.lib.mapAttrValues (profile: profile.shell) build-profiles;
}
