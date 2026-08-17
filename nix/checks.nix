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

{ pkgs
, ty-bin
, profiles
, crates
, deckPackages
, wasmExamples
, wasmWidgetsBundle
, wasmStackSize
, wasmWrapperTestPackages
,
}:

let
  lib = pkgs.lib;
  # Cargo config spells the stack size out again because the target-specific
  # environment variable replaces its rustflags array rather than merging it.
  wasmStackConfig = ../.cargo/config.toml;
  examplesCargoConfig = ../widgets-wasm-examples + "/.cargo/config.toml";
  configuredStackSize = config:
    let
      sizes = lib.filter (match: match != null)
        (map (line: builtins.match ".*-zstack-size=([0-9]+).*" line)
          (lib.splitString "\n" (builtins.readFile config)));
    in
    if sizes == [ ] then null else lib.toInt (lib.head (lib.head sizes));
  publicAsset = import ./public-asset.nix { inherit lib; };
  publicAssetManifest = ./test-data/public-assets/manifest.json;
  publicJpg = publicAsset.mkPublicIcon publicAssetManifest "icon.jpg";
  publicJpeg = publicAsset.mkPublicIcon publicAssetManifest "icon.jpeg";
  rejects = value: !(builtins.tryEval (builtins.deepSeq value true)).success;
  parseSdkMajor = import ./sdk-major.nix { inherit lib; };
  sdkMajor = parseSdkMajor (builtins.readFile ../bmc-wasm-runtime/protocol/src/version.rs);
  launcher = deckPackages.core.pkg.wasmLauncher;
  profileWrapper = deckPackages.widget-clock.pkg;
  bakedWrapper = wasmWrapperTestPackages.baked;
  inherit (wasmWrapperTestPackages) mkWasmLauncher;
  serviceLib = import ./service.nix { inherit pkgs lib; };
  fakeThin = pkgs.writeShellApplication {
    name = "bmc-wasm-thin";
    text = "exit 0";
  };
  changedThin = pkgs.writeShellApplication {
    name = "bmc-wasm-thin";
    text = "exit 1";
  };
  fakeHost = pkgs.writeShellApplication {
    name = "bmc-wasm-host";
    text = "exit 0";
  };
  changedHost = pkgs.writeShellApplication {
    name = "bmc-wasm-host";
    text = "exit 1";
  };
  dependencyLauncher = mkWasmLauncher { thin = fakeThin; host = fakeHost; };
  hostChangedLauncher = mkWasmLauncher { thin = fakeThin; host = changedHost; };
  thinChangedLauncher = mkWasmLauncher { thin = changedThin; host = fakeHost; };
  mkDependencyService = dependsOn: serviceLib.mkOpenWrtService {
    name = "dependency-test";
    start = 63;
    inherit dependsOn;
  };
  noDependencyService = mkDependencyService [ ];
  dependencyService = mkDependencyService [ dependencyLauncher ];
  hostChangedService = mkDependencyService [ hostChangedLauncher ];
  thinChangedService = mkDependencyService [ thinChangedLauncher ];
  orderedDependencyService = mkDependencyService [ fakeThin fakeHost ];
  productionDependencyService = mkDependencyService [ launcher ];

  productionCrates = {
    inherit (crates) bmc bmc-openwrt wasm-thin wasm-host;
  };
  productionSourceFiles = lib.unique (lib.concatLists (lib.mapAttrsToList
    (_: crate: (profiles.fast.buildCrate crate { }).srcFiles)
    productionCrates));
  expectedProductionRoots = [
    "bmc/Cargo.toml"
    "bmc-openwrt/Cargo.toml"
    "bmc-wasm-thin/Cargo.toml"
    "bmc-wasm-host/Cargo.toml"
  ];
  forbiddenSourceRoots = [ "widgets" "widgets-wasm" "widgets-wasm-examples" ];
  isUnder = root: path: path == root || lib.hasPrefix "${root}/" path;
  sourceBoundaryViolations = builtins.filter
    (path: builtins.any (root: isUnder root path) forbiddenSourceRoots)
    productionSourceFiles;
  leafCrateLeaks = builtins.filter
    (isUnder "bmc-widget-manifest-tests")
    productionSourceFiles;
  licenseHeaderExtensions = lib.filter (extension: extension != "")
    (lib.splitString "\n" (builtins.readFile ../scripts/license_header_extensions.txt));
in
{
  production-widget-source-boundary =
    assert lib.assertMsg
      (builtins.all (path: builtins.elem path productionSourceFiles) expectedProductionRoots)
      "production source-boundary check is missing an expected crate root";
    assert lib.assertMsg (sourceBoundaryViolations == [ ])
      "widget sources leaked into a production closure: ${builtins.toJSON sourceBoundaryViolations}";
    assert lib.assertMsg (leafCrateLeaks == [ ])
      "bmc-widget-manifest-tests leaked into a production closure";
    pkgs.runCommand "production-widget-source-boundary" { } ''
      touch $out
    '';

  service-depends-on =
    assert rejects (serviceLib.mkOpenWrtService {
      name = "variables-collision";
      start = 63;
      variables.DEPENDS_ON = "invalid";
    }).name;
    assert rejects (serviceLib.mkOpenWrtDaemon {
      name = "extra-variables-collision";
      start = 63;
      command = "/bin/true";
      extraVariables.DEPENDS_ON = "invalid";
    }).name;
    assert toString dependencyLauncher.thin == toString hostChangedLauncher.thin;
    assert toString dependencyLauncher.host != toString hostChangedLauncher.host;
    assert toString dependencyLauncher.thin != toString thinChangedLauncher.thin;
    assert toString dependencyLauncher.host == toString thinChangedLauncher.host;
    pkgs.runCommand "service-depends-on" { } ''
      if grep -F 'DEPENDS_ON=' ${noDependencyService.service}; then
        echo "empty dependsOn rendered a dependency line" >&2
        exit 1
      fi
      grep -Fx 'DEPENDS_ON="${fakeThin} ${fakeHost}"' ${orderedDependencyService.service}
      grep -Fx 'DEPENDS_ON="${launcher}"' ${productionDependencyService.service}
      test '${dependencyLauncher}' != '${hostChangedLauncher}'
      test '${dependencyLauncher}' != '${thinChangedLauncher}'
      ! cmp -s ${dependencyService.service} ${hostChangedService.service}
      ! cmp -s ${dependencyService.service} ${thinChangedService.service}
      touch $out
    '';

  wasm-sdk-major =
    assert sdkMajor == 0;
    assert rejects (parseSdkMajor "pub const SDK_VERSION:(u16, u16, u16) = (0, 2, 0);");
    assert rejects
      (parseSdkMajor ''
        pub const SDK_VERSION: (u16, u16, u16) = (0, 2, 0);
        pub const SDK_VERSION: (u16, u16, u16) = (1, 0, 0);
      '');
    pkgs.runCommand "wasm-sdk-major" { } ''
      touch $out
    '';

  wasm-launcher =
    assert launcher.sdkMajor == sdkMajor;
    assert launcher.launcherName == "bmc-wasm-thin-v${toString sdkMajor}";
    pkgs.runCommand "wasm-launcher" { } ''
      script=${launcher}/bin/${launcher.launcherName}
      test -x "$script"
      grep -F '#!/bin/sh' "$script"
      grep -F 'exec ${launcher.thin}/bin/bmc-wasm-thin --host-bin ${launcher.host}/bin/bmc-wasm-host "$@"' "$script"
      touch $out
    '';

  wasm-wrapper-profile =
    assert profileWrapper.wrapperMode == "profile";
    pkgs.runCommand "wasm-wrapper-profile" { } ''
      script=${profileWrapper}/lib/bmc-widgets/${profileWrapper.name}/bin/${profileWrapper.name}
      grep -F '/run/current-profile/bin/${launcher.launcherName}' "$script"
      grep -F '"$@"' "$script"
      if grep -F '${launcher.thin}' "$script" || grep -F '${launcher.host}' "$script"; then
        echo "profile wrapper contains a baked runtime path" >&2
        exit 1
      fi
      touch $out
    '';

  wasm-wrapper-baked =
    assert builtins.all (mode: mode == "baked") wasmWrapperTestPackages.bakedModes;
    assert bakedWrapper.wrapperMode == "baked";
    pkgs.runCommand "wasm-wrapper-baked" { } ''
      script=${bakedWrapper}/lib/bmc-widgets/${bakedWrapper.name}/bin/${bakedWrapper.name}
      grep -F '${launcher.thin}/bin/bmc-wasm-thin' "$script"
      grep -F -- '--host-bin ${launcher.host}/bin/bmc-wasm-host' "$script"
      grep -F '"$@"' "$script"
      if grep -F '/run/current-profile/bin/${launcher.launcherName}' "$script"; then
        echo "baked wrapper contains the profile launcher" >&2
        exit 1
      fi
      touch $out
    '';

  public-widget-assets =
    assert builtins.readFileType publicJpg == "regular";
    assert publicJpg == publicJpeg;
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "/icon.jpg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "../icon.jpg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "./icon.jpg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "nested//icon.jpg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "icon.exe");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "missing.svg");
    assert rejects (publicAsset.mkPublicIcon publicAssetManifest "icon-link.jpg");
    let icon = deckPackages.widget-clock.metadata.assets.icon;
    in
    assert builtins.readFileType icon == "regular";
    assert !(lib.hasInfix "/lib/bmc-widgets/" (toString icon));
    pkgs.runCommand "public-widget-assets" { } ''
      touch $out
    '';

  cargo-deny = profiles.fast.mkCargoDeny {
    config = ../deny.toml;
    checks = [ "bans" "sources" ];
  };

  cargo-deny-wasm = profiles.wasm-widgets-debug.mkCargoDeny {
    config = ../deny-wasm.toml;
    checks = [ "bans" "sources" ];
  };

  cargo-deny-wasm-examples = profiles.wasm-examples-debug.mkCargoDeny {
    config = ../deny-wasm.toml;
    checks = [ "bans" "sources" ];
  };

  # Block allocating fmt macros (format!, println!, dbg!, …)
  # in widget code via ast-grep. cargo-deny is crate-level
  # — this is macro-level.
  no-fmt-in-wasm = pkgs.runCommand "no-fmt-in-wasm"
    {
      nativeBuildInputs = [ pkgs.ast-grep ];
      src = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.unions [
          ../sgconfig.yml
          ../rules
          ../bmc-wasm-runtime/sdk/src
          ../bmc-wasm-runtime/protocol/src
          ../widgets-wasm
          ../widgets-wasm-examples
        ];
      };
    } ''
    cd $src
    ast-grep scan --error
    touch $out
  '';

  # `-zstack-size` leaves no trace outside the linked module, so the only
  # honest confirmation is to read the reservation back off every shipped
  # widget. Guards the cargo-side copies of the size in the same pass.
  wasm-stack-size =
    assert lib.assertMsg
      (configuredStackSize wasmStackConfig == wasmStackSize)
      "cargo config disagrees with workspace.nix on the wasm stack size (${toString wasmStackSize})";
    assert lib.assertMsg
      (!(builtins.pathExists (../widgets-wasm + "/.cargo/config.toml")))
      "widgets-wasm/.cargo/config.toml would override the repository wasm rustflags";
    assert lib.assertMsg
      (!(builtins.pathExists examplesCargoConfig)
        || configuredStackSize examplesCargoConfig == null)
      "the examples cargo config must not override the repository wasm rustflags";
    pkgs.runCommand "wasm-stack-size" { nativeBuildInputs = [ pkgs.python3 ]; } ''
      python3 ${../bmc-wasm-runtime/tools/wasm_stack.py} \
        --expect ${toString wasmStackSize} \
        ${wasmWidgetsBundle}/*.wasm ${wasmExamples}/*.wasm
      touch $out
    '';

  docs-wasm = profiles.fast.mkCargoDoc {
    package = "bmc-wasm-sdk";
  };

  build-wasm-widgets = profiles.wasm-widgets-debug.build;

  clippy-wasm-widgets = profiles.wasm-widgets-debug.clippy.overrideAttrs (old: {
    buildPhase = builtins.replaceStrings [ " --all-targets" ] [ " --target wasm32-unknown-unknown" ] old.buildPhase;
  });

  # Widget unit tests, compiled and run on the host target.
  test-wasm-widgets = profiles.wasm-widgets-host.nextest;

  # Every first-party source file must carry the GPL license header.
  # The script's exclusion list mirrors docs/devel/license-headers.md.
  license-headers = pkgs.runCommand "license-headers"
    {
      src = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.unions [
          ../scripts/check_license_headers.sh
          ../scripts/license_header_extensions.txt
          (lib.fileset.fileFilter
            (f: builtins.any f.hasExt licenseHeaderExtensions)
            ../.)
        ];
      };
    } ''
    bash $src/scripts/check_license_headers.sh
    touch $out
  '';

  # Deliberately not a pipeline job — see tools/test_textures.py for why.
  # Kept as a check so the suite has one reproducible run without a local venv.
  iss-texture-tools = pkgs.runCommand "iss-texture-tools"
    {
      nativeBuildInputs = [
        (pkgs.python3.withPackages (ps: [ ps.pytest ps.pillow ps.numpy ]))
      ];
      src = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.difference
          (lib.fileset.unions [
            (lib.fileset.fileFilter (f: f.hasExt "py") ../widgets-wasm/iss-position/tools)
            ../widgets-wasm/iss-position/tools/pyproject.toml
            ../widgets-wasm/iss-position/src/render/texture.jpg
          ])
          # kept out so nothing here can import cartopy,
          # which this check's python deliberately lacks
          ../widgets-wasm/iss-position/tools/texture_render.py;
      };
    } ''
    cp -r $src/widgets-wasm/iss-position widget
    chmod -R +w widget
    cd widget/tools
    pytest -q
    touch $out
  '';

  python-lint = pkgs.runCommand "python-lint"
    {
      nativeBuildInputs = [ pkgs.ruff ty-bin pkgs.python3 ];
      src = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.difference
          (lib.fileset.unions [
            (lib.fileset.fileFilter (f: f.hasExt "py") ../.)
            ../ruff.toml
          ])
          # subprojects with their own nix dev shell, deps, and lint setup
          (lib.fileset.unions [
            ../widgets-wasm
            ../widgets-wasm-examples
            ../bmc-virt/harness
            ../bmc-tui
          ]);
      };
    } ''
    cd $src
    export RUFF_CACHE_DIR="$(mktemp -d)"
    ruff check
    # Fail on @deprecated APIs; must be a CLI flag — ty ignores [tool.ty.rules] here.
    ty check --error deprecated
    touch $out
  '';
}
