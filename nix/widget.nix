# Widget packaging: build individual widgets and combine them.
{ pkgs, lib, autopatchelfBinaries }:
let
  mkWidgetPackage =
    { name, crate, profile, features ? [ ], runtimeDeps ? _pkgs: [ ] }:
    let
      raw = profile.buildCrate crate { inherit features; };
      binary = autopatchelfBinaries {
        drv = raw;
        runtimeDeps = runtimeDeps profile.build_pkgs;
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
