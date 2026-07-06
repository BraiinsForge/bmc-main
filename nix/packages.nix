# packages: All package definitions with release metadata.
#
# Single source of truth for what packages exist. Each entry pairs
# build logic with release metadata. Consumers (e.g. init-artifacts)
# select the subset they need.
{ bmc
, armv7Pkgs
, deps
, wasmWidgets
, thin
, host
, mkWasmWidget
, wasmWidgetCatalog
, profile
, openwrtFeatures ? [ ]
}:
let
  inherit (bmc.lib) mkWidgetPackage;
  inherit (bmc) crates;
  inherit (deps) widgetRuntimeDeps frontend;
  lib = armv7Pkgs.lib;

  # Widget release metadata surfaced into the package index so the frontend
  # add-a-widget menu can discover installable widgets. Version, description and
  # picker fields are read from the widget's `manifest.json` at eval time
  # (read-only file read, no IFD). The icon path points into the built package's
  # installed assets so upstream index tooling can collect and translate it.
  mkWidgetMetadata = { name, pkg, manifest }:
    let
      m = builtins.fromJSON (builtins.readFile manifest);
      icon = m.icon or null;
    in
    {
      version = m.version;
      description = m.description;
      metadata = {
        widget = {
          uid = m.uid;
          display_name = m.name;
          category = m.category or "misc";
        } // lib.optionalAttrs (m ? subname && m.subname != null) {
          subname = m.subname;
        };
      } // lib.optionalAttrs (icon != null) {
        assets.icon = "${pkg}/lib/bmc-widgets/${name}/${icon}";
      };
    };

  # Per-wasm-widget package def, generated from the filesystem-derived catalog.
  mkWasmWidgetEntry = name: entry:
    let
      pkg = mkWasmWidget {
        inherit name thin host;
        wasmDir = wasmWidgets.${name};
        inherit (entry) wasmFile manifest;
      };
      meta = mkWidgetMetadata { inherit name pkg; inherit (entry) manifest; };
    in
    {
      inherit pkg;
      inherit (meta) version description metadata;
      category = "widget";
      upgrade_strategy = null;
      install_strategy = null;
    };

  shippableCatalog = lib.filterAttrs (_: w: w.isShippable) wasmWidgetCatalog;
  wasmWidgetPackages = lib.mapAttrs'
    (name: entry: lib.nameValuePair "widget-${name}" (mkWasmWidgetEntry name entry))
    shippableCatalog;
in
wasmWidgetPackages // {
  core = import ./pkgs/core { inherit bmc armv7Pkgs deps profile openwrtFeatures; };
  bos-avahi = import ./pkgs/bos-avahi { inherit bmc armv7Pkgs; };
  bmc-nix-cli = {
    pkg = profile.buildCrate crates.bmc-nix-cli { };
    version = "0.1.0";
    category = "core";
    description = "Nix package management CLI tool";
    upgrade_strategy = "reboot";
    install_strategy = null;
  };
  nix = {
    pkg = armv7Pkgs.nix;
    version = armv7Pkgs.nix.version;
    category = "core";
    description = "Nix package manager";
    upgrade_strategy = "reboot";
    install_strategy = null;
  };
  widget-flip-clock =
    let
      pkg = mkWidgetPackage {
        name = "flip-clock";
        crate = crates.widget-flip-clock;
        inherit profile;
        runtimeDeps = widgetRuntimeDeps.native;
        features = [ "standalone" ];
      };
      meta = mkWidgetMetadata {
        name = "flip-clock";
        inherit pkg;
        manifest = ../widgets/flip-clock/manifest.json;
      };
    in
    {
      inherit pkg;
      inherit (meta) version description metadata;
      category = "widget";
      upgrade_strategy = null;
      install_strategy = null;
    };
  bmc-frontend = {
    pkg = armv7Pkgs.runCommand "bmc-frontend-profile" { } ''
      mkdir -p $out/www
      ln -s ${frontend} $out/www/bmc
    '';
    version = "0.1.0";
    category = "dev";
    description = "Frontend web assets under www/bmc (dev use; not shipped)";
    upgrade_strategy = null;
    install_strategy = null;
  };
}
