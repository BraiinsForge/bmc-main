# Copyright (C) 2025  Braiins Systems s.r.o.
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

{ pkgs }:

let
  inherit (pkgs.pnpmToolchain) pnpm fetchPnpmDeps pnpmConfigHook;

  # Just the install inputs, so editing frontend sources does not reinstall node_modules.
  src = pkgs.lib.fileset.toSource {
    root = ../.;
    fileset = pkgs.lib.fileset.unions [
      ../package.json
      ../pnpm-lock.yaml
      ../pnpm-workspace.yaml
    ];
  };

  # Fetch-only: `--force` pulls every platform's optional packages, so the hash is the same on every host.
  pnpmDeps = fetchPnpmDeps {
    # The lockfile's hash in the name makes a stale `hash` fail as a mismatch
    # rather than reuse the old store path.
    pname = "bmc-fe-" + builtins.hashFile "sha1" ../pnpm-lock.yaml;
    inherit src pnpm;
    fetcherVersion = 4;
    hash = "sha256-ow4xQHZOVgAEi8gHWXysFD0oD8PauLsIXJj8VmKG46U=";
    # pnpm 12 leaves an empty `v11/links` on darwin and not on linux, the one difference
    # between the two stores; `rmdir` fails should it ever hold anything.
    preFixup = ''
      if [ -d "$storePath/v11/links" ]; then rmdir "$storePath/v11/links"; fi
    '';
  };

  inherit (pkgs.lib.importJSON ../package.json) devEngines;
  sameMajor = declared: provided: pkgs.lib.versions.major declared == pkgs.lib.versions.major provided;
in

assert pkgs.lib.assertMsg
  (devEngines.packageManager.name == "pnpm" && sameMajor devEngines.packageManager.version pnpm.version)
  "frontend/package.json declares ${devEngines.packageManager.name} ${devEngines.packageManager.version} but nixpkgs provides pnpm ${pnpm.version}; align the major versions";

assert pkgs.lib.assertMsg (sameMajor devEngines.runtime.version pkgs.nodejs.version)
  "frontend/package.json declares node ${devEngines.runtime.version} but nixpkgs provides node ${pkgs.nodejs.version}; align the major versions";

pkgs.stdenv.mkDerivation {
  name = "bmc-fe-node-modules";
  inherit src pnpmDeps;

  nativeBuildInputs = [
    pnpm
    pnpmConfigHook
  ]
  ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.autoPatchelfHook ];

  # nodejs lets the fixup phase rewrite the `#!/usr/bin/env node` shebangs in node_modules;
  # it resolves them from `buildInputs`, the interpreters that run the output.
  buildInputs = [ pkgs.gcc-unwrapped pkgs.nodejs ];

  dontBuild = true;

  installPhase = ''
    mkdir -p $out
    mv -t $out node_modules
  '';

  passthru = { inherit pnpmDeps; };
}
