{ pkgs, ty-bin, profiles, capture, wasmWidgets, wasmWidgetCatalog }:

let
  lib = pkgs.lib;

  # Widgets eligible for visual regression — only those
  # with a populated `capture/config.toml`.
  # Other widgets compile but don't ship capture fixtures yet.
  regressionCatalog = lib.filterAttrs (_: w: w.hasCaptureConfig) wasmWidgetCatalog;

  # One regression derivation per widget. Each pins to:
  #   - that widget's source dir only (per-widget src cache key)
  #   - that widget's docker-spider-narrowed wasm (per-widget wasm rebuild)
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

  # Clippy over the widgets-wasm workspace at the wasm32 target — enforces
  # the lint gate from docs/devel/wasm-widgets/best-practices.md.
  # Lib/bin targets only (no --all-targets): widget test code is host-only
  # by design and cannot compile for wasm32. crane bakes the clippy flags
  # into buildPhase at eval time, so strip the flag from the script itself.
  clippy-wasm-widgets = profiles.wasm-widgets-debug.clippy.overrideAttrs (old: {
    buildPhase = builtins.replaceStrings [ " --all-targets" ] [ "" ] old.buildPhase;
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
    ty check
    touch $out
  '';
}
