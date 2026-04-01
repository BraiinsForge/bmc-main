# mkPackage: Generic package builder for the BMC profile system.
#
# Assembles a package from optional components: a base derivation to symlink,
# hooks, activation scripts, arbitrary output files, copy-files for root
# filesystem, and conffiles for sysupgrade preservation.
{ pkgs, lib }:
let
  # mkPrioritizedEntries: Scan a directory for files with numeric prefixes
  # (e.g. "050-write-boundary") and produce a list of { prefix, bin } attrsets.
  mkPrioritizedEntries = dir:
    let
      entries = builtins.attrNames (builtins.readDir dir);
      parse = name:
        let
          parts = lib.splitString "-" name;
          prefix = builtins.head parts;
        in
        { inherit prefix; bin = dir + "/${name}"; };
    in
    map parse entries;
in
{
  inherit mkPrioritizedEntries;

  mkPackage =
    { name
    , package ? null
    , hooks ? [ ]
    , activation ? [ ]
    , out ? [ ]
    , copyFiles ? [ ]
    , conffiles ? [ ]
    }:
    let
      hooksCmds = lib.concatMapStringsSep "\n"
        (h:
          if lib.isDerivation h.bin
          then ''
            for hook in ${h.bin}/bin/*; do
              ln -s "$hook" "$out/hooks/${h.prefix}-$(basename "$hook")"
            done
          ''
          else ''
            ln -s ${h.bin} "$out/hooks/${h.prefix}-$(basename ${h.bin})"
          ''
        )
        hooks;

      activationCmds = lib.concatMapStringsSep "\n"
        (a:
          if lib.isDerivation a.bin
          then ''
            for act in ${a.bin}/bin/*; do
              ln -s "$act" "$out/core/activation/scripts/${a.prefix}-$(basename "$act")"
            done
          ''
          else ''
            cp ${a.bin} "$out/core/activation/scripts/${a.prefix}-$(basename ${a.bin})"
            chmod 755 "$out/core/activation/scripts/${a.prefix}-$(basename ${a.bin})"
          ''
        )
        activation;

      outCmds = lib.concatMapStringsSep "\n"
        (o: ''
          mkdir -p $out/${o.dest}
          cp -a ${o.src}/. $out/${o.dest}/
        '')
        out;

      copyFilesCmds = lib.concatMapStringsSep "\n"
        (cf:
          let
            dest = lib.removePrefix "/" cf.dest;
          in
          ''
            mkdir -p "$out/special/copy/$(dirname '${dest}')"
            cp '${cf.src}' "$out/special/copy/${dest}"
          '')
        copyFiles;

      conffilesPath = "/lib/upgrade/keep.d/${name}.conffiles";

      # Build extras (hooks, activation, out, copyFiles, conffiles) as a
      # separate derivation, then merge with package via symlinkJoin.
      extras = pkgs.runCommand "bmc-${name}-extras" { } ''
        mkdir -p $out

        ${lib.optionalString (hooks != [ ]) ''
          mkdir -p $out/hooks
          ${hooksCmds}
        ''}

        ${lib.optionalString (activation != [ ]) ''
          mkdir -p $out/core/activation/scripts
          ${activationCmds}
        ''}

        ${lib.optionalString (out != [ ]) outCmds}

        ${lib.optionalString (copyFiles != [ ] || conffiles != [ ]) ''
          mkdir -p $out/special/copy
          ${copyFilesCmds}
        ''}

        ${lib.optionalString (conffiles != [ ]) ''
          mkdir -p $out/special/copy/lib/upgrade/keep.d
          printf '%s\n' ${lib.escapeShellArgs (conffiles ++ [ conffilesPath ])} \
            > $out/special/copy/lib/upgrade/keep.d/${name}.conffiles
        ''}
      '';

      paths = lib.optional (package != null) package ++ [ extras ];
    in
    pkgs.symlinkJoin {
      name = "bmc-${name}";
      inherit paths;
    };
}
