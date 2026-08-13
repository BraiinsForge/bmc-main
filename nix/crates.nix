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
  bmc = defineCrate {
    path = "bmc";
    packageName = "bmc";
    binName = false;
  };
  bmc-mock = defineCrate {
    path = "bmc-mock";
    packageName = "bmc-mock";
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
  wasm-assets = defineCrate {
    path = "bmc-wasm-assets";
    packageName = "bmc-wasm-assets";
  };
  wasm-host = defineCrate {
    path = "bmc-wasm-host";
    packageName = "bmc-wasm-host";
  };
}
