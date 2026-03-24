{ pkgs, ty-bin, profiles }:

let
  lib = pkgs.lib;
in
{
  cargo-deny = profiles.fast.mkCargoDeny {
    config = "deny.toml";
    checks = [ "bans" "sources" ];
  };

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
          # examples have their own venv/deps and separate lint setup
          ../bmc-wasm-runtime/examples;
      };
    } ''
    cd $src
    export RUFF_CACHE_DIR="$(mktemp -d)"
    ruff check
    ty check
    touch $out
  '';
}
