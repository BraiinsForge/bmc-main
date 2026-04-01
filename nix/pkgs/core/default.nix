# Core package: bmc-openwrt with activation scripts, hooks, and copy-files.
{ bmc, armv7Pkgs, deps }:
let
  inherit (bmc.lib) mkPackage mkPrioritizedEntries autopatchelfBinaries;
  inherit (bmc) crates;
  inherit (deps) compositorRuntimeDeps;
  profile = bmc.profiles.armv7-glibc-release;
in
{
  pkg = mkPackage {
    name = "bmc-core";
    package = autopatchelfBinaries {
      drv = profile.buildCrate crates.bmc-openwrt { };
      runtimeDeps = compositorRuntimeDeps armv7Pkgs;
    };
    hooks = [
      { prefix = "001"; bin = profile.buildCrate crates.bmc-hook-merge-files { }; }
      { prefix = "002"; bin = profile.buildCrate crates.bmc-hook-file-symlinks { }; }
      { prefix = "099"; bin = profile.buildCrate crates.bmc-hook-activation-resolver { }; }
    ];
    activation = mkPrioritizedEntries ./activation ++ [
      { prefix = "055"; bin = profile.buildCrate crates.bmc-activation-copy-files { }; }
    ];
    out = [
      { src = ./scripts; dest = "bin"; }
    ];
    copyFiles = [
      { src = ./files/nix-mounter; dest = "/etc/init.d/nix-mounter"; }
      { src = ./files/nix-activator; dest = "/etc/init.d/nix-activator"; }
      { src = ./files/profile; dest = "/root/.profile"; }
    ];
    conffiles = [
      "/etc/init.d/nix-mounter"
      "/etc/init.d/nix-activator"
      "/etc/rc.d/S91nix-mounter"
      "/etc/rc.d/S92nix-activator"
      "/root/.profile"
    ];
  };
  version = "0.1.0";
  category = "core";
  description = "Core system package (bmc-openwrt + activation/hooks)";
  upgrade_strategy = "reboot";
  install_strategy = null;
}
