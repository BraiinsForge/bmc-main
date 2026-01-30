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

        # Shared deps used by both workspace.nix (for package builds) and devShells.
        # Single source of truth to keep build derivations and dev environments in sync.
        commonDeps = {
          # Rust build-time deps (protoc for protobufs, diffutils for cargo)
          buildDeps = with pkgs; [ protobuf diffutils ];

          # Env vars needed by Slint for font rendering
          env = {
            FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ pkgs.corefonts ]; };
          };

          # Runtime libs for GUI/display development (Slint, winit backends)
          guiDeps = with pkgs; [
            fontconfig # runtime dlopen for font enumeration
            xorg.libX11
            xorg.libXcursor
            xorg.libXrandr
            xorg.libXi
            xorg.libXinerama
            xorg.libXext
            xorg.libXft
            xorg.libXrender
            xorg.libxcb
            wayland
            wayland-protocols
            libxkbcommon
            libGL
            vulkan-loader
            mesa
          ];

          # Node.js tooling for frontend builds
          frontendDeps = with pkgs; [ nodejs yarn ];

          # Glibc libs for FHS compat - node_modules binaries (biome, sass-embedded)
          # expect standard /lib64/ld-linux-x86-64.so.2 interpreter
          fhsLibs = with pkgs; [ stdenv.cc.cc.lib glibc ];
        };

        workspace = import ./workspace.nix { inherit self pkgs commonDeps; };
        frontend = import ./frontend { inherit self pkgs; };

        # Full dev shell with Rust + frontend + GUI deps.
        # Uses buildFHSEnv to provide /lib64/ld-linux-x86-64.so.2 for node_modules binaries.
        fullDevShell = (pkgs.buildFHSEnv {
          name = "bmc-full-env";
          targetPkgs = pkgs: with pkgs; [
            ii.rustToolchain
          ]
          ++ commonDeps.buildDeps
          ++ commonDeps.frontendDeps
          ++ commonDeps.fhsLibs
          ++ commonDeps.guiDeps;

          runScript = "bash";
          profile = ''
            export FONTCONFIG_FILE=${commonDeps.env.FONTCONFIG_FILE}
          '';
        }).env;
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

        # default: full local dev (Rust + frontend + GUI)
        # armv7-*: ARM cross-compile shells from workspace.nix
        devShells = {
          inherit (workspace.devShells) armv7-release armv7-debug;
          default = fullDevShell;

          frontend = pkgs.mkShell {
            packages = [ pkgs.yarn pkgs.nodejs ];
            shellHook = ''
              export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [
                pkgs.libgcc
              ]}:$LD_LIBRARY_PATH
            '';
          };
        };
      });
}
