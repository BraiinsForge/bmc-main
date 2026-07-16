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
  description = "BMC-virt harness — dev shell + guest event-daemon venv (re-exported from the root uv workspace)";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    # The Python uv workspace (bmc-tui + this harness) is built by the
    # repo-root flake. The venv is re-exported below so bmc-virt/flake.nix can
    # keep consuming it via `path:./harness` without knowing about the move.
    bmc-root.url = "path:../..";
  };

  outputs = { nixpkgs, flake-utils, bmc-root, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        # Guest event daemon — the harness venv built from the root uv.lock.
        packages.default = bmc-root.packages.${system}.bmc-virt-harness;

        devShells.default = pkgs.mkShell {
          name = "bmc-virt-harness";
          packages = with pkgs; [
            python3
            uv
            ruff
            ty
            just
            sshpass
            openssh
          ];
          env.LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.stdenv.cc.cc.lib # libstdc++.so.6 for numpy/matplotlib wheels
          ];
          # `uv sync` resolves the workspace and writes .venv at the workspace
          # root (the repo root), not here — so there is no local venv to
          # activate. Use `uv run` (as the justfile does) for the synced env.
          shellHook = ''
            unset PYTHONPATH
          '';
        };
      });
}
