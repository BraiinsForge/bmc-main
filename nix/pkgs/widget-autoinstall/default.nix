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

# Installs the listed widgets on devices that lack them, until the frontend
# can install widgets. Core joins `pkg` into itself, so devices never see
# it as a package of its own; `pkg` is null while the list is empty.
#
# Each activation or boot runs the step, which only registers a procd
# instance: an activation script runs under the profile lock, so it cannot
# change the profile itself.
{ bmc, armv7Pkgs, widgets, bmcNixCli, orchestratorService }:
let
  inherit (armv7Pkgs) lib;

  names = [ ];

  # Name, version and store path come from the entry the index publishes,
  # so they cannot disagree with it.
  resolve = map (name:
    let
      entry = widgets.${name} or (throw "auto-install entry ${name} is not a widget package");
    in
    {
      inherit name;
      inherit (entry) version;
      storePath = "${entry.pkg}";
    });

  service = "bmc-widget-autoinstall";
  stepName = "widget-autoinstall";
  activationPrefix = "999";

  # `service list` that fails says nothing about the instance, so it never
  # counts as idle.
  serviceIdle = ''
    service_idle() {
      out="$(ubus call service list "{\"name\":\"$1\"}" 2>/dev/null)" || return 1
      ! printf '%s' "$out" | grep -q '"running": *true'
    }
  '';

  mkScripts = { bmcNixCli, orchestratorService, packages }:
    let
      packageArgs = lib.escapeShellArgs (lib.concatMap
        (p: [ "--name" p.name "--version" p.version "--store-path" p.storePath ])
        packages);

      worker = armv7Pkgs.writeTextFile {
        name = service;
        executable = true;
        destination = "/bin/${service}";
        text = ''
          #!/bin/ash
          # procd sets LD_PRELOAD for its instances.
          unset LD_PRELOAD
          # bmc-nix-cli spawns nix-store, which procd's PATH does not reach.
          export PATH="$PATH:/run/current-profile/bin:/nix/var/nix/profiles/per-user/root/profile/bin"

          generation="$1"

          ${serviceIdle}
          # Let the orchestrator of the activation that started us take the
          # profile lock first; its service actions are lost otherwise.
          waited=0
          until service_idle ${orchestratorService}; do
            if [ "$waited" -ge 600 ]; then
              logger -t ${service} "service orchestrator not seen idle after ''${waited}s, not installing"
              exit 0
            fi
            sleep 2
            waited=$((waited + 2))
          done

          ${lib.getExe bmcNixCli} add-packages --if-missing \
            --if-current "$generation" --profile-dir "$(dirname "$generation")" \
            ${packageArgs}
          logger -t ${service} "add-packages exited with status $?"
          exit 0
        '';
      };

      step = armv7Pkgs.writeTextFile {
        name = stepName;
        executable = true;
        destination = "/bin/${stepName}";
        text = ''
          #!/bin/sh
          # A failing activation script rolls the activation back, so every
          # error here is ignored.

          ${serviceIdle}
          # The worker's own install activates a generation and lands here;
          # re-registering would make procd kill it mid-activation. An
          # unanswered list may hide a running worker too.
          if ! service_idle ${service}; then
            logger -t ${service} "install worker running or procd unreachable, not registering" || true
            exit 0
          fi

          ubus call service delete '{"name":"${service}"}' >/dev/null 2>&1 || true
          ubus call service set "{
            \"name\": \"${service}\",
            \"instances\": {
              \"main\": {
                \"command\": [\"${lib.getExe worker}\", \"$PROFILE_NEW_GENERATION\"],
                \"stdout\": true,
                \"stderr\": true
              }
            }
          }" >/dev/null 2>&1 || logger -t ${service} "failed to register the install worker" || true
          exit 0
        '';
      };
    in
    { inherit worker step; };

  mkAutoinstallPackage = packages: bmc.lib.mkPackage {
    name = "widget-autoinstall";
    activation = [{
      prefix = activationPrefix;
      bin = (mkScripts { inherit bmcNixCli orchestratorService packages; }).step;
    }];
  };

  autoInstall = resolve names;

  testBusybox = armv7Pkgs.buildPackages.busybox;

  scriptsTest =
    let
      testOrchestrator = "orchestrator-under-test";
      stubCli = armv7Pkgs.buildPackages.writeTextFile {
        name = "bmc-nix-cli-stub";
        executable = true;
        destination = "/bin/bmc-nix-cli";
        text = ''
          #!${lib.getExe' testBusybox "ash"}
          printf '%s\n' "$@" >> "$CLI_ARGS"
          printf '%s\n' "$PATH" > "$CLI_PATH"
          exit "$CLI_EXIT"
        '';
      };
      underTest = mkScripts {
        bmcNixCli = stubCli;
        orchestratorService = testOrchestrator;
        packages = [{
          name = "widget-a";
          version = "1.0.0";
          storePath = "/nix/store/00000000000000000000000000000000-widget-a";
        }];
      };
    in
    armv7Pkgs.runCommand "widget-autoinstall-test" { } ''
      PATH=${testBusybox}/bin \
        ${lib.getExe' testBusybox "ash"} \
          ${./tests/widget-autoinstall.sh} \
          ${lib.getExe underTest.step} \
          ${lib.getExe underTest.worker} \
          ${testOrchestrator}
      touch $out
    '';

  # The real list may be empty, so the layout is checked on a package
  # built for a probe widget.
  probeWidget = armv7Pkgs.writeTextDir "share/widget-probe" "";
  probe = mkAutoinstallPackage [{
    name = "widget-probe";
    version = "0.0.0";
    storePath = "${probeWidget}";
  }];
  activationScript = probe.activationScripts.${stepName};
  layoutTest =
    let
      probeClosure = armv7Pkgs.buildPackages.closureInfo { rootPaths = [ probe ]; };
    in
    armv7Pkgs.runCommand "widget-autoinstall-layout-test" { } ''
      test -x ${probe}/${activationScript}
      grep -Fx ${probeWidget} ${probeClosure}/store-paths
      touch $out
    '';
in
{
  pkg = if autoInstall == [ ] then null else mkAutoinstallPackage autoInstall;
  inherit autoInstall resolve probe activationScript;
  tests = {
    scripts = scriptsTest;
    layout = layoutTest;
  };
}
