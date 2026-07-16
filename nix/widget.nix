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

# Widget packaging: build individual widgets and combine them.
{ pkgs, lib, autopatchelfBinaries }:
let
  mkWidgetPackage =
    { name, crate, profile, features ? [ ], runtimeDeps ? _pkgs: [ ] }:
    let
      raw = profile.buildCrate crate { inherit features; };
      binary = autopatchelfBinaries {
        drv = raw;
        runtimeDeps = runtimeDeps profile.pkgs;
      };
      widgetSrc = ../widgets + "/${name}";
    in
    pkgs.runCommand "bmc-widget-${name}" { } ''
      mkdir -p $out/lib/bmc-widgets/${name}/bin
      cp ${widgetSrc}/manifest.json $out/lib/bmc-widgets/${name}/
      cp -a ${binary}/bin/. $out/lib/bmc-widgets/${name}/bin/
      if [ -d "${widgetSrc}/assets" ]; then
        cp -r ${widgetSrc}/assets $out/lib/bmc-widgets/${name}/
      fi
    '';

  mkAllWidgets =
    { profile, widgets, runtimeDeps ? _pkgs: [ ] }:
    pkgs.symlinkJoin {
      name = "bmc-widgets";
      paths = lib.mapAttrsToList
        (name: widget:
          mkWidgetPackage {
            inherit name profile runtimeDeps;
            inherit (widget) crate;
            features = widget.features or [ ];
          }
        )
        widgets;
    };
in
{
  inherit mkWidgetPackage mkAllWidgets;
}
