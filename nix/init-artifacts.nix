# init-artifacts: Produces the initialization index, tarball, and factory index for ARM.
#
# Selects init packages from the full packages attrset and produces
# the init-index, init-tarball, and init-factory-index for device provisioning.
{ self
, pkgs
, lib
, mkIndex
, mkTarball
, mkFactoryIndex
, bmc-nix-cli
, packages
, hooksOverridePath ? null # native hooks for cross-compilation bootstrap
}:
let
  bosVersion = "2026-03-27-0-a11e594b-26.02.1";
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

  # Select init packages and convert to the list format mkIndex/mkTarball expect
  initPackages = map
    (name: packages.${name} // { inherit name; }
      // lib.optionalAttrs (name == "core") { metadata = { bmc_version = bosVersion; }; })
    initPackageNames;

  index = mkIndex {
    packages = initPackages;
    caches = [{
      name = "default";
      cache_url = "https://cache.braiins.com";
      cache_key = "cache.braiins.com:placeholder";
    }];
    commit = self.rev or "dirty";
  };

  nixConf = import ./nix-conf.nix { inherit pkgs; };

  # Initial keep.d entry so sysupgrade preserves /etc/nix/nix.conf before
  # the core package (which also lists it) has been activated.
  nixKeepD = pkgs.writeText "nix.conffiles" ''
    /etc/nix/nix.conf
  '';

  tarball = mkTarball {
    packages = initPackages;
    inherit bmc-nix-cli hooksOverridePath;
    bos_version = bosVersion;
    profile_path = profilePath;
    extraFiles = pkgs.runCommand "init-extra-files" { } ''
      install -D -m 644 ${nixConf} $out/etc/nix/nix.conf
      install -D -m 644 ${nixKeepD} $out/lib/upgrade/keep.d/nix.conffiles
    '';
  };

  # Factory index — placeholder URL for local testing.
  # CI replaces this with the real URL via scripts/build-factory-index.sh.
  factoryIndex = mkFactoryIndex {
    tarballs = [{
      bos_version = bosVersion;
      download_url = "https://cache.braiins.com/v1/nix-${bosVersion}.tar.gz";
      profile_path = profilePath;
    }];
  };
in
{
  init-index-armv7 = index;
  init-tarball-armv7 = tarball;
  init-factory-index = factoryIndex;
}
