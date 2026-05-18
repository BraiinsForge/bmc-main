# crates: All Rust crate definitions for the workspace.
{ defineCrate, lib, wasmExampleNames }:
let
  # Per-example wasm crate defs, generated from the discovered example
  # names. Paths are relative to the wasm-release workspace root
  # (bmc-wasm-runtime/examples/), and the cargo package name matches the
  # directory name for every example in this workspace.
  exampleCrates = lib.listToAttrs (map
    (name: lib.nameValuePair "widget-example-${name}" (defineCrate {
      path = "./${name}";
      packageName = name;
      # Use --package, not --bin, since wasm is cdylib
      binName = false;
    }))
    wasmExampleNames);
in
exampleCrates // {
  bmc-mock = defineCrate {
    path = "./bmc-mock";
    packageName = "bmc-mock";
  };
  bmc-nix-init-mock = defineCrate {
    path = "./bmc-nix-init-mock";
    packageName = "bmc-nix-init-mock";
  };
  bmc-nix-init-openwrt = defineCrate {
    path = "./bmc-nix-init-openwrt";
    packageName = "bmc-nix-init-openwrt";
    # Produce binary named "bmc-nix-init" to match the OpenWrt service
    binName = "bmc-nix-init";
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
  bmc-nix-service-orchestrator = defineCrate {
    path = "./bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-nix-service-orchestrator";
  };
  bmc-activation-copy-files = defineCrate {
    path = "./bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-activation-copy-files";
  };
  bmc-virt-leds = defineCrate {
    path = "./bmc-virt/leds";
    packageName = "bmc-virt-leds";
  };
  bmc-virt-relay = defineCrate {
    path = "./bmc-virt/relay";
    packageName = "bmc-virt-relay";
  };
  widget-digital-clock = defineCrate {
    path = "./widgets/digital-clock";
    packageName = "bmc-widget-digital-clock";
  };
  widget-flip-clock = defineCrate {
    path = "./widgets/flip-clock";
    packageName = "bmc-widget-flip-clock";
  };
  wasm-thin = defineCrate {
    path = "./bmc-wasm-thin";
    packageName = "bmc-wasm-thin";
  };
  wasm-host = defineCrate {
    path = "./bmc-wasm-host";
    packageName = "bmc-wasm-host";
  };
}
