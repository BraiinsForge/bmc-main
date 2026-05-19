# profiles: Build profile definitions for all target platforms.
{ rustflags, workspaces, pkgs, armv7Pkgs, x86Pkgs, aarch64Pkgs, ciPkgs }:
let
  # `fast` builds against upstream `pkgs` (no mesaOverlay),
  # so libgbm comes in as a separate package.
  # bmc-mock fails to link without it.
  #
  # The `ci` and `x86_64-*` profiles use our custom Mesa (which bundles
  # libgbm internally), so they don't go through this function.
  x86NativeTargetDeps = pkgs: with pkgs; [ libgbm ];

  mkX86 = attrs: workspaces.full.mkBuildProfile ({
    targetDeps = x86NativeTargetDeps;
    pkgs = x86Pkgs;
  } // attrs);

  mkAarch64 = attrs: workspaces.full.mkBuildProfile ({
    pkgs = aarch64Pkgs;
  } // attrs);

  # Each wasm widget workspace gets its own pair of release/debug profiles.
  # `mkBuildProfile` is workspace-bound (it pins the cargo Cargo.toml path),
  # so we instantiate the same shape per workspace rather than parameterizing
  # at build time.
  mkWasmRelease = workspace: workspace.mkBuildProfile {
    minimalDeps = true;
    rustProfile = "release";
    rustCrossTargetOverride = "wasm32-unknown-unknown";
    inherit pkgs;
  };
  mkWasmDebug = workspace: workspace.mkBuildProfile {
    minimalDeps = true;
    rustProfile = "dev";
    rustCrossTargetOverride = "wasm32-unknown-unknown";
    inherit pkgs;
  };
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
    allFeatures = true;
    nativeDeps = pkgs: with pkgs; [
      # bmc-nix activation entrypoint shells out to `flock(1)`; BusyBox
      # provides it on-device, but the sandboxed nextest build needs an
      # explicit util-linux.
      util-linux
    ];
  };
  # CI profile: `fast` plus a working headless EGL stack
  # so some tests can boot a real surfaceless context inside the Nix sandbox.
  # Used by the `test` and `nextest` checks.
  # The local `validate` loop sticks with `fast` to avoid the Mesa rebuild.
  ci = workspaces.full.mkBuildProfile {
    pkgs = ciPkgs;
    targetDeps = pkgs: with pkgs; [ mesa ];
    minimalDeps = false;
    rustProfile = "fast";
    allFeatures = true;
    nativeDeps = pkgs: with pkgs; [ util-linux ];
    env = {
      # Force Mesa onto its software path.
      #
      # With no DRM/Wayland/X11 in the sandbox, surfaceless is
      # the only EGL platform that can produce a display.
      #
      # llvmpipe is a gallium software driver that runs without a GPU.
      # softpipe also works but is markedly slower; llvmpipe is the default for CI.
      #
      # LIBGL_ALWAYS_SOFTWARE keeps Mesa from short-circuiting back
      # to a hardware path it can't actually use.
      EGL_PLATFORM = "surfaceless";
      MESA_LOADER_DRIVER_OVERRIDE = "llvmpipe";
      LIBGL_ALWAYS_SOFTWARE = "1";
    } // rustflags.makeRustflagsEnv {
      # Libraries needed for compositor tests.
      runtimePackages = with ciPkgs; [
        mesa
        wayland
        libxkbcommon
        libdrm
        libinput
        libGL
      ];
      rustCrossTarget = ciPkgs.stdenv.hostPlatform.rust.rustcTarget;
    };
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
  # Per-workspace wasm profiles. `wasm-widgets.nix` selects between them
  # based on each widget's workspace tag in the catalog.
  wasm-examples-release = mkWasmRelease workspaces.wasmExamples;
  wasm-examples-debug = mkWasmDebug workspaces.wasmExamples;
  wasm-widgets-release = mkWasmRelease workspaces.wasmWidgets;
  wasm-widgets-debug = mkWasmDebug workspaces.wasmWidgets;
  # glibc profiles for bmc-virt (x86_64, dynamically linked)
  x86_64-release = mkX86 { minimalDeps = true; rustProfile = "release"; };
  x86_64-debug = mkX86 { minimalDeps = false; rustProfile = "dev"; };
  x86_64-rr = mkX86 { minimalDeps = false; rustProfile = "rr"; };
  # glibc profiles for aarch64 (dynamically linked)
  aarch64-release = mkAarch64 { minimalDeps = true; rustProfile = "release"; };
  aarch64-debug = mkAarch64 { minimalDeps = false; rustProfile = "dev"; };
}
