{
  description = "BMC Flake";
  nixConfig.bash-prompt-prefix = "(bmc) ";

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
          ];
        };

        workspace = import ./workspace.nix {
          inherit self pkgs;
        };

        web-assets = pkgs.stdenvNoCC.mkDerivation {
          name = "bmc-web-assets";
          nativeBuildInputs = [ ];
          phases = [ "buildPhase" "installPhase" ];
          buildPhase = ''
            mkdir -p bmc
            cat <<-EOF > bmc/index.html
            <!DOCTYPE html>
            <html>
            <head><title>Hello</title></head>
            <body><h1>Hello, world!</h1></body>
            </html>
            EOF
          '';
          installPhase = ''
            mkdir -p $out
            cp -r bmc $out/
          '';
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

        packages = workspace.packages // { inherit web-assets; };

        devShells = workspace.devShells // {
          default = pkgs.mkShell { packages = [ pkgs.ii.rustToolchain ]; };
        };
      });
}
