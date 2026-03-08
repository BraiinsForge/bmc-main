# Widget packaging: build individual widgets and combine them.
{ pkgs, lib }:
let
  mkWidgetPackage =
    { name, crate, profile, features ? [ ] }:
    let
      binary = profile.buildCrate crate { inherit features; };
      widgetSrc = ../widgets + "/${name}";
    in
    pkgs.runCommand "bmc-widget-${name}" { } ''
      mkdir -p $out/lib/bmc-widgets/${name}/bin
      cp ${widgetSrc}/manifest.json $out/lib/bmc-widgets/${name}/
      cp ${binary}/bin/* $out/lib/bmc-widgets/${name}/bin/
      if [ -d "${widgetSrc}/assets" ]; then
        cp -r ${widgetSrc}/assets $out/lib/bmc-widgets/${name}/
      fi
    '';

  mkAllWidgets =
    { profile, widgets }:
    pkgs.symlinkJoin {
      name = "bmc-widgets";
      paths = lib.mapAttrsToList
        (name: widget:
          mkWidgetPackage {
            inherit name profile;
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
