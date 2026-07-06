{
  description = "BMC Flake";
  nixConfig.bash-prompt-prefix = "(bmc) ";

  inputs = {
    self.lfs = true;
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nixlib.url = "git+ssh://git@gitlab.ii.zone/nix/lib";
    ci-tools.url = "git+ssh://git@gitlab.ii.zone/nix/ci-tools.git";
    ci-tools.inputs.nixpkgs.follows = "nixpkgs";
    ci-tools.inputs.flake-utils.follows = "flake-utils";
    ci-tools.inputs.nixlib.follows = "nixlib";

    # uv2nix stack — builds the in-repo Python uv workspace (bmc-tui +
    # bmc-virt harness) from the root uv.lock.
    pyproject-nix = {
      url = "github:pyproject-nix/pyproject.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    uv2nix = {
      url = "github:pyproject-nix/uv2nix";
      inputs.pyproject-nix.follows = "pyproject-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    pyproject-build-systems = {
      url = "github:pyproject-nix/build-system-pkgs";
      inputs.pyproject-nix.follows = "pyproject-nix";
      inputs.uv2nix.follows = "uv2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, nixlib, ci-tools, pyproject-nix, uv2nix, pyproject-build-systems, ... }:
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

        # ── Python uv workspace (bmc-tui + bmc-virt harness) ─────────────────
        # Builds the in-repo Python packages from the root uv.lock. The harness
        # venv is consumed by bmc-virt/flake.nix to deploy the guest event
        # daemon; the interpreter follows the nixpkgs default (pkgs.python3).
        pythonWorkspace = uv2nix.lib.workspace.loadWorkspace { workspaceRoot = ./.; };
        pythonOverlay = pythonWorkspace.mkPyprojectOverlay { sourcePreference = "wheel"; };
        pythonSet =
          (pkgs.callPackage pyproject-nix.build.packages { python = pkgs.python3; }).overrideScope
            (pkgs.lib.composeManyExtensions [
              pyproject-build-systems.overlays.default
              pythonOverlay
            ]);
        # The harness venv contains both members (bmc-virt depends on bmc-tui),
        # so bmc_virt and bmc_tui are both importable for the guest daemon.
        bmc-virt-harness-venv = pythonSet.mkVirtualEnv "bmc-virt-harness" pythonWorkspace.deps.default;

        # Light venv backing the `deck` app — bmc-tui only (rich + tyro).
        bmc-tui-venv = pythonSet.mkVirtualEnv "bmc-tui" { bmc-tui = [ ]; };

        # Local dev shell with Rust + frontend + GUI deps (native only).
        localDevShell = (bmc.profiles.fast.mkShell {
          name = "bmc-local-env";
          packages = (commonDeps.frontendDeps pkgs)
            ++ (with pkgs; [
            cargo-watch
            ffmpeg-headless
            just
            odiff
            python3
            ruff
            ty
            uv
          ]);
        }).overrideAttrs (prev: {
          # numpy/matplotlib (pulled in by the harness Python tests via uv)
          # dlopen libstdc++ at import time; prepend it to the existing loader
          # path so `just python` works in the pure-nix CI shell.
          LD_LIBRARY_PATH =
            pkgs.lib.makeLibraryPath [ pkgs.stdenv.cc.cc.lib ]
            + ":"
            + (prev.LD_LIBRARY_PATH or "");
        });

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

        firmware-index-serve = pkgs.writeShellApplication {
          name = "firmware-index-serve";
          runtimeInputs = with pkgs; [ caddy ];
          runtimeEnv.FIRMWARE_INDEX_SERVE_CONFIG_DIR = ./scripts/firmware-index-serve;
          text = builtins.readFile ./scripts/firmware-index-serve/firmware-index-serve.sh;
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
            # bmc-tui has its own formatter config (bmc-tui/pyproject.toml)
            "bmc-tui/**/*.py"
            # yamlfmt canonicalises folded scalars to a single line (collapsing
            # the readable multi-line `>-` form used for `workspace_lints_ignore`).
            # Skip entirely; CI YAML is stable enough to hand-format.
            ".gitlab-ci.yml"
          ];
        };

        legacyPackages = workspace.legacyPackages // {
          inherit pkgs;
          inherit (workspace.bmc) armv7-nixpkgs;
          ci-tools = ci-tools.packages.${localSystem};
        };

        checks =
          # The armv7 cross outputs are CI-only: keep them out of
          # `nix flake check` (full armv7 workspace build + qemu test run).
          builtins.removeAttrs self.packages.${localSystem} [ "workspace-deps-armv7" "nextest-armv7" ]
          // workspace.checks // checks // {
            content = content-checks;
          };

        bmc = workspace.bmc;

        packages = workspace.packages // {
          wasm-capture = capture.package;
          wasm-examples = capture.wasmExamples;
          wasm-widgets = capture.wasmWidgetsBundle;
          verify-shared-crates = pkgs.writeShellApplication {
            name = "verify-shared-crates";
            runtimeInputs = with pkgs; [ coreutils git getopt jq ];
            text = ''exec ${./scripts/verify_crates.sh} "$@"'';
          };
          check-binary-lfs = pkgs.writeShellApplication {
            name = "check-binary-lfs";
            runtimeInputs = with pkgs; [ file gawk git ];
            text = ''exec ${./scripts/check-binary-lfs.sh} "$@"'';
          };
          bmc-virt-harness = bmc-virt-harness-venv;
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

        # Single device-tooling entry point:
        # `nix run .#deck -- <sysupgrade|deploy> …`.
        apps.deck = {
          type = "app";
          program = "${bmc-tui-venv}/bin/deck";
        };

        # Firmware release index test serving:
        # `nix run .#firmware-index-serve -- <proxy|local>`.
        apps.firmware-index-serve = {
          type = "app";
          program = pkgs.lib.getExe firmware-index-serve;
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
