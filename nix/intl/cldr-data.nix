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

# Fails when a committed file cut from the pinned CLDR differs from what the tooling writes now:
# the base blob every binary embeds, each profile pack's blob and unit patterns, and the tests' `es-MX` blob.
{ lib, runCommand, delta, intl-cldr-blob, intl-tools, cldr-json }:

runCommand "intl-cldr-data"
{
  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../assets/intl/cldr-base.postcard
      ../../assets/intl/profile-languages
      ../../bmc-intl/core/fixtures/es-MX/cldr.postcard
    ];
  };
  nativeBuildInputs = [ delta intl-cldr-blob intl-tools ];
  CLDR_JSON = cldr-json;
}
  ''
    status=0

    size() {
      if [ -f "$1" ]; then echo "$(stat -c %s "$1") bytes"; else echo "none"; fi
    }

    # COMMITTED FRESH RECIPE [DIFF]; a blob shows its size alone,
    # as `icu4x-datagen` writes blobs but cannot read one back.
    compare() {
      if ! cmp -s "$2" "$src/$1"; then
        echo "$1 is not what the pinned CLDR gives now: run \`just intl::$3\`"
        echo "  committed: $(size "$src/$1"), fresh: $(size "$2")"
        if [ -n "''${4:-}" ]; then "$4" "$src/$1" "$2"; fi
        status=1
      fi
    }

    # Without `--detect-dark-light=never`, delta writes terminal queries into the log,
    # which a terminal later printing that log answers with junk input.
    units_diff() {
      mkdir -p "$TMPDIR/diff"
      if ! intl show-units "$(dirname "$1")" >"$TMPDIR/diff/committed.json" 2>"$TMPDIR/diff/refused.log"; then
        cat "$TMPDIR/diff/refused.log"
        return
      fi
      intl show-units "$(dirname "$2")" >"$TMPDIR/diff/fresh.json"
      (cd "$TMPDIR/diff" && delta --no-gitconfig --detect-dark-light=never --paging=never \
        --width=variable --line-numbers committed.json fresh.json) || true
    }

    quiet() {
      "$@" >"$TMPDIR/tool.log" 2>&1 || { cat "$TMPDIR/tool.log"; exit 1; }
    }

    quiet intl-cldr-blob en "$TMPDIR/base.postcard"
    compare assets/intl/cldr-base.postcard "$TMPDIR/base.postcard" cldr-data

    quiet intl-cldr-blob es-MX "$TMPDIR/es-MX.postcard"
    compare bmc-intl/core/fixtures/es-MX/cldr.postcard "$TMPDIR/es-MX.postcard" fixture-data

    for pack in "$src"/assets/intl/profile-languages/*/; do
      key="$(basename "$pack")"
      dir="assets/intl/profile-languages/$key"

      fresh="$TMPDIR/packs/$key"
      mkdir -p "$fresh"

      quiet intl-cldr-blob "$key" "$fresh/cldr.postcard"
      quiet intl cldr-units "$fresh"

      compare "$dir/cldr.postcard" "$fresh/cldr.postcard" "pack-data $dir"
      compare "$dir/units.postcard" "$fresh/units.postcard" "pack-data $dir" units_diff
    done

    touch "$out"
    exit "$status"
  ''
