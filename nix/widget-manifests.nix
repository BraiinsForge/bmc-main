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

# The catalog of the widgets with nothing compiled, for a bmc-mock that renders none:
# each widget's manifest and icon, with a stub standing in for each of its programs.
{ lib
, runCommand
, writeShellScript
, widgets
, wasmWidgetCatalog
}:
let
  inherit (import ./public-asset.nix { inherit lib; }) mkPublicIcon;
  manifests =
    lib.mapAttrs (_: entry: entry.manifest) (lib.filterAttrs (_: entry: entry.isShippable) wasmWidgetCatalog)
    // lib.mapAttrs (name: _: ../widgets + "/${name}/manifest.json") widgets;
  # The registry requires each program a manifest declares to be executable.
  stub = writeShellScript "bmc-widget-stub" ''
    echo "$0: widget-manifests ships no programs; build .#widgets to run one" >&2
    exit 1
  '';
  installWidget = name: manifest:
    let
      parsed = builtins.fromJSON (builtins.readFile manifest);
      icon = parsed.icon or null;
      programs = [ parsed.binary ] ++ map (program: program.path) (lib.attrValues (parsed.programs or { }));
    in
    ''
      base=$out/lib/bmc-widgets/${name}
      install -D -m644 ${manifest} "$base/manifest.json"
      ${lib.optionalString (icon != null) ''
        install -D -m644 ${mkPublicIcon manifest icon} "$base/${icon}"
      ''}
      ${lib.concatMapStrings (path: ''
        install -D -m755 ${stub} "$base/${path}"
      '') programs}
    '';
in
runCommand "bmc-widget-manifests" { }
  (lib.concatStrings (lib.mapAttrsToList installWidget manifests))
