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

# packages: All package definitions with release metadata.
#
# Single source of truth for what packages exist. Each entry pairs
# build logic with release metadata. Consumers (e.g. init-artifacts)
# select the subset they need.
{ bmc
, armv7Pkgs
, deps
, wasmWidgets
, thin
, host
, wasmLauncher
, mkWasmWidget
, wasmWidgetCatalog
, profile
, openwrtFeatures ? [ ]
}:
let
  inherit (bmc.lib) mkWidgetPackage;
  inherit (bmc) crates;
  inherit (deps) widgetRuntimeDeps frontend;
  lib = armv7Pkgs.lib;
  inherit (import ./public-asset.nix { inherit lib; }) mkPublicIcon;

  # Widget release metadata surfaced into the package index so the frontend
  # add-a-widget menu can discover installable widgets. Version, description and
  # picker fields are read from the widget's `manifest.json` at eval time
  # (read-only file read, no IFD). The icon path identifies a standalone flat
  # store file so upstream index tooling can collect and translate it.
  mkWidgetMetadata = { manifest }:
    let
      m = builtins.fromJSON (builtins.readFile manifest);
      icon = m.icon or null;
    in
    {
      version = m.version;
      description = m.description;
      metadata = {
        widget = {
          uid = m.uid;
          display_name = m.name;
          category = m.category or "misc";
          supported_viewports = m.supported_viewports or [ ];
        } // lib.optionalAttrs (m ? subname && m.subname != null) {
          subname = m.subname;
        };
      } // lib.optionalAttrs (icon != null) {
        assets.icon = mkPublicIcon manifest icon;
      };
    };

  # Per-wasm-widget package def, generated from the filesystem-derived catalog.
  mkWasmWidgetEntry = name: entry:
    let
      pkg = mkWasmWidget {
        inherit name thin host;
        wrapperMode = "profile";
        wasmDir = wasmWidgets.${name};
        inherit (entry) wasmFile manifest;
      };
      meta = mkWidgetMetadata { inherit (entry) manifest; };
    in
    {
      inherit pkg;
      inherit (meta) version description metadata;
      category = "widget";
      upgrade_strategy = null;
      install_strategy = null;
    };

  shippableCatalog = lib.filterAttrs (_: w: w.isShippable) wasmWidgetCatalog;
  wasmWidgetPackages = lib.mapAttrs'
    (name: entry: lib.nameValuePair "widget-${name}" (mkWasmWidgetEntry name entry))
    shippableCatalog;
in
wasmWidgetPackages // {
  core = import ./pkgs/core {
    inherit bmc armv7Pkgs deps profile openwrtFeatures wasmLauncher;
  };
  bos-avahi = import ./pkgs/bos-avahi { inherit bmc armv7Pkgs; };
  bmc-nix-cli = {
    pkg = profile.buildCrate crates.bmc-nix-cli { };
    version = "0.1.0";
    category = "core";
    description = "Nix package management CLI tool";
    upgrade_strategy = "reboot";
    install_strategy = null;
  };
  nix = {
    pkg = armv7Pkgs.nix;
    version = armv7Pkgs.nix.version;
    category = "core";
    description = "Nix package manager";
    upgrade_strategy = "reboot";
    install_strategy = null;
  };
  widget-flip-clock =
    let
      pkg = mkWidgetPackage {
        name = "flip-clock";
        crate = crates.widget-flip-clock;
        inherit profile;
        runtimeDeps = widgetRuntimeDeps.native;
        features = [ "standalone" ];
      };
      meta = mkWidgetMetadata {
        manifest = ../widgets/flip-clock/manifest.json;
      };
    in
    {
      inherit pkg;
      inherit (meta) version description metadata;
      category = "widget";
      upgrade_strategy = null;
      install_strategy = null;
    };
  bmc-frontend = {
    pkg = armv7Pkgs.runCommand "bmc-frontend-profile" { } ''
      mkdir -p $out/www
      ln -s ${frontend} $out/www/bmc
    '';
    version = "0.1.0";
    category = "dev";
    description = "Frontend web assets under www/bmc (dev use; not shipped)";
    upgrade_strategy = null;
    install_strategy = null;
  };
}
