# packages: All package definitions with release metadata.
#
# Single source of truth for what packages exist. Each entry pairs
# build logic with release metadata. Consumers (e.g. init-artifacts)
# select the subset they need.
{ bmc, armv7Pkgs, deps, wasmWidgets, thin, host, mkWasmWidget, wasmWidgetCatalog }:
let
  inherit (bmc.lib) mkWidgetPackage;
  inherit (bmc) crates;
  inherit (deps) widgetRuntimeDeps frontend;
  lib = armv7Pkgs.lib;
  profile = bmc.profiles.armv7-glibc-release;

  # Per-wasm-widget package def, generated from the filesystem-derived catalog.
  # Description + version are read from each widget's `manifest.json` at eval time
  # (read-only file read, no IFD).
  mkWasmWidgetEntry = name: entry:
    let
      manifestJson = builtins.fromJSON (builtins.readFile entry.manifest);
    in
    {
      pkg = mkWasmWidget {
        inherit name thin host;
        wasmDir = wasmWidgets.${name};
        inherit (entry) wasmFile manifest;
      };
      version = manifestJson.version;
      category = "widget";
      description = manifestJson.description;
      upgrade_strategy = null;
      install_strategy = null;
    };

  shippableCatalog = lib.filterAttrs (_: w: w.isShippable) wasmWidgetCatalog;
  wasmWidgetPackages = lib.mapAttrs'
    (name: entry: lib.nameValuePair "widget-${name}" (mkWasmWidgetEntry name entry))
    shippableCatalog;
in
wasmWidgetPackages // {
  core = import ./pkgs/core { inherit bmc armv7Pkgs deps; };
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
  digital-clock = {
    pkg = mkWidgetPackage {
      name = "digital-clock";
      crate = crates.widget-digital-clock;
      inherit profile;
      runtimeDeps = widgetRuntimeDeps.slint;

      features = [ "standalone" ];
    };
    version = "1.0.0";
    category = "widget";
    description = "Digital clock widget";
    upgrade_strategy = null;
    install_strategy = null;
  };
  flip-clock = {
    pkg = mkWidgetPackage {
      name = "flip-clock";
      crate = crates.widget-flip-clock;
      inherit profile;
      runtimeDeps = widgetRuntimeDeps.native;

      features = [ "standalone" ];
    };
    version = "1.0.0";
    category = "widget";
    description = "Flip clock widget";
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
