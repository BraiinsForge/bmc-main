# profiles: Build profile definitions for all target platforms.
{ workspaces, pkgs, armv7Pkgs }:
{
  # fast profile (no cross compilation, non-portable binaries)
  fast = workspaces.full.mkBuildProfile {
    minimalDeps = false;
    rustProfile = "fast";
    targetDeps = pkgs: with pkgs; [
      # NOTE: for native compilation, mesa does not have
      # gbm, while for armv7 libgbm is kept in mesa.
      libgbm
    ];
    inherit pkgs;
  };
  # musl profiles for bmc-openwrt (statically linked)
  armv7-musl-release = workspaces.minimal.mkBuildProfile {
    minimalDeps = true;
    rustProfile = "release";
    pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
  };
  armv7-musl-debug = workspaces.minimal.mkBuildProfile {
    minimalDeps = false;
    rustProfile = "dev";
    pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
  };
  # glibc profiles for widgets/compositor (dynamically linked)
  armv7-glibc-release = workspaces.full.mkBuildProfile {
    minimalDeps = true;
    rustProfile = "release";
    pkgs = armv7Pkgs;
  };
  armv7-glibc-debug = workspaces.full.mkBuildProfile {
    minimalDeps = false;
    rustProfile = "dev";
    pkgs = armv7Pkgs;
  };
  wasm-release = workspaces.wasmExamples.mkBuildProfile {
    minimalDeps = true;
    rustProfile = "release";
    rustCrossTargetOverride = "wasm32-unknown-unknown";
    inherit pkgs;
  };
  wasm-debug = workspaces.wasmExamples.mkBuildProfile {
    minimalDeps = true;
    rustProfile = "dev";
    rustCrossTargetOverride = "wasm32-unknown-unknown";
    inherit pkgs;
  };
}
