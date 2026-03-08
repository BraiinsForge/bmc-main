# packages: All package definitions with release metadata.
#
# Single source of truth for what packages exist. Each entry pairs a
# pre-built ARM derivation with its release metadata. Consumers (e.g.
# init-artifacts) select the subset they need.
#
# The `armv7Packages` argument is an attrset of pre-built ARM derivations
# keyed by name (e.g. { bmc = <drv>; nix = <drv>; ... }).
{ armv7Packages }:
{
  bmc = {
    pkg = armv7Packages.bmc;
    version = "0.1.0";
    category = "core";
    description = "Main display application";
    upgrade_strategy = "reboot";
    install_strategy = null;
  };
  nix = {
    pkg = armv7Packages.nix;
    version = armv7Packages.nix.version;
    category = "core";
    description = "Nix package manager";
    upgrade_strategy = "reboot";
    install_strategy = null;
  };
  bmc-nix-activation = {
    pkg = armv7Packages.bmc-nix-activation;
    version = "0.1.0";
    category = "core";
    description = "Profile hooks and activation scripts";
    upgrade_strategy = "reboot";
    install_strategy = false;
  };
  digital-clock = {
    pkg = armv7Packages.digital-clock;
    version = "1.0.0";
    category = "widget";
    description = "Digital clock widget";
    upgrade_strategy = null;
    install_strategy = null;
  };
  flip-clock = {
    pkg = armv7Packages.flip-clock;
    version = "1.0.0";
    category = "widget";
    description = "Flip clock widget";
    upgrade_strategy = null;
    install_strategy = null;
  };
}
