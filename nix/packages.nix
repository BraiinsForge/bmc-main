# packages: All package definitions with release metadata.
#
# Single source of truth for what packages exist. Each entry pairs
# build logic with release metadata. Consumers (e.g. init-artifacts)
# select the subset they need.
{ bmc, armv7Pkgs, deps, wasmWidgets, thin, host, mkWasmWidget }:
let
  inherit (bmc.lib) mkWidgetPackage;
  inherit (bmc) crates;
  inherit (deps) widgetRuntimeDeps frontend;
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
      wasmDir = wasmWidgets.hello-widget;
      wasmFile = "hello_widget.wasm";
      manifest = ../bmc-wasm-runtime/examples/hello-widget/manifest.json;
      inherit thin host;
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
      wasmDir = wasmWidgets.calendar;
      wasmFile = "calendar.wasm";
      manifest = ../bmc-wasm-runtime/examples/calendar/manifest.json;
      inherit thin host;
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
      wasmDir = wasmWidgets.spacex-launch;
      wasmFile = "spacex_launch.wasm";
      manifest = ../bmc-wasm-runtime/examples/spacex-launch/manifest.json;
      inherit thin host;
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
      wasmDir = wasmWidgets.iss-position;
      wasmFile = "iss_position.wasm";
      manifest = ../bmc-wasm-runtime/examples/iss-position/manifest.json;
      inherit thin host;
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
      wasmDir = wasmWidgets.home-assistant;
      wasmFile = "home_assistant.wasm";
      manifest = ../bmc-wasm-runtime/examples/home-assistant/manifest.json;
      inherit thin host;
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
      wasmDir = wasmWidgets.media-control;
      wasmFile = "media_control.wasm";
      manifest = ../bmc-wasm-runtime/examples/media-control/manifest.json;
      inherit thin host;
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
      wasmDir = wasmWidgets.mesh-demo;
      wasmFile = "mesh_demo.wasm";
      manifest = ../bmc-wasm-runtime/examples/mesh-demo/manifest.json;
      inherit thin host;
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
      wasmDir = wasmWidgets.metronome;
      wasmFile = "metronome.wasm";
      manifest = ../bmc-wasm-runtime/examples/metronome/manifest.json;
      inherit thin host;
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
      wasmDir = wasmWidgets.pomodoro;
      wasmFile = "pomodoro.wasm";
      manifest = ../bmc-wasm-runtime/examples/pomodoro/manifest.json;
      inherit thin host;
    };
    version = "0.1.0";
    category = "widget";
    description = "Pomodoro timer with LED phase feedback and daily session tracking";
  };
  params-demo = {
    pkg = mkWasmWidget {
      name = "params-demo";
      wasmDir = wasmWidgets.params-demo;
      wasmFile = "params_demo.wasm";
      manifest = ../bmc-wasm-runtime/examples/params-demo/manifest.json;
      inherit thin host;
    };
    version = "0.1.0";
    category = "widget";
    description = "Read-back exemplar for every ParamKind variant + the structural-flag matrix";
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
