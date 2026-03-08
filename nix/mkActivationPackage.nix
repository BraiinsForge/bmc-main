# mkActivationPackage: Build the core activation package.
#
# Assembles hook binaries and activation scripts into a package that
# provides the profile build hooks (merge-files, file-symlinks) and
# the core activation scripts (write-boundary, link-current).
#
# Activation scripts are numbered for alphanumerical execution order.
# Everything before 50-write-boundary is side-effect-free (checks only).
#
# The hook binaries and activation scripts must be for the same
# architecture (both ARM or both native).
{ pkgs, lib }:
{ bmc-hook-merge-files
, bmc-hook-file-symlinks
, bmc-hook-activation-resolver
, profile_path ? "/nix/var/nix/gcroots/profiles/bmc"
}:
pkgs.runCommand "bmc-nix-activation" { } ''
  mkdir -p $out/hooks
  mkdir -p $out/core/activation/scripts

  # Hook binaries — numbered for lexicographic execution order.
  # activation-resolver runs last to generate the entrypoint from all scripts.
  ln -s ${bmc-hook-merge-files}/bin/bmc-hook-merge-files $out/hooks/01-merge-files
  ln -s ${bmc-hook-file-symlinks}/bin/bmc-hook-file-symlinks $out/hooks/02-file-symlinks
  ln -s ${bmc-hook-activation-resolver}/bin/bmc-hook-activation-resolver $out/hooks/03-activation-resolver

  # Activation scripts run in alphanumerical order.
  # Scripts before 50-write-boundary perform checks only (no side effects).
  # Scripts after 50-write-boundary may have side effects.
  cp ${./scripts/activation/50-write-boundary} $out/core/activation/scripts/50-write-boundary
  chmod 755 $out/core/activation/scripts/50-write-boundary

  cp ${./scripts/activation/zzz-link-current} $out/core/activation/scripts/zzz-link-current
  chmod 755 $out/core/activation/scripts/zzz-link-current
''
