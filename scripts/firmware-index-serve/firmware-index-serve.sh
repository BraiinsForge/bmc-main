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

usage() {
    cat <<EOF
Usage: firmware-index-serve <proxy|local>

Serve a BMC firmware release index for upgrade testing on :8080
(override the port with BMC_FIRMWARE_INDEX_PORT).

  proxy   Mirror the internal release server
          (upstream: BMC_FIRMWARE_INDEX_UPSTREAM, default https://downloads.braiins.com.ii.zone)
  local   Host a local index and firmware files
          (root: BMC_FIRMWARE_INDEX_ROOT, default ./docs/devel/firmware)

Point the device at this machine with
  BMC_INDEX_URL="http://<lan-ip>:8080/braiins-deck"
EOF
}

# Set by the flake app wrapper; falls back to the script's own directory
# so the script also runs straight from the repo checkout.
config_dir="${FIRMWARE_INDEX_SERVE_CONFIG_DIR:-$(dirname "$0")}"

case "${1:-}" in
proxy | local) exec caddy run --config "$config_dir/Caddyfile.$1" ;;
-h | --help) usage ;;
*)
    usage >&2
    exit 1
    ;;
esac
