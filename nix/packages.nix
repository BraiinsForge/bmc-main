# packages: All package definitions with release metadata.
#
# Single source of truth for what packages exist. Each entry pairs
# build logic with release metadata. Consumers (e.g. init-artifacts)
# select the subset they need.
{ bmc, armv7Pkgs, deps }:
let
  inherit (bmc.lib) mkWidgetPackage;
  inherit (bmc) crates;
  inherit (deps) widgetRuntimeDeps;
  profile = bmc.profiles.armv7-glibc-release;
in
{
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
}
