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
  # imports `../common.justfile`, which has to exist beside it,
  # the specs read the en-XA pack's web catalog from `../assets`,
  # and the device's date and number tables from `../bmc-shared`.
  installPhase = ''
    mkdir -p $out/frontend $out/assets/intl/pseudo/en-XA $out/bmc-shared/time/src $out/bmc-shared/utils/src
    cp -r $src/. -t $out/frontend
    cp ${../../common.justfile} $out/common.justfile
    cp ${../../assets/intl/pseudo/en-XA/web.json} $out/assets/intl/pseudo/en-XA/web.json
    cp ${../../bmc-shared/time/src/time.rs} $out/bmc-shared/time/src/time.rs
    cp ${../../bmc-shared/utils/src/number_format.rs} $out/bmc-shared/utils/src/number_format.rs
  '';

  # dependencies used for automatic shebang patching in fixupPhase
  buildInputs = [ pkgs.pnpmToolchain.pnpm ];
}
