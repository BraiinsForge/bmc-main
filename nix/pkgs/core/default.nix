# Core package: bmc-openwrt with activation scripts, hooks, and copy-files.
#
# `profile` and `openwrtFeatures` are supplied by the caller so a debug build
# can flip on the compositor's `profiling` feature (ii-stopwatch timing + the
# mesh::profile observability channel) without forking this file.
{ bmc, armv7Pkgs, deps, profile, openwrtFeatures ? [ ] }:
let
  inherit (bmc.lib) mkPackage mkPrioritizedEntries autopatchelfBinaries
    mkOpenWrtService mkOpenWrtDaemon;
  inherit (bmc) crates;
  inherit (deps) compositorRuntimeDeps frontend;

  nixConf = import ../../nix-conf.nix { pkgs = armv7Pkgs; };
  nixConfActivation = import ./nix-conf-activation.nix { pkgs = armv7Pkgs; inherit nixConf; };
  testBusybox = armv7Pkgs.buildPackages.busybox;
  nixConfActivationTest = armv7Pkgs.runCommand "nix-conf-activation-test" { } ''
    PATH=${testBusybox}/bin \
    NIX_CONF_ACTIVATION_SHELL=${testBusybox}/bin/ash \
      ${testBusybox}/bin/ash \
        ${./tests/nix-conf-activation.sh} \
        ${nixConfActivation}/bin/nix-conf-activation
    ${testBusybox}/bin/touch $out
  '';
  nixActivatorTest = armv7Pkgs.runCommand "nix-activator-test" { } ''
    PATH=${testBusybox}/bin \
    NIX_ACTIVATOR_TEST_SHELL=${testBusybox}/bin/ash \
      ${testBusybox}/bin/ash \
        ${./tests/nix-activator.sh} \
        ${./files/nix-activator}
    ${testBusybox}/bin/touch $out
  '';

  bmcNix = profile.buildCrate crates.bmc-nix { };
  selectBmcNixBin = bmc.lib.selectBmcNixBin { pkgs = armv7Pkgs; inherit bmcNix; };

  orchestrator = selectBmcNixBin "bmc-nix-service-orchestrator";

  # Ash wrapper around the orchestrator. procd will run the service in
  # a context where LD_PRELOAD is set; Shebang is pinned to /bin/ash
  # (BusyBox on OpenWrt) — we disable stdenv fixup so patchShebangs
  # does not rewrite it to a nix-store bash path.
  orchestratorWrapped = armv7Pkgs.runCommand "bmc-nix-service-orchestrator-wrapped"
    {
      dontFixup = true;
    } ''
    mkdir -p $out/bin
    cat > $out/bin/bmc-nix-service-orchestrator <<'WRAPPER'
    #!/bin/ash
    unset LD_PRELOAD
    exec ${orchestrator}/bin/bmc-nix-service-orchestrator "$@"
    WRAPPER
    chmod +x $out/bin/bmc-nix-service-orchestrator
  '';

  bmc-openwrt = autopatchelfBinaries {
    drv = profile.buildCrate crates.bmc-openwrt {
      env.BMC_WEB_FRONTEND_DIR = "${frontend}";
      features = openwrtFeatures;
    };
    runtimeDeps = compositorRuntimeDeps armv7Pkgs;
  };

  bmc-compositor = mkOpenWrtDaemon {
    name = "bmc-compositor";
    start = 95;
    # TODO: Re-enable once the host firmware drops its monolithic
    # bmc-openwrt and we ship a firmware that runs the compositor from
    # this package. Until then ship the service file but keep it
    # disabled so it doesn't race the legacy app.
    enabled = false;
    command = "${bmc-openwrt}/bin/bmc-openwrt";
    args = [ "--log-to-file" ];
    env = {
      XDG_RUNTIME_DIR = "/tmp/runtime";
      # The package upgrade path spawns nix-store; procd services don't get
      # the login-shell PATH from files/profile, so mirror it here.
      PATH = "/usr/sbin:/usr/bin:/sbin:/bin:/run/current-profile/bin:/nix/var/nix/profiles/per-user/root/profile/bin";
    };
    preStart = "mkdir -p /tmp/runtime";
  };

  # Firmware-bridging activation step.
  #
  # The host firmware currently does not ship /etc/init.d/nix-activator,
  # so this activation entry lays it down on every activation and
  # removes the legacy nix-mounter that the activator subsumes.
  #
  # Remove this derivation, the activation entry below, ./files/nix-activator,
  # and the /etc/init.d/nix-activator + /etc/rc.d/S91nix-activator conffiles
  # entries once the firmware bundles nix-activator natively.
  firmware-init-services = armv7Pkgs.writeTextFile {
    name = "firmware-init-services";
    executable = true;
    destination = "/bin/firmware-init-services";
    text = ''
      #!/bin/sh
      set -e

      src="${./files/nix-activator}"
      target=/etc/init.d/nix-activator

      mkdir -p /etc/init.d /etc/rc.d
      if ! cmp -s "$src" "$target" 2>/dev/null; then
          cp "$src" "$target.tmp"
          chmod 755 "$target.tmp"
          mv -Tf "$target.tmp" "$target"
      fi
      # Recreate the rc.d symlink atomically: `ln -sf` unlinks before it
      # symlinks, and a power loss in that window would leave no
      # S91nix-activator at all — the activator would never run again.
      # The dot prefix keeps the temp name out of rc.d's S*/K* globbing.
      ln -sfn ../init.d/nix-activator /etc/rc.d/.S91nix-activator.tmp
      mv -Tf /etc/rc.d/.S91nix-activator.tmp /etc/rc.d/S91nix-activator

      rm -f /etc/init.d/nix-mounter
      rm -f /etc/rc.d/S*nix-mounter /etc/rc.d/K*nix-mounter

      # These are rootfs writes, outside the profile filesystem that the
      # 998 write-boundary syncfs covers; flush them before it durably
      # flips current. A lost S91nix-activator does not self-heal —
      # nothing would re-run the activator.
      sync
    '';
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
      executable="${orchestratorWrapped}/bin/bmc-nix-service-orchestrator"
      current_link="$(dirname "$PROFILE_NEW_GENERATION")/current"

      # Remove a stale instance from a previous activation, if any.
      ubus call service delete "{\"name\":\"$service_name\"}" 2>/dev/null || true

      # Register a one-shot procd instance for the orchestrator.
      ubus call service set "{
        \"name\": \"$service_name\",
        \"instances\": {
          \"$instance_name\": {
            \"command\": [
              \"$executable\",
              \"--old-generation=$PROFILE_OLD_GENERATION\",
              \"--new-generation=$PROFILE_NEW_GENERATION\",
              \"--current-link=$current_link\",
              \"--instance-name=$service_name\",
              \"--timeout-seconds=300\"
            ],
            \"stdout\": true,
            \"stderr\": true
          }
        }
      }"
    '';
  };

  package = mkPackage {
    name = "bmc-core";
    package = bmc-openwrt;
    hooks = [
      { prefix = "001"; bin = selectBmcNixBin "bmc-hook-merge-files"; }
      { prefix = "002"; bin = selectBmcNixBin "bmc-hook-file-symlinks"; }
      { prefix = "099"; bin = selectBmcNixBin "bmc-hook-activation-resolver"; }
    ];
    activation = mkPrioritizedEntries ./activation ++ [
      { prefix = "052"; bin = nixConfActivation; }
      { prefix = "055"; bin = selectBmcNixBin "bmc-activation-copy-files"; }
      { prefix = "060"; bin = firmware-init-services; }
      { prefix = "090"; bin = start-service-orchestrator; }
      # Durable 'current' commit. Runs last, after every other activation
      # step and before the 999-activated completion flag, so 'current'
      # advances to the new generation only once its activation succeeds.
      { prefix = "998"; bin = selectBmcNixBin "bmc-activation-write-boundary"; }
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
      "/var/log/bmc/bmc.log"
      "/var/log/bmc/widgets.log"
      "/var/log/bmc/bmc-nix-cli.log"
      "/var/log/nix-orchestrator/nix-orchestrator.log"
      "/etc/nix/nix.conf"
      "/etc/nix-upgrade/servers.json"
      "/etc/nix-upgrade/gc.json"
      "/etc/init.d/nix-activator"
      "/etc/rc.d/S91nix-activator"
    ];
  };

  packageWithTests = package.overrideAttrs (old: {
    passthru = (old.passthru or { }) // {
      tests.activation = nixConfActivationTest;
      tests.activator = nixActivatorTest;
    };
  });
in
{
  pkg = packageWithTests;
  version = "0.1.0";
  category = "core";
  description = "Core system package (bmc-openwrt + activation/hooks)";
  upgrade_strategy = "reboot";
  install_strategy = null;
}
