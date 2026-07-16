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

# e2e-artifacts: per-version init artifacts for the sysupgrade e2e rig.
#
# Not a flake output — flake outputs cannot take parameters. The harness
# builds all four attrs in one invocation (one consistent evaluation):
#
#   nix build --impure -f nix/e2e-artifacts.nix \
#     index-a tarball-a index-b tarball-b \
#     --argstr bosVersionA <version-A> --argstr bosVersionB <version-B>
#
# --impure is required: builtins.getFlake on the enclosing (unlocked,
# local) flake and builtins.currentSystem fail under pure evaluation.
# The git+file scheme forces the git fetcher — a bare path would use the
# path fetcher, copying the whole worktree (target/, node_modules/, ...)
# into the store instead of just the tracked files.
# Variant B bumps bmc-nix-cli 0.1.0 -> 0.1.1 so the upgrade plan against
# an A-initialized store is non-empty.
{ bosVersionA
, bosVersionB
, flakeRef ? "git+file://" + builtins.toString ../.
, system ? builtins.currentSystem
}:
let
  flake = builtins.getFlake flakeRef;
  mkInitArtifacts = flake.lib.${system}.mkInitArtifacts;
  a = mkInitArtifacts { bosVersion = bosVersionA; };
  b = mkInitArtifacts {
    bosVersion = bosVersionB;
    packageBump = { name = "bmc-nix-cli"; version = "0.1.1"; };
  };
in
{
  index-a = a.init-index-armv7;
  tarball-a = a.init-tarball-armv7;
  index-b = b.init-index-armv7;
  tarball-b = b.init-tarball-armv7;
}
