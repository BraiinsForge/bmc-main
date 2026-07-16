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

{
  description = "OpenWrt feed cache builder for bmc-virt";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      config = import ./openwrt-config.nix;

      mkFeedCache = { pkgs, guestArch }:
        let
          openwrtTarget = if guestArch == "aarch64" then "armsr/armv8" else "x86/64";
          openwrtTargetDash = if guestArch == "aarch64" then "armsr-armv8" else "x86-64";

          imageBuilder = pkgs.fetchurl {
            url = "https://downloads.openwrt.org/releases/${config.openwrtVersion}/targets/${openwrtTarget}/openwrt-imagebuilder-${config.openwrtVersion}-${openwrtTargetDash}.Linux-x86_64.tar.zst";
            hash = config.imageBuilderHash.${guestArch};
          };

          packageListStr = builtins.concatStringsSep " " config.packageList;
          manifest = config.mkManifest config.packageList;
          manifestFile = pkgs.writeText "${guestArch}.sha256" manifest;
        in
        pkgs.stdenv.mkDerivation {
          pname = "openwrt-feed-cache-${guestArch}";
          version = config.openwrtVersion;

          dontUnpack = true;
          dontConfigure = true;
          dontFixup = true;

          nativeBuildInputs = with pkgs; [
            gnumake
            bash
            perl
            python311
            gawk
            getopt
            coreutils
            ncurses
            findutils
            which
            file
            unzip
            bzip2
            xz
            zstd
            zlib
            rsync
            wget
            cacert
          ];

          # Must be built with: nix build --option sandbox false
          # Runs `make image` to download all packages (explicit + profile
          # defaults + transitive deps).  The built image is discarded —
          # only the dl/ cache directory is kept.
          buildPhase = ''
            tar xf ${imageBuilder}
            cd openwrt-imagebuilder-*
            patchShebangs .
            find . \( -name "Makefile" -o -name "GNUmakefile" -o -name "*.mk" \) \
              -exec sed -i 's|/usr/bin/env|${pkgs.coreutils}/bin/env|g' {} +

            export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
            export NIX_SSL_CERT_FILE=$SSL_CERT_FILE

            # Disable signature verification — upstream re-signs feeds after
            # release, so the ImageBuilder's keys no longer match.
            sed -i '/^option check_signature/d' repositories.conf

            DUMMY_FILES=$(mktemp -d)
            DUMMY_BIN=$(mktemp -d)

            make image \
              SHELL="${pkgs.bash}/bin/bash" \
              PROFILE="generic" \
              PACKAGES="${packageListStr}" \
              FILES="$DUMMY_FILES" \
              BIN_DIR="$DUMMY_BIN" \
              ROOTFS_PARTSIZE=1024 \
              2>&1
          '';

          installPhase = ''
            mkdir -p $out
            tar cf $out/${guestArch}.tar -C dl .
            cp ${manifestFile} $out/${guestArch}.sha256
          '';
        };
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        # ImageBuilder is always an x86_64-linux binary.
        # On other hosts Nix delegates to a remote x86_64-linux builder.
        x86Pkgs = import nixpkgs { system = "x86_64-linux"; };
      in
      {
        packages = {
          x86_64 = mkFeedCache { pkgs = x86Pkgs; guestArch = "x86_64"; };
          aarch64 = mkFeedCache { pkgs = x86Pkgs; guestArch = "aarch64"; };
        };
      });
}
