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

{ pkgs, capture, wasmWidgets, wasmWidgetCatalog }:

let
  lib = pkgs.lib;

  # Widgets eligible for visual regression — only those
  # with a populated `capture/config.toml`.
  # Other widgets compile but don't ship capture fixtures yet.
  regressionCatalog =
    let
      catalog = lib.filterAttrs (_: w: w.hasCaptureConfig) wasmWidgetCatalog;
    in
    assert lib.assertMsg (catalog != { })
      "no widget carries a capture/config.toml, so wasm-regression would pass vacuously";
    catalog;

  # One derivation per widget. Each pins to:
  #   - that widget's source dir only (per-widget src cache key)
  #   - that widget's wasm derivation (per-widget wasm rebuild)
  #   - the capture wrapper for env + binary
  #
  # Every verify outcome is recorded in the output tree rather than the exit
  # status, so the job can upload the available evidence before the gate fails.
  mkWidgetReport = name: entry: pkgs.runCommand "wasm-regression-report-${name}"
    {
      nativeBuildInputs = [ capture.package ];
      src = entry.src;
      wasm = wasmWidgets.${name};
    } ''
    # verify labels every log line with the workspace directory's basename, and
    # this log is the artifact a human reads, so the directory is named rather
    # than mktemp'd — `tmp.XCv3Ka/clock` tells a reader nothing.
    widgets=$(mktemp -d)/widgets
    mkdir -p "$widgets"
    ln -s "$src" "$widgets/${name}"
    mkdir captures

    # Read verify's own status out of PIPESTATUS: piping into tee keeps the
    # build log streaming, and pipefail would otherwise let a tee write error
    # masquerade as a visual regression.
    set +o pipefail
    wasm-capture verify \
      --workspace="$widgets" \
      --wasm-dir="$wasm" \
      --output-dir=captures \
      --widget=${name} 2>&1 | tee captures/verify.log
    rc=''${PIPESTATUS[0]}

    # The log doubles as a downloadable artifact; verify paints for a TTY.
    sed -i 's/\x1b\[[0-9;]*m//g' captures/verify.log

    if [ ! -f captures/report.html ]; then
      verdict=broken
    elif [ "$rc" -eq 0 ]; then
      verdict=passed
    else
      verdict=failed
    fi

    mkdir -p "$out/$verdict/${name}"
    cp captures/verify.log "$out/$verdict/${name}/"
    if [ -f captures/report.html ]; then
      cp captures/report.html "$out/$verdict/${name}/"
    fi

    # Copied whole, not emptied into the artifact: the report links its media
    # by a path relative to itself, which flattening a level would break.
    if [ "$verdict" != passed ] && [ -d captures/${name} ]; then
      cp -r captures/${name} "$out/$verdict/${name}/"
    fi
  '';

  widgetReports = lib.mapAttrs mkWidgetReport regressionCatalog;

  # Merging the per-widget outputs unions their verdict trees.
  # `--no-preserve=mode` because store dirs are read-only and later widgets copy
  # into the ones the first widget created.
  report = pkgs.runCommand "wasm-regression-report" { } ''
    mkdir -p $out
    ${lib.concatMapStringsSep "\n"
      (d: ''cp -r --no-preserve=mode ${d}/. $out/'')
      (lib.attrValues widgetReports)}
  '';
in
{
  inherit report;

  # The captured logs already carry the per-frame counts and percentages, so the
  # gate replays them rather than inventing a summary format of its own.
  check = pkgs.runCommand "wasm-regression" { } ''
    if [ -d ${report}/failed ] || [ -d ${report}/broken ]; then
      for verdict in failed broken; do
        if [ -d ${report}/$verdict ]; then
          for log in ${report}/$verdict/*/verify.log; do
            cat "$log" >&2
          done
        fi
      done
      echo "download the wasm-regression job artifacts for report.html" >&2
      exit 1
    fi
    touch $out
  '';
}
