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

# WASM widget packaging primitives.
#
# Reusable pieces only — widget entries are derived from `wasmWidgetCatalog`
# in workspace.nix (filesystem-scanned) and surfaced under
# `.#deck-packages.<widget>` via nix/packages.nix.
#
# Exports:
#   - wasmExamples: flattened *.wasm from the SDK examples workspace.
#                   Single workspace cargo build, so its cache key is the full
#                   examples src — capture/regression tooling consumes this
#                   when it wants every example .wasm at once.
#                   Production widgets in `widgets-wasm/` get per-widget
#                   derivations via `wasmWidgets` only.
#   - wasmWidgets:  per-widget wasm derivations keyed by widget name; each
#                   is built via buildCrate, so change scope stays at the
#                   individual widget's wasm derivation.
#   - thin:         cross-compiled bmc-wasm-thin + autopatchelf for the
#                   selected profile (typically armv7-glibc-release)
#   - host:         cross-compiled bmc-wasm-host + autopatchelf for the
#                   selected profile (typically armv7-glibc-release)
#   - wasmLauncher: per-SDK-major launcher that pins thin and host together
#   - mkWasmWidget: build one lib/bmc-widgets/<name>/ tree (shell wrapper
#                   + .wasm blob + manifest) that execs the thin wrapper
{ pkgs
, profile                # bmc.profiles.<arch>-<profile> for the host build
, wasmReleaseProfiles    # attrset { wasmExamples = profile; wasmWidgets = profile; ... }
, crates                 # bmc.crates
, autopatchelfBinaries   # bmc.lib.autopatchelfBinaries
, widgetRuntimeDeps      # deps.widgetRuntimeDeps (expects .native fn)
, widgetCatalog          # workspace.nix:wasmWidgetCatalog (name → entry)
, hostFeatures ? [ ]     # cargo features for the bmc-wasm-host build
}:
let
  lib = pkgs.lib;
  inherit (import ./public-asset.nix { inherit lib; }) mkPublicIcon;
  parseSdkMajor = import ./sdk-major.nix { inherit lib; };
  sdkMajor = parseSdkMajor (builtins.readFile ../bmc-wasm-runtime/protocol/src/version.rs);
  launcherName = "bmc-wasm-thin-v${toString sdkMajor}";

  # Reference-cleaning installPhase shared by the examples-bundle and per-widget
  # wasm builds. rustc bakes panic-location strings that point into
  # $cargoVendorDir (e.g. bytes/chrono source files). The compile-time
  # toolchain remap scrubs the toolchain path, but not the vendor one —
  # blank its hash post-install so $out carries zero nix references, and
  # enforce that with allowedReferences.
  wasmInstallOverrides = old: {
    nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ pkgs.removeReferencesTo ];
    installPhase = ''
      mkdir -p $out
      find target -name '*.wasm' | while IFS= read -r wasm; do
        remove-references-to -t "$cargoVendorDir" "$wasm"
        cp "$wasm" $out/
      done
    '';
    allowedReferences = [ ];
  };

  # All SDK examples in one *.wasm-blob tree.
  # Single workspace cargo build, cache key is the full examples src.
  # Used by `capture.nix` to expose the `wasm-examples` flake package.
  # Not used for per-widget pipelines — touching anything in the examples workspace invalidates it.
  wasmExamples = wasmReleaseProfiles.wasmExamples.build.overrideAttrs wasmInstallOverrides;

  # All production widgets (widgets-wasm/) in one *.wasm-blob tree.
  # Mirror of `wasmExamples` for the production-widget workspace.
  # Exposed as the `wasm-widgets` flake package;
  # consumed by bmc-virt to populate the guest overlay's WASM_DIR alongside `wasm-examples`.
  wasmWidgetsBundle = wasmReleaseProfiles.wasmWidgets.build.overrideAttrs wasmInstallOverrides;

  # Per-widget wasm derivation.
  # Picks the release profile that owns the widget's workspace,
  # then builds only that widget's crate.
  mkWidgetWasm = name:
    let
      entry = widgetCatalog.${name};
      releaseProfile = wasmReleaseProfiles.${entry.workspaceName};
    in
    (releaseProfile.buildCrate crates."wasm-widget-${name}" { }).overrideAttrs wasmInstallOverrides;

  wasmWidgets = lib.mapAttrs (name: _: mkWidgetWasm name) widgetCatalog;

  # Per-widget thin wrapper binary. One build, any number of widgets
  # exec it. autopatchelfBinaries (not mkWidgetPackage) — the thin has
  # no manifest of its own. The binary ends up at $out/bin/bmc-wasm-thin.
  thin = autopatchelfBinaries {
    drv = profile.buildCrate crates.wasm-thin { };
    runtimeDeps = widgetRuntimeDeps.native profile.pkgs;
  };

  # Shared host binary (one build, any number of widgets use it).
  # autopatchelfBinaries (not mkWidgetPackage) — the host has no manifest
  # of its own. The binary ends up at $out/bin/bmc-wasm-host.
  host = autopatchelfBinaries {
    drv = profile.buildCrate crates.wasm-host {
      features = hostFeatures;
    };
    # widgetRuntimeDeps.native is a function; call with the profile's
    # target pkgs (armv7Pkgs for glibc arm profiles) — same convention
    # mkWidgetPackage uses internally.
    runtimeDeps = widgetRuntimeDeps.native profile.pkgs;
  };

  wasmLauncher = (pkgs.writeTextFile {
    name = launcherName;
    executable = true;
    destination = "/bin/${launcherName}";
    text = ''
      #!/bin/sh
      exec ${thin}/bin/bmc-wasm-thin --host-bin ${host}/bin/bmc-wasm-host "$@"
    '';
  }).overrideAttrs (old: {
    passthru = (old.passthru or { }) // {
      inherit sdkMajor launcherName thin host;
    };
  });

  # Per-widget packaging. Produces:
  #   $out/lib/bmc-widgets/<name>/bin/<name>          (shell wrapper)
  #   $out/lib/bmc-widgets/<name>/lib/wasm/<name>.wasm
  #   $out/lib/bmc-widgets/<name>/manifest.json
  #
  # Device-profile wrappers select the active per-major launcher. Other
  # bundles remain self-contained by baking the thin and host store paths.
  mkWasmWidget =
    { name          # e.g. "hello-widget"
    , wasmDir       # derivation with all *.wasm files flat in $out/
    , wasmFile      # e.g. "hello_widget.wasm" (cargo: hyphens → underscores)
    , manifest      # path to per-widget manifest.json
    , thin          # thin derivation with bin/bmc-wasm-thin
    , host          # host derivation with bin/bmc-wasm-host
    , wrapperMode
    }:
    let
      # The .wasm embeds its runtime assets; only the manifest icon needs
      # installing so the BMC /widgets/{uid}/icon endpoint can serve it.
      # mkPublicIcon owns resolution and validation, so the installed copy
      # can't accept an icon the published index copy rejects.
      icon = (builtins.fromJSON (builtins.readFile manifest)).icon or null;
      iconSrc = if icon == null then null else mkPublicIcon manifest icon;
      wrapperExec =
        if wrapperMode == "profile" then ''
          exec /run/current-profile/bin/${launcherName} \\
            --wasm $out/lib/bmc-widgets/${name}/lib/wasm/${name}.wasm \\
            "\$@"
        '' else ''
          exec ${thin}/bin/bmc-wasm-thin \\
            --wasm $out/lib/bmc-widgets/${name}/lib/wasm/${name}.wasm \\
            --host-bin ${host}/bin/bmc-wasm-host \\
            "\$@"
        '';
    in
    assert lib.assertMsg (builtins.elem wrapperMode [ "profile" "baked" ])
      "wrapperMode must be either profile or baked";
    (pkgs.runCommand "bmc-widget-${name}" { } ''
      base=$out/lib/bmc-widgets/${name}
      mkdir -p "$base/bin" "$base/lib/wasm"

      cp ${wasmDir}/${wasmFile} "$base/lib/wasm/${name}.wasm"
      cp ${manifest}            "$base/manifest.json"
      ${lib.optionalString (icon != null) ''
        mkdir -p "$(dirname "$base/${icon}")"
        cp ${iconSrc} "$base/${icon}"
      ''}

      # Unquoted heredoc expands $out to the packaged wasm path. The escaped
      # runtime argument vector survives into the generated script.
      cat > "$base/bin/${name}" <<EOF
      #!/bin/sh
      ${wrapperExec}
      EOF
      chmod +x "$base/bin/${name}"
    '').overrideAttrs (old: {
      passthru = (old.passthru or { }) // {
        inherit name wrapperMode;
      };
    });
in
{
  inherit sdkMajor launcherName wasmExamples wasmWidgetsBundle wasmWidgets thin host wasmLauncher mkWasmWidget;
}
