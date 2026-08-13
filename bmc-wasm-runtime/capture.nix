# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

# Nix package for the wasm-capture tool — visual regression testing orchestrator.
#
# Source-filtered to avoid recompilation when unrelated workspace crates change.
# Wrapped with runtime deps (odiff, ffmpeg, mesa llvmpipe) for headless CI use.
{ pkgs, commonDeps, profiles, wasmExamples, wasmWidgetsBundle, wasmStackSize }:
let
  lib = pkgs.lib;
  inherit (pkgs) ii;

  crate = ii.rust.defineCrate {
    path = "bmc-wasm-runtime";
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
      FONTCONFIG_FILE = pkgs.writeText "fonts.conf" ''
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
        <fontconfig>
          <dir>${pkgs.corefonts}</dir>
        </fontconfig>
      '';

      # Mesa/libGL are dlopened at runtime — make them discoverable.
      LD_LIBRARY_PATH = lib.makeLibraryPath [
        pkgs.mesa
        pkgs.libGL
      ];
    };
    text = ''
      exec capture "$@"
    '';
  };

  stackUsageReport = pkgs.runCommand "wasm-stack-usage-report"
    {
      nativeBuildInputs = [ wrapped ];
      src = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.unions [
          ./examples
          ../widgets-wasm
        ];
      };
    } ''
    mkdir captures "$out"
    wasm-capture verify \
      --workspace="$src/bmc-wasm-runtime/examples" \
      --wasm-dir="${wasmExamples}" \
      --workspace="$src/widgets-wasm" \
      --wasm-dir="${wasmWidgetsBundle}" \
      --output-dir=captures \
      --parallel \
      --stack-profile=${toString wasmStackSize} > "$out/stack-usage.md"
  '';
in
{
  package = wrapped;
  inherit unwrapped wasmExamples wasmWidgetsBundle stackUsageReport;
}
