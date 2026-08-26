# Copyright (C) 2025  Braiins Systems s.r.o.
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

{ pkgs }:

pkgs.stdenv.mkDerivation {
  name = "bmc-fe-patched-src";
  src = ../.;

  dontBuild = true;
  dontUnpack = true;

  # Nested under `frontend/` rather than sitting at the root: `justfile`
  # imports `../common.justfile`, which has to exist beside it.
  installPhase = ''
    mkdir -p $out/frontend
    cp -r $src/. -t $out/frontend
    cp ${../../common.justfile} $out/common.justfile
  '';

  # dependencies used for automatic shebang patching in fixupPhase
  buildInputs = [ pkgs.yarn ];
}
