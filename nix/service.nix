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
      envNames = builtins.attrNames env;
      envLines = lib.concatMapStringsSep "\n"
        (k: ''procd_set_param env "${k}=${env.${k}}"'')
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
        ]
        ++ lib.optional (env != { }) envLines
        ++ [
          "procd_set_param respawn ${toString respawn.threshold} ${toString respawn.timeout} ${toString respawn.retry}"
          "procd_set_param stdout ${boolToInt stdout}"
          "procd_set_param stderr ${boolToInt stderr}"
          ''procd_set_param pidfile "${pidFile}"''
          "procd_set_param term_timeout ${toString termTimeout}"
          "procd_close_instance"
        ]
      );
      reloadBody = "stop\nstart";
      generatedFunctions = [
        { name = "start_service"; body = startBody; }
        { name = "reload_service"; body = reloadBody; }
      ];
    in
    mkOpenWrtService {
      inherit name start stop enabled serviceConfig;
      variables = { USE_PROCD = "1"; } // extraVariables;
      functions = generatedFunctions ++ extraFunctions;
    };
in
{
  inherit mkOpenWrtService mkOpenWrtDaemon;
}
