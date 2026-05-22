{
  description = "BMC Flake";
  nixConfig.bash-prompt-prefix = "(bmc) ";

  inputs = {
    self.lfs = true;
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nixlib.url = "git+ssh://git@gitlab.ii.zone/nix/lib";
  };

  outputs = { self, nixpkgs, flake-utils, nixlib, ... }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ] (localSystem:
      let
        pkgs = import nixpkgs {
          inherit localSystem;
          config.allowUnfreePredicate = pkg: builtins.elem (nixpkgs.lib.getName pkg) [
            "corefonts"
          ];
          overlays = [
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

        workspace = import ./workspace.nix { inherit self pkgs; };
        inherit (workspace) commonDeps bmc;
        capture = import ./bmc-wasm-runtime/capture.nix {
          inherit pkgs commonDeps;
          inherit (workspace.bmc) profiles;
          inherit (workspace) wasmExamples wasmWidgetsBundle;
        };
        checks = import ./nix/checks.nix {
          inherit pkgs ty-bin capture;
          inherit (workspace) wasmWidgets wasmWidgetCatalog;
          inherit (workspace.bmc) profiles;
        };
        content-checks = nixlib.braiinschk.${localSystem} {
          mermaid = true;
          justfile = true;
          shell = true;
          config.exclude = [
            "frontend/node_modules/**"
          ];
        };

        # Local dev shell with Rust + frontend + GUI deps (native only).
        localDevShell = bmc.profiles.fast.mkShell {
          name = "bmc-local-env";
          packages = (commonDeps.frontendDeps pkgs)
            ++ (with pkgs; [
            cargo-watch
            ffmpeg-headless
            odiff
            python3
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

        # Format SVGs under bmc-virt and bmc-wasm-runtime using the shared svgo
        # config. fd honors .gitignore so node_modules/target/.venv are skipped.
        fmt-svg = pkgs.writeShellApplication {
          name = "fmt-svg";
          runtimeInputs = with pkgs; [ fd svgo ];
          text = ''
            fd --extension svg --type f \
               '.' 'bmc-virt' 'bmc-wasm-runtime' 'widgets-wasm' \
               --exec-batch svgo --quiet --config svgo.config.js {}
          '';
        };

        # Generic image compression under the given paths (default: cwd). Uses
        # oxipng for PNGs, jpegoptim for JPGs. fd honors .gitignore.
        fmt-images = pkgs.writeShellApplication {
          name = "fmt-images";
          runtimeInputs = with pkgs; [ fd oxipng jpegoptim ];
          text = ''
            paths=("$@")
            [ "''${#paths[@]}" -eq 0 ] && paths=('.')

            fd --extension png --type f \
               '.' "''${paths[@]}" \
               --exec-batch oxipng --zopfli --fast --alpha --strip=safe --opt=max --preserve {}

            fd --extension jpg --extension jpeg --type f \
               '.' "''${paths[@]}" \
               --exec-batch jpegoptim --max=70 --strip-all --threshold=5 {}
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
          justfile = true;
          # Configs
          toml = true;
          yaml = true;
          # Documents
          html = false;
          markdown = true;
          mermaid = true;
          copyright = true;

          config.exclude = [
            # Frontend specifies it's own formatting rules
            "frontend/*"
            # Upstream crates shall be formatted upstream
            "bmc-shared/ii-net-drv/*"
            # Harness has its own formatter config (bmc-virt/harness/pyproject.toml)
            "bmc-virt/harness/**/*.py"
            # yamlfmt canonicalises folded scalars to a single line (collapsing
            # the readable multi-line `>-` form used for WORKSPACE_LINTS_IGNORE_PATHS).
            # Skip entirely; CI YAML is stable enough to hand-format.
            ".gitlab-ci.yml"
          ];
        };

        legacyPackages = workspace.legacyPackages // {
          inherit pkgs;
          inherit (workspace.bmc) armv7-nixpkgs;
        };

        checks = self.packages.${localSystem} // workspace.checks // checks // {
          content = content-checks;
        };

        bmc = workspace.bmc;

        packages = workspace.packages // {
          wasm-capture = capture.package;
          wasm-examples = capture.wasmExamples;
          wasm-widgets = capture.wasmWidgetsBundle;
        };

        apps.fmt-svg = {
          type = "app";
          program = pkgs.lib.getExe fmt-svg;
        };

        apps.fmt-images = {
          type = "app";
          program = pkgs.lib.getExe fmt-images;
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
