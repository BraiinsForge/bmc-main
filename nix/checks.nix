{ pkgs, ty-bin, profiles, capture, wasmWidgets }:

let
  lib = pkgs.lib;

  # One regression derivation per widget. Each pins to:
  #   - that widget's source dir only (per-widget src cache key)
  #   - that widget's docker-spider-narrowed wasm (per-widget wasm rebuild)
  #   - the capture wrapper for env + binary
  #
  # Captures land in the build sandbox CWD at ./captures. On regression
  # `wasm-capture verify` exits non-zero and kills the derivation (set -e
  # is implicit in nix builders); $out is destroyed, but the CI runner
  # has `keep-failed = true` so the sandbox survives at
  # /tmp/nix-build-wasm-regression-<name>.drv-* for CI to scrape (see
  # .gitlab-ci.yml).
  mkWidgetCheck = name: pkgs.runCommand "wasm-regression-${name}"
    {
      nativeBuildInputs = [ capture.package ];
      src = ../bmc-wasm-runtime/examples + "/${name}";
      wasm = wasmWidgets.${name};
    } ''
    widgets=$(mktemp -d)
    ln -s "$src" "$widgets/${name}"
    mkdir captures
    wasm-capture verify \
      --widgets-dir="$widgets" \
      --wasm-dir="$wasm" \
      --output-dir=captures \
      --example=${name}
    mkdir -p $out
  '';

  widgetChecks = lib.mapAttrs (name: _: mkWidgetCheck name) wasmWidgets;
in
{
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

  # Aggregate check — depends on every per-widget regression derivation
  # so nix's scheduler runs them in parallel. The per-widget derivations
  # are internal and not exposed individually under flake.checks.
  wasm-regression = pkgs.runCommand "wasm-regression"
    {
      nativeBuildInputs = lib.attrValues widgetChecks;
    } ''
    mkdir -p $out
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
            ../bmc-virt/harness
          ]);
      };
    } ''
    cd $src
    export RUFF_CACHE_DIR="$(mktemp -d)"
    ruff check
    ty check
    touch $out
  '';
}
