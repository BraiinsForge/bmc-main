# Nix package for the wasm-capture tool — visual regression testing orchestrator.
#
# Source-filtered to avoid recompilation when unrelated workspace crates change.
# Wrapped with runtime deps (odiff, ffmpeg, mesa llvmpipe) for headless CI use.
{ self, pkgs, commonDeps, profiles, wasmExamples }:
let
  lib = pkgs.lib;
  inherit (pkgs) ii;

  crate = ii.rust.defineCrate {
    path = "./bmc-wasm-runtime";
    packageName = "bmc-wasm-runtime";
    binName = "capture";
  };

  unwrapped = profiles.ci.buildCrate crate {
    features = [ "capture" ];
  };

  # Shell wrapper that bundles runtime dependencies and forces software
  # rendering via Mesa llvmpipe (CI runners have no GPU).
  wrapped = pkgs.writeShellApplication {
    name = "wasm-capture";
    runtimeInputs = [
      unwrapped
      pkgs.odiff
      pkgs.ffmpeg-headless
    ];
    runtimeEnv = {
      # Force Mesa software rasterizer (llvmpipe) so EGL device enumeration
      # exposes a software device on headless CI runners without a GPU.
      LIBGL_ALWAYS_SOFTWARE = "1";
      # Tell libglvnd where Mesa's EGL ICD is — without this, device
      # enumeration (EGL_EXT_device_query) is not available.
      __EGL_VENDOR_LIBRARY_FILENAMES = "${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json";
      # Use surfaceless EGL platform so eglGetDisplay works without X11/Wayland.
      EGL_PLATFORM = "surfaceless";
      # Fontconfig in nix sandbox can't find system fonts — point it at the
      # nix store path of corefonts so text rendering is deterministic.
      FONTCONFIG_FILE = commonDeps.env.FONTCONFIG_FILE;
      # Mesa/libGL are dlopened at runtime — make them discoverable.
      LD_LIBRARY_PATH = lib.makeLibraryPath [
        pkgs.mesa
        pkgs.libGL
      ];
    };
    text = ''
      # Auto-detect flags not explicitly provided by the caller.
      # --wasm-dir is intentionally NOT auto-injected — callers pass it
      # explicitly so the wrapper derivation stays decoupled from the
      # workspace-wide wasmExamples build.
      has_widgets_dir=false
      has_output_dir=false
      for arg in "$@"; do
        case "$arg" in
          --widgets-dir | --widgets-dir=*) has_widgets_dir=true ;;
          --output-dir | --output-dir=*)   has_output_dir=true ;;
        esac
      done

      extra_args=()

      # Resolve widgets-dir: try repo root first, then bmc-wasm-runtime/.
      if [ "$has_widgets_dir" = false ]; then
        if [ -d "bmc-wasm-runtime/examples" ]; then
          extra_args+=(--widgets-dir=bmc-wasm-runtime/examples)
        fi
      fi

      # Resolve output-dir: mirror the widgets-dir prefix.
      if [ "$has_output_dir" = false ]; then
        if [ -d "bmc-wasm-runtime" ]; then
          extra_args+=(--output-dir=bmc-wasm-runtime/captures)
        fi
      fi

      exec capture "$@" "''${extra_args[@]}"
    '';
  };
in
{
  package = wrapped;
  inherit unwrapped wasmExamples;
}
