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

{ lib }:
source:
let
  declarations = builtins.filter
    (line: lib.hasPrefix "pub const SDK_VERSION:" line)
    (lib.splitString "\n" source);
  declaration =
    assert lib.assertMsg (builtins.length declarations == 1)
      "expected exactly one SDK_VERSION declaration";
    builtins.head declarations;
  parsed = builtins.match
    "pub const SDK_VERSION: \\(u16, u16, u16\\) = \\(([0-9]+), ([0-9]+), ([0-9]+)\\);"
    declaration;
in
assert lib.assertMsg (parsed != null)
  "SDK_VERSION declaration no longer matches the pinned grammar";
builtins.fromJSON (builtins.elemAt parsed 0)
