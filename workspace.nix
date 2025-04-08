{ self, pkgs }:
let lib = pkgs.lib; in
let
  crates = with pkgs.ii.rust; {
    bmc = defineCrate {
      path = "./bmc";
      packageName = "bmc";
    };
  };

  workspace = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    # packages that can be executed during compilation
    nativeDeps = pkgs: with pkgs; [
      protobuf
    ];
    # packages that will be cross-compiled for target arch
    targetDeps = build_pkgs: with build_pkgs; [
      # openssl.dev
    ];
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

  allCrates = {
    crate = [
      { def = "bmc"; }
    ];
  };

  # use each profile to build each crate
  allTuples = lib.cartesianProduct
    ({
      platform = [
        "openwrt"
      ];
      # NOTE: Update README.md when changing these sets!
      arch = [
        "armv7"
      ];
      profile = [
        "release"
        "debug"
      ];
    } // allCrates);

  packages = builtins.listToAttrs (lib.forEach allTuples ({ platform, arch, profile, crate }: {
    name = "${crate.def}-${platform}-${arch}-${profile}";
    value = build-profiles."${arch}-${profile}".buildCrate crates.${crate.def} {
      noDefaultFeatures = true;
      features = [ "${crate.def}/${platform}" ];
    };
  }));

  fastPackages = builtins.listToAttrs (lib.forEach (lib.cartesianProduct allCrates) ({ crate }: {
    name = "${crate.def}";
    value = build-profiles.fast.buildCrate crates.${crate.def} { };
  }));

  specialPackages = {
    workspace-deps = build-profiles.fast.deps;
    inherit (build-profiles.fast) build clippy test nextest;
  };

in
{
  packages = packages // fastPackages // specialPackages;
  devShells = pkgs.ii.lib.mapAttrValues (profile: profile.shell) build-profiles;
}
