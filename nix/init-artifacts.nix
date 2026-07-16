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

# init-artifacts: Produces the initialization index, tarball, and package feed for ARM.
#
# `mkInitArtifacts` builds the init index + tarball for a given BOS
# version; the flake's stock outputs are `mkInitArtifacts { }` plus the
# placeholder package feed. `packageBump` replaces one package with a
# symlinkJoin wrapper carrying a marker file, so its version and store
# path change — the sysupgrade e2e uses it to make the upgrade plan
# against an already-initialized store non-empty.
{ self
, pkgs
, lib
, mkIndex
, mkTarball
, mkPackageFeed
, bmc-nix-cli
, packages
, hooksOverridePath ? null # native hooks for cross-compilation bootstrap
}:
let
  defaultBosVersion = "2026-03-27-0-a11e594b-26.02.1";
  defaultPkgsVersion = "20260715";
  defaultProfilePath = "/nix/var/nix/gcroots/profiles/bmc";

  # Ship every widget package (manifest-derived WASM widgets plus the
  # native flip-clock) alongside the core runtime.
  widgetNames =
    lib.attrNames (lib.filterAttrs (_: p: (p.category or "") == "widget") packages);

  defaultInitPackageNames = [
    "core"
    "bmc-nix-cli"
    "nix"
    "bos-avahi"
  ] ++ widgetNames;

  # Minted as installed_by: system in the tarball's profile manifest.
  # Their absence from a later package index aborts the upgrade; every
  # other shipped package (widgets included) is user-owned and merely
  # goes stale when it disappears from the indexes.
  requiredPackageNames = [
    "core"
    "nix"
  ];

  mkInitArtifacts =
    { bosVersion ? defaultBosVersion
    , packageBump ? null
    , initPackageNames ? defaultInitPackageNames
    , profilePath ? defaultProfilePath
    }:
    let
      bump = p:
        if packageBump == null || p.name != packageBump.name then p
        else p // {
          version = packageBump.version;
          pkg = pkgs.symlinkJoin {
            name = "${p.name}-${packageBump.version}-bump";
            paths = [ p.pkg ];
            postBuild = ''
              mkdir -p $out/share/sysupgrade-e2e
              echo '${packageBump.version}' > $out/share/sysupgrade-e2e/bump
            '';
          };
        };

      # Select init packages and convert to the list format mkIndex/mkTarball expect
      initPackages = map
        (name: bump (packages.${name} // { inherit name; }
          // lib.optionalAttrs (name == "core") { metadata = { bmc_version = bosVersion; }; }))
        initPackageNames;
    in
    {
      init-index-armv7 = mkIndex {
        packages = initPackages;
        caches = [{
          name = "default";
          cache_url = "https://cache.braiins.com";
          cache_key = "cache.braiins.com:placeholder";
        }];
        commit = self.rev or "dirty";
      };

      init-tarball-armv7 = mkTarball {
        packages = initPackages;
        inherit bmc-nix-cli hooksOverridePath;
        bos_version = bosVersion;
        system_packages = requiredPackageNames;
        profile_path = profilePath;
      };
    };

  packageFeed = mkPackageFeed {
    entries = [{
      bos_version = defaultBosVersion;
      download_url = "https://downloads.braiinsforge.com/tarballs/nix-${defaultBosVersion}.tar.gz";
      index_url = "https://downloads.braiinsforge.com/indexes/pkgs-${defaultPkgsVersion}/nix-package-index.v1.json";
      profile_path = defaultProfilePath;
    }];
  };
in
{
  inherit mkInitArtifacts;
  packages = mkInitArtifacts { } // {
    init-package-feed = packageFeed;
  };
}
