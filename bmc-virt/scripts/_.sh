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

# shellcheck shell=bash
# Shared helpers for bmc-virt scripts.
# Source this: . "$(dirname "$0")/_.sh"
#
# Scripts using this should have a nix shebang that provides:
#   openssh, sshpass, coreutils

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VIRT_DIR="$(dirname "$SCRIPT_DIR")"

export DATADIR="${BMC_VIRT_DATA:-$VIRT_DIR/vm-data}"
export LOGDIR="$DATADIR/logs"
export SSH_OPTS=(-F /dev/null -o StrictHostKeyChecking=no -o UserKnownHostsFile="$DATADIR/known_hosts" -o WarnWeakCrypto=no -p 2222)

ssh_vm() {
    sshpass -p root ssh "${SSH_OPTS[@]}" root@localhost "$@"
}

scp_vm() {
    sshpass -p root scp -F /dev/null -o StrictHostKeyChecking=no -o UserKnownHostsFile="$DATADIR/known_hosts" -o WarnWeakCrypto=no -P 2222 -O "$@"
}
