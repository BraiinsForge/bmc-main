# Core package: bmc-openwrt with activation scripts, hooks, and copy-files.
{ bmc, armv7Pkgs, deps }:
let
  inherit (bmc.lib) mkPackage mkPrioritizedEntries autopatchelfBinaries
    mkOpenWrtService mkOpenWrtDaemon;
  inherit (bmc) crates;
  inherit (deps) compositorRuntimeDeps frontend;
  profile = bmc.profiles.armv7-glibc-release;

  orchestrator = profile.buildCrate crates.bmc-nix-service-orchestrator { };

  # Workaround: Nix-built glibc ships libc.so as a GNU ld linker script
  # (ASCII text), not a real ELF shared object. Glibc's ld.so internally
  # loads "libc.so" (unversioned) during initialization when launched by
  # procd, and fails on the linker script. This derivation provides a
  # directory with libc.so as a symlink to the real ELF (libc.so.6),
  # used via LD_LIBRARY_PATH in the procd service definition.
  glibc-libc-so-fix = armv7Pkgs.runCommand "glibc-libc-so-fix" { } ''
    mkdir -p $out/lib
    for f in ${armv7Pkgs.glibc}/lib/libc.so.*; do
      ln -s "$f" "$out/lib/libc.so"
      break
    done
  '';

  bmc-openwrt = autopatchelfBinaries {
    drv = profile.buildCrate crates.bmc-openwrt {
      env.BMC_WEB_FRONTEND_DIR = "${frontend}";
    };
    runtimeDeps = compositorRuntimeDeps armv7Pkgs;
  };

  bmc-compositor = mkOpenWrtDaemon {
    name = "bmc-compositor";
    start = 95;
    command = "${bmc-openwrt}/bin/bmc-openwrt";
    env = {
      XDG_RUNTIME_DIR = "/tmp/runtime";
    };
    preStart = "mkdir -p /tmp/runtime";
  };

  start-service-orchestrator = armv7Pkgs.writeTextFile {
    name = "start-service-orchestrator";
    executable = true;
    destination = "/bin/start-service-orchestrator";
    text = ''
      #!/bin/sh
      set -euxo pipefail

      service_name="bmc-nix-service-orchestrator"
      instance_name="main"
      binary="${orchestrator}/bin/bmc-nix-service-orchestrator"
      current_link="$(dirname "$PROFILE_NEW_GENERATION")/current"

      # Remove a stale instance from a previous activation, if any.
      ubus call service delete "{\"name\":\"$service_name\"}" 2>/dev/null || true

      # Register a one-shot procd instance for the orchestrator.
      ubus call service set "{
        \"name\": \"$service_name\",
        \"instances\": {
          \"$instance_name\": {
            \"command\": [
              \"$binary\",
              \"--old-generation\", \"$PROFILE_OLD_GENERATION\",
              \"--new-generation\", \"$PROFILE_NEW_GENERATION\",
              \"--current-link\", \"$current_link\",
              \"--instance-name\", \"$service_name\",
              \"--timeout-seconds\", \"300\"
            ],
            \"env\": {
              \"LD_LIBRARY_PATH\": \"${glibc-libc-so-fix}/lib\"
            },
            \"stdout\": true,
            \"stderr\": true
          }
        }
      }"
    '';
  };
in
{
  pkg = mkPackage {
    name = "bmc-core";
    package = bmc-openwrt;
    hooks = [
      { prefix = "001"; bin = profile.buildCrate crates.bmc-hook-merge-files { }; }
      { prefix = "002"; bin = profile.buildCrate crates.bmc-hook-file-symlinks { }; }
      { prefix = "099"; bin = profile.buildCrate crates.bmc-hook-activation-resolver { }; }
    ];
    activation = mkPrioritizedEntries ./activation ++ [
      { prefix = "055"; bin = profile.buildCrate crates.bmc-activation-copy-files { }; }
      { prefix = "090"; bin = start-service-orchestrator; }
    ];
    services = [ bmc-compositor ];
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
