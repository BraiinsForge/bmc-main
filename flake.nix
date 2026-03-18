{
  description = "BMC Flake";
  nixConfig.bash-prompt-prefix = "(bmc) ";

  inputs = {
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
        inherit (workspace) commonDeps;
        frontend = import ./frontend { inherit self pkgs; };

        # Local dev shell with Rust + frontend + GUI deps (native only).
        localDevShell = pkgs.mkShell {
          name = "bmc-local-env";

          nativeBuildInputs =
            commonDeps.buildDeps
            ++ commonDeps.guiBuildDeps
            ++ commonDeps.frontendDeps;

          buildInputs = with pkgs; [
            ii.rust.toolchain
          ];

          inherit (commonDeps) env;
        };

        # Full dev shell: local + ARM cross-compilation support.
        armv7Cc = pkgs.pkgsCross.armv7l-hf-multiplatform.pkgsStatic.stdenv.cc;
        fullDevShell = pkgs.mkShell {
          name = "bmc-full-env";
          inputsFrom = [ localDevShell ];
          buildInputs = [ armv7Cc ];
          env = commonDeps.env // {
            CC_armv7_unknown_linux_musleabihf =
              "${armv7Cc.targetPrefix}cc";
            CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER =
              "${armv7Cc.targetPrefix}cc";
          };
        };
      in
      {
        formatter = nixlib.braiinsfmt.${localSystem} {
          nix = true;
          rust = true;
          python = true;
          shell = true;
          protobuf = true;
          toml = true;
          yaml = true;
          copyright = true;

          config.exclude = [
            # Frontend specifies it's own formatting rules
            "frontend/*"
            # Markdown Files can be distorted when formatted
            "docs/*"
            # Upstream crates shall be formatted upstream
            "bmc-shared/ii-net-drv/*"
            "tooling/crates/index-bmc/*"
            "tooling/crates/index-common/*"
            "tooling/idxgen/idxgen-data/*"
            "tooling/minerctl/minerctl-defs/*"
            "tooling/tooling-std/*"
          ];
        };


        legacyPackages = {
          inherit pkgs;
          inherit (workspace.bmc) armv7-pkgs;
        };

        checks = self.packages.${localSystem} // frontend.checks;
        packages = workspace.packages // {
          frontend = frontend.build;
          yarnFiles = frontend.yarnFiles;
          shellcheck = pkgs.writeShellScriptBin "shellcheck" ''
            exec nix run "git+ssh://git@gitlab.ii.zone/nix/ci-tools.git?rev=6071d67e0c5ec498fc88017d36a54bb1b837ad83#shellcheck" "$@" 2>&1
          '';
        };

        # default: full local dev (Rust + frontend + GUI, both local and for Deck)
        # armv7-*: ARM cross-compile shells from workspace.nix
        # local: Just for local development, no compiler for Deck
        devShells = workspace.devShells // {
          local = localDevShell;
          full = fullDevShell;
          default = fullDevShell;
        };
      });
}
