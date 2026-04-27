# profiles: Build profile definitions for all target platforms.
{ workspaces, pkgs, armv7Pkgs, x86Pkgs, aarch64Pkgs }:
let
  # On x86_64 native, libgbm is a separate package (not part of mesa).
  x86NativeTargetDeps = pkgs: with pkgs; [ libgbm ];

  mkX86 = attrs: workspaces.full.mkBuildProfile ({
    targetDeps = x86NativeTargetDeps;
    pkgs = x86Pkgs;
  } // attrs);

  mkAarch64 = attrs: workspaces.full.mkBuildProfile ({
    pkgs = aarch64Pkgs;
  } // attrs);
in
{
  # fast profile (no cross compilation, non-portable native binaries)
  fast = workspaces.full.mkBuildProfile {
    inherit pkgs;
    targetDeps = x86NativeTargetDeps;
    minimalDeps = false;
    rustProfile = "fast";
    # Activate features for bins gated by `required-features` so they actually
    # get compiled/linted instead of being silently skipped by cargo.
    #
    # NOTE: ideally this would be `allFeatures = true;` (catches future bins
    # automatically), but `bmc-mock-display`'s `winit-skia` feature pulls in
    # `skia-bindings`, whose build script downloads the skia source from
    # github at compile time. The nix sandbox has no network access, so any
    # build that activates `winit-skia` fails. Until that feature is moved
    # out of the workspace's cargo feature graph (separate crate excluded
    # from `[workspace.members]`, or replaced with a `--cfg` rustflag), we
    # have to enumerate the features we want and accept that new gated bins
    # will need an addition here.
    features = [
      "bmc-display/slint-embed-files"
      "bmc-wasm-runtime/testbed"
      "bmc-wasm-runtime/capture"
    ];
    nativeDeps = pkgs: with pkgs; [
      # bmc-nix activation entrypoint shells out to `flock(1)`; BusyBox
      # provides it on-device, but the sandboxed nextest build needs an
      # explicit util-linux.
      util-linux
    ];
  };
  # musl profiles for statically linked binaries (bmc-nix-init-openwrt)
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
  # glibc profiles for bmc-virt (x86_64, dynamically linked)
  x86_64-release = mkX86 { minimalDeps = true; rustProfile = "release"; };
  x86_64-debug = mkX86 { minimalDeps = false; rustProfile = "dev"; };
  x86_64-rr = mkX86 { minimalDeps = false; rustProfile = "rr"; };
  # glibc profiles for aarch64 (dynamically linked)
  aarch64-release = mkAarch64 { minimalDeps = true; rustProfile = "release"; };
  aarch64-debug = mkAarch64 { minimalDeps = false; rustProfile = "dev"; };
}
