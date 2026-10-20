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

# The `intl` apps. Apps rather than checks, like `ast-grep`: they read the working tree
# and its history, so a stale translator file fails before it is committed;
# a build sandbox would only ever see a committed one.
{ writeShellApplication, git, ii, formatjs-cli, intl-tools }:

let
  intl = name: text: writeShellApplication {
    inherit name text;
    # The toolchain for `cargo metadata`, which names the crates that can declare host copy.
    runtimeInputs = [ formatjs-cli git ii.rust.toolchain intl-tools ];
  };
in
{
  verify = intl "intl-verify" ''exec intl verify "$@"'';
  # A caller's own `--since` replaces the default rather than repeating the flag.
  untranslated = intl "intl-untranslated" ''
    case " $* " in
      *" --since"*) exec intl untranslated "$@" ;;
      *) exec intl untranslated --since merge-base "$@" ;;
    esac
  '';
}
