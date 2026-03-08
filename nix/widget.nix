# Widget packaging: build individual widgets and combine them.
{ pkgs
, lib
, makeRustflagsEnv
, waylandRuntimeDeps
,
}:
let
  # Build a widget package with the correct directory structure
  mkWidgetPackage =
    { name
    , crate
    , profile
    , features ? [ ]
    , runtimeDeps ? waylandRuntimeDeps
    ,
    }:
    let
      rustCrossTarget =
        if profile ? rustCrossTarget then
          profile.rustCrossTarget
        else
          pkgs.stdenv.hostPlatform.rust.rustcTarget;
      runtimePackages =
        if builtins.isFunction runtimeDeps then runtimeDeps (profile.build_pkgs or pkgs) else runtimeDeps;
      binary = profile.buildCrate crate {
        inherit features;
        env = makeRustflagsEnv { inherit runtimePackages rustCrossTarget; };
      };
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

  # Build all widgets for a given profile and combine into a single output
  mkAllWidgets =
    { profile
    , widgets
    , runtimeDeps ? waylandRuntimeDeps
    ,
    }:
    pkgs.symlinkJoin {
      name = "bmc-widgets";
      paths = lib.mapAttrsToList
        (
          name: widget:
            mkWidgetPackage {
              inherit name profile;
              inherit (widget) crate;
              features = widget.features or [ ];
              runtimeDeps = widget.runtimeDeps or runtimeDeps;
            }
        )
        widgets;
    };
in
{
  inherit mkWidgetPackage mkAllWidgets;
}
