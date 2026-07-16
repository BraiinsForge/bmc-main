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

# autopatchelf-binaries: Patch ELF binaries using autoPatchelfHook + runtimeDependencies.
#
# Applied via overrideAttrs on the buildCrate derivation.
# autoPatchelfHook runs in postFixupHooks (after --shrink-rpath),
# so the rpaths it sets are not stripped.
{ lib, autoPatchelfHook }:
{ drv, runtimeDeps ? [ ] }:
drv.overrideAttrs (prev: {
  nativeBuildInputs = (prev.nativeBuildInputs or [ ]) ++ [ autoPatchelfHook ];
  # runtimeDependencies: autoPatchelfHook appends these /lib paths
  # to rpath of all dynamic executables, regardless of DT_NEEDED.
  runtimeDependencies = (prev.runtimeDependencies or [ ]) ++ (map lib.getLib runtimeDeps);
})
