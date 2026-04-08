# packages: All package definitions with release metadata.
#
# Single source of truth for what packages exist. Each entry pairs
# build logic with release metadata. Consumers (e.g. init-artifacts)
# select the subset they need.
{ bmc, armv7Pkgs, deps }:
let
  inherit (bmc.lib) mkCorePackage mkWidgetPackage autopatchelfBinaries;
  inherit (bmc) crates;
  inherit (deps) compositorRuntimeDeps widgetRuntimeDeps;
  profile = bmc.profiles.armv7-glibc-release;
in
{
  core = {
    pkg = mkCorePackage {
      bmc-openwrt = autopatchelfBinaries {
        drv = profile.buildCrate crates.bmc-openwrt { };
        runtimeDeps = compositorRuntimeDeps armv7Pkgs;
      };
      bmc-hook-merge-files = profile.buildCrate crates.bmc-hook-merge-files { };
      bmc-hook-file-symlinks = profile.buildCrate crates.bmc-hook-file-symlinks { };
      bmc-hook-activation-resolver = profile.buildCrate crates.bmc-hook-activation-resolver { };
    };
    version = "0.1.0";
    category = "core";
    description = "Core system package (bmc-openwrt + activation/hooks)";
    upgrade_strategy = "reboot";
    install_strategy = null;
  };
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
