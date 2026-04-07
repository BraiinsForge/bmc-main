# Core package: bmc-openwrt with activation scripts, hooks, and copy-files.
{ bmc, armv7Pkgs, deps }:
let
  inherit (bmc.lib) mkPackage mkPrioritizedEntries autopatchelfBinaries
    mkOpenWrtService;
  inherit (bmc) crates;
  inherit (deps) compositorRuntimeDeps;
  profile = bmc.profiles.armv7-glibc-release;

  nix-mounter = mkOpenWrtService {
    name = "nix-mounter";
    start = 91;
    serviceConfig = { init = [ ]; removed = [ ]; upgrade = [ ]; };
    functions = [
      {
        name = "boot";
        body = ''
          [ -d /mnt/data/nix ] || return 0
          mkdir -p /nix
          mount --bind /mnt/data/nix /nix
        '';
      }
    ];
  };

  nix-activator = mkOpenWrtService {
    name = "nix-activator";
    start = 92;
    serviceConfig = { init = [ ]; removed = [ ]; upgrade = [ ]; };
    functions = [
      {
        name = "boot";
        body = ''
          grep -q ' /nix ' /proc/mounts || return 0
          profile_dir="/nix/var/nix/gcroots/profiles/bmc"
          current="$profile_dir/current"
          if [ -L "$current" ]; then
              entrypoint="$(readlink -f "$current")/core/activation/entrypoint"
              if [ -x "$entrypoint" ]; then
                  "$entrypoint"
              fi
          fi
        '';
      }
    ];
  };
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
    services = [ nix-mounter nix-activator ];
    out = [
      { src = ./scripts; dest = "bin"; }
    ];
    copyFiles = [
      { src = ./files/profile; dest = "/root/.profile"; }
    ];
    conffiles = [
      "/root/.profile"
    ];
  };
  version = "0.1.0";
  category = "core";
  description = "Core system package (bmc-openwrt + activation/hooks)";
  upgrade_strategy = "reboot";
  install_strategy = null;
}
