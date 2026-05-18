# WASM widget packaging primitives.
#
# Reusable pieces only — widget entries live in nix/packages.nix so they
# surface under .#deck-packages.<widget>, alongside digital-clock and
# flip-clock.
#
# Exports:
#   - wasmExamples: flattened *.wasm from a wasm-release workspace build
#   - wasmWidgets:  per-widget wasm derivations keyed by widget name; each
#                   is built via buildCrate so its src closure is narrowed
#                   by docker-spider — change one widget, only that
#                   widget's wasm rebuilds.
#   - thin:         cross-compiled bmc-wasm-thin + autopatchelf for the
#                   selected profile (typically armv7-glibc-release)
#   - host:         cross-compiled bmc-wasm-host + autopatchelf for the
#                   selected profile (typically armv7-glibc-release)
#   - mkWasmWidget: build one lib/bmc-widgets/<name>/ tree (shell wrapper
#                   + .wasm blob + manifest) that execs the thin wrapper
{ pkgs
, profile               # bmc.profiles.<arch>-<profile> for the host build
, wasmReleaseProfile    # bmc.profiles.wasm-release for collecting .wasm
, crates                # bmc.crates
, autopatchelfBinaries  # bmc.lib.autopatchelfBinaries
, widgetRuntimeDeps     # deps.widgetRuntimeDeps (expects .native fn)
, hostFeatures ? [ ]    # cargo features for the bmc-wasm-host build
, wasmExampleNames      # list of example crate names (e.g. "hello-widget")
}:
let
  lib = pkgs.lib;

  # Reference-cleaning installPhase shared by wasmExamples and per-example
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

  # All examples in one *.wasm-blob tree. Single workspace cargo build,
  # so its cache key is the full workspace src. Used by `capture.nix` to
  # expose the `wasm-examples` flake package and as the manual override
  # target when a wrapper caller wants every wasm at once. Do NOT use
  # this for per-widget pipelines — touching anything in repo invalidates it.
  wasmExamples = wasmReleaseProfile.build.overrideAttrs wasmInstallOverrides;

  # Per-widget wasm derivation, ensuring rebuilds happen only on actual
  # changes of the widget.
  mkWidgetWasm = name:
    (wasmReleaseProfile.buildCrate crates."widget-example-${name}" { }).overrideAttrs wasmInstallOverrides;

  wasmWidgets = lib.genAttrs wasmExampleNames mkWidgetWasm;

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

  # Per-widget packaging. Produces:
  #   $out/lib/bmc-widgets/<name>/bin/<name>          (shell wrapper)
  #   $out/lib/bmc-widgets/<name>/lib/wasm/<name>.wasm
  #   $out/lib/bmc-widgets/<name>/manifest.json
  #
  # The wrapper bakes an absolute Nix-store path for both the host and
  # the .wasm blob — no $(dirname $0) tricks.
  mkWasmWidget =
    { name          # e.g. "hello-widget"
    , wasmDir       # derivation with all *.wasm files flat in $out/
    , wasmFile      # e.g. "hello_widget.wasm" (cargo: hyphens → underscores)
    , manifest      # path to per-widget manifest.json
    , thin          # thin derivation with bin/bmc-wasm-thin
    , host          # host derivation with bin/bmc-wasm-host
    }:
    pkgs.runCommand "bmc-widget-${name}" { } ''
      base=$out/lib/bmc-widgets/${name}
      mkdir -p "$base/bin" "$base/lib/wasm"

      cp ${wasmDir}/${wasmFile} "$base/lib/wasm/${name}.wasm"
      cp ${manifest}            "$base/manifest.json"

      # Unquoted heredoc on purpose: we want $out expanded now (at build
      # time) to bake the absolute store path into the wrapper, while
      # \$@ must survive into the generated script for runtime args.
      cat > "$base/bin/${name}" <<EOF
      #!/bin/sh
      exec ${thin}/bin/bmc-wasm-thin \\
        --wasm $out/lib/bmc-widgets/${name}/lib/wasm/${name}.wasm \\
        --host-bin ${host}/bin/bmc-wasm-host \\
        "\$@"
      EOF
      chmod +x "$base/bin/${name}"
    '';
in
{
  inherit wasmExamples wasmWidgets thin host mkWasmWidget;
}
