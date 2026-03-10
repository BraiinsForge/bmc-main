# profiles: Build profile definitions for all target platforms.
{ workspaces, pkgs, fixedArmv7Pkgs }:
{
  # fast profile (no cross compilation, non-portable binaries)
  fast = workspaces.full.mkBuildProfile {
    minimal_deps = false;
    rustProfile = "fast";
  };
  # musl profiles for bmc-openwrt (statically linked)
  armv7-release = workspaces.minimal.mkBuildProfile {
    suffix = "armv7";
    minimal_deps = true;
    rustProfile = "release";
    rustCrossTarget = "armv7-unknown-linux-musleabihf";
    build_pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
  };
  armv7-debug = workspaces.minimal.mkBuildProfile {
    suffix = "armv7";
    minimal_deps = false;
    rustProfile = "dev";
    rustCrossTarget = "armv7-unknown-linux-musleabihf";
    build_pkgs = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic;
  };
  # glibc profiles for widgets/compositor (dynamically linked)
  armv7-glibc-release = workspaces.full.mkBuildProfile {
    suffix = "armv7";
    minimal_deps = true;
    rustProfile = "release";
    rustCrossTarget = "armv7-unknown-linux-gnueabihf";
    build_pkgs = fixedArmv7Pkgs;
    wrapNixGL = true;
  };
  armv7-glibc-debug = workspaces.full.mkBuildProfile {
    suffix = "armv7";
    minimal_deps = false;
    rustProfile = "dev";
    rustCrossTarget = "armv7-unknown-linux-gnueabihf";
    build_pkgs = fixedArmv7Pkgs;
    wrapNixGL = true;
  };
}
