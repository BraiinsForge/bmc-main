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

{ pkgs }:

# An app rather than a check, for the same reason as `content-checks`:
# it scans the working tree, which a build sandbox cannot reach.
#
# Each rule's own `files:` globs are therefore the only scoping, so a rule
# added under `.config/ast-grep/rules` is enforced without touching this file.
pkgs.writeShellApplication {
  name = "ast-grep-scan";
  runtimeInputs = [ pkgs.ast-grep pkgs.git ];
  text = ''
    cd "$(git rev-parse --show-toplevel)"
    exec ast-grep scan --error --config .config/ast-grep/sgconfig.yml "$@"
  '';
}
