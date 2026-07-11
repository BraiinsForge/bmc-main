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
  profilePath = "/nix/var/nix/gcroots/profiles/bmc";

  # Ship every widget package (manifest-derived WASM widgets plus the
  # native flip-clock) alongside the core runtime.
  widgetNames =
    lib.attrNames (lib.filterAttrs (_: p: (p.category or "") == "widget") packages);

  initPackageNames = [
    "core"
    "bmc-nix-cli"
    "nix"
    "bos-avahi"
  ] ++ widgetNames;

  mkInitArtifacts = { bosVersion ? defaultBosVersion, packageBump ? null }:
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
        profile_path = profilePath;
      };
    };

  packageFeed = mkPackageFeed {
    entries = [{
      bos_version = defaultBosVersion;
      download_url = "https://downloads.braiinsforge.com/tarballs/nix-${defaultBosVersion}.tar.gz";
      index_url = "https://downloads.braiinsforge.com/indexes/pkgs-${defaultPkgsVersion}/nix-package-index.v1.json";
      profile_path = profilePath;
    }];
  };
in
{
  inherit mkInitArtifacts;
  packages = mkInitArtifacts { } // {
    init-package-feed = packageFeed;
  };
}
