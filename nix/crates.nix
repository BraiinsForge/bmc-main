# crates: All Rust crate definitions for the workspace.
{ defineCrate }:
{
  bmc-mock = defineCrate {
    path = "./bmc-mock";
    packageName = "bmc-mock";
  };
  bmc-openwrt = defineCrate {
    path = "./bmc-openwrt";
    packageName = "bmc-openwrt";
  };
  bmc-nix-cli = defineCrate {
    path = "./bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-nix-cli";
  };
  bmc-hook-merge-files = defineCrate {
    path = "./bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-hook-merge-files";
  };
  bmc-hook-file-symlinks = defineCrate {
    path = "./bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-hook-file-symlinks";
  };
  bmc-hook-activation-resolver = defineCrate {
    path = "./bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-hook-activation-resolver";
  };
  widget-digital-clock = defineCrate {
    path = "./widgets/digital-clock";
    packageName = "bmc-widget-digital-clock";
  };
  widget-flip-clock = defineCrate {
    path = "./widgets/flip-clock";
    packageName = "bmc-widget-flip-clock";
  };
  widget-wasm = defineCrate {
    path = "./widgets/wasm";
    packageName = "bmc-widget-wasm";
  };
}
