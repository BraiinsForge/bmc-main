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

# mkPackage: Generic package builder for the BMC profile system.
#
# Assembles a package from optional components: a base derivation to symlink,
# hooks, activation scripts, arbitrary output files, copy-files for root
# filesystem, and conffiles for sysupgrade preservation.
{ pkgs, lib }:
let
  # mkPrioritizedEntries: Scan a directory for files with numeric prefixes
  # (e.g. "095-link-current") and produce a list of { prefix, bin } attrsets.
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
    , services ? [ ]
    , out ? [ ]
    , copyFiles ? [ ]
    , conffiles ? [ ]
    , postBuild ? ""
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

      servicesCmds = lib.concatMapStringsSep "\n"
        (s: ''
          cp ${s.service} $out/etc/init.d/${s.name}
        ''
        + lib.optionalString (s.enabled or true) ''
          ln -s ../init.d/${s.name} $out/etc/rc.d/S${toString s.start}${s.name}
        ''
        + lib.optionalString ((s.enabled or true) && (s.stop or null) != null) ''
          ln -s ../init.d/${s.name} $out/etc/rc.d/K${toString s.stop}${s.name}
        ''
        + lib.optionalString ((s.serviceConfigFile or null) != null) ''
          cp ${s.serviceConfigFile} $out/etc/init.d.conf/${s.name}.json
        '')
        services;

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

      servicesConffiles = lib.concatMap
        (s:
          [ "/etc/init.d/${s.name}" ]
          ++ lib.optional (s.enabled or true)
            "/etc/rc.d/S${toString s.start}${s.name}"
          ++ lib.optional ((s.enabled or true) && (s.stop or null) != null)
            "/etc/rc.d/K${toString s.stop}${s.name}"
        )
        services;

      allConffiles = servicesConffiles ++ conffiles;

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

        ${lib.optionalString (services != [ ]) ''
          mkdir -p $out/etc/init.d $out/etc/rc.d $out/etc/init.d.conf
          ${servicesCmds}
          # Mirror init.d and into special/copy so the
          # copy-files activation lands them at /etc on the device.
          # Do not mirror /etc/rc.d/*: those symlinks are created (and
          # removed) at activation time by rc.common's `enable` /
          # `disable`, driven by the orchestrator's `always` / `removed`
          # action lists. Shipping them via copy-files dereferences the
          # symlinks into plain files and breaks procd's service-name
          # derivation. Conffiles still list the rc.d paths so sysupgrade
          # preserves the runtime-created symlinks across flashes.
          mkdir -p $out/special/copy/etc/init.d $out/special/copy/etc/init.d.conf
          for f in $out/etc/init.d/*; do
            ln -s "$f" $out/special/copy/etc/init.d/
          done
        ''}

        ${lib.optionalString (out != [ ]) outCmds}

        ${lib.optionalString (copyFiles != [ ] || allConffiles != [ ]) ''
          mkdir -p $out/special/copy
          ${copyFilesCmds}
        ''}

        ${lib.optionalString (allConffiles != [ ]) ''
          mkdir -p $out/special/copy/lib/upgrade/keep.d
          printf '%s\n' ${lib.escapeShellArgs (allConffiles ++ [ conffilesPath ])} \
            > $out/special/copy/lib/upgrade/keep.d/${name}.conffiles
        ''}

        ${postBuild}
      '';

      paths = lib.optional (package != null) package ++ [ extras ];
    in
    pkgs.symlinkJoin {
      name = "bmc-${name}";
      inherit paths;
    };
}
