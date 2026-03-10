# Core package: bmc-openwrt binary, activation scripts, and profile hooks.
{ pkgs, lib }:
{ bmc-openwrt ? null
, bmc-hook-merge-files
, bmc-hook-file-symlinks
, bmc-hook-activation-resolver
}:
pkgs.runCommand "bmc-core" { } ''
  mkdir -p $out/bin
  mkdir -p $out/hooks
  mkdir -p $out/core/activation/scripts

  # Hook binaries — numbered for lexicographic execution order.
  # activation-resolver runs last to generate the entrypoint from all scripts.
  ln -s ${bmc-hook-merge-files}/bin/bmc-hook-merge-files $out/hooks/001-merge-files
  ln -s ${bmc-hook-file-symlinks}/bin/bmc-hook-file-symlinks $out/hooks/002-file-symlinks
  ln -s ${bmc-hook-activation-resolver}/bin/bmc-hook-activation-resolver $out/hooks/003-activation-resolver

  # Activation scripts run in alphanumerical order.
  # Scripts before 50-write-boundary perform checks only (no side effects).
  # Scripts after 50-write-boundary may have side effects.
  cp -r ${./activation/scripts}/. $out/core/activation/scripts
  chmod -R 755 $out/core/activation/scripts

  # Helper scripts
  mkdir -p $out/bin
  cp -r ${./scripts}/. $out/bin
  chmod -R 755 $out/bin

  ${lib.optionalString (bmc-openwrt != null) ''
    ln -s ${bmc-openwrt}/bin/bmc-openwrt $out/bin/bmc-openwrt
  ''}
''
