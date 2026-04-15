{
  description = "Service orchestrator integration tests";

  inputs.bmc-main.url = "path:../../..";

  outputs = { self, bmc-main, ... }:
    let
      system = "x86_64-linux";
      bmc = bmc-main.bmc.${system};
      inherit (bmc.lib) mkPackage mkOpenWrtService;

      # -- test-init: new service → boot() + start() --------------------------
      test-init-service = mkOpenWrtService {
        name = "test-init";
        start = 95;
        functions = [
          { name = "boot"; body = ''echo "boot" > /tmp/init_boot''; }
          { name = "start"; body = ''echo "start" > /tmp/init_start''; }
        ];
      };

      # -- test-upgrade v1: writes /tmp/upgrade_failed on reload ---------------
      test-upgrade-v1-service = mkOpenWrtService {
        name = "test-upgrade";
        start = 96;
        functions = [
          { name = "reload"; body = ''echo "reload-v1" > /tmp/upgrade_failed''; }
        ];
      };

      # -- test-upgrade v2: writes /tmp/upgrade_reload on reload ---------------
      test-upgrade-v2-service = mkOpenWrtService {
        name = "test-upgrade";
        start = 96;
        functions = [
          { name = "boot"; body = ''echo "boot" > /tmp/upgrade_init_failed''; }
          { name = "start"; body = ''echo "start" > /tmp/upgrade_init_failed''; }
          { name = "reload"; body = ''echo "reload-v2" > /tmp/upgrade_reload''; }
        ];
      };

      # -- test-remove: removed service → stop() ------------------------------
      test-remove-service = mkOpenWrtService {
        name = "test-remove";
        start = 97;
        functions = [
          { name = "stop"; body = ''echo "stop" > /tmp/remove_stop''; }
        ];
      };

      # Package derivations (ARM)
      test-init-pkg = mkPackage {
        name = "test-init";
        services = [ test-init-service ];
      };
      test-upgrade-v1-pkg = mkPackage {
        name = "test-upgrade";
        services = [ test-upgrade-v1-service ];
      };
      test-upgrade-v2-pkg = mkPackage {
        name = "test-upgrade";
        services = [ test-upgrade-v2-service ];
      };
      test-remove-pkg = mkPackage {
        name = "test-remove";
        services = [ test-remove-service ];
      };

      pkgs = bmc-main.legacyPackages.${system}.pkgs;

      wrapper = pkgs.writeShellScriptBin "test-orchestrator" ''
        set -eu
        export TEST_INIT_STORE_PATH="${test-init-pkg}"
        export TEST_UPGRADE_V1_STORE_PATH="${test-upgrade-v1-pkg}"
        export TEST_UPGRADE_V2_STORE_PATH="${test-upgrade-v2-pkg}"
        export TEST_REMOVE_STORE_PATH="${test-remove-pkg}"

        # Source the test script from the same store path
        . "$(dirname "$0")/../share/test-orchestrator/test-orchestrator.sh"
      '';

      script = pkgs.runCommand "test-orchestrator-share" {} ''
        mkdir -p $out/share/test-orchestrator
        cp ${./test-orchestrator.sh} $out/share/test-orchestrator/test-orchestrator.sh
      '';
    in
    {
      packages.${system}.default = pkgs.symlinkJoin {
        name = "test-orchestrator";
        paths = [ wrapper script ];
      };
    };
}
