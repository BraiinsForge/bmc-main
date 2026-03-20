{
  description = "BMC Flake";
  nixConfig.bash-prompt-prefix = "(bmc) ";

  inputs = {
    self.lfs = true;
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nixlib.url = "git+ssh://git@gitlab.ii.zone/nix/lib";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, nixlib, fenix, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ] (localSystem:
      let
        pkgs = import nixpkgs {
          inherit localSystem;
          config.allowUnfreePredicate = pkg: builtins.elem (nixpkgs.lib.getName pkg) [
            "corefonts"
          ];
          overlays = [
            fenix.overlays.default
            nixlib.overlays.default
            (nixlib.mkRustOverlayFromToolchainFile {
              file = ./rust-toolchain.toml;
              sha256 = "sha256-qqF33vNuAdU5vua96VKVIwuc43j4EFeEXbjQ6+l4mO4=";
            })
            # Overlay yarn & nodejs
            (final: prev: {
              nodejs = prev.nodejs_22;
              yarn = prev.yarn.override { nodejs = prev.nodejs_22; };
            })
          ];
        };
        lib = pkgs.lib;

        workspace = import ./workspace.nix { inherit self pkgs; };
        inherit (workspace) commonDeps bmc;
        frontend = import ./frontend { inherit self pkgs; };
        capture = import ./bmc-wasm-runtime/capture.nix {
          inherit self pkgs commonDeps;
          inherit (workspace.bmc) profiles;
        };
        checks = import ./nix/checks.nix {
          inherit pkgs ty-bin;
          inherit (workspace.bmc) profiles;
        };

        # Local dev shell with Rust + frontend + GUI deps (native only).
        localDevShell = bmc.profiles.fast.mkShell {
          name = "bmc-local-env";
          packages = (commonDeps.frontendDeps pkgs)
            ++ (with pkgs; [
            ffmpeg-headless
            odiff
          ]);
        };

        # Pre-built ty binary — avoids compiling from source on CI builders
        # that lack the nixos binary cache substituter.
        ty-bin = pkgs.stdenv.mkDerivation {
          pname = "ty";
          version = "0.0.23";
          src = pkgs.fetchurl {
            url = "https://github.com/astral-sh/ty/releases/download/0.0.23/ty-x86_64-unknown-linux-gnu.tar.gz";
            hash = "sha256-4ctmL00e9mcc9K0lQVOaTbGlYwhIA054ON3V4ymjeXU=";
          };
          nativeBuildInputs = [ pkgs.autoPatchelfHook ];
          buildInputs = [ pkgs.stdenv.cc.cc.lib ];
          sourceRoot = ".";
          unpackPhase = "tar xzf $src";
          installPhase = ''
            install -Dm755 ty-x86_64-unknown-linux-gnu/ty $out/bin/ty
          '';
        };

        fmt-svg = pkgs.writeShellApplication {
          name = "fmt-svg";
          runtimeInputs = with pkgs; [ findutils svgo coreutils ];
          text = ''
            find bmc-wasm-runtime -name '*.svg' \
              -not -path '*/target/*' \
              -not -path '*/.venv/*' \
              -print0 | xargs -0 -P "$(nproc)" -I {} svgo --config bmc-wasm-runtime/svgo.config.js -i {} -o {}
          '';
        };
      in
      {
        formatter = nixlib.braiinsfmt.${localSystem} {
          # Code
          nix = true;
          rust = true;
          python = true;
          protobuf = true;
          # Scripts
          shell = true;
          # Configs
          toml = true;
          yaml = true;
          # Documents
          html = false;
          markdown = true;
          copyright = true;

          config.exclude = [
            # Frontend specifies it's own formatting rules
            "frontend/*"
            # Markdown Files can be distorted when formatted
            "docs/*"
            # Upstream crates shall be formatted upstream
            "bmc-shared/ii-net-drv/*"
          ];
        };


        legacyPackages = {
          inherit pkgs;
          inherit (workspace.bmc) armv7-pkgs;
        };

        checks = self.packages.${localSystem} // frontend.checks // checks;
        packages = workspace.packages // {
          wasm-capture = capture.package;
          frontend = frontend.build;
          yarnFiles = frontend.yarnFiles;
          shellcheck = pkgs.writeShellScriptBin "shellcheck" ''
            exec nix run "git+ssh://git@gitlab.ii.zone/nix/ci-tools.git?rev=6071d67e0c5ec498fc88017d36a54bb1b837ad83#shellcheck" "$@" 2>&1
          '';
        };

        apps.fmt-svg = {
          type = "app";
          program = pkgs.lib.getExe fmt-svg;
        };

        apps.wasm-capture = {
          type = "app";
          program = pkgs.lib.getExe capture.package;
        };

        # default: full local dev (Rust + frontend + GUI, both local and for Deck)
        # armv7-*: ARM cross-compile shells from workspace.nix
        # local: Just for local development, no compiler for Deck
        devShells = workspace.devShells // {
          local = localDevShell;
          default = localDevShell;
        };
      });
}
