{
  description = "cratezero Flake";
  nixConfig.bash-prompt-prefix = "(cratezero) ";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nixlib.url = "git+ssh://git@gitlab.ii.zone/bos/nixlib";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, nixlib, fenix, ... } @ inputs:
    flake-utils.lib.eachSystem [ "x86_64-linux" ] (localSystem:
      let
        pkgs = import nixpkgs {
          inherit localSystem;
          overlays = [
            fenix.overlays.default
            (nixlib.mkOverlay {
              loadRustToolchain = p: p.fenix.fromToolchainFile {
                file = ./rust-toolchain.toml;
                sha256 = "sha256-X/4ZBHO3iW0fOenQ3foEvscgAPJYl2abspaBThDOukI=";
              };
            })
          ];
        };

        workspace = import ./workspace.nix {
          inherit self pkgs;
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
        };

        checks = self.packages.${localSystem};

        packages = workspace.packages // { };

        devShells = workspace.devShells // {
          default = pkgs.mkShell { packages = [ pkgs.ii.rustToolchain ]; };
        };
      });
}
