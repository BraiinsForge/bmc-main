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

# `intl-cldr-blob LOCALE OUT`: LOCALE's ICU4X blob, holding the data the marker probe links.
# The one place blobs are made, so the recipes and the freshness check cannot disagree.
{ writeShellApplication, coreutils, icu4x-datagen, cldr-json, intl-markers }:

writeShellApplication {
  name = "intl-cldr-blob";
  runtimeInputs = [ icu4x-datagen coreutils ];
  text = ''
    if [ "$#" -ne 2 ]; then
      echo "usage: intl-cldr-blob LOCALE OUT" >&2
      exit 2
    fi

    mkdir -p "$(dirname "$2")"

    # `retain-base-languages`: `check_catalog` refuses a pack whose locale data was deduplicated into root.
    exec icu4x-datagen \
      --markers-for-bin ${intl-markers}/bin/bmc-intl-markers \
      --locales "$1" \
      --cldr-root ${cldr-json} \
      --deduplication retain-base-languages \
      --format blob \
      --overwrite \
      --out "$2"
  '';
}
