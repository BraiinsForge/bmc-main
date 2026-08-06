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

{ pkgs, ty-bin, profiles, capture, wasmWidgets, wasmWidgetCatalog, deckPackages }:

let
  lib = pkgs.lib;
  publicAsset = import ./public-asset.nix { inherit lib; };
  publicAssetManifest = ./test-data/public-assets/manifest.json;
  publicJpg = publicAsset.mkPublicIcon publicAssetManifest "icon.jpg";
  publicJpeg = publicAsset.mkPublicIcon publicAssetManifest "icon.jpeg";
  rejects = value: !(builtins.tryEval (builtins.deepSeq value true)).success;
  licenseHeaderExtensions = lib.filter (extension: extension != "")
    (lib.splitString "\n" (builtins.readFile ../scripts/license_header_extensions.txt));

  # Widgets eligible for visual regression — only those
  # with a populated `capture/config.toml`.
  # Other widgets compile but don't ship capture fixtures yet.
  regressionCatalog = lib.filterAttrs (_: w: w.hasCaptureConfig) wasmWidgetCatalog;

  # One regression derivation per widget. Each pins to:
  #   - that widget's source dir only (per-widget src cache key)
  #   - that widget's wasm derivation (per-widget wasm rebuild)
  #   - the capture wrapper for env + binary
  mkWidgetCheck = name: entry: pkgs.runCommand "wasm-regression-${name}"
    {
      nativeBuildInputs = [ capture.package ];
      src = entry.src;
      wasm = wasmWidgets.${name};
    } ''
    widgets=$(mktemp -d)
    ln -s "$src" "$widgets/${name}"
    mkdir captures
    wasm-capture verify \
      --workspace="$widgets" \
      --wasm-dir="$wasm" \
      --output-dir=captures \
      --widget=${name}
    mkdir -p $out
  '';

  widgetChecks = lib.mapAttrs mkWidgetCheck regressionCatalog;
in
{
  public-widget-assets =
    assert builtins.readFileType publicJpg == "regular";
    assert publicJpg == publicJpeg;
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "/icon.jpg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "../icon.jpg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "./icon.jpg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "nested//icon.jpg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "icon.exe");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "missing.svg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "icon-link.jpg");
    let icon = deckPackages.widget-clock.metadata.assets.icon;
    in
    assert builtins.readFileType icon == "regular";
    assert !(lib.hasInfix "/lib/bmc-widgets/" (toString icon));
    pkgs.runCommand "public-widget-assets" { } ''
      touch $out
    '';

  cargo-deny = profiles.fast.mkCargoDeny {
    config = "deny.toml";
    checks = [ "bans" "sources" ];
  };

  # Wasm-side cargo-deny — blocks bloat crates (serde, tokio, hyper, …)
  # from the wasm32 dep graph so they can't creep into widget binaries.
  # Target restriction lives in `deny-wasm.toml`'s `[graph].targets`.
  cargo-deny-wasm = profiles.fast.mkCargoDeny {
    config = "deny-wasm.toml";
    checks = [ "bans" "sources" ];
  };

  # Block allocating fmt macros (format!, println!, dbg!, …)
  # in widget code via ast-grep. cargo-deny is crate-level
  # — this is macro-level.
  no-fmt-in-wasm = pkgs.runCommand "no-fmt-in-wasm"
    {
      nativeBuildInputs = [ pkgs.ast-grep ];
      src = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.unions [
          ../sgconfig.yml
          ../rules
          ../bmc-wasm-runtime/sdk/src
          ../bmc-wasm-runtime/protocol/src
          ../bmc-wasm-runtime/examples
          ../widgets-wasm
        ];
      };
    } ''
    cd $src
    ast-grep scan --error
    touch $out
  '';

  docs-wasm = profiles.fast.mkCargoDoc {
    package = "bmc-wasm-sdk";
  };

  build-wasm-widgets = profiles.wasm-widgets-debug.build;

  clippy-wasm-widgets = profiles.wasm-widgets-debug.clippy.overrideAttrs (old: {
    buildPhase = builtins.replaceStrings [ " --all-targets" ] [ " --target wasm32-unknown-unknown" ] old.buildPhase;
  });

  # Widget unit tests, compiled and run on the host target.
  test-wasm-widgets = profiles.wasm-widgets-host.nextest;

  # Aggregate check — depends on every per-widget regression derivation
  # so nix's scheduler runs them in parallel. The per-widget derivations
  # are internal and not exposed individually under flake.checks.
  wasm-regression = pkgs.runCommand "wasm-regression"
    {
      nativeBuildInputs = lib.attrValues widgetChecks;
    } ''
    mkdir -p $out
  '';

  # Every first-party source file must carry the GPL license header.
  # The script's exclusion list mirrors docs/devel/license-headers.md.
  license-headers = pkgs.runCommand "license-headers"
    {
      src = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.unions [
          ../scripts/check_license_headers.sh
          ../scripts/license_header_extensions.txt
          (lib.fileset.fileFilter
            (f: builtins.any f.hasExt licenseHeaderExtensions)
            ../.)
        ];
      };
    } ''
    bash $src/scripts/check_license_headers.sh
    touch $out
  '';

  python-lint = pkgs.runCommand "python-lint"
    {
      nativeBuildInputs = [ pkgs.ruff ty-bin pkgs.python3 ];
      src = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.difference
          (lib.fileset.unions [
            (lib.fileset.fileFilter (f: f.hasExt "py") ../.)
            ../ruff.toml
          ])
          # subprojects with their own nix dev shell, deps, and lint setup
          (lib.fileset.unions [
            ../bmc-wasm-runtime/examples
            ../widgets-wasm
            ../bmc-virt/harness
            ../bmc-tui
          ]);
      };
    } ''
    cd $src
    export RUFF_CACHE_DIR="$(mktemp -d)"
    ruff check
    # Fail on @deprecated APIs; must be a CLI flag — ty ignores [tool.ty.rules] here.
    ty check --error deprecated
    touch $out
  '';
}
