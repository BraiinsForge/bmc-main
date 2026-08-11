# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

# Core package: bmc-openwrt with activation scripts, hooks, and copy-files.
#
# `profile` and `openwrtFeatures` are supplied by the caller so a debug build
# can flip on the compositor's `profiling` feature (ii-stopwatch timing + the
# mesh::profile observability channel) without forking this file.
{ bmc, armv7Pkgs, deps, profile, wasmLauncher, openwrtFeatures ? [ ] }:
let
  inherit (bmc.lib) mkPackage mkPrioritizedEntries autopatchelfBinaries
    mkOpenWrtService mkOpenWrtDaemon;
  inherit (bmc) crates;
  inherit (deps) compositorRuntimeDeps frontend sounds;

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
  firmwareInitServicesTest = armv7Pkgs.runCommand "firmware-init-services-test" { } ''
    PATH=${testBusybox}/bin \
    FIRMWARE_INIT_SERVICES_TEST_SHELL=${testBusybox}/bin/ash \
      ${testBusybox}/bin/ash \
        ${./tests/firmware-init-services.sh} \
        ${firmware-init-services}/bin/firmware-init-services
    ${testBusybox}/bin/touch $out
  '';
  bmcCompositorServiceTest = armv7Pkgs.runCommand "bmc-compositor-service-test" { } ''
    PATH=${testBusybox}/bin \
      ${testBusybox}/bin/ash \
        ${./tests/bmc-compositor-service.sh} \
        ${bmc-compositor.service} \
        ${testBusybox}/bin/busybox
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
      env.BMC_SOUNDS_DIR = "${sounds}";
      features = openwrtFeatures;
    };
    runtimeDeps = compositorRuntimeDeps armv7Pkgs;
  };

  bmc-compositor = mkOpenWrtDaemon {
    name = "bmc-compositor";
    start = 95;
    enabled = true;
    command = "${bmc-openwrt}/bin/bmc-openwrt";
    args = [ "--log-to-file" ];
    env = {
      MESA_SHADER_CACHE_MAX_SIZE = "16M";
      XDG_CACHE_HOME = "/mnt/data/bmc/cache";
      XDG_RUNTIME_DIR = "/tmp/runtime";
      # The package upgrade path spawns nix-store; procd services don't get
      # the login-shell PATH from files/profile, so mirror it here.
      PATH = "/usr/sbin:/usr/bin:/sbin:/bin:/run/current-profile/bin:/nix/var/nix/profiles/per-user/root/profile/bin";
    };
    preStart = ''
      mkdir -p /tmp/runtime
      rm -rf /.cache/mesa_shader_cache \
        || logger -t bmc-compositor "failed to remove legacy Mesa shader cache"
      if mkdir -p /mnt/data/bmc/cache; then
        chmod 0700 /mnt/data/bmc/cache \
          || logger -t bmc-compositor "failed to secure persistent cache directory"
      else
        logger -t bmc-compositor "failed to create persistent cache directory"
      fi
    '';
  };

  # Firmware-bridging activation step.
  #
  # Firmware that bundles nix-activator is the canonical source: boot
  # runs the ROM copy from the ROM rc.d link and this entry leaves its
  # init.d path alone. Only on firmware without one does it lay down
  # ./files/nix-activator (the same contract in shell, independent of
  # the ROM CLI) as an overlay copy of /etc/init.d/nix-activator plus
  # an S91 rc.d link. It also stops and disables the legacy bmc service
  # before the profile-managed compositor starts. Neither bridge file is
  # a sysupgrade conffile: flashing any firmware sheds the bridge — a
  # bundling image boots its own
  # activator to consume the staged marker, while flashing another
  # bridge-needing image leaves the profile dormant until the next
  # deploy or init re-runs activation (accepted for the transition).
  # Either way this entry removes the legacy nix-mounter that the
  # activator subsumes.
  #
  # Remove this derivation, the activation entry below, and
  # ./files/nix-activator once no supported firmware lacks the bundled
  # activator.
  firmware-init-services = armv7Pkgs.writeTextFile {
    name = "firmware-init-services";
    executable = true;
    destination = "/bin/firmware-init-services";
    text = ''
      #!/bin/sh
      set -e

      root="''${FIRMWARE_INIT_SERVICES_ROOT:-}"
      src="${./files/nix-activator}"
      rom="$root/rom/etc/init.d/nix-activator"
      target="$root/etc/init.d/nix-activator"

      if [ ! -x "$rom" ]; then
          legacy_bmc="$root/etc/init.d/bmc"
          if [ -x "$legacy_bmc" ]; then
              "$legacy_bmc" stop
              "$legacy_bmc" disable
          fi

          mkdir -p "$root/etc/init.d" "$root/etc/rc.d"
          if ! cmp -s "$src" "$target" 2>/dev/null; then
              cp "$src" "$target.tmp"
              chmod 755 "$target.tmp"
              mv -Tf "$target.tmp" "$target"
          fi
      fi

      # The overlay S91 link is only needed when the firmware provides
      # no rc.d link of its own; a ROM link resolves through the merged
      # /etc/init.d path, so it runs the overlay copy where one masks
      # the ROM script. With both links enabled, boot ran the activator
      # twice and stacked /nix bind mounts. S91 is always
      # bootstrap-owned — firmware links its activator in the S6x
      # range — so it is safe to drop whenever the ROM has a link.
      if ls "$root"/rom/etc/rc.d/S[0-9]*nix-activator >/dev/null 2>&1; then
          rm -f "$root/etc/rc.d/S91nix-activator"
      else
          # Recreate the rc.d symlink atomically: `ln -sf` unlinks before
          # it symlinks, and a power loss in that window would leave no
          # S91nix-activator at all — the activator would never run again.
          # The dot prefix keeps the temp name out of rc.d's S*/K* globbing.
          ln -sfn ../init.d/nix-activator "$root/etc/rc.d/.S91nix-activator.tmp"
          mv -Tf "$root/etc/rc.d/.S91nix-activator.tmp" "$root/etc/rc.d/S91nix-activator"
      fi

      rm -f "$root/etc/init.d/nix-mounter"
      rm -f "$root"/etc/rc.d/S*nix-mounter "$root"/etc/rc.d/K*nix-mounter

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
    postBuild = ''
      chmod u+w $out/bin
      cp ${wasmLauncher}/bin/${wasmLauncher.launcherName} $out/bin/
    '';
    copyFiles = [
      { src = ./files/profile; dest = "/root/.profile"; }
    ];
    conffiles = [
      "/root/.profile"
      "/etc/bmc"
      "/var/log/bmc/bmc.log"
      "/var/log/bmc/widgets.log"
      "/var/log/bmc/bmc-nix-cli.log"
      "/var/log/nix-orchestrator/nix-orchestrator.log"
      "/etc/nix/nix.conf"
      "/etc/nix-upgrade/servers.json"
      "/etc/nix-upgrade/gc.json"
    ];
  };

  packageWithTests = package.overrideAttrs (old: {
    passthru = (old.passthru or { }) // {
      inherit wasmLauncher;
      tests.activation = nixConfActivationTest;
      tests.activator = nixActivatorTest;
      tests.compositor-service = bmcCompositorServiceTest;
      tests.firmware-init-services = firmwareInitServicesTest;
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
