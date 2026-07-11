# mkTarball: Build an initial Nix store tarball for device provisioning.
#
# Produces a gzipped tarball containing:
# - All Nix store paths (full closure)
# - Nix SQLite database (populated via nix-store --load-db)
# - A pre-built profile generation (symlink tree, manifest, hooks)
#
# The profile is NOT activated (no `current` symlink). Activation
# happens on the device at first boot via `bmc-nix-cli init`.
{ pkgs, lib, mkIndex }:
{ packages # same format as mkIndex
, bmc-nix-cli # derivation of the bmc-nix CLI tool (native build)
, bos_version # e.g. "26.02"
, profile_path ? "/nix/var/nix/gcroots/profiles/bmc"
, hooksOverridePath ? null # path to native hook executables for cross-compilation bootstrap
}:
let
  # Generate a temporary index for bmc-nix-cli to consume
  index = mkIndex {
    inherit packages;
    caches = [{
      name = "local";
      cache_url = "file:///nix/store";
      cache_key = "local";
    }];
  };

  # Compute full runtime closure of all packages (bmc-nix-cli is a native
  # build tool, not an ARM runtime dependency -- it must not be in the closure)
  closureInfo = pkgs.closureInfo {
    rootPaths = map (p: p.pkg) packages;
  };

  tarballName = "nix-${bos_version}.tar.gz";
in
pkgs.runCommand "nix-tarball-${bos_version}"
{
  nativeBuildInputs = [ pkgs.nix pkgs.gzip bmc-nix-cli ];
  passthru = { inherit closureInfo index; };
} ''
  set -euo pipefail

  rootDir=$TMPDIR/root
  mkdir -p $rootDir/nix/store

  # 1. Build profile generation inside the tarball root
  bmc-nix-cli build-profile \
    --no-activate \
    --index ${index}/nix-package-index.v1.json \
    --profile-dir $rootDir${profile_path} \
    ${lib.optionalString (hooksOverridePath != null) "--hooks-override-path ${hooksOverridePath}"}

  # 2. Copy all store paths from the closure
  while IFS= read -r storePath; do
    cp -a "$storePath" $rootDir/nix/store/
  done < ${closureInfo}/store-paths

  # 3. Populate the Nix database
  export NIX_REMOTE=local?root=$rootDir
  export USER=nobody
  nix-store --load-db < ${closureInfo}/registration

  # 4. Create tarball
  mkdir -p $out
  tar -czf $out/${tarballName} \
    -C $rootDir \
    --sort=name \
    --mtime='@1' \
    --owner=0 --group=0 \
    --numeric-owner \
    --exclude='.lock' \
    .

  # 5. Write metadata
  cat > $out/metadata.json << 'METAEOF'
  ${builtins.toJSON {
    inherit bos_version profile_path;
    tarball_name = tarballName;
  }}
  METAEOF
''
