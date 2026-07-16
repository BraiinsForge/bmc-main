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

pkgs.stdenv.mkDerivation {
  name = "ii-fe-yarn-files-fixup";

  dontUnpack = true;
  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    cp -r $offlineCache $out
  '';

  buildInputs = [ pkgs.gcc-unwrapped ];
  nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.autoPatchelfHook ];

  # There are some binaries that
  #  - we do not use
  #  - use musl instead of glibc
  #  - we haven't found a way to not install
  # This is a workaround to ignore errors arising from these.
  autoPatchelfIgnoreMissingDeps = [ "libc.musl-x86_64.so.1" ];

  offlineCache = pkgs.stdenv.mkDerivation {
    pname = "bmc-fe-yarn-offline-cache";

    # This is the essential trick that allows FODs to be invalidated automatically.
    version = "0-" + builtins.hashFile "sha1" ../yarn.lock;

    src = ../.;

    nativeBuildInputs = with pkgs; [
      yarn
      cacert
    ];

    buildPhase = ''
      export HOME="$(mktemp -d)"

      # Disabled telemetry and make sure
      # that cache will be where we expect it
      yarn config set enableTelemetry 0
      yarn config set cacheFolder .yarn/cache
      yarn config set globalFolder .yarn/cache

      yarn config set --json supportedArchitectures.os '["linux", "darwin"]'
      yarn config set --json supportedArchitectures.cpu '["arm", "arm64", "ia32", "x64"]'

      yarn install
    '';

    installPhase = ''
      mkdir -p $out/.yarn
      mv -t $out/.yarn .yarn/cache
      mv -t $out .pnp.* node_modules
    '';

    dontConfigure = true;
    dontFixup = true;

    outputHashMode = "recursive";
    outputHash =
      # Platform-specific hashes because yarn/npm binaries differ between platforms
      # To get the hash for a new platform, set it to pkgs.lib.fakeHash
      # and run `nix build .#yarnFiles` - nix will tell you the expected hash
      {
        "x86_64-linux" = "sha256-vh2Bo+/AvHWP03miyNCwktHvIYldurpZW0zNuW7TrMU=";
        "aarch64-linux" = "sha256-vh2Bo+/AvHWP03miyNCwktHvIYldurpZW0zNuW7TrMU=";
        "x86_64-darwin" = "sha256-EpYwye7XrL9YnWgmhri3rXsvSo7lYUeZoBst57dPiRA=";
        "aarch64-darwin" = "sha256-EpYwye7XrL9YnWgmhri3rXsvSo7lYUeZoBst57dPiRA=";
      }.${pkgs.stdenv.hostPlatform.system} or (throw "Unsupported platform: ${pkgs.stdenv.hostPlatform.system}");
  };
}
