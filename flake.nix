# Copyright (C) 2025  Braiins Systems s.r.o.
# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

{
  description = "BMC Flake";
  nixConfig.bash-prompt-prefix = "(bmc) ";

  inputs = {
    self.lfs = true;
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nix-source-info.url = "github:BraiinsForge/nix-source-info";
    nixlib.url = "github:BraiinsForge/nix-lib/master";
    nixlib.inputs.nix-source-info.follows = "nix-source-info";

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

  outputs = { self, nixpkgs, flake-utils, nixlib, pyproject-nix, uv2nix, pyproject-build-systems, ... }:
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
              sha256 = "sha256-A1abGIbOtcBSdrUMhDGrER3pRM1hQP4fp9gh3Y4PKc8=";
            })
            # Overlay yarn & nodejs
            (final: prev: {
              nodejs = prev.nodejs_24;
              yarn = prev.yarn.override { nodejs = prev.nodejs_24; };
            })
          ];
        };

        workspace = import ./workspace.nix { inherit self pkgs; };
        inherit (workspace) commonDeps bmc;
        capture = import ./bmc-wasm-runtime/capture.nix {
          inherit pkgs commonDeps;
          inherit (workspace.bmc) profiles;
          inherit (workspace) wasmExamples wasmWidgetsBundle wasmStackSize;
        };
        checks = import ./nix/checks.nix {
          inherit pkgs ty-bin;
          deckPackages = workspace.legacyPackages.deck-packages;
          inherit (workspace.bmc) profiles crates;
          inherit (workspace) wasmExamples wasmWidgetsBundle wasmStackSize;
          inherit (workspace) wasmWrapperTestPackages;
        };
        wasmRegression = import ./nix/wasm-regression.nix {
          inherit pkgs capture;
          inherit (workspace) wasmWidgets wasmWidgetCatalog;
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
            e2fsprogs
            ffmpeg-headless
            grpcurl
            just
            odiff
            python3
            ruff
            ty
            util-linux
            uv
          ]);
        }).overrideAttrs (prev: {
          # Reserved for artifacts we do not build ourselves: numpy/matplotlib,
          # pulled in by the harness Python tests via uv, dlopen libstdc++
          # and libz at import time — `just python` needs them on the loader path.
          LD_LIBRARY_PATH =
            pkgs.lib.makeLibraryPath [
              pkgs.stdenv.cc.cc.lib
              pkgs.zlib
            ]
            + ":"
            + (prev.LD_LIBRARY_PATH or "");
          # bmc-openwrt/build.rs bakes these into its test binaries as an rpath,
          # keeping the compositor libraries off the loader path.
          BMC_TEST_RPATH = pkgs.lib.makeLibraryPath [
            pkgs.libinput
            pkgs.udev
            pkgs.wayland
            pkgs.libxkbcommon
            # The gallery's scenes dylib reaches bmc-system-overlay, which links gbm.
            pkgs.libgbm
          ];
        });

        # Pre-built ty binary — avoids compiling from source on CI builders
        # that lack the nixos binary cache substituter.
        ty-bin = pkgs.stdenv.mkDerivation {
          pname = "ty";
          version = "0.0.63";
          src = pkgs.fetchurl {
            url = "https://github.com/astral-sh/ty/releases/download/0.0.63/ty-x86_64-unknown-linux-gnu.tar.gz";
            hash = "sha256-6JFqPBEKzk1PJec7WFDv2UR+B5bZPsnho6tpvIxG9Ww=";
          };
          nativeBuildInputs = [ pkgs.autoPatchelfHook ];
          buildInputs = [ pkgs.stdenv.cc.cc.lib ];
          sourceRoot = ".";
          unpackPhase = "tar xzf $src";
          installPhase = ''
            install -Dm755 ty-x86_64-unknown-linux-gnu/ty $out/bin/ty
          '';
        };

        # Format SVGs with the shared svgo config.
        # fd honors .gitignore, so node_modules/target/.venv are skipped.
        fmt-svg = pkgs.writeShellApplication {
          name = "fmt-svg";
          runtimeInputs = with pkgs; [ fd svgo ];
          text = ''
            fd --extension svg --type f \
               '.' 'bmc-virt' 'bmc-wasm-runtime' 'widgets-wasm' 'bmc-field-schema' \
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

            # The ISS globe texture is excluded. Its encoding is chosen
            # by a size/PSNR sweep in widgets-wasm/iss-position/tools;
            # another pass here would requantize that choice.
            # Matched by bare name — fd anchors a glob containing '/' to the search root,
            # so a path pattern stops excluding once this runs on a deeper path.
            fd --extension jpg --extension jpeg --type f \
               --exclude 'texture.jpg' \
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

        upgrade-server = import ./nix/upgrade-server { inherit pkgs; };

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
            # Generated by bmc-widget-codegen; carries no license header
            # (see docs/devel/license-headers.md) and regeneration would
            # drop anything addlicense stamps onto it.
            "*/src/manifest_params.rs"
          ];
        };

        legacyPackages = workspace.legacyPackages // {
          inherit pkgs;
          inherit (workspace.bmc) armv7-nixpkgs;
        };

        checks =
          # The armv7 cross outputs are CI-only: keep them out of
          # `nix flake check` (full armv7 workspace build + qemu test run).
          # The regression report always builds by design; `wasm-regression`
          # is the gate over it.
          builtins.removeAttrs self.packages.${localSystem} [
            "workspace-deps-armv7"
            "workspace-deps-armv7-musl"
            "bmc-nix-cli-armv7-release"
            "nextest-armv7"
            "wasm-regression-report"
          ]
          // workspace.checks // checks
          // { wasm-regression = wasmRegression.check; };

        bmc = workspace.bmc;

        # Per-version init artifacts for the sysupgrade e2e rig
        # (consumed by nix/e2e-artifacts.nix via builtins.getFlake).
        lib = { inherit (workspace) mkInitArtifacts; };

        packages = workspace.packages // {
          wasm-capture = capture.package;
          wasm-stack-usage-report = capture.stackUsageReport;
          wasm-regression-report = wasmRegression.report;
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

        apps.upgrade-server = {
          type = "app";
          program = pkgs.lib.getExe upgrade-server;
        };

        # Single device-tooling entry point:
        # `nix run .#deck -- <sysupgrade|deploy> …`.
        apps.deck = {
          type = "app";
          program = pkgs.lib.getExe (pkgs.writeShellApplication {
            name = "deck";
            runtimeInputs = [ pkgs.grpcurl ];
            text = ''
              exec ${bmc-tui-venv}/bin/deck "$@"
            '';
          });
        };

        # Firmware release index test serving:
        # `nix run .#firmware-index-serve -- <proxy|local>`.
        apps.firmware-index-serve = {
          type = "app";
          program = pkgs.lib.getExe firmware-index-serve;
        };

        # An app, not a check: the script lists files with `git`,
        # which a build sandbox cannot do.
        # As a `checks` entry it was only ever built, never run.
        apps.content-checks = {
          type = "app";
          program = pkgs.lib.getExe content-checks;
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
