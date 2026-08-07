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

# mkOpenWrtService / mkOpenWrtDaemon: Generators for OpenWrt init.d scripts.
#
# mkOpenWrtService produces an executable flat file at $out matching the
# OpenWrt /etc/init.d convention.  mkOpenWrtDaemon wraps it for procd-managed
# daemons with declarative configuration.
{ pkgs, lib }:
let
  # indentBody: Take a body string, strip surrounding whitespace, and
  # re-indent every line with 4 spaces.
  indentBody = body:
    let
      trimmed = lib.trim body;
      lines = lib.splitString "\n" trimmed;
      indented = map (l: "    " + l) lines;
    in
    lib.concatStringsSep "\n" indented;

  # renderFunction: Render a single shell function definition.
  renderFunction = fn:
    "${fn.name}() {\n${indentBody fn.body}\n}";

  mkOpenWrtService =
    { name
    , start
    , stop ? 80
    , enabled ? true
    , serviceConfig ? null
    , shebang ? "#!/bin/sh /etc/rc.common"
    , variables ? { }
    , functions ? [ ]
    }:
    # NOTE: this is because at 90, unmount is called and since
    # Nix lives at a /mnt/data partition, it needs to not be busy
    # anymore.
      assert lib.assertMsg (stop == null || stop < 90)
        "mkOpenWrtService(${name}): stop must be lower than 90, got ${toString stop}";
      # NOTE: Production firmware mounts /nix through its ROM nix-activator at
      # START=62. Developer-only transitional firmware uses S91, so services in
      # slots 63..91 are unsupported there; keeping the production bound is
      # deliberate because that firmware is being discontinued.
      assert lib.assertMsg (start > 62)
        "mkOpenWrtService(${name}): start must be greater than 62, got ${toString start}";
      let
        allVariables = { START = toString start; }
          // lib.optionalAttrs (stop != null) { STOP = toString stop; }
          // variables;
        varLines = lib.concatMapStringsSep "\n"
          (k: ''${k}="${allVariables.${k}}"'')
          (builtins.attrNames allVariables);
        funcBlock = lib.concatStringsSep "\n\n" (map renderFunction functions);
        script = shebang + "\n\n"
          + varLines + "\n"
          + (lib.optionalString (functions != [ ]) ("\n" + funcBlock + "\n"));
        service = pkgs.writeTextFile {
          name = "init.d-${name}";
          text = script;
          executable = true;
        };
        # When the caller disables the service without supplying an explicit
        # serviceConfig, write an all-empty action set so the orchestrator does
        # not run `enable` (from the default `always`) or any other lifecycle
        # action against a service the caller asked not to touch. An explicit
        # serviceConfig always wins — the caller remains in charge.
        disabledDefault = {
          init = [ ];
          upgrade = [ ];
          removed = [ ];
          always = [ ];
        };
        effectiveServiceConfig =
          if serviceConfig != null then serviceConfig
          else if !enabled then disabledDefault
          else null;
        serviceConfigFile =
          if effectiveServiceConfig != null
          then pkgs.writeText "init.d.conf-${name}.json" (builtins.toJSON effectiveServiceConfig)
          else null;
      in
      { inherit name service start stop enabled serviceConfigFile; };

  mkOpenWrtDaemon =
    { name
    , start
    , command
    , args ? [ ]
    , env ? { }
    , preStart ? ""
    , respawn ? { threshold = 3600; timeout = 5; retry = 0; }
    , termTimeout ? 20
    , pidFile ? "/var/run/${name}.pid"
    , stop ? 80
    , enabled ? true
    , serviceConfig ? null
    , extraVariables ? { }
    , extraFunctions ? [ ]
    , stdout ? true
    , stderr ? true
    }:
    let
      quotedArgs = lib.concatMapStringsSep " "
        (a: ''"${a}"'')
        args;
      commandLine = ''"${command}"''
        + lib.optionalString (args != [ ]) (" " + quotedArgs);
      # Every daemon learns the name activation knows it by,
      # so it can find what the orchestrator files under that name.
      daemonEnv = env // { BMC_SERVICE_NAME = name; };
      envNames = builtins.attrNames daemonEnv;
      # A single call: repeated `procd_set_param env` calls overwrite each
      # other, keeping only the last variable.
      envLines = "procd_set_param env " + lib.concatMapStringsSep " "
        (k: ''"${k}=${daemonEnv.${k}}"'')
        envNames;
      boolToInt = b: if b then "1" else "0";
      startBody = lib.concatStringsSep "\n" (
        lib.optional (preStart != "") preStart
        ++ [
          "procd_open_instance"
          # NOTE: unfortunately we need to resort to this hack.
          # procd sets LD_PRELOAD to /lib/libsetlbf.so that depends on libc.so
          # breaking loading of libc, since libc.so from Nix store is a linker script.
          "procd_set_param command /bin/ash -c 'unset LD_PRELOAD; exec ${commandLine}'"
          envLines
          "procd_set_param respawn ${toString respawn.threshold} ${toString respawn.timeout} ${toString respawn.retry}"
          "procd_set_param stdout ${boolToInt stdout}"
          "procd_set_param stderr ${boolToInt stderr}"
          ''procd_set_param pidfile "${pidFile}"''
          "procd_set_param term_timeout ${toString termTimeout}"
          "procd_close_instance"
        ]
      );
      reloadBody = "stop\nstart";
      # Wait for the daemon PID to fully exit. procd bounds this via
      # term_timeout (SIGTERM then SIGKILL), so this loop is not open-ended
      # in practice.
      stoppedBody = ''
        pid=$(cat "${pidFile}" 2>/dev/null || true)
        [ -n "$pid" ] || return 0
        while [ -e /proc/$pid ]; do sleep 1; done
      '';
      generatedFunctions = [
        { name = "start_service"; body = startBody; }
        { name = "reload_service"; body = reloadBody; }
        { name = "service_stopped"; body = stoppedBody; }
      ];
    in
    assert lib.assertMsg (!(env ? BMC_SERVICE_NAME))
      "mkOpenWrtDaemon(${name}): env.BMC_SERVICE_NAME is reserved";
    mkOpenWrtService {
      inherit name start stop enabled serviceConfig;
      variables = { USE_PROCD = "1"; } // extraVariables;
      functions = generatedFunctions ++ extraFunctions;
    };
in
# Eval-time self-test: forcing a rejected start slot must throw, so the
  # bound cannot be lost in a refactor without every firmware eval failing.
assert lib.assertMsg
  (!(builtins.tryEval (mkOpenWrtService { name = "self-test"; start = 62; }).name).success)
  "mkOpenWrtService must reject start <= 62";
assert lib.assertMsg
  (
    !(builtins.tryEval (mkOpenWrtDaemon {
      name = "self-test";
      start = 63;
      command = "/bin/true";
      env.BMC_SERVICE_NAME = "conflict";
    }).name).success
  )
  "mkOpenWrtDaemon must reject env.BMC_SERVICE_NAME";
{
  inherit mkOpenWrtService mkOpenWrtDaemon;
}
