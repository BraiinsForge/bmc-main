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
            (nixlib.mkOverlay {
              loadRustToolchain = p: p.fenix.fromToolchainFile {
                file = ./rust-toolchain.toml;
                sha256 = "sha256-X/4ZBHO3iW0fOenQ3foEvscgAPJYl2abspaBThDOukI=";
              };
            })
            # Overlay yarn & nodejs
            (final: prev: {
              nodejs = prev.nodejs_22;
              yarn = prev.yarn.override { nodejs = prev.nodejs_22; };
            })
          ];
        };

        workspace = import ./workspace.nix { inherit self pkgs; };
        frontend = import ./frontend { inherit self pkgs; };
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

        checks = self.packages.${localSystem} // frontend.checks;
        packages = workspace.packages // {
          frontend = frontend.build;
          yarnFiles = frontend.yarnFiles;
          shellcheck = pkgs.writeShellScriptBin "shellcheck" ''
            exec nix run "git+ssh://git@gitlab.ii.zone/nix/ci-tools.git?rev=6071d67e0c5ec498fc88017d36a54bb1b837ad83#shellcheck" "$@" 2>&1
          '';
        };

        devShells = workspace.devShells // {
          frontend = pkgs.mkShell {
            packages = [ pkgs.yarn pkgs.nodejs ];
            shellHook = ''
              export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [
                pkgs.libgcc
              ]}:$LD_LIBRARY_PATH
            '';
          };
          default = pkgs.mkShell { packages = [ pkgs.ii.rustToolchain ]; };
        };
      });
}
