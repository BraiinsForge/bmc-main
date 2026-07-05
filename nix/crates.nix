# crates: All Rust crate definitions for the workspace.
{ defineCrate, lib, wasmWidgetCatalog }:
let
  # Per-widget wasm crate defs, generated from the filesystem-derived catalog.
  # `path` stays relative to the widget's own workspace (cargo resolves against
  # the workspace `Cargo.toml`), so the catalog's `workspaceName` tag drives
  # which release profile picks the crate up in `nix/wasm-widgets.nix`.
  wasmWidgetCrates = lib.mapAttrs'
    (name: entry: lib.nameValuePair "wasm-widget-${name}" (defineCrate {
      path = "${name}";
      # Select by the crate's real package name (from its manifest); it may differ
      # from the dir name to dodge a collision with a same-named dep crate (`image`).
      packageName = (builtins.fromTOML (builtins.readFile (entry.src + "/Cargo.toml"))).package.name;
      # Use --package, not --bin, since wasm is cdylib
      binName = false;
    }))
    wasmWidgetCatalog;
in
wasmWidgetCrates // {
  bmc-mock = defineCrate {
    path = "bmc-mock";
    packageName = "bmc-mock";
  };
  bmc-nix-init-mock = defineCrate {
    path = "bmc-nix-init-mock";
    packageName = "bmc-nix-init-mock";
  };
  bmc-nix-init-openwrt = defineCrate {
    path = "bmc-nix-init-openwrt";
    packageName = "bmc-nix-init-openwrt";
    # Produce binary named "bmc-nix-init" to match the OpenWrt service
    binName = "bmc-nix-init";
  };
  bmc-openwrt = defineCrate {
    path = "bmc-openwrt";
    packageName = "bmc-openwrt";
  };
  bmc-nix = defineCrate {
    path = "bmc-nix";
    packageName = "bmc-nix";
    binName = false;
  };
  bmc-nix-cli = defineCrate {
    path = "bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-nix-cli";
  };
  bmc-hook-merge-files = defineCrate {
    path = "bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-hook-merge-files";
  };
  bmc-hook-file-symlinks = defineCrate {
    path = "bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-hook-file-symlinks";
  };
  bmc-hook-activation-resolver = defineCrate {
    path = "bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-hook-activation-resolver";
  };
  bmc-activation-copy-files = defineCrate {
    path = "bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-activation-copy-files";
  };
  bmc-activation-write-boundary = defineCrate {
    path = "./bmc-nix";
    packageName = "bmc-nix";
    binName = "bmc-activation-write-boundary";
  };
  bmc-virt-leds = defineCrate {
    path = "bmc-virt/leds";
    packageName = "bmc-virt-leds";
  };
  bmc-virt-relay = defineCrate {
    path = "bmc-virt/relay";
    packageName = "bmc-virt-relay";
  };
  widget-flip-clock = defineCrate {
    path = "widgets/flip-clock";
    packageName = "bmc-widget-flip-clock";
  };
  wasm-thin = defineCrate {
    path = "bmc-wasm-thin";
    packageName = "bmc-wasm-thin";
  };
  wasm-host = defineCrate {
    path = "bmc-wasm-host";
    packageName = "bmc-wasm-host";
  };
}
