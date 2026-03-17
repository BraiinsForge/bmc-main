# packages: All package definitions with release metadata.
#
# Single source of truth for what packages exist. Each entry pairs
# build logic with release metadata. Consumers (e.g. init-artifacts)
# select the subset they need.
{ bmc, armv7Pkgs, deps, wasmExamples, host, mkWasmWidget }:
let
  inherit (bmc.lib) mkWidgetPackage;
  inherit (bmc) crates;
  inherit (deps) widgetRuntimeDeps;
  profile = bmc.profiles.armv7-glibc-release;
in
{
  core = import ./pkgs/core { inherit bmc armv7Pkgs deps; };
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
  digital-clock = {
    pkg = mkWidgetPackage {
      name = "digital-clock";
      crate = crates.widget-digital-clock;
      inherit profile;
      runtimeDeps = widgetRuntimeDeps.slint;

      features = [ "standalone" ];
    };
    version = "1.0.0";
    category = "widget";
    description = "Digital clock widget";
    upgrade_strategy = null;
    install_strategy = null;
  };
  flip-clock = {
    pkg = mkWidgetPackage {
      name = "flip-clock";
      crate = crates.widget-flip-clock;
      inherit profile;
      runtimeDeps = widgetRuntimeDeps.native;

      features = [ "standalone" ];
    };
    version = "1.0.0";
    category = "widget";
    description = "Flip clock widget";
    upgrade_strategy = null;
    install_strategy = null;
  };
  hello-widget = {
    pkg = mkWasmWidget {
      name = "hello-widget";
      wasmDir = wasmExamples;
      wasmFile = "hello_widget.wasm";
      manifest = ../bmc-wasm-runtime/examples/hello-widget/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "Minimal WASM widget";
    upgrade_strategy = null;
    install_strategy = null;
  };
  calendar = {
    pkg = mkWasmWidget {
      name = "calendar";
      wasmDir = wasmExamples;
      wasmFile = "calendar.wasm";
      manifest = ../bmc-wasm-runtime/examples/calendar/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "iCal agenda view of upcoming events";
    upgrade_strategy = null;
    install_strategy = null;
  };
  spacex-launch = {
    pkg = mkWasmWidget {
      name = "spacex-launch";
      wasmDir = wasmExamples;
      wasmFile = "spacex_launch.wasm";
      manifest = ../bmc-wasm-runtime/examples/spacex-launch/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "Next SpaceX launch countdown and mission details";
    upgrade_strategy = null;
    install_strategy = null;
  };
  iss-position = {
    pkg = mkWasmWidget {
      name = "iss-position";
      wasmDir = wasmExamples;
      wasmFile = "iss_position.wasm";
      manifest = ../bmc-wasm-runtime/examples/iss-position/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "Live ISS position on a world map with orbital track";
    upgrade_strategy = null;
    install_strategy = null;
  };
  home-assistant = {
    pkg = mkWasmWidget {
      name = "home-assistant";
      wasmDir = wasmExamples;
      wasmFile = "home_assistant.wasm";
      manifest = ../bmc-wasm-runtime/examples/home-assistant/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "Live Home Assistant entity state via WebSocket";
    upgrade_strategy = null;
    install_strategy = null;
  };
  media-control = {
    pkg = mkWasmWidget {
      name = "media-control";
      wasmDir = wasmExamples;
      wasmFile = "media_control.wasm";
      manifest = ../bmc-wasm-runtime/examples/media-control/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "UPnP/DLNA/Cast/Kodi media playback remote";
    upgrade_strategy = null;
    install_strategy = null;
  };
  mesh-demo = {
    pkg = mkWasmWidget {
      name = "mesh-demo";
      wasmDir = wasmExamples;
      wasmFile = "mesh_demo.wasm";
      manifest = ../bmc-wasm-runtime/examples/mesh-demo/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "3D mesh rendering demo with Suzanne and dice tray";
    upgrade_strategy = null;
    install_strategy = null;
  };
  metronome = {
    pkg = mkWasmWidget {
      name = "metronome";
      wasmDir = wasmExamples;
      wasmFile = "metronome.wasm";
      manifest = ../bmc-wasm-runtime/examples/metronome/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "Tap-to-tempo metronome with audible click track";
    upgrade_strategy = null;
    install_strategy = null;
  };
  pomodoro = {
    pkg = mkWasmWidget {
      name = "pomodoro";
      wasmDir = wasmExamples;
      wasmFile = "pomodoro.wasm";
      manifest = ../bmc-wasm-runtime/examples/pomodoro/manifest.json;
      inherit host;
    };
    version = "0.1.0";
    category = "widget";
    description = "Pomodoro timer with LED phase feedback and daily session tracking";
    upgrade_strategy = null;
    install_strategy = null;
  };
}
