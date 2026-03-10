# init-artifacts: Produces the initialization index and tarball for ARM.
#
# Selects init packages from the full packages attrset and produces
# the init-index and init-tarball for device provisioning.
{ self
, pkgs
, lib
, mkIndex
, mkTarball
, bmc-nix-cli
, packages
, hooksOverridePath ? null # native hooks for cross-compilation bootstrap
}:
let
  initPackageNames = [
    "core"
    "nix"
    "digital-clock"
    "flip-clock"
  ];

  # Select init packages and convert to the list format mkIndex/mkTarball expect
  initPackages = map
    (name: packages.${name} // { inherit name; })
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

  tarball = mkTarball {
    packages = initPackages;
    inherit bmc-nix-cli hooksOverridePath;
    bos_version = "26.02";
    extraFiles = pkgs.writeTextDir "etc/nix/nix.conf" ''
      substituters = https://cache.braiins.com
      trusted-public-keys = cache.braiins.com:placeholder
    '';
  };
in
{
  init-index-armv7 = index;
  init-tarball-armv7 = tarball;
}
