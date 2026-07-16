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

# bmc-lib: Shared package builders for the BMC project.
{ pkgs, lib, armv7Pkgs }:
let
  autopatchelfBinaries = import ./autopatchelf-binaries.nix {
    inherit lib;
    autoPatchelfHook = armv7Pkgs.autoPatchelfHook;
  };
  packageLib = import ./package.nix { inherit pkgs lib; };
  serviceLib = import ./service.nix { inherit pkgs lib; };
  mkIndex = import ./mkIndex.nix { inherit pkgs lib; };
  mkTarball = import ./mkTarball.nix { inherit pkgs lib mkIndex; };
  mkPackageFeed = import ./mkPackageFeed.nix { inherit pkgs; };
  # Symlink a single named binary out of a pre-built `bmc-nix` derivation so
  # every bin compiles once. Asserts the binary exists first, otherwise a wrong
  # or removed name would yield a dangling link that only fails when
  # dereferenced (packaging / device boot).
  selectBmcNixBin = { pkgs, bmcNix }: name: pkgs.runCommand name { } ''
    test -e ${bmcNix}/bin/${name}
    mkdir -p $out/bin
    ln -s ${bmcNix}/bin/${name} $out/bin/${name}
  '';
in
{
  inherit autopatchelfBinaries selectBmcNixBin;
  inherit (packageLib) mkPackage mkPrioritizedEntries;
  inherit (serviceLib) mkOpenWrtService mkOpenWrtDaemon;
  inherit mkIndex mkTarball mkPackageFeed;
  mkCorePackage = import ./pkgs/core/package.nix { inherit pkgs lib; };
  inherit (import ./widget.nix { inherit pkgs lib autopatchelfBinaries; })
    mkWidgetPackage mkAllWidgets;
}
